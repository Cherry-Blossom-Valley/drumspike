"""P4-19: the roll fill of the app measured where no yardstick reference exists.

    uv run --with numpy --with soundfile --with mido python tools/roll_fill.py heldout   # Session E, Summer Nights
    uv run --with numpy --with soundfile --with mido python tools/roll_fill.py kit       # the DRT kit, fill off and on
    uv run --with numpy --with soundfile --with mido python tools/roll_fill.py all
    options: --samples DIR (the sessions, default ~/.cache/drumspike/sessions), --drt DIR (the DRT kit, default
    ~/.cache/drumspike/drt-testkit)

`heldout`: the rule was fitted on the seven yardstick songs; the two other sessions of the folder have no usable snare
reference. Each snare top mic there is analysed by the app (`service_tool cli run`, default settings: the fill on);
every added hit (`"roll": true`) is tested as in P4-18: a stroke on the second snare mic (`s btm`, `s bot`: a rise of
10 dB or more, `firm_fixes.rise_db`) and an onset of a sample layer within 4 ms (the layer's latency taken from the
track's normal hits, `firm_fixes.layer_offset`). Summer Nights has each mic in four files (`_01`, `_02`, `_04`,
`_05`); each is its own track with the bottom mic and the layers of the same number. The four files are the same audio
(`s bot_02` is `s bot_01` shifted by 0.2023 s, P4-19 verifier), so their added hits are the same strokes four times: count
the distinct strokes (8 in P4-19), not the rows.

`kit`: the nine DRT kit files analysed by the app with `DRUMSPIKE_ROLL_RULE=0` and with the default; found and false
against the kit's ground truth (`miss_causes.score_events`).

Needs the release `service_tool` (or `DRUMSPIKE_CALIB_TOOL`). Outputs:
`~/.cache/drumspike/roll-fill/` (`heldout.json`, `kit.json`, `report.md`). The sessions are read only; `--samples`
also points `fix_metric.audio_path` (which `firm_fixes.read_stem` uses) at that folder for this process.
"""
import argparse
import json
import os
import shutil
import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
import firm_fixes as ff  # noqa: E402
import fix_metric as fm  # noqa: E402
import miss_causes as mc  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
OUT = Path.home() / ".cache" / "drumspike" / "roll-fill"
SAMPLES = Path.home() / ".cache" / "drumspike" / "samples"
DRT = Path.home() / ".cache" / "drumspike" / "drt-testkit"
MIN_CONFIRMED = 0.90   # the controller's bar for the fill on by default

# (session, folder, track stem, witness stem, layer stems)
HELDOUT = [("Session E", "Session E", "s top-AtA2", "s btm-AtA2",
            ["s sample savior-AtA2", "s sample sd3-AtA2", "s trigger-AtA2"])]
HELDOUT += [("Session I", "Session I", f"s top_{k}", f"s bot_{k}",
             [f"s samp {n}_{k}" for n in (1, 2, 3)]) for k in ("01", "02", "04", "05")]


# ---------------------------------------------------------------- the tests of an added hit (pure; the tests cover these)

def layer_hits(normal, layers, tol=ff.LAYER_TOL_S):
    """The layer onsets moved onto the track's time: each layer's latency is the median signed distance of its nearest
    onset to the track's normal hits (`firm_fixes.layer_offset`). Returns {layer: sorted onsets on the track's time}."""
    return {k: sorted(t - ff.layer_offset(normal, v) for t in v) for k, v in layers.items()}


def confirm(added, wit_x, sr, layers, tol=ff.LAYER_TOL_S, rise_min=ff.RISE_MIN_DB):
    """Per added hit time: the witness rise (None without a witness), whether it shows a stroke there, whether a layer
    has an onset within `tol`, and either."""
    rows = []
    for t in added:
        rw = ff.rise_db(wit_x, sr, t) if wit_x is not None else None
        wit = rw is not None and rw >= rise_min
        lay = any(len(v) and abs(ff.nearest(v, t)) <= tol for v in layers.values())
        rows.append({"t": t, "rise_wit": rw, "witness": bool(wit), "layer": bool(lay), "confirmed": bool(wit or lay)})
    return rows


def summary(rows):
    n = len(rows)
    c = {k: sum(1 for r in rows if r[k]) for k in ("witness", "layer", "confirmed")}
    return {"added": n, **c, "share": (c["confirmed"] / n) if n else None}


# ---------------------------------------------------------------- app runs

def run_track(out, ident, audio, role, env_extra=None):
    """One app analysis of one file (`miss_causes.run_item`, the fill as configured; `ident` a safe name): the
    events."""
    tool, model = mc.find_tool(), "none".resolve()
    if not tool.is_file():
        raise SystemExit(f"need the release service_tool ({tool}) and the model ({model})")
    # the app names the track after the file (spaces become `_`): a link with a safe name keeps `events_get` simple
    link = Path(out) / "audio" / f"{ident}.wav"
    link.parent.mkdir(parents=True, exist_ok=True)
    if link.is_symlink() or link.exists():
        link.unlink()
    link.symlink_to(Path(audio))
    audio = link
    old = {k: os.environ.get(k) for k in (env_extra or {})}
    os.environ.update(env_extra or {})
    try:
        fm.wait_for_room()
        fm.LOCK.parent.mkdir(parents=True, exist_ok=True)
        fm.LOCK.write_text(f"roll_fill {ident}\n")
        try:
            wd = mc.run_item(tool, out, {"id": ident, "audio": str(audio), "role": role, "note": mc.ROLE_NOTE[role]},
                             "model", model)
        finally:
            fm.LOCK.unlink(missing_ok=True)
    finally:
        for k, v in old.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v
    return json.loads((wd / "events.json").read_text())["events"]


