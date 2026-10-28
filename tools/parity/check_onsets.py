"""DSP onset fallback (P2-5): corpus check and recall/precision against the R3 onsets.

    cargo build --release -p drumspike-devtools --bin parity
    uv run --with numpy python tools/parity/check_onsets.py corpus <corpus_dir> \\
        [--work <dir>] [--bin path] [--json out.json] [--set key=value]...
    uv run --with numpy python tools/parity/check_onsets.py real <refs_run_dir> <samples_dir> \\
        [--work <dir>] [--bin path] [--json out.json] [--md out.md] [--set role:key=value]...

`corpus`: the corpus of make_corpus.py (its manifest.json lists the true onsets of every
file). Every file runs through `parity onsets <wav> kick <out.json>` (`--set key=value` is
passed on). A true onset at t s is
found when a hit frame differs from round(t * 100) by at most 1 (one-to-one matching, the
same frame criterion as the real tracks). The onsets past the end of a cut file
(pcm16_short100, pcm16_partial5000) do not count. Every hit without a true onset is an extra
hit, classified by where it lies: "in silence" when no synthetic hit sounds there (each
tests/helpers.kick_hit lasts exactly 0.4 s, so a hit sounds in [t, t + 0.4 s]; 1 frame of
margin), else "in a sounding hit" (the note says when it is within 2 frames of the hard
0.4 s cut of an earlier hit, a step in the signal). PASS needs every onset found and no extra
hit in silence. A silent file (zeros, 10 s at 48 kHz, written to <work>) must give no hit
for any role.

`real`: <refs_run_dir> is a reference run of make_detect_refs.py (`items.json` and
`<stem>/r3.json`, the madmom peak picking on the PyTorch activations). The real tracks are
read from <samples_dir>/<Session> - session folder/Audio Files/<file> (read only). Role per token:
`k in` kick (class BD), `s top` snare (SD), `t1`..`t4` tom (TT), `hh` hat (HH), `oh` cymbal
(CY+RD). song1 is skipped: its drum tracks are byte-identical copies of Session_A tracks and
its click tracks are not in the local sessions. A hit matches an R3 onset of the role's
class when the frame indices differ by at most 1 (and, second table, 2); one-to-one
matching, closest pairs first. Recall = matched / R3 onsets, precision = matched / hits.
Tables per track and per role (sums of the counts). `--set role:key=value` passes
`--set key=value` to the binary for that role (tuning).

Hits are cached as <work>/<stem>.json only within one run; nothing is reused across runs.
"""
import argparse
import json
import os
import struct
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parents[2]
DEFAULT_BIN = ROOT / "target" / "release" / "parity"
ROLE_OF_TOKEN = {"k in": "kick", "s top": "snare", "t1": "tom", "t2": "tom", "t3": "tom", "t4": "tom",
                 "hh": "hat", "oh": "cymbal"}
CLASS_OF_ROLE = {"kick": 0, "snare": 1, "tom": 2, "hat": 3, "cymbal": 4}
ROLES = ["kick", "snare", "tom", "hat", "cymbal"]


def run_bin(binary, wav, role, out, sets=()):
    cmd = [str(binary), "onsets", str(wav), role, str(out)]
    for s in sets:
        cmd += ["--set", s]
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode != 0:
        raise RuntimeError(f"{' '.join(cmd)}: {r.stderr.strip()}")
    d = json.loads(Path(out).read_text())
    if not isinstance(d, dict) or "hits" not in d:
        raise RuntimeError(f"{out}: unexpected output shape {type(d)}")
    return d


def match(ref, det, tol):
    """One-to-one matching of two sorted number lists within tol, closest pairs first.
    Returns the matched pairs (ref value, det value)."""
    ref = np.asarray(ref, dtype=float)
    det = np.asarray(det, dtype=float)
    pairs = []
    j0 = 0
    for i, r in enumerate(ref):
        while j0 < len(det) and det[j0] < r - tol:
            j0 += 1
        j = j0
        while j < len(det) and det[j] <= r + tol:
            pairs.append((abs(det[j] - r), i, j))
            j += 1
    pairs.sort()
    used_r, used_d, out = set(), set(), []
    for _, i, j in pairs:
        if i in used_r or j in used_d:
            continue
        used_r.add(i)
        used_d.add(j)
        out.append((ref[i], det[j]))
    return out


def write_silence(path, sr=48000, seconds=10.0):
    n = int(sr * seconds)
    data = b"\0" * (4 * n)
    fmt = struct.pack("<HHIIHH", 3, 1, sr, sr * 4, 4, 32)
    body = b"WAVE" + b"fmt " + struct.pack("<I", len(fmt)) + fmt + b"data" + struct.pack("<I", len(data)) + data
    Path(path).write_bytes(b"RIFF" + struct.pack("<I", len(body)) + body)


