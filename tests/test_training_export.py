"""#343: tools/training_export.py: aggregates per role and op, per precision (#353), the truth list, no names and
paths in the printout."""
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TOOL = ROOT / "tools" / "training_export.py"


def rec(op, frm, to, role="kick", refine=8, track="Secret Track", precision=None):
    return {"op": op, "from": frm, "to": to, "role": role, "refine": refine, "track": track,
            "project": {"id": "abc", "name": "Secret Song"}, "precision": precision}


def write(path, records, extra=""):
    path.write_text("".join(json.dumps(r) + "\n" for r in records) + extra, encoding="utf-8")


def test_aggregates_the_truth_list_and_a_public_printout(tmp_path):
    log = tmp_path / "secret_dir" / "log.jsonl"
    log.parent.mkdir()
    write(log, [rec("move", 1000, 1000 + 3, precision="ms"), rec("move", 2000, 2000 - 1, precision="sample"),
                rec("move", 3000, 3000 + 500, refine=7, precision="ms"),
                rec("add", None, 4000, precision="sample"), rec("delete", 5000, None, precision="ms"),
                rec("learn_this", 6000, None, role="snare")],
          extra="not json\n\n")
    out = tmp_path / "truth.json"
    p = subprocess.run([sys.executable, "-I", str(TOOL), str(log), "--out", str(out)], capture_output=True, text=True, check=True)
    assert "records: 6 (1 unreadable lines skipped)" in p.stdout
    assert "kick move: 3" in p.stdout
    assert "to-from (samples): -1 1, +2..+10 1, >=+100 1" in p.stdout
    # the per-precision counts and histograms (#353)
    assert "precision sample: 1, to-from: -1 1" in p.stdout
    assert "precision ms: 2, to-from: +2..+10 1, >=+100 1" in p.stdout
    assert "precision -: 1" in p.stdout
    assert "refine rule: 7 x1, 8 x2" in p.stdout
    assert "kick add: 1" in p.stdout and "snare learn_this: 1" in p.stdout
    assert "kick add: 1\n  precision sample: 1" in p.stdout   # an add has no to-from histogram
    assert "Secret" not in p.stdout and "secret_dir" not in p.stdout and str(tmp_path) not in p.stdout
    truth = json.loads(out.read_text())
    assert len(truth) == 6
    assert truth[0] == {"track": "Secret Track", "role": "kick", "from": 1000, "to": 1003, "op": "move", "precision": "ms"}
    assert truth[3] == {"track": "Secret Track", "role": "kick", "from": None, "to": 4000, "op": "add", "precision": "sample"}
    assert truth[5]["precision"] is None
    assert set(truth[0]) == {"track", "role", "from", "to", "op", "precision"}


def test_a_confirm_record_counts_per_role_and_precision_and_stays_in_the_truth_list(tmp_path):
    """#354: a confirm record (from = to, the hit reviewed right where it is) counts per role, carries its
    precision when the view was sent, and is kept in the truth list with op confirm."""
    log = tmp_path / "log.jsonl"
    write(log, [rec("move", 1000, 1003, precision="sample"),
                rec("confirm", 48000, 48000, role="snare", precision="sample"),
                rec("confirm", 72000, 72000, precision=None)])
    out = tmp_path / "truth.json"
    p = subprocess.run([sys.executable, "-I", str(TOOL), str(log), "--out", str(out)], capture_output=True, text=True, check=True)
    assert "records: 3" in p.stdout
    assert "kick move: 1" in p.stdout
    assert "snare confirm: 1" in p.stdout and "kick confirm: 1" in p.stdout
    assert "to-from (samples): 0 1" in p.stdout   # from = to: the bin 0
    assert "precision sample: 1, to-from: 0 1" in p.stdout
    assert "precision -: 1" in p.stdout
    truth = json.loads(out.read_text())
    assert len(truth) == 3
    assert {"track": "Secret Track", "role": "snare", "from": 48000, "to": 48000, "op": "confirm", "precision": "sample"} in truth
    assert truth[2]["op"] == "confirm" and truth[2]["from"] == 72000 and truth[2]["to"] == 72000


def test_two_logs_and_the_bins_edges(tmp_path):
    sys.path.insert(0, str(ROOT / "tools"))
    try:
        import training_export as te
    finally:
        sys.path.pop(0)
    assert [te.delta_bin(d) for d in (-500, -100, -99, -11, -10, -2, -1, 0, 1, 2, 10, 11, 99, 100)] == [
        "<=-100", "<=-100", "-99..-11", "-99..-11", "-10..-2", "-10..-2", "-1", "0", "+1", "+2..+10", "+2..+10",
        "+11..+99", "+11..+99", ">=+100"]
    a, b = tmp_path / "a.jsonl", tmp_path / "b.jsonl"
    write(a, [rec("move", 0, 1, precision="sample")])
    write(b, [rec("move", 0, 0, precision="ms")])
    records, bad = te.read_records([a, b])
    assert (len(records), bad) == (2, 0)
    agg = te.aggregate(records)
    assert agg["kick"]["move"]["n"] == 2
    assert agg["kick"]["move"]["prec"]["sample"]["n"] == 1 and agg["kick"]["move"]["prec"]["ms"]["n"] == 1
    # an unknown precision in a record reads as `-` (an older log, a Learn mark)
    assert te.precision_of({"precision": "px"}) == "-" and te.precision_of({}) == "-"
