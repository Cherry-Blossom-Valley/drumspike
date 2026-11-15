#!/usr/bin/env python3
"""The M1a gate of ticket #525 (check.sh check -> this script). One stage per gate line.

Stages (`all` runs every one; each prints ok/FAIL with its evidence):

  fixture        the synthetic session: four tracks with bleed between them (kick, snare,
                 tom, hat; names that give the roles), written to out/fixtures; the wall
                 clock and the file hashes of the repo are taken here (stage files)
  reference      the event list of `service_tool analyze` on a project with the same audio
                 and the same roles (tracks_add from the fixture names), as raw JSON bytes;
                 the stage states the event count and fails on 0
  identity-clap  the plugin (the .clap bundle) analyses the four-track session in the ARA
                 test host; the events are byte for byte the reference's, and the notes of
                 every source sit at the exact samples of the events
  identity-vst3  the plugin inside the wrapped VST3 (Contents/PlugIns) is byte-identical
                 to the standalone CLAP and analyses the same session with the same result
  regions        one source, two playback regions at different timeline positions, one of
                 them trimmed; the plugin makes the track from segments; the events are
                 byte for byte the service's on a project with the same segments, and the
                 hits of the gap and of the trimmed-away part are absent
  archive        analyse, store the ARA archive, restore it in a NEW process: the event
                 lists are byte-identical, the decode counter of the service stays at 0,
                 and a stored archive fixture from the repo loads (with its version field)
  threads        audio blocks during the analysis have no overrun; with
                 DS_ARA_FAULT=in-process the analysis runs inside `process` and the host
                 reports the overrun (the check can fail)
  conformance    the independent host: the ARATestHost of the ARA SDK examples (built by
                 check.sh from the S1 pattern) runs its own tests on the plugin, on the
                 CLAP and on the wrapped VST3 - ContentReading, PropertyUpdates,
                 Archiving, PlaybackRendering, EditorView - each must exit 0 (the review
                 note on #525; the plugin has no editor GUI, but it implements the
                 EditorView role interface and its playback renderer passes audio
                 through, so no test is skipped)
  files          the plugin wrote only below its cache folder: no file in the repo changed
                 since the fixture stage and no fixture audio file changed
  pluginval      pluginval 1.0.4 at strictness 5 on the wrapped VST3 (no CLAP_PATH: the
                 wrapper must find the CLAP inside its own bundle)
  validate       clap-validator 0.4.1 on the CLAP bundle
  universal      lipo -info names arm64 and x86_64 for the CLAP and the VST3 binary
  standalone     cargo metadata --no-deps at the repo root names no plugin crate; cargo
                 tree -p drumspike-desktop names no plugin crate and no plugin SDK crate;
                 the diff touches only plugins/ara/ and docs/design/daw-plugin/
  crate          cargo fmt --all -- --check, cargo clippy --all-targets -- -D warnings,
                 cargo test in plugins/ara
  workspace      the same three at the repo root, without the ARA SDK (ARA_API_DIR unset)

Paths: the built parts live in .deps/ (see check.sh); the workspace release binaries come
from $DS_SPIKE_TOOLS (default ../../target/release); the plugin builds into its own
./target. The cache folder of the plugin is overridden with DS_ARA_CACHE into out/ per run.
"""

from __future__ import annotations

import hashlib
import json
import math
import os
import shutil
import struct
import subprocess
import sys
import time
import wave
from pathlib import Path

HERE = Path(__file__).resolve().parent
DEPS = Path(os.environ.get("DS_ARA_DEPS", HERE / ".deps"))
OUT = HERE / "out"
ROOT = HERE.parent.parent
CLT = subprocess.run(["xcode-select", "-p"], capture_output=True, text=True).stdout.strip()
WS_TARGET = Path(os.environ.get("WS_CARGO_TARGET_DIR", os.environ.get("CARGO_TARGET_DIR", ROOT / "target")))
TOOLS = Path(os.environ.get("DS_SPIKE_TOOLS", WS_TARGET / "release"))
CLAP = DEPS / "DrumSpikeAra.clap"
CLAP_BIN = CLAP / "Contents" / "MacOS" / "DrumSpikeAra"
VST3 = DEPS / "build-wrapper" / "DrumSpikeAra.vst3"
VST3_BIN = VST3 / "Contents" / "MacOS" / "DrumSpikeAra"
EMBEDDED_CLAP = VST3 / "Contents" / "PlugIns" / "DrumSpikeAra.clap"
HOST = HERE / "target" / "release" / "ara-host"
SERVICE_TOOL = TOOLS / "service_tool"
PLUGINVAL = DEPS / "pluginval.app" / "Contents" / "MacOS" / "pluginval"
CLAP_VALIDATOR = DEPS / "clap-validator-src" / "target" / "release" / "clap-validator"
TESTHOST = Path(os.environ.get("DS_ARA_TESTHOST", DEPS / "build-examples" / "bin" / "Debug" / "ARATestHost"))
FIXTURES = OUT / "fixtures"
ARCHIVE_FIXTURE = HERE / "fixtures" / "archive-v1.ara"

