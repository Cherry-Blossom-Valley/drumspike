#!/usr/bin/env python3
"""The M1a gate of ticket #525 (check.sh check -> this script). One stage per gate line.

Stages (`all` runs every one; each prints ok/FAIL with its evidence):

  fixture        the synthetic session: four tracks with bleed between them (kick, snare,
                 tom, hat; names that give the roles), written to out/fixtures; the wall
                 clock and the file hashes of the repo are taken here (stage files)
  reference      the event list of `service_tool analyze` on a project with the same audio
                 and the same roles (tracks_add from the fixture names), as raw JSON bytes;
                 the stage states the event count and fails on 0
  tables         every ARA interface table the plugin declares, walked up to its
                 structSize by a small mode of ara-host (factory, document controller,
                 playback renderer, editor renderer, editor view): every slot inside the
                 declared size must carry a function - the stage fails on the first
                 empty slot, on an assigned role without an interface, on 0 slots
                 checked, and on a slot count other than the expected 65 (3 + 54 + 2 +
                 4 + 2). The fault run (DS_ARA_TABLES_FAULT, the plugin serves one table
                 with an emptied slot) must be refused by the walk.
  identity-clap  the plugin (the .clap bundle) analyses the four-track session in the ARA
                 test host; the events are byte for byte the reference's, and the notes of
                 every source sit at the exact samples of the events
  identity-vst3  the plugin inside the wrapped VST3 (Contents/PlugIns) is byte-identical
                 to the standalone CLAP and analyses the same session with the same result
  regions        one source, two playback regions at different timeline positions, one of
                 them trimmed; the plugin makes the track from segments; the events are
                 byte for byte the service's on a project with the same segments, and the
                 hits of the gap and of the trimmed-away part are absent. The update and
                 destroy case: the properties of the SECOND region are rewritten (the
                 first keeps its times) and the second region is removed (no record is
                 left behind).
  archive        analyse, store the ARA archive, restore it in a NEW process: the event
                 lists are byte-identical, the restore decodes no file (no read row and
                 no cache WAV written by the restore run; the fault run with
                 DS_ARA_RESTORE_FAULT=decode makes that proof fail), no absolute path is
                 stored, and a stored archive fixture from the repo loads (with its
                 version field)
  doc-service    M1b1 line 1: two analysis requests in one document, the services made are
                 1 and dropped with the controller (fault: a service per job)
  doc-tracks     M1b1 line 2: four sources in two requests, four tracks in the project, the
                 events of the first two byte for byte as before the second request (fault:
                 the second request forgets the earlier tracks)
  doc-two        M1b1 line 3: two document controllers in one process, own folders, events
                 byte for byte the reference of each document alone, no file of the first
                 folder changes while the second works (fault: one shared folder)
  doc-archive    M1b1 line 4: the archive stores the document id; a restore in a new
                 process uses the same folder, opens the project with the stored events and
                 decodes nothing; an archive of M1a restores into a fresh id (fault: another
                 folder, no stored events)
  doc-destroy    M1b1 line 5: one of three sources is destroyed; the track leaves the
                 project and the plugin's source list (fault: the source stays)
  doc-home       M1b1 line 6: with HOME below out/, 0 files are written below the home
                 outside the cache folder, the service home is inside it (fault: the
                 service home below the user's home)
  threads        audio blocks during the analysis have no overrun, the block count has a
                 lower limit, and the thread of the analysis is not the thread of the
                 request call; with DS_ARA_FAULT=in-process the analysis runs inside
                 `process` on the request caller's thread and both checks fire (the
                 check can fail)
  conformance    the independent host: the ARATestHost of the ARA SDK examples (built by
                 check.sh from the S1 pattern) runs its own tests on the plugin, on the
                 CLAP and on the wrapped VST3 - ContentReading, PropertyUpdates,
                 Archiving, PlaybackRendering, EditorView - each must exit 0 within its
                 120 s limit (a hung test is a FAIL with its name, not a 5 h gate; the
                 review note on #525; the plugin has no editor GUI, but it implements
                 the EditorView role interface and its playback renderer passes audio
                 through, so no test is skipped). Every run keeps the DS_ARA_CACHE
                 override, so the plugin writes only below out/.
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
import io
import json
import math
import os
import shutil
import struct
import subprocess
import sys
import time
from pathlib import Path

# the trailing iXML chunk of the fixtures (the S1 pattern): even length, keeps the
# ARATestHost's audio loader from spinning at EOF (see make_wav)
IXML_CHUNK = b"<BWFXML><IXML_VERSION>1.5</IXML_VERSION></BWFXML> "

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
# The new times of the second region in the update and destroy case (the update must
# land on that region, not on the first of the source).
REGIONS_UPDATE = (0.0, 0.9, 1.0)
# Hits that must be absent: 1.5 s is in the gap between the regions, 2.6 s behind both.
ABSENT = [1.5, 2.6]
FILES = [name for name, *_ in SPEC]


def run(
    cmd: list, env: dict | None = None, log: str | None = None, cwd: Path | None = None, timeout: int = 1800,
) -> tuple[int, str]:
    """One command with the env overlay (None removes the key), captured to out/<log>.
    A timeout kills the child and reports rc 124 (the caller turns it into a FAIL)."""
    e = dict(os.environ)
    if env:
        for k, v in env.items():
            if v is None:
                e.pop(k, None)
            else:
                e[k] = v
    try:
        p = subprocess.run(cmd, capture_output=True, text=True, env=e, timeout=timeout, cwd=cwd)
    except subprocess.TimeoutExpired:
        text = f"timeout after {timeout}s: {' '.join(str(c) for c in cmd)}"
        if log:
            (OUT / log).write_text(text, encoding="utf-8")
        return 124, text
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


# The expected slot count of the tables walk: ARAFactory 3, document controller 54,
# playback renderer 2, editor renderer 4, editor view 2.
EXPECTED_TABLE_SLOTS = 65


def absolute_paths_in(blob) -> list[str]:
    """Every string of the stored archive, dict keys included, that looks like an
    absolute path (the archive must stay portable: it carries names, never host folders)."""
    bad: list[str] = []

    def walk(node) -> None:
        if isinstance(node, str):
            if node.startswith("/") or (len(node) > 2 and node[1] == ":" and node[2] == "\\"):
                bad.append(node)
        elif isinstance(node, dict):
            # the keys too: the events object is keyed by track name
            for k, v in node.items():
                walk(k)
                walk(v)
        elif isinstance(node, list):
            for v in node:
                walk(v)

    walk(blob)
    return bad


def no_absolute_paths(label: str, blob) -> bool:
    """Fail on an absolute path anywhere in the stored archive, keys and values."""
    bad = absolute_paths_in(blob)
    return pass_fail(
        label,
        not bad,
        "no absolute path stored in the archive"
        if not bad
        else f"{len(bad)} absolute path(s) stored in the archive: {bad[:4]}",
    )


# ---------------------------------------------------------------- fixture


def make_wav(path: Path, rate: int, channels: int, secs: float, hits: list, bleed: list) -> None:
    """Deterministic drum figure: decaying sine bursts on a low noise floor, plus quiet
    copies of a neighbour's hits (the bleed). Written by hand (the layout of the `wave`
    module, canonical PCM 16) plus a trailing iXML chunk: the ARATestHost's audio loader
    (ICST AudioFile.cpp GotoChunk) spins forever on a file that ends after the data
    chunk - the S1 fixtures carry the same chunk for the same reason."""
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
    fmt = struct.pack("<HHIIHH", 1, channels, rate, rate * channels * 2, channels * 2, 16)
    body = (
        b"WAVE"
        + b"fmt " + struct.pack("<I", len(fmt)) + fmt
        + b"data" + struct.pack("<I", len(frames)) + bytes(frames)
        + b"iXML" + struct.pack("<I", len(IXML_CHUNK)) + IXML_CHUNK
    )
    path.write_bytes(b"RIFF" + struct.pack("<I", len(body)) + body)


def stage_fixture() -> bool:
    global T0
    FIXTURES.mkdir(parents=True, exist_ok=True)
    for name, rate, ch, secs, hits, bleed in SPEC:
        make_wav(FIXTURES / name, rate, ch, secs, hits, bleed)
    T0 = time.time()
    (OUT / "t0").write_text(str(T0), encoding="utf-8")
    snapshot().write_text(json.dumps(walk_tree()), encoding="utf-8")
    # the fixtures sit in out/, which walk_tree skips: their hashes go into their own
    # record, written here, right after the wavs are made (stage_files compares it)
    (OUT / "fixture-sha256.json").write_text(
        json.dumps({name: hashlib.sha256((FIXTURES / name).read_bytes()).hexdigest() for name, *_ in SPEC}),
        encoding="utf-8",
    )
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
    changed = [p for p in after if p not in before or tuple(before[p]) != after[p]]
    walked = len(after)
    ok = pass_fail(
        "files-repo",
        not changed and walked > 0,
        f"{walked} files of the repo walked, {len(changed)} changed during the runs"
        + (f": {changed[:5]}" if changed else ""),
    ) and ok
    recorded = json.loads((OUT / "fixture-sha256.json").read_text())
    for name, *_ in SPEC:
        h = hashlib.sha256((FIXTURES / name).read_bytes()).hexdigest()
        ok = pass_fail("files-fixtures", recorded.get(name) == h, f"{name}: sha256 unchanged") and ok
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


def host_run(
    label: str, bundle: Path, args: list, cache: str, fault: bool = False, extra: dict | None = None, keep_cache: bool = False,
) -> tuple[int, str, list]:
    """One ara-host run with a fresh cache and report file (`keep_cache`: the cache folder
    of an earlier run stays, for a run in a NEW process on the same cache). Returns
    (rc, text, json lines)."""
    out_file = OUT / f"report-{label}.jsonl"
    if out_file.exists():
        out_file.unlink()
    cache_dir = OUT / cache
    if cache_dir.exists() and not keep_cache:
        shutil.rmtree(cache_dir)
    env = {**no_switch(), "DS_ARA_CACHE": str(cache_dir), "DS_ARA_OUT": str(out_file)}
    if fault:
        env["DS_ARA_FAULT"] = "in-process"
    if extra:
        env.update(extra)
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
    rc, text, lines = host_run(f"identity-{label}", bundle, ["session", "--files", *[str(FIXTURES / f) for f in FILES]], f"cache-identity-{label}")
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


def project_docs(cache: str) -> list[Path]:
    """The project documents of one run's cache folder: one per ARA document,
    `documents/<document id>/project/*.drumspike`."""
    root = OUT / cache / "documents"
    return sorted(root.glob("*/project/*.drumspike")) if root.is_dir() else []


def doc_of_run(cache: str) -> Path:
    """The one project document of a run with one ARA document (the M1a runs). The M1a
    layout (`session/session.drumspike`) is the fallback, so a stage can run on a base build."""
    docs = project_docs(cache)
    if len(docs) == 1:
        return docs[0]
    return OUT / cache / "session" / "session.drumspike"


def doc_segments(cache: str) -> list | None:
    """The segments of the plugin's document in one run's cache folder (every track)."""
    path = doc_of_run(cache)
    if not path.exists():
        return None
    doc = json.loads(path.read_text())
    segs: list = []
    for track in (doc.get("tracks") or {}).values():
        segs.extend(track.get("segments") or [])
    return segs


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
    doc_path = doc_of_run("cache-regions")
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

    # the update and destroy case: the properties of the SECOND region are rewritten and
    # the region is removed. A host region ref names its region, so the first region of
    # the source keeps its times and no record of the second is left in the document.
    base = ["regions", "--file", str(FIXTURES / REGIONS_FILE)]
    for a, d, p in REGIONS:
        base += ["--region", f"{a},{d},{p}"]

    rc, _text, _lines = host_run("regions-update", CLAP, [*base, "--update-region", f"1:{REGIONS_UPDATE[0]},{REGIONS_UPDATE[1]},{REGIONS_UPDATE[2]}"], "cache-regions-update")
    segs = doc_segments("cache-regions-update")
    landed = (
        segs is not None
        and len(segs) == 2
        and abs(segs[0]["start"] - REGIONS[0][0]) < 1e-9
        and abs(segs[0]["length"] - REGIONS[0][1]) < 1e-9
        and abs(segs[1]["start"] - REGIONS_UPDATE[0]) < 1e-9
        and abs(segs[1]["length"] - REGIONS_UPDATE[1]) < 1e-9
    )
    ok = pass_fail(
        "regions-update-record",
        rc == 0 and landed,
        f"after the update of the second region the document carries {len(segs or [])} segment(s), "
        f"first {segs[0]['start'] if segs else None}..{segs[0]['length'] if segs else None}, "
        f"second {segs[1]['start'] if segs and len(segs) > 1 else None}..{segs[1]['length'] if segs and len(segs) > 1 else None}",
    ) and ok

    rc, _text, _lines_d = host_run(
        "regions-destroy", CLAP, [*base, "--update-region", f"1:{REGIONS_UPDATE[0]},{REGIONS_UPDATE[1]},{REGIONS_UPDATE[2]}", "--destroy-region", "1"], "cache-regions-destroy"
    )
    segs = doc_segments("cache-regions-destroy")
    gone = (
        segs is not None
        and len(segs) == 1
        and abs(segs[0]["start"] - REGIONS[0][0]) < 1e-9
        and abs(segs[0]["length"] - REGIONS[0][1]) < 1e-9
    )
    ok = pass_fail(
        "regions-destroy-record",
        rc == 0 and gone,
        f"after the destroy of the second region the document carries {len(segs or [])} segment(s) "
        f"(the first region's times, no record of the removed region)",
    ) and ok
    rc, _text, lines_one = host_run("regions-one", CLAP, ["regions", "--file", str(FIXTURES / REGIONS_FILE), "--region", f"{REGIONS[0][0]},{REGIONS[0][1]},{REGIONS[0][2]}"], "cache-regions-one")
    ok_one, events_one = plugin_service_row("regions-one")
    ok = pass_fail("regions-one", rc == 0 and ok_one, f"the one-region reference run rc={rc}") and ok
    ok_destroy, events_destroy = plugin_service_row("regions-destroy")
    ok = pass_fail("regions-destroy-events", ok_destroy, "the destroy run wrote a service row") and ok
    if ok_one and ok_destroy:
        # the events of the mutated document are the one-region events: the removed
        # region contributes nothing and the surviving region kept its times
        ok = compare_events("regions-destroy", events_destroy, events_one) and ok
    return ok


def restore_decode_evidence(label: str, cache: str) -> tuple[list, list]:
    """The decode evidence of one restore run: its read rows and its cache WAVs."""
    report = OUT / f"report-{label}.jsonl"
    read_rows: list[str] = []
    if report.exists():
        read_rows = [ln for ln in report.read_text().splitlines() if '"event":"read"' in ln]
    # every WAV below the cache folder (the audio of a document lives in its own folder)
    root = OUT / cache
    wavs = sorted(root.rglob("*.wav")) if root.is_dir() else []
    return read_rows, wavs


def stage_archive() -> bool:
    """Gate item 4: store, restore in a new process, the decode counter stays at 0, and a
    stored archive fixture from the repo loads."""
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("archive", False, "no host or no CLAP: run check.sh build first")
    archive = OUT / "session.araarchive"
    rc, text, lines = host_run(
        "archive-store", CLAP, ["store", "--files", *[str(FIXTURES / f) for f in FILES], "--archive", str(archive)], "cache-archive-store"
    )
    if rc != 0 or not archive.exists():
        return pass_fail("archive-store", False, f"ara-host store rc={rc} (out/host-archive-store.log)")
    ok_store, events_store = plugin_service_row("archive-store")
    if not ok_store:
        return False
    want = (OUT / "events-reference.json").read_bytes()
    ok = compare_events("archive-store", events_store, want)
    ok = no_absolute_paths("archive-paths", json.loads(archive.read_text())) and ok

    # the restore in a NEW process: no analysis, no decode, the notes from the blob
    rc, text, lines = host_run(
        "archive-restore", CLAP, ["restore", "--files", *[str(FIXTURES / f) for f in FILES], "--archive", str(archive)], "cache-archive-restore"
    )
    if rc != 0:
        return pass_fail("archive-restore", False, f"ara-host restore rc={rc} (out/host-archive-restore.log)")
    ok_r, events_restore = plugin_service_row("archive-restore")
    if not ok_r:
        return False
    # the restore decodes no file: no read row in its report and no cache WAV in its
    # cache folder (the decode counter of a fresh service is always empty, so the
    # restore itself is measured - review finding 3 on #525)
    read_rows, wavs = restore_decode_evidence("archive-restore", "cache-archive-restore")
    ok = pass_fail(
        "archive-decodes",
        not read_rows and not wavs,
        f"the restore decoded no file: {len(read_rows)} read row(s) in its report, {len(wavs)} cache WAV(s)",
    )
    ok = compare_events("archive-restore", events_restore, events_store) and ok
    ok = check_notes("archive-restore", lines, events_store, rates_of()) and ok

    # the fault run: the plugin decodes one source during the restore on purpose, so the
    # no-decode proof fires (the check can fail, as the threads fault run does)
    rc, text, lines = host_run(
        "archive-restore-fault",
        CLAP,
        ["restore", "--files", *[str(FIXTURES / f) for f in FILES], "--archive", str(archive)],
        "cache-archive-restore-fault",
        extra={"DS_ARA_RESTORE_FAULT": "decode"},
    )
    read_rows, wavs = restore_decode_evidence("archive-restore-fault", "cache-archive-restore-fault")
    ok = pass_fail(
        "archive-decodes-fault",
        bool(read_rows) or bool(wavs),
        f"the fault switch decoded one source during the restore: {len(read_rows)} read row(s), "
        f"{len(wavs)} cache WAV(s) - the no-decode proof fires",
    ) and ok

    # the stored archive fixture of the repo loads, and it carries the version field
    if not ARCHIVE_FIXTURE.exists():
        return pass_fail(
            "archive-fixture", False, f"no stored archive fixture at {ARCHIVE_FIXTURE} (check.py make-fixture)"
        )
    blob = json.loads(ARCHIVE_FIXTURE.read_text())
    ok = no_absolute_paths("archive-fixture-paths", blob) and ok
    ok = pass_fail(
        "archive-fixture-version",
        blob.get("version") == 1,
        f"the fixture carries version {blob.get('version')}",
    )
    rc, text, lines = host_run(
        "archive-fixture", CLAP, ["restore", "--files", *[str(FIXTURES / f) for f in FILES], "--archive", str(ARCHIVE_FIXTURE)], "cache-archive-fixture"
    )
    if rc != 0:
        return pass_fail("archive-fixture", False, f"the fixture restore rc={rc} (out/host-archive-fixture.log)")
    ok_f, events_fixture = plugin_service_row("archive-fixture")
    if not ok_f:
        return False
    ok = compare_events("archive-fixture", events_fixture, want) and ok
    return ok


def stage_archive_paths_fault() -> bool:
    """The fault run of the archive path check: an archive blob made from the real stored
    one, with an absolute source name and, in a second blob, an absolute track key, must
    make the check report a failure (the check can fail). The deliberate failure is not
    added to the global list; only a check that stays silent is."""
    archive = OUT / "session.araarchive"
    if not archive.exists():
        return pass_fail("archive-paths-fault", False, "no stored archive: run the archive stage first")
    real = json.loads(archive.read_text())
    if not real.get("sources") or not real.get("events"):
        return pass_fail("archive-paths-fault", False, "the stored archive has no sources or no events")
    if absolute_paths_in(real):
        return pass_fail("archive-paths-fault", False, "the real stored archive already holds an absolute path")
    in_value = json.loads(json.dumps(real))
    in_value["sources"][0]["name"] = str(FIXTURES / FILES[0])
    in_key = json.loads(json.dumps(real))
    first = next(iter(in_key["events"]))
    in_key["events"][str(FIXTURES / first)] = in_key["events"].pop(first)
    marker = len(FAILURES)
    saved_stdout = sys.stdout
    sys.stdout = io.StringIO()  # the two deliberate FAIL lines are no result of the run
    try:
        ok_value = no_absolute_paths("archive-paths-fault-value", in_value)
        ok_key = no_absolute_paths("archive-paths-fault-key", in_key)
    finally:
        sys.stdout = saved_stdout
    del FAILURES[marker:]
    return pass_fail(
        "archive-paths-fault",
        not ok_value and not ok_key,
        f"an absolute source name was reported: {not ok_value}, an absolute track key was reported: "
        f"{not ok_key} - the path check can fail",
    )


def stage_archive_restore_pathname() -> bool:
    """A host that names an audio source by its path and gives no persistent ID: the
    archive stores the reduced name (the file name), so the restore must match on the
    reduced form of the host's name. The notes of the restore must be the stored ones."""
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("archive-restore-pathname", False, "no host or no CLAP: run check.sh build first")
    if not (OUT / "events-reference.json").exists():
        return pass_fail("archive-restore-pathname", False, "no reference events: run the reference stage first")
    files = [str(FIXTURES / f) for f in FILES]
    archive = OUT / "pathname.araarchive"
    rc, text, store_lines = host_run(
        "pathname-store", CLAP, ["store", "--files", *files, "--archive", str(archive)], "cache-pathname-store"
    )
    if rc != 0 or not archive.exists():
        return pass_fail("archive-restore-pathname", False, f"ara-host store rc={rc} (out/host-pathname-store.log)")
    ok_store, events_store = plugin_service_row("pathname-store")
    if not ok_store:
        return False
    rc, text, lines = host_run(
        "pathname-restore",
        CLAP,
        ["restore", "--files", *files, "--archive", str(archive), "--no-persistent-id"],
        "cache-pathname-restore",
    )
    if rc != 0:
        return pass_fail("archive-restore-pathname", False, f"ara-host restore rc={rc} (out/host-pathname-restore.log)")
    names = [r.get("name", "") for r in lines if r.get("event") == "host_notes"]
    absolute = [n for n in names if n.startswith("/")]
    ok = pass_fail(
        "archive-restore-pathname-names",
        bool(names) and len(absolute) == len(names),
        f"the host named {len(absolute)} of {len(names)} source(s) by an absolute path, with no persistent ID",
    )
    ok_r, events_restore = plugin_service_row("pathname-restore")
    if not ok_r:
        return False
    ok = compare_events("archive-restore-pathname", events_restore, events_store) and ok
    # the restored notes are the stored ones: the host reads them through the content
    # readers, and a source that found no archive entry gives none (the case of the defect)
    stored = {r.get("name"): r.get("samples") for r in store_lines if r.get("event") == "host_notes"}
    total, mismatched = 0, []
    for r in lines:
        if r.get("event") != "host_notes":
            continue
        total += r.get("count", 0)
        if r.get("samples") != stored.get(r.get("name")) or not r.get("samples"):
            mismatched.append(stem_of(r.get("name", "")))
    ok = pass_fail(
        "archive-restore-pathname-notes",
        total > 0 and not mismatched,
        f"{total} restored note(s) equal the stored ones"
        if total > 0 and not mismatched
        else f"{total} restored note(s); source(s) without the stored notes: {mismatched}",
    ) and ok
    return check_notes("archive-restore-pathname", lines, events_store, rates_of()) and ok


