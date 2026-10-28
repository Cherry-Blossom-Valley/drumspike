"""#209: the audio-spike check on the real sessions (ticket 209, gate 3).

    uv run --with numpy --with soundfile python tools/audio_spikes.py check
    uv run --with numpy --with soundfile python tools/audio_spikes.py check \\
        --songs "Session A,Session B,Session C" --tom t2 \\
        --report docs/migration/reports/audio-spikes.md

Default: the first three songs of `fix_metric.songs()` that have a kick mic, a snare mic and a
tom close mic (`--songs` a comma list, `--tom` the tom mic name). Per session the three mics
become tracks of a scratch project (`service_tool`, as `tools/fix_metric.py run`), the app
analyses them, and `export_all` with `formats: ["midi", "spikes", "audio"]` writes
`<track>_spike.wav` and `<track>_audio_spike.wav` into the project's Exports/. The independent
checker (`tools/check_exports.py`, properties AS1-AS3) verifies the audio file: every maximal
nonzero run starts at a hit sample — the snippet starts where the blip of the same hit does,
0 differences — at least one hit is checked (a track with hits and 0 checked snippets is a
FAIL, the gate's rule), and the file holds the source's mono mix at the hit samples. The
exported files stay in `~/.cache/drumspike/audio-spikes/<song>/` for the eye (the coordinator
looks at a few snippets), with the checker's report in `check.json`. The report at --report
(default docs/migration/reports/audio-spikes.md) lists the verdict, the per-song counts and
the snippet length distribution per drum. Exit 1 on any problem or 0 hits checked.
"""
import argparse
import datetime
import json
import os
import platform
import subprocess
import sys
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
import check_exports  # noqa: E402

OUT = Path.home() / ".cache" / "drumspike" / "audio-spikes"
NOTES = {"kick": 36, "snare": 38, "tom": 47}


def songs(limit=3, names=None):
    """[(song, cfg, {role: (mic, stem)})] of the sessions to check: the first `limit` songs with
    a kick, a snare and a tom close mic, in `fix_metric.songs()` order (`names` picks exactly)."""
    import fix_metric
    all_songs = fix_metric.songs()
    if names:
        missing = [n for n in names if n not in all_songs]
        if missing:
            raise SystemExit(f"unknown songs {missing}; the refs.json songs are {sorted(all_songs)}")
        picked = names
    else:
        picked = []
        for name, cfg in all_songs.items():
            if mics_of(cfg):
                picked.append(name)
            if len(picked) == limit:
                break
        if len(picked) < limit:
            raise SystemExit(f"only {len(picked)} songs with a kick, a snare and a tom close mic "
                             f"({sorted(all_songs)})")
    out = []
    for name in picked:
        cfg = all_songs[name]
        out.append((name, cfg, mics_of(cfg)))
    return out


def mics_of(cfg):
    """{role: (mic, stem)} of one song: the first kick mic, the first snare mic, the first tom
    close mic that the session does not exclude."""
    excluded = cfg.get("excluded") or set()
    picked = {}
    for mic, (stem, role) in cfg["mics"].items():
        if mic in excluded:
            continue
        if role not in picked:
            picked[role] = (mic, stem)
    if not ("kick" in picked and "snare" in picked and "toms" in picked):
        return {}
    return {"kick": picked["kick"], "snare": picked["snare"], "tom": picked["toms"]}


