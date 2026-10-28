"""P4-5: the agent API end to end. The same session goes through three routes and the exports must be the same bytes:

  app   the app's own commands as the page sends them (`service_tool run`, the test bridge's `invoke`: whole event
        lists with `events_set`, no project lock), phase by phase;
  cli   the command-line tool, one process per command (`service_tool cli <command> --project <folder>`; the app binary
        carries the same engine as `drumspike-desktop cli`), with the project lock;
  mcp   the MCP server (`service_tool mcp`, JSON-RPC lines over stdio), one process for the whole run.

Steps (the same for all three, each on its own copy of the base project): set the detection params of the kick (sensitivity 70, a minimum
gap), analyse, set the velocity map and a loudness cut, edit the hits (an add, a move, a delete and a velocity; the app
route writes the resulting list), make the kick sub follow the kick in, save, Export All. Then every file of `Exports/`
is compared byte for byte, the documents' hits are compared, and `tools/check_exports.py` (P4-4) must find no problem.

Fast case (G2): a synthetic session and the DSP detector. Slow case: `DRUMSPIKE_AGENT_SESSION=<a real session folder with
"Audio Files/">` (the Session A session in ~/.cache/drumspike/sessions, read only: a scratch project references its audio) and the
model; `DRUMSPIKE_AGENT_REPORT=<file.json>` writes the numbers (seconds per route and per step)."""
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

import pytest

from core_tools import SERVICE_TOOL, require

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
sys.path.insert(0, str(ROOT / "apps" / "desktop" / "selftest"))
import check_exports  # noqa: E402
from make_project import build_project  # noqa: E402
from test_exports import STANDARD, drum, write_wav  # noqa: E402

K, F, S = "k in", "k sub", "s top"


class Fail(AssertionError):
    pass


def agent_env(home, extra=None):
    return {**os.environ, "DRUMSPIKE_HOME": str(home / "home"), "DRUMSPIKE_PROJECTS": str(home / "projects"),
            **(extra or {})}


class Route:
    """One way to run a command on the project. `call(cmd, **args)` returns the answer; commands that take a `sid` get
    the route's own project; `analyze_start` waits for the job."""

    name = "?"

    def velocity_map(self, name, lo, hi, cut):
        """An agent route: `track_velocity_set` stores the velocities and cut marks itself (apply defaults to true)."""
        r = self.call("track_velocity_set", name=name, lo_db=lo, hi_db=hi, cut_db=cut)
        assert r.get("applied") is True, r
        return r

    def __init__(self, tool, home):
        self.exe, self.home = tool, home
        self.env = agent_env(home, {})
        self.times = {}
        self.folder = None

    def timed(self, cmd, fn):
        t0 = time.monotonic()
        r = fn()
        self.times[cmd] = round(self.times.get(cmd, 0) + time.monotonic() - t0, 3)
        return r


class AppRoute(Route):
    name = "app"

    def run(self, steps):
        f = self.home / f"script{len(list(self.home.glob('script*.json')))}.json"
        f.write_text(json.dumps(steps))
        r = subprocess.run([str(self.exe), "run", str(f)], capture_output=True, text=True, env=self.env)
        assert r.returncode == 0, r.stderr[-800:]
        rows = [json.loads(line) for line in r.stdout.splitlines() if line.strip()]
        for row in rows:
            assert "answer" in row, row
        return rows

    def call(self, cmd, **args):
        def go():
            steps = [{"cmd": "project_open", "args": {"path": str(self.folder)}},
                     {"cmd": cmd, "args": {"sid": "$sid", **args}}]
            if cmd == "analyze_start":
                steps.append({"cmd": "wait_analysis", "args": {"sid": "$sid"}})
            steps.append({"cmd": "project_save", "args": {"sid": "$sid"}})
            rows = self.run(steps)
            if cmd == "analyze_start":
                return {**rows[1]["answer"], "status": rows[2]["answer"]}
            return rows[1]["answer"]
        return self.timed(cmd, go)

    def edit(self, name, ops, edited):
        return self.call("events_set", name=name, events=edited)

    def velocity_map(self, name, lo, hi, cut):
        """The page applies the answer itself (one undo step): events_set with the answered hits."""
        vs = self.call("track_velocity_set", name=name, lo_db=lo, hi_db=hi, cut_db=cut)["events"]
        return self.call("events_set", name=name, events=vs)


class CliRoute(Route):
    name = "cli"

    def call(self, cmd, **args):
        argv = [str(self.exe), "cli", cmd, "--project", str(self.folder), "--args", json.dumps(args)]

        def go():
            r = subprocess.run(argv, capture_output=True, text=True, env=self.env)
            if r.returncode != 0:
                raise Fail(f"{cmd}: exit {r.returncode}: {r.stderr[-600:]}")
            if cmd == "analyze_start":
                assert '"progress"' in r.stderr, "analysis prints progress lines on stderr"
            return json.loads(r.stdout)
        return self.timed(cmd, go)

    def edit(self, name, ops, edited):
        return self.call("events_edit", name=name, ops=ops)