FAILURES: list[str] = []
T0: float = 0.0

# The synthetic session: name, rate, channels, seconds, hits (t, Hz, amp), bleed (t, Hz, amp)
# from another track. The bleed makes every track carry content of a neighbour.
SPEC = [
    ("m1a_kick_48k_mono.wav", 48000, 1, 3.0,
     [(0.10, 60.0, 0.9), (0.75, 60.0, 0.9), (1.42, 60.0, 0.85), (2.30, 60.0, 0.9)],
     [(0.30, 200.0, 0.12), (1.00, 200.0, 0.10), (1.80, 200.0, 0.12)]),
    ("m1a_snare_48k_stereo.wav", 48000, 2, 2.0,
     [(0.20, 200.0, 0.85), (0.85, 200.0, 0.9), (1.65, 200.0, 0.8)],
     [(0.55, 60.0, 0.12), (1.30, 60.0, 0.10)]),
    ("m1a_tom_96k_mono.wav", 96000, 1, 3.0,
     [(0.40, 120.0, 0.9), (0.90, 120.0, 0.85), (1.50, 120.0, 0.9), (2.00, 120.0, 0.85), (2.60, 120.0, 0.9)],
     [(0.70, 200.0, 0.12), (1.75, 200.0, 0.10)]),
    ("m1a_hat_48k_mono.wav", 48000, 1, 1.6,
     [(0.15, 8000.0, 0.5), (0.45, 8000.0, 0.5), (0.95, 8000.0, 0.45), (1.30, 8000.0, 0.5)],
     [(1.05, 120.0, 0.10)]),
]
# The regions fixture (gate: one source, two playback regions at different timeline
# positions, one of them trimmed): the tom, regions [0.0, 1.2) and [1.8, 2.3).
REGIONS_FILE = "m1a_tom_96k_mono.wav"
REGIONS = [(0.0, 1.2, 0.0), (1.8, 0.5, 2.5)]
# Hits that must be absent: 1.5 s is in the gap between the regions, 2.6 s behind both.
ABSENT = [1.5, 2.6]
FILES = [name for name, *_ in SPEC]


def run(cmd: list, env: dict | None = None, log: str | None = None, cwd: Path | None = None) -> tuple[int, str]:
    """One command with the env overlay (None removes the key), captured to out/<log>."""
    e = dict(os.environ)
    if env:
        for k, v in env.items():
            if v is None:
                e.pop(k, None)
            else:
                e[k] = v
    p = subprocess.run(cmd, capture_output=True, text=True, env=e, timeout=1800, cwd=cwd)
    text = (p.stdout or "") + (p.stderr or "")
    if log:
        (OUT / log).write_text(text, encoding="utf-8")
    return p.returncode, text


def pass_fail(stage: str, ok: bool, evidence: str) -> bool:
    print(f"{'ok  ' if ok else 'FAIL'} {stage}: {evidence}")
    if not ok:
        FAILURES.append(stage)
    return ok


def safe_name(stem: str) -> str:
    """The app's track name of a file stem (crates/app/src/tracks.rs safe_name)."""
    out, gap = "", False
    for c in stem:
        if c.isascii() and (c.isalnum() or c == "-"):
            if gap and out:
                out += "_"
            gap = False
            out += c
        else:
            gap = True
    return out or "track0"


def stem_of(path: str) -> str:
    return Path(path).stem


def no_switch() -> dict:
    return {k: None for k in ("DS_ARA_CACHE", "DS_ARA_OUT", "DS_ARA_FAULT")}


# ---------------------------------------------------------------- fixture


def make_wav(path: Path, rate: int, channels: int, secs: float, hits: list, bleed: list) -> None:
    """Deterministic drum figure: decaying sine bursts on a low noise floor, plus quiet
    copies of a neighbour's hits (the bleed)."""
    n = int(rate * secs)
    data = [[0.0] * n for _ in range(channels)]
    lcg = 12345
    for i in range(n):
        t = i / rate
        s = 0.0
        for h, f, a in hits + bleed:
            dt = t - h
            if 0.0 <= dt < 0.25:
                s += math.sin(2 * math.pi * f * dt) * math.exp(-dt * 30.0) * a
        lcg = (lcg * 1103515245 + 12345) & 0x7FFFFFFF
        s += (lcg / 0x7FFFFFFF - 0.5) * 0.003
        v = int(max(-1.0, min(1.0, s)) * 32000)
        for c in range(channels):
            data[c][i] = v
    frames = bytearray()
    for i in range(n):
        for c in range(channels):
            frames += struct.pack("<h", data[c][i])
    with wave.open(str(path), "wb") as w:
        w.setnchannels(channels)
        w.setsampwidth(2)
        w.setframerate(rate)
        w.writeframes(bytes(frames))


