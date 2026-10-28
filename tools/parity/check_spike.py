"""P3-3c: spike WAV export and velocity recompute of the app service against the frozen Python side.

    cargo build --release -p drumspike-devtools
    uv run --project . python tools/parity/check_spike.py <corpus_dir> \\
        [--samples <dir> [--layout named|flat] [--real 3]] [--json out.json] [--bin service_tool]

<corpus_dir> is the output of make_corpus.py. Each source file (the corpus; with --samples the
first --real real tracks of tests/golden/detect) becomes a one-track project folder whose
document holds the hit events (the refined hits of check_midi.py for a corpus file, the R4
onsets of the golden detect file for a real track, plus one hit before the start and one past
the end). Two checks, both read only on the sources:

spikes. `service_tool spikes <projects>` runs Service::export_spikes on the events of the
document. The reference is drumspike_core.write_spike_wav (the binding of the retired server) on
the source with the same times and the sample format that this script reads from the source's
`fmt ` chunk (16, 24, 32 bit PCM or float), frozen as frozen.wav_summary in
tests/golden/frozen/spike.json (P3-12). Rule: identical bytes; the bext time reference of the spike file equals the source's
(None for a source without a bext chunk); the answer's sr, n_samples and format equal the
source's.

velocities. `service_tool velocities` runs Service::velocities_recompute on the events with two
parameter sets (the defaults, and vel_lo_db -50 / vel_hi_db -10 in the track's params). The
reference is what the retired server's route did: engine.load_f32(source, mix_mode) and
engine.recompute_velocities(x, sr, events, lo_db, hi_db), frozen in tests/golden/frozen/velocities.json.
Rule: the same list (values and key order of every event).

Exit status 1 when a case differs.
"""
import argparse
import hashlib
import json
import struct
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tests"))
sys.path.insert(0, str(ROOT / "apps" / "desktop" / "selftest"))
sys.path.insert(0, str(Path(__file__).resolve().parent))
import frozen  # noqa: E402
from make_project import build_project  # noqa: E402
from check_detect import local_path  # noqa: E402
from check_midi import bext_time_reference, corpus_events  # noqa: E402

SPIKES = frozen.store("spike")
VELOCITIES = frozen.store("velocities")