class McpRoute(Route):
    name = "mcp"

    def __init__(self, *a):
        super().__init__(*a)
        self.proc = subprocess.Popen([str(self.exe), "mcp"], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                     stderr=subprocess.DEVNULL, text=True, env=self.env)
        self.n = 0
        self.sid = None
        init = self.rpc("initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                                       "clientInfo": {"name": "test", "version": "0"}})
        assert init["protocolVersion"] == "2025-06-18" and "tools" in init["capabilities"]
        self.proc.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n")
        self.proc.stdin.flush()
        self.tools = {t["name"]: t for t in self.rpc("tools/list")["tools"]}

    def rpc(self, method, params=None):
        self.n += 1
        self.proc.stdin.write(json.dumps({"jsonrpc": "2.0", "id": self.n, "method": method,
                                          **({"params": params} if params is not None else {})}) + "\n")
        self.proc.stdin.flush()
        line = self.proc.stdout.readline()
        assert line, "the MCP server closed its output"
        msg = json.loads(line)  # only JSON-RPC on stdout
        assert msg["id"] == self.n, msg
        assert "error" not in msg, msg
        return msg["result"]

    def tool(self, tool_name, /, **args):
        name = tool_name
        res = self.rpc("tools/call", {"name": name, "arguments": args})
        body = json.loads(res["content"][0]["text"])
        if res["isError"]:
            raise Fail(f"{name}: {body}")
        if isinstance(body, dict):
            assert res["structuredContent"] == body
        return body

    def call(self, cmd, /, **args):
        def go():
            if self.sid is None:
                self.sid = self.tool("project_open", path=str(self.folder))["sid"]
            out = self.tool(cmd, sid=self.sid, **args)
            if cmd == "analyze_start":
                while (st := self.tool("analyze_status", sid=self.sid))["state"] in ("queued", "running"):
                    time.sleep(0.05)
                out = {**out, "status": st}
            elif cmd == "export_all":
                assert "job" in out, "a Progress tool answers a job id at once"
                while (job := self.tool("job_get", job=out["job"]))["state"] == "running":
                    time.sleep(0.05)
                assert job["state"] == "done", job
                out = job["result"]
            return out
        return self.timed(cmd, go)

    def edit(self, name, ops, edited):
        return self.call("events_edit", name=name, ops=ops)

    def close(self):
        self.proc.stdin.close()
        self.proc.wait(timeout=30)


def scenario(r):
    """The steps on the project folder of the route (a copy of the base project); returns the answer of Export All."""
    r.call("track_params_set", name=K, params={"sensitivity": 70, "min_gap_ms": 25, "refine": True,
                                                "vel_lo_db": -40, "vel_hi_db": -6})
    st = r.call("analyze_start")["status"]
    assert st["state"] == "done" and all(t["state"] == "done" for t in st["tracks"].values()), st
    # the velocity map first: it recomputes every velocity, so the edits below survive in the exports
    r.velocity_map(K, -45, -8, -60)
    got = r.call("events_get", name=K)
    ev, sr = got["events"], got["sr"]
    assert len(ev) >= 6, len(ev)
    new_t = (round(ev[1]["t"] * sr) + 7) / sr
    add_t = round((ev[0]["t"] + ev[1]["t"]) / 2 * sr) / sr
    ops = [{"op": "add", "t": add_t}, {"op": "move", "index": 1, "t": new_t}, {"op": "delete", "index": 2},
           {"op": "velocity", "index": 3, "v": 127}]
    edited = [dict(e) for e in ev]
    edited[1] = {**edited[1], "t": new_t, "orig": ev[1]["t"]}
    edited[1].pop("cut", None)          # an edited hit is never cut (events_edit and the page both drop the mark)
    edited[3] = {**edited[3], "v": 127}
    edited = [e for i, e in enumerate(edited) if i != 2] + [{"t": add_t, "v": 100, "added": True}]
    edited.sort(key=lambda e: e["t"])
    r.edit(K, ops, edited)
    r.call("track_set_follow", name=F, source=K)
    r.call("project_save")
    return r.call("export_all")


def doc_events(folder, name):
    doc = json.loads(next(Path(folder).glob("*.drumspike")).read_text())
    return doc["tracks"][name]["events"], doc["tracks"][name].get("params", {})


def run_all(base_project, tmp, report_name=None):
    tool = require(SERVICE_TOOL)
    app_home = tmp / "app"
    app_home.mkdir()
    routes, answers = [], {}
    for cls in (AppRoute, CliRoute, McpRoute):
        home = tmp / cls.name if cls is not AppRoute else app_home
        home.mkdir(exist_ok=True)
        r = cls(tool, home)
        r.folder = tmp / "projects" / cls.name / base_project.name   # P4-30: each route works on its own copy
        shutil.copytree(base_project, r.folder)
        routes.append(r)
        try:
            t0 = time.monotonic()
            answers[r.name] = scenario(r)
            r.times["total"] = round(time.monotonic() - t0, 3)
        finally:
            if isinstance(r, McpRoute):
                r.close()
    return routes, answers


def compare(routes, answers):
    app, cli, mcp = routes
    files = {r.name: sorted(p.name for p in (r.folder / "Exports").iterdir()) for r in routes}
    assert files["app"] == files["cli"] == files["mcp"] and len(files["app"]) >= 2 * 3, files
    differ = []
    for name in files["app"]:
        digests = {r.name: (r.folder / "Exports" / name).read_bytes() for r in routes}
        if not (digests["app"] == digests["cli"] == digests["mcp"]):
            differ.append(name)
    assert differ == [], f"exports differ between the routes: {differ}"
    for name in (K, F):
        ref, _ = doc_events(app.folder, name)
        for r in (cli, mcp):
            assert doc_events(r.folder, name)[0] == ref, f"{name}: hits of {r.name} differ from the app route"
    ref_params = doc_events(app.folder, K)[1]
    assert ref_params["sensitivity"] == 70 and ref_params["vel_lo_db"] == -45 and ref_params["vel_cut_db"] == -60
    ref_hits = doc_events(app.folder, K)[0]
    assert any(e.get("added") for e in ref_hits) and any("orig" in e for e in ref_hits) and any(e["v"] == 127 for e in ref_hits)
    for r in (cli, mcp):
        assert doc_events(r.folder, K)[1] == ref_params
    for r in routes:
        rep = check_exports.check_project(str(r.folder), answers[r.name])
        bad = rep["problems"] + [f"{n}: {p}" for n, t in rep["tracks"].items() for p in t["problems"]]
        assert bad == [], f"{r.name}: {bad}"
        assert answers[r.name]["errors"] == []
    return {"files": files["app"], "tracks_checked": len(rep["tracks"])}


def synthetic_base(tmp):
    a = tmp / "audio"
    a.mkdir()
    sr = 48_000
    idx = [int(sr * (0.4 + 0.37 * i)) for i in range(14)]
    tracks = []
    for name, role, note, gain in ((K, "kick", 36, 0.8), (F, "kick", 35, 0.3), (S, "snare", 38, 0.7)):
        sub = [i - 480 for i in idx] if name == F else (idx if name == K else [i + 9000 for i in idx])
        w = write_wav(a / f"{name}.wav", drum(sr, 7, sub, gain), sr, "pcm24")
        tracks.append((name, str(w), role, note))
    return Path(build_project(tmp / "base", "Agent", tracks, "copied"))


def test_the_three_routes_export_the_same_bytes_on_a_synthetic_session():
    tmp = Path(tempfile.mkdtemp(prefix="ds_agent_"))
    try:
        routes, answers = run_all(synthetic_base(tmp), tmp)
        info = compare(routes, answers)
        assert info["tracks_checked"] == 3
        mcp = routes[2]
        # every tool of the agent surface is on the list, with a schema and the save hint where it matters
        assert "job_get" in mcp.tools and "events_edit" in mcp.tools
        assert "project_save" in mcp.tools["events_edit"]["description"]
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


@pytest.mark.skipif(not os.environ.get("DRUMSPIKE_AGENT_SESSION"),
                    reason="slow: set DRUMSPIKE_AGENT_SESSION to a real session folder (Audio Files/ inside)")
def test_the_three_routes_export_the_same_bytes_on_a_real_session():
    session = Path(os.environ["DRUMSPIKE_AGENT_SESSION"])
    tracks = []
    for f in sorted((session / "Audio Files").glob("*.wav")):
        stem = f.stem.rsplit("_", 1)[0].lower()
        key = next((k for k in list(STANDARD) + ["oh", "hh", "r far"] if stem == k), None)
        if key and key not in [t[0] for t in tracks]:
            role, note = STANDARD.get(key, ("cymbal", 49))
            tracks.append((key, str(f), role, note))
    assert {K, F, S} <= {t[0] for t in tracks}, tracks
    tmp = Path(tempfile.mkdtemp(prefix="ds_agent_real_"))
    try:
        base = Path(build_project(tmp / "base", "Session A", tracks, "referenced"))
        t0 = time.monotonic()
        routes, answers = run_all(base, tmp)
        info = compare(routes, answers)
        report = {"session": session.name, "tracks": len(tracks), "seconds": round(time.monotonic() - t0, 1),
                  "routes": {r.name: r.times for r in routes}, **info}
        print(json.dumps(report))
        if os.environ.get("DRUMSPIKE_AGENT_REPORT"):
            Path(os.environ["DRUMSPIKE_AGENT_REPORT"]).write_text(json.dumps(report, indent=1))
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
