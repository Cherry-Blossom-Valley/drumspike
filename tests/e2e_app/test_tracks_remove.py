"""P4-39a: Track > Remove Track through the bridge. The menu item and the Delete or Backspace key (when no hit and no tempo marker
is selected) ask "Remove track X and its N hits?" with Remove and Cancel (the page's confirm dialog, never confirm()); Remove
takes the track out of the project (the service: `tracks_remove`), selects the next track (else the previous one) and says
"X removed (undo: Cmd+Z)"; one undo step brings the track back with its hits, params and mix (`tracks_restore`), redo removes it
again without a question. A follower of a removed track becomes a normal track. The agent command and the page give the same
project. Every wait is on the page's own state (the status line, S); no sleeps."""
import json

from playwright.sync_api import expect

from app_support import invoke, menu, open_project, wait_until
from test_menus import select_track, with_hits

DIALOG = "#confirmModal"
HITS = {"A": [0.5, 1.0, 1.5], "B": [0.6, 1.1, 1.6, 2.1], "C": [0.7, 1.2]}
TRACKS = (("A", "kick", 36), ("B", "snare", 38), ("C", "tom", 48))


def setup(app, make_project, tracks=TRACKS):
    folder = make_project(tracks=tracks)
    sid = open_project(app, folder)
    with_hits(app, sid, {n: ts for n, ts in HITS.items() if n in [t[0] for t in tracks]})
    return folder, sid


def order(app):
    return app.evaluate("() => S.order.slice()")


def ask_remove(app, name, via="menu"):
    """Select the track, start the removal by the menu item or the key; the dialog is open when this returns."""
    select_track(app, name)
    if via == "menu":
        menu(app, "tracks_remove", wait=False)
    else:
        app.keyboard.press(via)
    expect(app.locator(DIALOG)).to_be_visible()


def remove(app, name, via="menu"):
    ask_remove(app, name, via)
    app.click(f"{DIALOG} button[data-act='remove']")
    expect(app.locator(DIALOG)).to_be_hidden()
    expect(app.locator("#stMsg")).to_have_text(f"{name} removed (undo: Cmd+Z)")


def doc_tracks(app, sid):
    return invoke(app, "tracks_get", sid=sid)


def test_the_menu_item_asks_and_remove_takes_the_track_out_everywhere(app, make_project):
    folder, sid = setup(app, make_project)
    assert (folder / "Analysis" / "B").is_dir()                            # the fixture really has the folder the removal deletes
    ask_remove(app, "B")
    assert app.locator("#confirmTitle").inner_text() == "Remove track B and its 4 hits?"
    assert app.locator(f"{DIALOG} button").all_inner_texts() == ["Cancel", "Remove"]
    assert order(app) == ["A", "B", "C"]                                    # nothing happens before the answer
    app.click(f"{DIALOG} button[data-act='remove']")
    expect(app.locator("#stMsg")).to_have_text("B removed (undo: Cmd+Z)")
    assert order(app) == ["A", "C"] and app.evaluate("() => Object.keys(S.tracks)") == ["A", "C"]
    assert app.evaluate("() => Object.keys(S.events).sort()") == ["A", "C"]
    assert app.locator(".track").count() == 2 and app.locator('.track[data-name="B"]').count() == 0   # lanes and mixer rows
    assert app.evaluate("() => S.selTrack") == "C"                          # the next track in order
    assert app.locator("#insName").input_value() == "C"                     # the Inspector shows it
    assert app.evaluate("() => DSShell.menuState().tracks") == ["A", "C"]  # the follow menus
    got = doc_tracks(app, sid)
    assert got["order"] == ["A", "C"] and list(got["tracks"]) == ["A", "C"] and got["dirty"] is True
    assert not (folder / "Analysis" / "B").exists() and (folder / "Analysis" / "A").is_dir()
    assert invoke(app, "events_get", sid=sid, name="B")["code"] == 400
    assert app.evaluate("() => S.undo.length") == 1 and app.evaluate("() => S.undo[0].type") == "trkdel"


def test_the_last_track_leaves_the_previous_one_selected(app, make_project):
    _, sid = setup(app, make_project)
    remove(app, "C")
    assert order(app) == ["A", "B"] and app.evaluate("() => S.selTrack") == "B"
    remove(app, "A")
    assert order(app) == ["B"] and app.evaluate("() => S.selTrack") == "B"
    remove(app, "B")                                                        # the last one: an empty project
    assert order(app) == [] and app.evaluate("() => S.selTrack") is None
    assert doc_tracks(app, sid)["tracks"] == {}


