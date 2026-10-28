"""#434 and #466: the MIDI timing check (`tools/check_midi_timing.py`) passes: the exported .mid
files hold the user's tempo in a DAW that applies only that tempo (118.000 within 1 ms over 6
minutes; 92.000 within 1 sample at 48 kHz over 5 minutes, the owner's session) and round-trip
through their own tempo events within the documented limit of a whole-microsecond set_tempo."""

import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def test_check_midi_timing_passes():
    r = subprocess.run(
        [sys.executable, str(ROOT / "tools" / "check_midi_timing.py")],
        capture_output=True,
        text=True,
        cwd=ROOT,
        timeout=1200,
    )
    assert r.returncode == 0, r.stdout[-2000:] + r.stderr[-800:]
    assert "5/5 cases pass" in r.stdout, r.stdout[-500:]