MIN_BLOCKS = 16


def report_rows(label: str) -> list:
    """The JSON rows of the plugin's report of one run (DS_ARA_OUT)."""
    path = OUT / f"report-{label}.jsonl"
    if not path.exists():
        return []
    rows = []
    for ln in path.read_text().splitlines():
        if ln.strip():
            try:
                rows.append(json.loads(ln))
            except ValueError:
                pass
    return rows


def stage_threads() -> bool:
    """Gate item 6: audio blocks during the analysis have no overrun, the block count has
    a lower limit, and the analysis does not run on the request caller's thread; the
    fault run (the analysis inside `process`) makes both checks fire."""
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("threads", False, "no host or no CLAP: run check.sh build first")
    rc, text, lines = host_run("blocks", CLAP, ["blocks", "--files", *[str(FIXTURES / f) for f in FILES], "--frames", "512"], "cache-blocks")
    report = next((r for r in reversed(lines) if r.get("event") == "blocks"), {})
    rows = report_rows("blocks")
    requests = [r for r in rows if r.get("event") == "request"]
    service = next((r for r in rows if r.get("event") == "service"), {})
    on_request_thread = [r for r in requests if r.get("thread") == service.get("analysis_thread")]
    ok = pass_fail(
        "threads",
        rc == 0 and report.get("overruns") == 0 and report.get("event_file") is True,
        f"rc={rc}, {report.get('blocks')} blocks while the analysis ran, overruns={report.get('overruns')}, "
        f"worst {report.get('worst_block_us')}us of {report.get('budget_us')}us per block",
    )
    ok = pass_fail(
        "threads-blocks",
        rc == 0 and report.get("blocks", 0) >= MIN_BLOCKS,
        f"{report.get('blocks')} blocks ran while the analysis was in flight (lower limit {MIN_BLOCKS}; "
        "a synchronous analysis inside the request call would end the loop at the first block)",
    ) and ok
    ok = pass_fail(
        "threads-off-request",
        bool(requests) and not on_request_thread,
        f"{len(requests)} request call(s) on thread {[r.get('thread') for r in requests]}, "
        f"the analysis ran on {service.get('analysis_thread')} - not the caller's thread",
    ) and ok
    rc, text, lines = host_run(
        "blocks-fault", CLAP, ["blocks", "--files", *[str(FIXTURES / f) for f in FILES], "--frames", "512"], "cache-blocks-fault", fault=True
    )
    report = next((r for r in reversed(lines) if r.get("event") == "blocks"), {})
    ok = pass_fail(
        "threads-fault",
        rc != 0 and report.get("overruns", 0) >= 1,
        f"the fault switch ran the analysis inside process: rc={rc}, {report.get('overruns')} block(s) over budget "
        f"(worst {report.get('worst_block_us')}us), event_file={report.get('event_file')}",
    ) and ok
    requests = [r for r in report_rows("blocks-fault") if r.get("event") == "request"]
    service = next((r for r in report_rows("blocks-fault") if r.get("event") == "service"), {})
    ok = pass_fail(
        "threads-fault-thread",
        bool(requests) and service.get("analysis_thread") in [r.get("thread") for r in requests],
        f"the fault job ran on the request caller's thread ({service.get('analysis_thread')}): "
        "the off-request measurement fires on the deliberate violation",
    ) and ok
    return ok


