"""#443: tools/shape_measure.py measures the track-shape placement on generated rows, no internal data.

The planted defect of the Gate: a hit whose rule position is 3 samples off must show as
"differ by 2 or more" in the agreement table. A set with 0 hits makes the report FAIL."""
import csv
import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "tools"))
import shape_measure  # noqa: E402

PRE, POST, UP = 16, 8, 4


def stroke_window(at):
    """A 960-sample window at 48 kHz: a 0.01 line, a small lead swing and a stroke peak after `at`.

    The foot (the first motion off the line) is at `at`; the template matches it at lag 0."""
    x = np.full(960, 0.01)
    x[at - 3:at] = [0.012, 0.008, 0.006]          # the swing of the other sign before the edge
    x[at] = 0.02
    x[at + 1:at + 40] = np.linspace(0.05, 1.0, 39)
    x[at + 40:at + 240] = np.linspace(1.0, 0.4, 200)
    x[at + 240:] = 0.3
    return x


def write_data(tmp_path, rows):
    """Rows to DATA_DIR: fit_v10.csv (i, session, role, cur_off, label_off, owner, confirmed) + .f32."""
    x = np.zeros((len(rows), 960), dtype="<f4")
    with open(Path(tmp_path) / "fit_v10.csv", "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=["i", "session", "role", "cur_off", "label_off", "owner", "confirmed"])
        w.writeheader()
        for n, r in enumerate(rows):
            x[n] = stroke_window(480)
            w.writerow({"i": n, "session": r.get("session", "Session A"), "role": r["role"],
                        "cur_off": r["cur_off"], "label_off": r["label_off"],
                        "owner": r.get("owner", "0"), "confirmed": r.get("confirmed", "1")})
    x.tofile(Path(tmp_path) / "fit_v10.f32")


def base_rows(n=20, role="kick"):
    """Gold rows placed exactly: the rule at the label."""
    return [{"role": role, "cur_off": "0", "label_off": "0"} for _ in range(n)]


def test_the_shape_places_what_the_rule_placed_and_the_agreement_table_counts_the_defect(tmp_path, capsys):
    """A row whose rule position is 3 samples late of the label: the rule errs by 3, the shape still
    finds the foot, so the two differ by 3 - the "differ by 2+" group, with the rule's error in it."""
    rows = base_rows()
    rows.append({"role": "kick", "cur_off": "3", "label_off": "0"})
    write_data(tmp_path, rows)
    res = shape_measure.measure(shape_measure.load_rows(tmp_path, PRE, POST), pre=PRE, post=POST)
    kick = res["sets"]["kick"]
    assert kick["rule"]["n"] == 21 and kick["shape"]["n"] == 21
    assert kick["rule"]["exact"] != 100.0
    assert kick["shape"]["exact"] == 100.0        # the shape finds the foot of the moved hit too
    ag = res["agreement"]["kick differ by 2+"]
    assert ag["n"] == 1 and ag["rule_exact"] == 0.0    # the rule errs there by 3, not exactly
    assert res["agreement"]["kick agree"]["n"] == 20


def test_unclear_line_and_owner_rows_get_their_own_sets(tmp_path):
    rows = base_rows(10)
    rows += [{"role": "kick", "cur_off": "0", "label_off": "0", "owner": "1"} for _ in range(5)]
    write_data(tmp_path, rows)
    res = shape_measure.measure(shape_measure.load_rows(tmp_path, PRE, POST), pre=PRE, post=POST)
    assert res["sets"]["kick"]["rule"]["n"] == 15
    assert res["sets"]["owner kick"]["rule"]["n"] == 5


def test_a_line_with_0_hits_fails_the_report():
    res = {"sets": {"kick": {"rule": {"n": 0}}}, "agreement": {}, "tracks": []}
    assert shape_measure.print_report(res, out=sys.stderr) == 1


def test_load_rows_reads_the_arguments_only(tmp_path):
    """The tool refuses an empty data dir loudly and reads whatever dir it is given."""
    try:
        shape_measure.load_rows(tmp_path, PRE, POST)
        raised = False
    except SystemExit:
        raised = True
    assert raised
    write_data(tmp_path, base_rows(3))
    rows = shape_measure.load_rows(tmp_path, PRE, POST)
    assert len(rows) == 3 and rows[0]["rule"] == 480 and rows[0]["label"] == 480