def stage_fixture() -> bool:
    global T0
    FIXTURES.mkdir(parents=True, exist_ok=True)
    for name, rate, ch, secs, hits, bleed in SPEC:
        make_wav(FIXTURES / name, rate, ch, secs, hits, bleed)
    T0 = time.time()
    (OUT / "t0").write_text(str(T0), encoding="utf-8")
    snapshot().write_text(json.dumps(walk_tree()), encoding="utf-8")
    n = sum(len(h) for _, _, _, _, h, _ in SPEC)
    return pass_fail(
        "fixture",
        all((FIXTURES / name).stat().st_size > 44 for name, *_ in SPEC),
        f"{len(SPEC)} tracks with bleed between them ({n} lead hits), out/fixtures/",
    )


def tree_roots() -> list[Path]:
    return [ROOT]


SKIP_DIRS = {".git", "target", ".deps", "out", ".venv", "node_modules", "__pycache__", ".hermes"}


def walk_tree() -> dict:
    """path -> (size, mtime) for every file of the repo outside the build dirs."""
    seen = {}
    for root in tree_roots():
        for dirpath, dirnames, filenames in os.walk(root):
            dirnames[:] = [d for d in dirnames if d not in SKIP_DIRS]
            for f in filenames:
                p = Path(dirpath) / f
                try:
                    st = p.stat()
                    seen[str(p)] = (st.st_size, st.st_mtime)
                except OSError:
                    pass
    return seen


def snapshot() -> Path:
    return OUT / "tree-snapshot.json"


def stage_files() -> bool:
    ok = True
    before = json.loads(snapshot().read_text())
    after = walk_tree()
    changed = [p for p in after if p not in before or before[p] != after[p]]
    ok = pass_fail(
        "files-repo",
        not changed,
        f"no file of the repo changed during the runs ({len(changed)} changed)"
        + (f": {changed[:5]}" if changed else ""),
    ) and ok
    for name, *_ in SPEC:
        p = FIXTURES / name
        h = hashlib.sha256(p.read_bytes()).hexdigest()
        st = before.get(str(p))
        ok = pass_fail("files-fixtures", st is not None and h == st[0], f"{name}: sha256 unchanged") and ok
    return ok


# ---------------------------------------------------------------- the service reference


def service_tool_events(project: Path, label: str) -> tuple[bool, bytes, int]:
    """`service_tool analyze <project>`: the events object as raw bytes (the last key of
    the answer line), with the event count. Fails on 0."""
    rc, text = run([str(SERVICE_TOOL), "analyze", str(project)], env=no_switch(), log=f"service_tool-{label}.log")
    line = [ln for ln in text.strip().splitlines() if ln.startswith("{")]
    if rc != 0 or not line:
        return pass_fail(f"service-{label}", False, f"service_tool analyze rc={rc}"), b"", 0
    raw = line[-1]
    idx = raw.find('"events":')
    if idx < 0 or not raw.endswith("}"):
        return pass_fail(f"service-{label}", False, 'the answer line has no trailing "events" object'), b"", 0
    events = raw[idx + len('"events":') : -1].encode()
    try:
        parsed = json.loads(events)
        tracks = len(parsed) if isinstance(parsed, dict) else 1
        n = sum(len(v) for v in parsed.values()) if isinstance(parsed, dict) else len(parsed)
    except (ValueError, TypeError):
        return pass_fail(f"service-{label}", False, "the events object is not JSON"), b"", 0
    ok = pass_fail(
        f"service-{label}", n > 0, f"{n} event(s) on {tracks} track(s), {len(events)} bytes"
    )
    return ok, events, n


def build_reference_project() -> bool:
    """The reference project: the same audio, the same roles (tracks_add guesses them from
    the fixture names, as the plugin does)."""
    script = OUT / "run-reference.json"
    script.write_text(
        json.dumps(
            [
                {"cmd": "project_new", "args": {"name": "m1a-ref", "dir": str(OUT)}},
                {"cmd": "tracks_add", "args": {"sid": "$sid", "paths": [str(FIXTURES / f) for f in FILES], "copy": True}},
                {"cmd": "project_save", "args": {"sid": "$sid", "force": True}},
            ]
        ),
        encoding="utf-8",
    )
    rc, text = run([str(SERVICE_TOOL), "run", str(script)], env=no_switch(), log="reference-build.log")
    ok = rc == 0 and (OUT / "m1a-ref").is_dir()
    return pass_fail("reference-project", ok, f"project at out/m1a-ref with the copied audio (service_tool run rc={rc})")