# ---------------------------------------------------------------- M1b1: one service and one project per ARA document
#
# The ara-host modes `grow` (the sources arrive in batches, one source can be destroyed at
# the end) and `docs` (two document controllers in one process) drive these stages. The
# plugin reports `job` rows (one per analysis round of a document: its id, folder, the
# tracks of the project, the events as the bytes the service gave), a `services` row (the
# services made and dropped), `doc_restored` rows and `source_destroyed` rows. Every check
# is a function that returns (ok, evidence); its fault run (a switch of the plugin that
# breaks exactly that line) must make the same function return False.

_GROW: dict[str, tuple[int, list]] = {}
HOST_WAIT = "30"  # seconds the host waits for one analysis: a broken plugin fails, it does not hang


def job_rows(label: str) -> list:
    return [r for r in report_rows(label) if r.get("event") == "job"]


def event_rows(label: str, event: str) -> list:
    return [r for r in report_rows(label) if r.get("event") == event]


def services_row(label: str) -> dict:
    rows = event_rows(label, "services")
    return rows[-1] if rows else {}


def track_names(files: list) -> list:
    return [safe_name(stem_of(f)) for f in files]


def fixture_args(files: list) -> list:
    return [str(FIXTURES / f) for f in files]


def grow_run(label: str, files: list, batch: int, destroy: int | None = None, extra: dict | None = None) -> tuple[int, list]:
    """One `grow` run of the host (memoised: two stages read one run)."""
    if label not in _GROW:
        args = ["grow", "--files", *fixture_args(files), "--batch", str(batch), "--wait", HOST_WAIT]
        if destroy is not None:
            args += ["--destroy", str(destroy)]
        rc, _text, lines = host_run(label, CLAP, args, f"cache-{label}", extra=extra)
        _GROW[label] = (rc, lines)
    return _GROW[label]


