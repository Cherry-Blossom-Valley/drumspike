"""#150: the review queue (workflow polish rank 2). The service rates each hit (`track_confidence`,
`crates/app/src/confidence.rs`); D steps through the doubtful hits of all tracks, lowest confidence first, selects and
scrolls to each, then says "no more doubtful hits"; Shift+D steps back. The info line names why the selected hit is
doubtful. The project: 200 hits on two tracks, 10 of them planted doubtful (rescue and roll fill hits at different
levels under the track median). Hits that look doubtful but are not (a flam at full level, a cut rescue hit, an added
quiet hit) stay out of the queue. On a model track the model's margin rates a hit, not its level: a quiet hit (a ghost
note) with a strong activation is not doubtful."""
import struct
from pathlib import Path

import pytest
from playwright.sync_api import expect

from app_support import invoke, menu, open_project, wait_until
from helpers import SR, synth_track, write_wav
from test_subtracks import analyse, has_model, js

N = 100                       # hits per track, 0.5 s apart
STEP = 0.5
BASE_DB = -3.0                # the level of a plain hit (the track median)
KICK_RESCUE = {10: 1, 25: 2, 40: 3, 55: 4, 70: 5, 85: 6}   # hit index -> dB under the median
SNARE_ROLL = {5: 2, 30: 5, 60: 8, 95: 11}
KICK_FLAM = 50                # full level: confidence 0.5, not doubtful
KICK_CUT = 15                 # a rescue hit out of the output
SNARE_ADDED = 20              # a quiet roll hit the user added


def onsets():
    return [1.0 + k * STEP for k in range(N)]


def track_events(tags):
    out = []
    for k, t in enumerate(onsets()):
        e = {"t": round(t * SR) / SR, "v": 100}
        e.update(tags.get(k, {}))
        out.append(e)
    return out


def build(make_project, tmp_path):
    src = tmp_path / "src_Review"
    src.mkdir()
    kick_q = {**KICK_RESCUE, KICK_CUT: 14}
    snare_q = {**SNARE_ROLL, SNARE_ADDED: 20}
    specs = []
    for name, role, note, quiet, tags in (
        ("Kick In", "kick", 36, kick_q,
         {**{k: {"rescue": True} for k in KICK_RESCUE}, KICK_FLAM: {"flam": True}, KICK_CUT: {"rescue": True, "cut": True}}),
        ("Snare Top", "snare", 38, snare_q,
         {**{k: {"roll": True} for k in SNARE_ROLL}, SNARE_ADDED: {"roll": True, "added": True}}),
    ):
        x, _ = synth_track(onsets(), 1.0 + N * STEP + 1.0,
                           gains_db=[BASE_DB - quiet.get(k, 0) for k in range(N)])
        wav = write_wav(src / f"{name}.wav", x)
        specs.append((name, wav, role, note, track_events(tags)))
    return Path(make_project.builder.build_project(tmp_path / "projects_Review", "Review", specs, "copied"))


def expected_queue():
    """[(track, t, c)] of the planted doubtful hits, lowest confidence first (the cap of the tag x the level)."""
    q = [("Kick In", onsets()[k], 0.3 * (1 - db / 24)) for k, db in KICK_RESCUE.items()]
    q += [("Snare Top", onsets()[k], 0.4 * (1 - db / 24)) for k, db in SNARE_ROLL.items()]
    return sorted(q, key=lambda p: p[2])


def selected(page):
    return js(page, """() => { const e = S.sel && S.events[S.sel.track][S.sel.index];
      return e ? {track: S.sel.track, t: e.t, t0: S.view.t0, t1: S.view.t1, ph: S.playhead} : null; }""")


def press_and_wait(page, key, n_msg):
    """Press the key; wait for the status message of this step (the step is async: the service rates first)."""
    page.evaluate("() => { document.querySelector('#stMsg').textContent = ''; }")
    page.keyboard.press(key)
    page.wait_for_function("s => document.querySelector('#stMsg').textContent === s", arg=n_msg)


def test_the_service_gives_one_confidence_per_hit(app, make_project, tmp_path):
    sid = open_project(app, build(make_project, tmp_path))
    kick = invoke(app, "track_confidence", sid=sid, name="Kick In")
    assert kick["source"] in ("level", "model") and kick["doubt_below"] == 0.5
    conf = kick["confidence"]
    assert len(conf) == N and all(0.0 <= r["c"] <= 1.0 for r in conf)
    doubt = [k for k, r in enumerate(conf) if r["doubt"]]
    assert doubt == sorted(KICK_RESCUE), doubt
    for k, db in KICK_RESCUE.items():
        assert abs(conf[k]["c"] - 0.3 * (1 - db / 24)) < 0.002, (k, conf[k])
        assert conf[k]["why"] == ["kick rescue (no model hit)"], conf[k]
    assert conf[KICK_FLAM] == {"c": 0.5, "doubt": False, "why": ["flam second stroke"]}
    assert conf[KICK_CUT]["why"] == ["not in the output"] and not conf[KICK_CUT]["doubt"]
    assert conf[0] == {"c": 1.0, "doubt": False, "why": []}
    snare = invoke(app, "track_confidence", sid=sid, name="Snare Top")["confidence"]
    assert [k for k, r in enumerate(snare) if r["doubt"]] == sorted(SNARE_ROLL)
    assert snare[SNARE_ADDED]["why"] == ["edited by hand"]
    # the page's list wins over the stored one: without the tags only the level counts, and only the hit 14 dB
    # under the median (the cut one, now in the output) is doubtful
    plain = [{"t": e["t"], "v": 100} for e in invoke(app, "events_get", sid=sid, name="Kick In")["events"]]
    rated = invoke(app, "track_confidence", sid=sid, name="Kick In", events=plain)["confidence"]
    assert [k for k, r in enumerate(rated) if r["doubt"]] == [KICK_CUT]
    assert rated[KICK_CUT]["why"] == ["14 dB under the track median"], rated[KICK_CUT]


