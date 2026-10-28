"""P4-7b step 5: the per-hit alignment of the one-shot layer against the printed sample tracks of the three
real sessions (measure only).

    uv run python tools/layer_eval.py run      [--out DIR] [--polarity auto|normal] [--tool service_tool]
    uv run python tools/layer_eval.py score    [--out DIR]

The sessions (~/.cache/drumspike/sessions, read only; the songs of the P4-6b `refs.json`) hold sample tracks
that the mixing engineer printed from a sampler next to the close mics: `k samp*` / `k sampl*` next to `k in`,
`s samp*` / `s sampl*` next to `s top` (trigger and reverb prints left out). Per printed track:

1. the app analyses the close mic (the release `service_tool run`: the app's analysis with the model);
2. a one-shot is cut from the printed track at a loud, isolated hit (10 ms before its transient to 300 ms
   after, a 20 ms fade at the end): the sampler's own sample, so the layer can match the print;
3. the app sets it as the layer of the close mic (polarity auto or normal, align on, fixed curve), answers
   `track_layer_align` and renders `Exports/<mic>_layer.wav`;
4. score: per hit of the app with a printed transient, the onset position the app chose (`round(t * sr) +
   lag`) against the onset position of the one-shot in the print (the cross-correlation peak of the same
   attack template in the printed track, sub-sample), in samples; the polarity of the app (the majority)
   against the print's polarity relative to the close mic (the sign of the normalised correlation of the
   print and the mic at the same samples, majority); and the match of the render with the mic against the
   match of the print with the mic over the same window (1 ms before to 10 ms after the hit), per hit.

Outputs in ~/.cache/drumspike/layer-eval/ (`--polarity normal`: layer-eval-normal/, which reuses the events of
the first run when they are there): `run/<song>_events.json` (the app's events), `run/<song>_<print>/`
(one-shot, alignment answer, layer render), `scores.json`, `report.md`. Each step skips what is there (resumable),
one log per step in `logs/`.
"""
import argparse
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

import numpy as np
import soundfile as sf

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "apps" / "desktop" / "selftest"))
from make_project import build_project  # noqa: E402

SAMPLES = Path.home() / ".cache" / "drumspike" / "samples"
OUT = Path.home() / ".cache" / "drumspike" / "layer-eval"
SONGS = {"Session A": "Session A", "Session B": "Session B",
         "Session C": "Session C"}
MICS = {"k in": ("kick", 36, ("k samp", "k sampl")), "s top": ("snare", 38, ("s samp", "s sampl"))}
SKIP_WORDS = ("trig", "rev")
PRE_S, ATTACK_S = 0.001, 0.010       # the app's attack template (AlignParams::default)
WIN_PRE_S, WIN_POST_S = 0.001, 0.010  # the match window around a hit
PRINT_SEARCH_S = 0.010               # where the print's onset is searched around the hit
PRINTED_MIN = 0.5                    # a hit has a printed transient when the template matches the print this well


def stem(f):
    """`k in_02.wav` -> `k in`; `k samp 1_02.L.wav` -> `k samp 1`."""
    n = Path(f).name
    for ext in (".L.wav", ".R.wav", ".wav"):
        if n.endswith(ext):
            n = n[: -len(ext)]
            break
    return n.rsplit("_", 1)[0]


def session_files(folder):
    """{mic: (close mic file, [printed files])} of a session folder; a stereo print split into .L/.R takes .L."""
    files = sorted((folder / "Audio Files").glob("*.wav"))
    out = {}
    for mic, (_, _, prefixes) in MICS.items():
        close = [f for f in files if stem(f) == mic]
        prints = [f for f in files if any(stem(f).startswith(p + " ") for p in prefixes)
                  and not any(w in stem(f) for w in SKIP_WORDS) and not f.name.endswith(".R.wav")]
        if close:
            out[mic] = (close[0], prints)
    return out


def mono(path):
    x, sr = sf.read(path, dtype="float64", always_2d=True)
    return x.mean(axis=1), sr


def ncc(a, b):
    na, nb = np.sqrt(np.dot(a, a)), np.sqrt(np.dot(b, b))
    return float(np.dot(a, b) / (na * nb)) if na > 0 and nb > 0 else 0.0


