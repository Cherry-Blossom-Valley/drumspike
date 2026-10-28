"""#434 and #466: the MIDI timing check. Exports .mid files through the real Rust export (the dev
binary `crates/core/src/bin/export-midi-case.rs`, built on demand with
`cargo build -p drumspike-core --bin export-midi-case`; a release binary can be given with
`--bin`), reads them back with mido and, per case, counts

* the round trip: every note's ticks converted to seconds with the file's own tempo events
  against the hit times of the same export, max error in ticks (drift case gate: <= 1 tick) or
  in samples at 48 kHz (92 BPM case gate: <= 2 samples, the documented limit of a whole-
  microsecond set_tempo);
* the DAW error: the same notes against a DAW that ignores the file's tempo events and applies
  only the single tempo the user set, max error in ms for the 118 BPM cases (gate: <= 1 ms over
  6 minutes) and in samples at 48 kHz for the 92.000 BPM cases (#466 gate: <= 1 sample over
  5 minutes; before the #466 fix the notes sat on the ticks of the written 652174 us, which a
  DAW at 92.000 plays up to 1.8 samples early).

The drift case (the default map, a caller tempo of 118 BPM, one hit per beat from 2.025 s to
6 minutes) FAILS the round trip on the code before the #434 fix: the old export computed the
ticks from the exact tempo while `int(6e7/bpm)` truncated the written `set_tempo` (16.4 ticks
over 6 minutes). The 92 BPM cases FAIL the DAW check before the #466 fix (ticks from the
rounded set_tempo). Counted output, exit 1 when a gate fails.

Usage: `uv run python tools/check_midi_timing.py [--bin path] [--keep]`
"""

import argparse
import json
import subprocess
import sys
import tempfile
from pathlib import Path

import mido

ROOT = Path(__file__).resolve().parents[1]
TARGET = Path(__import__("os").environ.get("CARGO_TARGET_DIR") or ROOT / "target")
PPQ = 24_000
BPM = 118.0
BAR1 = 2.025
N = 600  # one hit per beat from bar 1 to past 6 minutes

HITS = [BAR1 + i * (60.0 / BPM) for i in range(N)]
EVENTS = [{"t": t, "v": 100} for t in HITS]

# #466: the owner's session. 92.000 BPM, bar 1 at 0 s, hits on exact sample positions at 48 kHz
# from 2 s to 300 s every quarter beat, so a DAW at the user's tempo must land every note on
# its sample (within the 24000-PPQ tick rounding).
BPM92 = 92.0
SR = 48_000
BEAT92 = 60.0 / BPM92
HITS92 = []
_k = 0
while True:
    t = 2.0 + _k * 0.25 * BEAT92
    if t > 300.0:
        break
    HITS92.append(round(t * SR) / SR)
    _k += 1
EVENTS92 = [{"t": t, "v": 100} for t in HITS92]
N92 = len(HITS92)


def case(name):
    """The cases: the drift case (default map, caller tempo), the stored map, the multi-track
    file; and the #466 cases at 92.000 BPM with the sample-level DAW check."""
    yield name + ": single tempo 118 (default map)", {
        "kind": "single", "events": EVENTS, "note": 36, "ppq": PPQ,
        "map": None, "legacy_tempo": BPM,
    }, {"bpm": BPM, "hits": HITS, "daw_ms": True}
    yield name + ": stored map 118", {
        "kind": "single", "events": EVENTS, "note": 36, "ppq": PPQ,
        "map": {"bpm": BPM, "bar1_s": BAR1}, "legacy_tempo": 120.0,
    }, {"bpm": BPM, "hits": HITS, "daw_ms": True}
    yield name + ": multi-track, tempo 118 (default map)", {
        "kind": "multi", "note": 36, "ppq": PPQ, "map": None, "legacy_tempo": BPM,
        "tracks": [{"name": "Kick In", "note": 36, "events": EVENTS}],
    }, {"bpm": BPM, "hits": HITS, "daw_ms": True}
    yield name + ": single tempo 92 (default map, #466)", {
        "kind": "single", "events": EVENTS92, "note": 36, "ppq": PPQ,
        "map": None, "legacy_tempo": BPM92,
    }, {"bpm": BPM92, "hits": HITS92, "daw_ms": False}
    yield name + ": stored map 92 (default tempo, #466)", {
        "kind": "single", "events": EVENTS92, "note": 36, "ppq": PPQ,
        "map": {"bpm": BPM92, "bar1_s": 0.0}, "legacy_tempo": 120.0,
    }, {"bpm": BPM92, "hits": HITS92, "daw_ms": False}


def build_bin(debug_bin):
    """The dev binary, built into the debug target when it is not there yet."""
    if not debug_bin.is_file():
        r = subprocess.run(
            ["cargo", "build", "--quiet", "-p", "drumspike-core", "--bin", "export-midi-case"],
            cwd=ROOT, capture_output=True, text=True,
        )
        if r.returncode != 0:
            raise RuntimeError("cargo build failed: " + r.stderr[-800:])
    return debug_bin


def note_ons(path):
    """The note-on ticks of track 1 (the drum track) of a file."""
    mid = mido.MidiFile(path)
    ons = []
    for tr in mid.tracks:
        if not any(m.type == "note_on" for m in tr):
            continue
        at = 0
        for m in tr:
            at += m.time
            if m.type == "note_on" and m.velocity > 0:
                ons.append(at)
    return mid.ticks_per_beat, ons