def test_d_visits_the_doubtful_hits_lowest_first_then_says_no_more(app, make_project, tmp_path):
    open_project(app, build(make_project, tmp_path))
    js(app, "() => { S.sel = null; setView(0, 2); updateStatus(); }")    # zoomed in: each step must scroll
    want = expected_queue()
    assert len(want) == 10
    seen = []
    for i, (track, t, c) in enumerate(want):
        press_and_wait(app, "d", f"doubtful hit {i + 1} of 10")
        s = selected(app)
        assert s["track"] == track and abs(s["t"] - t) < 1e-6, (i, s, track, t)
        assert s["t0"] <= s["t"] <= s["t1"], ("not scrolled to the hit", s)
        assert abs(s["ph"] - t) < 1e-4
        why = app.locator("#ilDoubt").inner_text()
        expect(app.locator("#ilDoubtF")).to_be_visible()
        assert why.startswith(f"{round(c, 3):.2f}: "), (why, c)          # the service rounds c to 3 decimals
        assert ("kick rescue (no model hit)" if track == "Kick In" else "roll fill") in why, why
        seen.append((track, round(s["t"], 6)))
    assert len(set(seen)) == 10
    press_and_wait(app, "d", "no more doubtful hits")
    s = selected(app)
    assert (s["track"], round(s["t"], 6)) == seen[-1]                    # the last hit stays selected
    press_and_wait(app, "Shift+D", "doubtful hit 9 of 10")
    s = selected(app)
    assert (s["track"], round(s["t"], 6)) == seen[-2]
    # a plain hit selected (Tab): the info line has no Doubt field, and D starts at the lowest again
    js(app, "() => { S.sel = {track: 'Kick In', index: 0}; updateStatus(); }")
    expect(app.locator("#ilDoubtF")).to_be_hidden()
    press_and_wait(app, "d", "doubtful hit 1 of 10")
    assert (selected(app)["track"], round(selected(app)["t"], 6)) == seen[0]
    press_and_wait(app, "Shift+D", "no earlier doubtful hits")


def test_an_edit_takes_a_hit_out_of_the_queue(app, make_project, tmp_path):
    open_project(app, build(make_project, tmp_path))
    press_and_wait(app, "d", "doubtful hit 1 of 10")
    first = selected(app)
    app.keyboard.press(".")                                               # nudge: the hit is moved by hand
    wait_until(lambda: abs(selected(app)["t"] - first["t"]) > 1e-9, msg="the nudge moves the hit")
    expect(app.locator("#ilDoubtF")).to_be_hidden()                       # the old rating is not shown for the moved hit
    press_and_wait(app, "d", "doubtful hit 1 of 9")                       # the moved hit left the queue
    assert selected(app)["t"] != first["t"]


def test_the_review_commands_are_in_the_keymap_the_menu_and_the_help_panel(app, make_project, tmp_path):
    rows = js(app, "() => DSKeys.COMMANDS.filter(c => c.id.startsWith('review_')).map(c => [c.id, c.menu, c.path, c.title, DSKeys.keysOf(c, 'default')])")
    assert rows == [["review_next", "review_next", "Hits", "Next Doubtful Hit", ["D"]],
                    ["review_prev", "review_prev", "Hits", "Previous Doubtful Hit", ["Shift+D"]]]
    open_project(app, build(make_project, tmp_path))
    assert menu(app, "review_next") is True                              # the native menu item runs the step
    wait_until(lambda: app.locator("#stMsg").inner_text() == "doubtful hit 1 of 10", msg="Hits > Next Doubtful Hit")
    assert menu(app, "review_prev") is True
    wait_until(lambda: app.locator("#stMsg").inner_text() == "no earlier doubtful hits", msg="Hits > Previous Doubtful Hit")
    menu(app, "shortcuts")
    expect(app.locator("#helpModal")).to_be_visible()
    row = app.locator("#keymapTable tr[data-cmd='review_next']")
    expect(row).to_contain_text("Hits > Next Doubtful Hit")
    expect(row).to_contain_text("D")
    app.click("#helpDone")