def cmd_corpus(a):
    corpus = Path(a.corpus)
    man = json.loads((corpus / "manifest.json").read_text())
    if not isinstance(man, dict) or "files" not in man:
        sys.exit("manifest.json: no 'files' list")
    work = Path(a.work or tempfile.mkdtemp(prefix="onsets_corpus_"))
    work.mkdir(parents=True, exist_ok=True)
    rows, ok = [], True
    for f in man["files"]:
        d = run_bin(a.bin, corpus / f["name"], "kick", work / (Path(f["name"]).stem + ".json"), a.set or [])
        dur = d["n_samples"] / d["sr"]
        truth = [t for t in f["onsets"] if t < dur]
        frames = [h["frame"] for h in d["hits"]]
        tframes = [int(np.round(t * 100)) for t in truth]
        m = match(tframes, frames, 1)
        offs = [int(det - ref) for ref, det in m]
        used = {int(det) for _, det in m}
        in_silence, in_sound, notes = 0, 0, []
        for k in frames:
            if k in used:
                continue
            t = k / 100
            sounding = [o for o in truth if o - 0.01 <= t <= o + 0.4 + 0.01]
            if sounding and t >= dur - 0.02:
                in_sound += 1
                notes.append(f"frame {k}: in a sounding hit, at the hard cut of the file end ({dur:.3f} s)")
            elif sounding:
                in_sound += 1
                cut = [o for o in truth if abs(t - (o + 0.4)) <= 0.02]
                notes.append(f"frame {k}: in a sounding hit" + (f", at the 0.4 s cut of the hit at {cut[0]} s" if cut else ""))
            else:
                in_silence += 1
                notes.append(f"frame {k}: in silence")
        found = len(m)
        good = found == len(truth) and in_silence == 0
        ok &= good
        rows.append({"file": f["name"], "sr": d["sr"], "onsets": len(truth), "found": found,
                     "extra_in_silence": in_silence, "extra_in_sound": in_sound, "notes": notes,
                     "offset_frames": {str(k): offs.count(k) for k in (-1, 0, 1)},
                     "detect_s": round(d["seconds"]["detect"], 4), "pass": good})
    sil = work / "silence_48k.wav"
    write_silence(sil)
    ds = run_bin(a.bin, sil, "kick", work / "silence.json")
    sil_hits = {r: len(run_bin(a.bin, sil, r, work / f"silence_{r}.json")["hits"]) for r in ROLES + ["other"]}
    ok &= all(v == 0 for v in sil_hits.values())
    print("| File | Rate | Onsets | Found within 1 frame | Offset -1/0/+1 | Extra in silence | Extra in a sounding hit | Result |")
    print("|---|---|---|---|---|---|---|---|")
    for r in rows:
        o = r["offset_frames"]
        print(f"| `{r['file']}` | {r['sr']} | {r['onsets']} | {r['found']} | {o['-1']}/{o['0']}/{o['1']} | "
              f"{r['extra_in_silence']} | {r['extra_in_sound']} | {'PASS' if r['pass'] else 'FAIL'} |")
    for r in rows:
        for n in r["notes"]:
            print(f"  {r['file']}: {n}")
    tot_on = sum(r["onsets"] for r in rows)
    tot_found = sum(r["found"] for r in rows)
    tot_sil = sum(r["extra_in_silence"] for r in rows)
    tot_snd = sum(r["extra_in_sound"] for r in rows)
    print(f"total: {tot_found}/{tot_on} found within 1 frame, {tot_sil} extra hits in silence, "
          f"{tot_snd} extra hits in a sounding hit; silence file (10 s zeros, 48 kHz, {ds['frames']} frames): "
          f"hits per role {sil_hits}")
    print("PASS" if ok else "FAIL")
    if a.json:
        Path(a.json).write_text(json.dumps({"files": rows, "silence_hits": sil_hits, "pass": ok, "found": tot_found,
                                            "onsets": tot_on, "extra_in_silence": tot_sil,
                                            "extra_in_sound": tot_snd}, indent=1))
    return 0 if ok else 1


def local_path(samples, item):
    sess = item["session"].replace("_", " ")
    return Path(samples) / f"{sess} - session folder" / "Audio Files" / item["file"]


