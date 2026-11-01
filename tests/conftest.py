"""Shared fixtures: a kick WAV with known onsets; the JS coverage plugin (tests/jscov.py).

P3-12: the Python server is gone. The tests check the Rust core and the app against frozen
references (tests/frozen.py, tests/golden/) and run the page against the app's test bridge
(tests/e2e_app). The Rust tools they call are in tests/core_tools.py."""
import os

import pytest

from core_tools import PARITY, SERVICE_TOOL
from helpers import ROOT, synth_track, write_wav  # noqa: F401  (ROOT puts the repo on sys.path)

pytest_plugins = ["jscov"]   # app.js / backend.js / lanes-gl.js / tiles-worker.js coverage of tests/e2e_app


def pytest_collection_modifyitems(config, items):
    """#277: the media tests are off by default.

    `addopts = "-m 'not media'"` would be wrong: every explicit `-m` overrides addopts.
    Instead deselect the media items unless the `-m` expression names media, or
    DRUMSPIKE_MEDIA_TESTS=1 asks for them on purpose.
    """
    if os.environ.get("DRUMSPIKE_MEDIA_TESTS") == "1":
        return
    markexpr = config.getoption("markexpr") or ""
    if "media" in markexpr:
        return
    selected = []
    for item in items:
        if "media" in item.keywords:
            continue
        selected.append(item)
    if len(selected) != len(items):
        config.hook.pytest_deselected(items=[i for i in items if i not in selected])
        items[:] = selected


@pytest.fixture
def kick_wav(tmp_path):
    """Kick close-mic WAV with 6 isolated hits; returns (path, onset_samples)."""
    onsets = [0.25, 0.75, 1.25, 1.75, 2.25, 2.75]
    x, idx = synth_track(onsets, 3.5)
    return write_wav(tmp_path / "Kick In.wav", x), idx


def pytest_report_header(config):
    built = [p.name for p in (PARITY, SERVICE_TOOL) if p.is_file()]
    return f"drumspike Rust tools built: {', '.join(built) or 'none'} (cargo build --release -p drumspike-devtools)"