def test_cancel_keeps_the_track(app, make_project):
    folder, sid = setup(app, make_project)
    ask_remove(app, "B")
    app.click(f"{DIALOG} button[data-act='cancel']")
    expect(app.locator(DIALOG)).to_be_hidden()
    expect(app.locator("#stMsg")).to_have_text("remove track cancelled")
    assert order(app) == ["A", "B", "C"] and app.evaluate("() => S.selTrack") == "B"
    assert list(doc_tracks(app, sid)["tracks"]) == ["A", "B", "C"] and (folder / "Analysis" / "B").is_dir()
    assert app.evaluate("() => S.undo.length") == 0
    assert len(invoke(app, "events_get", sid=sid, name="B")["events"]) == 4


# P4-39c (merged with P4-39a): a project with tracks always has a selected track, so Track > Remove Track never meets
# "no track selected" while tracks exist; the former test of that message is gone. The no-tracks state is covered by
# test_track_selected.py (the Inspector placeholder).
def test_delete_and_backspace_remove_the_selected_track_only_without_a_hit_or_a_marker(app, make_project):
    _, sid = setup(app, make_project)
    # a selected hit: Delete deletes the hit as before, no question
    select_track(app, "B")
    app.evaluate("() => { S.sel = { track: 'B', index: 1 }; }")
    app.keyboard.press("Delete")
    app.wait_for_function("() => S.events['B'].length === 3")
    expect(app.locator(DIALOG)).to_be_hidden()
    assert order(app) == ["A", "B", "C"]
    # a selected tempo marker: Delete takes the marker (here the start tempo stays), no question
    app.evaluate("() => { S.sel = null; TM.sel = { kind: 'tempo', i: 0 }; }")
    app.keyboard.press("Backspace")
    expect(app.locator("#stMsg")).to_have_text("the start tempo stays: edit its value")
    expect(app.locator(DIALOG)).to_be_hidden()
    assert order(app) == ["A", "B", "C"]
    app.evaluate("() => { TM.sel = null; }")
    # no hit, no marker: Delete asks (3 hits are left), Cancel keeps; Backspace asks again, Remove removes
    ask_remove(app, "B", via="Delete")
    assert app.locator("#confirmTitle").inner_text() == "Remove track B and its 3 hits?"
    app.click(f"{DIALOG} button[data-act='cancel']")
    assert order(app) == ["A", "B", "C"]
    remove(app, "B", via="Backspace")
    assert order(app) == ["A", "C"] and list(doc_tracks(app, sid)["tracks"]) == ["A", "C"]
    # a text field keeps its keys: Backspace in the Inspector name field removes no track
    app.locator("#insName").click()
    app.keyboard.press("Backspace")
    expect(app.locator(DIALOG)).to_be_hidden()
    assert order(app) == ["A", "C"]


def test_undo_brings_the_track_back_with_hits_params_and_mix_and_redo_removes_it_again(app, make_project):
    folder, sid = setup(app, make_project)
    invoke(app, "track_params_set", sid=sid, name="B", params={"min_gap_ms": 41.0})
    invoke(app, "track_set_mix", sid=sid, name="B", volume_db=-6.5, pan=0.25)
    app.evaluate("async () => { await loadProject(); }")
    app.wait_for_function("() => S.loaded > 0 && S.order.length === 3")
    before = invoke(app, "tracks_get", sid=sid)["tracks"]["B"]
    hits = invoke(app, "events_get", sid=sid, name="B")["events"]
    app.evaluate("() => { S.muted['B'] = true; }")
    remove(app, "B")
    assert not app.evaluate("() => 'B' in S.muted")
    menu(app, "undo")
    app.wait_for_function("() => S.order.length === 3 && S.order[1] === 'B' && !!S.events['B'] && !!S.peaks['B']")
    expect(app.locator("#stMsg")).to_have_text("undid Remove Track B (B)")
    assert order(app) == ["A", "B", "C"] and app.evaluate("() => S.selTrack") == "B"
    got = invoke(app, "tracks_get", sid=sid)
    assert got["order"] == ["A", "B", "C"] and got["tracks"]["B"] == before     # every field: params, mix, source
    assert invoke(app, "events_get", sid=sid, name="B")["events"] == hits       # the hits with their ids and values
    assert app.evaluate("() => S.events['B'].map(e => e.t)") == [e["t"] for e in hits]
    assert (folder / "Analysis" / "B" / "peaks.bin").is_file()                  # the lane cache is built again
    assert app.evaluate("() => S.muted['B']") is True                           # the page's mute comes back
    assert app.evaluate("() => [S.undo.length, S.redo.length]") == [0, 1]
    menu(app, "redo")                                                           # no question on a redo
    expect(app.locator("#stMsg")).to_have_text("redid Remove Track B (B)")
    assert order(app) == ["A", "C"] and list(doc_tracks(app, sid)["tracks"]) == ["A", "C"]
    assert app.evaluate("() => [S.undo.length, S.redo.length]") == [1, 0]
    menu(app, "undo")
    app.wait_for_function("() => S.order.length === 3")
    assert invoke(app, "tracks_get", sid=sid)["tracks"]["B"] == before


