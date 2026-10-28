"""P4-13 stage B: the page side of the flam rule (`static/app.js`). The Inspector's counts line adds "Flams: N hits
added" (`insFlags`); a page write of the whole hit list (a loudness step, `events_set`) keeps the `flam` tag
(`normEvents`); the Detection section of a snare track has the checkbox "Split flams" (on unless the track param
`flam_split` is false; it applies on the next re-analyze), a kick track has none, and a follower's is disabled."""
from app_support import edits_done, inspect_track, invoke, open_project, wait_until
from helpers import SR
from test_editor import IDX, build, service_events


def project(make_project, tmp_path, name, flam):
    events = [{"t": i / SR, "v": 100, **({"flam": True} if k in flam else {})} for k, i in enumerate(IDX)]
    return build(make_project, tmp_path, tracks=(("Snare Top", "snare", 38), ("Kick In", "kick", 36)),
                 events={"Snare Top": events}, name=name)


def test_the_counts_line_and_a_page_write_keep_the_flam_hits(app, make_project, tmp_path):
    sid = open_project(app, project(make_project, tmp_path, "Flams", flam={1, 2}))
    ins = inspect_track(app, "Snare Top")
    assert ins.locator(".insFlags").inner_text().strip() == "Flams: 2 hits added"
    assert "second strokes of flams" in ins.locator(".insFlags").get_attribute("title")
    before = [e["v"] for e in service_events(app, sid, "Snare Top")]
    ins.locator(".pVLo").fill("-70")
    ins.locator(".pVLo").blur()
    edits_done(app)

    def written():
        app.evaluate("() => flushEvents()")
        return [e["v"] for e in invoke(app, "events_get", sid=sid, name="Snare Top")["events"]] != before
    wait_until(written, msg="the loudness step writes the hit list")
    stored = service_events(app, sid, "Snare Top")
    assert sorted(round(e["t"] * SR) for e in stored if e.get("flam")) == IDX[1:3], stored


def test_the_split_flams_switch_is_a_snare_track_param(app, make_project, tmp_path):
    open_project(app, project(make_project, tmp_path, "Switch", flam=set()))
    ins = inspect_track(app, "Snare Top")
    box = ins.locator(".pFlam")
    assert box.is_checked() and box.get_attribute("title") == "Applies on the next re-analyze"
    box.uncheck()
    assert app.evaluate("() => S.tracks['Snare Top'].params.flam_split") is False
    box.check()
    assert app.evaluate("() => S.tracks['Snare Top'].params.flam_split") is True
    ins = inspect_track(app, "Kick In")
    assert ins.locator(".pFlam").count() == 0                      # no switch on a kick track
    # a follower's detection settings are not used: the switch is disabled (the page's state alone, rebuilt)
    inspect_track(app, "Snare Top")
    app.evaluate("() => { S.tracks['Snare Top'].follows = 'Kick In'; syncInspector(true); }")
    assert ins.locator(".pFlam").is_disabled()


def test_the_counts_line_counts_only_flam_hits_in_the_output(app, make_project, tmp_path):
    extras = [{"flam": True}, {"flam": True, "cut": True}, {"flam": True, "filtered": True}]
    events = [{"t": i / SR, "v": 100, **extra} for i, extra in zip(IDX, extras)]
    folder = build(make_project, tmp_path, tracks=(("Snare Top", "snare", 38),), events={"Snare Top": events}, name="Cut")
    open_project(app, folder)
    assert inspect_track(app, "Snare Top").locator(".insFlags").inner_text().strip() == "Flams: 1 hit added"
