"""P4-7d: the Layer section's dialogs on the bridge. Load one-shot opens the native open dialog in the folder the last
load of its kind used (kept per app); Render Layer opens a save dialog with the name "<track> layer.wav" in the export
folder; a cancel writes nothing; a save over a one-shot in use is refused and the file stays as it was. The dialog
stubs are window.__DS_PICK__ (open), window.__DS_SAVE__ (save) and window.__dialogOpts / __saveOpts (what was asked)."""
import os
from pathlib import Path

from playwright.sync_api import expect

from app_support import file_sha, save_to
from test_layer import choose, layered, shot_file, show  # noqa: F401  (layered is a fixture)


def realdir(path):
    return os.path.realpath(os.path.dirname(str(path)))


def test_render_asks_for_the_track_named_file_in_the_export_folder(app, layered):  # noqa: F811
    pdir, shots = layered
    show(app, pdir)
    choose(app, shot_file(shots, "kick.wav"))
    want = Path(pdir) / "Exports" / "Kick In layer.wav"
    save_to(app, want)
    app.evaluate("() => { window.__saveOpts = null; }")
    app.locator("#inspector .lyRender").click()
    expect(app.locator("#stMsg")).to_contain_text("layer rendered to")
    opts = app.evaluate("() => window.__saveOpts")
    assert os.path.basename(opts["defaultPath"]) == "Kick In layer.wav"
    assert realdir(opts["defaultPath"]) == os.path.realpath(Path(pdir) / "Exports")
    assert want.is_file()


def test_cancel_writes_nothing(app, layered):  # noqa: F811
    pdir, shots = layered
    show(app, pdir)
    choose(app, shot_file(shots, "kick.wav"))
    app.evaluate("() => { window.__DS_SAVE__ = null; }")   # the dialog is cancelled
    app.locator("#inspector .lyRender").click()
    expect(app.locator("#stMsg")).to_contain_text("render cancelled")
    exports = Path(pdir) / "Exports"
    assert not exports.is_dir() or not list(exports.glob("*layer*"))


def test_a_save_over_a_one_shot_in_use_is_refused_and_the_file_is_unchanged(app, layered):  # noqa: F811
    pdir, shots = layered
    show(app, pdir)
    shot = shot_file(shots, "kick.wav")
    choose(app, shot)
    before = file_sha(shot)
    save_to(app, shot)   # planted fault: the render would replace the layer's own one-shot
    app.locator("#inspector .lyRender").click()
    expect(app.locator("#stMsg")).to_contain_text("one-shot in use")
    assert file_sha(shot) == before


def test_the_load_dialog_starts_in_the_folder_the_last_load_used(app, layered):  # noqa: F811
    pdir, shots = layered
    show(app, pdir)
    first = shot_file(shots, "kick.wav")
    choose(app, first)   # the first load, its folder is kept for the next one
    app.evaluate("() => { window.__dialogOpts = null; window.__DS_PICK__ = null; }")
    app.locator("#inspector .lyChoose").click()
    app.wait_for_function("() => window.__dialogOpts && window.__dialogOpts.defaultPath")
    got = app.evaluate("() => window.__dialogOpts.defaultPath")
    assert os.path.realpath(got) == os.path.realpath(os.path.dirname(first))