def test_a_follower_of_the_removed_track_becomes_a_normal_track_and_follows_again_after_undo(app, make_project):
    _, sid = setup(app, make_project)
    assert invoke(app, "track_set_follow", sid=sid, name="B", source="A")["follows"] == "A"
    app.evaluate("async () => { await loadProject(); }")
    app.wait_for_function("() => S.loaded > 0 && S.tracks['B'].follows === 'A'")
    own = invoke(app, "events_get", sid=sid, name="B")["events"]
    ask_remove(app, "A")
    assert "B will detect on its own" in app.locator("#confirmMsg").inner_text()
    app.click(f"{DIALOG} button[data-act='remove']")
    expect(app.locator("#stMsg")).to_have_text("A removed (undo: Cmd+Z)")
    assert order(app) == ["B", "C"] and not app.evaluate("() => S.tracks['B'].follows")
    assert not invoke(app, "tracks_get", sid=sid)["tracks"]["B"].get("follows")
    assert invoke(app, "events_get", sid=sid, name="B")["events"] == own        # its hits stay
    select_track(app, "B")
    assert app.evaluate("() => DSShell.menuState().follows") is None                # Track > Follow Hits Of shows no source
    assert app.locator('.track[data-name="B"] .tfollow').inner_text() == ""
    menu(app, "undo")
    app.wait_for_function("() => S.order.length === 3 && S.tracks['B'].follows === 'A'")
    assert invoke(app, "tracks_get", sid=sid)["tracks"]["B"]["follows"] == "A"
    assert app.locator('.track[data-name="B"] .tfollow').inner_text() == " · follows A"
    assert invoke(app, "events_get", sid=sid, name="B")["events"] == own


def test_the_agent_command_and_the_page_remove_the_same_things(app, make_project):
    """Parity: the page's removal and `tracks_remove` of the agent API leave the same project (the page has no removal code of
    its own); `tracks_restore` with the answer's snapshot gives the page's undo."""
    f1, s1 = setup(app, make_project)
    f2 = make_project(tracks=TRACKS)
    s2 = invoke(app, "project_open", path=str(f2))["sid"]
    for n, ts in HITS.items():
        invoke(app, "events_set", sid=s2, name=n, events=[{"t": t, "v": 90, "id": f"{n[0]}{i}"} for i, t in enumerate(ts)])
    invoke(app, "peaks_build_start", sid=s2)
    wait_until(lambda: invoke(app, "peaks_build_status", sid=s2)["state"] == "done", msg="the lane caches of the agent's project")
    remove(app, "B")
    r = invoke(app, "tracks_remove", sid=s2, names=["B"], snapshot=True)
    assert r["removed"] == ["B"] and r["order"] == ["A", "C"] and "snapshot" in r
    page_side, agent_side = doc_tracks(app, s1), doc_tracks(app, s2)
    assert page_side["order"] == agent_side["order"] == ["A", "C"]
    assert json.dumps(page_side["tracks"], sort_keys=True) == json.dumps(agent_side["tracks"], sort_keys=True) == json.dumps(r["tracks"], sort_keys=True)
    assert sorted(p.name for p in (f1 / "Analysis").iterdir()) == sorted(p.name for p in (f2 / "Analysis").iterdir()) == ["A", "C"]
    menu(app, "undo")
    app.wait_for_function("() => S.order.length === 3")
    back = invoke(app, "tracks_restore", sid=s2, tracks=r["snapshot"])
    assert back["restored"] == ["B"] and back["order"] == ["A", "B", "C"]
    wait_until(lambda: (f2 / "Analysis" / "B" / "peaks.bin").is_file(), msg="the lane cache of the restored track")
    assert json.dumps(doc_tracks(app, s1)["tracks"], sort_keys=True) == json.dumps(doc_tracks(app, s2)["tracks"], sort_keys=True)
    assert invoke(app, "events_get", sid=s1, name="B")["events"] == invoke(app, "events_get", sid=s2, name="B")["events"]
    assert invoke(app, "tracks_remove", sid=s2, names=["Nope"])["code"] == 400
