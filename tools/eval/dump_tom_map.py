"""A5 helper: dump the tom mapping per drummer to JSON for the Rust fast gate probe.

    uv run --with numpy --with soundfile python tools/eval/dump_tom_map.py > ~/.cache/drumspike/a5_tom_map.json
"""
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "eval"))
import enst  # noqa: E402

root = Path.home() / ".cache" / "drumspike" / "datasets" / "ENST-drums-public"
out = {}
for d in enst.DRUMMERS:
    m = enst.tom_mapping(root, d)
    out[d] = {k: v for k, v in m.items()}
print(json.dumps(out))
