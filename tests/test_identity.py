"""Tests of the identity snapshot compare logic (tools/identity.py) on small synthetic manifests.

They need no corpus and no binary. The planted faults prove the gate can fail: one velocity changed, one item
missing, too few items, a reordered key. The first test proves the clean case passes, so a compare that always
says "different" would fail here and one that always says "identical" would fail in the others.
"""
import json
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))
import identity  # noqa: E402


def events(n=5, velocity=100):
    return [{"t": 0.5 * i, "v": velocity, "bleed": False, "extra": {"flam": True}} for i in range(n)]


def item(evs, midi=b"MThd"):
    return identity.make_item({"events": identity.events_sub(evs), "midi": identity.sha(midi)}, {"events": len(evs)})


def manifest(n_items=4, **change):
    """A manifest with n_items per part; `change` = {key: item} replaces items of the part `golden`."""
    parts = {}
    for part in identity.PARTS:
        items = {f"{part}-{i}": item(events(3 + i)) for i in range(n_items)}
        parts[part] = {"items": items}
    parts["golden"]["items"].update(change)
    return {"format": identity.FORMAT, "meta": {"tool_sha256": "x", "env": {}}, "parts": parts}


MIN = {"golden": 4, "enst": 4, "sessions": 4}


def run(a, b, **kw):
    kw.setdefault("minimum", MIN)
    return identity.compare(a, b, **kw)


def test_same_manifest_is_identical():
    code, lines = run(manifest(), manifest())
    assert code == 0, lines
    assert any(l.startswith("golden: compared 4 items, 0 differ") for l in lines)
    assert lines[-1] == "identical"


def test_one_velocity_changed_fails():
    changed = events(3)
    changed[1]["v"] = 101
    code, lines = run(manifest(), manifest(**{"golden-0": item(changed)}))
    assert code == 1
    assert any("compared 4 items, 1 differ" in l for l in lines)
    assert any(l.startswith("  DIFF golden-0: events") for l in lines)


def test_midi_byte_changed_fails_and_names_the_midi():
    code, lines = run(manifest(), manifest(**{"golden-1": item(events(4), midi=b"MThe")}))
    assert code == 1
    assert any("DIFF golden-1: midi" in l for l in lines)


def test_mark_absent_vs_false_fails():
    a = events(3)
    b = events(3)
    del b[0]["bleed"]                      # absent is not the same as false
    code, _ = run(manifest(**{"golden-0": item(a)}), manifest(**{"golden-0": item(b)}))
    assert code == 1


def test_key_order_matters():
    a = [{"t": 1.0, "v": 90}]
    b = [{"v": 90, "t": 1.0}]
    code, _ = run(manifest(**{"golden-0": item(a)}), manifest(**{"golden-0": item(b)}))
    assert code == 1


def test_event_order_matters():
    a = [{"t": 1.0, "v": 90}, {"t": 2.0, "v": 80}]
    code, _ = run(manifest(**{"golden-0": item(a)}), manifest(**{"golden-0": item(a[::-1])}))
    assert code == 1


def test_missing_item_fails():
    b = manifest()
    del b["parts"]["enst"]["items"]["enst-2"]
    code, lines = run(manifest(), b)
    assert code == 1
    assert any("MISSING in B: enst-2" in l for l in lines)


def test_extra_item_fails():
    b = manifest()
    b["parts"]["sessions"]["items"]["sessions-new"] = item(events(2))
    code, lines = run(manifest(), b)
    assert code == 1
    assert any("MISSING in A: sessions-new" in l for l in lines)


def test_too_few_items_fails_even_when_equal():
    code, lines = run(manifest(2), manifest(2))
    assert code == 1
    assert any("only 2 items compared, at least 4 wanted" in l for l in lines)


def test_empty_part_fails():
    code, lines = run(manifest(0), manifest(0))
    assert code == 1
    assert any("only 0 items compared" in l for l in lines)


def test_default_minimum_fails_a_small_manifest():
    code, _ = identity.compare(manifest(4), manifest(4))        # the default minimum wants the real corpus
    assert code == 1


def test_part_missing_fails():
    b = manifest()
    del b["parts"]["sessions"]
    code, lines = run(manifest(), b)
    assert code == 1
    assert any("part missing in manifest B" in l for l in lines)


def test_parts_filter_compares_only_the_named_part():
    b = manifest()
    del b["parts"]["sessions"]
    code, lines = run(manifest(), b, parts=["golden"])
    assert code == 0, lines
    assert not any(l.startswith("sessions:") for l in lines)


def test_bad_manifest_fails():
    code, lines = run({"format": 99, "parts": {}}, manifest())
    assert code == 1 and "FAIL manifest A" in lines[0]


def test_counts_in_the_difference_line():
    code, lines = run(manifest(), manifest(**{"golden-0": item(events(7))}))
    assert code == 1
    assert any("counts: events 3->7" in l for l in lines)


def test_cli_exit_codes(tmp_path, capsys):
    a, b, c = tmp_path / "a.json", tmp_path / "b.json", tmp_path / "c.json"
    a.write_text(json.dumps(manifest()))
    b.write_text(json.dumps(manifest()))
    changed = events(3)
    changed[2]["v"] = 7
    c.write_text(json.dumps(manifest(**{"golden-3": item(changed)})))
    mins = [x for part in identity.PARTS for x in ("--min", f"{part}=4")]
    assert identity.main(["compare", str(a), str(b), *mins]) == 0
    assert identity.main(["compare", str(a), str(c), *mins]) == 1
    assert "DIFF golden-3" in capsys.readouterr().out
    assert identity.main(["compare", str(a), str(b)]) == 1          # default minimum
    with pytest.raises(SystemExit):
        identity.main(["compare", str(a), str(tmp_path / "missing.json")])


def test_make_item_depends_on_every_sub_hash():
    a = identity.make_item({"events": "aa", "midi": "bb"})
    assert a == identity.make_item({"midi": "bb", "events": "aa"})
    assert a["sha256"] != identity.make_item({"events": "aa"})["sha256"]
    assert a["sha256"] != identity.make_item({"events": "aa", "midi": "bc"})["sha256"]
