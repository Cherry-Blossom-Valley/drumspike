"""#540 gate: no page test reads a drawn text right after a wait on state only.

The frame draw (drawHead in static/app.js) writes #timeDisplay and #sampleDisplay one frame
after the state changes. An assert that reads such an element directly after a wait_for_function
on state is a race under load. This script lists every place in tests/e2e_app/ where an assert
reads inner_text() or text_content() of such an element, directly after a wait_for_function on
state, and fails when one is found (or when it checked none at all).

A place is safe when the nearest wait before the assert already waits for the text of that
element (wait_for_function that mentions the element and its text), or through the shared helper
wait_for_time_display in test_host_follow.py, or when the element is not written by the frame
draw at all (for example #playBtn, written with the state in transport.js).

Run: uv run python tests/e2e_app/check_frame_text_reads.py
"""

import re
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
STATIC = HERE.parent.parent / "static"

# elements the frame draw writes (drawHead, static/app.js: a text is written only when it changed)
FRAME_WRITTEN = {"#timeDisplay", "#sampleDisplay"}
# shared text waits (helpers that wait for the text of an element)
TEXT_HELPERS = {"wait_for_time_display"}

WAIT_RE = re.compile(r"wait_for_function\(|\b(" + "|".join(TEXT_HELPERS) + r")\(")
ID_RE = re.compile(r"#[\w-]+")
READ_RE = re.compile(r"\.(inner_text|text_content)\(")
ASSERT_RE = re.compile(r"^\s*assert\b|^\s*.*expect\(")


def wait_is_text_wait(line: str, ids: set) -> bool:
    if any(h in line for h in TEXT_HELPERS):
        return True
    if "wait_for_function" not in line:
        return False
    return bool(ids & set(ID_RE.findall(line))) and ("textContent" in line or "inner_text" in line)


def main() -> int:
    checked, flagged = 0, []
    for path in sorted(HERE.rglob("test_*.py")):
        lines = path.read_text().splitlines()
        for i, line in enumerate(lines):
            if not (READ_RE.search(line) and ASSERT_RE.match(line)):
                continue
            ids = set(ID_RE.findall(line)) & FRAME_WRITTEN
            if not ids:
                continue   # the frame draw does not write this element
            checked += 1
            # the nearest wait in the lines before the assert
            wait_line = next((lines[j] for j in range(i - 1, max(i - 9, -1), -1) if WAIT_RE.search(lines[j])), None)
            where = f"{path.relative_to(HERE.parent.parent)}:{i + 1}"
            if wait_line is None:
                flagged.append((where, "no wait directly before the read", line.strip()))
            elif not wait_is_text_wait(wait_line, ids):
                flagged.append((where, "the wait before it is on state, not on the text", line.strip()))
    print(f"places checked: {checked}")
    for where, why, line in flagged:
        print(f"FLAGGED {where}: {why}\n    {line}")
    if checked == 0:
        print("FAIL: the script checked no places — it must read the tests")
        return 1
    if flagged:
        print(f"FAIL: {len(flagged)} place(s) read a drawn text right after a wait on state")
        return 1
    print("OK: every read of a frame-written text is awaited on the text (or the element is not frame-written)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
