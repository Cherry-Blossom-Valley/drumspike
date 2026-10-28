"""Chromium precise coverage of static/app.js, backend.js, lanes-gl.js, tiles-worker.js and keymap.js (CDP), merged over every test that
uses the `page` fixture: tests/e2e_app (the page against the app's test bridge, P3-7a; since P3-12 the only browser suite).
A pytest plugin (tests/conftest.py: pytest_plugins). Result: coverage-js.json (per file the lines, the covered lines,
the uncovered line numbers and the named functions never called) and a line per file in the terminal summary.
The run fails when a file is under DRUMSPIKE_JS_FAIL_UNDER (85) % of its lines, unless only a part of the suite
ran (-k, or -m other than "e2e_app"): the full run (G2) and the run of -m e2e_app alone (G4) gate.
CI shards (DRUMSPIKE_E2E_SHARD, tests/e2e_app/conftest.py) do not gate; with DRUMSPIKE_JS_COV_OUT=<file> they save the
raw merged data, and `python tests/jscov.py merge <files>` merges the files, writes coverage-js.json and gates."""
import base64
import hashlib
import json
import os
from pathlib import Path

import pytest

from helpers import ROOT

JS_FAIL_UNDER = float(os.environ.get("DRUMSPIKE_JS_FAIL_UNDER", "85"))   # % of lines, for each file
JS_FILES = ("app.js", "backend.js", "lanes-gl.js", "tiles-worker.js", "keymap.js")   # tiles-worker.js: a test page runs it with a stand-in scope; keymap.js: P3-13b

# "covered" and "src" map a file name to its coverage bytes and its source; "called" maps
# "file:function" to whether it ran; "take" saves the coverage of the open page (reload helper)
JS_COV = {"covered": {}, "src": {}, "called": {}, "take": lambda: None}


def _merge(res):
    for script in res["result"]:
        name = next((f for f in JS_FILES if script["url"].split("?")[0].endswith("/" + f)), None)
        if name is None:
            continue
        src = (ROOT / "static" / name).read_text()
        units = len(src.encode("utf-16-le")) // 2                 # CDP offsets are UTF-16 code units
        cov = bytearray(units)
        for fn in script["functions"]:
            for r in fn["ranges"]:                             # nested ranges follow their parent
                end = min(r["endOffset"], units)
                cov[r["startOffset"]:end] = (b"\x01" if r["count"] else b"\x00") * (end - r["startOffset"])
            if fn["functionName"]:
                key = f"{name}:{fn['functionName']}"
                was = JS_COV["called"].get(key, False)
                JS_COV["called"][key] = was or bool(fn["ranges"][0]["count"])
        old = JS_COV["covered"].get(name)
        JS_COV["covered"][name] = cov if old is None else bytearray(a | b for a, b in zip(old, cov))
        JS_COV["src"][name] = src


@pytest.fixture(autouse=True)
def _js_coverage(request):
    if "page" not in request.fixturenames:
        yield
        return
    if getattr(request.node, "callspec", None) and request.node.callspec.params.get("browser_name", "chromium") != "chromium":
        yield   # #463: the coverage probe is CDP (chromium only); other browsers run without it
        return
    page = request.getfixturevalue("page")
    cdp = page.context.new_cdp_session(page)
    cdp.send("Profiler.enable")
    cdp.send("Profiler.startPreciseCoverage", {"callCount": True, "detailed": True})

    def take():
        try:
            _merge(cdp.send("Profiler.takePreciseCoverage"))
        except Exception:
            pass
    JS_COV["take"] = take
    yield
    take()


def build_report(covered, srcs, called):
    """Per file: lines, covered lines, percent, never-called functions, uncovered line numbers."""
    files = {}
    for name, cov in covered.items():
        src = srcs[name]
        total = hit = pos = 0
        missed = []
        for lineno, line in enumerate(src.splitlines(keepends=True), 1):
            offsets, o = [], pos
            for ch in line:
                if not ch.isspace():
                    offsets.append(o)
                o += len(ch.encode("utf-16-le")) // 2
            body = line.strip()
            if body and not body.startswith(("//", "/*", "*")):
                total += 1
                ok = any(cov[x] for x in offsets if x < len(cov))
                hit += ok
                if not ok:
                    missed.append(lineno)
            pos = o
        never = sorted(k.split(":", 1)[1] for k, v in called.items() if not v and k.startswith(name + ":"))
        files[name] = {"file": f"static/{name}", "lines": total, "covered": hit,
                       "percent": round(100 * hit / max(total, 1), 1), "never_called": never,
                       "uncovered_lines": missed}
    report = dict(files["app.js"]) if "app.js" in files else {}
    report["files"] = files
    return report


