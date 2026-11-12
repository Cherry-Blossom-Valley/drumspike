"""#494: the check against old clip sources (tools/media/compose.py).

The gate: with the manifest current (tools/media/clip-sources.json), the check passes; on a
planted old source — an explainer file whose sha256 is not the one the manifest names — it
FAILs, naming the scene, the file and both digests; an explainer missing from the manifest
fails too. Recordings are not listed and pass unchecked.

The manifest is tied to the checkout: it also holds the sha256 of every listed explainer's
script and builder, and a plain pytest (no media marker, runs in CI) fails with the file name
when one of them differs — an explainer rebuilt on any host turns CI red until the manifest
gets the new build. A test of that test plants a changed builder and proves the red.

The output lands atomically: a failed ffmpeg run removes the temporary file and leaves the
previous output exactly as it was.
"""
import hashlib
import json
import sys
from pathlib import Path

import pytest

from helpers import ROOT

sys.path.insert(0, str(ROOT / "tools" / "media"))
import compose as cp  # noqa: E402

MANIFEST = json.loads((ROOT / "tools" / "media" / "clip-sources.json").read_text(encoding="utf-8"))


def plan_with(source):
    return {"scenes": [{"id": "pv-exact", "clips": [{"source": str(source), "start": 1.0, "end": 2.0}]}]}


def write_explainer(tmp_path, name, payload):
    p = tmp_path / name
    p.write_bytes(payload)
    return p


def _tmp_manifest(tmp_path, explainers):
    p = tmp_path / "clip-sources.json"
    p.write_text(json.dumps({"explainers": explainers}), encoding="utf-8")
    return p


def test_manifest_lists_the_current_builds():
    for name, entry in MANIFEST["explainers"].items():
        assert name.startswith("explainer-") and len(entry["file"]) == 64
        assert Path(entry["script"]["path"]).is_file() and len(entry["script"]["sha256"]) == 64
        assert Path(entry["builder"]["path"]).is_file() and len(entry["builder"]["sha256"]) == 64


def test_manifest_matches_the_checkout():
    """The plain CI check: the script and the builder of every listed explainer are exactly the
    builds the manifest names. Red here means: rebuild the explainer, pick the clips of its
    scenes again, then update the manifest."""
    assert cp.manifest_mismatches(ROOT) == []


def test_changed_builder_is_planted_red():
    """A test of the test: one changed builder in a planted manifest and the check names the
    explainer file and the builder path."""
    entry = dict(MANIFEST["explainers"]["explainer-04.mp4"])
    entry["builder"] = {"path": entry["builder"]["path"], "sha256": "0" * 64}
    bad = cp.manifest_mismatches(ROOT, {"explainers": {"explainer-04.mp4": entry}})
    assert len(bad) == 1
    assert "explainer-04.mp4" in bad[0] and entry["builder"]["path"] in bad[0]


def test_current_source_passes(tmp_path):
    payload = b"current build"
    p = write_explainer(tmp_path, "explainer-04.mp4", payload)
    manifest = {"explainers": {"explainer-04.mp4": {"file": hashlib.sha256(payload).hexdigest()}}}
    original = cp.CLIP_SOURCES
    cp.CLIP_SOURCES = _tmp_manifest(tmp_path, manifest["explainers"])
    try:
        cp.check_sources_current(plan_with(p))   # no raise
    finally:
        cp.CLIP_SOURCES = original


def test_planted_old_source_fails(tmp_path):
    old = b"the old 2026-11-02 build, 4811323 bytes"
    p = write_explainer(tmp_path, "explainer-04.mp4", old)
    current = MANIFEST["explainers"]["explainer-04.mp4"]["file"]
    original = cp.CLIP_SOURCES
    cp.CLIP_SOURCES = _tmp_manifest(tmp_path, {"explainer-04.mp4": {"file": current}})
    try:
        with pytest.raises(cp.ComposeError) as e:
            cp.check_sources_current(plan_with(p))
    finally:
        cp.CLIP_SOURCES = original
    msg = str(e.value)
    assert "pv-exact" in msg and "explainer-04.mp4" in msg
    assert "is not the build" in msg and "pick the clips of this scene again" in msg
    assert hashlib.sha256(old).hexdigest()[:16] in msg and current[:16] in msg


def test_explainer_missing_from_manifest_fails(tmp_path):
    p = write_explainer(tmp_path, "explainer-99.mp4", b"unknown build")
    original = cp.CLIP_SOURCES
    cp.CLIP_SOURCES = _tmp_manifest(tmp_path, {})
    try:
        with pytest.raises(cp.ComposeError) as e:
            cp.check_sources_current(plan_with(p))
    finally:
        cp.CLIP_SOURCES = original
    assert "not in" in str(e.value)


def test_recording_source_is_not_checked(tmp_path):
    p = write_explainer(tmp_path, "part1.mp4", b"a recording, not listed")
    original = cp.CLIP_SOURCES
    cp.CLIP_SOURCES = _tmp_manifest(tmp_path, {})
    try:
        cp.check_sources_current(plan_with(p))   # no raise
    finally:
        cp.CLIP_SOURCES = original


def test_failed_run_leaves_the_old_output(tmp_path):
    out = tmp_path / "product-video.mp4"
    tmp = tmp_path / (out.name + ".partial.mp4")
    out.write_bytes(b"the good output of the green run")
    tmp.write_bytes(b"a partial file of a failed run")
    with pytest.raises(cp.ComposeError):
        cp.finalize_output(type("R", (), {"returncode": 1, "stderr": "ffmpeg error"})(), tmp, out)
    assert out.read_bytes() == b"the good output of the green run"
    assert not tmp.exists()


def test_successful_run_moves_the_tmp_into_place(tmp_path):
    out = tmp_path / "product-video.mp4"
    tmp = tmp_path / (out.name + ".partial.mp4")
    out.write_bytes(b"the old output")
    tmp.write_bytes(b"the fresh full render")
    cp.finalize_output(type("R", (), {"returncode": 0, "stderr": ""})(), tmp, out)
    assert out.read_bytes() == b"the fresh full render"
    assert not tmp.exists()
