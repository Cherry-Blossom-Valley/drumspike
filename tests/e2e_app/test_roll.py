"""P4-19: the page keeps the `roll` tag of the roll fill's hits (`static/app.js` `normEvents`). A loudness step (the
velocity floor of the Inspector) writes the track's whole hit list back to the service (`events_set`); the hits the
fill added must still carry `roll: true` there."""
from app_support import edits_done, inspect_track, invoke, open_project, wait_until
from test_editor import IDX, build, service_events
from helpers import SR


def test_a_page_write_of_the_hit_list_keeps_the_roll_tag(app, make_project, tmp_path):
    events = [{"t": i / SR, "v": 100, **({"roll": True} if k else {})} for k, i in enumerate(IDX)]
    sid = open_project(app, build(make_project, tmp_path, tracks=(("Snare Top", "snare", 38),),
                                  events={"Snare Top": events}, name="Roll"))
    assert len([e for e in service_events(app, sid, "Snare Top") if e.get("roll")]) == 2   # the document's tags
    before = [e["v"] for e in service_events(app, sid, "Snare Top")]
    ins = inspect_track(app, "Snare Top")
    ins.locator(".pVLo").fill("-70")
    ins.locator(".pVLo").blur()
    edits_done(app)

    def written():
        app.evaluate("() => flushEvents()")
        return [e["v"] for e in invoke(app, "events_get", sid=sid, name="Snare Top")["events"]] != before
    wait_until(written, msg="the loudness step writes the hit list")
    stored = service_events(app, sid, "Snare Top")
    assert sorted(round(e["t"] * SR) for e in stored if e.get("roll")) == IDX[1:], stored
