"""P4-10 stage 5: the page side of the kick rescue (`static/app.js`). The Inspector's counts line adds "Kick: N quiet hits
added" (`insFlags`), only for rescue hits in the output (not cut, not filtered); a page write of the whole hit list (a
loudness step, `events_set`) keeps the `rescue` tag (`normEvents`)."""
from app_support import edits_done, inspect_track, invoke, open_project, wait_until
from helpers import SR
from test_editor import IDX, build, service_events


def test_the_counts_line_and_a_page_write_keep_the_rescue_hits(app, make_project, tmp_path):
    events = [{"t": i / SR, "v": 100, **({"rescue": True} if k in (1, 2) else {})} for k, i in enumerate(IDX)]
    folder = build(make_project, tmp_path, tracks=(("Kick In", "kick", 36),), events={"Kick In": events}, name="Rescue")
    sid = open_project(app, folder)
    ins = inspect_track(app, "Kick In")
    assert ins.locator(".insFlags").inner_text().strip() == "Kick: 2 quiet hits added"
    assert "hits the onset detector found" in ins.locator(".insFlags").get_attribute("title")
    before = [e["v"] for e in service_events(app, sid, "Kick In")]
    ins.locator(".pVLo").fill("-70")
    ins.locator(".pVLo").blur()
    edits_done(app)

    def written():
        app.evaluate("() => flushEvents()")
        return [e["v"] for e in invoke(app, "events_get", sid=sid, name="Kick In")["events"]] != before
    wait_until(written, msg="the loudness step writes the hit list")
    stored = service_events(app, sid, "Kick In")
    assert sorted(round(e["t"] * SR) for e in stored if e.get("rescue")) == IDX[1:3], stored


def test_the_counts_line_counts_only_rescue_hits_in_the_output(app, make_project, tmp_path):
    extras = [{"rescue": True}, {"rescue": True, "cut": True}, {"rescue": True, "filtered": True}]
    events = [{"t": i / SR, "v": 100, **extra} for i, extra in zip(IDX, extras)]
    folder = build(make_project, tmp_path, tracks=(("Kick In", "kick", 36),), events={"Kick In": events}, name="Cut")
    open_project(app, folder)
    assert inspect_track(app, "Kick In").locator(".insFlags").inner_text().strip() == "Kick: 1 quiet hit added"