def use_samples(path):
    """Read the sessions from `path`: this module and `fix_metric.audio_path` (for `firm_fixes.read_stem`)."""
    global SAMPLES
    SAMPLES = Path(path)
    fm.audio_path = lambda cfg, stem: SAMPLES / cfg["dir"] / "Audio Files" / f"{stem}.wav"


def audio_of(folder, stem):
    """The mono file of `stem` (a link for a stereo pair is not needed: the app reads the L file of a pair as mono)."""
    p = SAMPLES / folder / "Audio Files" / f"{stem}.wav"
    return p if p.is_file() else None


def do_heldout(out, log=print):
    out = Path(out)
    res = []
    for song, folder, stem, wit, layers in HELDOUT:
        audio = audio_of(folder, stem)
        if audio is None:
            log(f"{song} {stem}: no file")
            continue
        ev = run_track(out / "heldout", fm.track_name(song) + "_" + fm.track_name(stem), audio, "snare")
        cfg = {"dir": folder}
        x, sr = ff.read_stem(cfg, stem)
        wx, wsr = ff.read_stem(cfg, wit)
        if wx is not None and wsr != sr:
            wx = None
        normal = [e["t"] for e in ev if not e.get("roll")]
        added = [e["t"] for e in ev if e.get("roll")]
        lay = layer_hits(normal, ff.layer_times(cfg, layers))
        rows = confirm(added, wx, sr, lay)
        s = summary(rows)
        res.append({"song": song, "track": stem, "witness": wit if wx is not None else None, "layers": sorted(lay),
                    "events": len(ev), **s, "rows": rows})
        log(f"{song} {stem}: {len(ev)} hits, {s['added']} added, witness {s['witness']}, layer {s['layer']}, "
            f"confirmed {s['confirmed']}")
    (out / "heldout.json").write_text(json.dumps(res))
    shutil.rmtree(out / "heldout" / "run-home", ignore_errors=True)
    shutil.rmtree(out / "heldout" / "run-projects", ignore_errors=True)
    return res


def do_kit(out, drt=None, log=print):
    out = Path(out)
    rows = []
    for it in mc.drt_items(Path(drt or DRT)):
        r = {"file": it["file"], "role": it["role"]}
        for name, env in (("off", {"DRUMSPIKE_ROLL_RULE": "0"}), ("on", {})):
            ev = run_track(out / "kit" / name, it["id"].replace("/", "_"), it["audio"], it["role"], env)
            found, false, _ = mc.score_events(it, [e["t"] for e in ev])
            r[name] = {"found": sum(found.values()), "false": false, "found_by": found,
                       "added": sum(1 for e in ev if e.get("roll"))}
        r["targets"] = sum(1 for g in it["gt"] if g["group"] != "bleed")
        rows.append(r)
        log(f"{it['file']}: found {r['off']['found']} to {r['on']['found']} of {r['targets']}, "
            f"false {r['off']['false']} to {r['on']['false']}")
    (out / "kit.json").write_text(json.dumps(rows))
    for name in ("off", "on"):
        shutil.rmtree(out / "kit" / name / "run-home", ignore_errors=True)
        shutil.rmtree(out / "kit" / name / "run-projects", ignore_errors=True)
    return rows


def do_report(out, log=print):
    out = Path(out)
    lines = []
    if (out / "heldout.json").is_file():
        res = json.loads((out / "heldout.json").read_text())
        body = [[r["song"], r["track"], r["events"], r["added"], r["witness"], r["layer"], r["confirmed"]] for r in res]
        tot = summary([x for r in res for x in r["rows"]])
        body.append(["**all**", "", sum(r["events"] for r in res), tot["added"], tot["witness"], tot["layer"],
                     tot["confirmed"]])
        share = "-" if tot["share"] is None else f"{100 * tot['share']:.0f} %"
        lines += ["### Held out: the added hits and their confirmation", "",
                  ff.md_table(["session", "track", "hits", "added (roll)", "stroke on the bottom mic", "layer onset",
                               "confirmed (either)"], body), "",
                  f"Confirmed: {share} (the bar for default on: {100 * MIN_CONFIRMED:.0f} %).", ""]
    if (out / "kit.json").is_file():
        rows = json.loads((out / "kit.json").read_text())
        body = [[r["file"], r["role"], r["targets"], f"{r['off']['found']} / {r['off']['false']}",
                 f"{r['on']['found']} / {r['on']['false']}", r["on"]["added"]] for r in rows]
        lines += ["### DRT kit: the app with the fill off and on (found / false)", "",
                  ff.md_table(["file", "role", "targets", "off", "on", "roll hits"], body), ""]
    (out / "report.md").write_text("\n".join(lines))
    log("\n".join(lines))


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("cmd", choices=["heldout", "kit", "report", "all"])
    ap.add_argument("--out", default=str(OUT))
    ap.add_argument("--samples", default=str(SAMPLES))
    ap.add_argument("--drt", default=str(DRT))
    a = ap.parse_args(argv)
    use_samples(a.samples)
    Path(a.out).mkdir(parents=True, exist_ok=True)
    if a.cmd in ("heldout", "all"):
        do_heldout(a.out)
    if a.cmd in ("kit", "all"):
        do_kit(a.out, a.drt)
    do_report(a.out)
    return 0


if __name__ == "__main__":
    sys.exit(main())