def window(x, a, b):
    """x[a:b] with zeros outside the signal."""
    out = np.zeros(b - a)
    lo, hi = max(a, 0), min(b, len(x))
    if hi > lo:
        out[lo - a:hi - a] = x[lo:hi]
    return out


def parabolic(y, i):
    if i <= 0 or i >= len(y) - 1:
        return 0.0
    a, b, c = y[i - 1], y[i], y[i + 1]
    den = a - 2 * b + c
    return 0.0 if den >= 0 else float(np.clip(0.5 * (a - c) / den, -0.5, 0.5))


def find_template(x, center, tpl, pre, search):
    """The position of the template's onset in x near `center` (sub-sample), by the largest |normalised
    correlation| within +-search samples, and its signed height."""
    n = len(tpl)
    seg = window(x, center - pre - search, center - pre + search + n)
    ys = np.array([ncc(tpl, seg[k:k + n]) for k in range(2 * search + 1)])
    j = int(np.argmax(np.abs(ys)))
    s = 1.0 if ys[j] >= 0 else -1.0
    return center - search + j + parabolic(s * ys, j), float(ys[j])


def cut_one_shot(y, sr, idx, pre_s=0.010, post_s=0.300):
    """A one-shot from the print `y`: the loudest hit (peak in its first 30 ms) among the hits with nothing in
    the 50 ms before and the 400 ms after; cut from 10 ms before its transient (the first sample at 5 % of that
    peak) to 300 ms after, a 20 ms fade at the end. Returns (samples, hit index used)."""
    idx = sorted(idx)
    best, best_pk = None, 0.0
    for k, i in enumerate(idx):
        prev = idx[k - 1] if k else -10 ** 9
        nxt = idx[k + 1] if k + 1 < len(idx) else 10 ** 9
        if i - prev < 0.05 * sr or nxt - i < 0.4 * sr:
            continue
        pk = float(np.max(np.abs(window(y, i - int(0.005 * sr), i + int(0.030 * sr)))))
        if pk > best_pk:
            best, best_pk = i, pk
    if best is None:
        raise RuntimeError("no isolated hit in the print")
    seg = window(y, best - int(0.010 * sr), best + int(0.030 * sr))
    start = best - int(0.010 * sr) + int(np.argmax(np.abs(seg) >= 0.05 * best_pk))
    shot = window(y, start - int(pre_s * sr), start + int(post_s * sr)).copy()
    fade = int(0.020 * sr)
    shot[-fade:] *= np.linspace(1.0, 0.0, fade)
    return shot, best


def run_steps(tool, steps, work, log):
    work.mkdir(parents=True, exist_ok=True)
    f = work / "steps.json"
    f.write_text(json.dumps(steps))
    env = dict(os.environ, DRUMSPIKE_HOME=str(work / "home"), DRUMSPIKE_PROJECTS=str(work / "projects"),
               DRUMSPIKE_AUDIO="null", DRUMSPIKE_MASTER_GAIN="0")
    r = subprocess.run([str(tool), "run", str(f)], capture_output=True, text=True, env=env, stdin=subprocess.DEVNULL)
    log.write_text(r.stdout + "\n--- stderr\n" + r.stderr)
    rows = [json.loads(x) for x in r.stdout.splitlines() if x.strip()]
    bad = [x for x in rows if "answer" not in x]
    if r.returncode != 0 or bad or len(rows) != len(steps):
        raise RuntimeError(f"service_tool run: exit {r.returncode} {bad[:1]} {r.stderr[-400:]}")
    return rows


