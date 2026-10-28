"""#378: the audio-spike snippet lengths per role on ENST-Drums.

    cargo build --release -p drumspike-core --example spike_len
    uv run --with numpy --with soundfile --with mido --with mir_eval \
        python tools/eval/spike_len.py          # medians per role over every item
    ENST_LIMIT=2 python tools/eval/spike_len.py # a smoke run

Data and rules are the ones of tools/eval/enst.py (imported): ENST_ROOT, the item index, the
tom label to mic mapping. Per close mic (kick, snare, the tom mics) and item, the annotated
onsets of the mic's drum go through `target/release/examples/spike_len` (drumspike-core, the
real export rule `audio_snippet_lengths`): the median snippet length per mic and per role
(kick, snare, tom pooled). The tool FAILs when the kick median is under 15 ms (#378: the
snippet must cover 1.5 cycles of the LOW tone, not only the attack click). Aggregate numbers
only, never audio or derived per-hit data.

With a defect before #378 the kick snippets sat on the attack click (a few ms), so the gate
FAILs on main and passes with the fix.
"""

import json
import os
import statistics
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools" / "eval"))
import enst  # noqa: E402  (the #276 harness: index, mapping)

EXAMPLE = ROOT / "target" / "release" / "examples" / "spike_len"
KICK_MIN_MS = 15.0
ROLES = ("kick", "snare", "tom")


def mic_role(mic):
    """The role of a close mic: the tom mics pool into `tom`."""
    if mic == "kick":
        return "kick"
    if mic == "snare":
        return "snare"
    return "tom"


def run_example(wav, times):
    """One wav + hit times through the spike_len example: [length ms]."""
    args = [str(EXAMPLE), "--wav", str(wav), "--times", ",".join(f"{t:.6f}" for t in times)]
    r = subprocess.run(args, capture_output=True, text=True)
    if r.returncode != 0:
        raise RuntimeError(f"spike_len failed ({r.returncode}): {r.stderr[-400:]}")
    return json.loads(r.stdout.strip().splitlines()[-1])["lengths_ms"]


def do_run(root, log=print):
    """Median snippet length per mic and role over every item of every drummer."""
    if not EXAMPLE.is_file():
        raise SystemExit(
            f"FAIL: {EXAMPLE} is missing: cargo build --release -p drumspike-core --example spike_len"
        )
    idx = enst.index_items(root)
    tom_maps = {d: enst.tom_mapping(root, d) for d in enst.DRUMMERS}
    limit = int(os.environ.get("ENST_LIMIT", "0") or 0)
    per_mic = {}
    n = 0
    for drummer, items in idx.items():
        for stem, item in items.items():
            if limit and n >= limit:
                break
            present = [m for m in enst.CLOSE_MICS if m in item["wav"] and item["wav"][m].is_file()]
            refs = enst.item_refs(enst.read_annotation(item["ann"]), tom_maps[drummer])
            for mic in present:
                if mic not in ("kick", "snare") and not mic.startswith("tom"):
                    continue
                times = sorted(t for t, _ in refs.get(mic, []))
                if not times:
                    continue
                try:
                    lens = run_example(item["wav"][mic], times)
                except RuntimeError as e:
                    log(f"WARN {drummer}/{stem}/{mic}: {e}")
                    continue
                per_mic.setdefault(mic, []).extend(lens)
                n += 1
        if limit and n >= limit:
            break
    if not per_mic:
        raise SystemExit("FAIL: no mic produced lengths (ENST_ROOT set? wav files present?)")
    log(f"items with at least one mic: {n}")
    per_role = {}
    for mic in sorted(per_mic):
        lens = sorted(per_mic[mic])
        role = mic_role(mic)
        per_role.setdefault(role, []).extend(lens)
        log(f"{mic:8s} hits {len(lens):5d}  median {statistics.median(lens):7.2f} ms"
            f"  p10 {lens[int(0.10 * (len(lens) - 1))]:7.2f}  p90 {lens[int(0.90 * (len(lens) - 1))]:7.2f}")
    log("")
    for role in ROLES:
        if role not in per_role:
            raise SystemExit(f"FAIL: no lengths for role {role}")
        log(f"{role:8s} median {statistics.median(per_role[role]):7.2f} ms over {len(per_role[role])} hits")
    kick = statistics.median(per_role["kick"])
    if kick < KICK_MIN_MS:
        raise SystemExit(f"FAIL: the kick median snippet is {kick:.2f} ms, below {KICK_MIN_MS} ms (#378)")
    log(f"PASS: the kick median snippet is {kick:.2f} ms >= {KICK_MIN_MS} ms")


def main(argv=None):
    root = os.environ.get("ENST_ROOT")
    if not root:
        raise SystemExit("FAIL: ENST_ROOT is not set (a missing root FAILs, #276)")
    do_run(Path(root))


if __name__ == "__main__":
    main()