def host_notes_of(lines: list, phase: str) -> dict:
    return {r["name"]: r["samples"] for r in lines if r.get("event") == "host_notes" and r.get("phase") == phase}


def tracks_of_doc(path: Path) -> dict:
    return json.loads(path.read_text()).get("tracks") or {}


def file_state(root: Path) -> dict:
    """path -> (size, mtime_ns) of every file below `root`."""
    out = {}
    if root.is_dir():
        for p in root.rglob("*"):
            if p.is_file():
                st = p.stat()
                out[str(p)] = (st.st_size, st.st_mtime_ns)
    return out


# ---- gate line 1: one service per document

SERVICE_FILES = FILES  # four sources, two requests of two


def check_services(label: str) -> tuple[bool, str]:
    jobs = job_rows(label)
    sv = services_row(label)
    docs = {j.get("doc") for j in jobs}
    made, dropped = sv.get("made"), sv.get("dropped")
    ok = made == 1 and dropped == 1 and len(jobs) == 2 and len(docs) == 1
    return ok, (
        f"{len(jobs)} analysis requests in {len(docs)} document(s): {made} service(s) made, {dropped} dropped "
        "with the document controller (want 1 and 1)"
    )


def stage_doc_service() -> bool:
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("doc-service", False, "no host or no CLAP: run check.sh build first")
    rc, _lines = grow_run("doc-grow", SERVICE_FILES, 2)
    if rc != 0:
        return pass_fail("doc-service", False, f"ara-host grow rc={rc} (out/host-doc-grow.log)")
    ok, evidence = check_services("doc-grow")
    return pass_fail("doc-service", ok, evidence)


