"""#558: a host (the DAW plugin) may give a track a `display_name`, the name the user sees in the DAW. The page
shows it in the track header and in the Inspector (text and title). The key of the track stays its name: select,
rename and analyse send the key, and a blur of the name field that changed nothing renames nothing. The standalone
app has no `display_name`; the other tests show that nothing changes there."""
import json

from playwright.sync_api import expect

from app_support import open_project, wait_until


def with_display_names(page):
    """Every answer of tracks_get carries `display_name: "DAW <key>"`, as the plugin adds it."""
    def add(route):
        r = route.fetch()
        body = r.json()
        for key, track in body["tracks"].items():
            track["display_name"] = f"DAW {key}"
        route.fulfill(response=r, json=body)
    page.route("**/invoke/tracks_get", add)
    page.evaluate("async () => { await loadProject(); }")


def test_a_display_name_is_shown_and_every_command_keeps_the_key(app, make_project):
    open_project(app, make_project())
    with_display_names(app)
    header = app.locator('.track[data-name="Kick In"] .tnm')
    expect(header).to_have_text("DAW Kick In")
    expect(header).to_have_attribute("title", "DAW Kick In")
    assert app.evaluate("() => S.order") == ["Kick In", "Snare Top"], "the keys are the names"
    # select: the header click selects the key and the Inspector shows the display name
    app.evaluate("() => { if (!S.inspector) setInspector(true); }")
    app.locator('.track[data-name="Kick In"] .thead .tnote').click()
    app.wait_for_function("() => S.selTrack === 'Kick In'")
    ins = app.locator("#inspector")
    expect(ins.locator("#insName")).to_have_value("DAW Kick In")
    assert app.evaluate("() => document.querySelector('#insName').dataset.name") == "Kick In"
    # a blur that changed nothing renames nothing
    app.locator("#insName").focus()
    app.keyboard.press("Tab")
    app.wait_for_timeout(300)
    assert app.evaluate("() => S.order") == ["Kick In", "Snare Top"]
    # analyse: the request names the key
    with app.expect_request(lambda r: "/invoke/analyze_start" in r.url) as req:
        ins.locator(".bReanalyze").first.click()
    body = json.loads(req.value.post_data or "{}")
    assert "Kick In" in json.dumps(body) and "DAW" not in json.dumps(body), body
    wait_until(lambda: not app.evaluate("() => analysisRunning"), timeout=60, msg="the analysis ends")
    # rename: the typed name is the new key, the header shows the display name of the new key
    app.locator("#insName").fill("Kick Inside")
    app.locator("#insName").blur()
    wait_until(lambda: app.evaluate("() => S.order") == ["Kick Inside", "Snare Top"], msg="the rename")
    expect(app.locator('.track[data-name="Kick Inside"] .tnm')).to_have_text("DAW Kick Inside")
