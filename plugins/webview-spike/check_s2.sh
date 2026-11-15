#!/bin/zsh
# The S2 gate of ticket #505. One self-contained script: the editor of the spike plugin in
# the test host, every phase of the ticket on the PR head.
#
#   1. boot: the host opens the editor and waits for the page's ready message on the native
#      side; it fails when the message does not arrive in 20 s (the host enforces the limit),
#   2. the host opens and closes the editor 50 times and fails on a crash or a missed ready,
#   3. the page reports its frame times while it scrolls and zooms the fixture of 8 tracks
#      and 5 minutes; the script fails when the 95th percentile is above 33 ms or on 0 frames,
#   4. the host sends Space, Right, Cmd+Z, Tab and Esc as real events; the script fails when
#      none of them reached the page (the per-key table goes into the report),
#   5. the focus comes back after a click in the host: the host takes the first responder
#      (its click in the host), a key then must NOT reach the page; a real mouse-down into
#      the editor and a key then must reach the page again. document.hasFocus() cannot turn
#      true on a host run from a shell (no activation, no key window); the input path is
#      the evidence.
#   6. the host resizes the window twice; the script fails when the page does not follow,
#   7. the fault run: a page that never sends the ready message; the script passes only when
#      that run reports the failure,
#   8. the standalone rule: cargo metadata --no-deps at the repo root does not name the spike
#      crate, cargo tree -p drumspike-desktop does not name it, and the diff of this head
#      touches only plugins/webview-spike/ and docs/design/daw-plugin/.
#
# The report is docs/design/daw-plugin/s2-page-in-plugin.md; the script fails when it is
# missing or carries no result table. Exit 0 = pass.
# Needs the built parts of ./spike.sh deps and ./spike.sh build.
set -u
cd "$(dirname "$0")" || exit 2
OUT=$PWD/out
HOST=$PWD/target/release/webview-spike-host
CLAP=$PWD/.deps/DrumSpikeWebviewSpike.clap
VST3=$PWD/.deps/build-wrapper/DrumSpikeWebviewSpike.vst3
ROOT=$PWD/../..
export CLAP_PATH=$PWD/.deps   # the wrapped VST3 finds the CLAP here at run time
REPORT=$ROOT/docs/design/daw-plugin/s2-page-in-plugin.md
fail=0

[[ -x $HOST ]] || { echo "FAIL: no test host at $HOST (run ./spike.sh build)"; exit 2; }
[[ -d $CLAP ]] || { echo "FAIL: no CLAP at $CLAP (run ./spike.sh build)"; exit 2; }
[[ -d $VST3 ]] || { echo "FAIL: no wrapped VST3 at $VST3 (run ./spike.sh build)"; exit 2; }
mkdir -p $OUT

run_guarded() {  # run_guarded <logfile> <seconds> <cmd...> : one command with a wall-clock guard
  local log=$1 limit=$2; shift 2
  "$@" > $log 2>&1 & local pid=$! n=0
  while kill -0 $pid 2>/dev/null; do /bin/sleep 1; n=$((n+1)); [[ $n -ge $limit ]] && { kill $pid; echo "FAIL: timeout after ${limit}s ($log)"; return 9; }; done
  wait $pid; return $?
}

verdict() {  # verdict <name> <rc> <want-rc> <detail>
  if [[ $2 -eq $3 ]]; then echo "ok   $1: $4"; else echo "FAIL $1: $4 (out/$( basename ${5:-x.log} 2>/dev/null))"; fi
}

# ---- 1. boot: the ready message of the page on the native side ----------------------
rm -f $OUT/ready-1.json $OUT/measure.json $OUT/keys.json $OUT/focus.json $OUT/resize.json
run_guarded $OUT/boot.log 40 $HOST $CLAP --mode boot --out $OUT
rc=$?
if [[ $rc -ne 0 ]] || [[ ! -f $OUT/ready-1.json ]]; then
  echo "FAIL boot: the editor did not report ready in 20 s (rc=$rc, out/boot.log)"; fail=1
else
  /usr/bin/python3 - $OUT/ready-1.json <<'PYREADY' || fail=1
