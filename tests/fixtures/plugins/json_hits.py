#!/usr/bin/env python3
"""Example detector plugin for the #363 adapter test (tests only, never bundled).

Protocol 1: reads one JSON request line from stdin, reads the hit times from
``times.json`` next to this file (``{"<role>": [t, ...], ...}`` or a flat list for any
role) and prints one JSON response line on stdout. No model, no network.
"""

import json
import sys
from pathlib import Path

TIMES = Path(__file__).with_name("times.json")


def main() -> None:
    request = json.loads(sys.stdin.readline())
    table = json.loads(TIMES.read_text())
    times = table.get(request["role"], []) if isinstance(table, dict) else list(table)
    hits = [{"t": t, "class": request["role"], "confidence": 1.0} for t in times]
    print(json.dumps({"hits": hits}))


if __name__ == "__main__":
    main()
