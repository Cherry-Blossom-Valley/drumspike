"""P3-8a: the generator of apps/desktop/THIRD_PARTY.md (licence rule B5) and the committed file."""

import importlib.util
import json
import re
import shutil
import subprocess
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "apps" / "desktop" / "scripts" / "gen_third_party.py"


@pytest.fixture(scope="module")
def gen():
    spec = importlib.util.spec_from_file_location("gen_third_party", SCRIPT)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def choose(gen, expr):
    return gen.choose(gen.parse(gen.tokens(expr)))


def test_an_or_expression_takes_the_preferred_approved_licence(gen):
    assert choose(gen, "MIT OR Apache-2.0") == ["MIT"]
    assert choose(gen, "Apache-2.0 OR GPL-3.0") == ["Apache-2.0"]
    assert choose(gen, "(MIT OR Apache-2.0) AND Unicode-3.0") == ["MIT", "Unicode-3.0"]
    assert choose(gen, "Apache-2.0 WITH LLVM-exception") == ["Apache-2.0"]
    assert choose(gen, "MIT/Apache-2.0") == ["MIT"]


def test_mpl_is_approved_and_other_licences_stop(gen):
    assert choose(gen, "MPL-2.0") == ["MPL-2.0"]
    assert choose(gen, "GPL-3.0") is None
    assert choose(gen, "LGPL-2.1 OR GPL-2.0") is None
    assert choose(gen, "MIT AND GPL-3.0") is None


def test_the_unlicense_is_approved_by_the_owner(gen):
    assert "Unlicense" in gen.APPROVED
    assert choose(gen, "Unlicense") == ["Unlicense"]
    assert gen.PENDING == set()


@pytest.mark.skipif(shutil.which("cargo") is None, reason="needs cargo")
@pytest.mark.skipif(sys.platform != "darwin",
                    reason="off macOS: the macOS-only build-dependencies (swift-rs, base64 0.21.7) "
                           "resolve only on a macOS host, so the committed list (generated on a Mac) "
                           "looks stale; the Apple targets themselves resolve fine")
def test_the_committed_list_is_current_and_its_count_equals_cargo(gen):
    r = subprocess.run(["uv", "run", "--no-project", "--python", "3.12", str(SCRIPT), "--check"],
                       cwd=ROOT, capture_output=True, text=True)
    assert r.returncode == 0, r.stderr
    n = int(subprocess.run(["uv", "run", "--no-project", "--python", "3.12", str(SCRIPT), "--count"],
                           cwd=ROOT, capture_output=True, text=True, check=True).stdout)
    text = (ROOT / "apps" / "desktop" / "THIRD_PARTY.md").read_text(encoding="utf-8")
    assert int(re.search(r"## Crates \((\d+)\)", text).group(1)) == n
    rows = re.findall(r"^\| ([^|]+) \| (\d[^|]*) \| ", text, re.M)
    assert len(rows) == n
    assert "Mozilla Public License Version 2.0" in text
