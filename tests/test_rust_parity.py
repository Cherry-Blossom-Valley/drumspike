"""Phase 1 parity: the Rust core (crates/core) against engine.py on the synthetic corpus.

P3-12: the engine.py side is frozen (tests/golden/frozen/: parity_io, refine, corpus_events, midi),
the checks read it. Runs when the release parity binary exists (`cargo build --release -p
drumspike-devtools`); skipped otherwise. The corpus is written to the test's tmp folder (no audio in
the repo); its WAV files must have the SHA-256 the frozen inputs name.
"""
import sys
from pathlib import Path

import pytest

from helpers import ROOT  # noqa: F401  (puts the repo on sys.path)

PARITY_DIR = ROOT / "tools" / "parity"
PARITY_BIN = ROOT / "target" / "release" / "parity"
sys.path.insert(0, str(PARITY_DIR))

pytestmark = pytest.mark.skipif(not PARITY_BIN.exists(),
                                reason="parity binary not built: cargo build --release -p drumspike-devtools")


def test_rust_io_peaks_and_velocities_match_engine(tmp_path, capsys):
    import check_parity
    import make_corpus

    sys.argv = ["make_corpus.py", str(tmp_path)]
    make_corpus.main()
    rc = check_parity.main([str(tmp_path), "--bin", str(PARITY_BIN), "--json", str(tmp_path / "parity.json")])
    out = capsys.readouterr().out
    assert rc == 0, out
    assert "FAIL" not in out
    assert "/" in out.splitlines()[-1] and out.splitlines()[-1].endswith("cases PASS")


def test_rust_refinement_matches_engine(tmp_path, capsys):
    """P1-4: refined times and velocities on the corpus and the refine_bench scenarios.

    The check passes a hit that is within 1 sample; the assertion below demands that every
    hit is identical, so a floating-point-order difference shows up here, not in a report.
    """
    import json

    import check_refine

    rc = check_refine.main([str(tmp_path), "--bin", str(PARITY_BIN), "--json", str(tmp_path / "refine.json")])
    out = capsys.readouterr().out
    assert rc == 0, out
    assert "FAIL" not in out
    res = json.loads((tmp_path / "refine.json").read_text())
    assert isinstance(res, dict) and res["pass"] == res["total"] > 0, list(res)[:5]
    assert res["totals"]["identical"] == res["totals"]["hits"] > 1000, res["totals"]
    assert res["totals"]["velocities_exact"] == res["totals"]["hits"], res["totals"]


def test_rust_midi_and_spike_export_match_engine(tmp_path, capsys):
    """P1-5: MIDI files byte-identical to the frozen files of engine.export_midi (and the note messages
    mido read from them), spike WAVs with a full-scale blip per hit and the source's bext
    time reference, read back with Python's wave module and the Rust reader."""
    import json

    import check_midi

    rc = check_midi.main([str(tmp_path), "--bin", str(PARITY_BIN), "--json", str(tmp_path / "midi.json")])
    out = capsys.readouterr().out
    assert rc == 0, out
    assert "FAIL" not in out
    res = json.loads((tmp_path / "midi.json").read_text())
    assert isinstance(res, dict) and res["pass"] == res["total"] > 0, list(res)[:5]
    assert res["midi_bytes_identical"] == res["midi_cases"] > 20, res
    assert len(res["cases"]["spike"]) >= 3