def tempo_events(path):
    """(tick, us per quarter) of the file's own set_tempo events, sorted by tick."""
    mid = mido.MidiFile(path)
    evs = []
    for tr in mid.tracks:
        at = 0
        for m in tr:
            at += m.time
            if m.type == "set_tempo":
                evs.append((at, m.tempo))
    evs.sort()
    if not evs or evs[0][0] != 0:
        evs.insert(0, (0, 500_000))
    return evs


def tick_to_s(tick, tempos, ppq):
    """The time of a tick through the file's own tempo events, in whole microseconds."""
    s = 0.0
    last_t, last_us = 0, tempos[0][1]
    for t, us in tempos:
        if t >= tick:
            break
        s += (t - last_t) * last_us / (ppq * 1e6)
        last_t, last_us = t, us
    return s + (tick - last_t) * last_us / (ppq * 1e6)


def integer_limit_samples(bpm, ppq, t_max):
    """#466: what a whole-microsecond `set_tempo` costs in the round trip through the file's own
    events: `t_max` seconds of the relative mismatch of that integer against the exact tempo
    (6e7 / bpm), plus half a tick of note rounding, in samples at 48 kHz. The mismatch alone is
    the ticket's 1.9 samples at 300 s and 92.000 BPM; the note rounding adds up to half a tick
    (0.65 samples at 24000 PPQ) on top, so the round-trip gate here is the sum."""
    exact = 6e7 / bpm
    us = round(exact)
    return (t_max * abs(us - exact) / exact + 0.5 * us / (ppq * 1e6)) * SR


def integer_limit_ticks(bpm, ppq, t_max):
    """The same limit in ticks of the written tempo (the #434 drift cases)."""
    exact = 6e7 / bpm
    us = round(exact)
    return 0.5 + t_max * abs(us - exact) / exact * (ppq * 1e6) / us


def check_case(label, job, tmp, bin_path, ctx):
    """One case: export, read back, the round trip and the DAW error. Returns (ok, lines)."""
    path = tmp / (label.split(":")[0].replace(" ", "_").replace(",", "").replace("#", "") + ".mid")
    job = dict(job, path=str(path))
    jf = tmp / "job.json"
    jf.write_text(json.dumps(job))
    r = subprocess.run([str(bin_path), str(jf)], capture_output=True, text=True)
    if r.returncode != 0:
        return False, [f"FAIL {label}: the export failed: {r.stderr[-300:]}"]
    ppq, ons = note_ons(path)
    hits, bpm = ctx["hits"], ctx["bpm"]
    if len(ons) != len(hits):
        return False, [f"FAIL {label}: {len(ons)} notes, expected {len(hits)}"]
    tempos = tempo_events(path)
    own = [tick_to_s(t, tempos, ppq) for t in ons]
    rt = [abs(t - h) for t, h in zip(own, hits)]
    daw = [abs(t * 60.0 / (bpm * ppq) - h) for t, h in zip(ons, hits)]
    if ctx["daw_ms"]:
        # the #434 cases: ticks for the round trip, ms for the DAW at 118.000; the round-trip
        # gate is the documented integer-set_tempo limit (#466), no longer 1 tick
        rt_gate_limit = integer_limit_ticks(bpm, ppq, 307.0)
        rt_gate = max(rt) * ppq * 1e6 / tempos[0][1]
        daw_err = max(daw) * 1e3
        rt_ok = rt_gate <= rt_gate_limit
        daw_ok = daw_err <= 1.0
        lines = [
            f"{'PASS' if rt_ok else 'FAIL'} {label}: round trip {rt_gate:.3f} ticks (gate <= {rt_gate_limit:.1f}, integer set_tempo limit)",
            f"{'PASS' if daw_ok else 'FAIL'} {label}: DAW at 118.000 {daw_err:.4f} ms (gate <= 1 ms over 6 min)",
        ]
    else:
        # the #466 cases: samples at 48 kHz for both, against a DAW at the user's 92.000
        rt_gate_limit = integer_limit_samples(bpm, ppq, 300.0)
        rt_gate = max(rt) * SR
        daw_err = max(daw) * SR
        rt_ok = rt_gate <= rt_gate_limit
        daw_ok = daw_err <= 1.0
        lines = [
            f"{'PASS' if rt_ok else 'FAIL'} {label}: round trip {rt_gate:.3f} samples (gate <= {rt_gate_limit:.2f}, integer set_tempo limit)",
            f"{'PASS' if daw_ok else 'FAIL'} {label}: DAW at 92.000 {daw_err:.3f} samples (gate <= 1)",
        ]
    return rt_ok and daw_ok, lines


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--bin", type=Path, default=TARGET / "debug" / "export-midi-case",
                    help="the export-midi-case binary (default: built into target/debug)")
    ap.add_argument("--keep", action="store_true", help="keep the scratch folder")
    args = ap.parse_args()
    bin_path = build_bin(args.bin)
    n = ok = 0
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        for label, job, ctx in case("drift"):
            n += 1
            good, lines = check_case(label, job, tmp, bin_path, ctx)
            ok += good
            print("\n".join(lines))
    print(
        f"\n{ok}/{n} cases pass (the 118 BPM cases gate the #434 round trip, the 92 BPM cases"
        " the #466 DAW check; both fail on the code before their fix)"
    )
    return 0 if ok == n else 1


if __name__ == "__main__":
    sys.exit(main())