def export_song(tool, work, name, cfg, mics):
    """The scratch project of one song, exported with the blip and the audio spikes; the
    project folder. Raises on any step error, like fix_metric's run_song."""
    import fix_metric
    pdir = work / "p" / "audio-spikes"
    if pdir.exists():
        import shutil
        shutil.rmtree(pdir)
    steps = [{"command": "project_new", "args": {"name": "audio-spikes", "dir": str(work / "p")}},
             {"command": "tracks_add",
              "args": {"sid": "$sid", "paths": [str(fix_metric.audio_path(cfg, stem)) for _, stem in mics.values()],
                       "roles": [role for role in mics], "notes": [NOTES[role] for role in mics]},
              "wait": True},
             {"command": "analyze_start", "args": {"sid": "$sid"}, "wait": True},
             # the app CLI's `run` never saves (crates/app/src/cli.rs run_script): the checker
             # reads the document, so save the tracks and the analysis before the export
             {"command": "project_save", "args": {"sid": "$sid"}},
             {"command": "export_all", "args": {"sid": "$sid", "formats": ["midi", "spikes", "audio"]}}]
    script = work / f"{fix_metric.track_name(name)}.json"
    script.write_text(json.dumps(steps))
    env = {**os.environ, "DRUMSPIKE_HOME": str(work / "h")}
    r = subprocess.run([str(tool), "cli", "run", str(script)], capture_output=True, text=True,
                       env=env, stdin=subprocess.DEVNULL)
    rows = [json.loads(x) for x in r.stdout.splitlines() if x.strip()]
    bad = [x for x in rows if not x.get("ok", False)]
    if r.returncode != 0 or bad or len(rows) != len(steps):
        raise RuntimeError(f"{name}: exit {r.returncode} {bad[:1]} {r.stderr[-400:]}")
    add = rows[1]["result"]
    if len(add.get("added", [])) != len(mics) or add.get("rejected"):
        raise RuntimeError(f"{name}: tracks_add added {len(add.get('added', []))} of {len(mics)} "
                           f"tracks: {add.get('rejected', [])[:2]}")
    return pdir, rows[-1]["result"]


def distribution(lengths):
    """The distribution row of a drum's snippet lengths (samples), or None without any."""
    if not lengths:
        return None
    a = np.asarray(lengths)
    return {"n": int(len(a)), "min": int(a.min()), "p50": int(np.percentile(a, 50)),
            "mean": round(float(a.mean()), 1), "p90": int(np.percentile(a, 90)), "max": int(a.max())}


def report_md(rows, per_drum, out_dir, report):
    """The report: the verdict, the per-song table, the length distribution per drum."""
    import fix_metric
    try:
        head = subprocess.run(["git", "rev-parse", "HEAD"], cwd=ROOT, capture_output=True,
                              text=True).stdout.strip()
    except OSError:
        head = "?"
    bad = [(name, p) for name, rep, _ in rows for p in rep["problems"]] + \
          [(n, p) for name, rep, _ in rows for n, tr in rep["tracks"].items() for p in tr["problems"]]
    checked_any = any(tr.get("audio", {}).get("checked", 0) > 0
                      for _, rep, _ in rows for tr in rep["tracks"].values())
    verdict = "PASS" if not bad and checked_any else "FAIL"
    lines = ["# Audio spikes: the real-sessions check (#209)", "",
             f"Run {datetime.datetime.now(datetime.timezone.utc):%Y-%m-%d %H:%M UTC} on {platform.node()} "
             f"({platform.machine()}), main head {head or '?'}, the checker's properties AS1-AS3 "
             f"(`tools/check_exports.py`). The blip and the audio spikes of the kick mic, the "
             f"snare mic and one tom close mic of each session, exported by the app "
             f"(`export_all`, `formats: [\"spikes\", \"audio\"]`); the exports stay under "
             f"`{out_dir}`.", ""]
    if verdict == "PASS":
        lines += ["## Verdict", "", "PASS. Every snippet starts at the hit sample of its blip "
                  "(0 differences), at least one hit per track is checked, and the hit samples "
                  "hold the source's mono mix.", ""]
    else:
        lines += ["## Verdict", "", "FAIL.", ""]
        lines += [f"- {n}: {p}" for n, p in bad]
        if not bad:
            lines += ["- no track checked any hit: the exports hold 0 measurable snippets "
                      "(audio missing, a silent export, or the analysis did not complete)"]
        lines += [""]
    lines += ["## Per song and drum", "",
              "| song | drum | mic | hits | snippets | differences | hits checked |",
              "|---|---|---|---|---|---|---|"]
    for name, rep, mics in rows:
        for role, (mic, _) in mics.items():
            tr = rep["tracks"].get(fix_metric.track_name(mics[role][1]), {})
            a = tr.get("audio", {})
            lines.append(f"| {name} | {role} | {mic} | {tr.get('hits', '?')} | {a.get('runs', '?')} | "
                         f"{a.get('differences', '?')} | {a.get('checked', '?')} |")
    lines += ["", "## Snippet length distribution per drum (samples)", "",
              "| drum | n | min | p50 | mean | p90 | max |", "|---|---|---|---|---|---|---|"]
    for role in ("kick", "snare", "tom"):
        d = distribution(per_drum.get(role, []))
        lines.append(f"| {role} | " + " | ".join(str(d[k]) if d else "-" for k in
                                                 ("n", "min", "p50", "mean", "p90", "max")) + " |")
    lines += ["", "A length is the nonzero run of one observable snippet: the source's own audio "
              "from the hit sample to the fade-out (about 1.5 waveform periods, clamped to "
              "2-30 ms, the 10 ms default without a period, cut at the next hit). Hits whose "
              "snippet merged into the previous one (cut at the next hit) or that are silent at "
              "the hit are not measurable and not in the distribution.", ""]
    Path(report).write_text("\n".join(lines))