# ---------------------------------------------------------------- the host runs


def host_run(label: str, bundle: Path, args: list, cache: str, fault: bool = False) -> tuple[int, str, list]:
    """One ara-host run with a fresh cache and report file. Returns (rc, text, json lines)."""
    out_file = OUT / f"report-{label}.jsonl"
    if out_file.exists():
        out_file.unlink()
    cache_dir = OUT / cache
    if cache_dir.exists():
        shutil.rmtree(cache_dir)
    env = {**no_switch(), "DS_ARA_CACHE": str(cache_dir), "DS_ARA_OUT": str(out_file)}
    if fault:
        env["DS_ARA_FAULT"] = "in-process"
    rc, text = run([str(HOST), str(bundle), *args], env=env, log=f"host-{label}.log")
    lines = []
    for ln in text.strip().splitlines():
        ln = ln.strip()
        if ln.startswith("{"):
            try:
                lines.append(json.loads(ln))
            except ValueError:
                pass
    return rc, text, lines


def plugin_service_row(label: str) -> tuple[bool, bytes]:
    """The last service row of the plugin's report: the events object as raw bytes."""
    path = OUT / f"report-{label}.jsonl"
    if not path.exists():
        return pass_fail(f"plugin-{label}", False, f"no plugin report at {path.name}"), b""
    rows = [json.loads(ln) for ln in path.read_text().splitlines() if ln.strip()]
    rows = [r for r in rows if r.get("event") == "service"]
    if not rows:
        return pass_fail(f"plugin-{label}", False, "the plugin wrote no service row"), b""
    raw = path.read_text().splitlines()
    line = [ln for ln in raw if '"event":"service"' in ln][-1]
    idx = line.find('"events":')
    if idx < 0 or not line.rstrip().endswith("}"):
        return pass_fail(f"plugin-{label}", False, 'the service row has no trailing "events" object'), b""
    events = line.rstrip()[idx + len('"events":') : -1].encode()
    row = rows[-1]
    counts = row.get("decode_counts", {})
    print(f"    plugin-{label}: analysis {row.get('analysis_ms')} ms, decodes {counts or '{}'}")
    return True, events


def compare_events(label: str, got: bytes, want: bytes) -> bool:
    if got == want:
        return pass_fail(f"compare-{label}", True, f"byte for byte the reference events ({len(got)} bytes)")
    note = "other bytes"
    try:
        if json.loads(got) == json.loads(want):
            note = "same JSON, other bytes"
    except ValueError:
        note = "not JSON"
    (OUT / f"compare-{label}.diff").write_text(
        f"plugin: {got.decode(errors='replace')}\n\nreference: {want.decode(errors='replace')}\n", encoding="utf-8"
    )
    return pass_fail(f"compare-{label}", False, f"{len(got)} bytes against {len(want)}: {note} (out/compare-{label}.diff)")


def check_notes(label: str, lines: list, events: bytes, rate_of: dict) -> bool:
    """Gate item 5: the content reader of each source gives one note per event, at the
    exact sample of the event (t * rate of the source)."""
    try:
        ref = json.loads(events)
    except ValueError:
        return pass_fail(f"notes-{label}", False, "the reference events are not JSON")
    notes_rows = [r for r in lines if r.get("event") == "host_notes"]
    if not notes_rows:
        return pass_fail(f"notes-{label}", False, "the host printed no host_notes rows")
    ok, total = True, 0
    for row in notes_rows:
        name = row.get("name", "")
        track = safe_name(stem_of(name))
        want = ref.get(track)
        if want is None:
            ok = pass_fail(f"notes-{label}", False, f"no reference events for track {track} ({name})") and ok
            continue
        rate = rate_of.get(name) or rate_of.get(stem_of(name)) or 0
        want_samples = [round(ev["t"] * rate) for ev in want]
        got_samples = [round(s) for s in row.get("samples", [])]
        total += len(got_samples)
        if row.get("count") != len(want) or got_samples != want_samples:
            ok = pass_fail(
                f"notes-{label}",
                False,
                f"{track}: {row.get('count')} notes {got_samples[:6]} against {len(want)} events {want_samples[:6]}",
            ) and ok
            continue
        print(f"    notes-{label}: {track}: {len(want)} notes at the exact samples")
    if total == 0:
        return pass_fail(f"notes-{label}", False, "0 notes read through the content readers") and ok
    return ok