import json, sys
r = json.load(open(sys.argv[1]))
assert r.get("kind") == "ready" and r.get("sid") == "fixture-s2", r
assert r.get("tracks") == 8, r
print("ok   boot: the page is ready in the editor (sid=%s, %s tracks, %dx%d)" % (r["sid"], r["tracks"], r["innerW"], r["innerH"]))
PYREADY
fi

# ---- 2. the 50 open/close cycles ----------------------------------------------------
run_guarded $OUT/cycles.log 900 $HOST $CLAP --mode boot --cycles 50 --out $OUT
rc=$?
if [[ $rc -ne 0 ]] || [[ ! -f $OUT/ready-50.json ]]; then
  echo "FAIL cycles: 50 open/close did not come through (rc=$rc, out/cycles.log)"; fail=1
else
  echo "ok   cycles: the editor opened and closed 50 times, ready every time, no crash"
fi

# ---- 3. the frame times while the page scrolls and zooms ----------------------------
run_guarded $OUT/measure-run.log 90 $HOST $CLAP --mode measure --out $OUT
rc=$?
if [[ $rc -ne 0 ]] || [[ ! -f $OUT/measure.json ]]; then
  echo "FAIL measure: no frame-time report (rc=$rc, out/measure-run.log)"; fail=1
else
  /usr/bin/python3 - $OUT/measure.json <<'PYMEASURE' || fail=1
import json, sys
r = json.load(open(sys.argv[1]))
frames, p95 = r.get("frames", 0), r.get("p95_ms")
if not frames:
    print("FAIL measure: 0 frames reported"); sys.exit(1)
if p95 is None or p95 > 33.0:
    print("FAIL measure: p95 %.2f ms is above 33 ms" % (p95 or -1)); sys.exit(1)
print("ok   measure: %d frames over 8 s of scroll and zoom, p95 %.2f ms (max %.2f ms), %dx%d px" %
      (frames, p95, r.get("max_ms", -1), r.get("innerW", 0), r.get("innerH", 0)))
PYMEASURE
fi

# ---- 4. the keys of the gate reach the page -----------------------------------------
run_guarded $OUT/keys-run.log 60 $HOST $CLAP --mode keys --out $OUT
rc=$?
if [[ $rc -ne 0 ]] || [[ ! -f $OUT/keys.json ]]; then
  echo "FAIL keys: no key report (rc=$rc, out/keys-run.log)"; fail=1
else
  /usr/bin/python3 - $OUT/keys.json <<'PYKEYS' || fail=1
import json, sys
r = json.load(open(sys.argv[1]))
seen = {e["key"]: e for e in r.get("seen", [])}
WANT = [("Space", False), ("Right", False), ("Z", True), ("Tab", False), ("Esc", False)]
missing = [k for k, cmd in WANT if k not in seen]
if missing:
    print("FAIL keys: %s did not reach the page" % ", ".join(missing)); sys.exit(1)
acted = [k for k, cmd in WANT if k in seen and seen[k].get("page")]
hostcalls = [c.get("call") for c in r.get("hostCalls", [])]
print("ok   keys: all five reached the page (%s); the page ran: %s; host transport calls: %s" %
      (", ".join("%s%s" % (k, "+Cmd" if seen[k].get("meta") else "") for k, _ in WANT),
       ",".join(acted) or "none", ",".join(hostcalls) or "none"))
PYKEYS
fi

# ---- 5. the focus comes back after a click in the host ------------------------------
run_guarded $OUT/focus-run.log 60 $HOST $CLAP --mode focus --out $OUT
rc=$?
if [[ $rc -ne 0 ]] || [[ ! -f $OUT/focus.json ]]; then
  echo "FAIL focus: no focus report (rc=$rc, out/focus-run.log)"; fail=1
else
  /usr/bin/python3 - $OUT/focus.json <<'PYFOCUS' || fail=1
import json, sys
r = json.load(open(sys.argv[1]))
s = r.get("samples", [])
keys = r.get("keys", [])
if not s:
    print("FAIL focus: no samples"); sys.exit(1)
if not keys:
    print("FAIL focus: no key reached the page at all"); sys.exit(1)
lost = [k for k in keys if 2800 <= k["t"] <= 4300]
back = [k for k in keys if k["t"] >= 4700]
hf = sorted(set(x["hasFocus"] for x in s))
if lost:
    print("FAIL focus: a key reached the page while the host had the focus"); sys.exit(1)
