"""#494: the check against old clip sources (tools/media/compose.py check_sources_current).

The gate: with the manifest current (tools/media/clip-sources.json), the check passes; on a
planted old source — an explainer file whose sha256 is not the manifest's — it FAILs, naming
the scene, the file and both digests; and an explainer missing from the manifest fails too.
Recordings are not listed and pass unchecked.
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


def test_manifest_lists_the_current_builds():
    for name, digest in MANIFEST["explainers"].items():
        assert name.startswith("explainer-") and len(digest) == 64


def test_current_source_passes(tmp_path):
    payload = b"current build"
    p = write_explainer(tmp_path, "explainer-04.mp4", payload)
    manifest = {"explainers": {"explainer-04.mp4": hashlib.sha256(payload).hexdigest()}}
    original = cp.CLIP_SOURCES
    cp.CLIP_SOURCES = _tmp_manifest(tmp_path, manifest)
    try:
        cp.check_sources_current(plan_with(p))   # no raise
    finally:
        cp.CLIP_SOURCES = original


def test_planted_old_source_fails(tmp_path):
    old = b"the old 2026-11-02 build, 4811323 bytes"
    p = write_explainer(tmp_path, "explainer-04.mp4", old)
    current = MANIFEST["explainers"]["explainer-04.mp4"]
    manifest = {"explainers": {"explainer-04.mp4": current}}
    original = cp.CLIP_SOURCES
    cp.CLIP_SOURCES = _tmp_manifest(tmp_path, manifest)
    try:
        with pytest.raises(cp.ComposeError) as e:
            cp.check_sources_current(plan_with(p))
    finally:
        cp.CLIP_SOURCES = original
    msg = str(e.value)
    assert "pv-exact" in msg and "explainer-04.mp4" in msg and "old build" in msg
    assert hashlib.sha256(old).hexdigest()[:16] in msg and current[:16] in msg


def test_explainer_missing_from_manifest_fails(tmp_path):
    p = write_explainer(tmp_path, "explainer-99.mp4", b"unknown build")
    original = cp.CLIP_SOURCES
    cp.CLIP_SOURCES = _tmp_manifest(tmp_path, {"explainers": {}})
    try:
        with pytest.raises(cp.ComposeError) as e:
            cp.check_sources_current(plan_with(p))
    finally:
        cp.CLIP_SOURCES = original
    assert "not in" in str(e.value)


def test_recording_source_is_not_checked(tmp_path):
    p = write_explainer(tmp_path, "part1.mp4", b"a recording, not listed")
    original = cp.CLIP_SOURCES
    cp.CLIP_SOURCES = _tmp_manifest(tmp_path, {"explainers": {}})
    try:
        cp.check_sources_current(plan_with(p))   # no raise
    finally:
        cp.CLIP_SOURCES = original


def _tmp_manifest(tmp_path, manifest):
    p = tmp_path / "clip-sources.json"
    p.write_text(json.dumps(manifest), encoding="utf-8")
    return p
