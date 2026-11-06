"""#343: tools/training_export.py: aggregates per role and op, the truth list, no names and paths in the printout."""
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TOOL = ROOT / "tools" / "training_export.py"


def rec(op, frm, to, role="kick", refine=8, track="Secret Track"):
    return {"op": op, "from": frm, "to": to, "role": role, "refine": refine, "track": track,
            "project": {"id": "abc", "name": "Secret Song"}}


def write(path, records, extra=""):
    path.write_text("".join(json.dumps(r) + "\n" for r in records) + extra, encoding="utf-8")


def test_aggregates_the_truth_list_and_a_public_printout(tmp_path):
    log = tmp_path / "secret_dir" / "log.jsonl"
    log.parent.mkdir()
    write(log, [rec("move", 1000, 1000 + 3), rec("move", 2000, 2000 - 1), rec("move", 3000, 3000 + 500, refine=7),
                rec("add", None, 4000), rec("delete", 5000, None), rec("learn_this", 6000, None, role="snare")],
          extra="not json\n\n")
    out = tmp_path / "truth.json"
    p = subprocess.run([sys.executable, "-I", str(TOOL), str(log), "--out", str(out)], capture_output=True, text=True, check=True)
    assert "records: 6 (1 unreadable lines skipped)" in p.stdout
    assert "kick move: 3" in p.stdout
    assert "to-from (samples): -1 1, +2..+10 1, >=+100 1" in p.stdout
    assert "refine rule: 7 x1, 8 x2" in p.stdout
    assert "kick add: 1" in p.stdout and "snare learn_this: 1" in p.stdout
    assert "Secret" not in p.stdout and "secret_dir" not in p.stdout and str(tmp_path) not in p.stdout
    truth = json.loads(out.read_text())
    assert len(truth) == 6
    assert truth[0] == {"track": "Secret Track", "role": "kick", "from": 1000, "to": 1003, "op": "move"}
    assert truth[3] == {"track": "Secret Track", "role": "kick", "from": None, "to": 4000, "op": "add"}
    assert set(truth[0]) == {"track", "role", "from", "to", "op"}


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
    write(a, [rec("move", 0, 1)])
    write(b, [rec("move", 0, 0)])
    records, bad = te.read_records([a, b])
    assert (len(records), bad) == (2, 0)
    assert te.aggregate(records)["kick"]["move"]["n"] == 2