if not back:
    print("FAIL focus: the input did not come back after the click into the editor"); sys.exit(1)
print("ok   focus: the host click took the input from the page and the click into the editor gave it back; document.hasFocus()=%s over the run" % hf)
PYFOCUS
fi

# ---- 6. a resize works ---------------------------------------------------------------
run_guarded $OUT/resize-run.log 60 $HOST $CLAP --mode resize --out $OUT
rc=$?
if [[ $rc -ne 0 ]] || [[ ! -f $OUT/resize.json ]]; then
  echo "FAIL resize: no resize report (rc=$rc, out/resize-run.log)"; fail=1
else
  /usr/bin/python3 - $OUT/resize.json <<'PYRESIZE' || fail=1
import json, sys
r = json.load(open(sys.argv[1]))
s = r.get("samples", [])
if len(s) < 3:
    print("FAIL resize: too few samples"); sys.exit(1)
w0 = s[0]["innerW"]
big = max(x["innerW"] for x in s if x["t"] > 2500)
small = min(x["innerW"] for x in s if x["t"] > 4500)
if not (big > w0 + 100 and small < w0 - 100):
    print("FAIL resize: the page did not follow the window (%d -> %d -> %d)" % (w0, big, small)); sys.exit(1)
print("ok   resize: the page follows the window (%d -> %d -> %d px wide)" % (w0, big, small))
PYRESIZE
fi

# ---- 7. the fault run: a page that never sends ready must report the failure ---------
rm -rf $OUT/fault; mkdir -p $OUT/fault
run_guarded $OUT/fault.log 60 $HOST $CLAP --mode no-ready --out $OUT/fault
rc=$?
if [[ $rc -eq 0 ]] || [[ -f $OUT/fault/ready-1.json ]]; then
  echo "FAIL fault: the no-ready run passed (rc=$rc)"; fail=1
else
  echo "ok   fault: the host reported the missing ready message (rc=$rc, out/fault.log)"
fi

# ---- 8. the standalone rule ----------------------------------------------------------
if (cd $ROOT && cargo metadata --no-deps --format-version 1) > $OUT/s2-metadata.json 2>$OUT/s2-metadata.err \
   && ! grep -q ds-webview-spike $OUT/s2-metadata.json; then
  echo "ok   standalone: cargo metadata --no-deps at the repo root does not name the spike crate"
else
  echo "FAIL standalone: cargo metadata --no-deps at the repo root (out/s2-metadata.err)"; fail=1
fi
if (cd $ROOT && cargo tree -p drumspike-desktop 2>$OUT/s2-tree.err) | grep -q ds-webview-spike; then
  echo "FAIL standalone: cargo tree -p drumspike-desktop names the spike crate"; fail=1
else
  echo "ok   standalone: cargo tree -p drumspike-desktop does not name the spike crate"
fi
if git -C $ROOT rev-parse --verify -q origin/main >/dev/null; then
  bad=0
  for f in ${(f)"$(git -C $ROOT diff --name-only origin/main...HEAD)"}; do
    if [[ $f == plugins/webview-spike/* || $f == docs/design/daw-plugin/* ]]; then continue; fi
    echo "FAIL standalone: the diff touches $f"; bad=1
  done
  [[ $bad -eq 0 ]] && echo "ok   standalone: the diff touches only plugins/webview-spike/ and docs/design/daw-plugin/"
  [[ $bad -eq 0 ]] || fail=1
else
  echo "FAIL standalone: no origin/main next to this head"; fail=1
fi

# ---- the report must be there and carry the result table ----------------------------
if [[ -f $REPORT ]] && grep -q "Result" $REPORT && grep -q "^| " $REPORT; then
  echo "ok   report: $REPORT carries the result table"
else
  echo "FAIL report: $REPORT is missing or carries no result table"; fail=1
fi

# ---- the verdict ---------------------------------------------------------------------
if [[ $fail -eq 0 ]]; then
  echo "S2 CHECK: PASS (boot ready, 50 open/close cycles, frame times p95 <= 33 ms, keys, focus, resize, fault run, standalone rule)"
else
  echo "S2 CHECK: FAIL"
fi
exit $fail