def low_files(report):
    return {n: f["percent"] for n, f in report["files"].items() if f["percent"] < JS_FAIL_UNDER}


def dump_raw(path):
    """Raw merged coverage of this run (a CI shard): bytes as base64, the "called" map, the SHA-256 of each source."""
    raw = {"covered": {n: base64.b64encode(bytes(c)).decode() for n, c in JS_COV["covered"].items()},
           "called": JS_COV["called"],
           "sha256": {n: hashlib.sha256(s.encode()).hexdigest() for n, s in JS_COV["src"].items()}}
    Path(path).write_text(json.dumps(raw))


def merge_raw(paths):
    """OR the raw files of several shards; returns (report, covered, called). Sources are read from static/."""
    covered, called, srcs = {}, {}, {}
    for p in paths:
        raw = json.loads(Path(p).read_text())
        for name, b64 in raw["covered"].items():
            src = (ROOT / "static" / name).read_text()
            if hashlib.sha256(src.encode()).hexdigest() != raw["sha256"][name]:
                raise SystemExit(f"{p}: static/{name} differs from the source of the shard run")
            cov = bytearray(base64.b64decode(b64))
            old = covered.get(name)
            covered[name] = cov if old is None else bytearray(a | b for a, b in zip(old, cov))
            srcs[name] = src
        for k, v in raw["called"].items():
            called[k] = called.get(k, False) or v
    return build_report(covered, srcs, called)


def main(argv):
    """python tests/jscov.py merge <raw files...>: writes coverage-js.json, exit 1 when a file is under the limit."""
    if len(argv) < 3 or argv[1] != "merge":
        print(main.__doc__)
        return 2
    report = merge_raw(argv[2:])
    (ROOT / "coverage-js.json").write_text(json.dumps(report, indent=1))
    for name, f in report["files"].items():
        print(f"E2E coverage static/{name}: {f['covered']}/{f['lines']} lines ({f['percent']} %)")
    low = low_files(report)
    if low:
        print(f"FAIL: coverage {low} is below {JS_FAIL_UNDER} %")
        return 1
    print(f"OK: every file is at {JS_FAIL_UNDER} % or more")
    return 0


@pytest.fixture(scope="session", autouse=True)
def _js_coverage_report(request):
    yield
    if not JS_COV["covered"]:
        return
    out = os.environ.get("DRUMSPIKE_JS_COV_OUT")
    if out:                                    # CI shard: save the raw data, tests/jscov.py merge gates
        dump_raw(out)
    report = build_report(JS_COV["covered"], JS_COV["src"], JS_COV["called"])
    files = report["files"]
    (ROOT / "coverage-js.json").write_text(json.dumps(report, indent=1))
    JS_COV["report"] = report
    tr = request.config.pluginmanager.get_plugin("terminalreporter")
    if tr:
        for name, f in files.items():
            tr.write_line(f"\nE2E coverage static/{name}: {f['covered']}/{f['lines']} lines ({f['percent']} %); "
                          f"{len(f['never_called'])} named functions never called (see coverage-js.json)")


def pytest_sessionfinish(session, exitstatus):
    """Fail the run when the coverage of a file drops below JS_FAIL_UNDER (only when all e2e_app tests ran;
    not in a shard run: the CI job js-coverage gates the merged data)."""
    rep = JS_COV.get("report")
    if rep and not os.environ.get("DRUMSPIKE_E2E_SHARD") and session.config.option.keyword == "" \
            and session.config.option.markexpr in ("", "e2e_app"):
        low = low_files(rep)
        if low:
            tr = session.config.pluginmanager.get_plugin("terminalreporter")
            if tr:
                tr.write_line(f"FAIL: coverage {low} is below {JS_FAIL_UNDER} %", red=True)
            session.exitstatus = pytest.ExitCode.TESTS_FAILED


if __name__ == "__main__":
    import sys
    sys.exit(main(sys.argv))
