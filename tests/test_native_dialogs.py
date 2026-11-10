"""#429: the page asks every question with the in-page dialog (askConfirm). A native confirm(), alert() or
prompt() blocks the page and shows nothing at all in some hosts. This check reads every static/*.js and fails
on such a call. Comments and string literals are cut out first; a method or a local function of the same name
is no call."""
import re
from pathlib import Path

STATIC = Path(__file__).resolve().parents[1] / "static"

_CUT = re.compile(r'"(?:\\.|[^"\\\n])*"|\'(?:\\.|[^\'\\\n])*\'|`(?:\\.|[^`\\])*`|/\*.*?\*/|//[^\n]*', re.S)
_CALL = re.compile(r"(?<![\w$.])(?:(?:window|globalThis)\.)?(confirm|alert|prompt)\s*\(")
_DEFINITION = re.compile(r"^\s*(?:async\s+)?(?:function\s+)?(?:confirm|alert|prompt)\s*\([^)]*\)\s*\{")


def native_dialog_calls(folder):
    """(file name, line number, line) of each native dialog call under folder/*.js, and the count of files read."""
    found, files = [], sorted(Path(folder).glob("*.js"))
    for f in files:
        text = _CUT.sub(lambda m: "\n" * m.group(0).count("\n"), f.read_text())
        for n, line in enumerate(text.splitlines(), 1):
            if _CALL.search(line) and not _DEFINITION.match(line):
                found.append((f.name, n, line.strip()))
    return found, len(files)


def test_the_page_calls_no_native_dialog():
    found, read = native_dialog_calls(STATIC)
    print(f"native-dialog check: {read} files read, {len(found)} native dialog calls")
    assert read > 0, "the check read no file"
    assert not found, f"native dialog in the page (use askConfirm): {found}"


def test_the_check_finds_a_native_call_and_skips_text_and_names(tmp_path):
    (tmp_path / "a.js").write_text(
        'const ok = window.confirm("sure?");\n'
        "alert(`x`);\n"
        'const s = "confirm(1)"; // prompt(2)\n'
        "/* alert(3) */\n"
        "const o = {\n"
        "  async confirm(a) { return a; },\n"
        "};\n"
        "await o.confirm(1);\n")
    found, read = native_dialog_calls(tmp_path)
    assert read == 1
    assert [(n, l.split("(")[0]) for _, n, l in found] == [(1, "const ok = window.confirm"), (2, "alert")]
    found, read = native_dialog_calls(tmp_path / "none")
    assert read == 0 and found == []
