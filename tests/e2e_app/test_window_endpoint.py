"""#388: the agent drives the running window through the local endpoint (crates/app/src/window_relay.rs): the bridge opens
it like the app does (DRUMSPIKE_WINDOW_ENDPOINT=1), the page long-polls and runs the named DSAgent helper, the answer comes
back over HTTP. A call needs the token of <home>/window-endpoint.json; what the endpoint returns is what the page itself
answers; a move through it changes the hit and the status line of the window."""
import json
import urllib.error
import urllib.request

from app_support import edits_done
from test_agent_helpers import TRACK, opened
from test_training_mode import project_log_lines
from test_editor import IDX


def endpoint(warm_page):
    info = json.loads((warm_page.home / "window-endpoint.json").read_text())
    return f"http://127.0.0.1:{info['port']}", info["token"]


def post(base, token, fn, args=(), timeout_ms=20000):
    req = urllib.request.Request(
        base + "/call", data=json.dumps({"fn": fn, "args": list(args), "timeout_ms": timeout_ms}).encode(), method="POST",
        headers={"Authorization": f"Bearer {token}"} if token else {})
    try:
        with urllib.request.urlopen(req, timeout=40) as r:
            return r.status, json.loads(r.read())
    except urllib.error.HTTPError as e:
        return e.code, json.loads(e.read())


def test_a_call_without_the_token_is_refused_and_the_page_never_sees_it(app, warm_page, make_project, tmp_path):
    opened(app, make_project, tmp_path)
    base, token = endpoint(warm_page)
    assert post(base, None, "hits", [TRACK])[0] == 401
    assert post(base, "wrong", "hits", [TRACK])[0] == 401
    assert (warm_page.home / "window-endpoint.json").stat().st_mode & 0o077 == 0   # the owner only
    assert post(base, token, "hits", [TRACK])[0] == 200


def test_look_move_and_confirm_through_the_endpoint_change_the_window_and_its_status_line(app, warm_page, make_project, tmp_path):
    opened(app, make_project, tmp_path)
    base, token = endpoint(warm_page)

    def ask(fn, *args):
        status, body = post(base, token, fn, args)
        assert status == 200, body
        return body["value"]

    own = app.evaluate("async () => await DSAgent.hits('Kick In')")
    assert ask("hits", TRACK) == own                                    # the endpoint answers what the page answers
    assert [h["sample"] for h in ask("hits", TRACK)] == IDX

    status, body = post(base, token, "move", [TRACK, 1, 3])
    assert status == 409 and "Training Mode is off" in body["message"]   # the page's own refusal comes back
    assert ask("setTraining", True)["training"] is True
    before = len(project_log_lines(app))
    proj = ask("project")
    assert proj["tracks"] == [TRACK] or TRACK in proj["tracks"], proj

    seen = ask("look", TRACK, 1)
    assert seen["sample"] == IDX[1]
    line = ask("statusLine")
    assert line["selection"].startswith(f"{TRACK}: hit 2/3"), line

    moved = ask("move", TRACK, 1, 3)
    assert moved["moved"] is True and moved["from"] == IDX[1] and moved["to"] == IDX[1] + 3
    assert [h["sample"] for h in ask("hits", TRACK)][1] == IDX[1] + 3   # the hit moved in the window
    assert app.evaluate("() => S.events['Kick In'][1].t * S.tracks['Kick In'].sr") == IDX[1] + 3
    after_move = ask("statusLine")
    assert after_move["message"] and after_move["training"] == "training: 1 records", after_move
    edits_done(app)
    (rec,) = project_log_lines(app)[before:]
    assert (rec["op"], rec["by"]) == ("move", "agent"), rec   # #389: a move through the endpoint says who made it

    confirmed = ask("confirm", TRACK, 2)
    assert confirmed == {"confirmed": True, "index": 2}
    after_confirm = ask("statusLine")
    assert after_confirm["selection"].startswith(f"{TRACK}: hit 3/3")
    assert after_confirm["message"] != after_move["message"] or after_confirm["training"] != after_move["training"]
    assert ask("setTraining", False)["training"] is False
