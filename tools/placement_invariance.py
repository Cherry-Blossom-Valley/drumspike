"""#443: measure the placement rule's invariance on real stems (internal data; aggregates only).

    uv run --no-project --with numpy,soundfile python tools/placement_invariance.py MEASURE_DIR SAMPLES_ROOT \
        [--bounds tools/placement_invariance.bounds.json] [--per 40] [--seed 7] \
        [--parity target/release/parity] [--service-tool target/release/service_tool] \
        [--delays 1,2,3,4,5,6,7] [--len 20] [--skip-delay] [--rule 10]

MEASURE_DIR holds the stored tool runs (`*before.json`); SAMPLES_ROOT holds
`<session name>/Audio Files/*.wav`. For each hit of each kick, snare and tom track the tool cuts an
excerpt (0.120 s before, 0.080 s after the hit), places it with the current rule, then places a
changed copy of the same audio and counts how far the answer moved. One line per change:
gain -12 dB and +6 dB, polarity flip, DC offset of 1 % of the hit's peak, 50 Hz hum at 2 % of the
peak, white noise 50 dB under the peak, the coarse time moved by -3/-1/+1/+3 ms, the stem
resampled to 96 kHz (compared in 48 kHz samples), and the whole excerpt delayed by 1 to 7 samples
and run through the FULL track analysis (the hit must move by exactly that count).

Per line and role it prints the hits and the share moved by 1, by 2 or more and by 5 or more
samples (a difference counts from half a sample; for the delay line: the share placed exactly,
5 or more off, and the hits lost). The count per line is printed and a line with 0 hits is a FAIL.
With --bounds FILE (one bound per line and role) the tool exits non-zero when a share is over its
bound. All data comes from the arguments, never from a path in the code.

The rule runs through the repo's devtools binaries: `parity refine` for the window lines
(the current rule version, or --rule N; one batched job per line) and `service_tool core-analyze`
(DSP detector + the current rule) for the delay line. No second copy of the core is built.
"""
import argparse
import json
import os
import re
import struct
import subprocess
import sys
import tempfile
import wave
from pathlib import Path

import numpy as np

PRE, POST = 0.120, 0.080          # the excerpt cut around a hit, in seconds
GUARD_S = 0.3                     # silence between excerpts in the batched wav
SR = 48000
GAINS = {"gain -12 dB": 10 ** (-12 / 20), "gain +6 dB": 10 ** (6 / 20)}
NOISE_DB = -50                    # white noise under the hit's peak
HEAD_GUARD = 512 + 8              # samples of head room for the delay line (>= max delay)
RULE = 10                         # the current rule version (`parity refine` takes the number)


# ---------------------------------------------------------------- the placer interface