def rates_of() -> dict:
    """name and stem -> sample rate, from the fixtures (the host reads the same files)."""
    out = {}
    for name, rate, ch, *_ in SPEC:
        out[str(FIXTURES / name)] = rate
        out[name] = rate
        out[stem_of(name)] = rate
    return out


def stage_identity(label: str, bundle: Path) -> bool:
    """Gate items 2 and 5 on one bundle: the four-track session, byte compare, notes."""
    if not HOST.exists() or not bundle.exists():
        return pass_fail(f"identity-{label}", False, f"no host or no bundle at {bundle}")
    rc, text, lines = host_run(f"identity-{label}", bundle, ["session", "--files", *FILES], f"cache-identity-{label}")
    if rc != 0:
        return pass_fail(f"identity-{label}", False, f"ara-host session rc={rc} (out/host-identity-{label}.log)")
    ok, events = plugin_service_row(f"identity-{label}")
    if not ok:
        return False
    want = (OUT / "events-reference.json").read_bytes()
    n = sum(len(v) for v in json.loads(want).values())
    ok = pass_fail(f"identity-{label}", True, f"{n} events on {len(json.loads(want))} tracks") and ok
    ok = compare_events(f"{label}-session", events, want) and ok
    ok = check_notes(f"{label}-session", lines, want, rates_of()) and ok
    return ok


def stage_regions() -> bool:
    """Gate item 3: one source, two regions (one trimmed); the track is made from
    segments; the events are the service's on a project with the same segments."""
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("regions", False, "no host or no CLAP: run check.sh build first")
    args = ["regions", "--file", str(FIXTURES / REGIONS_FILE)]
    for a, d, p in REGIONS:
        args += ["--region", f"{a},{d},{p}"]
    rc, text, lines = host_run("regions", CLAP, args, "cache-regions")
    if rc != 0:
        return pass_fail("regions", False, f"ara-host regions rc={rc} (out/host-regions.log)")
    ok, events = plugin_service_row("regions")
    if not ok:
        return False

    # the reference: a copy of the plugin's own document (the segments the plugin made
    # from the ARA regions) with the source records pointed back at the fixture file
    doc_path = OUT / "cache-regions" / "session" / "session.drumspike"
    if not doc_path.exists():
        return pass_fail("regions", False, f"the plugin wrote no document at {doc_path}")
    doc = json.loads(doc_path.read_text())
    fixture = FIXTURES / REGIONS_FILE
    st = fixture.stat()
    rec = {
        "mode": "referenced",
        "path": str(fixture),
        "relative": os.path.relpath(fixture, OUT / "ref-regions").replace(os.sep, "/"),
        "size": st.st_size,
        "mtime": int(st.st_mtime),
        "head_sha1": hashlib.sha1(fixture.read_bytes()[: 1 << 20]).hexdigest(),
    }
    ref_dir = OUT / "ref-regions"
    ref_dir.mkdir(parents=True, exist_ok=True)
    for track in (doc.get("tracks") or {}).values():
        for seg in track.get("segments") or []:
            seg["source"] = rec
    (ref_dir / "ref-regions.drumspike").write_text(json.dumps(doc), encoding="utf-8")
    ok2, ref_events, n = service_tool_events(ref_dir, "regions")
    if not ok2:
        return False
    ok = compare_events("regions", events, ref_events) and ok

    # the trim: the hits of the gap and of the part behind the regions are absent
    listed = json.loads(ref_events)
    track_events = next(iter(listed.values()))
    bad = [t for t in ABSENT if any(abs(ev["t"] - t) < 0.05 for ev in track_events)]
    ok = pass_fail(
        "regions-trim",
        not bad,
        f"no event at the gap hit (1.5 s) or behind the regions (2.6 s); {len(track_events)} events on the covered slices"
        if not bad
        else f"events at {bad} exist but their audio is trimmed away",
    )
    ok = check_notes("regions", lines, ref_events, rates_of()) and ok
    return ok