def file_sha(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


DEFAULT_BIN = ROOT / "target" / "release" / "service_tool"
PARAM_SETS = {"default": {}, "wide": {"vel_lo_db": -50.0, "vel_hi_db": -10.0}}


def source_format(path):
    """(format name for write_spike_wav, sample rate, frames) from the `fmt ` and `data` chunks."""
    b = Path(path).read_bytes()
    pos, fmt, frames = 12, None, None
    while pos + 8 <= len(b):
        cid, size = b[pos:pos + 4], struct.unpack("<I", b[pos + 4:pos + 8])[0]
        body = b[pos + 8:pos + 8 + size]
        if cid == b"fmt ":
            tag, ch, sr, _, block, bits = struct.unpack("<HHIIHH", body[:16])
            if tag == 0xFFFE:
                tag = struct.unpack("<H", body[24:26])[0]
            fmt = (tag, ch, sr, block, bits)
        elif cid == b"data":
            frames = min(size, len(b) - pos - 8) // fmt[3]
            break
        pos += 8 + size + (size % 2)
    tag, _, sr, _, bits = fmt
    name = "float32" if tag == 3 else {16: "pcm16", 24: "pcm24", 32: "pcm32"}[bits]
    return name, sr, frames


def events_with_extras(times):
    """Hit events as the page holds them: some with extra keys; one before the start, one past the end."""
    ev = []
    for i, t in enumerate(times):
        e = {"t": float(t), "v": 64 + i % 60}
        if i % 7 == 0:
            e.update({"added": True, "id": f"l{i}"})
        if i % 11 == 0:
            e.update({"orig": float(t) + 0.001, "rise": 12.5})
        ev.append(e)
    ev.append({"t": -0.5, "v": 100})
    ev.append({"t": 1.0e6, "v": 100})
    return ev


def make_project(base, stem, src, events, params):
    return Path(build_project(base, stem, [("T", src, "kick", 36, events, {"params": params})], mix_mode="ffmpeg"))


def run_tool(bin_path, args):
    r = subprocess.run([str(bin_path), *args], capture_output=True, text=True)
    if r.returncode != 0:
        raise SystemExit(f"service_tool failed: {r.stderr[-600:]}")
    return r.stdout


def cases(args):
    """(label, source path, hit times) for the corpus files and the real tracks."""
    corpus = Path(args.corpus)
    out = [(f"corpus/{n}", corpus / n, [e["t"] for e in ev]) for n, (_, _, _, ev) in corpus_events(corpus).items()]
    if args.samples:
        real = []
        for p in sorted((ROOT / "tests" / "golden" / "detect").glob("*.json")):
            g = json.loads(p.read_text())
            if isinstance(g, dict) and g.get("set") == "real" and g["session"] != "song1":
                path = local_path(args.samples, g, args.layout)
                if path.exists():
                    real.append((f"real/{g['stem']}", path, [o[2] for o in g["r4"]["onsets"]]))
        out += real[::max(1, len(real) // max(1, args.real))][:args.real]
    return out


def check_spikes(items, bin_path, scratch, rows):
    bad = 0
    dirs = []
    for label, src, times in items:
        stem = label.replace("/", "_").replace(" ", "_")
        dirs.append((label, src, make_project(scratch / "spk", stem, src, events_with_extras(times), {}), times))
    got = {}
    for line in run_tool(bin_path, ["spikes", *[str(d[2]) for d in dirs]]).splitlines():
        row = json.loads(line)
        assert isinstance(row, dict) and "tracks" in row, type(row)
        got[row["project"]] = row["tracks"]["T"]
    for label, src, pdir, times in dirs:
        r = got[str(pdir)]
        problems = []
        if "error" in r:
            problems.append(f"service error: {r['error']}")
        else:
            fmt, sr, frames = source_format(src)
            all_times = [e["t"] for e in events_with_extras(times)]
            inputs = {"source_sha256": file_sha(src), "times": all_times, "format": fmt}
            want = SPIKES.ref(label, inputs)
            mine = frozen.wav_summary(Path(r["path"]).read_bytes())
            if not frozen.same(mine, want):
                problems.append(f"bytes differ from the frozen spike file ({mine['sha256'][:12]} vs "
                                f"{(want.get('sha256') or want.get('$sha256', ''))[:12]})")
            tref_src, tref_out = bext_time_reference(src), bext_time_reference(r["path"])
            if tref_src != tref_out:
                problems.append(f"bext time reference {tref_out} != source {tref_src}")
            if (r["sr"], r["n_samples"], r["format"]) != (sr, frames, fmt):
                problems.append(f"answer {(r['sr'], r['n_samples'], r['format'])} != {(sr, frames, fmt)}")
            if r["time_reference"] != tref_src:
                problems.append(f"answer time_reference {r['time_reference']} != {tref_src}")
        rows.append({"check": "spike", "case": label, "format": r.get("format"), "bytes": r.get("size"),
                     "time_reference": r.get("time_reference"), "problems": problems})
        bad += bool(problems)
        print(f"{'FAIL' if problems else 'PASS'} spike {label:55s} {r.get('format', '-'):8s} {r.get('size', 0):>11,d} B  tref {r.get('time_reference')}")
        for p in problems:
            print("      " + p)
    return bad


def check_velocities(items, bin_path, scratch, rows):
    bad = 0
    for pname, params in PARAM_SETS.items():
        jobs, refs = [], []
        for label, src, times in items:
            ev = events_with_extras(times)
            stem = f"{pname}_" + label.replace("/", "_").replace(" ", "_")
            pdir = make_project(scratch / "vel", stem, src, [], params)
            jobs.append({"project": str(pdir), "track": "T", "events": ev})
            lo, hi = params.get("vel_lo_db", -40.0), params.get("vel_hi_db", -6.0)
            inputs = {"source_sha256": file_sha(src), "events": ev, "lo_db": lo, "hi_db": hi}
            refs.append(VELOCITIES.ref(f"{pname}/{label}", inputs, canonical_host=True))
        jf = scratch / f"vel_{pname}.json"
        jf.write_text(json.dumps(jobs))
        out = json.loads(run_tool(bin_path, ["velocities", str(jf)]))
        assert isinstance(out, list) and len(out) == len(jobs), type(out)
        for (label, _, _), job, ref, r in zip(items, jobs, refs, out):
            problems = []
            if "error" in r:
                problems.append(f"service error: {r['error']}")
            else:
                ev = r["events"]
                if frozen.is_digest(ref):
                    if not frozen.same(ev, ref):
                        problems.append("the events differ from the frozen reference (SHA-256)")
                    ref = ev
                if [list(e) for e in ev] != [list(e) for e in ref]:
                    problems.append("key order differs")
                if ev != ref:
                    d = next((i for i, (a, b) in enumerate(zip(ev, ref)) if a != b), min(len(ev), len(ref)))
                    problems.append(f"first difference at event {d}: {ev[d:d + 1]} vs {ref[d:d + 1]}")
                if [e["t"] for e in ev] != [e["t"] for e in job["events"]]:
                    problems.append("times changed")
            rows.append({"check": f"velocities:{pname}", "case": label, "events": len(ref), "problems": problems})
            bad += bool(problems)
            print(f"{'FAIL' if problems else 'PASS'} velocities[{pname}] {label:50s} {len(ref):6d} events")
            for p in problems:
                print("      " + p)
    return bad


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("corpus")
    ap.add_argument("--samples")
    ap.add_argument("--layout", choices=["named", "flat"], default="named")
    ap.add_argument("--real", type=int, default=3)
    ap.add_argument("--bin", type=Path, default=DEFAULT_BIN)
    ap.add_argument("--json")
    a = ap.parse_args()
    if not a.bin.exists():
        print(f"service_tool missing: {a.bin} (cargo build --release -p drumspike-devtools)")
        return 2
    items = cases(a)
    rows = []
    with tempfile.TemporaryDirectory(prefix="ds_spike_") as td:
        scratch = Path(td)
        bad = check_spikes(items, a.bin, scratch, rows) + check_velocities(items, a.bin, scratch, rows)
    n = len(rows)
    print(f"\n{n - bad}/{n} cases PASS ({len(items)} files: {sum(1 for i in items if i[0].startswith('corpus'))} corpus, "
          f"{sum(1 for i in items if i[0].startswith('real'))} real)")
    if a.json:
        Path(a.json).write_text(json.dumps({"rows": rows, "bad": bad}, indent=1))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
