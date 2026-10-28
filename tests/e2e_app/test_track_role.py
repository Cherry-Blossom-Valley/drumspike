"""0.9.1 part 1: the role of a track in the Inspector Track section. The select sets the role
(track_set_role): a track with activations is re-analyzed (the hits are re-picked) and Edit >
Undo restores the role, the note and the hits; the role source hint shows for a guessed track
(or one without a recorded source, an older project); a role change offers the new role's
default note when the track played on the old role's default, and the offer sets it.
Every wait is on the page's own state; no sleeps."""
import pytest
from playwright.sync_api import expect

from app_support import inspect_track, menu, open_project
from test_subtracks import analyse, has_model

READY = "() => typeof S !== 'undefined' && S.loaded > 0"


def role_sel(app):
    return app.locator("#insRoleSel")


def src_text(app):
    return app.locator("#insRoleSrc").inner_text()


def offer(app):
    return app.locator("#insNoteOfferRow")


def page_events(app, name):
    return app.evaluate("n => S.events[n].map(e => ({t: e.t, v: e.v}))", name)


def show(app, make_project, tracks=(("Clap_01", "other", 36), ("Kick In", "kick", 36))):
    sid = open_project(app, make_project(tracks=tracks))
    inspect_track(app, "Clap_01")
    app.evaluate("() => { INSSEC.open.track = true; insSecApply(); }")
    app.wait_for_function(
        "() => document.querySelector('#insRoleSel') && document.querySelector('#insRoleSel').value === 'other'")
    return sid


def test_an_unmatched_name_warns(app, make_project):
    show(app, make_project)
    assert role_sel(app).input_value() == "other"
    assert src_text(app) == "not recognised from the name: choose the role"
    expect(app.locator("#insRoleSrc")).to_have_class("phint insRoleHint lyWarn")


def test_a_matched_name_tells_the_words(app, make_project):
    sid = open_project(app, make_project(tracks=(
        ("Clap_01", "other", 36),
        ("Kick In", "kick", 36, None, {"role_source": "scheme", "role_match": "kick"}),
    )))
    inspect_track(app, "Kick In")
    app.evaluate("() => { INSSEC.open.track = true; insSecApply(); }")
    app.wait_for_function(
        "() => document.querySelector('#insRoleSel') && document.querySelector('#insRoleSel').value === 'kick'")
    assert src_text(app) == "from the name (kick = kick)"
    expect(app.locator("#insRoleSrc")).to_have_class("phint insRoleHint")


def test_undo_restores_the_matched_name_hint(app, make_project):
    sid = open_project(app, make_project(tracks=(
        ("Kick In", "kick", 36, None, {"role_source": "scheme", "role_match": "kick"}),
    )))
    inspect_track(app, "Kick In")
    app.evaluate("() => { INSSEC.open.track = true; insSecApply(); }")
    app.wait_for_function(
        "() => document.querySelector('#insRoleSel') && document.querySelector('#insRoleSel').value === 'kick'")
    assert src_text(app) == "from the name (kick = kick)"
    role_sel(app).select_option("snare")
    app.wait_for_function("() => S.tracks['Kick In'].role === 'snare'")
    assert src_text(app) == "set by you"
    menu(app, "undo")
    app.wait_for_function("() => S.tracks['Kick In'].role === 'kick'")
    assert src_text(app) == "from the name (kick = kick)"


def test_the_note_offer_works(app, make_project):
    show(app, make_project)
    role_sel(app).select_option("snare")
    expect(offer(app)).to_be_visible()
    expect(app.locator("#insNoteOffer")).to_have_text("snare: default note D1 (38)")
    assert app.evaluate("() => S.tracks['Clap_01'].note") == 36
    app.locator("#insNoteOfferBtn").click()
    app.wait_for_function("() => S.tracks['Clap_01'].note === 38")
    expect(offer(app)).to_be_hidden()
    assert app.evaluate("() => S.tracks['Clap_01'].role") == "snare"
    assert src_text(app) == "set by you"


def test_a_role_change_reanalyzes_and_undo_restores_role_note_and_hits(app, make_project, tmp_path):
    if not has_model(app):
        pytest.skip("the re-pick of a role change needs the model (the DSP fallback keeps no activations)")
    show(app, make_project)
    analyse(app)
    before = page_events(app, "Clap_01")
    assert before, "the model found hits"

    role_sel(app).select_option("snare")
    app.wait_for_function(
        "b => S.events['Clap_01'] && S.events['Clap_01'].length && JSON.stringify(S.events['Clap_01'].map(e => ({t: e.t, v: e.v}))) !== JSON.stringify(b)", arg=before)
    assert app.evaluate("() => S.tracks['Clap_01'].role") == "snare"
    assert src_text(app) == "set by you"

    menu(app, "undo")
    app.wait_for_function("() => S.tracks['Clap_01'].role === 'other'")
    app.wait_for_function(
        "b => S.events['Clap_01'] && JSON.stringify(S.events['Clap_01'].map(e => ({t: e.t, v: e.v}))) === JSON.stringify(b)", arg=before)
    assert app.evaluate("() => S.tracks['Clap_01'].note") == 36
