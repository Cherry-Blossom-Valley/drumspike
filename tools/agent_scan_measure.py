"""#412: measure `DSAgent.scan` on real sessions, through the page helper and the app's bridge.

    uv run python tools/agent_scan_measure.py --session <folder> [--session <folder> ...]
        [--markdown <path>] [--keep]

`--session` is a recording folder with an `Audio Files` folder of WAV files (a Pro Tools
session folder). The tool builds a project with ALL tracks of the session (rule a needs the
tom and hat hits), opens it in the page on `ds-bridge`, analyses it and calls the page
helper (`DSAgent.scan`, read-only) on every kick and snare track. It prints per track: the
hits, the detector candidates, the listed candidates on the untouched track and the cut
candidates left out as bleed and as tail. Then the fault injection: every tenth hit of the
track is written away through `flushEvents` (the page's own write path, no edit history), the
scan runs again, and the tool counts the removed hits that are listed and the misses with
their cause (no candidate within 30 ms, or a candidate present but not listed). It exits 1
when a removed hit that has a candidate is not listed, and when it measured 0 tracks.

`--markdown <path>` appends an aggregate table (one row per session, in the order given, no
file or session names) to the report.

The bridge binary is `target/debug/ds-bridge` (or $DRUMSPIKE_BRIDGE), built with
`cargo build -p drumspike-devtools --bin ds-bridge`; the tool refuses a binary older than the
Rust sources. Chromium comes from the repo's Playwright install.
"""
import argparse
import json
import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
READY = "typeof S !== 'undefined' && typeof B !== 'undefined' && B.app === true && S.loaded > 0"
CAND_WINDOW_MS = 30   # a removed hit "has a candidate" when one stands within this window
TAIL_AFTER = 0.060


ROLE_NOTE = {"kick": 36, "snare": 38, "tom": 45, "hat": 42, "cymbal": 49, "other": 44}


def role_of(name):
    """The role of a standard drum track from its file stem: the naming scheme of the sessions."""
    n = name.lower()
    if n.startswith("k"):
        return "kick"
    if n.startswith("s"):
        return "snare"
    if n.startswith("hh"):
        return "hat"
    if n.startswith("t"):
        return "tom"
    if n.startswith("oh") or n.startswith("r"):
        return "cymbal"
    return "other"


def session_wavs(folder):
    """[(stem, path)] of every WAV of the session, sorted (all tracks: rule a needs the tom and hat hits)."""
    audio = Path(folder) / "Audio Files"
    if not audio.is_dir():
        raise SystemExit(f"FAIL: no Audio Files folder in {folder}")
    return sorted((p.stem, p) for p in audio.glob("*.wav"))


