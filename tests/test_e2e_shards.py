"""#125: the e2e shard parts "<name>@<i>/<n>" split a shard exactly (no file lost, none twice)."""

import importlib.util
import json
import sys
from pathlib import Path

import pytest

E2E = Path(__file__).resolve().parent / "e2e_app"


@pytest.fixture(scope="module")
def shard_selects():
    sys.path.insert(0, str(E2E))
    try:
        spec = importlib.util.spec_from_file_location("e2e_conftest_for_shards", E2E / "conftest.py")
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
    finally:
        sys.path.remove(str(E2E))
    return mod.shard_selects


@pytest.mark.parametrize("n", [1, 2, 3, 4])
def test_the_parts_of_rest_split_it_exactly(shard_selects, n):
    files = sorted(p.name for p in E2E.glob("test_*.py"))
    rest = [f for f in files if shard_selects(f, "rest", files)]
    parts = [[f for f in files if shard_selects(f, f"rest@{i}/{n}", files)] for i in range(1, n + 1)]
    assert sorted(f for p in parts for f in p) == rest
    assert all(parts), parts   # no empty part
    named = json.loads((E2E.parent / "e2e_shards.json").read_text())
    assert not set(rest) & set(named["x86"])


def test_a_bad_part_is_refused(shard_selects):
    files = sorted(p.name for p in E2E.glob("test_*.py"))
    with pytest.raises(ValueError):
        shard_selects(files[0], "rest@4/3", files)