def do_check(args):
    """The 3 real sessions: export, check, report. Exit 1 on any problem or 0 hits checked."""
    tool = Path(args.tool or os.environ.get("DRUMSPIKE_CALIB_TOOL")
                or ROOT / "target" / "release" / "service_tool")
    if not tool.is_file():
        raise SystemExit(f"need the release service_tool ({tool})")
    out_dir = Path(args.out or OUT)
    out_dir.mkdir(parents=True, exist_ok=True)
    names = [s.strip() for s in args.songs.split(",")] if args.songs else None
    rows, per_drum = [], {}
    for name, cfg, mics in songs(names=names):
        if args.tom:
            if args.tom not in cfg["mics"]:
                raise SystemExit(f"{name}: no mic {args.tom!r} (the session's mics: "
                                 f"{sorted(cfg['mics'])})")
            mics["tom"] = (args.tom, cfg["mics"][args.tom][0])
        work = out_dir / "scratch" / fix_track(name)
        work.mkdir(parents=True, exist_ok=True)
        pdir, ans = export_song(tool, work, name, cfg, mics)
        rep = check_exports.check_project(pdir, answer=ans)
        (work / "check.json").write_text(json.dumps(rep, indent=1))
        for role in ("kick", "snare", "tom"):
            tr = rep["tracks"].get(fix_track(mics[role][1]), {})
            per_drum.setdefault(role, []).extend(tr.get("audio", {}).get("lengths", []))
        rows.append((name, rep, mics))
        print(f"checked {name}: {sum(len(tr.get('audio', {}).get('lengths', [])) for tr in rep['tracks'].values())} "
              f"measurable snippets")
    report = args.report or str(ROOT / "docs" / "migration" / "reports" / "audio-spikes.md")
    if report != "-":
        Path(report).parent.mkdir(parents=True, exist_ok=True)
        report_md(rows, per_drum, out_dir, report)
        print(f"report: {report}")
    checked_any = any(tr.get("audio", {}).get("checked", 0) > 0
                      for _, rep, _ in rows for tr in rep["tracks"].values())
    bad = any(rep["problems"] or any(tr["problems"] for tr in rep["tracks"].values())
              for _, rep, _ in rows)
    if bad or not checked_any:
        raise SystemExit(1)


def fix_track(stem):
    """The app's track name of a file, via fix_metric (the same rule)."""
    import fix_metric
    return fix_metric.track_name(stem)


def main(argv=None):
    ap = argparse.ArgumentParser(description=(__doc__ or "").splitlines()[0])
    ap.add_argument("cmd", choices=["check"])
    ap.add_argument("--songs", default=None, help="comma list; default the first 3 usable songs")
    ap.add_argument("--tom", default=None, help="the tom close mic; default the first")
    ap.add_argument("--tool", default=None, help="the service_tool (default: the release build)")
    ap.add_argument("--out", default=None, help=f"output dir (default {OUT})")
    ap.add_argument("--report", default=None,
                    help="the report path ('-' for none; default docs/migration/reports/audio-spikes.md)")
    args = ap.parse_args(argv)
    if args.cmd == "check":
        do_check(args)


if __name__ == "__main__":
    main()
