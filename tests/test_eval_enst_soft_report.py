"""The soft-gate report (#81): the row shape, the counted lines and the verdict."""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools" / "eval"))

import enst_soft_report  # noqa: E402


def test_per_rows_pools_the_tom_mics_and_the_pooled_rows():
    # (config, drummer, mic): the tom mics pool into one tom row, the pooled rows sum the drummers.
    per = {("dsp", "drummer_1", "snare"): {"ref": [1.0, 2.0], "est": [1.0, 5.0]},
           ("dsp", "drummer_1", "tom_1"): {"ref": [3.0], "est": [3.0]},
           ("dsp", "drummer_1", "tom_2"): {"ref": [4.0], "est": []},
           ("dsp", "drummer_2", "snare"): {"ref": [1.5], "est": [1.5]}}
    rows = enst_soft_report.per_rows(per)
    assert set(rows) == {"drummer_1|snare", "drummer_1|tom", "drummer_2|snare", "pooled|snare",
                         "pooled|tom"}
    p, r, f = rows["pooled|snare"]
    # ref [1.0, 2.0, 1.5] vs est [1.0, 5.0, 1.5]: two of three matched both ways
    assert r == 2 / 3 and p == 2 / 3 and abs(f - 2 / 3) < 1e-9
    # the tom mics pooled: one of the two reference hits matched
    assert rows["pooled|tom"][1] == 0.5


def test_compare_counts_and_decides_on_the_pooled_snare():
    hard = {"pooled|snare": (0.99, 0.80, 0.88), "pooled|kick": (1.0, 1.0, 1.0)}
    # recall up, precision at the floor: PASS
    soft = {"pooled|snare": (0.96, 0.90, 0.93), "pooled|kick": (1.0, 1.0, 1.0)}
    lines, ok = enst_soft_report.compare(hard, soft)
    assert ok and "PASS" in lines[-1]
    assert sum(1 for ln in lines if ln.startswith("pooled|")) == 2
    # a soft precision under the floor fails even with the recall up
    low_p = dict(soft, **{"pooled|snare": (0.90, 0.90, 0.90)})
    lines, ok = enst_soft_report.compare(hard, low_p)
    assert not ok and "FAIL" in lines[-1]
    # equal recall is not strictly above
    equal = dict(soft, **{"pooled|snare": (0.99, 0.80, 0.88)})
    lines, ok = enst_soft_report.compare(hard, equal)
    assert not ok and "strictly above: False" in "\n".join(lines)
    # a missing row is counted and fails
    lines, ok = enst_soft_report.compare({"pooled|snare": hard["pooled|snare"]}, soft)
    assert not ok and any(ln.startswith("MISSING pooled|kick") for ln in lines)
