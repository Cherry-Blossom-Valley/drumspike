"""P4-40e: the onset view of the Inspector's Layer section in the page on the bridge (silent:
DRUMSPIKE_AUDIO=null). The view draws the first 200 ms of the one-shot with a line at the onset;
a drag of the line sends one track_layer_set {onset} and overrides the measured onset (the render
and the export checker follow it), Reset sends onset null and goes back to the measured onset.
Every check runs once more against a planted fault and must fail there."""
import json
from pathlib import Path

import pytest
import soundfile as sf
from playwright.sync_api import expect

from app_support import render_layer, wait_until
from test_layer import HITS, SR, checked, choose, layer_of, layered, one_shot, save, shot_file, show


def view(app):
    return app.locator("#inspector .lyView")


def line_x(app, which="eff"):
    """The x of the onset line in CSS px on the view's canvas."""
    return app.evaluate(
        """([n, which]) => {
          const l = S.tracks[n].layer, c = document.querySelector('#inspector .lyView');
          const t = which === "eff" ? (l.onset_set != null ? l.onset_set : l.onset) : l.onset;
          return lyViewX(n, t / l.sr, c.clientWidth);
        }""",
        ["Kick In", which],
    )


def accent(app):
    return app.evaluate(
        "() => { const a = LN_CSS.getPropertyValue('--accent').trim(), n = parseInt(a.slice(1), 16); return [n >> 16 & 255, n >> 8 & 255, n & 255]; }"
    )


def column_color(app, x):
    """Whether one canvas column holds a pixel of the accent (the onset line)."""
    rgb = accent(app)
    return app.evaluate(
        """([x, rgb]) => {
          const c = document.querySelector('#inspector .lyView'), dpr = devicePixelRatio || 1;
          const d = c.getContext('2d').getImageData(Math.round(x * dpr), 0, 1, c.height).data;
          for (let i = 0; i < d.length; i += 4) {
            if (Math.abs(d[i] - rgb[0]) < 40 && Math.abs(d[i + 1] - rgb[1]) < 40 && Math.abs(d[i + 2] - rgb[2]) < 40) return true;
          }
          return false;
        }""",
        [x, rgb],
    )


def wave_pixels(app):
    """Grey waveform columns on the view (the one-shot drawn)."""
    return app.evaluate(
        """() => {
          const c = document.querySelector('#inspector .lyView'), dpr = devicePixelRatio || 1;
          const d = c.getContext('2d').getImageData(0, 0, c.width, c.height).data;
          let n = 0;
          for (let i = 0; i < d.length; i += 4) if (d[i] > 120 && d[i] < 160 && d[i + 1] > 130 && d[i + 1] < 165 && d[i + 2] > 140 && d[i + 2] < 175) n++;
          return n;
        }""",
    )


def info(app):
    return app.evaluate("() => document.querySelector('#inspector .lyViewInfo').textContent")


def drag_line(app, dx):
    """A drag of the onset line by dx CSS px (a pointer drag, as a user does)."""
    view(app).scroll_into_view_if_needed()                     # the canvas may sit below the fold
    box = view(app).bounding_box()
    x0 = box["x"] + line_x(app)
    y = box["y"] + box["height"] / 2
    app.mouse.move(x0, y)
    app.mouse.down()
    app.mouse.move(x0 + dx, y, steps=4)
    app.mouse.up()


def test_the_view_draws_the_one_shot_and_the_onset_line(app, layered):
    pdir, shots = layered
    sec = show(app, pdir)
    expect(sec.locator(".lyViewWrap")).to_be_hidden()          # no view without a one-shot
    choose(app, shot_file(shots, "kick.wav"))
    expect(sec.locator(".lyViewWrap")).to_be_visible()
    wait_until(lambda: wave_pixels(app) > 50, 15, "the view drew the one-shot")
    assert column_color(app, line_x(app)), "the onset line on the view"
    assert "onset" not in info(app)                            # the onset text lives only in .lyInfo
    assert "of 0.15 s" in info(app)
    cut = app.evaluate(
        "() => { const el = document.querySelector('#inspector .lyViewInfo'); return el.scrollWidth > el.clientWidth; }",
    )
    assert not cut, "the view info is not cut off at the default Inspector width"
    # planted fault: a line drawn 10 px off the onset is not the onset line
    assert not column_color(app, line_x(app) + 10), "the onset line is 10 px off"


def test_a_four_second_one_shot_draws_its_wave(app, layered, tmp_path):
    pdir, _ = layered
    sec = show(app, pdir)
    long_shot = tmp_path / "long.wav"
    sf.write(long_shot, one_shot(SR, pre_s=2.0, secs=4.0), SR, subtype="FLOAT")   # about 4 s, the sound at 2 s
    choose(app, str(long_shot))
    expect(sec.locator(".lyViewWrap")).to_be_visible()
    wait_until(lambda: wave_pixels(app) > 50, 15, "the view drew the 4 s one-shot")
    assert column_color(app, line_x(app)), "the onset line on the 4 s view"
    assert "from 1." in info(app), "the window opens at the onset, not at 0 s"
    assert "off the view" not in info(app)


def test_play_starts_the_audition_of_the_one_shot(app, layered):
    pdir, shots = layered
    sec = show(app, pdir)
    choose(app, shot_file(shots, "kick.wav"))
    sec.locator(".lyPlay").click()
    expect(app.locator("#stMsg")).to_contain_text("one-shot of Kick In")


