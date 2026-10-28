"""P4-24: the Inspector of the selected track says how many hits the automatic rules changed: bleed hits out of the
output (`insFlags` in `static/app.js`). Only the non-zero parts; no line when all are zero."""
from app_support import inspect_track, invoke, open_project, wait_until
from helpers import SR
from test_editor import IDX, build, service_events

LINE = ".insFlagRow"


def flagged(make_project, tmp_path, name, bleed, roll, edited=()):
    """A snare track: hit k carries `bleed`, `roll` or `edited` when k is in the given sets."""
    events = [{"t": i / SR, "v": 100,
               **({"bleed": True} if k in bleed else {}), **({"roll": True} if k in roll else {}),
               **({"edited": True} if k in edited else {})} for k, i in enumerate(IDX)]
    return build(make_project, tmp_path, tracks=(("Snare Top", "snare", 38), ("Kick In", "kick", 36)),
                 events={"Snare Top": events}, name=name)


def text(ins):
    row = ins.locator(LINE)
    return None if "hidden" in (row.get_attribute("class") or "") else row.inner_text().strip()


def test_the_inspector_counts_bleed_hits_and_a_restore_lowers_the_bleed_count(app, make_project, tmp_path):
    sid = open_project(app, flagged(make_project, tmp_path, "Flags", bleed={0, 1}, roll=set()))
    ins = inspect_track(app, "Snare Top")
    assert text(ins) == "Bleed rules: 2 hits out of the output"
    assert "hand velocity edit" in ins.locator(".insFlags").get_attribute("title")
    invoke(app, "events_edit", sid=sid, name="Snare Top", ops=[{"op": "restore", "index": 0}])
    assert sum(1 for e in service_events(app, sid, "Snare Top") if e.get("bleed")) == 1
    app.evaluate("() => loadEventsOnly()")
    wait_until(lambda: text(ins) == "Bleed rules: 1 hit out of the output", msg="the restore lowers the count")


def test_an_edited_bleed_hit_is_not_out_and_a_track_without_flags_shows_no_line(app, make_project, tmp_path):
    open_project(app, flagged(make_project, tmp_path, "Edited", bleed={0, 1}, roll=set(), edited={1}))
    ins = inspect_track(app, "Snare Top")
    assert text(ins) == "Bleed rules: 1 hit out of the output"      # no roll part; the edited hit is not out
    ins = inspect_track(app, "Kick In")
    assert text(ins) is None                                        # no flags: no line