def cmd_real(a):
    refs = Path(a.refs)
    items = json.loads((refs / "items.json").read_text())
    if not isinstance(items, dict) or "items" not in items:
        sys.exit("items.json: no 'items' list")
    sets = {}
    for s in a.set or []:
        role, kv = s.split(":", 1)
        sets.setdefault(role, []).append(kv)
    work = Path(a.work or tempfile.mkdtemp(prefix="onsets_real_"))
    work.mkdir(parents=True, exist_ok=True)
    jobs, skipped = [], []
    for it in items["items"]:
        if it["set"] == "corpus":
            continue
        if it["session"] == "song1":
            skipped.append(it["stem"])
            continue
        role = ROLE_OF_TOKEN.get(it["token"])
        wav = local_path(a.samples, it)
        if role is None or not wav.exists():
            skipped.append(it["stem"])
            continue
        jobs.append((it["stem"], role, wav))

    def one(job):
        stem, role, wav = job
        d = run_bin(a.bin, wav, role, work / f"{stem}.json", sets.get(role, []))
        r3 = json.loads((refs / stem / "r3.json").read_text())
        if not isinstance(r3, dict) or "onsets" not in r3:
            raise RuntimeError(f"{stem}/r3.json: unexpected shape")
        ref = sorted(o[0] for o in r3["onsets"] if o[1] == CLASS_OF_ROLE[role])
        det = [h["frame"] for h in d["hits"]]
        m1 = match(ref, det, 1)
        m2 = match(ref, det, 2)
        offs = [int(dd - rr) for rr, dd in m2]
        hist = {k: offs.count(k) for k in range(-2, 3)}
        return {"stem": stem, "role": role, "ref": len(ref), "hits": len(det), "m1": len(m1), "m2": len(m2),
                "offset_hist": hist, "frames": d["frames"], "detect_s": d["seconds"]["detect"]}

    with ThreadPoolExecutor(max_workers=a.jobs) as ex:
        rows = list(ex.map(one, jobs))

    def rp(m, ref, hits):
        r = m / ref if ref else float("nan")
        p = m / hits if hits else float("nan")
        return r, p

    def pct(v):
        return "-" if v != v else f"{100 * v:.1f} %"

    lines = ["| Track | Role | R3 onsets | Hits | Recall 1 fr | Precision 1 fr | Recall 2 fr | Precision 2 fr | "
             "Offset -2/-1/0/+1/+2 |", "|---|---|---|---|---|---|---|---|---|"]
    for r in sorted(rows, key=lambda r: (ROLES.index(r["role"]), r["stem"])):
        r1, p1 = rp(r["m1"], r["ref"], r["hits"])
        r2, p2 = rp(r["m2"], r["ref"], r["hits"])
        h = r["offset_hist"]
        lines.append(f"| `{r['stem']}` | {r['role']} | {r['ref']} | {r['hits']} | {pct(r1)} | {pct(p1)} | {pct(r2)} | "
                     f"{pct(p2)} | {h[-2]}/{h[-1]}/{h[0]}/{h[1]}/{h[2]} |")
    lines += ["", "| Role | Tracks | R3 onsets | Hits | Recall 1 fr | Precision 1 fr | F 1 fr | Recall 2 fr | "
                  "Precision 2 fr |", "|---|---|---|---|---|---|---|---|---|"]
    summary = {}
    for role in ROLES:
        rr = [r for r in rows if r["role"] == role]
        ref = sum(r["ref"] for r in rr)
        hits = sum(r["hits"] for r in rr)
        m1 = sum(r["m1"] for r in rr)
        m2 = sum(r["m2"] for r in rr)
        r1, p1 = rp(m1, ref, hits)
        r2, p2 = rp(m2, ref, hits)
        f1 = 2 * r1 * p1 / (r1 + p1) if (r1 + p1) else 0.0
        summary[role] = {"tracks": len(rr), "ref": ref, "hits": hits, "m1": m1, "m2": m2,
                         "recall1": r1, "precision1": p1, "f1": f1, "recall2": r2, "precision2": p2}
        lines.append(f"| {role} | {len(rr)} | {ref} | {hits} | {pct(r1)} | {pct(p1)} | {pct(f1)} | {pct(r2)} | {pct(p2)} |")
    text = "\n".join(lines)
    print(text)
    print(f"\nskipped: {', '.join(skipped)}")
    if a.md:
        Path(a.md).write_text(text + "\n")
    if a.json:
        Path(a.json).write_text(json.dumps({"tracks": rows, "roles": summary, "skipped": skipped,
                                            "sets": sets}, indent=1))
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    c = sub.add_parser("corpus")
    c.add_argument("corpus")
    c.add_argument("--set", action="append", help="key=value passed to the binary (role kick)")
    r = sub.add_parser("real")
    r.add_argument("refs")
    r.add_argument("samples")
    r.add_argument("--set", action="append")
    r.add_argument("--md")
    r.add_argument("--jobs", type=int, default=max(1, (os.cpu_count() or 4) // 3))
    for p in (c, r):
        p.add_argument("--work")
        p.add_argument("--bin", default=str(DEFAULT_BIN))
        p.add_argument("--json")
    a = ap.parse_args()
    return cmd_corpus(a) if a.cmd == "corpus" else cmd_real(a)


if __name__ == "__main__":
    sys.exit(main())
