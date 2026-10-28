"""#439: the key command table (static/keymap.js) is sound without the app.

The same checks the e2e run makes (test_keys.py) and menu.rs runs on the compiled
table, here as a plain pytest: the file parses, no key is in two commands per
preset (the Cubase fallback keeps the default keys), and no key is a macOS
standard shortcut (owner rule of 2026-10-21; Cmd+Q and Cmd+, stay, the standard
keys of exactly those commands). The #439 View toggles must have a key.
"""
import json
import re
import pytest

ROOT = __import__("pathlib").Path(__file__).resolve().parent.parent
KEYMAP = ROOT / "static" / "keymap.js"

RESERVED = ["Cmd+Space", "Control+Space", "Control+Up", "Control+Down", "Control+Left", "Control+Right",
            "Cmd+Tab", "Cmd+`", "Shift+Cmd+3", "Shift+Cmd+4", "Shift+Cmd+5", "Cmd+H", "Cmd+M", "Cmd+Option+Escape"]
ALLOWED = {"Cmd+Q": "quit", "Cmd+,": "settings"}   # the macOS standard keys of exactly those commands

def rows():
    """One dict per command object of the table (the fields menu.rs parses)."""
    text = KEYMAP.read_text()
    out = []
    for chunk in text.split('{"id"')[1:]:
        end = chunk.index("}")
        obj = json.loads('{"id"' + chunk[:end + 1].rstrip().rstrip(","))
        out.append(obj)
    return out

def keys_of(row, preset):
    """DSKeys.keysOf: in cubase the row's list, else the menu key plus the page keys."""
    if preset == "cubase" and isinstance(row.get("cubase"), list):
        return list(row["cubase"])
    return ([row["menu_key"]] if row.get("menu_key") else []) + list(row.get("page_keys") or [])

def test_the_table_parses_and_ids_are_unique():
    rows_ = rows()
    assert len(rows_) > 50
    ids = [r["id"] for r in rows_]
    assert len(ids) == len(set(ids))

def test_no_key_is_in_two_commands_per_preset():
    for preset in ("default", "cubase"):
        seen = {}
        for r in rows():
            for k in keys_of(r, preset):
                assert k not in seen, f"{preset}: key {k} is in two commands: {seen.get(k)} and {r['id']}"
                seen[k] = r["id"]

def test_no_key_is_a_macos_standard_shortcut():
    bad = [[p, r["id"], k] for r in rows() for p in ("default", "cubase")
           for k in keys_of(r, p) if k in RESERVED and ALLOWED.get(k) != r["id"]]
    assert bad == [], bad

def test_a_menu_key_always_has_a_modifier_and_a_menu_item():
    for r in rows():
        if r.get("menu_key"):
            assert re.search(r"\b(Cmd|Control|Option)\+", r["menu_key"]), r["id"]
            assert r.get("menu"), f"{r['id']}: menu key without a menu item"

@pytest.mark.parametrize("cmd,key", [("ruler_beats", "Shift+B"), ("ruler_markers", "Shift+R"),
                                     ("height_small", "Option+Cmd+1"), ("height_medium", "Option+Cmd+2"),
                                     ("height_large", "Option+Cmd+3")])
def test_the_view_toggles_of_439_have_a_key(cmd, key):
    row = next(r for r in rows() if r["id"] == cmd)
    assert key in keys_of(row, "default") and key in keys_of(row, "cubase")