def do_run(out, tool, polarity="auto"):
    tool = Path(tool or ROOT / "target" / "release" / "service_tool")
    if not tool.is_file():
        raise SystemExit(f"need the release service_tool: {tool}")
    (out / "run").mkdir(parents=True, exist_ok=True)
    (out / "logs").mkdir(parents=True, exist_ok=True)
    for song, folder in SONGS.items():
        files = session_files(SAMPLES / folder)
        key = song.replace(" ", "_")
        # step 1: the app's analysis of the close mics (one project, both mics)
        ev_file = out / "run" / f"{key}_events.json"
        shared = OUT / "run" / f"{key}_events.json"
        if not ev_file.is_file() and shared.is_file() and shared != ev_file:
            shutil.copyfile(shared, ev_file)                      # the same analysis for every polarity run
        if not ev_file.is_file():
            work = out / "scratch" / f"{key}_analysis"
            shutil.rmtree(work, ignore_errors=True)
            pdir = Path(build_project(work, "Eval", [(m, str(f), MICS[m][0], MICS[m][1]) for m, (f, _) in files.items()]))
            steps = [{"cmd": "project_open", "args": {"path": str(pdir)}},
                     {"cmd": "analyze_start", "args": {"sid": "$sid"}},
                     {"cmd": "wait_analysis", "args": {"sid": "$sid"}}]
            steps += [{"cmd": "events_get", "args": {"sid": "$sid", "name": m}} for m in files]
            rows = run_steps(tool, steps, work, out / "logs" / f"{key}_analysis.log")
            ev = {m: rows[3 + k]["answer"]["events"] for k, m in enumerate(files)}
            ev_file.write_text(json.dumps(ev))
            shutil.rmtree(work, ignore_errors=True)
            print(f"{song}: analysed {', '.join(f'{m} {len(v)} hits' for m, v in ev.items())}", flush=True)
        events = json.loads(ev_file.read_text())
        # steps 2 and 3 per printed track
        for mic, (close, prints) in files.items():
            for p in prints:
                pkey = f"{key}_{stem(p).replace(' ', '_')}"
                dst = out / "run" / pkey
                if (dst / "align.json").is_file() and (dst / "layer.wav").is_file():
                    continue
                dst.mkdir(parents=True, exist_ok=True)
                y, sr = mono(p)
                idx = [round(e["t"] * sr) for e in events[mic] if not e.get("cut") and not e.get("bleed")]
                shot, used = cut_one_shot(y, sr, idx)
                sf.write(dst / "oneshot.wav", shot, sr, subtype="FLOAT")
                work = out / "scratch" / pkey
                shutil.rmtree(work, ignore_errors=True)
                pdir = Path(build_project(work, "Eval", [(mic, str(close), MICS[mic][0], MICS[mic][1], events[mic])]))
                steps = [{"cmd": "project_open", "args": {"path": str(pdir)}},
                         {"cmd": "track_layer_set", "args": {"sid": "$sid", "name": mic, "path": str(dst / "oneshot.wav"),
                                                             "polarity": polarity, "align": True,
                                                             "curve": {"kind": "fixed"}}},
                         {"cmd": "track_layer_align", "args": {"sid": "$sid", "name": mic}},
                         {"cmd": "export_layer", "args": {"sid": "$sid", "name": mic}}]
                rows = run_steps(tool, steps, work, out / "logs" / f"{pkey}.log")
                layer = rows[1]["answer"]["layer"]
                (dst / "align.json").write_text(json.dumps({"layer": layer, "align": rows[2]["answer"],
                                                            "export": rows[3]["answer"], "cut_at": used,
                                                            "print": str(p), "close": str(close)}))
                shutil.copyfile(rows[3]["answer"]["path"], dst / "layer.wav")
                shutil.rmtree(work, ignore_errors=True)
                print(f"{song} {stem(p)}: {rows[2]['answer']['polarity']}, median match {rows[2]['answer']['median_match']}",
                      flush=True)