def stage_archive() -> bool:
    """Gate item 4: store, restore in a new process, the decode counter stays at 0, and a
    stored archive fixture from the repo loads."""
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("archive", False, "no host or no CLAP: run check.sh build first")
    archive = OUT / "session.araarchive"
    rc, text, lines = host_run(
        "archive-store", CLAP, ["store", "--files", *FILES, "--archive", str(archive)], "cache-archive-store"
    )
    if rc != 0 or not archive.exists():
        return pass_fail("archive-store", False, f"ara-host store rc={rc} (out/host-archive-store.log)")
    ok_store, events_store = plugin_service_row("archive-store")
    if not ok_store:
        return False
    want = (OUT / "events-reference.json").read_bytes()
    ok = compare_events("archive-store", events_store, want) and ok

    # the restore in a NEW process: no analysis, no decode, the notes from the blob
    rc, text, lines = host_run(
        "archive-restore", CLAP, ["restore", "--files", *FILES, "--archive", str(archive)], "cache-archive-restore"
    )
    if rc != 0:
        return pass_fail("archive-restore", False, f"ara-host restore rc={rc} (out/host-archive-restore.log)")
    ok_r, events_restore = plugin_service_row("archive-restore")
    if not ok_r:
        return False
    row = [json.loads(ln) for ln in (OUT / "report-archive-restore.jsonl").read_text().splitlines() if '"event":"service"' in ln][-1]
    counts = row.get("decode_counts", {"_": 1})
    ok = pass_fail(
        "archive-decodes",
        counts == {},
        f"the decode counter of the restore stays at 0: {json.dumps(row.get('decode_counts'))}",
    )
    ok = compare_events("archive-restore", events_restore, events_store) and ok
    ok = check_notes("archive-restore", lines, events_store, rates_of()) and ok

    # the stored archive fixture of the repo loads, and it carries the version field
    if not ARCHIVE_FIXTURE.exists():
        return pass_fail(
            "archive-fixture", False, f"no stored archive fixture at {ARCHIVE_FIXTURE} (check.py make-fixture)"
        )
    blob = json.loads(ARCHIVE_FIXTURE.read_text())
    ok = pass_fail(
        "archive-fixture-version",
        blob.get("version") == 1,
        f"the fixture carries version {blob.get('version')}",
    )
    rc, text, lines = host_run(
        "archive-fixture", CLAP, ["restore", "--files", *FILES, "--archive", str(ARCHIVE_FIXTURE)], "cache-archive-fixture"
    )
    if rc != 0:
        return pass_fail("archive-fixture", False, f"the fixture restore rc={rc} (out/host-archive-fixture.log)")
    ok_f, events_fixture = plugin_service_row("archive-fixture")
    if not ok_f:
        return False
    ok = compare_events("archive-fixture", events_fixture, want) and ok
    return ok


def stage_threads() -> bool:
    """Gate item 6: audio blocks during the analysis have no overrun; the fault run (the
    analysis inside `process`) makes the check fail."""
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("threads", False, "no host or no CLAP: run check.sh build first")
    rc, text, lines = host_run("blocks", CLAP, ["blocks", "--files", *FILES, "--frames", "512"], "cache-blocks")
    report = next((r for r in reversed(lines) if r.get("event") == "blocks"), {})
    ok = pass_fail(
        "threads",
        rc == 0 and report.get("overruns") == 0 and report.get("event_file") is True,
        f"rc={rc}, {report.get('blocks')} blocks while the analysis ran, overruns={report.get('overruns')}, "
        f"worst {report.get('worst_block_us')}us of {report.get('budget_us')}us per block",
    )
    rc, text, lines = host_run(
        "blocks-fault", CLAP, ["blocks", "--files", *FILES, "--frames", "512"], "cache-blocks-fault", fault=True
    )
    report = next((r for r in reversed(lines) if r.get("event") == "blocks"), {})
    ok = pass_fail(
        "threads-fault",
        rc != 0 and report.get("overruns", 0) >= 1,
        f"the fault switch ran the analysis inside process: rc={rc}, {report.get('overruns')} block(s) over budget "
        f"(worst {report.get('worst_block_us')}us), event_file={report.get('event_file')}",
    ) and ok
    return ok


def stage_conformance() -> bool:
    """The independent ARA conformance run (review note on #525): the ARATestHost of the
    ARA SDK examples - a second author, built by check.sh from the S1 pattern - runs its
    own tests on this plugin, on the CLAP and on the wrapped VST3. Each test must exit 0.
    The gate's own ara-host stays the host of the identity, regions, archive, notes and
    threads stages (gate lines 2 to 7); this run sits beside it. No test is skipped: the
    plugin implements the EditorView role interface (no editor GUI, the handlers are
    no-ops) and its playback renderer passes audio through."""
    if not TESTHOST.exists():
        return pass_fail("conformance", False, f"no ARATestHost at {TESTHOST} (run check.sh build first)")
    files_of = {
        "ContentReading": FILES,
        "PropertyUpdates": ["m1a_kick_48k_mono.wav"],
        "Archiving": ["m1a_kick_48k_mono.wav"],
        "PlaybackRendering": ["m1a_kick_48k_mono.wav", "m1a_hat_48k_mono.wav"],
        "EditorView": ["m1a_kick_48k_mono.wav"],
    }
    ok = True
    for kind, bundle in (("clap", CLAP), ("vst3", VST3)):
        if not bundle.exists():
            return pass_fail("conformance", False, f"no {kind} bundle at {bundle} (run check.sh build first)")
        for test, names in files_of.items():
            # no CLAP_PATH: the wrapped VST3 must find the CLAP inside its own bundle, and
            # the runs must not reach another plugin by accident; stray writes go to out/
            env = {**no_switch(), "CLAP_PATH": None}
            rc, _text = run(
                [str(TESTHOST), f"-{kind}", str(bundle), "-file", *[str(FIXTURES / name) for name in names], "-test", test],
                env=env,
                log=f"conformance-{kind}-{test}.log",
                cwd=OUT,
            )
            ok = pass_fail(
                f"conformance-{kind}-{test}",
                rc == 0,
                f"ARATestHost {test} on the {kind}: rc={rc} (out/conformance-{kind}-{test}.log)",
            ) and ok
    return ok