def stage_doc_service_fault() -> bool:
    """The fault run: the plugin makes a new service for every job (the M1a way)."""
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("doc-service-fault", False, "no host or no CLAP: run check.sh build first")
    rc, _lines = grow_run("doc-grow-service-fault", SERVICE_FILES, 2, extra={"DS_ARA_SERVICE_FAULT": "per-job"})
    ok, evidence = check_services("doc-grow-service-fault")
    return pass_fail(
        "doc-service-fault",
        rc == 0 and not ok and services_row("doc-grow-service-fault").get("made", 0) >= 2,
        f"the fault switch made a service per job and the check refused it: {evidence}",
    )


# ---- gate line 2: the tracks stay

def check_tracks(label: str, cache: str, lines: list, files: list) -> tuple[bool, str]:
    jobs = job_rows(label)
    docs0 = project_docs(cache)
    on_disk0 = len(tracks_of_doc(docs0[0])) if len(docs0) == 1 else len(tracks_of_doc(doc_of_run(cache))) if doc_of_run(cache).exists() else 0
    if len(jobs) != 2:
        return False, f"{len(jobs)} job rows, want 2 (one per request); the document on disk has {on_disk0} tracks, want {len(files)}"
    first, second = jobs
    t1, t2 = first.get("track_json") or {}, second.get("track_json") or {}
    kept = [n for n in t1 if t2.get(n) == t1[n]]  # the strings are the service's bytes
    docs = project_docs(cache)
    doc_tracks = tracks_of_doc(docs[0]) if len(docs) == 1 else {}
    n_tracks = len(second.get("project_tracks") or [])
    events = sum(len(json.loads(v)) for v in t2.values())
    in_doc = bool(t2) and all(n in doc_tracks and doc_tracks[n].get("events") == json.loads(t2[n]) for n in t2)
    notes1, notes2 = host_notes_of(lines, "round-1"), host_notes_of(lines, "final")
    notes_same = bool(notes1) and all(notes2.get(n) == v for n, v in notes1.items())
    ok = (
        n_tracks == len(files)
        and len(doc_tracks) == len(files)
        and list(t2) == track_names(files)
        and len(t1) == 2
        and kept == list(t1)
        and events > 0
        and in_doc
        and notes_same
    )
    return ok, (
        f"{n_tracks} tracks and {events} events in the project after the second request; "
        f"{len(kept)} of {len(t1)} earlier tracks keep their events byte for byte; "
        f"the document on disk has {len(doc_tracks)} tracks (events equal: {in_doc}); "
        f"the host reads the same notes for the first sources: {notes_same}"
    )


def stage_doc_tracks() -> bool:
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("doc-tracks", False, "no host or no CLAP: run check.sh build first")
    rc, lines = grow_run("doc-grow", SERVICE_FILES, 2)
    if rc != 0:
        return pass_fail("doc-tracks", False, f"ara-host grow rc={rc} (out/host-doc-grow.log)")
    ok, evidence = check_tracks("doc-grow", "cache-doc-grow", lines, SERVICE_FILES)
    return pass_fail("doc-tracks", ok, evidence)


def stage_doc_tracks_fault() -> bool:
    """The fault run: the second request forgets the earlier tracks (the M1a way)."""
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("doc-tracks-fault", False, "no host or no CLAP: run check.sh build first")
    rc, lines = grow_run("doc-grow-tracks-fault", SERVICE_FILES, 2, extra={"DS_ARA_TRACKS_FAULT": "forget"})
    ok, evidence = check_tracks("doc-grow-tracks-fault", "cache-doc-grow-tracks-fault", lines, SERVICE_FILES)
    return pass_fail(
        "doc-tracks-fault",
        rc == 0 and not ok,
        f"the fault switch dropped the earlier tracks and the check refused it: {evidence}",
    )


# ---- gate line 3: two documents in one process

DOC_A, DOC_B = FILES[:2], FILES[2:]


def reference_alone(name: str, files: list) -> bytes | None:
    """`service_tool analyze` on a project of these files alone: the raw events object."""
    ref = OUT / name
    if ref.exists():
        shutil.rmtree(ref)
    script = OUT / f"run-{name}.json"
    script.write_text(
        json.dumps(
            [
                {"cmd": "project_new", "args": {"name": name, "dir": str(OUT)}},
                {"cmd": "tracks_add", "args": {"sid": "$sid", "paths": fixture_args(files), "copy": True}},
                {"cmd": "project_save", "args": {"sid": "$sid", "force": True}},
            ]
        ),
        encoding="utf-8",
    )
    rc, _text = run([str(SERVICE_TOOL), "run", str(script)], env=no_switch(), log=f"{name}-build.log")
    if rc != 0:
        return None
    ok, events, n = service_tool_events(ref, name)
    return events if ok else None


def check_two(label: str, cache: str, lines: list, refs: list) -> tuple[bool, str]:
    by_doc: dict = {}
    for j in job_rows(label):
        by_doc.setdefault(j.get("doc"), []).append(j)
    if len(by_doc) != 2:
        return False, f"{len(by_doc)} document(s) in the report, want 2"
    rows = [r[-1] for r in by_doc.values()]
    folders = [Path(r.get("folder", "")) for r in rows]
    root = OUT / cache / "documents"
    own = len({str(f) for f in folders}) == 2 and all(f.parent == root and f.is_dir() for f in folders)
    same = [r.get("events_json", "").encode() == ref for r, ref in zip(rows, refs)]
    events = sum(len(v) for r in rows for v in (json.loads(x) for x in (r.get("track_json") or {}).values()))
    start = next((r["unix_ns"] for r in lines if r.get("event") == "host_phase" and r.get("phase") == "start"), 0)
    state_a = file_state(folders[0]) if own else {}
    quiet = bool(state_a) and start > 0 and all(mt <= start for _size, mt in state_a.values())
    docs = project_docs(cache)
    two_tracks = len(docs) == 2 and all(len(tracks_of_doc(d)) == 2 for d in docs)
    audio = [sorted(p.name[:3] for p in (f / "audio").glob("*.wav")) for f in folders] if own else []
    own_numbering = audio == [["000", "001"], ["000", "001"]]
    ok = own and all(same) and events > 0 and quiet and two_tracks and own_numbering
    return ok, (
        f"two documents, {events} events: own folders {own}, events byte for byte the reference of each document "
        f"alone {same}, no file of the first folder changed while the second worked ({len(state_a)} files, quiet {quiet}), "
        f"each project holds its 2 tracks {two_tracks}, own source numbering {own_numbering}"
    )


def doc_two_run(label: str, extra: dict | None = None) -> tuple[int, list]:
    rc, _text, lines = host_run(
        label, CLAP, ["docs", "--files", *fixture_args(DOC_A), "--files2", *fixture_args(DOC_B), "--wait", HOST_WAIT], f"cache-{label}", extra=extra
    )
    return rc, lines


