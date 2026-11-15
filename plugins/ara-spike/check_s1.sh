#!/bin/zsh
# The S1 gate of ticket #504. One self-contained script:
#
# 1. it generates three WAV fixtures (different lengths, one at another sample rate, all with
#    an iXML chunk: the test host loader loops without one),
# 2. it loads the plugin in the test host of the ARA examples - the CLAP and the wrapped VST3 -
#    with -test ContentReading; the plugin reads every sample of each fixture through ARA on a
#    background thread and writes one JSON line per source to $DS_ARA_SPIKE_OUT (a file of JSON
#    objects: sample count, sample rate, channel count, FNV-1a hash of all samples),
# 3. it reads each fixture itself (a RIFF walk over fmt and data) and compares every value with
#    what the plugin reported; a difference fails the script,
# 4. pluginval 1.0.4 at strictness 5 on the wrapped VST3; clap-validator 0.4.1 on the CLAP
#    (pluginval 1.0.4 has no CLAP support; the version is named in the report),
# 5. the standalone rule: cargo metadata --no-deps at the repo root does not name the spike
#    crate, and the diff of this head touches no file outside plugins/ara-spike/ and
#    docs/design/daw-plugin/.
#
# The script prints the number of sources it compared and fails on 0. Exit 0 = pass.
# Needs the built parts of ./spike.sh deps and ./spike.sh build (DS_ARA_DEPS overrides .deps).
set -u
cd "$(dirname "$0")" || exit 2
DEPS=${DS_ARA_DEPS:-$PWD/.deps}
OUT=$PWD/out
H=$DEPS/build-examples/bin/Debug/ARATestHost
CLAP=$DEPS/DrumSpikeAraSpike.clap
VST3=$DEPS/build-wrapper/DrumSpikeAraSpike.vst3
PLUGINVAL=$DEPS/pluginval.app/Contents/MacOS/pluginval
CLAP_VALIDATOR=$DEPS/clap-validator-src/target/release/clap-validator
ROOT=$PWD/../..
F=$PWD/fixtures
export CLAP_PATH=$DEPS   # the wrapped VST3 finds the CLAP here at run time
fail=0

[[ -x $H ]] || { echo "FAIL: no test host at $H (run ./spike.sh build)"; exit 2; }
[[ -d $CLAP ]] || { echo "FAIL: no CLAP at $CLAP (run ./spike.sh build)"; exit 2; }
[[ -d $VST3 ]] || { echo "FAIL: no VST3 at $VST3 (run ./spike.sh build)"; exit 2; }
mkdir -p $OUT

run_guarded() {  # run_guarded <logfile> <seconds> <cmd...> : one command with a wall-clock guard
  local log=$1 limit=$2; shift 2
  "$@" > $log 2>&1 & local pid=$! n=0
  while kill -0 $pid 2>/dev/null; do /bin/sleep 1; n=$((n+1)); [[ $n -ge $limit ]] && { kill $pid; echo "FAIL: timeout after ${limit}s ($log)"; return 9; }; done
  wait $pid; return $?
}

# ---- 1. the fixtures -------------------------------------------------------------
rm -f $OUT/s1-clap.jsonl $OUT/s1-vst3.jsonl $OUT/s1-compared-*.txt(N)
/usr/bin/python3 - $F <<'PYMAKE' || exit 2
import os, struct, sys
fix = sys.argv[1]
IXML = b"<BWFXML><IXML_VERSION>1.5</IXML_VERSION></BWFXML> "  # even length; the loader needs an iXML chunk
assert len(IXML) % 2 == 0
SPEC = {  # three fixtures: different lengths (3 s, 1 s, 2 s), one at another sample rate (96 kHz)
    "s1_kick_48k_mono.wav": (48000, 1, 3, [1000, 24001, 77777, 143999]),
    "s1_hat_48k_stereo.wav": (48000, 2, 1, [5, 4410, 47995]),
    "s1_tom_96k_mono.wav": (96000, 1, 2, [12345, 100000, 191999]),
}
def samples(rate, ch, secs, hits):
    n = rate * secs
    data = [[0.0] * n for _ in range(ch)]
    for c in range(ch):
        for i in range(0, n, 97):            # low noise floor so the hash sees every region
            data[c][i] = ((i * 2654435761 + c) % 1000) / 1e6
    for k, h in enumerate(hits):
        data[0][h] = 0.9 - 0.1 * k           # the click, channel 0 only
        if h + 1 < n: data[0][h + 1] = 0.0
    return data
