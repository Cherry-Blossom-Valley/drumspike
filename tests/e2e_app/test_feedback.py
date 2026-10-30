"""#164 (part 2 of #153): the Feedback button and its form. The gateway is mocked: the bridge
answers the command (static/backend.js records the payload in window.__DS_FEEDBACK__, or fails
once with window.__DS_FEEDBACK_FAIL__). Checked here: the preview shows the meta and no path or
name of the open test project; a text with a path is blocked; Send posts exactly the previewed
JSON; a gateway error keeps the text. The form is looked at in the browser pane and one
screenshot goes to test-shots/ (164-feedback-form.png)."""
import json

from app_support import invoke, open_project, shot_dir

PREVIEW = "() => JSON.parse(document.querySelector('#fbPreview').textContent)"
SENT = "() => window.__DS_FEEDBACK__ || []"


def open_form(app):
    app.click("#feedbackBtn")
    app.wait_for_selector("#fbModal:not(.hidden)")


def refresh(app):
    """The refresh runs on the fields' input events; a fill dispatches them, this only waits."""
    app.wait_for_timeout(50)


def test_feedback_form_previews_the_meta_and_never_a_path_or_project_name(app, make_project):
    sid = open_project(app, make_project())
    pname = invoke(app, "project_get", sid=sid)["name"]
    open_form(app)
    p = app.evaluate(PREVIEW)
    assert p["kind"] == "bug" and p["title"] == "" and p["description"] == ""
    meta = p["meta"]
    for key in ("app_version", "build_sha", "os", "arch", "language", "detector"):
        assert isinstance(meta[key], str) and meta[key], f"meta.{key}: {meta}"
    counts = meta["counts"]
    assert counts["tracks"] >= 1 and isinstance(counts["hits"], int), counts
    assert counts["sample_rate"] and isinstance(counts["session_length_s"], int), counts
    # counts only: no name, no path of the open test project anywhere in the preview
    text = app.inner_text("#fbPreview")
    assert pname and pname not in text, text
    assert "/home" not in text and "/Users" not in text and str(sid) not in text, text


def test_feedback_blocks_a_text_with_a_path_and_sends_exactly_the_preview(app, make_project):
    open_project(app, make_project())
    open_form(app)
    app.fill("#fbTitleIn", "Kick misses on 2 and 4")
    app.fill("#fbDesc", "It broke in /home/nobody/secret.wav every time.")
    refresh(app)
    assert app.evaluate("() => document.querySelector('#fbSend').disabled") is True
    assert "/home" in app.inner_text("#fbMsg")
    assert app.evaluate(SENT) == []
    # the guard runs in the command again (crates/app/src/feedback.rs): the bridge answers it here
    assert app.evaluate("() => typeof B.feedbackSend === 'function'")
    # the path out, the preview follows, and Send posts exactly the previewed JSON
    app.fill("#fbDesc", "The review queue lists them every time.")
    app.fill("#fbSteps", "Open the demo project, press D twice.")
    refresh(app)
    assert app.evaluate("() => document.querySelector('#fbSend').disabled") is False
    expect_payload = json.loads(app.inner_text("#fbPreview"))
    app.click("#fbSend")
    app.wait_for_function("() => (window.__DS_FEEDBACK__ || []).length > 0")
    sent = app.evaluate(SENT)[0]
    assert sent == expect_payload, "the POST is the previewed JSON"
    assert sent["kind"] == "bug" and sent["title"] == "Kick misses on 2 and 4"
    assert sent["steps"] == "Open the demo project, press D twice."
    assert sent["meta"]["counts"]["tracks"] >= 1
    assert "Sent" in app.inner_text("#fbMsg") and "issues/" in app.inner_text("#fbMsg")


def test_a_gateway_error_keeps_the_text(app, make_project):
    open_project(app, make_project())
    open_form(app)
    app.fill("#fbTitleIn", "Still broken")
    app.fill("#fbDesc", "The lanes flash black on zoom.")
    refresh(app)
    app.evaluate("() => { window.__DS_FEEDBACK_FAIL__ = '503: the feedback gateway answered 503'; }")
    app.click("#fbSend")
    app.wait_for_function("() => /Not sent/.test(document.querySelector('#fbMsg').textContent)")
    assert "503" in app.inner_text("#fbMsg")
    assert app.input_value("#fbTitleIn") == "Still broken"
    assert app.input_value("#fbDesc") == "The lanes flash black on zoom."
    assert len(app.evaluate(SENT)) == 1, "the failed POST is recorded once, nothing more"
    # the text survives a second attempt with the gateway back
    app.click("#fbSend")
    app.wait_for_function("() => (window.__DS_FEEDBACK__ || []).length === 2")
    assert "Sent" in app.inner_text("#fbMsg")


def test_the_form_in_the_browser_pane(app, make_project):
    """The look of the form: one screenshot of the open dialog (the coordinator's dir)."""
    open_project(app, make_project())
    open_form(app)
    app.fill("#fbTitleIn", "Kick misses on 2 and 4")
    app.fill("#fbDesc", "The review queue lists them every time.")
    refresh(app)
    shot = shot_dir("164-feedback-form.png")
    app.locator("#fbModal .modal-box").screenshot(path=str(shot))
    # the form is a modal of the page with the three fields and the preview
    for sel in ("#fbKind", "#fbTitleIn", "#fbDesc", "#fbSteps", "#fbPreview", "#fbSend", "#fbCancel"):
        assert app.locator(sel).is_visible(), sel
    app.click("#fbCancel")
    app.wait_for_selector("#fbModal.hidden", state="attached")
    # planted fault: the button would be missing from the top bar
    assert app.locator("#feedbackBtn").count() == 1