# ---------------------------------------------------------------- validators


def stage_pluginval() -> bool:
    if not PLUGINVAL.exists() or not VST3.exists():
        return pass_fail("pluginval", False, f"no pluginval or no VST3 ({PLUGINVAL}, {VST3})")
    out_file = OUT / "report-pluginval.jsonl"
    if out_file.exists():
        out_file.unlink()
    env = {**no_switch(), "DS_ARA_CACHE": str(OUT / "cache-pluginval"), "DS_ARA_OUT": str(out_file)}
    rc, text = run(
        [str(PLUGINVAL), "--strictness-level", "5", "--timeout-ms", "120000", str(VST3)],
        env=env,
        log="pluginval.log",
    )
    ok = pass_fail("pluginval", rc == 0, f"strictness 5 on the wrapped VST3, rc={rc} (out/pluginval.log)")
    if not ok:
        return False
    if not out_file.exists():
        return pass_fail("pluginval", False, "pluginval passed but the plugin wrote no report (is the CLAP inside?)")
    ok_ev, events = plugin_service_row("pluginval")
    if not ok_ev:
        return pass_fail("pluginval-events", False, "pluginval ran the plugin but no analysis was requested")
    want = (OUT / "events-reference.json").read_bytes()
    return compare_events("pluginval", events, want)


def stage_validate() -> bool:
    if not CLAP_VALIDATOR.exists() or not CLAP.exists():
        return pass_fail("validate", False, f"no clap-validator or no CLAP ({CLAP_VALIDATOR}, {CLAP})")
    rc, _ = run([str(CLAP_VALIDATOR), "validate", str(CLAP)], env=no_switch(), log="clap-validator.log")
    return pass_fail("validate", rc == 0, f"clap-validator on the CLAP bundle, rc={rc} (out/clap-validator.log)")


def stage_universal() -> bool:
    ok = True
    for name, binary in (("CLAP", CLAP_BIN), ("VST3", VST3_BIN)):
        if not binary.exists():
            return pass_fail("universal", False, f"no {name} binary at {binary}")
        rc, text = run(["lipo", "-info", str(binary)])
        names = text.strip().split(":")[-1].strip()
        ok = pass_fail("universal", rc == 0 and "arm64" in names and "x86_64" in names, f"lipo -info {name}: {names}") and ok
    return ok


def stage_standalone() -> bool:
    ok = True
    rc, text = run(["cargo", "tree", "-p", "drumspike-desktop"], log="tree-desktop.log", cwd=ROOT)
    bad = [ln for ln in text.splitlines() if "drumspike-ara" in ln or "clap-sys" in ln]
    ok = (
        pass_fail(
            "standalone-tree",
            ok and rc == 0 and not bad,
            f"cargo tree -p drumspike-desktop names neither the plugin crate nor a plugin SDK crate ({len(bad)} hits)",
        )
        and ok
    )
    rc, text = run(["cargo", "metadata", "--no-deps", "--format-version", "1"], log="metadata.log", cwd=ROOT)
    ok = (
        pass_fail(
            "standalone-metadata",
            ok and rc == 0 and "drumspike-ara" not in text,
            "cargo metadata --no-deps at the repo root does not name the plugin crate",
        )
        and ok
    )
    rc, text = run(["git", "-C", str(ROOT), "diff", "--name-only", "origin/main...HEAD"])
    touched = [ln for ln in text.splitlines() if ln.strip() and not ln.startswith(("plugins/ara/", "docs/design/daw-plugin/"))]
    ok = (
        pass_fail(
            "standalone-diff",
            ok and rc == 0 and not touched,
            f"the diff touches only plugins/ara/ and docs/design/daw-plugin/ ({len(touched)} files outside)",
        )
        and ok
    )
    return ok


# ---------------------------------------------------------------- the cargo trios


