#!/usr/bin/env python3
"""The check of the service spike (S3, #506). One stage per gate line; `all` runs every one.

Stages (each prints PASS/FAIL and one line of evidence; `all` exits 0 when every stage passed):

  fixture     the synthetic fixture: a 3 s 48 kHz mono WAV and a project built from it
              (service_tool: project_new, tracks_add with copy, project_save)
  service     the event list of `service_tool analyze` on the fixture, as raw JSON bytes
  clap        the test host loads the CLAP: audio blocks run while the analysis runs; one
              block over its own duration is an overrun and fails; the event list the plugin
              wrote is compared byte for byte with the one of `service_tool`
  panic       the host run with a forced panic in `process`: the process lives and the guard
              catches the panic
  vst3        pluginval at strictness 5 on the wrapped VST3 (with the switch on, so the
              wrapped plugin writes its event list too; compared as in `clap`)
  validate    clap-validator on the CLAP bundle
  universal   lipo -info names arm64 and x86_64 for the CLAP and the VST3 binaries
  standalone  the desktop release build is unchanged: `cargo tree -p drumspike-desktop`
              names neither the new crate nor clap-sys, `cargo metadata --no-deps` at the
              repo root does not name the new crate, and the diff of this head changes no
              file of crates/ or apps/

Paths: the workspace release binaries come from $DS_SPIKE_TOOLS (default
../../target/release next to this script). The built parts live in .deps/ (see spike.sh).
"""

import json
import math
import os
import struct
import subprocess
import sys
import wave
from pathlib import Path
from typing import Optional

HERE = Path(__file__).resolve().parent
DEPS = HERE / ".deps"
OUT = HERE / "out"
TOOLS = Path(os.environ.get("DS_SPIKE_TOOLS", HERE / "../../target/release")).resolve()
CLAP_BUNDLE = DEPS / "DrumSpikeServiceSpike.clap"
CLAP_BIN = CLAP_BUNDLE / "Contents" / "MacOS" / "DrumSpikeServiceSpike"
VST3_BUNDLE = DEPS / "build-wrapper" / "DrumSpikeServiceSpike.vst3"
VST3_BIN = VST3_BUNDLE / "Contents" / "MacOS" / "DrumSpikeServiceSpike"
HOST = HERE / "target" / "release" / "service-spike-host"
SERVICE_TOOL = TOOLS / "service_tool"
PLUGINVAL = DEPS / "pluginval.app" / "Contents" / "MacOS" / "pluginval"
CLAP_VALIDATOR = DEPS / "clap-validator-src" / "target" / "release" / "clap-validator"
REPO = HERE.parent.parent

FAILURES: list[str] = []


def run(cmd: list[str], env: Optional[dict] = None, log: Optional[str] = None, cwd: Optional[Path] = None) -> tuple[int, str]:
    """One command, its output captured to out/<log> and returned with its exit code."""
    e = dict(os.environ)
    if env:
        for k, v in env.items():
            if v is None:
                e.pop(k, None)
            else:
                e[k] = v
    p = subprocess.run(cmd, capture_output=True, text=True, env=e, timeout=900, cwd=cwd)
    text = (p.stdout or "") + (p.stderr or "")
    if log:
        (OUT / log).write_text(text, encoding="utf-8")
    return p.returncode, text


def pass_fail(stage: str, ok: bool, evidence: str) -> bool:
    print(f"{'ok  ' if ok else 'FAIL'} {stage}: {evidence}")
    if not ok:
        FAILURES.append(stage)
    return ok


def switch(project: Path, out_file: Path, meta: Optional[Path] = None) -> dict:
    """The test switch: the environment the plugin reads in instantiate."""
    env = {"DS_SERVICE_SPIKE_PROJECT": str(project), "DS_SERVICE_SPIKE_OUT": str(out_file)}
    if meta:
        env["DS_SERVICE_SPIKE_META"] = str(meta)
    return env


def no_switch() -> dict:
    return {k: None for k in ("DS_SERVICE_SPIKE_PROJECT", "DS_SERVICE_SPIKE_OUT", "DS_SERVICE_SPIKE_META", "DS_SERVICE_SPIKE_PANIC")}


# ---------------------------------------------------------------- fixture