def stage_doc_two() -> bool:
    if not HOST.exists() or not CLAP.exists() or not SERVICE_TOOL.exists():
        return pass_fail("doc-two", False, "no host, no CLAP or no service_tool: run check.sh build first")
    ref_a, ref_b = reference_alone("m1b1-ref-a", DOC_A), reference_alone("m1b1-ref-b", DOC_B)
    if ref_a is None or ref_b is None:
        return pass_fail("doc-two", False, "the reference of one document alone failed")
    rc, lines = doc_two_run("doc-two")
    if rc != 0:
        return pass_fail("doc-two", False, f"ara-host docs rc={rc} (out/host-doc-two.log)")
    ok, evidence = check_two("doc-two", "cache-doc-two", lines, [ref_a, ref_b])
    return pass_fail("doc-two", ok, evidence)


def stage_doc_two_fault() -> bool:
    """The fault run: both documents use one folder (the M1a way)."""
    if not HOST.exists() or not CLAP.exists() or not SERVICE_TOOL.exists():
        return pass_fail("doc-two-fault", False, "no host, no CLAP or no service_tool: run check.sh build first")
    ref_a, ref_b = reference_alone("m1b1-ref-a", DOC_A), reference_alone("m1b1-ref-b", DOC_B)
    if ref_a is None or ref_b is None:
        return pass_fail("doc-two-fault", False, "the reference of one document alone failed")
    rc, lines = doc_two_run("doc-two-fault", extra={"DS_ARA_FOLDER_FAULT": "shared"})
    ok, evidence = check_two("doc-two-fault", "cache-doc-two-fault", lines, [ref_a, ref_b])
    return pass_fail(
        "doc-two-fault", not ok, f"the fault switch put both documents in one folder and the check refused it (rc={rc}): {evidence}"
    )


# ---- gate line 4: the id in the archive, the same folder after a restore

_ARCHIVE_STORE: dict[str, Path] = {}


def archive_store() -> tuple[bool, Path, str]:
    """Store the four-source session once (memoised): the archive, the cache folder stays."""
    archive = OUT / "doc-archive.araarchive"
    if "archive" not in _ARCHIVE_STORE:
        rc, _text, _lines = host_run(
            "doc-archive-store", CLAP, ["store", "--files", *fixture_args(FILES), "--archive", str(archive)], "cache-doc-archive"
        )
        if rc != 0 or not archive.exists():
            return False, archive, f"ara-host store rc={rc} (out/host-doc-archive-store.log)"
        _ARCHIVE_STORE["archive"] = archive
    return True, archive, ""


def check_archive(archive: Path, restore_label: str, cache: str, audio_before: dict) -> tuple[bool, str]:
    blob = json.loads(archive.read_text())
    doc_id = blob.get("doc_id")
    stored = job_rows("doc-archive-store")
    if not isinstance(doc_id, str) or not doc_id or not stored:
        return False, f"the archive stores no document id ({doc_id!r}) or the store run has no job row"
    want = stored[-1]
    rows = [r for r in event_rows(restore_label, "doc_restored")]
    if len(rows) != 1:
        return False, f"{len(rows)} doc_restored rows, want 1"
    row = rows[0]
    same_folder = row.get("doc") == doc_id and row.get("folder") == want.get("folder")
    events_same = row.get("events_json") == want.get("events_json") and bool(row.get("events_json"))
    n_events = sum(len(v) for v in json.loads(row.get("events_json") or "{}").values())
    names = row.get("project_tracks") == want.get("project_tracks") and len(row.get("project_tracks") or []) == len(FILES)
    reads = [r for r in report_rows(restore_label) if r.get("event") == "read"]
    decodes = row.get("decodes")
    audio_after = {k: v for k, v in file_state(OUT / cache).items() if k.endswith(".wav")}
    no_decode = not reads and decodes == 0 and audio_after == audio_before and bool(audio_before)
    docs = project_docs(cache)
    on_disk = len(docs) == 1 and all(
        t.get("events") == json.loads(want["track_json"][n]) for n, t in tracks_of_doc(docs[0]).items()
    )
    ok = same_folder and events_same and n_events > 0 and names and no_decode and on_disk
    return ok, (
        f"restored in a new process: the stored document id gives the same folder {same_folder}, "
        f"the project opens with the stored events byte for byte {events_same} ({n_events} events, "
        f"{len(row.get('project_tracks') or [])} tracks), no decode: {len(reads)} read row(s), decode counter {decodes}, "
        f"{len(audio_before)} cache WAV(s) unchanged {audio_after == audio_before}, document on disk equal {on_disk}"
    )


def restore_run(label: str, archive: Path, cache: str, extra: dict | None = None) -> tuple[int, dict]:
    before = {k: v for k, v in file_state(OUT / cache).items() if k.endswith(".wav")}
    rc, _text, _lines = host_run(
        label, CLAP, ["restore", "--files", *fixture_args(FILES), "--archive", str(archive)], cache, extra=extra, keep_cache=True
    )
    return rc, before


def stage_doc_archive() -> bool:
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("doc-archive", False, "no host or no CLAP: run check.sh build first")
    ok_store, archive, why = archive_store()
    if not ok_store:
        return pass_fail("doc-archive", False, why)
    rc, before = restore_run("doc-archive-restore", archive, "cache-doc-archive")
    if rc != 0:
        return pass_fail("doc-archive", False, f"ara-host restore rc={rc} (out/host-doc-archive-restore.log)")
    ok, evidence = check_archive(archive, "doc-archive-restore", "cache-doc-archive", before)
    ok = pass_fail("doc-archive", ok, evidence)
    # an archive of M1a (version 1, no document id): the restore makes a fresh id and
    # opens the project with the stored events, and decodes nothing
    if not ARCHIVE_FIXTURE.exists():
        return pass_fail("doc-archive-v1", False, f"no stored archive fixture at {ARCHIVE_FIXTURE}")
    rc, _text, _lines = host_run(
        "doc-archive-v1",
        CLAP,
        ["restore", "--files", *fixture_args(FILES), "--archive", str(ARCHIVE_FIXTURE)],
        "cache-doc-archive-v1",
    )
    rows = event_rows("doc-archive-v1", "doc_restored")
    fixture = json.loads(ARCHIVE_FIXTURE.read_text())
    row = rows[0] if len(rows) == 1 else {}
    events = json.loads(row.get("events_json") or "{}")
    reads = [r for r in report_rows("doc-archive-v1") if r.get("event") == "read"]
    v1_ok = (
        rc == 0
        and fixture.get("version") == 1
        and "doc_id" not in fixture
        and bool(row.get("doc"))
        and events == fixture.get("events")
        and sum(len(v) for v in events.values()) > 0
        and row.get("decodes") == 0
        and not reads
    )
    return pass_fail(
        "doc-archive-v1",
        v1_ok,
        f"the committed version-1 archive (no document id) restores into the fresh id {row.get('doc')} with "
        f"{sum(len(v) for v in events.values())} stored events, {len(reads)} read row(s), decode counter {row.get('decodes')}",
    ) and ok