def test_a_new_one_shot_is_drawn_not_the_cached_one(app, layered):
    pdir, shots = layered
    show(app, pdir)
    choose(app, shot_file(shots, "kick.wav", seed=1))
    wait_until(lambda: wave_pixels(app) > 50, 15, "the view drew the first one-shot")
    second = shot_file(shots, "kick2.wav", seed=5)             # the same length and rate: the same window
    choose(app, second)
    app.wait_for_function("p => LYV.cache['Kick In'] && LYV.cache['Kick In'].key === p", arg=second)
    assert wave_pixels(app) > 50


def test_a_failed_wave_read_shows_the_message_not_a_black_view(app, layered):
    pdir, shots = layered
    show(app, pdir)
    choose(app, shot_file(shots, "kick.wav", seed=1))
    wait_until(lambda: wave_pixels(app) > 50, 15, "the view drew the one-shot")
    app.evaluate("() => { B.layerWave = async () => { throw new Error('forced layer wave failure'); }; }")
    choose(app, shot_file(shots, "kick2.wav", seed=5))         # a new window read: the forced failure
    expect(app.locator("#inspector .lyViewInfo")).to_contain_text("forced layer wave failure")
    assert wave_pixels(app) == 0
    assert "the wave could not be read" in info(app)


def test_dragging_the_line_overrides_the_onset_and_the_render_moves(app, layered):
    pdir, shots = layered
    sec = show(app, pdir)
    choose(app, shot_file(shots, "kick.wav"))
    onset = layer_of(app)["onset"]
    W = app.evaluate("() => document.querySelector('#inspector .lyView').clientWidth")
    span = app.evaluate(
        "() => { const l = S.tracks['Kick In'].layer, w = lyViewWin('Kick In'); return (w.t1 - w.t0) * l.sr; }"
    )
    dx = 30                                                    # 30 px across the 200 ms window
    want = round(onset + dx / W * span)
    drag_line(app, dx)
    app.wait_for_function("w => S.tracks['Kick In'].layer.onset_set === w", arg=want)
    assert layer_of(app)["onset"] == onset                     # the measured onset is kept
    expect(sec.locator(".lyInfo")).to_contain_text("set by hand")
    assert "onset" not in info(app)
    # the render follows the override, and the export checker honours it (L2 on the document)
    render_layer(app, pdir)
    expect(app.locator("#stMsg")).to_contain_text("layer rendered to")
    save(app)
    found, rep = checked(pdir)
    assert found == [] and rep["tracks"]["Kick In"]["layer"]["onset"] == want
    out, sr = sf.read(pdir / "Exports" / "Kick In_layer.wav", dtype="float32")
    shot, _ = sf.read(shot_file(shots, "kick.wav"), dtype="float32")
    assert out[round(HITS[0] * sr)] == pytest.approx(shot[want], rel=1e-6)
    # planted fault: a checker that reads the measured onset misses the moved voices
    doc_path = next(pdir.glob("*.drumspike"))
    doc = json.loads(doc_path.read_text())
    saved = doc["tracks"]["Kick In"]["layer"].pop("onset_set")
    doc_path.write_text(json.dumps(doc))
    assert checked(pdir)[0], "the checker missed the override when it reads the measured onset"
    doc["tracks"]["Kick In"]["layer"]["onset_set"] = saved
    doc_path.write_text(json.dumps(doc))


def test_reset_returns_to_the_measured_onset(app, layered):
    pdir, shots = layered
    sec = show(app, pdir)
    choose(app, shot_file(shots, "kick.wav"))
    onset = layer_of(app)["onset"]
    drag_line(app, 30)
    app.wait_for_function("() => S.tracks['Kick In'].layer.onset_set != null")
    sec.locator(".lyReset").click()
    app.wait_for_function("() => S.tracks['Kick In'].layer.onset_set == null")
    assert layer_of(app)["onset"] == onset
    expect(sec.locator(".lyInfo")).to_contain_text("measured")
    expect(sec.locator(".lyInfo")).not_to_contain_text("set by hand")
    assert column_color(app, line_x(app)), "the line back on the measured onset"
    render_layer(app, pdir)
    expect(app.locator("#stMsg")).to_contain_text("layer rendered to")
    save(app)
    assert checked(pdir)[0] == []
    # planted fault: a Reset that sends 0 instead of null keeps an override (of sample 0)
    drag_line(app, 30)
    app.wait_for_function("() => S.tracks['Kick In'].layer.onset_set != null")
    app.evaluate("() => lySet('Kick In', { onset: 0 })")
    app.wait_for_function("() => S.tracks['Kick In'].layer.onset_set === 0")
    assert layer_of(app)["onset"] == onset
    expect(sec.locator(".lyInfo")).to_contain_text("set by hand")


def test_the_onset_section_names_its_parts_and_the_cursor_shows_the_drag(app, layered):
    pdir, shots = layered
    sec = show(app, pdir)
    choose(app, shot_file(shots, "kick.wav"))
    expect(sec.locator(".lyViewTitle")).to_have_text("Onset")
    expect(sec.locator(".lyViewHint")).to_be_visible()
    expect(sec.locator(".lyPlay")).to_have_text("Play one-shot")
    expect(sec.locator(".lyReset")).to_have_text("Reset onset")
    expect(sec.locator(".lyInfo")).to_contain_text("measured")
    view(app).scroll_into_view_if_needed()
    box = view(app).bounding_box()
    y = box["y"] + box["height"] / 2
    app.mouse.move(box["x"] + line_x(app), y)                  # on the line: the line moves
    app.wait_for_function("() => document.querySelector('#inspector .lyView').style.cursor === 'ew-resize'")
    app.mouse.move(box["x"] + line_x(app) + 40, y)             # off the line: the wave pans
    app.wait_for_function("() => document.querySelector('#inspector .lyView').style.cursor === 'grab'")
    assert column_color(app, line_x(app)), "the onset line with its handle"