def place_with_parity(records, parity, tmp, rule=RULE):
    """`parity refine`: records are (x, sr, t0); returns the refined position in buffer samples.

    All excerpts are batched into one float wav, silence-guarded, so one job runs per line."""
    srs = {sr for _, sr, _ in records}
    if len(srs) != 1:
        sys.exit(f"FAIL: one sample rate per batch, got {sorted(srs)}")
    sr = srs.pop()
    guard = int(GUARD_S * sr)
    chunks, hits, off = [], [], 0
    for x, _, t0 in records:
        chunks.append(np.zeros(guard + len(x) + guard, dtype=np.float32))
        chunks[-1][guard:guard + len(x)] = x
        hits.append(off + guard + t0 * sr)
        off += len(x) + 2 * guard
    wav = tmp / "batch.wav"
    write_float_wav(wav, np.concatenate(chunks), sr)
    job = tmp / "refine.json"
    job.write_text(json.dumps({"wav": str(wav), "rule": rule,
                               "hits": [{"t0": float(h / sr)} for h in hits]}))
    r = subprocess.run([str(parity), "refine", str(job)], capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit(f"FAIL: parity refine: {r.stderr[-400:]}")
    out = json.loads(r.stdout)
    return np.array([(ref["t"] * sr) for ref in out["refined"]])


def write_float_wav(path, x, sr):
    """A mono IEEE-float wav (format tag 3), what the repo's wav reader accepts."""
    data = x.astype("<f4").tobytes()
    with open(path, "wb") as f:
        f.write(b"RIFF" + struct.pack("<I", 36 + len(data)) + b"WAVE")
        f.write(b"fmt " + struct.pack("<IHHIIHH", 16, 3, 1, sr, sr * 4, 4, 32))
        f.write(b"data" + struct.pack("<I", len(data)) + data)


def analyze_with_service_tool(records, service_tool, tmp):
    """`service_tool core-analyze` (DSP detector + the current rule) on whole tracks.

    records are (x, sr, role); returns per record the hit positions in buffer samples."""
    jobs, paths = [], []
    for n, (x, sr, role) in enumerate(records):
        p = tmp / f"track{n}.wav"
        write_float_wav(p, x, sr)
        paths.append(p)
        jobs.append({"wav": str(p), "role": role})
    jf = tmp / "jobs.json"
    jf.write_text(json.dumps(jobs))
    r = subprocess.run([str(service_tool), "core-analyze", str(jf)], capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit(f"FAIL: service_tool core-analyze: {r.stderr[-400:]}")
    out = [json.loads(line) for line in r.stdout.splitlines() if line.strip()]
    if len(out) != len(records):
        sys.exit(f"FAIL: core-analyze answered for {len(out)} of {len(records)} records")
    return [np.array([e["t"] * rec[1] for e in o["events"]]) for o, rec in zip(out, records)]


# ---------------------------------------------------------------- the window lines

def hit_peak(x, at):
    """The hit's peak: the loudest sample of the 5 ms after it."""
    return np.abs(x[at:at + 240]).max()


def upsample_96(x):
    """A band-limited 2x copy of the excerpt (FFT zero-padding, no scipy)."""
    n = len(x)
    f = np.fft.rfft(x.astype(np.float64))
    g = np.zeros(n + 1, dtype=complex)
    g[:len(f)] = f
    return (np.fft.irfft(g, 2 * n) * 2).astype(np.float32)


def window_records(hits, rng):
    """The base records and one record set per change line.

    Returns (base_records, {line: records, scale}): a record is (x, sr, t0)."""
    base = [(h["x"], SR, h["at"] / SR) for h in hits]
    lines = {}
    for name, g in GAINS.items():
        lines[name] = ([(np.clip(h["x"] * g, -1, 1), SR, h["at"] / SR) for h in hits], 1.0, 0.5)
    lines["polarity flip"] = ([(-h["x"], SR, h["at"] / SR) for h in hits], 1.0, 0.5)
    lines["DC offset 1% of peak"] = ([(h["x"] + 0.01 * hit_peak(h["x"], h["at"]), SR, h["at"] / SR) for h in hits], 1.0, 0.5)
    t = np.arange(max(len(h["x"]) for h in hits)) / SR
    lines["50 Hz hum 2% of peak"] = ([(np.clip(h["x"] + 0.02 * hit_peak(h["x"], h["at"]) * np.sin(2 * np.pi * 50 * t[:len(h["x"])]), -1, 1).astype(np.float32), SR, h["at"] / SR) for h in hits], 1.0, 0.5)
    lines["white noise 50 dB under peak"] = ([(h["x"] + hit_peak(h["x"], h["at"]) * 10 ** (NOISE_DB / 20) * rng.standard_normal(len(h["x"])).astype(np.float32), SR, h["at"] / SR) for h in hits], 1.0, 0.5)
    for ms in (-3, -1, 1, 3):
        lines[f"coarse time {ms:+d} ms"] = ([(h["x"], SR, (h["at"] + ms * SR / 1000) / SR) for h in hits], 1.0, 0.5)
    lines["96 kHz resample"] = ([(upsample_96(h["x"]), 96000, h["at"] / SR) for h in hits], 0.5, 0.51)
    return base, lines


def shares(deltas, roles, tol=0.5):
    """Per role: hits, share moved by 1, by 2+, by 5+ (percent).

    A difference of up to half a sample counts as not moved (`tol`, a bit over 0.5 for the
    resampled lines, where the grids differ)."""
    out = {}
    for role in ("kick", "snare", "tom", "all"):
        d = np.abs(deltas[roles == role]) if role != "all" else np.abs(deltas)
        if len(d) == 0:
            continue
        out[role] = {"n": int(len(d)), "moved1": 100 * float((d >= tol).mean()),
                     "moved2": 100 * float((d >= 1.5).mean()), "moved5": 100 * float((d >= 4.5).mean())}
    return out


def measure_window_lines(hits, place, rng):
    """Run every change line; {"lines": {line: shares}, "base": positions}."""
    base_records, lines = window_records(hits, rng)
    base_pos = place(base_records)
    roles = np.array([h["role"] for h in hits])
    out = {}
    for name, (records, scale, tol) in lines.items():
        pos = place(records)
        out[name] = shares((pos * scale) - base_pos, roles, tol)
    return {"lines": out, "base": base_pos}


# ---------------------------------------------------------------- the delay line

def pick_excerpts(measure_dir, samples_root, seconds, limit):
    """Tracks with 20 or more events: one `seconds` excerpt around the median event."""
    norm = lambda s: re.sub(r"[^a-z0-9]", "", s.lower())  # noqa: E731
    import soundfile as sf
    excerpts = []
    for f in sorted(Path(measure_dir).glob("*before.json")):
        name = f.name[: -len("_before.json")]
        folder = Path(samples_root) / name / "Audio Files"
        if not folder.is_dir():
            continue
        wavs = {norm(Path(w).stem): w for w in os.listdir(folder) if w.lower().endswith(".wav")}
        d = json.load(open(f))
        for key, tr in d.items():
            w = wavs.get(norm(key))
            if not w or tr.get("role") not in ("kick", "snare", "tom") or len(tr.get("events", [])) < 20:
                continue
            path = folder / w
            sr = sf.info(path).samplerate
            ts = [e["t"] for e in tr["events"]]
            mid = ts[len(ts) // 2]
            a = max(sr, int((mid - seconds / 2) * sr))
            n = int(seconds * sr)
            x, _ = sf.read(path, start=a - HEAD_GUARD, frames=n + HEAD_GUARD, dtype="float32", always_2d=True)
            excerpts.append({"role": tr["role"], "x": np.ascontiguousarray(x[:, 0]), "sr": sr, "len": n})
            if limit and len(excerpts) >= limit:
                return excerpts
    return excerpts


def measure_delay_line(excerpts, analyze, delays, head_guard=HEAD_GUARD):
    """The whole audio delayed by `delays` samples through the full track analysis.

    A hit of the base run must reappear exactly `delay` samples later. {"delay": {role: shares}}."""
    out = {}
    for delay in delays:
        base = analyze([(e["x"][head_guard:], e["sr"], e["role"]) for e in excerpts])
        records, index = [], []
        for n, e in enumerate(excerpts):
            records.append((e["x"][head_guard - delay:], e["sr"], e["role"]))
            index.append(n)
        got = analyze(records)
        tot = {}
        for n, e in enumerate(excerpts):
            sr, x = e["sr"], e["x"]
            inner = np.round(base[n])[(base[n] > sr // 2) & (base[n] < len(x) - head_guard - sr // 2)]
            g = np.round(got[n] - delay)
            for b in inner:
                j = np.abs(g - b).min() if len(g) else 10 ** 9
                t = tot.setdefault(e["role"], {"n": 0, "exact": 0, "off1": 0, "off2": 0, "off5": 0, "lost": 0})
                t["n"] += 1
                if j == 0:
                    t["exact"] += 1
                elif j == 1:
                    t["off1"] += 1
                elif j < 5:
                    t["off2"] += 1
                elif j < 960:
                    t["off5"] += 1
                else:
                    t["lost"] += 1
        out[f"delay {delay} sample" + ("s" if delay != 1 else "")] = {
            role: {"n": t["n"], "exact": 100 * t["exact"] / max(t["n"], 1),
                   "off1": 100 * t["off1"] / max(t["n"], 1), "off2": 100 * t["off2"] / max(t["n"], 1),
                   "off5": 100 * t["off5"] / max(t["n"], 1), "lost": 100 * t["lost"] / max(t["n"], 1)}
            for role, t in sorted(tot.items())}
    return out


# ---------------------------------------------------------------- reporting

def print_report(window, delay, out=sys.stdout):
    """One line per change and role; the count per line is printed, 0 hits is a FAIL."""
    fail = False
    if not window and not delay:
        print("FAIL: 0 hits measured", file=out)
        return 1
    for line, by_role in window.items():
        for role, m in by_role.items():
            print(f"{line:32} {role:5} n={m['n']:4}  moved={m['moved1']:5.1f}%  2+={m['moved2']:5.1f}%  5+={m['moved5']:5.1f}%", file=out)
        if not by_role:
            print(f"FAIL: {line}: 0 hits", file=out)
            fail = True
    for line, by_role in delay.items():
        for role, m in by_role.items():
            print(f"{line:32} {role:5} n={m['n']:4}  exact={m['exact']:5.1f}%  1off={m['off1']:4.1f}%  2-4={m['off2']:4.1f}%  5+off={m['off5']:5.1f}%  lost={m['lost']:4.1f}%", file=out)
        if not by_role:
            print(f"FAIL: {line}: 0 hits", file=out)
            fail = True
    return 1 if fail else 0


def over_bounds(window, delay, bounds):
    """The shares over their bound, as [(line, role, share, bound)]."""
    hits = []
    for kind, res in (("window", window), ("delay", delay)):
        b = bounds.get(kind, {})
        for line, by_role in res.items():
            for role, m in by_role.items():
                lb = b.get(line, {}).get(role, {})
                for key, val in m.items():
                    if key == "n" or key not in lb:
                        continue
                    # 'exact' is a floor (the hits must move by exactly the delay); everything
                    # else is a ceiling.
                    if (val < lb[key]) if key == "exact" else (val > lb[key]):
                        hits.append((f"{line} {role}", key, val, lb[key]))
    return hits


def load_hits(measure_dir, samples_root, per, seed):
    """The window hits: `per` per track, cut around the stored position."""
    norm = lambda s: re.sub(r"[^a-z0-9]", "", s.lower())  # noqa: E731
    import soundfile as sf
    rng = np.random.default_rng(seed)
    hits = []
    for f in sorted(Path(measure_dir).glob("*before.json")):
        name = f.name[: -len("_before.json")]
        folder = Path(samples_root) / name / "Audio Files"
        if not folder.is_dir():
            continue
        wavs = {norm(Path(w).stem): w for w in os.listdir(folder) if w.lower().endswith(".wav")}
        d = json.load(open(f))
        for key, tr in d.items():
            w = wavs.get(norm(key))
            if not w or tr.get("role") not in ("kick", "snare", "tom"):
                continue
            ev = [e for e in tr.get("events", []) if not e.get("bleed") and not e.get("cut")]
            if len(ev) < 5:
                continue
            path = folder / w
            x, sr = sf.read(path, dtype="float32", always_2d=True)
            x = np.ascontiguousarray(x[:, 0])
            if sr != SR:
                print(f"note: {path.name} is {sr} Hz, skipped (the tool measures at {SR} Hz)")
                continue
            for i in rng.choice(len(ev), size=min(per, len(ev)), replace=False):
                s = round(ev[i]["t"] * sr)
                a, b = s - int(PRE * sr), s + int(POST * sr)
                if a < 0 or b > len(x):
                    continue
                hits.append({"role": tr["role"], "x": x[a:b].copy(), "at": s - a, "sr": sr})
    counts = {r: sum(h["role"] == r for h in hits) for r in ("kick", "snare", "tom")}
    print(f"hits {len(hits)} {counts}")
    return hits, rng


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("measure_dir", help="the stored tool runs: *before.json")
    ap.add_argument("samples_root", help="<session name>/Audio Files/*.wav")
    ap.add_argument("--bounds", help="the bounds file: one bound per line and role")
    ap.add_argument("--per", type=int, default=40, help="hits picked per track")
    ap.add_argument("--seed", type=int, default=7)
    ap.add_argument("--parity", default="target/release/parity")
    ap.add_argument("--service-tool", default="target/release/service_tool")
    ap.add_argument("--delays", default="1,2,3,4,5,6,7", help="the delay line's shifts, in samples")
    ap.add_argument("--len", type=float, default=20.0, help="seconds per delay-line excerpt")
    ap.add_argument("--excerpts", type=int, default=0, help="cap the delay line's excerpt count")
    ap.add_argument("--skip-delay", action="store_true", help="skip the full-track delay line")
    ap.add_argument("--rule", type=int, default=RULE,
                    help="the rule version of the window lines (the delay line always runs the current rule)")
    a = ap.parse_args(argv)
    parity, service_tool = Path(a.parity), Path(a.service_tool)
    if not parity.is_file():
        sys.exit(f"FAIL: {parity} not built: cargo build --release -p drumspike-devtools")
    if not a.skip_delay and not service_tool.is_file():
        sys.exit(f"FAIL: {service_tool} not built: cargo build --release -p drumspike-devtools")
    hits, rng = load_hits(a.measure_dir, a.samples_root, a.per, a.seed)
    if not hits:
        sys.exit("FAIL: 0 hits collected")
    with tempfile.TemporaryDirectory(prefix="placement-inv-") as td:
        tmp = Path(td)
        place = lambda records: place_with_parity(records, parity, tmp, a.rule)  # noqa: E731
        window = measure_window_lines(hits, place, rng)["lines"]
        delay = {}
        if not a.skip_delay:
            excerpts = pick_excerpts(a.measure_dir, a.samples_root, a.len, a.excerpts)
            print(f"delay-line excerpts {len(excerpts)}")
            if excerpts:
                delays = [int(d) for d in a.delays.split(",")]
                analyze = lambda recs: analyze_with_service_tool(recs, service_tool, tmp)  # noqa: E731
                delay = measure_delay_line(excerpts, analyze, delays)
    code = print_report(window, delay)
    if a.bounds:
        bad = over_bounds(window, delay, json.load(open(a.bounds)))
        for line, key, val, bound in bad:
            print(f"OVER BOUND: {line} {key} {val:.1f} > {bound}", file=sys.stderr)
        if bad:
            code = 1
    return code


if __name__ == "__main__":
    sys.exit(main() or 0)