def stage_doc_archive_fault() -> bool:
    """Two fault runs of the restore: another folder than the stored id gives, and a
    project without the stored events. The check must refuse both."""
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("doc-archive-fault", False, "no host or no CLAP: run check.sh build first")
    ok_store, archive, why = archive_store()
    if not ok_store:
        return pass_fail("doc-archive-fault", False, why)
    refused = {}
    for fault in ("newid", "empty"):
        rc, before = restore_run(f"doc-archive-restore-{fault}", archive, "cache-doc-archive", extra={"DS_ARA_RESTORE_FAULT": fault})
        ok, evidence = check_archive(archive, f"doc-archive-restore-{fault}", "cache-doc-archive", before)
        refused[fault] = (rc == 0 and not ok, evidence)
    return pass_fail(
        "doc-archive-fault",
        all(v[0] for v in refused.values()),
        "the fault switches (another folder, no stored events) were refused by the check: "
        + "; ".join(f"{k}: {v[0]}" for k, v in refused.items()),
    )


# ---- gate line 5: a destroyed source leaves the project

DESTROY_FILES = [FILES[0], FILES[1], FILES[2]]  # kick, snare, tom; the snare goes


def check_destroy(label: str, cache: str, lines: list) -> tuple[bool, str]:
    jobs = job_rows(label)
    docs0 = project_docs(cache)
    on_disk0 = len(tracks_of_doc(docs0[0])) if len(docs0) == 1 else len(tracks_of_doc(doc_of_run(cache))) if doc_of_run(cache).exists() else 0
    if len(jobs) != 1:
        return False, f"{len(jobs)} job rows, want 1; the document on disk has {on_disk0} tracks after the destroy, want 2"
    before = jobs[0].get("track_json") or {}
    names = track_names(DESTROY_FILES)
    gone, left = names[1], [names[0], names[2]]
    docs = project_docs(cache)
    doc_tracks = tracks_of_doc(docs[0]) if len(docs) == 1 else {}
    destroyed = event_rows(label, "source_destroyed")
    row = destroyed[-1] if destroyed else {}
    events_same = bool(left) and all(
        n in doc_tracks and doc_tracks[n].get("events") == json.loads(before.get(n, "null")) for n in left
    )
    wavs = sorted(p.name[:3] for d in (OUT / cache / "documents").glob("*/audio") for p in d.glob("*.wav"))
    notes_after = host_notes_of(lines, "after-destroy")
    notes_final = host_notes_of(lines, "final")
    notes_same = len(notes_after) == 2 and all(notes_final.get(n) == v for n, v in notes_after.items())
    ok = (
        len(before) == 3
        and list(doc_tracks) == left
        and gone not in doc_tracks
        and len(destroyed) == 1
        and row.get("sources_left") == 2
        and row.get("project_tracks") == left
        and events_same
        and wavs == ["000", "002"]
        and notes_same
    )
    return ok, (
        f"3 sources analysed, 1 destroyed: the plugin's source list holds {row.get('sources_left')}, the project "
        f"{row.get('project_tracks')}, the document on disk {list(doc_tracks)} (the destroyed track gone: "
        f"{gone not in doc_tracks}), the events of the others byte for byte as before {events_same}, "
        f"cache audio of the destroyed source gone {wavs == ['000', '002']}"
    )


def stage_doc_destroy() -> bool:
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("doc-destroy", False, "no host or no CLAP: run check.sh build first")
    rc, lines = grow_run("doc-destroy", DESTROY_FILES, 3, destroy=1)
    if rc != 0:
        return pass_fail("doc-destroy", False, f"ara-host grow rc={rc} (out/host-doc-destroy.log)")
    ok, evidence = check_destroy("doc-destroy", "cache-doc-destroy", lines)
    return pass_fail("doc-destroy", ok, evidence)


def stage_doc_destroy_fault() -> bool:
    """The fault run: the plugin keeps the destroyed source (the M1a way)."""
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("doc-destroy-fault", False, "no host or no CLAP: run check.sh build first")
    rc, lines = grow_run("doc-destroy-fault", DESTROY_FILES, 3, destroy=1, extra={"DS_ARA_DESTROY_FAULT": "keep"})
    ok, evidence = check_destroy("doc-destroy-fault", "cache-doc-destroy-fault", lines)
    return pass_fail(
        "doc-destroy-fault", rc == 0 and not ok, f"the fault switch kept the destroyed source and the check refused it: {evidence}"
    )


# ---- gate line 6: the service home is inside the cache folder, nothing else below the user's home

HOME_FILES = FILES[:2]


def check_home(label: str, home: Path, cache: Path) -> tuple[bool, str]:
    """The files written below `home` outside `cache`: 0. The files inside the cache and
    the service home inside it (with its recent list) make the check non-vacuous."""
    below = [p for p in home.rglob("*") if p.is_file()] if home.is_dir() else []
    outside = [p for p in below if cache not in p.parents]
    inside = [p for p in below if cache in p.parents]
    jobs = job_rows(label)
    svc_home = Path(jobs[-1].get("service_home", "")) if jobs else Path("")
    home_in_cache = bool(jobs) and cache in svc_home.parents and (svc_home / "recent.json").is_file()
    ok = not outside and len(inside) > 0 and home_in_cache
    listed = [str(p.relative_to(home)) for p in outside[:3]]
    return ok, (
        f"{len(outside)} file(s) written below the user's home outside the cache folder{': ' + str(listed) if listed else ''}, "
        f"{len(inside)} inside it; the service home is inside the cache folder with its recent list: {home_in_cache}"
    )


def home_run(label: str, extra: dict | None = None) -> tuple[int, Path, Path]:
    """A run with HOME pointed at a folder below out/ (the real home is never written) and
    the cache folder where the README puts it below that home."""
    home = OUT / f"home-{label}"
    if home.exists():
        shutil.rmtree(home)
    home.mkdir(parents=True)
    cache_rel = f"home-{label}/Library/Caches/DrumSpike/ARA"
    rc, _text, _lines = host_run(
        label, CLAP, ["session", "--files", *fixture_args(HOME_FILES)], cache_rel, extra={"HOME": str(home), **(extra or {})}
    )
    return rc, home, OUT / cache_rel


def stage_doc_home() -> bool:
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("doc-home", False, "no host or no CLAP: run check.sh build first")
    rc, home, cache = home_run("doc-home")
    if rc != 0:
        return pass_fail("doc-home", False, f"ara-host session rc={rc} (out/host-doc-home.log)")
    ok, evidence = check_home("doc-home", home, cache)
    return pass_fail("doc-home", ok, evidence)


def stage_doc_home_fault() -> bool:
    """The fault run: the service home is a folder below the user's home, outside the cache."""
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("doc-home-fault", False, "no host or no CLAP: run check.sh build first")
    rc, home, cache = home_run("doc-home-fault", extra={"DS_ARA_HOME_FAULT": "1"})
    ok, evidence = check_home("doc-home-fault", home, cache)
    return pass_fail(
        "doc-home-fault", rc == 0 and not ok, f"the fault switch put the service home below the user's home and the check refused it: {evidence}"
    )


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
            # the runs must not reach another plugin by accident. The DS_ARA_CACHE
            # override stays: without it the plugin wrote audio/ and session/ into the
            # real cache folder the README names (review finding 5 on #525); now every
            # stray write lands below out/.
            env = {
                **no_switch(),
                "CLAP_PATH": None,
                "DS_ARA_CACHE": str(OUT / f"cache-conformance-{kind}-{test}"),
            }
            rc, _text = run(
                [str(TESTHOST), f"-{kind}", str(bundle), "-file", *[str(FIXTURES / name) for name in names], "-test", test],
                env=env,
                log=f"conformance-{kind}-{test}.log",
                cwd=OUT,
                timeout=120,
            )
            evidence = f"ARATestHost {test} on the {kind}: rc={rc} (out/conformance-{kind}-{test}.log)"
            if rc == 124:
                evidence = (
                    f"ARATestHost {test} on the {kind}: TIMEOUT after 120 s, killed as a FAIL"
                    f" (out/conformance-{kind}-{test}.log)"
                )
            ok = pass_fail(f"conformance-{kind}-{test}", rc == 0, evidence) and ok
    return ok