os.makedirs(fix, exist_ok=True)
for name, (rate, ch, secs, hits) in SPEC.items():
    d = samples(rate, ch, secs, hits); n = rate * secs
    body = b"".join(struct.pack("<f", d[c][i]) for i in range(n) for c in range(ch))
    fmt = struct.pack("<HHIIHH", 3, ch, rate, rate * ch * 4, ch * 4, 32)
    riff = b"WAVE" + b"fmt " + struct.pack("<I", len(fmt)) + fmt + b"fact" + struct.pack("<II", 4, n) \
         + b"data" + struct.pack("<I", len(body)) + body + b"iXML" + struct.pack("<I", len(IXML)) + IXML
    open(os.path.join(fix, name), "wb").write(b"RIFF" + struct.pack("<I", len(riff)) + riff)
print("made", len(SPEC), "fixtures in", fix)
PYMAKE

# ---- 2. + 3. the read test and the compare, per path ------------------------------
read_check() {  # read_check <kind>
  local kind=$1 B out
  if [[ $kind = clap ]]; then B=$CLAP; else B=$VST3; fi
  out=$OUT/s1-$kind.jsonl
  DS_ARA_SPIKE_OUT=$out run_guarded $OUT/s1-read-$kind.log 120 $H -$kind $B \
    -file $F/s1_kick_48k_mono.wav $F/s1_hat_48k_stereo.wav $F/s1_tom_96k_mono.wav -test ContentReading
  local rc=$?
  if [[ $rc -ne 0 ]]; then echo "FAIL $kind: test host rc=$rc (out/s1-read-$kind.log)"; return 1; fi
  /usr/bin/python3 - $out $F $kind <<'PYVERIFY'
import json, os, struct, sys
out, fix, label = sys.argv[1], sys.argv[2], sys.argv[3]
NAMES = ["s1_kick_48k_mono.wav", "s1_hat_48k_stereo.wav", "s1_tom_96k_mono.wav"]
def wav_read(path):
    """The script's own read of the WAV file: a RIFF walk, then count, rate, channels, FNV-1a."""
    raw = open(path, "rb").read()
    assert raw[:4] == b"RIFF" and raw[8:12] == b"WAVE", path
    i, rate, ch, data = 12, None, None, None
    while i + 8 <= len(raw):
        cid, size = raw[i:i+4], struct.unpack("<I", raw[i+4:i+8])[0]
        body = raw[i+8:i+8+size]
        if cid == b"fmt ":
            ch = struct.unpack("<H", body[2:4])[0]
            rate = struct.unpack("<I", body[4:8])[0]
        elif cid == b"data":
            data = body
        i += 8 + size + (size & 1)
    assert data is not None and rate and ch, path
    n = len(data) // (4 * ch)
    h = 0xcbf29ce484222325
    for b in data:  # frame-major, channel-minor, f32 LE: the order the plugin hashes
        h = ((h ^ b) * 0x100000001b3) & 0xFFFFFFFFFFFFFFFF
    return dict(sample_count=n, sample_rate=float(rate), channels=ch, fnv64="%016x" % h)
rows = [json.loads(l) for l in open(out) if l.strip()]
reads = {os.path.basename(r["name"]): r for r in rows if r.get("event") == "read"}
bad = 0; compared = 0
for name in NAMES:
    e = wav_read(os.path.join(fix, name)); r = reads.get(name)
    if r is None:
        print("FAIL %s %s: the plugin read nothing" % (label, name)); bad += 1; continue
    compared += 1
    for k in ("sample_count", "sample_rate", "channels", "fnv64"):
        if r[k] != e[k]:
            print("FAIL %s %s: %s plugin=%s expected-from-wav=%s" % (label, name, k, r[k], e[k])); bad += 1
    if r.get("samples_read") != e["sample_count"] or not r.get("read_ok"):
        print("FAIL %s %s: read incomplete" % (label, name)); bad += 1
    if r.get("caller_thread") == r.get("reader_thread"):
        print("FAIL %s %s: the read ran on the host's calling thread" % (label, name)); bad += 1
    print("ok   %s %s: %d samples x %d ch at %d Hz, hash equal to the WAV read, background thread"
          % (label, name, e["sample_count"], e["channels"], int(e["sample_rate"])))
open(os.path.join(os.path.dirname(out), "s1-compared-%s.txt" % label), "w").write(str(compared))
if compared == 0 or bad:
    print("S1 CHECK %s: FAIL (%d sources compared)" % (label, compared)); sys.exit(1)
print("S1 CHECK %s: PASS (%d sources compared)" % (label, compared))
PYVERIFY
  return $?
}