def build_project(folder, parent, name):
    """A project with every WAV of the session as a track (referenced), via the selftest builder."""
    import importlib.util
    spec = importlib.util.spec_from_file_location("ds_make_project", ROOT / "apps" / "desktop" / "selftest" / "make_project.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    tracks = [(stem, str(path), role_of(stem), ROLE_NOTE[role_of(stem)]) for stem, path in session_wavs(folder)]
    return Path(mod.build_project(parent, name, tracks, mode="referenced"))


def bridge_binary():
    exe = Path(os.environ.get("DRUMSPIKE_BRIDGE") or ROOT / "target" / "debug" / "ds-bridge")
    if not exe.is_file():
        raise SystemExit(f"FAIL: no ds-bridge at {exe} (cargo build -p drumspike-devtools --bin ds-bridge)")
    built = exe.stat().st_mtime
    for pat in ("crates/core/**/*.rs", "crates/app/**/*.rs", "crates/devtools/**/*.rs"):
        for src in ROOT.glob(pat):
            if src.stat().st_mtime > built:
                raise SystemExit(f"FAIL: stale ds-bridge (newest source {src}): rebuild it")
    return exe


def start_bridge(home):
    env = dict(os.environ, DRUMSPIKE_HOME=str(home), DRUMSPIKE_PROJECTS=str(home / "projects"),
               DRUMSPIKE_AUDIO="null", DRUMSPIKE_MASTER_GAIN="0",
               DRUMSPIKE_WINDOW_ENDPOINT="1", DRUMSPIKE_NO_WARM_PAGE="1")
    exe = bridge_binary()
    proc = subprocess.Popen([str(exe), "--static", str(ROOT / "static"), "--exit-on-stdin-eof"],
                            cwd=ROOT, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                            stderr=subprocess.STDOUT, text=True, env=env)
    import re
    line = proc.stdout.readline()
    m = re.search(r"http://127\.0\.0\.1:\d+", line)
    if not m:
        proc.kill()
        raise SystemExit(f"FAIL: ds-bridge did not start: {line!r} {proc.stdout.read()}")
    return proc, m.group(0)


def ev(page, js, arg=None):
    return page.evaluate(js, arg)


def open_project(page, folder):
    """File > Open with the native dialog answered by __DS_PICK__; waits for the lanes."""
    want = os.path.realpath(str(folder))
    dirty = page.evaluate("""async () => { if (!S.sid) return false; try {
        return (await B.projectCleanCheck(S.sid)).dirty; } catch (e) { return false; } }""")
    page.evaluate("f => { window.__DS_PICK__ = f; }", str(folder))
    page.evaluate("async () => { await fetch('/invoke/selftest_menu', {method: 'POST', headers: {'Content-Type': 'application/json'},"
                  " body: JSON.stringify({id: 'open'})}); }")
    if dirty:
        end = time.time() + 10
        while time.time() < end:
            clicked = page.evaluate("""() => { const b = [...document.querySelectorAll('#confirmActions button')]
                .find(b => b.dataset.act === 'skip'); if (b) { b.click(); return true; } return false; }""")
            if clicked:
                break
            got = page.evaluate("async () => S.sid ? (await B.projectGet(S.sid).catch(() => null))?.folder ?? null : null")
            if got and os.path.realpath(got) == want:
                break
            time.sleep(0.1)
    end = time.time() + 120
    while time.time() < end:
        got = page.evaluate("async () => S.sid ? (await B.projectGet(S.sid).catch(() => null))?.folder ?? null : null")
        if got and os.path.realpath(got) == want:
            break
        time.sleep(0.2)
    else:
        raise SystemExit(f"FAIL: the page did not open {want} in 120 s")
    page.wait_for_function("() => S.loaded > 0 && S.order.length > 0 && S.order.every(n => n in S.peaks)", timeout=120_000)


def analyse(page):
    """The analyse-all button; a project that already carries events asks before the re-analyze."""
    page.click("#analyzeAllBtn")
    try:
        page.locator("#confirmActions button[data-act='go']").click(timeout=3000)
    except Exception:
        pass
    page.wait_for_function("() => document.querySelector('#progressText')?.textContent === 'analysis done'", timeout=1_800_000)
    page.wait_for_function("() => !analysisRunning", timeout=120_000)


def scan(page, track):
    return page.evaluate("async t => await DSAgent.scan(t)", track)


def candidate_times(page, track):
    return page.evaluate("async t => (await B.histograms(S.sid, t, S.events[t])).detection.candidates.map(c => c[0])", track)


def breakdown(page, track):
    """The listed candidates of the untouched track, by needed sensitivity and by distance to the
    nearest hit of any track (the decision rule's report of what the rest is)."""
    return page.evaluate("""async t => {
      const h = await B.histograms(S.sid, t, S.events[t]);
      const r = await DSAgent.scan(t);
      const listed = new Map(r.candidates.map(c => [Math.round(c.time * 1e6), c]));
      const hits = [];
      for (const n of Object.keys(S.tracks)) for (const e of S.events[n] || []) hits.push(e.t);
      const sens = { "<=60": 0, "61-80": 0, "81-100": 0, ">100": 0 };
      const dist = { "<=10ms": 0, "10-30ms": 0, "30-60ms": 0, "60-200ms": 0, ">200ms": 0 };
      for (const c of h.detection.candidates) {
        if (!listed.has(Math.round(c[0] * 1e6))) continue;
        const v = c[3];
        sens[v <= 60 ? "<=60" : v <= 80 ? "61-80" : v <= 100 ? "81-100" : ">100"]++;
        let d = Infinity;
        for (const x of hits) d = Math.min(d, Math.abs(x - c[0]));
        dist[d <= 0.01 ? "<=10ms" : d <= 0.03 ? "10-30ms" : d <= 0.06 ? "30-60ms" : d <= 0.2 ? "60-200ms" : ">200ms"]++;
      }
      return { sens, dist };
    }""", track)


def measure_session(page, folder, tmp):
    """One session: the untouched scan per kick and snare track, then the fault injection.

    Returns (name, rows, ok): one row per measured track with the printed numbers and the
    misses with their cause; ok is False when a removed hit that has a candidate is not listed."""
    name = "Session " + chr(64 + measure_session.n)
    measure_session.n += 1
    proj = build_project(folder, tmp, name.replace(" ", "-"))
    open_project(page, proj)
    analyse(page)
    rows, ok = [], True
    for track in page.evaluate("() => S.order"):
        role = page.evaluate("t => S.tracks[t].role", track)
        if role not in ("kick", "snare"):
            continue
        times = page.evaluate("t => S.events[t].map(e => e.t)", track)
        if not times:
            continue
        r = scan(page, track)
        cands = candidate_times(page, track)
        row = {"track": role, "hits": len(times), "candidates": len(cands),
               "listed": len(r["candidates"]), "bleed": r["left_out"]["bleed"], "tail": r["left_out"]["tail"],
               "removed": 0, "listed_removed": 0, "no_candidate": 0, "not_listed": 0}
        bd = breakdown(page, track)
        row["sens"] = bd["sens"]
        row["dist"] = bd["dist"]
        # fault injection: every tenth hit written away through the page's own write path
        keep = page.evaluate("""t => { const keep = S.events[t].map((_, i) => i % 10 !== 9);
                                          S.events[t] = S.events[t].filter((_, i) => i % 10 !== 9);
                                          return keep; }""", track)
        removed = [t for t, k in zip(times, keep) if not k]
        page.evaluate("async () => { await flushEvents(); }")
        after = scan(page, track)
        listed_t = [c["time"] for c in after["candidates"]]
        for rt in removed:
            row["removed"] += 1
            has = [c for c in cands if abs(c - rt) <= CAND_WINDOW_MS / 1000]
            listed = any(abs(c - rt) <= CAND_WINDOW_MS / 1000 for c in listed_t)
            if listed:
                row["listed_removed"] += 1
            elif not has:
                row["no_candidate"] += 1      # e.g. the second stroke of a flam: no candidate at all
            else:
                row["not_listed"] += 1        # a candidate stands there and the scan does not list it
                ok = False
        rows.append(row)
        print(f"{name} {track} ({role}): hits {row['hits']}, candidates {row['candidates']}, "
              f"listed {row['listed']} ({100 * row['listed'] / row['hits']:.1f}% of the hits), "
              f"left_out bleed {row['bleed']} tail {row['tail']}")
        print(f"    listed by needed sensitivity: {json.dumps(row['sens'])}")
        print(f"    listed by distance to the nearest hit of any track: {json.dumps(row['dist'])}")
        print(f"    injection: removed {row['removed']}, listed {row['listed_removed']}, "
              f"not listed {row['not_listed']} (candidate present), no candidate {row['no_candidate']}")
        # write the hits back, so the next track's untouched scan sees every bleed anchor again
        page.evaluate("""({t, evs}) => { S.events[t] = evs; return flushEvents(); }""",
                       {"t": track, "evs": [{"t": t} for t in times]})
    return name, rows, ok


measure_session.n = 1


def markdown_table(all_rows):
    """The aggregate table: one row per session, no file or session names."""
    out = ["| session | track | hits | candidates | listed | left out bleed | left out tail | removed | listed | not listed (candidate present) | no candidate |",
           "|---|---|---|---|---|---|---|---|---|---|---|"]
    for name, rows, _ok in all_rows:
        for r in rows:
            out.append(f"| {name} | {r['track']} | {r['hits']} | {r['candidates']} | {r['listed']} | "
                       f"{r['bleed']} | {r['tail']} | {r['removed']} | {r['listed_removed']} | "
                       f"{r['not_listed']} | {r['no_candidate']} |")
    return "\n".join(out)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--session", action="append", required=True, help="a recording folder with Audio Files (repeatable)")
    ap.add_argument("--markdown", help="append the aggregate table to this markdown file")
    ap.add_argument("--keep", action="store_true", help="keep the built projects and the bridge home")
    a = ap.parse_args(argv)
    from playwright.sync_api import sync_playwright

    tmp = Path(tempfile.mkdtemp(prefix="agent-scan-measure-"))
    proc = None
    all_rows, fail = [], False
    try:
        proc, url = start_bridge(tmp / "home")
        with sync_playwright() as pw:
            browser = pw.chromium.launch()
            page = browser.new_page()
            page.goto(url + "/", timeout=180_000)
            page.wait_for_function(READY, timeout=180_000)
            for folder in a.session:
                measure_session.n = len(all_rows) + 1
                name, rows, ok = measure_session(page, folder, tmp)
                all_rows.append((name, rows, ok))
                fail = fail or not ok
            browser.close()
    finally:
        if proc:
            proc.terminate()
            proc.wait(10)
        if not a.keep:
            import shutil
            shutil.rmtree(tmp, ignore_errors=True)
        else:
            print(f"kept: {tmp}")
    if not any(rows for _n, rows, _o in all_rows):
        print("FAIL: measured 0 tracks")
        return 1
    print(markdown_table(all_rows))
    if a.markdown:
        with open(a.markdown, "a") as f:
            f.write(markdown_table(all_rows) + "\n")
    if fail:
        print("FAIL: a removed hit that has a candidate is not listed")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main() or 0)