def stage_crate() -> bool:
    env = {"ARA_API_DIR": str(DEPS / "ARA_SDK" / "ARA_API"), "LIBCLANG_PATH": f"{CLT}/usr/lib"}
    steps = [
        ("fmt", ["cargo", "fmt", "--all", "--", "--check"]),
        ("clippy", ["cargo", "clippy", "--all-targets", "--locked", "--", "-D", "warnings"]),
        ("test", ["cargo", "test", "--locked"]),
    ]
    ok = True
    for name, cmd in steps:
        rc, text = run(cmd, env=env, log=f"crate-{name}.log", cwd=HERE)
        ok = pass_fail(f"crate-{name}", rc == 0, f"{name} of the plugin crate, rc={rc} (out/crate-{name}.log)") and ok
    return ok


def stage_workspace() -> bool:
    # without the ARA SDK: ARA_API_DIR must not be set (the workspace builds without it)
    env = {"ARA_API_DIR": None, "CARGO_TARGET_DIR": str(WS_TARGET)}
    steps = [
        ("fmt", ["cargo", "fmt", "--all", "--", "--check"]),
        ("clippy", ["cargo", "clippy", "--all-targets", "--locked", "--", "-D", "warnings"]),
        ("test", ["cargo", "test", "--workspace", "--locked", "--no-fail-fast"]),
    ]
    ok = True
    for name, cmd in steps:
        rc, text = run(cmd, env=env, log=f"workspace-{name}.log", cwd=ROOT)
        ok = pass_fail(f"workspace-{name}", rc == 0, f"workspace {name} without the ARA SDK, rc={rc} (out/workspace-{name}.log)") and ok
    return ok


# ---------------------------------------------------------------- fixture archive


def stage_make_fixture() -> bool:
    """One store run whose archive becomes the committed fixture (check.py make-fixture)."""
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("make-fixture", False, "no host or no CLAP: run check.sh build first")
    archive = OUT / "fixture-store.araarchive"
    rc, text, lines = host_run(
        "fixture-store", CLAP, ["store", "--files", *FILES, "--archive", str(archive)], "cache-fixture-store"
    )
    if rc != 0 or not archive.exists():
        return pass_fail("make-fixture", False, f"ara-host store rc={rc} (out/host-fixture-store.log)")
    blob = json.loads(archive.read_text())
    blob["note"] = "Stored by plugins/ara check.py make-fixture; regenerate when the fixtures change."
    ARCHIVE_FIXTURE.parent.mkdir(parents=True, exist_ok=True)
    ARCHIVE_FIXTURE.write_text(json.dumps(blob), encoding="utf-8")
    return pass_fail(
        "make-fixture",
        blob.get("version") == 1 and len(blob.get("sources", [])) == len(FILES),
        f"{ARCHIVE_FIXTURE} written (version {blob.get('version')}, {len(blob.get('sources', []))} sources)",
    )


def stage_reference() -> bool:
    if not build_reference_project():
        return False
    ok, events, n = service_tool_events(OUT / "m1a-ref", "reference")
    if not ok:
        return False
    (OUT / "events-reference.json").write_bytes(events)
    return True


STAGES = {
    "fixture": stage_fixture,
    "reference": stage_reference,
    "identity-clap": lambda: stage_identity("clap", CLAP),
    "identity-vst3": lambda: stage_identity("vst3", EMBEDDED_CLAP),
    "regions": stage_regions,
    "archive": stage_archive,
    "threads": stage_threads,
    "conformance": stage_conformance,
    "files": stage_files,
    "pluginval": stage_pluginval,
    "validate": stage_validate,
    "universal": stage_universal,
    "standalone": stage_standalone,
    "crate": stage_crate,
    "workspace": stage_workspace,
    "make-fixture": stage_make_fixture,
}
ORDER = [
    "fixture", "reference", "identity-clap", "identity-vst3", "regions", "archive",
    "threads", "conformance", "files", "pluginval", "validate", "universal", "standalone",
    "crate", "workspace",
]


def main() -> int:
    stages = sys.argv[1:] or ["all"]
    if stages == ["all"]:
        stages = ORDER
    OUT.mkdir(parents=True, exist_ok=True)
    global T0
    if (OUT / "t0").exists() and "fixture" not in stages:
        T0 = float((OUT / "t0").read_text())
    for s in stages:
        if s == "reference":
            stage_reference()
        elif s in STAGES:
            STAGES[s]()
        else:
            FAILURES.append(s)
            print(f"FAIL {s}: unknown stage")
    if FAILURES:
        print(f"M1A CHECK: FAIL ({', '.join(FAILURES)})")
        return 1
    print("M1A CHECK: PASS (identity, regions, archive, notes, threads, conformance, files, pluginval, clap-validator, universal, standalone, crate, workspace)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