for kind in clap vst3; do
  read_check $kind || fail=1
done

# ---- 4. the validators ------------------------------------------------------------
if [[ -x $PLUGINVAL ]]; then
  run_guarded $OUT/s1-pluginval.log 300 $PLUGINVAL --strictness-level 5 --timeout-ms 120000 $VST3
  if [[ $? -eq 0 ]]; then echo "ok   pluginval: strictness 5 on the wrapped VST3 (out/s1-pluginval.log)"
  else echo "FAIL pluginval: strictness 5 on the wrapped VST3 (out/s1-pluginval.log)"; fail=1; fi
else
  echo "FAIL pluginval: not at $PLUGINVAL (run ./spike.sh deps)"; fail=1
fi
if [[ -x $CLAP_VALIDATOR ]]; then
  run_guarded $OUT/s1-clap-validator.log 300 $CLAP_VALIDATOR validate $CLAP
  if [[ $? -eq 0 ]]; then echo "ok   clap-validator: validate on the CLAP bundle (out/s1-clap-validator.log)"
  else echo "FAIL clap-validator (out/s1-clap-validator.log)"; fail=1; fi
else
  echo "FAIL clap-validator: not at $CLAP_VALIDATOR (run ./spike.sh deps)"; fail=1
fi

# ---- 5. the standalone rule -------------------------------------------------------
if (cd $ROOT && cargo metadata --no-deps --format-version 1) > $OUT/s1-metadata.json 2>$OUT/s1-metadata.err \
   && ! grep -q ds-ara-spike $OUT/s1-metadata.json; then
  echo "ok   standalone: cargo metadata --no-deps at the repo root does not name the spike crate"
else
  echo "FAIL standalone: cargo metadata --no-deps at the repo root (out/s1-metadata.err)"; fail=1
fi
if git -C $ROOT rev-parse --verify -q origin/main >/dev/null; then
  bad=0
  for f in ${(f)"$(git -C $ROOT diff --name-only origin/main...HEAD)"}; do
    if [[ $f == plugins/ara-spike/* || $f == docs/design/daw-plugin/* ]]; then continue; fi
    echo "FAIL standalone: the diff touches $f"; bad=1
  done
  [[ $bad -eq 0 ]] && echo "ok   standalone: the diff touches only plugins/ara-spike/ and docs/design/daw-plugin/"
  [[ $bad -eq 0 ]] || fail=1
else
  echo "FAIL standalone: no origin/main next to this head"; fail=1
fi

# ---- the verdict -------------------------------------------------------------------
total=0
for c in $OUT/s1-compared-clap.txt $OUT/s1-compared-vst3.txt; do
  [[ -f $c ]] && total=$(( total + $(cat $c) ))
done
if [[ $total -eq 0 ]]; then echo "FAIL: compared 0 sources"; fail=1; fi
if [[ $fail -eq 0 ]]; then
  echo "S1 CHECK: PASS ($total source loads compared across the CLAP and the wrapped VST3, pluginval, clap-validator, standalone rule)"
else
  echo "S1 CHECK: FAIL"
fi
exit $fail