def make_wav(path: Path) -> None:
    """A deterministic synthetic drum figure: five decaying 200 Hz bursts on a low noise floor."""
    sr = 48000
    hits = [0.10, 0.40, 0.80, 1.30, 1.90]
    lcg = 12345
    frames = bytearray()
    for i in range(int(sr * 3.0)):
        t = i / sr
        s = 0.0
        for h in hits:
            dt = t - h
            if 0.0 <= dt < 0.25:
                s += math.sin(2 * math.pi * 200.0 * dt) * math.exp(-dt * 30.0) * 0.9
        lcg = (lcg * 1103515245 + 12345) & 0x7FFFFFFF
        s += (lcg / 0x7FFFFFFF - 0.5) * 0.003
        frames += struct.pack("<h", int(max(-1.0, min(1.0, s)) * 32000))
    with wave.open(str(path), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(sr)
        w.writeframes(bytes(frames))


def stage_fixture(events: bytes = b"") -> bool:
    proj = OUT / "fixture"
    wav = OUT / "fixture.wav"
    if proj.exists():
        subprocess.run(["rm", "-rf", str(proj)], check=True)
    wav.parent.mkdir(parents=True, exist_ok=True)
    make_wav(wav)
    script = OUT / "run-fixture.json"
    script.write_text(
        json.dumps(
            [
                {"cmd": "project_new", "args": {"name": "fixture", "dir": str(OUT)}},
                {"cmd": "tracks_add", "args": {"sid": "$sid", "paths": [str(wav)], "copy": True}},
                {"cmd": "project_save", "args": {"sid": "$sid", "force": True}},
            ]
        ),
        encoding="utf-8",
    )
    rc, text = run([str(SERVICE_TOOL), "run", str(script)], env=no_switch(), log="fixture.log")
    ok = rc == 0 and (proj / "Audio Files").is_dir()
    return pass_fail("fixture", ok, f"project at out/fixture with the copied audio (service_tool run rc={rc})")


# ---------------------------------------------------------------- service (the reference)


def stage_service() -> tuple[bool, bytes]:
    analysis = OUT / "Analysis"
    if analysis.exists():
        subprocess.run(["rm", "-rf", str(analysis)], check=True)
    rc, text = run([str(SERVICE_TOOL), "analyze", str(OUT / "fixture")], env=no_switch(), log="service_tool.log")
    line = [ln for ln in text.strip().splitlines() if ln.startswith("{")]
    if rc != 0 or not line:
        return pass_fail("service", False, f"service_tool analyze rc={rc}"), b""
    raw = line[-1]
    idx = raw.find('"events":')
    if idx < 0 or not raw.endswith("}"):
        return pass_fail("service", False, 'the answer line has no trailing "events" object'), b""
    events = raw[idx + len('"events":') : -1].encode()  # the events object is the last key
    try:
        parsed = json.loads(events)
        n_tracks = len(parsed)
    except ValueError:
        return pass_fail("service", False, "the events object is not JSON"), b""
    (OUT / "events-service_tool.json").write_bytes(events)
    return (
        pass_fail("service", True, f"{n_tracks} track(s), {len(events)} bytes, out/events-service_tool.json"),
        events,
    )


# ---------------------------------------------------------------- the host runs


def host_run(label: str, env: dict) -> tuple[bool, dict]:
    """One test-host run: its JSON report is the last stdout line."""
    for f in (env.get("DS_SERVICE_SPIKE_OUT"), env.get("DS_SERVICE_SPIKE_META")):
        if f and Path(f).exists():
            Path(f).unlink()
    rc, text = run([str(HOST), str(CLAP_BUNDLE), "--frames", "512"], env=env, log=f"host-{label}.log")
    report = {}
    for ln in reversed(text.strip().splitlines()):
        if ln.startswith("{"):
            report = json.loads(ln)
            break
    ok = rc == 0 and report.get("overruns") == 0 and report.get("event_file") is True
    evidence = f"rc={rc}, {report.get('blocks')} blocks, overruns={report.get('overruns')}, worst={report.get('worst_block_us')}us of {report.get('budget_us')}us"
    return pass_fail(f"host-{label}", ok, evidence), report


def compare_events(label: str, events: bytes, path: Path) -> bool:
    if not path.exists():
        return pass_fail(f"compare-{label}", False, f"no event list at {path.name}")
    got = path.read_bytes()
    same = got == events
    if not same:
        try:
            same = json.loads(got) == json.loads(events)
            note = "same JSON, other bytes"
        except ValueError:
            note = "not JSON"
        return pass_fail(f"compare-{label}", False, f"{len(got)} bytes against {len(events)}: {note} (out/compare-{label}.diff)")
    return pass_fail(f"compare-{label}", True, f"byte for byte the event list of service_tool ({len(got)} bytes)")


def stage_clap(events: bytes) -> bool:
    if not HOST.exists() or not CLAP_BUNDLE.exists():
        return pass_fail("host-clap", False, "no host or no CLAP bundle: run spike.sh build first")
    out = OUT / "events-plugin-clap.json"
    ok, _ = host_run("clap", {**no_switch(), **switch(OUT / "fixture", out, OUT / "meta-clap.json")})
    if not ok:
        return False
    if not compare_events("clap", events, out):
        return False
    meta = OUT / "meta-clap.json"
    t = json.loads(meta.read_text())["analysis_ms"] if meta.exists() else -1
    print(f"    the analysis inside the plugin took {t} ms")
    return True


def stage_panic(events: bytes) -> bool:
    if not HOST.exists():
        return pass_fail("host-panic", False, "no host: run spike.sh build first")
    env = {**no_switch(), "DS_SERVICE_SPIKE_PANIC": "process"}
    rc, text = run([str(HOST), str(CLAP_BUNDLE), "--blocks", "128"], env=env, log="host-panic.log")
    caught = text.count("caught panic in process")
    ok = rc == 0 and caught >= 1
    return pass_fail("host-panic", ok, f"rc={rc}, the boundary caught {caught} forced panics, the process lived")


# ---------------------------------------------------------------- validators


def stage_vst3(events: bytes) -> bool:
    if not VST3_BUNDLE.exists():
        return pass_fail("pluginval", False, "no VST3: run spike.sh build first")
    out = OUT / "events-plugin-vst3.json"
    env = {
        **no_switch(),
        "CLAP_PATH": str(DEPS),
        **switch(OUT / "fixture", out, OUT / "meta-vst3.json"),
    }
    rc, _ = run([str(PLUGINVAL), "--strictness-level", "5", "--timeout-ms", "120000", str(VST3_BUNDLE)], env=env, log="pluginval.log")
    ok = pass_fail("pluginval", rc == 0, f"strictness 5 on the wrapped VST3, rc={rc} (out/pluginval.log)")
    if not ok:
        return False
    if not out.exists():
        return pass_fail("compare-vst3", False, "pluginval passed but wrote no event list (is CLAP_PATH right?)")
    return compare_events("vst3", events, out)


def stage_validate(events: bytes) -> bool:
    if not CLAP_VALIDATOR.exists():
        return pass_fail("clap-validator", False, "no clap-validator: run spike.sh deps first")
    rc, _ = run([str(CLAP_VALIDATOR), "validate", str(CLAP_BUNDLE)], env=no_switch(), log="clap-validator.log")
    return pass_fail("clap-validator", rc == 0, f"validate on {CLAP_BUNDLE.name}, rc={rc} (out/clap-validator.log)")


# ---------------------------------------------------------------- universal and standalone


def stage_universal(events: bytes) -> bool:
    ok = True
    for name, binary in (("CLAP", CLAP_BIN), ("VST3", VST3_BIN)):
        if not binary.exists():
            return pass_fail("universal", False, f"no {name} binary")
        rc, text = run(["lipo", "-info", str(binary)])
        names = text.strip().split(":")[-1].strip()
        good = rc == 0 and "arm64" in names and "x86_64" in names
        ok = pass_fail("universal", good, f"lipo -info {name}: {names}") and ok
    return ok


def stage_standalone(events: bytes) -> bool:
    ok = True
    rc, text = run(["cargo", "tree", "-p", "drumspike-desktop"], log="tree-desktop.log", cwd=REPO)
    bad = [ln for ln in text.splitlines() if "ds-service-spike" in ln or "clap-sys" in ln]
    ok = pass_fail("standalone", ok and rc == 0 and not bad, f"cargo tree -p drumspike-desktop names neither the new crate nor clap-sys ({len(bad)} hits)") and ok
    rc, text = run(["cargo", "metadata", "--no-deps", "--format-version", "1"], log="metadata.log", cwd=REPO)
    named = "ds-service-spike" in text
    ok = pass_fail("standalone", ok and rc == 0 and not named, "cargo metadata --no-deps at the repo root does not name the new crate") and ok
    rc, text = run(["git", "-C", str(REPO), "diff", "--name-only", "origin/main...HEAD"])
    touched = [ln for ln in text.splitlines() if ln.startswith("crates/") or ln.startswith("apps/")]
    ok = pass_fail("standalone", ok and rc == 0 and not touched, f"the diff changes no file of crates/ or apps/ ({len(touched)} files)") and ok
    return ok


STAGES = {
    "fixture": stage_fixture,
    "clap": stage_clap,
    "panic": stage_panic,
    "vst3": stage_vst3,
    "validate": stage_validate,
    "universal": stage_universal,
    "standalone": stage_standalone,
}


def main() -> int:
    stages = sys.argv[1:] or ["all"]
    if stages == ["all"]:
        stages = ["fixture", "service", "clap", "panic", "vst3", "validate", "universal", "standalone"]
    OUT.mkdir(parents=True, exist_ok=True)
    events = b""
    for s in stages:
        if s == "service":
            _, events = stage_service()
        elif s in STAGES:
            STAGES[s](events)
        else:
            FAILURES.append(s)
            print(f"FAIL {s}: unknown stage")
    if FAILURES:
        print(f"SPIKE: FAIL ({', '.join(FAILURES)})")
        return 1
    print("SPIKE: PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