def score_print(close, printed, layer_wav, info):
    """The numbers of one printed track (see the module text)."""
    mic, sr = mono(close)
    y, _ = mono(printed)
    r, _ = mono(layer_wav)
    shot, _ = mono(Path(layer_wav).with_name("oneshot.wav"))
    onset = int(info["layer"]["onset"])
    pre = int(round(PRE_S * sr))
    tpl = shot[onset - pre:onset - pre + int(round(ATTACK_S * sr))]
    al = info["align"]
    sign = int(al["sign"])
    search = int(round(PRINT_SEARCH_S * sr))
    w0, w1 = int(round(WIN_PRE_S * sr)), int(round(WIN_POST_S * sr))
    rows = []
    for h in al["hits"]:
        if h["match"] is None:
            continue
        i = round(h["t"] * sr)
        p_pos, p_h = find_template(y, i, tpl, pre, search)
        if abs(p_h) < PRINTED_MIN:
            continue                                     # no printed transient at this hit
        ours = i + h["lag"]
        a, b = i - w0, i + w1
        m = window(mic, a, b)
        rows.append({"t": h["t"], "offset": ours - p_pos, "match": h["match"], "print_match": abs(p_h),
                     "print_sign": 1 if ncc(window(y, a, b), m) >= 0 else -1, "own_sign": h["sign"],
                     "render_ncc": ncc(window(r, a, b), m), "print_ncc": ncc(window(y, a, b), m)})
    off = np.array([x["offset"] for x in rows])
    psign = np.array([x["print_sign"] for x in rows])
    rn, pn = np.array([x["render_ncc"] for x in rows]), np.array([x["print_ncc"] for x in rows])
    print_pol = 1 if np.sum(psign > 0) >= np.sum(psign < 0) else -1
    return {
        "hits_app": len(al["hits"]), "hits_printed": len(rows),
        "offset_median": float(np.median(off)), "offset_mean": float(np.mean(off)),
        "offset_abs_p90": float(np.percentile(np.abs(off), 90)),
        "within_1": float(np.mean(np.abs(off) <= 1.0)), "within_halfms": float(np.mean(np.abs(off) <= 0.0005 * sr)),
        "app_polarity": sign, "print_polarity": print_pol, "print_sign_share": float(np.mean(psign == print_pol)),
        "polarity_agrees": sign == print_pol,
        "match_median": float(np.median([x["match"] for x in rows])),
        "render_ncc_median": float(np.median(rn)), "print_ncc_median": float(np.median(pn)),
        "render_ge_print": float(np.mean(rn >= pn - 1e-9)),
        "flips": len(al["flips"]), "uncertain": al["uncertain"], "sr": sr, "rows": rows,
    }


def do_score(out):
    res = {}
    for d in sorted((out / "run").iterdir()):
        if not d.is_dir() or not (d / "align.json").is_file():
            continue
        info = json.loads((d / "align.json").read_text())
        res[d.name] = score_print(info["close"], info["print"], d / "layer.wav", info)
    (out / "scores.json").write_text(json.dumps(res, indent=1))
    lines = ["| print | hits (app / printed) | offset median (smp) | mean | p90 abs | within 1 smp | within 0.5 ms | "
             "polarity app / print (share) | match median | render / print ncc median | render >= print |",
             "|---|---|---|---|---|---|---|---|---|---|---|"]
    for k, v in res.items():
        pol = lambda s: "inv" if s < 0 else "norm"
        lines.append(f"| {k} | {v['hits_app']} / {v['hits_printed']} | {v['offset_median']:+.2f} | {v['offset_mean']:+.2f} | "
                     f"{v['offset_abs_p90']:.2f} | {100 * v['within_1']:.0f} % | {100 * v['within_halfms']:.0f} % | "
                     f"{pol(v['app_polarity'])} / {pol(v['print_polarity'])} ({100 * v['print_sign_share']:.0f} %) | "
                     f"{v['match_median']:.3f} | {v['render_ncc_median']:.3f} / {v['print_ncc_median']:.3f} | "
                     f"{100 * v['render_ge_print']:.0f} % |")
    text = "\n".join(lines)
    (out / "report.md").write_text(text + "\n")
    print(text)
    return res


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("what", choices=["run", "score"])
    ap.add_argument("--out", default=None, help="default: ~/.cache/drumspike/layer-eval (-normal for --polarity normal)")
    ap.add_argument("--polarity", default="auto", choices=["auto", "normal"], help="run: the layer polarity")
    ap.add_argument("--tool", default=None, help="run: the service_tool (default: the release build)")
    a = ap.parse_args(argv)
    out = Path(a.out) if a.out else (OUT if a.polarity == "auto" else OUT.with_name(OUT.name + "-normal"))
    if a.what == "run":
        do_run(out, a.tool, a.polarity)
    else:
        do_score(out)


if __name__ == "__main__":
    main()