# ---------------------------------------------------------------- validators


def stage_tables() -> bool:
    """The interface-table walk (the empty-slot finding on #525): ara-host tables loads
    the plugin, binds all three roles and walks every ARA interface table up to the
    structSize the plugin declares - the ARA factory, the document controller and the
    playback renderer, editor renderer and editor view tables. ARA marks optional parts
    by structSize (ARAInterface.h), so every slot inside the declared size must carry a
    function; the SDK hosts call the slots unchecked (ARAHostDispatch.cpp). The stage
    fails on the first empty slot, on an assigned role without an interface, on 0 slots
    checked, and on a slot count other than the expected 65."""
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("tables", False, "no host or no CLAP: run check.sh build first")
    rc, text = run([str(HOST), str(CLAP), "tables"], env={**no_switch(), "DS_ARA_CACHE": str(OUT / "cache-tables")}, log="tables.log")
    if rc != 0:
        return pass_fail("tables", False, f"ara-host tables rc={rc} (out/tables.log)")
    row = None
    for line in reversed((text or "").strip().splitlines()):
        try:
            candidate = json.loads(line)
        except ValueError:
            continue
        if isinstance(candidate, dict) and candidate.get("event") == "tables":
            row = candidate
            break
    if row is None:
        return pass_fail("tables", False, "ara-host tables printed no report line (out/tables.log)")
    n = row.get("slots", 0)
    walked = ", ".join(f"{k}={v}" for k, v in row.get("tables", {}).items())
    return pass_fail(
        "tables",
        n == EXPECTED_TABLE_SLOTS,
        f"{n} of {EXPECTED_TABLE_SLOTS} slots walked up to the declared structSizes ({walked}); "
        "the stage fails on an empty slot, on 0, and on a count other than the expected 65",
    )


def stage_tables_fault() -> bool:
    """The fault run of the tables stage: the plugin serves a document controller table
    with one emptied slot (DS_ARA_TABLES_FAULT), and the walk must refuse it - the proof
    that the empty-slot check can fail (as the threads fault run does)."""
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("tables-fault", False, "no host or no CLAP: run check.sh build first")
    rc, text = run(
        [str(HOST), str(CLAP), "tables"],
        env={**no_switch(), "DS_ARA_CACHE": str(OUT / "cache-tables-fault"), "DS_ARA_TABLES_FAULT": "1"},
        log="tables-fault.log",
    )
    row = None
    for line in reversed((text or "").strip().splitlines()):
        try:
            candidate = json.loads(line)
        except ValueError:
            continue
        if isinstance(candidate, dict) and candidate.get("event") == "tables-fault":
            row = candidate
            break
    return pass_fail(
        "tables-fault",
        rc == 0 and row is not None and row.get("detected") is True,
        f"the walk refused the emptied slot: rc={rc}, detected={row.get('detected') if row else None}, "
        f"error={row.get('error') if row else None} (out/tables-fault.log)",
    )


def stage_pluginval() -> bool:
    if not PLUGINVAL.exists() or not VST3.exists():
        return pass_fail("pluginval", False, f"no pluginval or no VST3 ({PLUGINVAL}, {VST3})")
    env = {**no_switch(), "DS_ARA_CACHE": str(OUT / "cache-pluginval")}
    rc, text = run(
        [str(PLUGINVAL), "--strictness-level", "5", "--timeout-ms", "120000", str(VST3)],
        env=env,
        log="pluginval.log",
    )
    ok = pass_fail("pluginval", rc == 0, f"strictness 5 on the wrapped VST3, rc={rc} (out/pluginval.log)")
    if not ok:
        return False
    # pluginval is no ARA host: no document controller, so the plugin never runs an
    # analysis here and writes no report. The proof that the scan drove THIS plugin is
    # the log: the wrapper's plugin comes from the CLAP descriptor (the inner CLAP).
    if "Drum Spike ARA" not in text:
        return pass_fail("pluginval", False, "pluginval passed but its log does not name the plugin (is the CLAP inside?)")
    return True


def stage_validate() -> bool:
    if not CLAP_VALIDATOR.exists() or not CLAP.exists():
        return pass_fail("validate", False, f"no clap-validator or no CLAP ({CLAP_VALIDATOR}, {CLAP})")
    env = {**no_switch(), "DS_ARA_CACHE": str(OUT / "cache-validate")}
    rc, _ = run([str(CLAP_VALIDATOR), "validate", str(CLAP)], env=env, log="clap-validator.log")
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
        "fixture-store", CLAP, ["store", "--files", *[str(FIXTURES / f) for f in FILES], "--archive", str(archive)], "cache-fixture-store"
    )
    if rc != 0 or not archive.exists():
        return pass_fail("make-fixture", False, f"ara-host store rc={rc} (out/host-fixture-store.log)")
    blob = json.loads(archive.read_text())
    blob["note"] = "Stored by plugins/ara check.py make-fixture; regenerate when the fixtures change."
    ok_paths = no_absolute_paths("make-fixture-paths", blob)
    ARCHIVE_FIXTURE.parent.mkdir(parents=True, exist_ok=True)
    ARCHIVE_FIXTURE.write_text(json.dumps(blob), encoding="utf-8")
    return pass_fail(
        "make-fixture",
        ok_paths and blob.get("version") == 1 and len(blob.get("sources", [])) == len(FILES),
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
    "tables": stage_tables,
    "tables-fault": stage_tables_fault,
    "identity-clap": lambda: stage_identity("clap", CLAP),
    "identity-vst3": lambda: stage_identity("vst3", EMBEDDED_CLAP),
    "regions": stage_regions,
    "archive": stage_archive,
    "archive-paths-fault": stage_archive_paths_fault,
    "archive-restore-pathname": stage_archive_restore_pathname,
    "threads": stage_threads,
    "doc-service": stage_doc_service,
    "doc-service-fault": stage_doc_service_fault,
    "doc-tracks": stage_doc_tracks,
    "doc-tracks-fault": stage_doc_tracks_fault,
    "doc-two": stage_doc_two,
    "doc-two-fault": stage_doc_two_fault,
    "doc-archive": stage_doc_archive,
    "doc-archive-fault": stage_doc_archive_fault,
    "doc-destroy": stage_doc_destroy,
    "doc-destroy-fault": stage_doc_destroy_fault,
    "doc-home": stage_doc_home,
    "doc-home-fault": stage_doc_home_fault,
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
    "fixture", "reference", "tables", "tables-fault", "identity-clap", "identity-vst3", "regions", "archive",
    "archive-paths-fault", "archive-restore-pathname",
    "threads", "doc-service", "doc-service-fault", "doc-tracks", "doc-tracks-fault", "doc-two", "doc-two-fault",
    "doc-archive", "doc-archive-fault", "doc-destroy", "doc-destroy-fault", "doc-home", "doc-home-fault",
    "conformance", "files", "pluginval", "validate", "universal", "standalone",
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
    print("M1A CHECK: PASS (tables, tables-fault, identity, regions, archive, notes, threads, document service, tracks, two documents, archive id, destroy, home, conformance, files, pluginval, clap-validator, universal, standalone, crate, workspace)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
