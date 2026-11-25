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
  doc-tracks     M1b1 line 2: four sources in two requests; the result of all four tracks is
                 byte for byte the result of one analysis of all four together (a fixture
                 whose split result differs: the snare in a tom); the host is told for each
                 earlier source whose notes changed (faults: the second request forgets the
                 earlier tracks; only the new tracks are analysed)
  doc-rounds     requests 40 ms apart are one round (fault: no settle time)
  doc-close      the destroy of a controller while its analysis runs: the call returns in
                 under 1 s, the round was running at the destroy (a round_start row before
                 it), it ends as cancelled within a stated time and writes no job row,
                 nothing is written after the document closed, the worker ends at the
                 unload of the library, the document on disk is consistent and a new
                 analysis in its folder works; a cancel that arrives before the analysis
                 starts ends the round without running it (doc-close-early) (faults: the
                 long wait, no cancel, no stop of the worker at the unload)
  doc-id-hostile an archive with an id that is no path-safe id (`../x`, an absolute path,
                 empty, other forms) or with track names that are paths: no file appears
                 outside documents/<valid id>/ (fault: the plugin trusts the archive)
  doc-same-id    the same archive restored into two controllers of one process: two ids,
                 two folders, both results equal to the reference (fault: no id registry)
  doc-restore-busy  restore, then store at once while the worker is busy with another
                 document: the second archive has every event (fault: the events are set on
                 the worker thread)
  doc-store-window  a store while a round publishes, and a store before the first round has
                 ended (F5, #572): the archive never holds the notes of a round without its
                 events (fault: the events come after the notes, as before)
  doc-restore-repair  an archive from that window (notes, no events) restores as sources with no
                 result and one round analyses them; a track analysed to
                 zero hits is a result (fault: the restore counts every source as done)
  doc-readd      a restored track is online after its audio is read, with its stored hits and settings, and
                 a track whose audio content differs is analysed again (F6, #582) (fault: the record of a
                 restored track is not pointed at the new cache file)
  doc-retry      a round that fails once, then a new request: one track per source, the
                 reference result (fault: a second track for every retried source)
  doc-rerequest  a request for sources that are complete starts no round (fault: a full
                 round with the reason removal)
  doc-errors     three tracks, one WAV damaged: the two good tracks equal the reference of
                 the two together, the bad one is reported with its error and has no notes,
                 the round is marked partial (fault: the error is not read)
  doc-regions-change  analyse, then change the second region: the events and the notes are
                 those of the reference for the new regions (fault: the change is not seen)
  doc-panic      a panic in the reader thread, and one in the worker thread: reported, the
                 worker goes on, a new request gives the reference result (fault: threads
                 without the guard)
  doc-source-names  host source names that end in `..`, are absolute or empty: 0 files outside
                 documents/<id>/ (fault: the plugin takes the host name as the file name)
  doc-region-move  a region moved in playback time only starts no round (fault: it does)
  doc-late-audio a source whose samples access never comes does not block the document: the
                 round goes on without it, names it and is partial (fault: wait for ever)
  doc-late-audio-arrives  the audio of the late source comes after the first round: a second
                 round runs, equal to the reference of both tracks (fault: no round)
  switches       one list (switches.json) of the DS_ARA_* variables: the source, check.sh
                 and the README use no other and the README table is made from it; a normal
                 run clears all of them; check.sh refuses one in the caller's environment
  no-unwrap      no unwrap and no expect in the non-test code of the plugin
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
  pluginval      pluginval 1.0.4 at strictness 10 on the wrapped VST3, the gate build and the
                 delivery build (no CLAP_PATH: the wrapper must find the CLAP inside its own
                 bundle); the fault run: a bundle without its CLAP, a log of another level
  bundle-version M1d1 line 2: CFBundleVersion and CFBundleShortVersionString of the CLAP, the VST3
                 and the CLAP inside it (both builds) are the version of Cargo.toml; the ARA
                 factory and the CLAP descriptor say the same (fault: the old wrapper plist)
  bundle-sign    M1d1 line 3: `codesign --verify --deep --strict` passes on the CLAP and on both
                 VST3 bundles, the signature is ad hoc and no linker signature, the seal covers
                 the page (fault: one changed page file fails the verify)
  delivery-switches  M1d1 line 4: the delivery build (no cargo feature test-switches) ignores
                 DS_ARA_ROUND_FAULT=request-only and gives the round; the gate build obeys it
                 (fault: the same check refuses the gate build)
  drag-source    round 3 of #579: no gate run starts a system drag session (a drop lands on the owner's desktop): the
                 source has one call of the system drag, in one function behind the test branch, and no posted event,
                 NSWorkspace or general pasteboard (fault: a second call is planted)
  export-dialog  M1c2 (#579): the REAL Export dialog in the REAL page, opened from the host-mode menu in two
                 editors of one document: no folder error, both drag tiles ready with a file count, a drag
                 starts, Export writes the files, a second open does the same, Choose keeps the folder for
                 the document (faults: the rule of M1c1, no memory of the chosen folder)
  export-rows    M1c4 (#589): the REAL Export dialog makes ONE file per DAW track and kind, byte-equal to the files of the handles: a track with clips
                 of three audio files, the merged MIDI, the scope Selected track, a chosen folder, an edit and Export at once (the dialog waits for
                 the new set), a set that never gets ready, forged rows, names and folders (faults: one file per audio file, a copy of a stale
                 set, any folder, a subfolder, a name from the page, any row id)
  deliver        M1c3 (#576): the files are made before the user asks: one MIDI, spike and audio spike file per DAW track with the hits of
                 all clips at their song positions, the merged MIDI file, states and the stale rule, the handles in the page, the cost of
                 an edit, a job that never ends, hostile track names, the audio sets gone after the last editor closed and back after the next
                 open (#599) (faults: one file per source, a stale take, every row again, one MIDI track per drum, no watchdog, the raw
                 track name, no removal, no new make)
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
import re
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
# the delivery variant (check.sh build): the same wrapper, the CLAP built without the cargo feature
# test-switches, so it reads no fault or test switch
DELIVERY_CLAP = DEPS / "delivery" / "DrumSpikeAra.clap"
DELIVERY_VST3 = DEPS / "delivery" / "DrumSpikeAra.vst3"
DELIVERY_EMBEDDED = DELIVERY_VST3 / "Contents" / "PlugIns" / "DrumSpikeAra.clap"
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


USER_DIR_NONE = "<none>"


def is_host_command(cmd: list) -> bool:
    """True for the ara-host binary however it is named: the same path, the same file after the links are resolved, or the same file name."""
    if not cmd:
        return False
    c = str(cmd[0])
    try:
        return c == str(HOST) or Path(c).resolve() == Path(HOST).resolve() or Path(c).name == Path(HOST).name
    except OSError:
        return Path(c).name == Path(HOST).name


def guard_user_dir(cmd: list, e: dict, log: str | None) -> None:
    """#577: a run never reaches the real folder of the user defaults. Every host process gets `DS_ARA_USER_DIR` below out/ (one folder per log name);
    the value USER_DIR_NONE leaves it unset (the stage `defaults-real-path`). A value outside out/ is refused for ANY command that carries it (the host,
    cargo, a script), however the host is started."""
    if is_host_command(cmd):
        if e.get("DS_ARA_USER_DIR") == USER_DIR_NONE:
            e.pop("DS_ARA_USER_DIR")
        elif not e.get("DS_ARA_USER_DIR"):
            e["DS_ARA_USER_DIR"] = str(OUT / ("user-" + (log or "run").replace("/", "-")))
    d = e.get("DS_ARA_USER_DIR")
    if d and not Path(d).resolve().is_relative_to(OUT.resolve()):
        raise AssertionError(f"a run would use {d} as the folder of the user defaults: outside {OUT}")


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
    guard_user_dir(cmd, e, log)
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


def load_switches() -> list[dict]:
    """switches.json: the one list of the DS_ARA_* variables (the README block, no_switch and
    the guard of check.sh are made from it)."""
    return json.loads((HERE / "switches.json").read_text(encoding="utf-8"))


def no_switch() -> dict:
    """The overlay that clears every DS_ARA_* variable the plugin reads (config, fault and
    scenario), so a normal run never inherits one."""
    return {e["name"]: None for e in load_switches() if e["kind"] != "check"}


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
    rounds = sv.get("rounds")
    ok = made == 1 and dropped == 1 and len(jobs) == 2 and rounds == 2 and len(docs) == 1
    return ok, (
        f"{len(jobs)} analysis requests in {len(docs)} document(s), {rounds} rounds: {made} service(s) made, {dropped} dropped "
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


# ---- gate line 2: all tracks of a document are analysed together

# The fixture whose split result differs from the result of all tracks together: two toms that
# hear the snare (a quieter copy 2 and 3 ms after each of its 8 hits). The tom bleed rule
# marks those hits (`bleed: true`, out of the output, so no ARA note) when the snare is
# analysed together with the toms, and cannot when it is not.
TOM1_FILE, TOM2_FILE, SNARE8_FILE = "tom1_m1b1_bleed.wav", "tom2_m1b1.wav", "snare_m1b1_8hits.wav"
SNARE8_HITS = [0.2, 0.5, 0.8, 1.1, 1.4, 1.7, 2.0, 2.3]
BLEED_FILES = [TOM1_FILE, TOM2_FILE, SNARE8_FILE, FILES[0]]  # two toms, then the snare and the kick: two requests of two


def ensure_bleed_fixture() -> None:
    FIXTURES.mkdir(parents=True, exist_ok=True)
    made = {
        SNARE8_FILE: ([(t, 200.0, 0.85) for t in SNARE8_HITS], []),
        TOM1_FILE: (
            [(0.35, 120.0, 0.9), (0.95, 120.0, 0.85), (1.55, 120.0, 0.9), (2.15, 120.0, 0.85)],
            [(t + 0.002, 200.0, 0.3) for t in SNARE8_HITS],
        ),
        TOM2_FILE: (
            [(0.65, 100.0, 0.9), (1.25, 100.0, 0.85), (1.85, 100.0, 0.9), (2.45, 100.0, 0.85)],
            [(t + 0.003, 200.0, 0.24) for t in SNARE8_HITS],
        ),
    }
    for name, (hits, bleed) in made.items():
        if not (FIXTURES / name).exists():
            make_wav(FIXTURES / name, 48000, 1, 3.0, hits, bleed)


def events_diff(a_json: str, b_json: str) -> tuple[int, int]:
    """(events that differ, events in all) between two events objects, track by track."""
    a, b = json.loads(a_json or "{}"), json.loads(b_json or "{}")
    differ, total = 0, 0
    for name in set(a) | set(b):
        ea, eb = a.get(name) or [], b.get(name) or []
        total += max(len(ea), len(eb))
        differ += sum(1 for x, y in zip(ea, eb) if x != y) + abs(len(ea) - len(eb))
    return differ, total


def notes_changed(lines: list, before: str, after: str) -> set:
    """The sources whose notes the host reads differently in the phase `after` than in `before`."""
    b, a = host_notes_of(lines, before), host_notes_of(lines, after)
    return {n for n in b if n in a and a[n] != b[n]}


def notified(lines: list, phase: str) -> set:
    return {r["name"] for r in lines if r.get("event") == "host_content_changed" and r.get("phase") == phase}


def check_tracks(label: str, cache: str, lines: list, files: list, ref: bytes) -> tuple[bool, str]:
    jobs = job_rows(label)
    docs0 = project_docs(cache)
    on_disk0 = len(tracks_of_doc(docs0[0])) if len(docs0) == 1 else 0
    if len(jobs) != 2:
        return False, f"{len(jobs)} job rows, want 2 (one per request); the document on disk has {on_disk0} tracks, want {len(files)}"
    second = jobs[1]
    t2 = second.get("track_json") or {}
    docs = project_docs(cache)
    doc_tracks = tracks_of_doc(docs[0]) if len(docs) == 1 else {}
    events = sum(len(json.loads(v)) for v in t2.values())
    same = second.get("events_json", "").encode() == ref
    differ, total = events_diff(second.get("events_json", ""), ref.decode())
    in_doc = bool(t2) and all(n in doc_tracks and doc_tracks[n].get("events") == json.loads(t2[n]) for n in t2)
    expected = notes_changed(lines, "round-1", "final")
    told = notified(lines, "round-2")
    ok = (
        same
        and list(t2) == track_names(files)
        and len(doc_tracks) == len(files)
        and len(second.get("analysed") or []) == len(files)
        and events > 0
        and in_doc
        and bool(expected)
        and told == expected
        and services_row(label).get("rounds") == 2
    )
    return ok, (
        f"{len(t2)} tracks and {events} events after the second request, byte for byte the reference of all four together: "
        f"{same} ({differ} of {total} events differ); round 2 analysed {len(second.get('analysed') or [])} tracks; "
        f"the document on disk has {len(doc_tracks)} tracks (events equal: {in_doc}); {services_row(label).get('rounds')} rounds; "
        f"sources whose notes changed {sorted(Path(n).stem for n in expected)}, told to the host {sorted(Path(n).stem for n in told)}"
    )


def bleed_reference() -> bytes | None:
    ensure_bleed_fixture()
    return reference_alone("m1b1-ref-bleed", BLEED_FILES)


def stage_doc_tracks() -> bool:
    if not HOST.exists() or not CLAP.exists() or not SERVICE_TOOL.exists():
        return pass_fail("doc-tracks", False, "no host, no CLAP or no service_tool: run check.sh build first")
    ref = bleed_reference()
    if ref is None:
        return pass_fail("doc-tracks", False, "the reference of the four tracks together failed")
    rc, lines = grow_run("doc-grow-bleed", BLEED_FILES, 2)
    if rc != 0:
        return pass_fail("doc-tracks", False, f"ara-host grow rc={rc} (out/host-doc-grow-bleed.log)")
    ok, evidence = check_tracks("doc-grow-bleed", "cache-doc-grow-bleed", lines, BLEED_FILES, ref)
    return pass_fail("doc-tracks", ok, evidence)


def stage_doc_tracks_fault() -> bool:
    """Two fault runs: the second request forgets the earlier tracks (the M1a way), and
    the round analyses only the new tracks again (the first version of M1b1). Both must be
    refused; the second one is the case the bleed fixture is made for."""
    if not HOST.exists() or not CLAP.exists() or not SERVICE_TOOL.exists():
        return pass_fail("doc-tracks-fault", False, "no host, no CLAP or no service_tool: run check.sh build first")
    ref = bleed_reference()
    if ref is None:
        return pass_fail("doc-tracks-fault", False, "the reference of the four tracks together failed")
    out = {}
    for name, env in (("forget", {"DS_ARA_TRACKS_FAULT": "forget"}), ("new-only", {"DS_ARA_ROUND_FAULT": "new-only"})):
        rc, lines = grow_run(f"doc-grow-bleed-{name}", BLEED_FILES, 2, extra=env)
        ok, evidence = check_tracks(f"doc-grow-bleed-{name}", f"cache-doc-grow-bleed-{name}", lines, BLEED_FILES, ref)
        out[name] = (rc == 0 and not ok, evidence)
    return pass_fail(
        "doc-tracks-fault",
        all(v[0] for v in out.values()),
        " | ".join(f"{k} switch refused: {v[0]}: {v[1]}" for k, v in out.items()),
    )


# ---- gate line 2b: requests that arrive close together are one round

def check_rounds(label: str, ref: bytes) -> tuple[bool, str]:
    jobs = job_rows(label)
    rounds = services_row(label).get("rounds")
    same = len(jobs) == 1 and jobs[0].get("events_json", "").encode() == ref
    return rounds == 1 and same, (
        f"{len(FILES)} requests 40 ms apart: {rounds} round(s) (want 1), {len(jobs)} job row(s), "
        f"events byte for byte the reference of all together: {same}"
    )


def stage_doc_rounds() -> bool:
    if not HOST.exists() or not CLAP.exists() or not (OUT / "events-reference.json").exists():
        return pass_fail("doc-rounds", False, "no host, no CLAP or no reference events: run check.sh build and the reference stage first")
    rc, _text, _lines = host_run(
        "doc-rounds",
        CLAP,
        ["grow", "--files", *fixture_args(FILES), "--batch", "1", "--gap-ms", "40", "--wait", HOST_WAIT],
        "cache-doc-rounds",
    )
    if rc != 0:
        return pass_fail("doc-rounds", False, f"ara-host grow rc={rc} (out/host-doc-rounds.log)")
    ok, evidence = check_rounds("doc-rounds", (OUT / "events-reference.json").read_bytes())
    return pass_fail("doc-rounds", ok, evidence)


def stage_doc_rounds_fault() -> bool:
    """The fault run: no settle time, every request starts a round."""
    if not HOST.exists() or not CLAP.exists() or not (OUT / "events-reference.json").exists():
        return pass_fail("doc-rounds-fault", False, "no host, no CLAP or no reference events")
    rc, _text, _lines = host_run(
        "doc-rounds-fault",
        CLAP,
        ["grow", "--files", *fixture_args(FILES), "--batch", "1", "--gap-ms", "40", "--wait", HOST_WAIT],
        "cache-doc-rounds-fault",
        extra={"DS_ARA_SETTLE_FAULT": "none"},
    )
    ok, evidence = check_rounds("doc-rounds-fault", (OUT / "events-reference.json").read_bytes())
    return pass_fail("doc-rounds-fault", rc == 0 and not ok, f"the fault switch gave every request its own round and the check refused it: {evidence}")


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


def check_two_parts(label: str, cache: str, lines: list, refs: list) -> tuple[dict, str]:
    """The lines of gate line 3, each on its own (the fault run must fail the line it breaks)."""
    by_doc: dict = {}
    for j in job_rows(label):
        by_doc.setdefault(j.get("doc"), []).append(j)
    if len(by_doc) != 2:
        return {"two": False}, f"{len(by_doc)} document(s) in the report, want 2"
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
    parts = {
        "two": True,
        "own": own,
        "same": all(same),
        "events": events > 0,
        "quiet": quiet,
        "two_tracks": two_tracks,
        "numbering": own_numbering,
    }
    return parts, (
        f"two documents, {events} events: own folders {own}, events byte for byte the reference of each document "
        f"alone {same}, no file of the first folder changed while the second worked ({len(state_a)} files, quiet {quiet}), "
        f"each project holds its 2 tracks {two_tracks}, own source numbering {own_numbering}"
    )


def check_two(label: str, cache: str, lines: list, refs: list) -> tuple[bool, str]:
    parts, evidence = check_two_parts(label, cache, lines, refs)
    return all(parts.values()), evidence


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
    parts, evidence = check_two_parts("doc-two-fault", "cache-doc-two-fault", lines, [ref_a, ref_b])
    # the expected failure line: the own-folders line (not any failure, such as a crash)
    refused = rc == 0 and parts.get("two") is True and parts.get("own") is False
    return pass_fail(
        "doc-two-fault",
        refused,
        f"the fault switch put both documents in one folder and the check refused it on the own-folders line (rc={rc}): {evidence}",
    )


# ---- gate line 4: the id in the archive, the same folder after a restore

_ARCHIVE_STORE: dict[str, Path] = {}


def archive_events(archive: Path) -> int | None:
    """The number of events an archive holds (all tracks); None when the archive cannot be read."""
    try:
        blob = json.loads(archive.read_text())
        return sum(len(v) for v in (blob.get("events") or {}).values())
    except (OSError, ValueError, AttributeError, TypeError):
        return None


def archive_store() -> tuple[bool, Path, str]:
    """Store the four-source session once (memoised): the archive, the cache folder stays."""
    archive = OUT / "doc-archive.araarchive"
    if "archive" not in _ARCHIVE_STORE:
        rc, _text, _lines = host_run(
            "doc-archive-store", CLAP, ["store", "--files", *fixture_args(FILES), "--archive", str(archive)], "cache-doc-archive"
        )
        if rc != 0 or not archive.exists():
            return False, archive, f"ara-host store rc={rc} (out/host-doc-archive-store.log)"
        held = archive_events(archive)
        if held is None:
            return False, archive, "the store archive cannot be read (not JSON, or not an archive): see out/host-doc-archive-store.log"
        if held == 0:
            # F5 (#572): name the defect here, not in the four stages that read this archive
            return False, archive, "the store archive holds no event: the store came in the window of a round that published its notes before its events"
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


# ---- gate line 5: a destroyed source leaves the project, the rest is analysed together again

DESTROY_FILES = [TOM1_FILE, TOM2_FILE, SNARE8_FILE]  # two toms and the snare they hear; the snare goes
DESTROY_REST = [TOM1_FILE, TOM2_FILE]


def check_destroy(label: str, cache: str, lines: list, ref_all: bytes, ref_rest: bytes) -> tuple[bool, str]:
    jobs = job_rows(label)
    docs0 = project_docs(cache)
    on_disk0 = len(tracks_of_doc(docs0[0])) if len(docs0) == 1 else 0
    first = [j for j in jobs if j.get("reason") == "request"]
    after = [j for j in jobs if j.get("reason") == "removal"]
    if len(first) != 1 or len(after) != 1:
        return False, (
            f"{len(first)} request round(s) and {len(after)} round(s) after the removal, want 1 and 1; "
            f"the document on disk has {on_disk0} tracks after the destroy, want 2"
        )
    names = track_names(DESTROY_FILES)
    left = [names[0], names[1]]
    docs = project_docs(cache)
    doc_tracks = tracks_of_doc(docs[0]) if len(docs) == 1 else {}
    destroyed = event_rows(label, "source_destroyed")
    row = destroyed[-1] if destroyed else {}
    rest = after[0]
    before_ok = first[0].get("events_json", "").encode() == ref_all
    rest_ok = rest.get("events_json", "").encode() == ref_rest
    differ, total = events_diff(first[0].get("events_json", ""), rest.get("events_json", ""))
    in_doc = bool(left) and all(
        n in doc_tracks and doc_tracks[n].get("events") == json.loads((rest.get("track_json") or {}).get(n, "null")) for n in left
    )
    wavs = sorted(p.name[:3] for d in (OUT / cache / "documents").glob("*/audio") for p in d.glob("*.wav"))
    expected = notes_changed(lines, "final", "after-destroy")
    told = notified(lines, "destroy")
    ok = (
        list(doc_tracks) == left
        and len(destroyed) == 1
        and row.get("sources_left") == 2
        and row.get("project_tracks") == left
        and before_ok
        and rest_ok
        and len(rest.get("analysed") or []) == 2
        and differ > 0
        and in_doc
        and wavs == ["000", "001"]
        and bool(expected)
        and told == expected
    )
    return ok, (
        f"3 sources analysed together (byte for byte the reference: {before_ok}), 1 destroyed: the plugin's source list holds "
        f"{row.get('sources_left')}, the project {row.get('project_tracks')}, the document on disk {list(doc_tracks)}; "
        f"the {len(rest.get('analysed') or [])} tracks that are left were analysed together again, byte for byte the reference of "
        f"the remaining set: {rest_ok} ({differ} of {total} events differ from before the destroy); document events equal {in_doc}; "
        f"cache audio of the destroyed source gone {wavs == ['000', '001']}; notes changed {sorted(Path(n).stem for n in expected)}, "
        f"told to the host {sorted(Path(n).stem for n in told)}"
    )


def destroy_refs() -> tuple[bytes, bytes] | None:
    ensure_bleed_fixture()
    all_, rest = reference_alone("m1b1-ref-three", DESTROY_FILES), reference_alone("m1b1-ref-rest", DESTROY_REST)
    return (all_, rest) if all_ is not None and rest is not None else None


def destroy_run(label: str, extra: dict | None = None) -> tuple[int, list]:
    if label not in _GROW:
        rc, _text, lines = host_run(
            label,
            CLAP,
            ["grow", "--files", *fixture_args(DESTROY_FILES), "--batch", "3", "--destroy", "2", "--pump-ms", "3000", "--wait", HOST_WAIT],
            f"cache-{label}",
            extra=extra,
        )
        _GROW[label] = (rc, lines)
    return _GROW[label]


def stage_doc_destroy() -> bool:
    if not HOST.exists() or not CLAP.exists() or not SERVICE_TOOL.exists():
        return pass_fail("doc-destroy", False, "no host, no CLAP or no service_tool: run check.sh build first")
    refs = destroy_refs()
    if refs is None:
        return pass_fail("doc-destroy", False, "a reference run failed")
    rc, lines = destroy_run("doc-destroy")
    if rc != 0:
        return pass_fail("doc-destroy", False, f"ara-host grow rc={rc} (out/host-doc-destroy.log)")
    ok, evidence = check_destroy("doc-destroy", "cache-doc-destroy", lines, *refs)
    return pass_fail("doc-destroy", ok, evidence)


def stage_doc_destroy_fault() -> bool:
    """Two fault runs: the plugin keeps the destroyed source (the M1a way), and it does not
    analyse the tracks that are left again. Both must be refused."""
    if not HOST.exists() or not CLAP.exists() or not SERVICE_TOOL.exists():
        return pass_fail("doc-destroy-fault", False, "no host, no CLAP or no service_tool: run check.sh build first")
    refs = destroy_refs()
    if refs is None:
        return pass_fail("doc-destroy-fault", False, "a reference run failed")
    out = {}
    for name, env in (("keep", {"DS_ARA_DESTROY_FAULT": "keep"}), ("no-reanalysis", {"DS_ARA_ROUND_FAULT": "new-only"})):
        rc, lines = destroy_run(f"doc-destroy-fault-{name}", env)
        ok, evidence = check_destroy(f"doc-destroy-fault-{name}", f"cache-doc-destroy-fault-{name}", lines, *refs)
        out[name] = (rc == 0 and not ok, evidence)
    return pass_fail(
        "doc-destroy-fault",
        all(v[0] for v in out.values()),
        " | ".join(f"{k} switch refused: {v[0]}: {v[1]}" for k, v in out.items()),
    )


# ---- finding 2 and review item 7: the destroy of a controller while its analysis runs

# Five dense sources of two minutes each: the analysis takes several seconds and a cancel
# takes effect between two tracks, so the round can end as cancelled soon after the destroy.
CLOSE_FILES = ["synth:120"] * 5
CLOSE_LIMIT_MS = 1000.0  # the destroy call on the host's main thread
CANCEL_LIMIT_MS = 3500.0  # from the destroy to the end of the round (one track is not cancelled in the middle)
CLOSE_AFTER_MS = 5000  # the host stays alive after the destroy: a late write would show now


def ns_of(row: dict) -> int:
    return int(row.get("unix_ns", 0))


def check_close(
    label: str, cache: str, rc: int, lines: list, text: str, deinit: bool = True, disk: bool = False
) -> tuple[dict, str]:
    """The lines of the close proof, each its own part (a fault run must fail the line it breaks):
    the destroy call is short; the analysis was running at the destroy (a round_start row before
    it, no end before it); the cancel took effect (the round ended as cancelled within the limit,
    no job row); nothing was written after the document closed; the worker ended at the unload;
    the document on disk is consistent and a new analysis in its folder works."""
    close = next((r for r in lines if r.get("event") == "host_close"), {})
    ms, at = close.get("ms"), ns_of(close)
    rows = report_rows(label)
    starts = [r for r in rows if r.get("event") == "round_start"]
    ends = [r for r in rows if r.get("event") == "round_end"]
    jobs = [r for r in rows if r.get("event") == "job"]
    closed = [r for r in rows if r.get("event") == "doc_closed"]
    unloaded = [r for r in rows if r.get("event") == "worker_unloaded"]
    running = bool(starts) and at > 0 and ns_of(starts[0]) < at and not [e for e in ends if ns_of(e) < at]
    cancelled_ends = [e for e in ends if e.get("outcome") == "cancelled" and ns_of(e) >= at]
    cancel_ms = (ns_of(cancelled_ends[0]) - at) / 1e6 if cancelled_ends else None
    cancelled = cancel_ms is not None and cancel_ms <= CANCEL_LIMIT_MS and not jobs
    root = OUT / cache
    time.sleep(0.2)
    files = [p for p in root.rglob("*") if p.is_file()] if root.is_dir() else []
    outside = [p for p in files if "documents" not in p.relative_to(root).parts]
    # nothing is written after the document closed: no file is newer than the doc_closed row
    late = [p.name for p in files if closed and p.stat().st_mtime_ns > ns_of(closed[-1]) + 5_000_000]
    quiet = bool(closed) and not late and not outside
    docs = project_docs(cache)
    consistent, reanalysed = False, "no project document"
    if not disk:
        consistent, reanalysed = True, "not measured in this run"
    elif len(docs) == 1:
        try:
            tracks = tracks_of_doc(docs[0])
            consistent = len(tracks) == len(CLOSE_FILES)
            ok_again, _events, n = service_tool_events(docs[0].parent, f"{label}-again")
            reanalysed = f"{n} events on a new analysis in the folder"
            consistent = consistent and ok_again
        except (ValueError, OSError) as e:
            reanalysed = f"the document is not readable: {e}"
    parts = {
        "short": ms is not None and ms < CLOSE_LIMIT_MS,
        "running": running,
        "cancelled": cancelled,
        "quiet": quiet,
        "clean": rc == 0 and "panicked" not in text,
        "unloaded": (not deinit) or any(r.get("ended") is True for r in unloaded),
        "consistent": consistent,
    }
    shutil.rmtree(root, ignore_errors=True)  # the long synthetic audio is not kept
    return parts, (
        f"destroy of the controller {ms} ms (limit {CLOSE_LIMIT_MS:.0f}); the round was running at the destroy {running} "
        f"({len(starts)} round_start row(s) before it); it ended as cancelled {cancel_ms} ms after it (limit {CANCEL_LIMIT_MS:.0f}), "
        f"{len(jobs)} job row(s); {len(late)} file(s) written after the document closed, {len(outside)} outside the documents folder; "
        f"worker ended at the unload {parts['unloaded']}; document on disk consistent {consistent} ({reanalysed}); "
        f"rc={rc} without a panic: {parts['clean']}"
    )


def close_run(label: str, extra: dict | None = None, early: bool = False, deinit: bool = True) -> tuple[int, list, str]:
    args = ["close-busy", "--files", *CLOSE_FILES, "--lead-ms", "900", "--after-ms", str(CLOSE_AFTER_MS)]
    if early:
        args = ["close-busy", "--files", *CLOSE_FILES[:3], "--lead-ms", "1300", "--after-ms", "3000"]
    if deinit:
        args.append("--deinit")
    rc, text, lines = host_run(label, CLAP, args, f"cache-{label}", extra=extra)
    return rc, lines, text


def stage_doc_close() -> bool:
    if not HOST.exists() or not CLAP.exists() or not SERVICE_TOOL.exists():
        return pass_fail("doc-close", False, "no host, no CLAP or no service_tool: run check.sh build first")
    rc, lines, text = close_run("doc-close")
    parts, evidence = check_close("doc-close", "cache-doc-close", rc, lines, text, disk=True)
    ok = pass_fail("doc-close", all(parts.values()), evidence)
    # a cancel that arrives after the round started its setup and before `analyze_start`: the
    # scenario `slow-setup` holds every round 1.5 s there; the round must end as cancelled
    # without running its analysis (no job row)
    rc, lines, text = close_run("doc-close-early", {"DS_ARA_SCENARIO": "slow-setup"}, early=True)
    parts, evidence = check_close("doc-close-early", "cache-doc-close-early", rc, lines, text)
    failed = [r for r in report_rows("doc-close-early") if r.get("event") == "job_failed"]
    never_started = len(failed) == 1 and "before it started" in str(failed[0].get("error"))
    early_ok = all(parts.values()) and never_started
    evidence += f"; the round ended with: {failed[0].get('error') if failed else None!r} (never started its analysis: {never_started})"
    return pass_fail("doc-close-early", early_ok, evidence) and ok


def stage_doc_close_fault() -> bool:
    """Three fault runs, each refused on its own line: the destroy waits for the round (the
    first version: the call is long), the destroy does not cancel the round (it runs on to its
    end behind the close: no cancelled end, a job row after the destroy), and the unload of
    the library leaves the worker running."""
    if not HOST.exists() or not CLAP.exists() or not SERVICE_TOOL.exists():
        return pass_fail("doc-close-fault", False, "no host, no CLAP or no service_tool: run check.sh build first")
    out = {}
    # (the unload cancels every round too, so the run that proves "no cancel" has no unload)
    for name, env, line, deinit in (
        ("wait", {"DS_ARA_CLOSE_FAULT": "wait"}, "short", True),
        ("nocancel", {"DS_ARA_CLOSE_FAULT": "nocancel"}, "cancelled", False),
        ("unload", {"DS_ARA_UNLOAD_FAULT": "skip"}, "unloaded", True),
    ):
        rc, lines, text = close_run(f"doc-close-fault-{name}", env, deinit=deinit)
        parts, evidence = check_close(f"doc-close-fault-{name}", f"cache-doc-close-fault-{name}", rc, lines, text, deinit=deinit)
        out[name] = (parts.get(line) is False, f"{line} line: {evidence}")
    return pass_fail(
        "doc-close-fault",
        all(v[0] for v in out.values()),
        " | ".join(f"{k} switch refused on its line: {v[0]}: {v[1]}" for k, v in out.items()),
    )


# ---- gate line 6: the service home is inside the cache folder, nothing else below the user's home

HOME_FILES = FILES[:2]


def check_home(label: str, home: Path, cache: Path) -> tuple[bool, str]:
    """The files written below `home` outside `cache`: 0. The files inside the cache and
    the service home inside it (with its recent list) make the check non-vacuous."""
    below = [p for p in home.rglob("*") if p.is_file()] if home.is_dir() else []
    allowed = home / "Library" / "Application Support" / "DrumSpike ARA"   # #577: the one other folder the plugin may write (the first Save only)
    outside = [p for p in below if cache not in p.parents and allowed not in p.parents]
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


# ---- review round 2, item 1: a hostile or damaged archive never makes the plugin write outside its folder

HEX_ID = re.compile(r"^[0-9a-f]{16}$")
HOSTILE_IDS = ["../x", "../../escape-rel", "ABSOLUTE", "", "0123456789ABCDEF"]
# the variants that must reach outside when the plugin trusts the id
ESCAPING_IDS = ["../../escape-rel", "ABSOLUTE"]


def hostile_blob(base: dict, variant: str, root: Path) -> dict:
    """A copy of a real stored archive with a bad document id (or bad track names)."""
    blob = json.loads(json.dumps(base))
    if variant == "tracks":
        first = blob["sources"][0]
        blob["events"]["../../escape-evkey"] = []
        first["track"] = "../../escape-track"
        return blob
    blob["doc_id"] = str(root / "escape-abs") if variant == "ABSOLUTE" else variant
    return blob


def hostile_run(label: str, base: dict, variant: str, extra: dict | None = None) -> tuple[dict, str]:
    """One restore of a hostile archive. The cache folder sits one level down in a folder of its
    own, so a path that leaves `documents/<id>/` lands in that folder and is listed. Returns the
    parts (each must hold) and the evidence."""
    root = OUT / f"hostile-{label}"
    if root.exists():
        shutil.rmtree(root)
    root.mkdir(parents=True)
    archive = OUT / f"hostile-{label}.araarchive"
    archive.write_text(json.dumps(hostile_blob(base, variant, root)), encoding="utf-8")
    rc, _text, _lines = host_run(
        label, CLAP, ["restore", "--files", *fixture_args(FILES), "--archive", str(archive)], f"hostile-{label}/cache", extra=extra
    )
    cache = root / "cache"
    outside = sorted(p.name for p in root.iterdir() if p.name != "cache")
    docs = sorted(d.name for d in (cache / "documents").iterdir()) if (cache / "documents").is_dir() else []
    rows = event_rows(label, "doc_restored")
    row = rows[0] if len(rows) == 1 else {}
    folder = Path(row.get("folder", ""))
    tracks = row.get("project_tracks") or []
    parts = {
        "rc": rc == 0 and len(rows) == 1,
        "own_id": bool(HEX_ID.match(str(row.get("doc", "")))),
        "no_escape": not outside,
        "documents": bool(docs) and all(HEX_ID.match(d) for d in docs),
        "folder": folder.parent == cache / "documents" and folder.name == row.get("doc"),
        "tracks": len(tracks) == len(FILES) and all("/" not in t and ".." not in t for t in tracks),
        "events": sum(len(v) for v in json.loads(row.get("events_json") or "{}").values()) > 0,
    }
    refused = [r.get("event") for r in report_rows(label) if r.get("event", "").endswith(("_refused", "_taken"))]
    return parts, (
        f"{variant!r}: rc={rc}, restored into {row.get('doc')!r}, outside the cache folder: {outside}, "
        f"documents {docs}, tracks {tracks[:2]}, report rows {refused}"
    )


def stage_doc_id_hostile() -> bool:
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("doc-id-hostile", False, "no host or no CLAP: run check.sh build first")
    ok_store, archive, why = archive_store()
    if not ok_store:
        return pass_fail("doc-id-hostile", False, why)
    base = json.loads(archive.read_text())
    ok, notes = True, []
    for k, variant in enumerate([*HOSTILE_IDS, "tracks"]):
        parts, evidence = hostile_run(f"doc-id-{k}", base, variant)
        if variant == "tracks" and not [r for r in report_rows(f"doc-id-{k}") if r.get("event") == "restore_refused"]:
            parts["refused_row"] = False
        good = all(parts.values())
        ok = ok and good
        notes.append(f"{'ok' if good else 'BAD ' + str([n for n, v in parts.items() if not v])} {evidence}")
    return pass_fail("doc-id-hostile", ok, "; ".join(notes))


def stage_doc_id_hostile_fault() -> bool:
    """The fault run: the plugin trusts the id and the names of the archive. The check must see
    a file outside `documents/<valid id>/` for the ids that leave it."""
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("doc-id-hostile-fault", False, "no host or no CLAP: run check.sh build first")
    ok_store, archive, why = archive_store()
    if not ok_store:
        return pass_fail("doc-id-hostile-fault", False, why)
    base = json.loads(archive.read_text())
    seen = {}
    for k, variant in enumerate(ESCAPING_IDS):
        parts, evidence = hostile_run(f"doc-id-fault-{k}", base, variant, extra={"DS_ARA_ID_FAULT": "trust"})
        seen[variant] = (parts["no_escape"] is False, evidence)
    return pass_fail(
        "doc-id-hostile-fault",
        all(v[0] for v in seen.values()),
        "the fault switch trusted the id and the check saw the file outside the folder: "
        + " | ".join(f"{k}: {v[0]}: {v[1]}" for k, v in seen.items()),
    )


# ---- review round 2, item 2: two live documents never share an id, so never a folder


def same_id_parts(label: str, cache: str, archive: Path) -> tuple[dict, str]:
    blob = json.loads(archive.read_text())
    want = (OUT / "events-reference.json").read_bytes().decode()
    rows = event_rows(label, "doc_restored")
    docs = {r.get("doc") for r in rows}
    folders = {r.get("folder") for r in rows}
    on_disk = project_docs(cache)
    parts = {
        "two_rows": len(rows) == 2,
        "own_folders": len(docs) == 2 and len(folders) == 2 and len(on_disk) == 2,
        "first_keeps_id": bool(rows) and rows[0].get("doc") == blob.get("doc_id"),
        "taken_row": any(r.get("event") == "doc_id_taken" for r in report_rows(label)),
        "both_equal_reference": len(rows) == 2 and all(r.get("events_json") == want for r in rows),
    }
    return parts, (
        f"the same archive restored into two controllers of one process: {len(rows)} restore row(s), {len(docs)} document id(s), "
        f"{len(folders)} folder(s), {len(on_disk)} project(s) on disk; the first kept the stored id {parts['first_keeps_id']}; "
        f"the second was told the id is taken {parts['taken_row']}; both results byte for byte the reference {parts['both_equal_reference']}"
    )


def same_id_run(label: str, archive: Path, extra: dict | None = None) -> int:
    rc, _text, _lines = host_run(
        label, CLAP, ["restore2", "--files", *fixture_args(FILES), "--archive", str(archive)], f"cache-{label}", extra=extra
    )
    return rc


def stage_doc_same_id() -> bool:
    if not HOST.exists() or not CLAP.exists() or not (OUT / "events-reference.json").exists():
        return pass_fail("doc-same-id", False, "no host, no CLAP or no reference events")
    ok_store, archive, why = archive_store()
    if not ok_store:
        return pass_fail("doc-same-id", False, why)
    rc = same_id_run("doc-same-id", archive)
    if rc != 0:
        return pass_fail("doc-same-id", False, f"ara-host restore2 rc={rc} (out/host-doc-same-id.log)")
    parts, evidence = same_id_parts("doc-same-id", "cache-doc-same-id", archive)
    return pass_fail("doc-same-id", all(parts.values()), evidence)


def stage_doc_same_id_fault() -> bool:
    if not HOST.exists() or not CLAP.exists() or not (OUT / "events-reference.json").exists():
        return pass_fail("doc-same-id-fault", False, "no host, no CLAP or no reference events")
    ok_store, archive, why = archive_store()
    if not ok_store:
        return pass_fail("doc-same-id-fault", False, why)
    rc = same_id_run("doc-same-id-fault", archive, {"DS_ARA_REGISTRY_FAULT": "off"})
    parts, evidence = same_id_parts("doc-same-id-fault", "cache-doc-same-id-fault", archive)
    return pass_fail(
        "doc-same-id-fault",
        rc == 0 and parts["own_folders"] is False,
        f"the fault switch (no registry of the ids in use) gave both documents one folder and the check refused it: {evidence}",
    )


# ---- review round 2, item 3: an archive stored right after a restore, while the worker is busy


def restore_busy_parts(label: str, rc: int, lines: list, archive_in: Path, archive_out: Path) -> tuple[dict, str]:
    first = json.loads(archive_in.read_text())
    got = json.loads(archive_out.read_text()) if archive_out.exists() else {}
    rows = report_rows(label)
    restore = next((r for r in lines if r.get("event") == "host_restore"), {})
    start = next((r for r in rows if r.get("event") == "round_start"), {})
    ends_before = [r for r in rows if r.get("event") == "round_end" and ns_of(r) < ns_of(restore)]
    n_first = sum(len(v) for v in (first.get("events") or {}).values())
    n_got = sum(len(v) for v in (got.get("events") or {}).values())
    parts = {
        "rc": rc == 0 and bool(got),
        "busy": bool(start) and ns_of(start) < ns_of(restore) and not ends_before,
        "events": n_got > 0 and got.get("events") == first.get("events"),
        "id": got.get("doc_id") == first.get("doc_id"),
        "notes": [s.get("notes") for s in got.get("sources", [])] == [s.get("notes") for s in first.get("sources", [])],
    }
    return parts, (
        f"restore, then store at once, while another document's round runs on the worker (round started "
        f"{(ns_of(restore) - ns_of(start)) / 1e6:.0f} ms before the restore, {len(ends_before)} round end(s) before it): "
        f"the second archive holds {n_got} events (the first {n_first}), equal {parts['events']}, same document id {parts['id']}, same notes {parts['notes']}"
    )


def restore_busy_run(label: str, archive: Path, extra: dict | None = None) -> tuple[int, list, Path]:
    out = OUT / f"{label}.araarchive"
    if out.exists():
        out.unlink()
    rc, _text, lines = host_run(
        label,
        CLAP,
        ["restore-busy", "--files", *fixture_args(FILES), "--archive", str(archive), "--out", str(out), "--busy", *["synth:200"] * 3],
        f"cache-{label}",
        extra=extra,
    )
    return rc, lines, out


def stage_doc_restore_busy() -> bool:
    if not HOST.exists() or not CLAP.exists() or not (OUT / "events-reference.json").exists():
        return pass_fail("doc-restore-busy", False, "no host, no CLAP or no reference events")
    ok_store, archive, why = archive_store()
    if not ok_store:
        return pass_fail("doc-restore-busy", False, why)
    rc, lines, out = restore_busy_run("doc-restore-busy", archive)
    parts, evidence = restore_busy_parts("doc-restore-busy", rc, lines, archive, out)
    ok = pass_fail("doc-restore-busy", all(parts.values()), evidence)
    # the second archive restores in a NEW process: every track, byte for byte
    if out.exists():
        rc2, _text, _lines = host_run(
            "doc-restore-busy-again", CLAP, ["restore", "--files", *fixture_args(FILES), "--archive", str(out)], "cache-doc-restore-busy-again"
        )
        rows = event_rows("doc-restore-busy-again", "doc_restored")
        row = rows[0] if len(rows) == 1 else {}
        want = (OUT / "events-reference.json").read_bytes().decode()
        again = rc2 == 0 and row.get("events_json") == want and len(row.get("project_tracks") or []) == len(FILES)
    else:
        again = False
    return pass_fail("doc-restore-busy-again", again, "the second archive restored in a new process: every track and every event, byte for byte the reference") and ok


def stage_doc_restore_busy_fault() -> bool:
    """The fault run: the events of the document are set by the worker (the first version). The
    archive stored in the window carries none."""
    if not HOST.exists() or not CLAP.exists() or not (OUT / "events-reference.json").exists():
        return pass_fail("doc-restore-busy-fault", False, "no host, no CLAP or no reference events")
    ok_store, archive, why = archive_store()
    if not ok_store:
        return pass_fail("doc-restore-busy-fault", False, why)
    rc, lines, out = restore_busy_run("doc-restore-busy-fault", archive, {"DS_ARA_RESTORE_FAULT": "late-events"})
    parts, evidence = restore_busy_parts("doc-restore-busy-fault", rc, lines, archive, out)
    return pass_fail(
        "doc-restore-busy-fault",
        parts["rc"] and parts["busy"] and parts["events"] is False,
        f"the fault switch set the events on the worker thread and the archive stored in the window lost them: {evidence}",
    )


# ---- F5 (#572): a store that the host asks for while a round publishes, or before the first round has ended


def store_window_parts(archive: Path) -> tuple[dict, str]:
    """What an archive stored in the publication of a round holds: with the notes of the sources it
    must hold their events (the round publishes both in one step)."""
    blob = json.loads(archive.read_text()) if archive.exists() else {}
    want = json.loads((OUT / "events-reference.json").read_text())
    got = blob.get("events") or {}
    n_notes = sum(len(s.get("notes") or []) for s in blob.get("sources", []))
    n_events = sum(len(v) for v in got.values())
    parts = {
        "notes": n_notes > 0,
        "events": got == want and n_events > 0,
        "tracks": sorted(s.get("track") for s in blob.get("sources", [])) == sorted(want),
    }
    return parts, f"the archive holds {n_notes} note(s) and {n_events} event(s) in {len(got)} track(s) (the reference: {sum(len(v) for v in want.values())} in {len(want)}), equal {parts['events']}"


def store_window_run(label: str, fault: str) -> tuple[int, Path]:
    out = OUT / f"{label}.araarchive"
    if out.exists():
        out.unlink()
    rc, _text, _lines = host_run(
        label, CLAP, ["store", "--files", *fixture_args(FILES), "--archive", str(out)], f"cache-{label}", extra={"DS_ARA_ROUND_FAULT": fault}
    )
    return rc, out


def restore_run_facts(label: str, archive: Path, extra: dict | None = None) -> dict:
    """One restore in a NEW process with the audio read and no request (`--enable-access`): what the report and
    the host say. `healthy`: the run ended with rc 0 and the restore row and the notes of every source are there,
    so a crash cannot pass as a refusal of the check."""
    rc, _text, lines = host_run(
        label,
        CLAP,
        ["restore", "--files", *fixture_args(FILES), "--archive", str(archive), "--enable-access"],
        f"cache-{label}",
        extra=extra,
    )
    counts = [r.get("count") for r in lines if r.get("event") == "host_notes"]
    jobs = job_rows(label)
    restored = event_rows(label, "doc_restored")
    return {
        "rc": rc,
        "counts": counts,
        "jobs": jobs,
        "restored": restored,
        "no_result": len(event_rows(label, "restore_no_result")),
        "healthy": rc == 0 and len(counts) == len(FILES) and len(restored) == 1,
    }


def early_restore_check(label: str, early: Path, late: Path, fault: str | None = None) -> tuple[bool, str, dict]:
    """The early archive (no result for any source) restored in a NEW process, the host reads the audio and
    never asks: the sources have no result, so one round analyses them and the notes are those of the late
    archive. (Fault `result`: the restore counts every archived source as done, so nothing is analysed.)"""
    f = restore_run_facts(label, early, {"DS_ARA_RESTORE_FAULT": fault} if fault else None)
    want = json.loads(late.read_text()) if late.exists() else {}
    want_counts = [len(s.get("notes") or []) for s in want.get("sources", [])]
    reference = (OUT / "events-reference.json").read_bytes().decode()
    same = bool(f["jobs"]) and f["jobs"][-1].get("events_json") == reference
    ok = f["healthy"] and bool(want_counts) and f["counts"] == want_counts and same
    return ok, (
        f"restored in a new process with the audio read (rc {f['rc']}): {f['no_result']} source(s) with no result, "
        f"{len(f['jobs'])} round(s), notes per source {f['counts']} (want {want_counts}), events byte for byte the reference {same}"
    ), f


def stage_doc_store_early_fault() -> bool:
    """The fault run: the restore counts a source of the early archive as done (the old way). No round
    analyses it, and the restored document shows no hit."""
    if not HOST.exists() or not CLAP.exists() or not (OUT / "events-reference.json").exists():
        return pass_fail("doc-store-early-fault", False, "no host, no CLAP or no reference events")
    early, late = OUT / "doc-store-early.araarchive", OUT / "doc-store-late.araarchive"
    if not early.exists() or not late.exists():
        stage_doc_store_window()
    ok, evidence, facts = early_restore_check("doc-store-early-restore-fault", early, late, "result")
    return pass_fail(
        "doc-store-early-fault",
        facts["healthy"] and not ok and not facts["jobs"] and facts["no_result"] == 0,
        f"the fault switch counted every archived source as done and the check refused it (a healthy run, no round): {evidence}",
    )


def stage_doc_restore_repair() -> bool:
    """An archive from the window of the old order (all notes, a track name for each source, no events) and one
    with a track analysed to zero hits. (1) The old archive: every source has no result, one round analyses
    them, the events and notes equal the reference. (2) The zero-hit track
    has its events key with an empty list: it is a result, no round runs for it."""
    if not HOST.exists() or not CLAP.exists() or not (OUT / "events-reference.json").exists():
        return pass_fail("doc-restore-repair", False, "no host, no CLAP or no reference events")
    old, late = OUT / "doc-restore-repair-old.araarchive", OUT / "doc-restore-repair-late.araarchive"
    for f in (old, late):
        if f.exists():
            f.unlink()
    rc, _text, _lines = host_run(
        "doc-restore-repair-store", CLAP, ["store", "--files", *fixture_args(FILES), "--archive", str(old)], "cache-doc-restore-repair-store",
        extra={"DS_ARA_ROUND_FAULT": "events-late"},
    )
    good = False
    blob: dict = {}
    if rc == 0 and old.exists():
        blob = json.loads(old.read_text())
        good = archive_events(old) == 0 and all(s.get("notes") and s.get("track") for s in blob.get("sources", []))
    if not good:
        return pass_fail("doc-restore-repair", False, f"the old-order archive was not made (rc={rc}, notes and track names with no events wanted)")
    # the late archive of a normal store: the notes to compare with
    rc_l, _t, _l = host_run("doc-restore-repair-late", CLAP, ["store", "--files", *fixture_args(FILES), "--archive", str(late)], "cache-doc-restore-repair-late")
    ok1, ev1, f1 = early_restore_check("doc-restore-repair-old", old, late)
    ok1 = ok1 and rc_l == 0 and f1["no_result"] == len(FILES)
    # (2) a good archive with one track analysed to zero hits
    good_blob = json.loads(late.read_text())
    zero = good_blob["sources"][1]["track"]
    good_blob["events"][zero] = []
    good_blob["sources"][1]["notes"] = []
    zero_archive = OUT / "doc-restore-repair-zero.araarchive"
    zero_archive.write_text(json.dumps(good_blob))
    f2 = restore_run_facts("doc-restore-repair-zero", zero_archive)
    kept = json.loads(f2["restored"][0].get("events_json") or "{}") if f2["restored"] else {}
    ok2 = f2["healthy"] and f2["no_result"] == 0 and not f2["jobs"] and kept.get(zero) == [] and f2["counts"][1] == 0
    return pass_fail(
        "doc-restore-repair",
        ok1 and ok2,
        f"old-order archive: {ev1}; zero-hit track {zero!r}: "
        f"{f2['no_result']} no-result row(s), {len(f2['jobs'])} round(s), events key kept as an empty list {kept.get(zero) == []}",
    )


def stage_doc_restore_repair_fault() -> bool:
    """The fault run: the restore counts the sources of the old-order archive as done (the way before #572)."""
    if not HOST.exists() or not CLAP.exists() or not (OUT / "events-reference.json").exists():
        return pass_fail("doc-restore-repair-fault", False, "no host, no CLAP or no reference events")
    old = OUT / "doc-restore-repair-old.araarchive"
    late = OUT / "doc-restore-repair-late.araarchive"
    if not old.exists() or not late.exists():
        stage_doc_restore_repair()
    ok, evidence, facts = early_restore_check("doc-restore-repair-fault-run", old, late, "result")
    return pass_fail(
        "doc-restore-repair-fault",
        facts["healthy"] and not ok and not facts["jobs"],
        f"the fault switch counted every archived source as done and the check refused it (a healthy run, no round): {evidence}",
    )


# ---- F6 (#582): a restored track is online after its audio is read

READD_SOURCE = 2  # the source of the archive that the runs restore alone (a track with hits)


def readd_archive(name: str, edit) -> Path:
    """A copy of the stored four-source archive with `edit(blob)` applied (the plugin's own archive, not a made-up one)."""
    ok, archive, why = archive_store()
    if not ok:
        raise RuntimeError(why)
    blob = json.loads(archive.read_text())
    edit(blob)
    out = OUT / f"doc-readd-{name}.araarchive"
    out.write_text(json.dumps(blob))
    return out


def readd_disk(cache: str) -> dict:
    """Track -> (size of the source record, size of the file it names): what the project document on disk says."""
    out: dict = {}
    for doc in project_docs(cache):
        for name, t in tracks_of_doc(doc).items():
            src = t.get("source") or ((t.get("segments") or [{}])[0].get("source")) or {}
            path = Path(src.get("path") or "")
            out[name] = (src.get("size"), path.stat().st_size if path.is_file() else None, bool(src.get("head_sha1")))
    return out


def readd_run(label: str, archive: Path, files: list, env: dict | None = None, by_name: bool = False) -> dict:
    """One restore in a NEW process with a fresh cache, the host reads the audio of every source it was given
    (the DAW sequence of F6: the cache folder has no file of the source when the document is restored)."""
    rc, _text, lines = host_run(
        label, CLAP, ["restore", "--files", *fixture_args(files), "--archive", str(archive), "--enable-access", *(["--no-persistent-id"] if by_name else [])],
        f"cache-{label}", extra=env,
    )
    restored = event_rows(label, "doc_restored")
    repairs = event_rows(label, "track_repair")
    rows = [t for r in repairs for t in r.get("tracks", [])]
    disk = readd_disk(f"cache-{label}")
    return {
        "rc": rc,
        "healthy": rc == 0 and len(restored) == 1,
        "tracks": restored[0].get("project_tracks") if restored else None,
        "repairs": rows,
        "rounds": len(event_rows(label, "round_start")),
        "jobs": job_rows(label),
        "differs": [r for r in event_rows(label, "restore_audio") if r.get("same") is False],
        "disk": disk,
        "online": bool(disk) and all(sz is not None and sz == real and h for sz, real, h in disk.values()),
    }


def readd_same(f: dict, tracks: list, events: dict) -> tuple[bool, str]:
    """The restored tracks are repaired, online on disk, kept their hits and started no round."""
    got = {r["track"]: r for r in f["repairs"]}
    kept = all(t in got and json.loads(got[t]["events_json"]) == events[t] for t in tracks)
    after = all(t in got and got[t]["after"] == "ok" for t in tracks)
    moved = all(t in got and got[t]["repointed"] for t in tracks)
    ok = f["healthy"] and sorted(f["tracks"] or []) == sorted(tracks) and kept and after and moved and f["rounds"] == 0 and f["online"] and not f["differs"]
    return ok, (
        f"rc {f['rc']}, tracks {f['tracks']}, {len(got)} repair row(s) (status after: {[r['after'] for r in got.values()]}, "
        f"before: {[r['before'] for r in got.values()]}), hits kept {kept}, {f['rounds']} round(s), "
        f"project document online on disk {f['online']} {f['disk'] and sorted(f['disk'].values(), key=str)[:1]}"
    )


def readd_unknown_parts(label: str, env: dict | None) -> list:
    """Parts of the M1 rule: (name, ok, evidence, run facts)."""
    ok_store, archive, why = archive_store()
    blob = json.loads(archive.read_text())
    src = blob["sources"][READD_SOURCE]
    track = src["track"]
    kick = FILES[0]
    twin = "m1a_twin_96k_mono.wav"  # the length of the tom, other hits
    make_wav(FIXTURES / twin, 96000, 1, 3.0, [(0.30, 90.0, 0.9), (1.10, 90.0, 0.85), (2.20, 90.0, 0.9)], [])
    out = []

    def one(pname: str, edit, files: list, what: str):
        a = readd_archive(pname, edit)
        f = readd_run(f"{label}-{pname}", a, files, env, by_name=True)
        job = f["jobs"][-1] if f["jobs"] else {}
        ok = (
            f["healthy"] and len(f["jobs"]) == 1 and job.get("analysed") == [track] and not job.get("errors")
            and not job.get("skipped") and not f["repairs"] and f["online"]
        )
        out.append((pname, ok, f"{what}: {len(f['jobs'])} round(s) analysing {job.get('analysed')} (want 1 for {[track]}), "
                    f"{len(f['repairs'])} repair row(s) (want 0), project document online on disk {f['online']}", f))

    def other_length(b):
        s = b["sources"][READD_SOURCE]
        s.pop("content_hash", None)
        b["sources"][0]["name"] = "unused.wav"  # the archive matches the first source of that name: the kick entry must not
        s["name"] = kick  # the kick is 1.5 times shorter than the tom of the archive: the stored count differs

    def no_count_same_length(b):
        s = b["sources"][READD_SOURCE]
        s.pop("content_hash", None)
        s.pop("sample_count", None)
        s["name"] = twin  # the length of the tom, so only the content is other

    one("oldother-len", other_length, [kick], "an old archive (no hash) and a file of another length")
    one("oldother-nocount", no_count_same_length, [twin], "an old archive (no hash, no sample count) and other audio of the same length")
    return out


def stage_doc_readd(fault: str | None = None) -> bool:
    """F6 (#582): sources A and B were analysed and stored, every source was destroyed and the document closed, a new
    document is restored from the archive and the audio is read again. The restored project record of a track holds no
    file identity (the cache file did not exist), so the service called the track offline (changed) for ever and no
    round ran. Now: (1) all four sources: every track online, its stored hits, no round; (2) one source alone
    (the others are gone; the host gives no persistent ID, so the archive matches it by name; its index in the document is not its index in the archive): the same; (3) hits and settings that the user changed
    in the archive survive; (4) an archive without a content hash (an earlier plugin) is treated as the same audio;
    (5) another content hash: one round analyses the track, which ends online."""
    label = "doc-readd" + ("-fault" if fault else "")
    if not HOST.exists() or not CLAP.exists():
        return pass_fail(label, False, "no host or no CLAP: run check.sh build first")
    ok_store, archive, why = archive_store()
    if not ok_store:
        return pass_fail(label, False, why)
    blob = json.loads(archive.read_text())
    srcs = blob["sources"]
    events = blob["events"]
    track = srcs[READD_SOURCE]["track"]
    hashed = all(len(s.get("content_hash") or "") == 16 for s in srcs)
    env = {"DS_ARA_RESTORE_FAULT": fault} if fault else None
    parts = {}
    parts["archive-hash"] = (hashed and len(events.get(track) or []) > 0, f"the stored archive carries a content hash for every source: {hashed}")
    runs: list = []
    f = readd_run(f"{label}-all", archive, FILES, env)
    runs.append(f)
    parts["all"] = readd_same(f, [s["track"] for s in srcs], events)
    healthy = f["healthy"]
    offline_on_disk = not f["online"]
    f = readd_run(f"{label}-one", archive, [FILES[READD_SOURCE]], env, by_name=True)
    runs.append(f)
    parts["one"] = readd_same(f, [track], events)
    healthy = healthy and f["healthy"]
    offline_on_disk = offline_on_disk and not f["online"]

    def hand(b):
        b["events"][track][0] = {**b["events"][track][0], "v": 7}
        b["sources"][READD_SOURCE]["settings"] = {**b["sources"][READD_SOURCE].get("settings", {}), "role": "cymbal", "note": 70, "role_source": "user"}

    edited = readd_archive("hand", hand)
    eblob = json.loads(edited.read_text())
    f = readd_run(f"{label}-hand", edited, [FILES[READD_SOURCE]], env, by_name=True)
    ok, ev = readd_same(f, [track], eblob["events"])
    got = {r["track"]: r for r in f["repairs"]}.get(track, {})
    parts["hand"] = (ok and got.get("note") == 70 and got.get("role") == "cymbal", f"hand edits and settings: {ev}; role {got.get('role')!r} note {got.get('note')} (want cymbal, 70)")

    def nohash(b):
        for s in b["sources"]:
            s.pop("content_hash", None)

    old = readd_archive("nohash", nohash)
    f = readd_run(f"{label}-nohash", old, [FILES[READD_SOURCE]], env, by_name=True)
    ok, ev = readd_same(f, [track], events)
    parts["nohash"] = (ok, f"an archive with no content hash: {ev}")

    def other(b):
        b["sources"][READD_SOURCE]["content_hash"] = "0000000000000001"

    changed = readd_archive("other", other)
    f = readd_run(f"{label}-other", changed, [FILES[READD_SOURCE]], env, by_name=True)
    job = f["jobs"][-1] if f["jobs"] else {}
    parts["other"] = (
        f["healthy"] and len(f["differs"]) == 1 and len(f["jobs"]) == 1 and job.get("analysed") == [track]
        and not job.get("errors") and not job.get("skipped") and f["online"],
        f"another content hash: {len(f['differs'])} difference row(s), {len(f['jobs'])} round(s) analysing {job.get('analysed')} "
        f"(errors {job.get('errors')}, skipped {job.get('skipped')}), project document online on disk {f['online']}",
    )
    # M1 (review 1): an archive of an older build holds no hash and perhaps no sample count. Other audio under the same
    # name must never get the old hits: other length = stale, no count = unknown = stale, one round for each.
    for pname, ok_u, ev_u, fu in readd_unknown_parts(label, env):
        parts[pname] = (ok_u, ev_u)
        runs.append(fu)
    ok = True
    for name, (good, evidence) in parts.items():
        ok = bool(good) and ok
        print(f"    {'ok  ' if good else ('refused' if fault else 'FAIL')} {label}:{name}: {evidence}")
    if fault:
        # the fault run must be refused on the line it breaks (the repair), with a healthy restore
        refused = not parts["one"][0] and not parts["all"][0]
        return pass_fail(
            label,
            healthy and refused and offline_on_disk and not any(r["repairs"] for r in runs[:2]),
            f"the plugin did not point the record of a restored track at the new cache file: both restores ran healthy ({healthy}) with no repair row, "
            f"the project document on disk holds records with no identity ({offline_on_disk}) and the check refused them ({refused})",
        )
    return pass_fail(label, ok, f"{len(parts)} parts")


def stage_doc_readd_fault() -> bool:
    return stage_doc_readd("no-repair")


def stage_doc_readd_unknown_fault() -> bool:
    """The fault run of the M1 rule (`unknown-same`): an archive with no hash and no sample count counts as the same
    audio, so other audio of the same length gets the old hits and no round runs. The check must refuse that."""
    label = "doc-readd-unknown-fault"
    if not HOST.exists() or not CLAP.exists():
        return pass_fail(label, False, "no host or no CLAP: run check.sh build first")
    ok_store, _archive, why = archive_store()
    if not ok_store:
        return pass_fail(label, False, why)
    parts = readd_unknown_parts(label, {"DS_ARA_RESTORE_FAULT": "unknown-same"})
    by = {p[0]: p for p in parts}
    nocount = by["oldother-nocount"]
    f = nocount[3]
    refused = not nocount[1]
    return pass_fail(
        label,
        f["healthy"] and refused and len(f["jobs"]) == 0 and len(f["repairs"]) == 1,
        f"the fault switch took an unknown content as the same audio: the restore ran healthy ({f['healthy']}), put the old hits "
        f"on other audio with {len(f['repairs'])} repair row(s) and {len(f['jobs'])} round(s), and the check refused it ({refused})",
    )


def stage_doc_store_window() -> bool:
    """(1) The host stores the moment its poll sees the first source complete, and the round
    needs 800 ms to publish (switch `publish-delay`, 200 ms for each source): the store waits for the
    publication and the archive holds all events. (2) The host stores before the first round has ended
    (the round waits 1.5 s before its analysis): the early archive is the last complete state, a
    document with no notes and no events, and the store does not wait for the round; the late
    archive of the same process holds every event, byte for byte the reference."""
    if not HOST.exists() or not CLAP.exists() or not (OUT / "events-reference.json").exists():
        return pass_fail("doc-store-window", False, "no host, no CLAP or no reference events")
    rc, archive = store_window_run("doc-store-window", "publish-delay")
    parts, evidence = store_window_parts(archive)
    ok = pass_fail("doc-store-window", rc == 0 and all(parts.values()), f"store in the publication (rc={rc}): {evidence}")
    early, late = OUT / "doc-store-early.araarchive", OUT / "doc-store-late.araarchive"
    for f in (early, late):
        if f.exists():
            f.unlink()
    rc2, _text, lines = host_run(
        "doc-store-early",
        CLAP,
        ["store-early", "--files", *fixture_args(FILES), "--archive", str(early), "--out", str(late), "--wait", HOST_WAIT],
        "cache-doc-store-early",
        extra={"DS_ARA_SCENARIO": "slow-setup"},
    )
    e = json.loads(early.read_text()) if early.exists() else {}
    row = next((r for r in lines if r.get("event") == "stored_early"), {})
    early_notes = sum(len(s.get("notes") or []) for s in e.get("sources", []))
    early_events = sum(len(v) for v in (e.get("events") or {}).values())
    late_parts, late_evidence = store_window_parts(late)
    again_ok, again_evidence, _facts = early_restore_check("doc-store-early-restore", early, late)
    early_ok = again_ok and (
        rc2 == 0
        and bool(e)
        and early_notes == 0
        and early_events == 0
        and len(e.get("sources", [])) == len(FILES)
        and row.get("store_ms", 10**9) < 1000
    )
    return pass_fail(
        "doc-store-early",
        early_ok and all(late_parts.values()),
        f"store before the first round ended (rc={rc2}): {len(e.get('sources', []))} source(s), {early_notes} note(s), {early_events} event(s), "
        f"store call {row.get('store_ms')} ms (the round needs 1.5 s more); the late archive: {late_evidence}; {again_evidence}",
    ) and ok


def stage_doc_store_window_fault() -> bool:
    """The fault run: the events come after the notes, with no lock held between (the old order). The
    archive of the store in that window holds all notes and no event, and the check must refuse it."""
    if not HOST.exists() or not CLAP.exists() or not (OUT / "events-reference.json").exists():
        return pass_fail("doc-store-window-fault", False, "no host, no CLAP or no reference events")
    rc, archive = store_window_run("doc-store-window-fault", "events-late")
    parts, evidence = store_window_parts(archive)
    return pass_fail(
        "doc-store-window-fault",
        rc == 0 and parts["notes"] and parts["events"] is False,
        f"the fault switch put the events after the notes and the archive stored in the window lost them: {evidence}",
    )


# ---- review round 2, items 4 and 10: a failed round and a request for a complete source


def retry_run(label: str, files: list, extra: dict | None = None, wait: str = HOST_WAIT) -> tuple[int, list]:
    rc, _text, lines = host_run(
        label,
        CLAP,
        ["grow", "--files", *fixture_args(files), "--batch", str(len(files)), "--rerequest", "--pump-ms", "1500", "--wait", wait],
        f"cache-{label}",
        extra=extra,
    )
    return rc, lines


def retry_parts(label: str, cache: str, rc: int, lines: list, ref: bytes, files: list) -> tuple[dict, str]:
    jobs = job_rows(label)
    failed = event_rows(label, "job_failed")
    docs = project_docs(cache)
    on_disk = list(tracks_of_doc(docs[0])) if len(docs) == 1 else []
    after = host_notes_of(lines, "after-rerequest")
    parts = {
        "rc": rc == 0,
        "failed_once": len(failed) == 1,
        "one_result": len(jobs) == 1 and jobs[0].get("events_json", "").encode() == ref,
        "one_track_per_source": on_disk == track_names(files) and (jobs[-1].get("project_tracks") if jobs else None) == track_names(files),
        "notes": len(after) == len(files) and all(after.values()),
    }
    return parts, (
        f"the first round failed ({len(failed)} job_failed row), the host asked again: {len(jobs)} result round(s), "
        f"events byte for byte the reference {parts['one_result']}; the document on disk holds {on_disk} (want {track_names(files)}); "
        f"notes after the retry {[len(v) for v in after.values()]}; rc={rc}"
    )


def stage_doc_retry() -> bool:
    if not HOST.exists() or not CLAP.exists() or not SERVICE_TOOL.exists():
        return pass_fail("doc-retry", False, "no host, no CLAP or no service_tool: run check.sh build first")
    ref = reference_alone("m1b1-ref-a", DOC_A)
    if ref is None:
        return pass_fail("doc-retry", False, "the reference failed")
    rc, lines = retry_run("doc-retry", DOC_A, {"DS_ARA_SCENARIO": "fail-first-round"})
    parts, evidence = retry_parts("doc-retry", "cache-doc-retry", rc, lines, ref, DOC_A)
    return pass_fail("doc-retry", all(parts.values()), evidence)


def stage_doc_retry_fault() -> bool:
    """The fault run: a retry adds the tracks again (the first version: `name_2`)."""
    if not HOST.exists() or not CLAP.exists() or not SERVICE_TOOL.exists():
        return pass_fail("doc-retry-fault", False, "no host, no CLAP or no service_tool: run check.sh build first")
    ref = reference_alone("m1b1-ref-a", DOC_A)
    if ref is None:
        return pass_fail("doc-retry-fault", False, "the reference failed")
    rc, lines = retry_run(
        "doc-retry-fault", DOC_A, {"DS_ARA_SCENARIO": "fail-first-round", "DS_ARA_RETRY_FAULT": "duplicate"}
    )
    parts, evidence = retry_parts("doc-retry-fault", "cache-doc-retry-fault", rc, lines, ref, DOC_A)
    return pass_fail(
        "doc-retry-fault",
        rc == 0 and parts["failed_once"] and parts["one_track_per_source"] is False and not parts["one_result"],
        f"the fault switch made a second track for each retried source and the check refused it: {evidence}",
    )


def rerequest_parts(label: str, rc: int, ref: bytes) -> tuple[dict, str]:
    jobs = job_rows(label)
    starts = event_rows(label, "round_start")
    parts = {
        "rc": rc == 0,
        "one_round": len(starts) == 1 and len(jobs) == 1 and services_row(label).get("rounds") == 1,
        "result": len(jobs) == 1 and jobs[0].get("events_json", "").encode() == ref,
        "no_removal": all(r.get("reason") != "removal" for r in starts),
    }
    return parts, (
        f"the host asked again for sources that are complete: {len(starts)} round(s) started in all (want 1, the first), "
        f"reasons {[r.get('reason') for r in starts]}, events byte for byte the reference {parts['result']}"
    )


def stage_doc_rerequest() -> bool:
    if not HOST.exists() or not CLAP.exists() or not SERVICE_TOOL.exists():
        return pass_fail("doc-rerequest", False, "no host, no CLAP or no service_tool: run check.sh build first")
    ref = reference_alone("m1b1-ref-a", DOC_A)
    if ref is None:
        return pass_fail("doc-rerequest", False, "the reference failed")
    rc, _lines = retry_run("doc-rerequest", DOC_A)
    parts, evidence = rerequest_parts("doc-rerequest", rc, ref)
    return pass_fail("doc-rerequest", all(parts.values()), evidence)


def stage_doc_rerequest_fault() -> bool:
    """The fault run: a request for a complete source starts a full round with the reason `removal`."""
    if not HOST.exists() or not CLAP.exists() or not SERVICE_TOOL.exists():
        return pass_fail("doc-rerequest-fault", False, "no host, no CLAP or no service_tool: run check.sh build first")
    ref = reference_alone("m1b1-ref-a", DOC_A)
    if ref is None:
        return pass_fail("doc-rerequest-fault", False, "the reference failed")
    rc, _lines = retry_run("doc-rerequest-fault", DOC_A, {"DS_ARA_ROUND_FAULT": "always"})
    parts, evidence = rerequest_parts("doc-rerequest-fault", rc, ref)
    return pass_fail(
        "doc-rerequest-fault",
        rc == 0 and parts["one_round"] is False and parts["no_removal"] is False,
        f"the fault switch started a round for the complete sources and the check refused it: {evidence}",
    )


# ---- review round 2, item 5: the error of a single track


ERROR_FILES = [FILES[0], FILES[1], FILES[2]]  # kick, snare, tom: the snare (index 1) is damaged
ERROR_GOOD = [FILES[0], FILES[2]]


def errors_parts(label: str, rc: int, lines: list, ref_good: bytes) -> tuple[dict, str]:
    jobs = job_rows(label)
    job = jobs[-1] if jobs else {}
    bad_track = safe_name(stem_of(ERROR_FILES[1]))
    good_tracks = track_names(ERROR_GOOD)
    t = job.get("track_json") or {}
    rebuilt = "{" + ",".join(f'"{n}":{t[n]}' for n in good_tracks if n in t) + "}"
    errs = job.get("errors") or []
    final = host_notes_of(lines, "final")
    parts = {
        "rc": rc == 0,
        "good_equal_reference": rebuilt.encode() == ref_good,
        "error_reported": [e.get("track") for e in errs] == [bad_track] and bool(errs and errs[0].get("error")),
        "no_events": t.get(bad_track) in ("[]", None),
        "partial": job.get("partial") is True,
        "no_notes": all(
            (len(v) == 0) == (Path(n).stem == Path(ERROR_FILES[1]).stem) for n, v in final.items()
        ) and len(final) == len(ERROR_FILES),
    }
    return parts, (
        f"three tracks, one WAV damaged: the two good tracks byte for byte the reference of the two together {parts['good_equal_reference']}; "
        f"the bad track {bad_track} is reported with an error {errs[:1]} and has events {t.get(bad_track)}; round marked partial "
        f"{job.get('partial')}; host notes {[(Path(n).stem, len(v)) for n, v in final.items()]}; ara-host rc={rc} (design: 0, the ARA "
        "protocol has no error channel, the host sees the analysis as complete without content and the report carries the error)"
    )


def errors_run(label: str, extra: dict | None = None) -> tuple[int, list]:
    rc, _text, lines = host_run(
        label,
        CLAP,
        ["grow", "--files", *fixture_args(ERROR_FILES), "--batch", "3", "--wait", HOST_WAIT],
        f"cache-{label}",
        extra={"DS_ARA_SCENARIO": "corrupt-wav:1", **(extra or {})},
    )
    return rc, lines


def stage_doc_errors() -> bool:
    if not HOST.exists() or not CLAP.exists() or not SERVICE_TOOL.exists():
        return pass_fail("doc-errors", False, "no host, no CLAP or no service_tool: run check.sh build first")
    ref = reference_alone("m1b1-ref-two-good", ERROR_GOOD)
    if ref is None:
        return pass_fail("doc-errors", False, "the reference of the two good tracks failed")
    rc, lines = errors_run("doc-errors")
    parts, evidence = errors_parts("doc-errors", rc, lines, ref)
    return pass_fail("doc-errors", all(parts.values()), evidence)


def stage_doc_errors_fault() -> bool:
    """The fault run: the error of a single track is not read (the first version)."""
    if not HOST.exists() or not CLAP.exists() or not SERVICE_TOOL.exists():
        return pass_fail("doc-errors-fault", False, "no host, no CLAP or no service_tool: run check.sh build first")
    ref = reference_alone("m1b1-ref-two-good", ERROR_GOOD)
    if ref is None:
        return pass_fail("doc-errors-fault", False, "the reference of the two good tracks failed")
    rc, lines = errors_run("doc-errors-fault", {"DS_ARA_ERRORS_FAULT": "swallow"})
    parts, evidence = errors_parts("doc-errors-fault", rc, lines, ref)
    return pass_fail(
        "doc-errors-fault",
        rc == 0 and parts["error_reported"] is False and parts["partial"] is False,
        f"the fault switch did not read the error of the track and the check refused it: {evidence}",
    )


# ---- review round 2, item 6: the regions of an earlier source change after the analysis

REGIONS_CHANGED = [(0.0, 1.2, 0.0), (1.4, 0.9, 1.4)]  # the second region of REGIONS, longer and earlier: the hit at 1.5 s is in it now
REGIONS_CHANGE_SPEC = "1:1.4,0.9,1.4"


def regions_args(regions: list) -> list:
    args = ["regions", "--file", str(FIXTURES / REGIONS_FILE)]
    for a, d, p in regions:
        args += ["--region", f"{a},{d},{p}"]
    return args


def service_reference_of_plugin_doc(cache: str, name: str) -> bytes | None:
    """The events the service gives on a copy of the plugin's own document (the segments the
    plugin made from the regions) with the source records pointed back at the fixture file."""
    doc_path = doc_of_run(cache)
    if not doc_path.exists():
        return None
    doc = json.loads(doc_path.read_text())
    fixture = FIXTURES / REGIONS_FILE
    st = fixture.stat()
    ref_dir = OUT / f"ref-{name}"
    ref_dir.mkdir(parents=True, exist_ok=True)
    rec = {
        "mode": "referenced",
        "path": str(fixture),
        "relative": os.path.relpath(fixture, ref_dir).replace(os.sep, "/"),
        "size": st.st_size,
        "mtime": int(st.st_mtime),
        "head_sha1": hashlib.sha1(fixture.read_bytes()[: 1 << 20]).hexdigest(),
    }
    for track in (doc.get("tracks") or {}).values():
        for seg in track.get("segments") or []:
            seg["source"] = rec
    (ref_dir / f"ref-{name}.drumspike").write_text(json.dumps(doc), encoding="utf-8")
    ok, events, _n = service_tool_events(ref_dir, name)
    return events if ok else None


def regions_change_parts(label: str, cache: str, lines: list, ref_events: bytes, ref_notes: list) -> tuple[dict, str]:
    jobs = job_rows(label)
    last = jobs[-1] if jobs else {}
    first = jobs[0] if jobs else {}
    after = [r for r in lines if r.get("event") == "host_notes" and r.get("phase") == "after-update"]
    before = [r for r in lines if r.get("event") == "host_notes" and r.get("phase") == "before-update"]
    segs = doc_segments(cache) or []
    track_events = next(iter(json.loads(ref_events).values()), [])
    parts = {
        "new_round": len(jobs) == 2 and [j.get("reason") for j in jobs] == ["request", "change"],
        "events": last.get("events_json", "").encode() == ref_events,
        "notes": bool(after) and after[0].get("samples") == (ref_notes[0].get("samples") if ref_notes else None),
        "changed": bool(first) and first.get("events_json") != last.get("events_json"),
        "segments": len(segs) == 2 and abs(segs[1]["start"] - REGIONS_CHANGED[1][0]) < 1e-9 and abs(segs[1]["length"] - REGIONS_CHANGED[1][1]) < 1e-9,
        "new_hit": any(abs(ev["t"] - 1.5) < 0.05 for ev in track_events),
    }
    return parts, (
        f"analysed, then the second region changed: {len(jobs)} round(s) with reasons {[j.get('reason') for j in jobs]}; the events of the "
        f"last round are byte for byte the service reference of the new regions {parts['events']} ({len(track_events)} events, the hit at 1.5 s now in: "
        f"{parts['new_hit']}); notes after the change equal the reference run's {parts['notes']} ({len(after[0].get('samples', [])) if after else None} vs "
        f"{len(before[0].get('samples', [])) if before else None} before); the document holds the new segment {parts['segments']}"
    )


def regions_change_run(label: str, extra: dict | None = None) -> tuple[int, list]:
    rc, _text, lines = host_run(
        label,
        CLAP,
        [*regions_args(REGIONS), "--update-region", REGIONS_CHANGE_SPEC, "--update-after", "--pump-ms", "3500"],
        f"cache-{label}",
        extra=extra,
    )
    return rc, lines


def regions_change_reference() -> tuple[bytes, list] | None:
    """A plain run with the new regions from the start: its events equal the service reference of
    its own document."""
    rc, _text, lines = host_run("regions-new", CLAP, regions_args(REGIONS_CHANGED), "cache-regions-new")
    ref = service_reference_of_plugin_doc("cache-regions-new", "regions-new")
    if rc != 0 or ref is None:
        return None
    notes = [r for r in lines if r.get("event") == "host_notes"]
    ok, events = plugin_service_row("regions-new")
    return (ref, notes) if ok and events == ref else None


def stage_doc_regions_change() -> bool:
    if not HOST.exists() or not CLAP.exists() or not SERVICE_TOOL.exists():
        return pass_fail("doc-regions-change", False, "no host, no CLAP or no service_tool: run check.sh build first")
    reference = regions_change_reference()
    if reference is None:
        return pass_fail("doc-regions-change", False, "the reference run with the new regions failed or differs from the service")
    rc, lines = regions_change_run("doc-regions-change")
    parts, evidence = regions_change_parts("doc-regions-change", "cache-doc-regions-change", lines, *reference)
    return pass_fail("doc-regions-change", rc == 0 and all(parts.values()), evidence)


def stage_doc_regions_change_fault() -> bool:
    """The fault run: a change of the regions is not seen (the first version)."""
    if not HOST.exists() or not CLAP.exists() or not SERVICE_TOOL.exists():
        return pass_fail("doc-regions-change-fault", False, "no host, no CLAP or no service_tool: run check.sh build first")
    reference = regions_change_reference()
    if reference is None:
        return pass_fail("doc-regions-change-fault", False, "the reference run with the new regions failed or differs from the service")
    rc, lines = regions_change_run("doc-regions-change-fault", {"DS_ARA_TOUCH_FAULT": "ignore"})
    parts, evidence = regions_change_parts("doc-regions-change-fault", "cache-doc-regions-change-fault", lines, *reference)
    return pass_fail(
        "doc-regions-change-fault",
        rc == 0 and parts["new_round"] is False and parts["events"] is False,
        f"the fault switch did not see the change and the check refused it: {evidence}",
    )


# ---- review round 2, item 9: a panic in the reader thread or in the worker thread

PANIC_WAIT = "8"


def panic_parts(label: str, rc: int, ref: bytes, kind: str) -> tuple[dict, str]:
    rows = report_rows(label)
    jobs = job_rows(label)
    marker = "worker_restarted" if kind == "worker" else "reader_panic"
    parts = {
        "rc": rc == 0,
        "reported": any(r.get("event") == marker for r in rows),
        "result": bool(jobs) and jobs[-1].get("events_json", "").encode() == ref,
    }
    return parts, (
        f"a panic in the {kind} thread: rc={rc}, report row {marker} {parts['reported']}, after the host asked again the last round's events are "
        f"byte for byte the reference {parts['result']} ({len(jobs)} result round(s))"
    )


def stage_doc_panic() -> bool:
    if not HOST.exists() or not CLAP.exists() or not SERVICE_TOOL.exists():
        return pass_fail("doc-panic", False, "no host, no CLAP or no service_tool: run check.sh build first")
    ref = reference_alone("m1b1-ref-a", DOC_A)
    if ref is None:
        return pass_fail("doc-panic", False, "the reference failed")
    ok = True
    for kind in ("worker", "reader"):
        rc, _lines = retry_run(f"doc-panic-{kind}", DOC_A, {"DS_ARA_PANIC_TEST": kind}, wait=PANIC_WAIT)
        parts, evidence = panic_parts(f"doc-panic-{kind}", rc, ref, kind)
        ok = pass_fail(f"doc-panic-{kind}", all(parts.values()), evidence) and ok
    return ok


def stage_doc_panic_fault() -> bool:
    """The fault run: the threads run without the guard. A panic ends the thread without a
    message: the round never starts (reader) or the worker is gone for ever (worker)."""
    if not HOST.exists() or not CLAP.exists() or not SERVICE_TOOL.exists():
        return pass_fail("doc-panic-fault", False, "no host, no CLAP or no service_tool: run check.sh build first")
    ref = reference_alone("m1b1-ref-a", DOC_A)
    if ref is None:
        return pass_fail("doc-panic-fault", False, "the reference failed")
    out = {}
    for kind in ("worker", "reader"):
        rc, _lines = retry_run(
            f"doc-panic-fault-{kind}", DOC_A, {"DS_ARA_PANIC_TEST": kind, "DS_ARA_GUARD_FAULT": "off"}, wait=PANIC_WAIT
        )
        parts, evidence = panic_parts(f"doc-panic-fault-{kind}", rc, ref, kind)
        out[kind] = (parts["reported"] is False and parts["result"] is False, evidence)
    return pass_fail(
        "doc-panic-fault",
        all(v[0] for v in out.values()),
        " | ".join(f"{k}: refused {v[0]}: {v[1]}" for k, v in out.items()),
    )


# ---- review round 3, item 1: a host source name never leaves the audio folder

# the first name climbs five levels from `audio/000-x`: out of audio, the document folder,
# documents and the cache folder, to the folder next to the cache (`file_stem` is None for a name
# that ends in `..`, and the old code used the whole name as the file name)
HOSTILE_NAMES = ["x/../../../../../escape-name/..", "a/..", "/abs/path/..", ""]
WAV_NAME = re.compile(r"^\d{3}-[^/\\]+\.wav$")


def source_names_parts(label: str, rc: int) -> tuple[dict, str]:
    root = OUT / f"hostile-{label}"
    cache = root / "cache"
    outside = sorted(p.name for p in root.iterdir() if p.name != "cache") if root.is_dir() else []
    docs = [d for d in (cache / "documents").iterdir()] if (cache / "documents").is_dir() else []
    wavs = [p for d in docs for p in (d / "audio").rglob("*") if p.is_file()]
    flat = [p for d in docs for p in (d / "audio").iterdir()]
    destroyed = event_rows(label, "source_destroyed")
    parts = {
        "rc": rc == 0,
        "no_escape": not outside,
        "one_document": len(docs) == 1,
        "wavs_in_audio": bool(wavs) and all(p.parent == d / "audio" and WAV_NAME.match(p.name) for d in docs for p in wavs),
        "no_stray_folders": all(p.is_file() for p in flat),
        "job": len(job_rows(label)) >= 1,
        "destroyed": len(destroyed) == 1 and destroyed[0].get("error") is None,
    }
    return parts, (
        f"{len(HOSTILE_NAMES)} hostile source names: rc={rc}, outside the cache folder {outside}, {len(wavs)} WAV file(s), "
        f"all directly in audio/ with a plain name {parts['wavs_in_audio']}, folders made in audio/ {not parts['no_stray_folders']}, "
        f"{len(job_rows(label))} round(s), one source destroyed and its WAV deleted inside the folder {parts['destroyed']}"
    )


def source_names_run(label: str, extra: dict | None = None) -> int:
    root = OUT / f"hostile-{label}"
    if root.exists():
        shutil.rmtree(root)
    root.mkdir(parents=True)
    rc, _text, _lines = host_run(
        label,
        CLAP,
        ["grow", "--files", *fixture_args(FILES), "--names", *HOSTILE_NAMES, "--batch", "4", "--destroy", "3", "--pump-ms", "1500", "--wait", HOST_WAIT],
        f"hostile-{label}/cache",
        extra=extra,
    )
    return rc


def stage_doc_source_names() -> bool:
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("doc-source-names", False, "no host or no CLAP: run check.sh build first")
    rc = source_names_run("doc-source-names")
    parts, evidence = source_names_parts("doc-source-names", rc)
    return pass_fail("doc-source-names", all(parts.values()), evidence)


def stage_doc_source_names_fault() -> bool:
    """The fault run: the old stem (the whole name when `file_stem` is None) names the WAV."""
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("doc-source-names-fault", False, "no host or no CLAP: run check.sh build first")
    rc = source_names_run("doc-source-names-fault", {"DS_ARA_ID_FAULT": "trust"})
    parts, evidence = source_names_parts("doc-source-names-fault", rc)
    return pass_fail(
        "doc-source-names-fault",
        parts["no_escape"] is False,
        f"the fault switch took the host name as the file name and the check saw a folder outside the cache folder: {evidence}",
    )


# ---- review round 3, item 2: a move in playback time alone starts no round

REGIONS_MOVE_SPEC = "1:1.8,0.5,3.5"  # the second region of REGIONS: the same slice, placed later on the timeline


def region_move_parts(label: str, rc: int, lines: list) -> tuple[dict, str]:
    jobs = job_rows(label)
    before = [r for r in lines if r.get("event") == "host_notes" and r.get("phase") == "before-update"]
    after = [r for r in lines if r.get("event") == "host_notes" and r.get("phase") == "after-update"]
    parts = {
        "rc": rc == 0,
        "no_round": len(jobs) == 1 and services_row(label).get("rounds") == 1,
        "notes": bool(before) and bool(after) and before[0].get("samples") == after[0].get("samples") and bool(before[0].get("samples")),
    }
    return parts, (
        f"the second region moved in playback time only: {len(jobs)} round(s) in all (want 1, the first), "
        f"notes unchanged {parts['notes']} ({len(before[0].get('samples', [])) if before else None} before, {len(after[0].get('samples', [])) if after else None} after)"
    )


def region_move_run(label: str, extra: dict | None = None) -> tuple[int, list]:
    rc, _text, lines = host_run(
        label,
        CLAP,
        [*regions_args(REGIONS), "--update-region", REGIONS_MOVE_SPEC, "--update-after", "--pump-ms", "2500"],
        f"cache-{label}",
        extra=extra,
    )
    return rc, lines


def stage_doc_region_move() -> bool:
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("doc-region-move", False, "no host or no CLAP: run check.sh build first")
    rc, lines = region_move_run("doc-region-move")
    parts, evidence = region_move_parts("doc-region-move", rc, lines)
    return pass_fail("doc-region-move", all(parts.values()), evidence)


def stage_doc_region_move_fault() -> bool:
    """The fault run: a move in playback time starts a round (the first version of the compare)."""
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("doc-region-move-fault", False, "no host or no CLAP: run check.sh build first")
    rc, lines = region_move_run("doc-region-move-fault", {"DS_ARA_TOUCH_FAULT": "playback-time"})
    parts, evidence = region_move_parts("doc-region-move-fault", rc, lines)
    return pass_fail(
        "doc-region-move-fault",
        rc == 0 and parts["no_round"] is False,
        f"the fault switch started a round for the move and the check refused it: {evidence}",
    )


# ---- review round 3, item 4: a source whose audio never comes does not block the document


def late_parts(label: str, rc: int, ref_kick: bytes) -> tuple[dict, str]:
    jobs = job_rows(label)
    job = jobs[0] if jobs else {}
    late = event_rows(label, "source_late")
    parts = {
        "rc": rc == 0,
        "round": len(jobs) >= 1 and job.get("events_json", "").encode() == ref_kick,
        "late_named": len(late) == 1 and [Path(n).stem for n in job.get("late", [])] == [Path(DOC_A[1]).stem],
        "partial": job.get("partial") is True,
    }
    return parts, (
        f"the audio of the second source never came: rc={rc}, {len(jobs)} round(s), the first with the events of the first source alone "
        f"byte for byte the reference {parts['round']}, the late source named {job.get('late')}, round partial {job.get('partial')}"
    )


def late_run(label: str, extra: dict | None = None) -> int:
    rc, _text, _lines = host_run(
        label,
        CLAP,
        ["grow", "--files", *fixture_args(DOC_A), "--batch", "2", "--no-access", "1", "--wait", PANIC_WAIT],
        f"cache-{label}",
        extra={"DS_ARA_SCENARIO": "audio-deadline-ms:1500", **(extra or {})},
    )
    return rc


def stage_doc_late_audio() -> bool:
    if not HOST.exists() or not CLAP.exists() or not SERVICE_TOOL.exists():
        return pass_fail("doc-late-audio", False, "no host, no CLAP or no service_tool: run check.sh build first")
    ref = reference_alone("m1b1-ref-kick", [DOC_A[0]])
    if ref is None:
        return pass_fail("doc-late-audio", False, "the reference failed")
    rc = late_run("doc-late-audio")
    parts, evidence = late_parts("doc-late-audio", rc, ref)
    return pass_fail("doc-late-audio", all(parts.values()), evidence)


def late_arrives_parts(label: str, rc: int, lines: list, ref_all: bytes, ref_kick: bytes) -> tuple[dict, str]:
    jobs = job_rows(label)
    after = host_notes_of(lines, "after-late")
    told = notified(lines, "late")
    parts = {
        "rc": rc == 0,
        "two_rounds": len(jobs) == 2,
        "first_without": len(jobs) >= 1 and jobs[0].get("events_json", "").encode() == ref_kick and jobs[0].get("partial") is True,
        "second_both": len(jobs) == 2 and jobs[1].get("events_json", "").encode() == ref_all and jobs[1].get("partial") is False,
        "notes": len(after) == len(DOC_A) and all(after.values()),
        "told": any(Path(n).stem == Path(DOC_A[1]).stem for n in told),
    }
    return parts, (
        f"the audio of the late source came after the first round: {len(jobs)} round(s); the first without it (partial "
        f"{jobs[0].get('partial') if jobs else None}), the second with both tracks byte for byte the reference of the two together "
        f"{parts['second_both']}; notes of both sources after it {[len(v) for v in after.values()]}; the host was told for the late source {parts['told']}"
    )


def late_arrives_run(label: str, extra: dict | None = None) -> tuple[int, list]:
    rc, _text, lines = host_run(
        label,
        CLAP,
        ["grow", "--files", *fixture_args(DOC_A), "--batch", "2", "--no-access", "1", "--enable-late", "1", "--wait", PANIC_WAIT],
        f"cache-{label}",
        extra={"DS_ARA_SCENARIO": "audio-deadline-ms:1500", **(extra or {})},
    )
    return rc, lines


def stage_doc_late_audio_arrives() -> bool:
    if not HOST.exists() or not CLAP.exists() or not SERVICE_TOOL.exists():
        return pass_fail("doc-late-audio-arrives", False, "no host, no CLAP or no service_tool: run check.sh build first")
    ref_all, ref_kick = reference_alone("m1b1-ref-a", DOC_A), reference_alone("m1b1-ref-kick", [DOC_A[0]])
    if ref_all is None or ref_kick is None:
        return pass_fail("doc-late-audio-arrives", False, "a reference failed")
    rc, lines = late_arrives_run("doc-late-audio-arrives")
    parts, evidence = late_arrives_parts("doc-late-audio-arrives", rc, lines, ref_all, ref_kick)
    return pass_fail("doc-late-audio-arrives", all(parts.values()), evidence)


def stage_doc_late_audio_arrives_fault() -> bool:
    """The fault run: audio that comes after the deadline starts no round (the first version)."""
    if not HOST.exists() or not CLAP.exists() or not SERVICE_TOOL.exists():
        return pass_fail("doc-late-audio-arrives-fault", False, "no host, no CLAP or no service_tool: run check.sh build first")
    ref_all, ref_kick = reference_alone("m1b1-ref-a", DOC_A), reference_alone("m1b1-ref-kick", [DOC_A[0]])
    if ref_all is None or ref_kick is None:
        return pass_fail("doc-late-audio-arrives-fault", False, "a reference failed")
    rc, lines = late_arrives_run("doc-late-audio-arrives-fault", {"DS_ARA_ROUND_FAULT": "no-late-round"})
    parts, evidence = late_arrives_parts("doc-late-audio-arrives-fault", rc, lines, ref_all, ref_kick)
    return pass_fail(
        "doc-late-audio-arrives-fault",
        rc == 0 and parts["first_without"] and parts["two_rounds"] is False and parts["second_both"] is False,
        f"the fault switch started no round for the late audio and the check refused it: {evidence}",
    )


def stage_doc_late_audio_fault() -> bool:
    """The fault run: a request waits for ever for audio (the first version): the host times out."""
    if not HOST.exists() or not CLAP.exists() or not SERVICE_TOOL.exists():
        return pass_fail("doc-late-audio-fault", False, "no host, no CLAP or no service_tool: run check.sh build first")
    ref = reference_alone("m1b1-ref-kick", [DOC_A[0]])
    if ref is None:
        return pass_fail("doc-late-audio-fault", False, "the reference failed")
    rc = late_run("doc-late-audio-fault", {"DS_ARA_ROUND_FAULT": "no-deadline"})
    parts, evidence = late_parts("doc-late-audio-fault", rc, ref)
    return pass_fail(
        "doc-late-audio-fault",
        parts["round"] is False and parts["late_named"] is False,
        f"the fault switch waited for ever and the check refused it (no round for the first source): {evidence}",
    )


# ---- M1b4 (#545): a source is analysed as soon as its audio is here, with or without a request

SHORT_WAIT = "8"  # seconds a fault run of these stages waits for a round that does not come


def nr_parts(label: str, rc: int, lines: list, files: list, ref: bytes, requests: int | None = 0) -> tuple[dict, str]:
    """A run in which the host never asks (`--no-request`): every source has its round, in one
    round, the events are the reference of the run with requests, the host reads notes and was
    told, and the report has no `request` row."""
    starts = event_rows(label, "round_start")
    jobs = job_rows(label)
    asked = event_rows(label, "request")
    notes = host_notes_of(lines, "round-1")
    told = notified(lines, "round-1")
    last = jobs[-1] if jobs else {}
    parts = {
        "rc": rc == 0,
        "round": len(starts) == 1 and len(jobs) == 1,
        "all_in_round": last.get("project_tracks") == track_names(files) and len(last.get("analysed") or []) == len(files),
        "events": last.get("events_json", "").encode() == ref,
        "no_request_row": len(asked) == 0 if requests == 0 else len(asked) == requests,
        "notes": len(notes) == len(files) and all(notes.values()),
        "told": len(told) == len(files),
    }
    what = "no round" if not starts else f"{len(starts)} round(s)"
    return parts, (
        f"the host enabled the samples access of {len(files)} sources and never asked for an analysis (rc={rc}): {what}, {len(jobs)} job row(s); "
        f"the last round has {len(last.get('analysed') or [])} analysed tracks, events byte for byte the reference of the run with requests "
        f"{parts['events']}; {len(asked)} request row(s); notes read after it {[len(v) for v in notes.values()]}; the host was told for {len(told)} source(s)"
    )


def nr_run(label: str, files: list, extra_args: list | None = None, env: dict | None = None, wait: str = HOST_WAIT) -> tuple[int, list]:
    rc, _text, lines = host_run(
        label,
        CLAP,
        ["grow", "--files", *fixture_args(files), "--batch", str(len(files)), "--wait", wait, *(extra_args or [])],
        f"cache-{label}",
        extra=env,
    )
    return rc, lines


def stage_no_request() -> bool:
    """M1b4 gate lines 1 and 7: the fixture in the mode "no request"; every source has a round, the events
    equal the reference, no `request` row; the host's own questions (incomplete, notes, told) are right."""
    ref_path = OUT / "events-reference.json"
    if not HOST.exists() or not CLAP.exists() or not ref_path.exists():
        return pass_fail("no-request", False, "no host, no CLAP or no reference events: run check.sh build and the reference stage first")
    rc, lines = nr_run("no-request", FILES, ["--no-request"])
    parts, evidence = nr_parts("no-request", rc, lines, FILES, ref_path.read_bytes())
    return pass_fail("no-request", all(parts.values()), evidence)


def stage_no_request_fault() -> bool:
    """Gate line 2: the old behaviour (a round only on a request): the stage fails with "no round"."""
    ref_path = OUT / "events-reference.json"
    if not HOST.exists() or not CLAP.exists() or not ref_path.exists():
        return pass_fail("no-request-fault", False, "no host, no CLAP or no reference events")
    rc, lines = nr_run("no-request-fault", FILES, ["--no-request"], {"DS_ARA_ROUND_FAULT": "request-only"}, SHORT_WAIT)
    parts, evidence = nr_parts("no-request-fault", rc, lines, FILES, ref_path.read_bytes())
    starts = event_rows("no-request-fault", "round_start")
    return pass_fail(
        "no-request-fault",
        not parts["round"] and not parts["events"] and not starts,
        f"the fault switch gave a round only on a request and the check refused it (no round): {evidence}",
    )


def order_parts(label: str, rc: int, lines: list, ref: bytes) -> tuple[dict, str]:
    starts = event_rows(label, "round_start")
    jobs = job_rows(label)
    asked = event_rows(label, "request")
    parts = {
        "rc": rc == 0,
        "one_round": len(starts) == 1 and len(jobs) == 1 and services_row(label).get("rounds") == 1,
        "events": len(jobs) >= 1 and jobs[-1].get("events_json", "").encode() == ref,
        "asked": len(asked) == len(FILES),
    }
    return parts, (
        f"{len(asked)} request(s): {len(starts)} round(s) started in all (want 1), {len(jobs)} job row(s), "
        f"events byte for byte the reference {parts['events']}"
    )


def stage_request_order() -> bool:
    """Gate line 3: a host that requests gets one round and the same events, whether the audio is read before
    the request ("read, then request": the request comes after the analysis is complete) or after it
    ("request, then read": the audio comes 300 ms after the request)."""
    ref_path = OUT / "events-reference.json"
    if not HOST.exists() or not CLAP.exists() or not ref_path.exists():
        return pass_fail("request-order", False, "no host, no CLAP or no reference events: run check.sh build and the reference stage first")
    ok = True
    for order in ("read-first", "request-first"):
        label = f"request-order-{order}"
        rc, lines = nr_run(label, FILES, ["--request-order", order])
        parts, evidence = order_parts(label, rc, lines, ref_path.read_bytes())
        ok = pass_fail(f"request-order-{order}", all(parts.values()), evidence) and ok
    return ok


def stage_request_order_fault() -> bool:
    """The fault run: a request for a source that is complete starts a full round, so "read, then request"
    has two rounds and the check refuses it."""
    ref_path = OUT / "events-reference.json"
    if not HOST.exists() or not CLAP.exists() or not ref_path.exists():
        return pass_fail("request-order-fault", False, "no host, no CLAP or no reference events")
    rc, lines = nr_run("request-order-fault", FILES, ["--request-order", "read-first"], {"DS_ARA_ROUND_FAULT": "always"})
    parts, evidence = order_parts("request-order-fault", rc, lines, ref_path.read_bytes())
    return pass_fail(
        "request-order-fault",
        rc == 0 and parts["one_round"] is False,
        f"the fault switch gave a second round for the request after the audio and the check refused it: {evidence}",
    )


GROUP_FILES = FILES[:3]


def group_parts(label: str, rc: int, lines: list, ref: bytes) -> tuple[dict, str]:
    parts, evidence = nr_parts(label, rc, lines, GROUP_FILES, ref)
    return parts, evidence.replace("sources and never asked", "sources (the audio of the last one comes 700 ms after the others) and never asked")


def group_run(label: str, env: dict | None = None) -> tuple[int, list]:
    return nr_run(label, GROUP_FILES, ["--no-request", "--slow-read", "2:700"], env)


def stage_no_request_group() -> bool:
    """Gate line 4: three sources without a request, the audio of the last one comes after the others (more
    than the settle time later): one round, all three tracks took part."""
    if not HOST.exists() or not CLAP.exists() or not SERVICE_TOOL.exists():
        return pass_fail("no-request-group", False, "no host, no CLAP or no service_tool: run check.sh build first")
    ref = reference_alone("m1b4-ref-three", GROUP_FILES)
    if ref is None:
        return pass_fail("no-request-group", False, "the reference failed")
    rc, lines = group_run("no-request-group")
    parts, evidence = group_parts("no-request-group", rc, lines, ref)
    return pass_fail("no-request-group", all(parts.values()), evidence)


def stage_no_request_group_fault() -> bool:
    """The fault run: a source is queued only when its audio is here: one round for the two early sources
    and one for the late one."""
    if not HOST.exists() or not CLAP.exists() or not SERVICE_TOOL.exists():
        return pass_fail("no-request-group-fault", False, "no host, no CLAP or no service_tool: run check.sh build first")
    ref = reference_alone("m1b4-ref-three", GROUP_FILES)
    if ref is None:
        return pass_fail("no-request-group-fault", False, "the reference failed")
    rc, lines = group_run("no-request-group-fault", {"DS_ARA_ROUND_FAULT": "ready-only"})
    parts, evidence = group_parts("no-request-group-fault", rc, lines, ref)
    return pass_fail(
        "no-request-group-fault",
        rc == 0 and parts["round"] is False and len(event_rows("no-request-group-fault", "round_start")) >= 2,
        f"the fault switch gave a round to each source as its audio came and the check refused it: {evidence}",
    )


def restored_parts(label: str, rc: int, lines: list, before: dict) -> tuple[dict, str]:
    starts = event_rows(label, "round_start")
    reads = [r for r in report_rows(label) if r.get("event") == "read" and r.get("read_ok")]
    after = {r["name"]: r["samples"] for r in lines if r.get("event") == "host_notes"}
    parts = {
        "rc": rc == 0,
        "audio_read": len(reads) == len(FILES),
        "no_round": len(starts) == 0 and len(job_rows(label)) == 0,
        "notes": len(after) == len(FILES) and all(after.values()),
    }
    return parts, (
        f"the archive was restored in a new process and the host read the audio of all sources without a request: {len(reads)} read row(s), "
        f"{len(starts)} round(s) started (want 0), {len(job_rows(label))} job row(s), notes {[len(v) for v in after.values()]}"
    )


def restored_run(label: str, env: dict | None = None) -> tuple[int, list, dict]:
    ok_store, archive, why = archive_store()
    if not ok_store:
        return 1, [], {}
    rc, _text, lines = host_run(
        label, CLAP, ["restore", "--files", *fixture_args(FILES), "--archive", str(archive), "--enable-access"], f"cache-{label}", extra=env
    )
    return rc, lines, {}


def stage_no_request_restored() -> bool:
    """Gate line 5: a source that was restored with its result is not analysed again only because its audio is read.
    The old code (a round only on a request) passes this stage too: it started no round either. Only the fault run
    (`read-again`) guards the rule against a plugin that now queues every read; the unit tests
    `a_source_queued_by_its_read_and_then_restored_leaves_the_queue` and
    `a_restored_stale_source_is_analysed_again_when_its_audio_is_read` guard the order and the stale case."""
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("no-request-restored", False, "no host or no CLAP: run check.sh build first")
    rc, lines, before = restored_run("no-request-restored")
    parts, evidence = restored_parts("no-request-restored", rc, lines, before)
    return pass_fail("no-request-restored", all(parts.values()), evidence)


def stage_no_request_restored_fault() -> bool:
    """The fault run: the read of a source with a result starts a round of the document."""
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("no-request-restored-fault", False, "no host or no CLAP: run check.sh build first")
    rc, lines, before = restored_run("no-request-restored-fault", {"DS_ARA_ROUND_FAULT": "read-again"})
    parts, evidence = restored_parts("no-request-restored-fault", rc, lines, before)
    return pass_fail(
        "no-request-restored-fault",
        rc == 0 and parts["no_round"] is False,
        f"the fault switch analysed the restored sources again when their audio was read and the check refused it: {evidence}",
    )


def editor_no_request_run(label: str, env: dict | None = None) -> tuple[int, list]:
    late = ["--late-files", *[str(FIXTURES / f) for f in FILES], "--arg", str(len(FILES)), "--no-request", "--wait", SHORT_WAIT]
    rc, _summary, rows = run_phase(label, "open-first", CLAP, env, late, files=[])
    return rc, rows


def stage_editor_no_request() -> bool:
    """Gate line 6: the editor is open before the audio; the host never asks; the page gets the four tracks
    without a reload by hand (the `plugin-reload` path)."""
    ref = _reference()
    if ref is None or not HOST.exists():
        return pass_fail("editor-no-request", False, "no reference or host")
    rc, rows = editor_no_request_run("editor-no-request")
    parts = open_first_parts(rc, rows, ref)
    asked = [r for r in rows if r.get("event") == "request"]
    good, ev = parts["editor-open-first"]
    return pass_fail("editor-no-request", good and not asked, f"{ev}; the host sent {len(asked)} request(s)")


def stage_editor_no_request_fault() -> bool:
    """The fault run: a round only on a request, so the page never gets a track."""
    ref = _reference()
    if ref is None:
        return pass_fail("editor-no-request-fault", False, "no reference")
    rc, rows = editor_no_request_run("editor-fault-no-request", {"DS_ARA_ROUND_FAULT": "request-only"})
    good, ev = open_first_parts(rc, rows, ref)["editor-open-first"]
    return pass_fail("editor-no-request-fault", not good, f"the fault switch gave a round only on a request and the check refused it: {ev}")


# ---- M1b4 round 2: new audio, retry, cancelled and failed reads

ALT = OUT / "fixtures-m1b4"
HAT = FILES[3]


def alt_hat() -> Path:
    """The hat of the fixture with other hits (same name, rate and length): the new audio of a content update."""
    ALT.mkdir(parents=True, exist_ok=True)
    path = ALT / HAT
    name, rate, ch, secs, _hits, bleed = SPEC[3]
    make_wav(path, rate, ch, secs, [(0.25, 8000.0, 0.5), (0.65, 8000.0, 0.5), (1.10, 8000.0, 0.45), (1.40, 8000.0, 0.5)], bleed)
    return path


def update_parts(label: str, rc: int, lines: list, ref_old: bytes, ref_new: bytes) -> tuple[dict, str]:
    jobs = job_rows(label)
    rounds = services_row(label).get("rounds")
    hat = track_names([HAT])[0]
    first = json.loads(jobs[0]["events_json"]) if jobs else {}
    last = json.loads(jobs[-1]["events_json"]) if jobs else {}
    told = notified(lines, "update")
    docs = project_docs(f"cache-{label}")
    wavs = sorted(p.name for p in (OUT / f"cache-{label}").rglob("*.wav") if HAT.split(".")[0] in p.name)
    parts = {
        "rc": rc == 0,
        "first_is_reference": len(jobs) >= 1 and jobs[0].get("events_json", "").encode() == ref_old,
        "one_more_round": len(jobs) == 2 and rounds == 2,
        "events_new": len(jobs) == 2 and jobs[-1].get("events_json", "").encode() == ref_new,
        "hat_changed": first.get(hat) != last.get(hat) and bool(last.get(hat)),
        "others_unchanged": all(first.get(n) == last.get(n) for n in last if n != hat),
        "told": any(Path(n).stem == Path(HAT).stem for n in told),
        "one_wav": len(wavs) == 1,
    }
    return parts, (
        f"after the first analysis the host replaced the samples of the hat and sent updateAudioSourceContent: {len(jobs)} job row(s), {rounds} round(s) in all "
        f"(want 2); events after it byte for byte the reference of the new audio {parts['events_new']}; the hat's hits changed {parts['hat_changed']}; the other tracks "
        f"unchanged {parts['others_unchanged']}; the host was told for {sorted(Path(n).stem for n in told)}; WAV files of the hat in the cache {wavs} (want one: the old one is deleted)"
    )


def update_run(label: str, env: dict | None = None) -> tuple[int, list]:
    new = alt_hat()
    rc, _text, lines = host_run(
        label,
        CLAP,
        ["grow", "--files", *fixture_args(FILES), "--batch", str(len(FILES)), "--no-request", "--update-content", f"3:{new}", "--wait", PANIC_WAIT],
        f"cache-{label}",
        extra=env,
    )
    return rc, lines


def update_refs() -> tuple[bytes, bytes] | None:
    ref_old = (OUT / "events-reference.json").read_bytes() if (OUT / "events-reference.json").exists() else None
    ref_new = reference_alone("m1b4-ref-updated", [*FILES[:3], str(alt_hat())])
    return (ref_old, ref_new) if ref_old and ref_new else None


def stage_no_request_update() -> bool:
    """M1: new audio after a content update of the host is read and analysed, in a host that never requests."""
    if not HOST.exists() or not CLAP.exists() or not SERVICE_TOOL.exists():
        return pass_fail("no-request-update", False, "no host, no CLAP or no service_tool: run check.sh build first")
    refs = update_refs()
    if refs is None:
        return pass_fail("no-request-update", False, "a reference failed")
    rc, lines = update_run("no-request-update")
    parts, evidence = update_parts("no-request-update", rc, lines, *refs)
    return pass_fail("no-request-update", all(parts.values()), evidence)


def stage_no_request_update_fault() -> bool:
    """The fault run: updateAudioSourceContent does nothing (the old way): no new round, the old hits stay."""
    if not HOST.exists() or not CLAP.exists() or not SERVICE_TOOL.exists():
        return pass_fail("no-request-update-fault", False, "no host, no CLAP or no service_tool: run check.sh build first")
    refs = update_refs()
    if refs is None:
        return pass_fail("no-request-update-fault", False, "a reference failed")
    rc, lines = update_run("no-request-update-fault", {"DS_ARA_ROUND_FAULT": "ignore-update"})
    parts, evidence = update_parts("no-request-update-fault", rc, lines, *refs)
    return pass_fail(
        "no-request-update-fault",
        not parts["one_more_round"] and not parts["events_new"],
        f"the fault switch ignored the content update and the check refused it (no new round, old events): {evidence}",
    )


BETWEEN_EXTRA = TOM1_FILE


def between_parts(label: str, rc: int, ref_new: bytes) -> tuple[dict, str]:
    jobs = job_rows(label)
    parts = {
        "rc": rc == 0,
        "rounds": len(jobs) == 3,
        "events_new": len(jobs) >= 1 and jobs[-1].get("events_json", "").encode() == ref_new,
    }
    return parts, (
        f"the host updated the audio of the hat with its samples access off, a new source was analysed meanwhile (a round on the OLD hat), then the access came back: "
        f"{len(jobs)} round(s) (want 3), events at the end byte for byte the reference of the NEW audio {parts['events_new']}"
    )


def between_run(label: str, env: dict | None = None) -> tuple[int, bytes | None]:
    ensure_bleed_fixture()
    new = alt_hat()
    extra = FIXTURES / BETWEEN_EXTRA
    ref = reference_alone("m1b4-ref-between", [*FILES[:3], str(new), str(extra)])
    rc, _text, _lines = host_run(
        label,
        CLAP,
        ["grow", "--files", *fixture_args(FILES), "--batch", str(len(FILES)), "--no-request", "--update-content", f"3:{new}", "--update-add", str(extra), "--wait", PANIC_WAIT],
        f"cache-{label}",
        extra=env,
    )
    return rc, ref


def stage_no_request_update_between() -> bool:
    """R1: a round that runs before the new audio is read must not mark the source done."""
    if not HOST.exists() or not CLAP.exists() or not SERVICE_TOOL.exists():
        return pass_fail("no-request-update-between", False, "no host, no CLAP or no service_tool: run check.sh build first")
    rc, ref = between_run("no-request-update-between")
    if ref is None:
        return pass_fail("no-request-update-between", False, "the reference failed")
    parts, evidence = between_parts("no-request-update-between", rc, ref)
    return pass_fail("no-request-update-between", all(parts.values()), evidence)


def stage_no_request_update_between_fault() -> bool:
    """The fault run: the mark goes after a round on the old audio: the hat keeps its old hits."""
    if not HOST.exists() or not CLAP.exists() or not SERVICE_TOOL.exists():
        return pass_fail("no-request-update-between-fault", False, "no host, no CLAP or no service_tool: run check.sh build first")
    rc, ref = between_run("no-request-update-between-fault", {"DS_ARA_ROUND_FAULT": "clear-stale"})
    if ref is None:
        return pass_fail("no-request-update-between-fault", False, "the reference failed")
    parts, evidence = between_parts("no-request-update-between-fault", rc, ref)
    return pass_fail(
        "no-request-update-between-fault",
        not parts["events_new"],
        f"the fault switch cleared the mark after a round on the old audio and the check refused it: {evidence}",
    )


def failed_read_parts(label: str, rc: int, ref_new: bytes) -> tuple[dict, str]:
    jobs = job_rows(label)
    failed = event_rows(label, "source_read_failed")
    parts = {
        "rc": rc == 0,
        "named": len(failed) >= 1,
        # a round may run on the old audio while the read is out (the changed snapshot); the last one has the new audio
        "events_new": len(jobs) >= 2 and jobs[-1].get("events_json", "").encode() == ref_new,
    }
    return parts, (
        f"the first read of the new audio failed: {len(failed)} source_read_failed row(s) (retry {[r.get('retry') for r in failed]}), {len(jobs)} round(s) (at least 2), "
        f"events at the end byte for byte the reference of the new audio {parts['events_new']}"
    )


def failed_read_run(label: str, env: dict | None = None) -> tuple[int, bytes | None]:
    new = alt_hat()
    ref = reference_alone("m1b4-ref-updated", [*FILES[:3], str(new)])
    rc, _text, _lines = host_run(
        label,
        CLAP,
        ["grow", "--files", *fixture_args(FILES), "--batch", str(len(FILES)), "--no-request", "--update-content", f"3:{new}", "--update-fail-reads", "1", "--linger-ms", "4000", "--wait", PANIC_WAIT],
        f"cache-{label}",
        extra={"DS_ARA_RETRY_MS": "300", **(env or {})},
    )
    return rc, ref


def stage_no_request_update_failed_read() -> bool:
    """R3: a failed read of the new audio is named and tried again."""
    if not HOST.exists() or not CLAP.exists() or not SERVICE_TOOL.exists():
        return pass_fail("no-request-update-failed-read", False, "no host, no CLAP or no service_tool: run check.sh build first")
    rc, ref = failed_read_run("no-request-update-failed-read")
    if ref is None:
        return pass_fail("no-request-update-failed-read", False, "the reference failed")
    parts, evidence = failed_read_parts("no-request-update-failed-read", rc, ref)
    return pass_fail("no-request-update-failed-read", all(parts.values()), evidence)


def stage_no_request_update_failed_read_fault() -> bool:
    """The fault run: no new try of the read: the old hits stay."""
    if not HOST.exists() or not CLAP.exists() or not SERVICE_TOOL.exists():
        return pass_fail("no-request-update-failed-read-fault", False, "no host, no CLAP or no service_tool: run check.sh build first")
    rc, ref = failed_read_run("no-request-update-failed-read-fault", {"DS_ARA_ROUND_FAULT": "no-read-retry"})
    if ref is None:
        return pass_fail("no-request-update-failed-read-fault", False, "the reference failed")
    parts, evidence = failed_read_parts("no-request-update-failed-read-fault", rc, ref)
    return pass_fail(
        "no-request-update-failed-read-fault",
        not parts["events_new"],
        f"the fault switch made no new try of the read and the check refused it: {evidence}",
    )


def forever_parts(label: str, rc: int, text: str) -> tuple[dict, str]:
    jobs = job_rows(label)
    failed = event_rows(label, "source_read_failed")
    hat = track_names([HAT])[0]
    first = json.loads(jobs[0]["events_json"]) if jobs else {}
    last = json.loads(jobs[-1]["events_json"]) if jobs else {}
    waited = "did not finish" in text
    parts = {
        "rc": rc == 0 and not waited,
        "named": len(failed) == 4 and failed[-1].get("retry") is False,
        "old_hits": bool(last.get(hat)) and last.get(hat) == first.get(hat),
        "added": len(jobs) >= 2 and len(last) == len(FILES) + 1,
    }
    return parts, (
        f"the read of the new audio failed for good ({len(failed)} source_read_failed rows, the last with retry {failed[-1].get('retry') if failed else None}), then another source was added: "
        f"the host's wait {'did not end' if waited else 'ended (every source complete)'}, the hat keeps its old hits {parts['old_hits']}, {len(jobs)} round(s), {len(last)} tracks"
    )


def forever_run(label: str, env: dict | None = None) -> tuple[int, str]:
    new = alt_hat()
    ensure_bleed_fixture()
    rc, text, _lines = host_run(
        label,
        CLAP,
        ["grow", "--files", *fixture_args(FILES), "--batch", str(len(FILES)), "--no-request", "--update-content", f"3:{new}", "--update-fail-reads", "99",
         "--linger-ms", "4000", "--add-late", str(FIXTURES / BETWEEN_EXTRA), "--wait", PANIC_WAIT],
        f"cache-{label}",
        extra={"DS_ARA_RETRY_MS": "300", **(env or {})},
    )
    return rc, text


def stage_no_request_update_failed_for_good() -> bool:
    """T1: a read of new audio that failed for good leaves the source complete-and-failed, also after another round."""
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("no-request-update-failed-for-good", False, "no host or no CLAP: run check.sh build first")
    rc, text = forever_run("no-request-update-failed-for-good")
    parts, evidence = forever_parts("no-request-update-failed-for-good", rc, text)
    return pass_fail("no-request-update-failed-for-good", all(parts.values()), evidence)


def stage_no_request_update_failed_for_good_fault() -> bool:
    """The fault run: a later round clears the failed mark: the source is incomplete for ever and the host waits."""
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("no-request-update-failed-for-good-fault", False, "no host or no CLAP: run check.sh build first")
    rc, text = forever_run("no-request-update-failed-for-good-fault", {"DS_ARA_ROUND_FAULT": "clear-failed"})
    parts, evidence = forever_parts("no-request-update-failed-for-good-fault", rc, text)
    return pass_fail(
        "no-request-update-failed-for-good-fault",
        not parts["rc"],
        f"the fault switch cleared the failed mark in a later round and the check refused it (the host waited for ever): {evidence}",
    )


def twice_parts(label: str, rc: int, ref_new: bytes) -> tuple[dict, str]:
    jobs = job_rows(label)
    failed = event_rows(label, "source_read_failed")
    parts = {
        "rc": rc == 0,
        "no_failure": len(failed) == 0,
        "events_new": len(jobs) >= 2 and jobs[-1].get("events_json", "").encode() == ref_new,
    }
    return parts, (
        f"two updates in a row, the first read was cancelled by the second: {len(failed)} source_read_failed row(s) (want 0: a cancelled read of an old generation is no failure), "
        f"events at the end byte for byte the reference of the new audio {parts['events_new']}"
    )


def twice_run(label: str, env: dict | None = None) -> tuple[int, bytes | None]:
    new = alt_hat()
    ref = reference_alone("m1b4-ref-updated", [*FILES[:3], str(new)])
    rc, _text, _lines = host_run(
        label,
        CLAP,
        ["grow", "--files", *fixture_args(FILES), "--batch", str(len(FILES)), "--no-request", "--slow-read", "3:500", "--update-content", f"3:{new}", "--update-twice", "--wait", PANIC_WAIT],
        f"cache-{label}",
        extra={"DS_ARA_RETRY_MS": "300", **(env or {})},
    )
    return rc, ref


def stage_no_request_update_twice() -> bool:
    """T3: a read that a second update cancelled burns no retry and writes no failure row."""
    if not HOST.exists() or not CLAP.exists() or not SERVICE_TOOL.exists():
        return pass_fail("no-request-update-twice", False, "no host, no CLAP or no service_tool: run check.sh build first")
    rc, ref = twice_run("no-request-update-twice")
    if ref is None:
        return pass_fail("no-request-update-twice", False, "the reference failed")
    parts, evidence = twice_parts("no-request-update-twice", rc, ref)
    return pass_fail("no-request-update-twice", all(parts.values()), evidence)


def stage_no_request_update_twice_fault() -> bool:
    """The fault run: the cancelled read counts as a failed read."""
    if not HOST.exists() or not CLAP.exists() or not SERVICE_TOOL.exists():
        return pass_fail("no-request-update-twice-fault", False, "no host, no CLAP or no service_tool: run check.sh build first")
    rc, ref = twice_run("no-request-update-twice-fault", {"DS_ARA_ROUND_FAULT": "cancel-fails"})
    if ref is None:
        return pass_fail("no-request-update-twice-fault", False, "the reference failed")
    parts, evidence = twice_parts("no-request-update-twice-fault", rc, ref)
    return pass_fail(
        "no-request-update-twice-fault",
        not parts["no_failure"],
        f"the fault switch counted the cancelled read as a failure and the check refused it: {evidence}",
    )


def files_parts(label: str, rc: int) -> tuple[dict, str]:
    wavs = sorted(p.name for p in (OUT / f"cache-{label}").rglob("*.wav"))
    parts = {"rc": rc == 0, "live_only": [w[:3] for w in wavs] == ["000", "001", "002"]}
    return parts, (
        f"the audio of the hat was updated, its new read kept failing, then the hat was destroyed: WAV files in the cache {wavs} "
        "(want one for each of the 3 live sources and none for the destroyed one, not even the file of its first audio)"
    )


def files_run(label: str, env: dict | None = None) -> int:
    rc, _text, _lines = host_run(
        label,
        CLAP,
        ["grow", "--files", *fixture_args(FILES), "--batch", str(len(FILES)), "--no-request", "--update-content", f"3:{alt_hat()}", "--update-fail-reads", "99", "--destroy", "3", "--pump-ms", "1500", "--wait", PANIC_WAIT],
        f"cache-{label}",
        extra={"DS_ARA_RETRY_MS": "300", **(env or {})},
    )
    return rc


def stage_no_request_files() -> bool:
    """R4: a destroyed source leaves no WAV in the cache (also the file of an earlier generation of its audio)."""
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("no-request-files", False, "no host or no CLAP: run check.sh build first")
    parts, evidence = files_parts("no-request-files", files_run("no-request-files"))
    return pass_fail("no-request-files", all(parts.values()), evidence)


def stage_no_request_files_fault() -> bool:
    """The fault run: the files of a destroyed source stay."""
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("no-request-files-fault", False, "no host or no CLAP: run check.sh build first")
    rc = files_run("no-request-files-fault", {"DS_ARA_ROUND_FAULT": "keep-wavs"})
    parts, evidence = files_parts("no-request-files-fault", rc)
    return pass_fail("no-request-files-fault", rc == 0 and not parts["live_only"], f"the fault switch kept the files and the check refused it: {evidence}")


def retry_parts_m2(label: str, rc: int, lines: list, ref: bytes, expect_ok: bool) -> tuple[dict, str]:
    failed = event_rows(label, "job_failed")
    retries = event_rows(label, "retry")
    gave_up = event_rows(label, "retry_gave_up")
    jobs = job_rows(label)
    after = host_notes_of(lines, "after-linger")
    parts = {
        "rc": rc == 0,
        "failed": len(failed) >= 1,
        "result": len(jobs) == 1 and jobs[0].get("events_json", "").encode() == ref,
        "notes": len(after) == len(FILES) and all(after.values()),
        "retried": len(retries) >= 1,
    }
    return parts, (
        f"the first round failed ({len(failed)} job_failed row(s)) and the host never asked again: {len(retries)} retry row(s) of the plugin, "
        f"{len(jobs)} result round(s), events byte for byte the reference {parts['result']}, notes after it {[len(v) for v in after.values()]}, "
        f"{len(gave_up)} retry_gave_up row(s)"
    )


def retry_run_m2(label: str, rounds: int, env: dict | None = None, linger: str = "6000") -> tuple[int, list]:
    rc, _text, lines = host_run(
        label,
        CLAP,
        ["grow", "--files", *fixture_args(FILES), "--batch", str(len(FILES)), "--no-request", "--linger-ms", linger, "--wait", HOST_WAIT],
        f"cache-{label}",
        extra={"DS_ARA_SCENARIO": f"fail-rounds:{rounds}", "DS_ARA_RETRY_MS": "300", **(env or {})},
    )
    return rc, lines


def stage_no_request_retry() -> bool:
    """M2: a failed round is tried again by the plugin when the host never requests."""
    ref_path = OUT / "events-reference.json"
    if not HOST.exists() or not CLAP.exists() or not ref_path.exists():
        return pass_fail("no-request-retry", False, "no host, no CLAP or no reference events: run check.sh build and the reference stage first")
    rc, lines = retry_run_m2("no-request-retry", 1)
    parts, evidence = retry_parts_m2("no-request-retry", rc, lines, ref_path.read_bytes(), True)
    return pass_fail("no-request-retry", all(parts.values()), evidence)


def stage_no_request_retry_fault() -> bool:
    """The fault run: no retry by the plugin: the track stays without hits."""
    ref_path = OUT / "events-reference.json"
    if not HOST.exists() or not CLAP.exists() or not ref_path.exists():
        return pass_fail("no-request-retry-fault", False, "no host, no CLAP or no reference events")
    rc, lines = retry_run_m2("no-request-retry-fault", 1, {"DS_ARA_ROUND_FAULT": "no-retry"}, "3000")
    parts, evidence = retry_parts_m2("no-request-retry-fault", rc, lines, ref_path.read_bytes(), True)
    return pass_fail(
        "no-request-retry-fault",
        rc == 0 and parts["failed"] and not parts["result"] and not parts["retried"],
        f"the fault switch gave no retry and the check refused it: {evidence}",
    )


def limit_parts(label: str, rc: int) -> tuple[dict, str]:
    failed = event_rows(label, "job_failed")
    gave_up = event_rows(label, "retry_gave_up")
    retries = event_rows(label, "retry")
    parts = {
        "rc": rc == 0,
        "three_retries": len(retries) == 3,
        "bounded": len(failed) == 4,
        "gave_up": len(gave_up) == 1,
    }
    return parts, f"every round fails: {len(failed)} failed round(s) (want 4: the first and three retries), {len(retries)} retry row(s), {len(gave_up)} retry_gave_up row(s)"


def stage_no_request_retry_limit() -> bool:
    """M2: the retries are bounded: three, then a row and a stop."""
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("no-request-retry-limit", False, "no host or no CLAP: run check.sh build first")
    rc, _lines = retry_run_m2("no-request-retry-limit", 99, linger="7000")
    parts, evidence = limit_parts("no-request-retry-limit", rc)
    return pass_fail("no-request-retry-limit", all(parts.values()), evidence)


def stage_no_request_retry_limit_fault() -> bool:
    """The fault run: the plugin never gives up."""
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("no-request-retry-limit-fault", False, "no host or no CLAP: run check.sh build first")
    rc, _lines = retry_run_m2("no-request-retry-limit-fault", 99, {"DS_ARA_ROUND_FAULT": "retry-forever"}, "7000")
    parts, evidence = limit_parts("no-request-retry-limit-fault", rc)
    return pass_fail(
        "no-request-retry-limit-fault",
        rc == 0 and not parts["bounded"] and not parts["gave_up"],
        f"the fault switch never gave up and the check refused it: {evidence}",
    )


def cancel_parts(label: str, rc: int) -> tuple[dict, str]:
    jobs = job_rows(label)
    rounds = services_row(label).get("rounds")
    late = event_rows(label, "source_late")
    parts = {
        "rc": rc == 0,
        "one_round": rounds == 1 and len(jobs) == 1,
        "two_tracks": len(jobs) == 1 and len(jobs[0].get("analysed") or []) == 2,
        "not_late": len(late) == 0,
    }
    return parts, (
        f"two sources analysed, then the host turned the samples access of a third off while its read ran: {rounds} round(s) in all (want 1: no round for "
        f"audio that did not come), {len(jobs)} job row(s), {len(late)} source_late row(s) (want 0: the read failed, it is not late)"
    )


def cancel_run(label: str, env: dict | None = None) -> int:
    rc, _text, _lines = host_run(
        label,
        CLAP,
        ["grow", "--files", *fixture_args(FILES[:3]), "--batch", "2", "--no-request", "--slow-read", "2:700", "--cancel-read", "round-2", "--linger-ms", "1500", "--wait", PANIC_WAIT],
        f"cache-{label}",
        extra=env,
    )
    return rc


def stage_no_request_cancel() -> bool:
    """M3: a cancelled read does not force a round of the complete tracks."""
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("no-request-cancel", False, "no host or no CLAP: run check.sh build first")
    parts, evidence = cancel_parts("no-request-cancel", cancel_run("no-request-cancel"))
    return pass_fail("no-request-cancel", all(parts.values()), evidence)


def stage_no_request_cancel_fault() -> bool:
    """The fault run: a source with a failed read forces a round."""
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("no-request-cancel-fault", False, "no host or no CLAP: run check.sh build first")
    rc = cancel_run("no-request-cancel-fault", {"DS_ARA_ROUND_FAULT": "force-failed"})
    parts, evidence = cancel_parts("no-request-cancel-fault", rc)
    return pass_fail(
        "no-request-cancel-fault",
        rc == 0 and not parts["one_round"],
        f"the fault switch forced a round for the cancelled read and the check refused it: {evidence}",
    )


def spawn_parts(label: str, rc: int, text: str) -> tuple[dict, str]:
    late = event_rows(label, "source_late")
    waited = "did not finish" in text
    parts = {"rc": rc == 0, "not_late": len(late) == 0, "no_wait": not waited}
    return parts, (
        f"the reader threads cannot start (rc={rc}): the host {'timed out waiting for the analysis' if waited else 'saw the sources complete'} (want: complete within 8 s, "
        f"not at the 30 s audio deadline), {len(late)} source_late row(s) (want 0)"
    )


def spawn_run(label: str, env: dict | None = None) -> tuple[int, str]:
    rc, text, _lines = host_run(
        label,
        CLAP,
        ["grow", "--files", *fixture_args(FILES[:2]), "--batch", "2", "--no-request", "--wait", PANIC_WAIT],
        f"cache-{label}",
        extra={"DS_ARA_SCENARIO": "no-reader-thread", **(env or {})},
    )
    return rc, text


def stage_no_request_spawn() -> bool:
    """M3: a reader thread that cannot start ends the read as failed at once (no wait for the audio deadline)."""
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("no-request-spawn", False, "no host or no CLAP: run check.sh build first")
    rc, text = spawn_run("no-request-spawn")
    parts, evidence = spawn_parts("no-request-spawn", rc, text)
    return pass_fail("no-request-spawn", all(parts.values()), evidence)


def stage_no_request_spawn_fault() -> bool:
    """The fault run: no `Failed` message: the sources wait for the audio deadline and the host times out."""
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("no-request-spawn-fault", False, "no host or no CLAP: run check.sh build first")
    rc, text = spawn_run("no-request-spawn-fault", {"DS_ARA_ROUND_FAULT": "no-spawn-failed"})
    parts, evidence = spawn_parts("no-request-spawn-fault", rc, text)
    return pass_fail(
        "no-request-spawn-fault",
        not parts["no_wait"],
        f"the fault switch sent no message for the reader that could not start and the check refused it: {evidence}",
    )


def state_parts(label: str, rc: int, lines: list) -> tuple[dict, str]:
    row = next((r for r in lines if r.get("event") == "plugin_state"), {})
    keys = ["load_own", "load_empty", "load_newer", "load_garbage"]
    # M1b10: the report has a row for every call of the host to the state (kind, bytes, answer), so a run in
    # a real host says whether it calls the state at all
    calls = [(r.get("kind"), r.get("bytes"), r.get("ok")) for r in report_rows(label) if r.get("event") == "state_call"]
    parts = {
        "has_state": row.get("has_state") is True,
        "saved": row.get("save_ok") is True and row.get("versioned") is True,
        "loads": all(row.get(k) is True for k in keys),
        "rows": [c[:2] for c in calls] == [("save", 10), ("load", 10), ("load", 0), ("load", 10), ("load", 26)]
        and all(c[2] is True for c in calls),
    }
    return parts, (
        f"the plugin has clap.state ({row.get('has_state')}); save wrote {row.get('saved_bytes')} bytes with the magic and a version ({row.get('versioned')}); "
        f"load answered true for its own blob {row.get('load_own')}, an empty stream {row.get('load_empty')}, a newer version {row.get('load_newer')} and garbage {row.get('load_garbage')}; "
        f"the report has {len(calls)} state_call rows {calls} (rc={rc})"
    )


def stage_state() -> bool:
    """M1b4 gate line 10: the plugin has `clap.state`. The CLAP path: the test host saves and loads. The VST3
    wrapper maps getState and setState to the same two functions; no tool in `.deps` calls them on the VST3
    alone (pluginval runs a state test only on its own terms), so the two functions have unit tests in the
    crate (stage `crate`), and clap-validator (stage `validate`) runs its state tests on the CLAP."""
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("state", False, "no host or no CLAP: run check.sh build first")
    rc, _text, lines = host_run("state", CLAP, ["state"], "cache-state")
    parts, evidence = state_parts("state", rc, lines)
    return pass_fail("state", rc == 0 and all(parts.values()), evidence)


def stage_state_fault() -> bool:
    """The fault run: the load refuses an empty stream, a newer version and garbage (LUNA then fails to make the instance)."""
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("state-fault", False, "no host or no CLAP: run check.sh build first")
    rc, _text, lines = host_run("state-fault", CLAP, ["state"], "cache-state-fault", extra={"DS_ARA_STATE_FAULT": "strict"})
    parts, evidence = state_parts("state-fault", rc, lines)
    ok = pass_fail(
        "state-fault",
        parts["saved"] and not parts["loads"] and rc != 0,
        f"the fault switch made the load refuse what it did not write and the check refused it: {evidence}",
    )
    # M1b10: a plugin that leaves the state calls out of the report fails the rows part
    rc, _text, lines = host_run("state-fault-rows", CLAP, ["state"], "cache-state-fault-rows", extra={"DS_ARA_STATE_FAULT": "no-rows"})
    parts, evidence = state_parts("state-fault-rows", rc, lines)
    return pass_fail(
        "state-fault-rows",
        parts["saved"] and parts["loads"] and not parts["rows"],
        f"the fault switch (no report row for a call of the host to the state) made the rows part fail: {evidence}",
    ) and ok


# ---- review round 2, item 8: one list of the DS_ARA_* variables

README = HERE / "README.md"
README_BEGIN, README_END = "<!-- switches:begin (made from switches.json by `check.py write-readme`) -->", "<!-- switches:end -->"
SOURCE_GLOBS = ["src/*.rs", "src/bin/*.rs", "check.sh", "build.rs"]  # what the plugin and the script read, not the stages


def readme_block() -> str:
    kinds = {
        "config": "The plugin reads it in every build, the delivery build too (the check sets it for every run; the hand checks in a DAW need it).",
        "check": "Read by the check only.",
        "fault": "Fault switch: breaks one line of the gate on purpose, so the check of that line can fail. Read only by a build with the cargo feature `test-switches` (the gate build); a delivery build ignores it.",
        "scenario": "Scenario switch: sets up a case for a stage (not a fault). Read only by a build with the cargo feature `test-switches`; a delivery build ignores it.",
    }
    lines = [README_BEGIN, "", "| Variable | Kind | Values | What it does |", "| --- | --- | --- | --- |"]
    for e in load_switches():
        lines.append(f"| `{e['name']}` | {e['kind']} | {e['values']} | {e['meaning']} |")
    lines += ["", "Kinds: " + " ".join(f"**{k}**: {v}" for k, v in kinds.items()), "", README_END]
    return "\n".join(lines)


def write_readme() -> None:
    text = README.read_text(encoding="utf-8")
    a, b = text.index(README_BEGIN), text.index(README_END) + len(README_END)
    README.write_text(text[:a] + readme_block() + text[b:], encoding="utf-8")


def names_in_text(text: str) -> set:
    return set(re.findall(r"DS_ARA_[A-Z0-9_]+", text))


def switches_problems(listed: set, source_text: str, readme_text: str) -> list[str]:
    """What is wrong with the list: a variable in the source or the README that the list does not
    name, a listed variable no source uses, or a README block that is not made from the list."""
    problems = []
    used = names_in_text(source_text)
    problems += [f"{n} is used but not listed" for n in sorted(used - listed)]
    problems += [f"{n} is listed but not used" for n in sorted(listed - used)]
    problems += [f"{n} is in the README but not listed" for n in sorted(names_in_text(readme_text) - listed)]
    if README_BEGIN not in readme_text or readme_block() not in readme_text:
        problems.append("the README block is not the one made from switches.json")
    return problems


def stage_switches() -> bool:
    if not (HERE / "switches.json").exists():
        return pass_fail("switches", False, "no switches.json")
    listed = {e["name"] for e in load_switches()}
    source = "\n".join(p.read_text(encoding="utf-8") for g in SOURCE_GLOBS for p in sorted(HERE.glob(g)))
    problems = switches_problems(listed, source, README.read_text(encoding="utf-8"))
    ok = pass_fail("switches-list", not problems, f"{len(listed)} variables; the source, the README and check.sh use no other; the README block is made from the list" if not problems else "; ".join(problems))
    # a normal run clears every one of them, even when the caller's environment holds them all
    saved = {k: os.environ.get(k) for k in listed}
    try:
        for k in listed:
            os.environ[k] = "x"
        cleared = no_switch()
        run_env = dict(os.environ)
        for k, v in cleared.items():
            if v is None:
                run_env.pop(k, None)
    finally:
        for k, v in saved.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v
    left = sorted(k for k in listed if k in run_env and next(e for e in load_switches() if e["name"] == k)["kind"] != "check")
    ok = pass_fail("switches-clear", not left, f"a normal run clears all {len(cleared)} plugin variables even when all {len(listed)} are set" if not left else f"left set: {left}") and ok
    # check.sh refuses a fault or test variable in the caller's environment, at the start
    guard = HERE / "check.sh"
    rc_bad, text_bad = run([str(guard), "guard"], env={"DS_ARA_ROUND_FAULT": "new-only"})
    rc_ok, _text = run([str(guard), "guard"], env={k: None for k in listed})
    ok = pass_fail(
        "switches-guard",
        rc_bad != 0 and "DS_ARA_ROUND_FAULT" in text_bad and rc_ok == 0,
        f"check.sh guard: rc={rc_bad} naming the variable with DS_ARA_ROUND_FAULT set, rc={rc_ok} without",
    ) and ok
    return ok


def stage_switches_fault() -> bool:
    """The check can fail: an unlisted variable in the source, a listed one nobody uses, and a
    README that was edited by hand are all reported."""
    listed = {e["name"] for e in load_switches()}
    source = "\n".join(p.read_text(encoding="utf-8") for g in SOURCE_GLOBS for p in sorted(HERE.glob(g)))
    readme = README.read_text(encoding="utf-8")
    unlisted = switches_problems(listed, source + '\nstd::env::var("DS_ARA_NEW_FAULT")', readme)
    unused = switches_problems(listed | {"DS_ARA_GHOST"}, source, readme)
    edited = switches_problems(listed, source, readme.replace("| `DS_ARA_CACHE` |", "| `DS_ARA_CACHE_X` |"))
    ok = (
        any("DS_ARA_NEW_FAULT is used but not listed" in p for p in unlisted)
        and any("DS_ARA_GHOST is listed but not used" in p for p in unused)
        and any("README" in p for p in edited)
    )
    return pass_fail("switches-fault", ok, f"an unlisted variable: {bool(unlisted)}, a listed variable nobody uses: {bool(unused)}, a README edited by hand: {bool(edited)} - each reported")


# ---- review round 2, item 9: no unwrap and no expect in the non-test code of the plugin

UNWRAP_FILES = ["src/deliver.rs", "src/editor.rs", "src/firstopen.rs", "src/lib.rs", "src/musical.rs", "src/playback.rs", "src/rows.rs", "src/service.rs", "src/state.rs", "src/tempo_ara.rs", "src/transport.rs", "src/wav.rs"]
UNWRAP_PATTERN = re.compile(r"\.(unwrap|expect)\s*\(|\.unwrap_unchecked\s*\(")


def unwraps_in(text: str) -> list[tuple[int, str]]:
    """The lines of the non-test code that unwrap or expect: the code up to the first `#[cfg(test)]`,
    without comment lines (the host tables are optional input; no panic on a missing function)."""
    found = []
    for n, line in enumerate(text.splitlines(), 1):
        if line.strip().startswith("#[cfg(test)]"):
            break
        if line.strip().startswith("//"):
            continue
        code = line.split("//", 1)[0]
        if UNWRAP_PATTERN.search(code):
            found.append((n, line.strip()))
    return found


HOST_TABLE_DEREF = re.compile(r"&\*\s*\(\s*\*[^;]*\.(archivingControllerInterface|audioAccessControllerInterface)\b")


def table_derefs_in(text: str) -> list[tuple[int, str]]:
    """The lines of the non-test code that dereference a host table straight from the host
    instance, with no null check in between (`&*(*host).archivingControllerInterface`): the table
    is read into a variable, checked, then dereferenced. A pattern check: it finds that form, it
    cannot prove that every other access is checked."""
    found = []
    for n, line in enumerate(text.splitlines(), 1):
        if line.strip().startswith("#[cfg(test)]"):
            break
        code = line.split("//", 1)[0]
        if HOST_TABLE_DEREF.search(code):
            found.append((n, line.strip()))
    return found


def stage_no_unwrap() -> bool:
    bad = {f: unwraps_in((HERE / f).read_text(encoding="utf-8")) + table_derefs_in((HERE / f).read_text(encoding="utf-8")) for f in UNWRAP_FILES}
    n = sum(len(v) for v in bad.values())
    return pass_fail(
        "no-unwrap",
        n == 0,
        f"0 unwrap or expect calls in the non-test code of {len(UNWRAP_FILES)} files"
        if n == 0
        else "; ".join(f"{f}:{ln}: {t}" for f, v in bad.items() for ln, t in v[:3]),
    )


def stage_no_unwrap_fault() -> bool:
    """The check can fail: a host-table unwrap and an expect in non-test code are found, and the
    same calls after `#[cfg(test)]` and in comments are not."""
    sample = (
        "fn a() {\n    let f = iface.createAudioReaderForSource.unwrap();\n    // x.unwrap() in a comment\n"
        "    let y = z.expect(\"no\");\n    let ok = w.unwrap_or(1);\n}\n#[cfg(test)]\nmod t { fn b() { x.unwrap(); } }\n"
    )
    found = unwraps_in(sample)
    deref = table_derefs_in("fn a() {\n    let a = &*(*ctl.host).archivingControllerInterface;\n    let t = (*h).audioAccessControllerInterface;\n}\n")
    return pass_fail(
        "no-unwrap-fault",
        [n for n, _ in found] == [2, 4] and [n for n, _ in deref] == [2],
        f"found the unwrap and the expect on lines {[n for n, _ in found]} of the sample, not the comment, unwrap_or or the test code; "
        f"found the direct dereference of a host table on line {[n for n, _ in deref]}, not the checked read",
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


PLUGINVAL_STRICTNESS = "10"


def pluginval_run(label: str, vst3: Path) -> tuple[int, str]:
    """pluginval at strictness 10 on one VST3 (no CLAP_PATH: the wrapper finds the CLAP in the bundle)."""
    env = {**no_switch(), "DS_ARA_CACHE": str(OUT / f"cache-{label}"), "CLAP_PATH": None}
    return run(
        [str(PLUGINVAL), "--strictness-level", PLUGINVAL_STRICTNESS, "--timeout-ms", "120000", str(vst3)],
        env=env,
        log=f"{label}.log",
    )


def pluginval_problems(rc: int, text: str) -> list[str]:
    """What is wrong with one pluginval run: a failure, a level other than 10, no word of success,
    or a log that does not name the plugin (the scan must have driven THIS plugin)."""
    bad = []
    if rc != 0:
        bad.append(f"rc={rc}")
    if f"Strictness level: {PLUGINVAL_STRICTNESS}" not in text:
        bad.append(f"the log does not say strictness level {PLUGINVAL_STRICTNESS}")
    if "SUCCESS" not in text:
        bad.append("the log has no SUCCESS")
    if "Drum Spike ARA" not in text:
        bad.append("the log does not name the plugin (is the CLAP inside?)")
    return bad


def stage_pluginval() -> bool:
    """Gate line 1 of M1d1: pluginval at strictness 10 on the wrapped VST3, the gate build and the
    delivery build (pluginval is no ARA host: no document controller, so no analysis runs here)."""
    ok = True
    for label, vst3 in (("pluginval", VST3), ("pluginval-delivery", DELIVERY_VST3)):
        if not PLUGINVAL.exists() or not vst3.exists():
            return pass_fail(label, False, f"no pluginval or no VST3 ({PLUGINVAL}, {vst3})")
        rc, text = pluginval_run(label, vst3)
        bad = pluginval_problems(rc, text)
        ver = re.search(r"Drum Spike ARA v([0-9][^\s]*)", text)
        ok = pass_fail(label, not bad, (
            f"strictness {PLUGINVAL_STRICTNESS} on the wrapped VST3 ({'delivery' if 'delivery' in label else 'gate'} build), rc={rc}, "
            f"the scan names the plugin version {ver.group(1) if ver else '?'} (out/{label}.log)") if not bad else "; ".join(bad) + f" (out/{label}.log)") and ok
    return ok


def stage_pluginval_fault() -> bool:
    """The stage can fail: a VST3 whose CLAP was taken out of the bundle (the wrapper finds no plugin),
    and a log that says another strictness level."""
    if not PLUGINVAL.exists() or not VST3.exists():
        return pass_fail("pluginval-fault", False, "no pluginval or no VST3")
    tmp = OUT / "pluginval-fault"
    if tmp.exists():
        shutil.rmtree(tmp)
    tmp.mkdir(parents=True)
    broken = tmp / "DrumSpikeAra.vst3"
    shutil.copytree(VST3, broken, symlinks=True)
    shutil.rmtree(broken / "Contents" / "PlugIns")
    rc, text = pluginval_run("pluginval-fault", broken)
    bad = pluginval_problems(rc, text)
    low = pluginval_problems(0, "Strictness level: 5\nSUCCESS\nDrum Spike ARA")
    return pass_fail(
        "pluginval-fault",
        bool(bad) and rc != 0 and any("strictness level" in b for b in low),
        f"a bundle without its CLAP is refused ({'; '.join(bad)}), and a log of strictness 5 is refused ({'; '.join(low)})",
    )


def stage_validate() -> bool:
    if not CLAP_VALIDATOR.exists() or not CLAP.exists():
        return pass_fail("validate", False, f"no clap-validator or no CLAP ({CLAP_VALIDATOR}, {CLAP})")
    env = {**no_switch(), "DS_ARA_CACHE": str(OUT / "cache-validate")}
    rc, _ = run([str(CLAP_VALIDATOR), "validate", str(CLAP)], env=env, log="clap-validator.log")
    return pass_fail("validate", rc == 0, f"clap-validator on the CLAP bundle, rc={rc} (out/clap-validator.log)")


def stage_universal() -> bool:
    ok = True
    delivery = (("delivery CLAP", DELIVERY_CLAP / "Contents" / "MacOS" / "DrumSpikeAra"), ("delivery VST3", DELIVERY_VST3 / "Contents" / "MacOS" / "DrumSpikeAra"))
    for name, binary in (("CLAP", CLAP_BIN), ("VST3", VST3_BIN), *delivery):
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


# ---------------------------------------------------------------- M1d1: the bundle hygiene


def crate_version() -> str:
    """The version of the plugin crate: Cargo.toml, the one source of every version in the bundles."""
    text = (HERE / "Cargo.toml").read_text(encoding="utf-8")
    package = text.split("[package]", 1)[1].split("\n[", 1)[0]
    m = re.search(r'^version\s*=\s*"([^"]+)"', package, re.M)
    return m.group(1) if m else ""


def plist_version_problems(label: str, plist: Path, want: str) -> list[str]:
    """CFBundleVersion and CFBundleShortVersionString of one Info.plist: both must be the version."""
    import plistlib

    if not plist.exists():
        return [f"{label}: no Info.plist"]
    data = plistlib.loads(plist.read_bytes())
    return [f"{label}: {key} is {data.get(key)!r}, not {want!r}" for key in ("CFBundleVersion", "CFBundleShortVersionString") if data.get(key) != want]


def bundle_version_targets() -> list[tuple[str, Path]]:
    """Every bundle that carries the version: the CLAP, the VST3 with the CLAP inside, and both of the delivery build."""
    return [
        ("CLAP", CLAP), ("VST3", VST3), ("CLAP in the VST3", EMBEDDED_CLAP),
        ("delivery CLAP", DELIVERY_CLAP), ("delivery VST3", DELIVERY_VST3), ("CLAP in the delivery VST3", DELIVERY_EMBEDDED),
    ]


def stage_bundle_version() -> bool:
    """M1d1 gate line 2: both version fields of every bundle, and the ARA factory and the CLAP descriptor, name the version of the crate."""
    want = crate_version()
    if not want:
        return pass_fail("bundle-version", False, "no version in Cargo.toml")
    problems: list[str] = []
    targets = bundle_version_targets()
    for label, bundle in targets:
        problems += plist_version_problems(label, bundle / "Contents" / "Info.plist", want)
    described = []
    for label, bundle in (("CLAP", CLAP), ("CLAP in the VST3", EMBEDDED_CLAP), ("delivery CLAP", DELIVERY_EMBEDDED)):
        _rc, _text, lines = host_run(f"bundle-version-{len(described)}", bundle, ["version"], "cache-bundle-version")
        row = next((r for r in lines if r.get("event") == "version"), None)
        if row is None:
            problems.append(f"{label}: the host read no version")
            continue
        described.append(label)
        for key in ("ara_factory", "clap_descriptor"):
            if row.get(key) != want:
                problems.append(f"{label}: {key} is {row.get(key)!r}, not {want!r}")
    return pass_fail(
        "bundle-version",
        not problems,
        f"CFBundleVersion and CFBundleShortVersionString of {len(targets)} bundles are {want} (Cargo.toml); the ARA factory and the CLAP descriptor of {len(described)} CLAPs say {want}"
        if not problems else "; ".join(problems),
    )


def stage_bundle_version_fault() -> bool:
    """The stage can fail: the plist of the wrapper as it was (the unfilled text and an empty field), and a version that is another."""
    want = crate_version() or "0.1.0"
    tmp = OUT / "bundle-version-fault"
    if tmp.exists():
        shutil.rmtree(tmp)
    (tmp / "Contents").mkdir(parents=True)
    import plistlib

    old = {"CFBundleVersion": "MACOSX_BUNDLE_SHORT_VERSION_STRING", "CFBundleShortVersionString": ""}
    (tmp / "Contents" / "Info.plist").write_bytes(plistlib.dumps(old))
    unfilled = plist_version_problems("old wrapper plist", tmp / "Contents" / "Info.plist", want)
    (tmp / "Contents" / "Info.plist").write_bytes(plistlib.dumps({"CFBundleVersion": want, "CFBundleShortVersionString": "0.0.1"}))
    other = plist_version_problems("other short version", tmp / "Contents" / "Info.plist", want)
    named = any("MACOSX_BUNDLE_SHORT_VERSION_STRING" in p for p in unfilled) and any("CFBundleShortVersionString is ''" in p for p in unfilled)
    return pass_fail(
        "bundle-version-fault",
        named and len(unfilled) == 2 and len(other) == 1,
        f"the old plist of the wrapper (unfilled text, empty field) gives {len(unfilled)} problems, a short version of another number gives {len(other)}",
    )


def codesign_verify(path: Path) -> tuple[int, str]:
    return run(["codesign", "--verify", "--deep", "--strict", "--verbose=2", str(path)])


def stage_bundle_sign() -> bool:
    """M1d1 gate line 3: every bundle is signed as a bundle (ad hoc, inner first): `codesign --verify --deep --strict` passes,
    the signature is no linker signature, and it seals the resources (the page among them)."""
    ok = True
    for label, bundle in (("CLAP", CLAP), ("VST3", VST3), ("delivery CLAP", DELIVERY_CLAP), ("delivery VST3", DELIVERY_VST3)):
        if not bundle.exists():
            return pass_fail("bundle-sign", False, f"no {label} at {bundle}")
        rc, text = codesign_verify(bundle)
        # a VST3 seals the CLAP inside it as one nested component, and the CLAP's own seal covers the page
        inner = bundle / "Contents" / "PlugIns" / "DrumSpikeAra.clap" if bundle.suffix == ".vst3" else bundle
        _rc2, info = run(["codesign", "-dvv", str(bundle)])
        _rc3, inner_info = run(["codesign", "-dvv", str(inner)])
        sealed = re.search(r"Sealed Resources version=\d+ rules=\d+ files=(\d+)", inner_info)
        outer = re.search(r"Sealed Resources version=\d+ rules=\d+ files=(\d+)", info)
        page = inner / "Contents" / "Resources" / "page"
        n_page = sum(1 for f in page.rglob("*") if f.is_file()) if page.is_dir() else 0
        good = (
            rc == 0 and bool(sealed) and bool(outer) and int(sealed.group(1)) >= n_page > 0
            and "linker-signed" not in info + inner_info and "Signature=adhoc" in info and "Signature=adhoc" in inner_info
        )
        ok = pass_fail(
            "bundle-sign",
            good,
            f"{label}: codesign --verify --deep --strict rc={rc}; ad hoc, no linker signature, the CLAP inside signed first; the seal of the CLAP covers "
            f"{sealed.group(1) if sealed else '0'} resource files (the page has {n_page})" if good else
            f"{label}: rc={rc}, sealed files {sealed.group(1) if sealed else None}, page files {n_page}: {text.strip()[:200]} {info.strip()[-300:]}",
        ) and ok
    return ok


def stage_bundle_sign_fault() -> bool:
    """The stage can fail: a copy of the VST3 verifies (the control); with one changed page file, one new file in the page and the
    inner CLAP replaced by another binary, the same verify fails."""
    if not VST3.exists():
        return pass_fail("bundle-sign-fault", False, "no VST3")
    tmp = OUT / "bundle-sign-fault"
    if tmp.exists():
        shutil.rmtree(tmp)
    tmp.mkdir(parents=True)
    copy = tmp / "DrumSpikeAra.vst3"
    rc_subprocess = subprocess.run(["ditto", str(VST3), str(copy)])
    rc0, _t0 = codesign_verify(copy)
    page = copy / "Contents" / "PlugIns" / "DrumSpikeAra.clap" / "Contents" / "Resources" / "page"
    target = page / "app.js"
    target.write_text(target.read_text(encoding="utf-8") + "\n// changed\n", encoding="utf-8")
    rc1, text1 = codesign_verify(copy)
    return pass_fail(
        "bundle-sign-fault",
        rc_subprocess.returncode == 0 and rc0 == 0 and rc1 != 0 and "app.js" in text1,
        f"a copy of the signed VST3 verifies (rc={rc0}); with one page file changed the verify fails (rc={rc1}) and names it: {next((ln.strip() for ln in text1.splitlines() if 'app.js' in ln), '')[-120:]}",
    )


DELIVERY_ENV = {"DS_ARA_ROUND_FAULT": "request-only"}


def delivery_run(label: str, bundle: Path) -> tuple[str, str]:
    """The fault switch `DS_ARA_ROUND_FAULT=request-only` makes a build that reads it start no round for a host that never asks.
    The state: "round" (the normal round: the switch had no effect), "no-round" (the host ran, the plugin reported, no round started
    and no job), "other" (a run that is neither), "not-run" (no host, no bundle, no reference, or a run that left no report: a crash
    or a missing part must never pass as "no round")."""
    ref_path = OUT / "events-reference.json"
    if not HOST.exists() or not bundle.exists() or not ref_path.exists():
        return "not-run", "no host, no bundle or no reference events: run check.sh build and the reference stage first"
    cache = f"cache-{label}"
    rc, text, lines = host_run(
        label, bundle, ["grow", "--files", *fixture_args(FILES), "--batch", str(len(FILES)), "--wait", SHORT_WAIT, "--no-request"], cache, extra=DELIVERY_ENV,
    )
    parts, evidence = nr_parts(label, rc, lines, FILES, ref_path.read_bytes())
    wrote = (OUT / cache).is_dir() and any((OUT / cache).rglob("*.wav"))
    rows = report_rows(label)
    # a host that ran and waited for a round that never came says so (rc 2, "did not finish") after the plugin read the audio
    waited = rc == 2 and "did not finish" in text and wrote
    if not (rows and rc == 0) and not waited:
        return "not-run", f"the host run crashed or left no usable result (rc={rc}, {len(rows)} report rows, the plugin wrote its cache {wrote}): {text.strip()[:120]}"
    if all(parts.values()) and wrote:
        return "round", f"{evidence}; the cache folder has the audio files {wrote}"
    if waited and not event_rows(label, "round_start") and not job_rows(label):
        return "no-round", f"the plugin read the audio and wrote its cache but started no round: {evidence}"
    return "other", f"{evidence}; the cache folder has the audio files {wrote}"


def stage_delivery_switches() -> bool:
    """M1d1 gate line 4: the rule is the cargo feature `test-switches` (the gate build has it, the delivery build has not). The delivery
    CLAP, inside the delivery VST3, ignores DS_ARA_ROUND_FAULT=request-only (it still gives the round); the report (DS_ARA_OUT) and the
    cache folder (DS_ARA_CACHE) work in it. The control: the gate CLAP with the same variable ran and gave no round."""
    state, ev = delivery_run("delivery-switches", DELIVERY_EMBEDDED)
    ok = pass_fail("delivery-switches", state == "round", f"the delivery build with DS_ARA_ROUND_FAULT=request-only still gives the round (state {state}): {ev}")
    state, cev = delivery_run("delivery-switches-control", EMBEDDED_CLAP)
    return pass_fail("delivery-switches-control", state == "no-round", f"the gate build with the same variable obeys it, it ran and gave no round (state {state}): {cev}") and ok


def stage_delivery_switches_fault() -> bool:
    """The check can fail: a build that reads the switches (the gate CLAP) ran and gave no round, so the same check refuses it as a delivery build."""
    state, ev = delivery_run("delivery-switches-fault", CLAP)
    return pass_fail("delivery-switches-fault", state == "no-round", f"the check refuses a build that reads the switch (the gate CLAP): it ran and gave no round (state {state}): {ev}")


# ---------------------------------------------------------------- the cargo trios


def stage_crate() -> bool:
    env = {"ARA_API_DIR": str(DEPS / "ARA_SDK" / "ARA_API"), "LIBCLANG_PATH": f"{CLT}/usr/lib"}
    # the unit tests name fault switches (transport), so they run with the feature; clippy runs on
    # both builds (a delivery build must be as clean as the gate build)
    steps = [
        ("fmt", ["cargo", "fmt", "--all", "--", "--check"]),
        ("clippy", ["cargo", "clippy", "--all-targets", "--locked", "--features", "test-switches", "--", "-D", "warnings"]),
        ("clippy-delivery", ["cargo", "clippy", "--all-targets", "--locked", "--", "-D", "warnings"]),
        ("test", ["cargo", "test", "--locked", "--features", "test-switches"]),
        ("test-delivery", ["cargo", "test", "--locked"]),
    ]
    ok = True
    for name, cmd in steps:
        rc, text = run(cmd, env=env, log=f"crate-{name}.log", cwd=HERE)
        passed = sum(int(m) for m in re.findall(r"test result: ok\. (\d+) passed", text)) if name.startswith("test") else None
        count = f", {passed} unit tests passed" if passed is not None else ""
        ok = pass_fail(f"crate-{name}", rc == 0 and (passed is None or passed > 0), f"{name} of the plugin crate, rc={rc}{count} (out/crate-{name}.log)") and ok
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


# ---------------------------------------------------------------- M1b2: the editor window
#
# The ara-host mode `editor` analyses the fixture session in an ARA document, binds a plugin
# instance to it, opens the plugin's editor (clap.gui, cocoa) in a real AppKit window and runs
# one phase. The page runs the real Drum Spike page on the real service of the document; a
# test driver (src/editor/driver.js, only under DS_ARA_EDITOR_MODE) reports to the plugin's
# report file. Every check is a function of the run that returns parts (name -> (ok,
# evidence)); each fault run makes one named part fail.

EDITOR_CYCLES = 50
MEMORY_LIMIT_MB = 50.0
FRAME_LIMIT_MS = 33.0


def editor_run(
    label: str, mode: str, bundle: Path | None = None, cycles: int = 1, env: dict | None = None,
    args: list | None = None, timeout: int = 1500, files: list | None = None,
) -> tuple[int, dict, list]:
    """One `ara-host editor` run with a fresh cache and report file. `files`: the sources of the
    document before the editor opens (default: the four fixtures; `[]`: none)."""
    out_file = OUT / f"report-{label}.jsonl"
    if out_file.exists():
        out_file.unlink()
    cache_dir = OUT / f"cache-{label}"
    if cache_dir.exists():
        shutil.rmtree(cache_dir)
    e = {**no_switch(), "DS_ARA_CACHE": str(cache_dir), "DS_ARA_OUT": str(out_file)}
    e.update(env or {})
    srcs = [str(FIXTURES / f) for f in FILES] if files is None else files
    cmd = [str(HOST), str(bundle or CLAP), "editor", *(["--files", *srcs] if srcs else []),
           "--editor-mode", mode, "--cycles", str(cycles), *(args or [])]
    rc, text = run(cmd, env=e, log=f"host-{label}.log", timeout=timeout)
    summary: dict = {}
    for ln in text.splitlines():
        if ln.startswith("{") and '"editor_run"' in ln:
            try:
                summary = json.loads(ln)
            except ValueError:
                pass
    return rc, summary, report_rows(label)


def page_reports(rows: list, kind: str) -> list:
    return [r["report"] for r in rows if r.get("event") == "page_report" and r.get("kind") == kind]


def events_object_of(raw: dict, names: list) -> bytes:
    """`{"<track>": <events array>, ...}` from the raw answers of `events_get`, byte for byte
    as the service wrote the arrays (the reference line has the same compact form)."""
    dec = json.JSONDecoder()
    parts = []
    for n in names:
        text = raw[n]
        i = text.index('"events":') + len('"events":')
        _, end = dec.raw_decode(text, i)
        parts.append(json.dumps(n) + ":" + text[i:end])
    return ("{" + ",".join(parts) + "}").encode()


def open_parts(rc: int, summary: dict, rows: list, cycles: int, ref: bytes, bundle_tail: str) -> dict:
    """Gate lines 1, 2, 4, 7 (main thread) and 10 of one run of the `ready` mode."""
    parts: dict = {}
    ready = [r for r in rows if r.get("event") == "page_ready"]
    parts["editor-ready"] = (
        rc == 0 and summary.get("opened") == cycles and len(ready) == cycles,
        f"the page was ready {len(ready)} of {cycles} times (rc={rc}, opened {summary.get('opened')}, {summary.get('why')})",
    )
    boots = page_reports(rows, "boot")
    bad = []
    n_tracks = n_events = 0
    for b in boots:
        names = b.get("tracks") or []
        n_tracks, n_events = len(names), b.get("n_events", 0)
        try:
            same = events_object_of(b["raw_events"], names) == ref
        except (KeyError, ValueError):
            same = False
        if not (b.get("loaded") and n_tracks == 4 and n_events > 0 and same and b.get("hostMode") and b.get("tauri")):
            bad.append((len(names), n_events, same))
    parts["editor-data"] = (
        len(boots) == cycles and not bad and n_tracks > 0 and n_events > 0,
        f"{len(boots)} boot reports: {n_tracks} tracks and {n_events} events in the page, the events the page reads "
        f"are byte for byte the reference ({len(ref)} bytes); {len(bad)} report(s) differ",
    )
    foreign = [x for b in boots for x in b.get("resources", []) if not x.startswith("dspage://")]
    viol = [v for b in boots for v in b.get("violations", [])]
    parts["editor-net"] = (
        bool(boots) and not foreign and not viol,
        f"{len(foreign)} request(s) to another scheme or host, {len(viol)} Content-Security-Policy violation(s) in {len(boots)} page loads",
    )
    opens = [r for r in rows if r.get("event") == "editor_open"]
    ok_dirs = [r for r in opens if str(r.get("page_dir", "")).endswith(bundle_tail) and r.get("page_found_by")]
    parts["editor-bundle"] = (
        bool(opens) and len(ok_dirs) == len(opens),
        f"the page of {len(ok_dirs)} of {len(opens)} editors came from <bundle>/{bundle_tail} (found by "
        f"{sorted({r.get('page_found_by') for r in opens})})",
    )
    closes = [r for r in rows if r.get("event") == "editor_close"]
    last = closes[-1] if closes else {}
    parts["editor-main-thread"] = (
        bool(closes) and last.get("off_main") == 0 and last.get("main_checks", 0) > 0
        and all(r.get("main_thread") for r in opens + closes),
        f"{last.get('main_checks')} web view calls checked, {last.get('off_main')} off the main thread; "
        f"every editor was made and dropped on the main thread",
    )
    after, first = summary.get("rss_after_mb", 0.0), summary.get("rss_open_mb", 0.0)
    parts["editor-memory"] = (
        rc == 0 and first > 0 and after - first <= MEMORY_LIMIT_MB,
        f"resident memory of the test host: closed {summary.get('rss_closed_mb')} MB, open {first} MB, "
        f"after {cycles} cycles {after} MB (limit: {MEMORY_LIMIT_MB:.0f} MB above the first open)",
    )
    return parts


def print_parts(prefix: str, parts: dict, only: list | None = None) -> bool:
    ok = True
    for name, (good, ev) in parts.items():
        if only and name not in only:
            continue
        ok = pass_fail(f"{name}{prefix}", good, ev) and ok
    return ok


def _reference() -> bytes | None:
    p = OUT / "events-reference.json"
    return p.read_bytes() if p.exists() else None


_EDITOR_OPEN: dict = {}


def editor_open_run(which: str) -> tuple[dict, str]:
    bundle, tail = (CLAP, "DrumSpikeAra.clap/Contents/Resources/page") if which == "clap" else (
        EMBEDDED_CLAP, "DrumSpikeAra.vst3/Contents/PlugIns/DrumSpikeAra.clap/Contents/Resources/page")
    if which not in _EDITOR_OPEN:
        ref = _reference()
        if ref is None or not HOST.exists() or not bundle.exists():
            _EDITOR_OPEN[which] = ({"editor-ready": (False, "no reference, host or bundle: run the earlier stages and check.sh build")}, tail)
        else:
            rc, summary, rows = editor_run(f"editor-open-{which}", "ready", bundle, EDITOR_CYCLES)
            _EDITOR_OPEN[which] = (open_parts(rc, summary, rows, EDITOR_CYCLES, ref, tail), tail)
    return _EDITOR_OPEN[which]


def stage_editor_open() -> bool:
    ok = True
    for which, suffix in (("clap", ""), ("vst3", "-vst3")):
        parts, _ = editor_open_run(which)
        ok = print_parts(suffix, parts) and ok
    return ok


def fault_part(stage: str, parts: dict, name: str, what: str) -> bool:
    good, ev = parts.get(name, (True, "part missing"))
    return pass_fail(stage, not good, f"the fault run ({what}) makes {name} fail: {ev}")


def stage_editor_fault() -> bool:
    ref = _reference()
    if ref is None:
        return pass_fail("editor-fault", False, "no reference events")
    ok = True
    tail = "DrumSpikeAra.clap/Contents/Resources/page"
    for label, mode, cycles, env, args, part, what in (
        ("ready", "ready", 1, {"DS_ARA_EDITOR_FAULT": "no-ready"}, [], "editor-ready", "a page that never reports ready"),
        ("bundle", "ready", 1, {"DS_ARA_EDITOR_FAULT": "no-bundle"}, [], "editor-bundle", "no page in the bundle"),
        ("net", "net", 1, {}, [], "editor-net", "the page asks for a foreign host"),
        ("data", "ready", 1, {}, ["--no-ara"], "editor-data", "an editor with no ARA document: no tracks"),
        ("memory", "ready", 20, {"DS_ARA_EDITOR_FAULT": "leak"}, [], "editor-memory", "every editor leaks 4 MB"),
        ("main", "ready", 1, {"DS_ARA_EDITOR_FAULT": "inline"}, [], "editor-main-thread", "a command runs on the main thread"),
    ):
        rc, summary, rows = editor_run(f"editor-fault-{label}", mode, CLAP, cycles, env, args, timeout=600)
        if label == "net":
            # the net mode waits for the page's `now` report; its violations are the evidence
            rep = page_reports(rows, "now")
            viol = [v for r in rep for v in r.get("violations", [])]
            foreign = [x for r in rep for x in r.get("resources", []) if not x.startswith("dspage://")]
            parts = {"editor-net": (not viol and not foreign, f"{len(viol)} violation(s), {len(foreign)} foreign request(s)")}
        else:
            parts = open_parts(rc, summary, rows, cycles, ref, tail)
        ok = fault_part(f"editor-fault-{label}", parts, part, what) and ok
    return ok


def run_phase(
    label: str, mode: str, bundle: Path, env: dict | None = None, args: list | None = None, files: list | None = None,
) -> tuple[int, dict, list]:
    return editor_run(label, mode, bundle, 1, env, args, timeout=300, files=files)


def keys_parts(rc: int, rows: list) -> dict:
    reps = page_reports(rows, "keys")
    seen = {e["key"]: e for e in (reps[0].get("seen", []) if reps else [])}
    want = ["Space", "Right", "Z", "Tab", "Esc"]
    missing = [k for k in want if k not in seen]
    acted = [k for k in want if k in seen and seen[k].get("page")]
    return {"editor-keys": (
        rc == 0 and bool(reps) and not missing and len(acted) >= 3 and seen.get("Z", {}).get("meta") is True,
        f"Space, Right, Cmd+Z, Tab, Esc reached the page: missing {missing or 'none'}; the page ran a command for {acted}")}


def focus_parts(rc: int, rows: list) -> dict:
    reps = page_reports(rows, "focus")
    keys = reps[0].get("keys", []) if reps else []
    lost = [k for k in keys if 2800 <= k["t"] <= 4300]
    back = [k for k in keys if k["t"] >= 4700]
    return {"editor-focus": (
        rc == 0 and bool(reps) and not lost and bool(back),
        f"after a click in the host {len(lost)} key(s) reached the page (want 0); after a click in the editor {len(back)} did (want 1)")}


def resize_parts(rc: int, rows: list) -> dict:
    reps = page_reports(rows, "resize")
    s = reps[0].get("samples", []) if reps else []
    if len(s) < 3:
        return {"editor-resize": (False, "too few samples")}
    w0 = s[0]["innerW"]
    big = max(x["innerW"] for x in s if x["t"] > 2500)
    small = min(x["innerW"] for x in s if x["t"] > 4500)
    return {"editor-resize": (rc == 0 and big > w0 + 100 and small < w0 - 100, f"the page follows the window: {w0} -> {big} -> {small} px wide")}


def measure_parts(rc: int, rows: list, summary: dict | None = None) -> dict:
    reps = page_reports(rows, "measure")
    r = reps[0] if reps else {}
    if summary is not None and summary.get("window_visible") is False:
        return {"editor-measure": (False, "the window of the test host is occluded (the screen of this host is locked), so WebKit "
                                          "suspends the page and it draws no frame: the frame time cannot be measured here")}
    frames, p95 = r.get("frames", 0), r.get("p95_ms")
    if r and r.get("visibility") != "visible":
        return {"editor-measure": (False, f"the page is {r.get('visibility')}: the screen of this host is locked or its window occluded, "
                                          f"so the page draws no frame and the frame time cannot be measured ({frames} frames)")}
    return {"editor-measure": (
        rc == 0 and frames >= 100 and p95 is not None and p95 <= FRAME_LIMIT_MS,
        f"{frames} frames over 8 s of scroll and zoom, p95 {p95} ms (max {r.get('max_ms')} ms, limit {FRAME_LIMIT_MS:.0f} ms)")}


def stage_editor_gui() -> bool:
    """Gate line 8: keys, focus, resize and the frame time on the real page, on the CLAP and
    on the plugin in the VST3; the first responder is the plugin's own call."""
    ok = True
    for which, bundle, suffix in (("clap", CLAP, ""), ("vst3", EMBEDDED_CLAP, "-vst3")):
        for mode, fn in (("keys", keys_parts), ("focus", focus_parts), ("resize", resize_parts), ("measure", measure_parts)):
            rc, summary, rows = run_phase(f"editor-{mode}-{which}", mode, bundle)
            parts = fn(rc, rows, summary) if mode == "measure" else fn(rc, rows)
            ok = print_parts(suffix, parts) and ok
    return ok


def stage_editor_gui_fault() -> bool:
    ok = True
    for mode, fn, fault, part, what in (
        ("keys", keys_parts, "no-focus", "editor-keys", "the web view is not the first responder"),
        ("focus", focus_parts, "no-focus", "editor-focus", "the web view is not the first responder"),
        ("resize", resize_parts, "no-resize", "editor-resize", "the web view does not follow the host view"),
        ("measure", measure_parts, "slow-frames", "editor-measure", "the page draws a slow frame every time"),
    ):
        rc, summary, rows = run_phase(f"editor-fault-{mode}", mode, CLAP, {"DS_ARA_EDITOR_FAULT": fault})
        parts = fn(rc, rows, None) if mode == "measure" else fn(rc, rows)
        ok = fault_part(f"editor-{mode}-fault", parts, part, what) and ok
    return ok


def shell_parts(rc: int, rows: list) -> dict:
    reps = page_reports(rows, "shell")
    r = reps[0] if reps else {}
    cmds = r.get("commands", {})
    unknown = [c for c, v in cmds.items() if not v.get("ok") and "unknown command" in str(v.get("message"))]
    answered = [c for c, v in cmds.items() if v.get("ok") or isinstance(v.get("code"), int)]
    bin_ = r.get("binary", {})
    err = r.get("error_shape") or {}
    parts = {
        "editor-shell": (
            rc == 0 and bool(reps) and len(cmds) >= 16 and not unknown and len(answered) == len(cmds),
            f"{len(answered)} of {len(cmds)} shell commands have a defined answer, {len(unknown)} 'unknown command' {unknown}; "
            f"quit_app answered and the host is alive (rc={rc})"),
        "editor-binary": (
            sorted(bin_) == ["peaks_tile", "samples_window", "track_layer_wave", "track_sampler_wave"]
            and all(v.get("ok") and v.get("len", 0) > 8 for v in bin_.values()),
            f"the four binary commands gave an ArrayBuffer: { {k: v.get('len') for k, v in bin_.items()} }"),
        "editor-error": (
            isinstance(err.get("code"), int) and isinstance(err.get("message"), str) and err.get("code") == 404,
            f"an error reaches the page as {{code, message}}: {err}"),
        "editor-tauri": (
            r.get("keys") == ["core", "dialog", "event"] and r.get("core") == ["Channel", "invoke"] and r.get("listen") is True
            and r.get("hostMode") is True,
            f"window.__TAURI__ has {r.get('keys')} with core {r.get('core')}, event.listen works, __DS_HOST_MODE__={r.get('hostMode')}"),
    }
    return parts


def page_panic_parts(rc: int, rows: list) -> dict:
    reps = page_reports(rows, "shell")
    r = reps[0] if reps else {}
    got = r.get("export_existing")
    caught = [x for x in rows if x.get("event") == "page_panic_caught"]
    return {"editor-panic": (
        rc == 0 and isinstance(got, dict) and got.get("code") == 500 and "panicked" in got.get("message", "") and bool(caught),
        f"a command that panics gave the page {got}; {len(caught)} panic caught, the host stays alive (rc={rc})")}


def stage_editor_shell() -> bool:
    ok = True
    wav = str(FIXTURES / FILES[1])
    for which, bundle, suffix in (("clap", CLAP, ""), ("vst3", EMBEDDED_CLAP, "-vst3")):
        rc, summary, rows = run_phase(f"editor-shell-{which}", "shell", bundle, args=["--no-regions", "--arg", wav])
        ok = print_parts(suffix, shell_parts(rc, rows)) and ok
    rc, summary, rows = run_phase("editor-panic", "shell", CLAP, {"DS_ARA_EDITOR_SCENARIO": "panic:export_existing"}, ["--no-regions", "--arg", wav])
    ok = print_parts("", page_panic_parts(rc, rows)) and ok
    return ok


def stage_editor_shell_fault() -> bool:
    wav = str(FIXTURES / FILES[1])
    ok = True
    rc, summary, rows = run_phase("editor-fault-shell", "shell", CLAP, {"DS_ARA_EDITOR_FAULT": "no-shell"}, ["--no-regions", "--arg", wav])
    ok = fault_part("editor-shell-fault", shell_parts(rc, rows), "editor-shell", "the shell commands are left to the service") and ok
    rc, summary, rows = run_phase(
        "editor-fault-panic", "shell", CLAP,
        {"DS_ARA_EDITOR_SCENARIO": "panic:export_existing", "DS_ARA_GUARD_FAULT": "off"}, ["--no-regions", "--arg", wav])
    ok = fault_part("editor-panic-fault", page_panic_parts(rc, rows), "editor-panic", "the request thread has no panic guard") and ok
    return ok


def edit_parts(rc: int, rows: list, restore_rc: int, restore_rows: list, ref: bytes, notes_lines: list) -> dict:
    """Gate line 5 (the edit lasts, also when the archive is stored at once) and its notes; the
    event of the service that reaches the page (gate line 3)."""
    reps = page_reports(rows, "edit")
    r = reps[0] if reps else {}
    if not r or r.get("error"):
        why = f"the page's edit did not run: {r.get('error')} (rc={rc})"
        return {"editor-edit": (False, why), "editor-edit-notes": (False, why), "editor-events": (False, why)}
    name, idx = r["name"], r["index"]
    before = json.loads(r["before_raw"])["events"]
    after = json.loads(r["after_raw"])["events"]
    moved = [i for i in range(len(before)) if before[i] != after[i]]
    restored = [x for x in restore_rows if x.get("event") == "doc_restored"]
    got = json.loads(restored[-1]["events_json"]) if restored else {}
    want = json.loads(ref)
    want[name] = after
    other_same = all(got.get(k) == v for k, v in want.items() if k != name)
    parts = {"editor-edit": (
        rc == 0 and restore_rc == 0 and moved == [idx] and got.get(name) == after and other_same,
        f"the page moved hit {idx} of {name} from {r['from']} to {r['to']} and the archive was stored at once; restored in a new "
        f"process it has that list ({len(after)} hits, {len(moved)} differs from the analysis) and the other {len(want) - 1} tracks are unchanged")}
    # the notes of the source: the rate comes from the host's own rows, the region is the whole source
    row = next((n for n in notes_lines if safe_name(stem_of(n.get("name", ""))) == name), None)
    if row is None:
        parts["editor-edit-notes"] = (False, f"no host_notes row for {name} after the restore: the evidence is missing")
    else:
        rate = row["rate"]
        ms, os_ = round(r["to"] * rate), round(r["from"] * rate)
        parts["editor-edit-notes"] = (
            ms in row["samples"] and os_ not in row["samples"],
            f"the ARA notes of {name} after the restore carry the moved hit at sample {ms} (rate {rate}) and no longer {os_}: {row['samples']}")
    ev = page_reports(rows, "edit-events")
    dirty = (ev[0].get("dirty_events") if ev else None) or []
    parts["editor-events"] = (
        any(d.get("dirty") is True for d in dirty),
        f"the event of the service (project-dirty) reached the page after its edit: {len(dirty)} event(s) {dirty[:2]}")
    return parts


def edit_run(label: str, bundle: Path, env: dict | None = None) -> dict:
    """The page edits a hit; the archive is stored at once (the moment the edit is answered), while
    the worker hears about the change 12 s late (the scenario `sync-delay`: only the worker's
    sync is delayed, the request thread's own record is not)."""
    ref = _reference()
    archive = OUT / f"{label}.ara"
    if archive.exists():
        archive.unlink()
    env = {"DS_ARA_EDITOR_SCENARIO": "sync-delay", **(env or {})}
    rc, summary, rows = run_phase(label, "edit", bundle, env, ["--store-at-edit", str(archive)])
    rrc, rtext, rlines = host_run(f"{label}-restore", bundle, ["restore", "--archive", str(archive), "--files", *[str(FIXTURES / f) for f in FILES]], f"cache-{label}")
    notes = [ln for ln in rlines if ln.get("event") == "host_notes"]
    return edit_parts(rc, rows, rrc, report_rows(f"{label}-restore"), ref, notes)


def stage_editor_edit() -> bool:
    if _reference() is None or not HOST.exists():
        return pass_fail("editor-edit", False, "no reference or host")
    ok = True
    for which, bundle, suffix in (("clap", CLAP, ""), ("vst3", EMBEDDED_CLAP, "-vst3")):
        ok = print_parts(suffix, edit_run(f"editor-edit-{which}", bundle)) and ok
    return ok


def stage_editor_edit_fault() -> bool:
    if _reference() is None:
        return pass_fail("editor-edit-fault", False, "no reference")
    ok = True
    for fault, part, what in (
        ("no-sync", "editor-edit", "a command of the page does not reach the archive"),
        ("no-record", "editor-edit", "the request thread does not record the edit (the worker hears 12 s late): an archive stored at once misses it"),
        ("no-notes", "editor-edit-notes", "the notes do not follow the edit"),
        ("no-events", "editor-events", "the events of the service do not reach the page"),
    ):
        parts = edit_run(f"editor-fault-edit-{fault}", CLAP, {"DS_ARA_EDITOR_FAULT": fault})
        ok = fault_part(f"editor-edit-fault-{fault}", parts, part, what) and ok
    return ok


def threads_parts(rc: int, summary: dict, rows: list) -> dict:
    reps = page_reports(rows, "analyse")
    r = reps[0] if reps else {}
    closes = [x for x in rows if x.get("event") == "editor_close"]
    last = closes[-1] if closes else {}
    return {"editor-threads": (
        rc == 0 and r.get("state") == "done" and r.get("channel_messages", 0) > 0 and summary.get("audio_overruns") == 0
        and summary.get("audio_blocks", 0) >= MIN_BLOCKS and last.get("off_main") == 0 and last.get("peak_inflight", 0) >= 1,
        f"an analysis started from the page ended '{r.get('state')}' in {r.get('ms')} ms with {r.get('channel_messages')} channel messages; "
        f"{summary.get('audio_blocks')} audio blocks ran meanwhile, {summary.get('audio_overruns')} overruns "
        f"(worst {summary.get('audio_worst_us')} us of {summary.get('audio_budget_us')} us); "
        f"{last.get('off_main')} web view or command call(s) off the right thread"),
        "editor-threads-analysis": (
            bool(last.get("progress_threads")) and bool(last.get("start_threads")) and bool(last.get("main_thread_id"))
            and last.get("main_thread_id") not in (last.get("progress_threads") or []) + (last.get("start_threads") or [])
            and not set(last.get("progress_threads") or []) & set(last.get("start_threads") or []),
            f"the analysis the page started reported its progress from thread(s) {last.get('progress_threads')}; its start came from "
            f"request thread(s) {last.get('start_threads')}; the main thread is {last.get('main_thread_id')}: the analysis ran on none of them")}


def stage_editor_threads() -> bool:
    ok = True
    for which, bundle, suffix in (("clap", CLAP, ""), ("vst3", EMBEDDED_CLAP, "-vst3")):
        rc, summary, rows = run_phase(f"editor-threads-{which}", "analyse", bundle)
        ok = print_parts(suffix, threads_parts(rc, summary, rows)) and ok
    return ok


def stage_editor_threads_fault() -> bool:
    rc, summary, rows = run_phase("editor-fault-threads", "analyse", CLAP, {"DS_ARA_EDITOR_FAULT": "inline"})
    parts = threads_parts(rc, summary, rows)
    ok = fault_part("editor-threads-fault", parts, "editor-threads", "a command runs on the main thread")
    return fault_part("editor-threads-analysis-fault", parts, "editor-threads-analysis", "the start of the analysis runs on the main thread") and ok


EXTRA_SYNTH = "synth:2"


def job_rows_of(rows: list) -> list:
    return [r for r in rows if r.get("event") == "job"]


def keep_edits_parts(rc: int, rows: list) -> dict:
    edits = page_reports(rows, "edit")
    fol = page_reports(rows, "follow")
    jobs = job_rows_of(rows)
    failed = [r for r in rows if r.get("event") == "job_failed"]
    if not edits or not fol:
        why = f"no edit or follow report (rc={rc})"
        return {"editor-keep-edits": (False, why), "editor-follow": (False, why)}
    e, f = edits[0], fol[0]
    name = e["name"]
    kept_service = any(abs(h["t"] - e["to"]) < 1e-9 for h in json.loads(f["raw_events"].get(name, '{"events":[]}'))["events"])
    kept_page = any(abs(h["t"] - e["to"]) < 1e-9 for h in f["page_events"].get(name, []))
    gone_old = not any(abs(h["t"] - e["from"]) < 1e-9 for h in f["page_events"].get(name, []))
    new_track = len(f["tracks"]) == len(FILES) + 1
    same_as_service = True
    for n in f["tracks"]:
        svc = json.loads(f["raw_events"][n])["events"]
        same_as_service = same_as_service and [(h["t"], h["v"]) for h in svc] == [(h["t"], h["v"]) for h in f["page_events"].get(n, [])]
    return {
        "editor-keep-edits": (
            rc == 0 and len(jobs) >= 2 and not failed and kept_service and kept_page and gone_old and new_track,
            f"a hit moved in the page ({e['from']} to {e['to']} on {name}), then the DAW added a source: {len(jobs)} rounds, "
            f"the moved hit is still there (service {kept_service}, page {kept_page}), the old place is empty ({gone_old}), "
            f"{len(f['tracks'])} tracks, {len(failed)} failed round(s)"),
        "editor-follow": (
            rc == 0 and f.get("ok") is True and new_track and same_as_service,
            f"the page holds the {len(f['tracks'])} tracks and the events of the service after the round, without a reload by hand "
            f"(ok {f.get('ok')}, same as the service {same_as_service}, {f.get('ms')} ms)"),
    }


def open_first_parts(rc: int, rows: list, ref: bytes) -> dict:
    boots = page_reports(rows, "boot")
    fol = page_reports(rows, "follow")
    if not boots or not fol:
        why = f"no boot or follow report (rc={rc})"
        return {"editor-open-first": (False, why)}
    b, f = boots[0], fol[0]
    try:
        same = events_object_of(f["raw_events"], f["tracks"]) == ref
    except (KeyError, ValueError):
        same = False
    return {"editor-open-first": (
        rc == 0 and b.get("tracks") == [] and b.get("tauri") is True and f.get("ok") is True and len(f["tracks"]) == 4 and same,
        f"the editor opened before any analysis: {len(b.get('tracks', []))} tracks, page text {b.get('text', '')[:70]!r}; when the first round "
        f"ended the page held {len(f['tracks'])} tracks without a reload by hand, the events byte for byte the reference ({same})")}


REFUSED_CALLS = ["tracks_add", "tracks_remove", "tracks_restore", "tracks_reorder", "track_rename", "tracks_scan", "project_close",
                 "project_open", "project_new", "project_save_as", "project_rename", "export_all", "transport_play", "feedback_send"]


def refuse_parts(rc: int, rows: list) -> dict:
    reps = page_reports(rows, "refuse")
    jobs = job_rows_of(rows)
    failed = [r for r in rows if r.get("event") == "job_failed"]
    if not reps:
        return {"editor-refuse": (False, f"no refuse report (rc={rc})")}
    r = reps[0]
    calls = r["calls"]
    bad = [c for c in REFUSED_CALLS if calls.get(c, {}).get("code") != 403]
    host_named = all("host" in calls.get(c, {}).get("message", "") or "not in the plugin" in calls.get(c, {}).get("message", "")
                     or "cache folder" in calls.get(c, {}).get("message", "") for c in REFUSED_CALLS)
    return {"editor-refuse": (
        rc == 0 and not bad and host_named and r.get("tracks_same") is True and len(jobs) >= 2 and not failed
        and len(jobs[-1].get("project_tracks", [])) == len(FILES) + 1,
        f"{len(REFUSED_CALLS) - len(bad)} of {len(REFUSED_CALLS)} commands that change the track set or the project files answered 403 "
        f"(not 403: {bad}), the track list is unchanged ({r.get('tracks_same')}), and a later round still ran "
        f"({len(jobs)} rounds, {len(failed)} failed, {len(jobs[-1].get('project_tracks', [])) if jobs else 0} tracks)")}


def race_parts(rc: int, rows: list) -> dict:
    reps = page_reports(rows, "analyse")
    jobs = job_rows_of(rows)
    failed = [r for r in rows if r.get("event") == "job_failed"]
    r = reps[0] if reps else {}
    return {"editor-race": (
        rc == 0 and r.get("state") == "done" and len(jobs) >= 2 and not failed
        and len(jobs[-1].get("project_tracks", [])) == len(FILES) + 2,
        f"a round of the DAW (a new source) came while the page's own analysis ran: the page's job ended '{r.get('state')}', "
        f"{len(jobs)} rounds, {len(failed)} failed round(s) {[x.get('error') for x in failed][:1]}")}


def stage_editor_follow() -> bool:
    ref = _reference()
    if ref is None or not HOST.exists():
        return pass_fail("editor-follow", False, "no reference or host")
    ok = True
    extra = ["--late-files", EXTRA_SYNTH, "--arg", str(len(FILES) + 1)]
    rc, summary, rows = run_phase("editor-follow", "edit-round", CLAP, args=extra)
    ok = print_parts("", keep_edits_parts(rc, rows)) and ok
    rc, summary, rows = run_phase("editor-follow-vst3", "edit-round", EMBEDDED_CLAP, args=extra)
    ok = print_parts("-vst3", keep_edits_parts(rc, rows)) and ok
    late = ["--late-files", *[str(FIXTURES / f) for f in FILES], "--arg", str(len(FILES))]
    rc, summary, rows = run_phase("editor-open-first", "open-first", CLAP, args=late, files=[])
    ok = print_parts("", open_first_parts(rc, rows, ref)) and ok
    return ok


def stage_editor_follow_fault() -> bool:
    ref = _reference()
    if ref is None:
        return pass_fail("editor-follow-fault", False, "no reference")
    extra = ["--late-files", EXTRA_SYNTH, "--arg", str(len(FILES) + 1)]
    ok = True
    rc, summary, rows = run_phase("editor-fault-keep", "edit-round", CLAP, {"DS_ARA_EDITOR_FAULT": "drop-edits"}, extra)
    ok = fault_part("editor-keep-edits-fault", keep_edits_parts(rc, rows), "editor-keep-edits", "a round drops the hand edits of the page") and ok
    rc, summary, rows = run_phase("editor-fault-follow", "edit-round", CLAP, {"DS_ARA_EDITOR_FAULT": "no-reload"}, extra)
    ok = fault_part("editor-follow-fault", keep_edits_parts(rc, rows), "editor-follow", "the page is not told that a round ended") and ok
    late = ["--late-files", *[str(FIXTURES / f) for f in FILES], "--arg", str(len(FILES))]
    rc, summary, rows = run_phase("editor-fault-open-first", "open-first", CLAP, {"DS_ARA_EDITOR_FAULT": "no-reload"}, late, files=[])
    ok = fault_part("editor-open-first-fault", open_first_parts(rc, rows, ref), "editor-open-first", "the page is not told that the first round ended") and ok
    return ok


def stage_editor_refuse() -> bool:
    rc, summary, rows = run_phase("editor-refuse", "refuse", CLAP, args=["--late-files", EXTRA_SYNTH])
    return print_parts("", refuse_parts(rc, rows))


def stage_editor_refuse_fault() -> bool:
    rc, summary, rows = run_phase("editor-fault-refuse", "refuse", CLAP, {"DS_ARA_EDITOR_FAULT": "allow-track-set"}, ["--late-files", EXTRA_SYNTH])
    return fault_part("editor-refuse-fault", refuse_parts(rc, rows), "editor-refuse", "the page may change the track set")


def race_run(label: str, env: dict | None = None) -> dict:
    rc, summary, rows = run_phase(label, "analyse-race", CLAP, env, ["--late-files", EXTRA_SYNTH],
                                  files=[*[str(FIXTURES / f) for f in FILES], "synth:150"])
    return race_parts(rc, rows)


def stage_editor_race() -> bool:
    return print_parts("", race_run("editor-race"))


def stage_editor_race_fault() -> bool:
    return fault_part("editor-race-fault", race_run("editor-fault-race", {"DS_ARA_ROUND_FAULT": "no-page-wait"}), "editor-race",
                      "a round does not wait for the page's job")


def tree_hashes(root: Path) -> dict:
    out = {}
    for p in sorted(root.rglob("*")):
        if p.is_file():
            out[str(p.relative_to(root))] = hashlib.sha256(p.read_bytes()).hexdigest()
    return out


def page_copy_parts(label: str, copy: Path) -> tuple[bool, str]:
    want = tree_hashes(ROOT / "static")
    got = tree_hashes(copy) if copy.is_dir() else {}
    missing = sorted(set(want) - set(got))
    extra = sorted(set(got) - set(want))
    diff = sorted(k for k in want if k in got and want[k] != got[k])
    return (
        bool(want) and not missing and not extra and not diff,
        f"{label}: {len(got)} files against {len(want)} of static/: {len(missing)} missing, {len(extra)} extra, {len(diff)} with other bytes {(missing + extra + diff)[:3]}",
    )


def stage_editor_page_copy() -> bool:
    ok = True
    for label, copy in (
        ("the CLAP", CLAP / "Contents" / "Resources" / "page"),
        ("the CLAP in the VST3", EMBEDDED_CLAP / "Contents" / "Resources" / "page"),
        ("the delivery CLAP", DELIVERY_CLAP / "Contents" / "Resources" / "page"),
        ("the CLAP in the delivery VST3", DELIVERY_EMBEDDED / "Contents" / "Resources" / "page"),
    ):
        good, ev = page_copy_parts(label, copy)
        ok = pass_fail("editor-page-copy", good, ev) and ok
    return ok


def stage_editor_page_copy_fault() -> bool:
    tmp = OUT / "page-copy-fault"
    if tmp.exists():
        shutil.rmtree(tmp)
    shutil.copytree(CLAP / "Contents" / "Resources" / "page", tmp)
    (tmp / "app.js").write_text((tmp / "app.js").read_text() + "\n// changed\n")
    good, ev = page_copy_parts("a copy with one changed file", tmp)
    ok = pass_fail("editor-page-copy-fault", not good, f"the comparison refuses a changed copy: {ev}")
    (tmp / "app.js").unlink()
    good, ev = page_copy_parts("a copy with one missing file", tmp)
    return pass_fail("editor-page-copy-fault", not good, f"and a missing file: {ev}") and ok


def view_parts(rc: int, rows: list) -> dict:
    opens = [r for r in rows if r.get("event") == "editor_open"]
    ready = [r for r in rows if r.get("event") == "page_ready"]
    closes = [r for r in rows if r.get("event") == "editor_close"]
    in_vst3 = [r for r in opens if "DrumSpikeAra.vst3/Contents/PlugIns/DrumSpikeAra.clap/Contents/Resources/page" in str(r.get("page_dir"))]
    return {"editor-vst3-view": (
        rc == 0 and bool(in_vst3) and bool(ready) and all(r.get("off_main") == 0 for r in closes),
        f"pluginval opened the editor of the wrapped VST3 through the wrapper's view: {len(opens)} editors, the page of {len(in_vst3)} from the "
        f"VST3 bundle, the page ready {len(ready)} times, 0 calls off the main thread (the plugin has no ARA document there)")}


def view_run(label: str, env: dict | None = None) -> dict:
    out_file = OUT / f"report-{label}.jsonl"
    if out_file.exists():
        out_file.unlink()
    e = {**no_switch(), "DS_ARA_CACHE": str(OUT / f"cache-{label}"), "DS_ARA_OUT": str(out_file), **(env or {})}
    rc, text = run([str(PLUGINVAL), "--strictness-level", "5", "--timeout-ms", "120000", str(VST3)], env=e, log=f"{label}.log")
    return view_parts(rc, report_rows(label))


def stage_editor_vst3_view() -> bool:
    if not PLUGINVAL.exists() or not VST3.exists():
        return pass_fail("editor-vst3-view", False, "no pluginval or no VST3")
    return print_parts("", view_run("editor-vst3-view"))


def stage_editor_vst3_view_fault() -> bool:
    if not PLUGINVAL.exists() or not VST3.exists():
        return pass_fail("editor-vst3-view-fault", False, "no pluginval or no VST3")
    return fault_part("editor-vst3-view-fault", view_run("editor-fault-vst3-view", {"DS_ARA_EDITOR_FAULT": "no-ready"}), "editor-vst3-view",
                      "a page that never reports ready")


def edit_sync_parts(label: str, rc: int, rows: list, cache: str) -> dict:
    """R3: without the delay scenario the worker's own sync runs: its row says it saved the project,
    the events are on disk, and the host heard that the notes of the source changed."""
    reps = page_reports(rows, "edit")
    syncs = [r for r in rows if r.get("event") == "page_sync"]
    if not reps:
        return {"editor-edit-sync": (False, f"no edit report (rc={rc})")}
    r = reps[0]
    disk = disk_events(cache, r["name"])
    on_disk = disk is not None and any(abs(h["t"] - r["to"]) < 1e-9 for h in disk)
    log = (OUT / f"host-{label}.log").read_text(errors="replace") if (OUT / f"host-{label}.log").exists() else ""
    told = [ln for ln in log.splitlines() if '"host_content_changed"' in ln and r["name"] in ln]
    return {"editor-edit-sync": (
        rc == 0 and bool(syncs) and any(x.get("saved") for x in syncs) and on_disk and bool(told),
        f"with no delay the worker's page_sync ran ({len(syncs)} row(s), saved {[x.get('saved') for x in syncs]}), the moved hit is in the project on disk ({on_disk}), "
        f"and the host was told that the notes of {r['name']} changed ({len(told)} notification(s))")}


def edit_sync_run(label: str, env: dict | None = None) -> dict:
    rc, summary, rows = run_phase(label, "edit", CLAP, env, ["--wait-sync"])
    return edit_sync_parts(label, rc, rows, f"cache-{label}")


def stage_editor_edit_sync() -> bool:
    if _reference() is None or not HOST.exists():
        return pass_fail("editor-edit-sync", False, "no reference or host")
    return print_parts("", edit_sync_run("editor-edit-sync"))


def stage_editor_edit_sync_fault() -> bool:
    # the worker never hears about the change: no sync row, no save by the worker
    return fault_part("editor-edit-sync-fault", edit_sync_run("editor-fault-edit-sync", {"DS_ARA_EDITOR_FAULT": "no-sync"}), "editor-edit-sync",
                      "the worker does not hear about the change")


def disk_events(cache: str, name: str) -> list | None:
    docs = project_docs(cache)
    if not docs:
        return None
    tr = tracks_of_doc(docs[0]).get(name) or {}
    return tr.get("events")


def edit_destroy_parts(rc: int, rows: list, archive: Path, cache: str, ref: bytes, restore_rc: int, restore_rows: list) -> dict:
    reps = page_reports(rows, "edit")
    if not reps or reps[0].get("error"):
        why = f"the page's edit did not run (rc={rc})"
        return {"editor-edit-destroy-archive": (False, why), "editor-edit-destroy-disk": (False, why)}
    r = reps[0]
    after = json.loads(r["after_raw"])["events"]
    restored = [x for x in restore_rows if x.get("event") == "doc_restored"]
    got = json.loads(restored[-1]["events_json"]) if restored else {}
    disk = disk_events(cache, r["name"])
    return {
        "editor-edit-destroy-archive": (
            rc == 0 and restore_rc == 0 and got.get(r["name"]) == after,
            f"the archive stored the moment the edit was answered, before the destroy, restored in a new process holds the edit "
            f"({len(after)} hits of {r['name']}; restored list has {len(got.get(r['name'], []))} hits, restore rc {restore_rc}, "
            f"{len(restored)} doc_restored row(s), first differing hit {next((i for i, (a, b) in enumerate(zip(after, got.get(r['name'], []))) if a != b), None)})"),
        "editor-edit-destroy-disk": (
            disk is not None and any(abs(h["t"] - r["to"]) < 1e-9 for h in disk) and not any(abs(h["t"] - r["from"]) < 1e-9 for h in disk),
            f"the project on disk after the destroy of the controller holds the moved hit at {r['to']} (and not at {r['from']}): "
            f"{None if disk is None else len(disk)} hits of {r['name']}"),
    }


def edit_destroy_run(label: str, env: dict | None = None) -> dict:
    ref = _reference()
    archive = OUT / f"{label}.ara"
    if archive.exists():
        archive.unlink()
    cache = f"cache-{label}"
    rc, summary, rows = run_phase(label, "edit-destroy", CLAP, {"DS_ARA_EDITOR_SCENARIO": "sync-delay", **(env or {})}, ["--store-at-edit", str(archive)])
    rrc, rtext, rlines = host_run(f"{label}-restore", CLAP, ["restore", "--archive", str(archive), "--files", *[str(FIXTURES / f) for f in FILES]], f"cache-{label}-r")
    return edit_destroy_parts(rc, rows, archive, f"cache-{label}", ref, rrc, report_rows(f"{label}-restore"))


def stage_editor_edit_destroy() -> bool:
    if _reference() is None or not HOST.exists():
        return pass_fail("editor-edit-destroy", False, "no reference or host")
    return print_parts("", edit_destroy_run("editor-edit-destroy"))


def stage_editor_edit_destroy_fault() -> bool:
    if _reference() is None:
        return pass_fail("editor-edit-destroy-fault", False, "no reference")
    parts = edit_destroy_run("editor-fault-edit-destroy", {"DS_ARA_EDITOR_FAULT": "no-record"})
    return fault_part("editor-edit-destroy-fault", parts, "editor-edit-destroy-archive", "the request thread does not record the edit")


def regions_keep_parts(rc: int, rows: list, ref: bytes) -> dict:
    """M1: the user moves a hit and sets a role in the page, then the DAW trims the region of that
    source: the hit and the role are still there; hits outside the new region stay in the event
    list (the notes leave them out); the other tracks are as the service gives them."""
    edits = page_reports(rows, "edit")
    fol = page_reports(rows, "follow")
    jobs = job_rows_of(rows)
    if not edits or not fol:
        return {"editor-regions-keep": (False, f"no edit or follow report (rc={rc})")}
    e, f = edits[0], fol[0]
    name = e["name"]
    evs = json.loads(f["raw_events"][name])["events"]
    moved = any(abs(h["t"] - e["to"]) < 1e-9 for h in evs)
    role = (f.get("roles") or {}).get(name)
    changed_round = len(jobs) >= 2 and jobs[-1].get("reason") in ("change", "request")
    outside = [h["t"] for h in evs if h["t"] >= 1.0]   # the region was trimmed to the first second
    refobj = json.loads(ref)
    follower = e.get("follower")
    others_same = all(json.loads(f["raw_events"][n])["events"] == refobj[n] for n in f["tracks"] if n not in (name, follower) and n in refobj)
    still_follows = bool(follower) and (f.get("follows") or {}).get(follower) == name   # R1: the follow link of the track survives the trim
    return {"editor-regions-keep": (
        rc == 0 and f.get("ok") is True and moved and role == "cymbal" and changed_round and others_same and still_follows,
        f"after a hit moved and a role set in the page, the host trimmed the region of {name}: {len(jobs)} rounds, "
        f"the moved hit is still in the events ({moved}), the role is {role!r} (set: 'cymbal'), the other tracks equal the reference ({others_same}), {follower!r} still follows it ({still_follows}), "
        f"{len(outside)} hit(s) outside the new region stay in the list")}


def stage_editor_regions_keep() -> bool:
    ref = _reference()
    if ref is None or not HOST.exists():
        return pass_fail("editor-regions-keep", False, "no reference or host")
    rc, summary, rows = run_phase("editor-regions-keep", "edit-region", CLAP, args=["--region", "0,1.0,0"])
    return print_parts("", regions_keep_parts(rc, rows, ref))


def stage_editor_regions_keep_fault() -> bool:
    ref = _reference()
    if ref is None:
        return pass_fail("editor-regions-keep-fault", False, "no reference")
    rc, summary, rows = run_phase("editor-fault-regions-keep", "edit-region", CLAP, {"DS_ARA_EDITOR_FAULT": "drop-track-data"}, ["--region", "0,1.0,0"])
    return fault_part("editor-regions-keep-fault", regions_keep_parts(rc, rows, ref), "editor-regions-keep", "a region change drops the hand data of the track")


def race_destroy_parts(rc: int, rows: list, window: Path) -> dict:
    """M3: the page analyses a long source while the host destroys another source."""
    jobs = job_rows_of(rows)
    failed = [r for r in rows if r.get("event") in ("job_failed",)]
    destroyed = [r for r in rows if r.get("event") == "source_destroyed"]
    wb = json.loads(window.read_text()) if window.exists() else {}
    ev = wb.get("events") or {}
    nonempty = sorted(k for k, v in ev.items() if v)
    keep = [n for n in FILES if safe_name(stem_of(n)) != safe_name(stem_of(FILES[0]))]
    keep_names = [safe_name(stem_of(n)) for n in keep]
    last = jobs[-1].get("project_tracks", []) if jobs else []
    gone = safe_name(stem_of(FILES[0]))
    return {"editor-race-destroy": (
        rc == 0 and not failed and bool(destroyed) and not any(r.get("error") for r in destroyed)
        and all(n in nonempty for n in keep_names) and gone not in last and len(last) == len(FILES),
        f"the host destroyed a source while the page's analysis ran: a store in that window holds the events of {len(nonempty)} tracks "
        f"(want the {len(keep_names)} others: {all(n in nonempty for n in keep_names)}); afterwards the removal ran "
        f"({len(destroyed)} source_destroyed row(s), errors {[r.get('error') for r in destroyed]}), the last round has {len(last)} tracks without {gone}, "
        f"{len(failed)} failed round(s)")}


def race_destroy_run(label: str, env: dict | None = None) -> dict:
    window = OUT / f"{label}.window.ara"
    if window.exists():
        window.unlink()
    rc, summary, rows = run_phase(label, "analyse-race", CLAP, env, ["--destroy-late", "0", "--archive-window", str(window)],
                                  files=[*[str(FIXTURES / f) for f in FILES], "synth:150"])
    return race_destroy_parts(rc, rows, window)


def stage_editor_race_destroy() -> bool:
    return print_parts("", race_destroy_run("editor-race-destroy"))


def stage_editor_race_destroy_fault() -> bool:
    return fault_part("editor-race-destroy-fault", race_destroy_run("editor-fault-race-destroy", {"DS_ARA_ROUND_FAULT": "no-page-wait"}),
                      "editor-race-destroy", "a removal does not wait for the job of the page and a failed removal wipes the events")


def shell_function_parts(page: Path) -> tuple[bool, str]:
    """S4, R5: every function, global and element of the page that the shell script uses exists in the bundled page."""
    src = (HERE / "src" / "editor" / "shell.js").read_text(encoding="utf-8")

    def listed(name: str) -> list:
        m = re.search(rf"var {name} = \[([^\]]*)\]", src)
        return re.findall(r'"([A-Za-z_]+)"', m.group(1)) if m else []

    funcs, globs, ids = listed("PAGE_FUNCTIONS"), listed("PAGE_GLOBALS"), listed("PAGE_IDS")
    text = "\n".join(p.read_text(encoding="utf-8", errors="replace") for p in sorted(page.glob("*.js")))
    html = (page / "index.html").read_text(encoding="utf-8", errors="replace")
    missing = [n for n in funcs if not re.search(rf"function\s+{n}\s*\(", text)]
    missing += [n for n in globs if not re.search(rf"(const|let|var)\s+{n}\b", text)]
    missing += ["#" + n for n in ids if f'id="{n}"' not in html]
    # S.view is the page's own object: its two keys are in the state
    if not re.search(r"view:\s*\{\s*t0", text):
        missing.append("S.view")
    total = len(funcs) + len(globs) + len(ids) + 1
    return (bool(funcs and globs and ids) and not missing,
            f"the shell script uses {total} parts of the page (functions {funcs}, globals {globs} and S.view, element {ids}); {len(missing)} missing in the bundled page {missing}")


def stage_editor_shell_functions() -> bool:
    ok, ev = shell_function_parts(CLAP / "Contents" / "Resources" / "page")
    return pass_fail("editor-shell-functions", ok, ev)


def stage_editor_shell_functions_fault() -> bool:
    tmp = OUT / "page-fn-fault"
    if tmp.exists():
        shutil.rmtree(tmp)
    shutil.copytree(CLAP / "Contents" / "Resources" / "page", tmp)
    p = tmp / "session.js"
    p.write_text(p.read_text().replace("async function loadProject(", "async function loadProjectRenamed("))
    ok, ev = shell_function_parts(tmp)
    return pass_fail("editor-shell-functions-fault", not ok, f"a renamed function of the page is found: {ev}")


def shell_report_parts(rc: int, rows: list, expect: bool) -> dict:
    """R5 (d): the shell's report of a missing function reaches the plugin: a row `page_shell_report`."""
    reports = [r for r in rows if r.get("event") == "page_shell_report"]
    names = [n for r in reports for n in (r.get("what") or {}).get("missing_page_functions", [])]
    reloaded = bool(page_reports(rows, "follow"))
    if expect:
        return {"editor-shell-report": (rc == 0 and reloaded and "renamedLoadProject" in names,
                f"a function the shell needs is missing in the page: the shell's report reached the plugin as a row ({names})")}
    return {"editor-shell-report": (rc == 0 and reloaded and not reports,
            f"the page has every part the shell uses: a reload ran and the plugin got no missing-function report ({len(reports)} rows)")}


def shell_report_run(label: str, env: dict | None = None, expect: bool = True) -> dict:
    rc, summary, rows = run_phase(label, "edit-round", CLAP, env, ["--late-files", EXTRA_SYNTH, "--arg", str(len(FILES) + 1)])
    return shell_report_parts(rc, rows, expect)


def stage_editor_shell_report() -> bool:
    ok = print_parts("", shell_report_run("editor-shell-report", None, False))
    return ok


def stage_editor_shell_report_fault() -> bool:
    # the injected missing function must reach the plugin as a row; the quiet check of the normal run refuses that run
    parts = shell_report_run("editor-fault-shell-report", {"DS_ARA_EDITOR_FAULT": "missing-function"}, False)
    ok = fault_part("editor-shell-report-fault", parts, "editor-shell-report", "a function of the page is missing")
    parts = shell_report_run("editor-fault-shell-report2", {"DS_ARA_EDITOR_FAULT": "missing-function"}, True)
    return print_parts("", parts) and ok



# ---------------------------------------------------------------- M1b5: the host transport (#546)
#
# The test host plays a transport script through process calls (it makes the plugin's audio thread
# see play, stop and locate) and writes its own truth on the wall clock (the row transport_script).
# The page driver samples what the page draws, a sample a frame, with the same clock. The stage
# compares the two. The plugin's regions are one region for every source (start in the source 0.4 s,
# length 2.4 s, at 2.0 s of the song), so the page time is the song position minus 1.6 s.

TRANSPORT_REGION = (0.4, 2.4, 2.0)
TRANSPORT_ERR_MS = 50.0
TRANSPORT_RATE = (15.0, 40.0)   # messages a second while the host plays
# The 50 ms limit holds over the stable part of each play stretch: the first 0.5 s after a start or a
# locate (the page anchors, a loaded machine may be late) and the last 0.1 s before the next change are not counted.
STABLE_FROM_S = 0.5


def transport_files() -> list:
    return [str(FIXTURES / FILES[0]), str(FIXTURES / FILES[2])]


def host_row(label: str, event: str) -> dict:
    """A row the test host printed to its stdout (out/host-<label>.log)."""
    path = OUT / f"host-{label}.log"
    if path.exists():
        for ln in path.read_text(errors="replace").splitlines():
            if ln.startswith("{") and f'"{event}"' in ln:
                try:
                    return json.loads(ln)
                except ValueError:
                    pass
    return {}


def region_page_time(song: float) -> float:
    """The page time of a song position. Since M1b8 (#555) the page shows song time: the position of the host goes to the
    page as it is, with no mapping through a region or a track (the name stays for the many callers)."""
    return max(0.0, song)


def transport_run(label: str, bundle: Path, env: dict | None = None, kind: str = "script", arg: str = "11.5",
                  mode: str = "transport", args: list | None = None, files: list | None = None) -> tuple[int, dict, list, dict]:
    extra = ["--transport", kind, "--arg", arg, *(args if args is not None else ["--regions-all", ",".join(str(x) for x in TRANSPORT_REGION)])]
    rc, summary, rows = run_phase(label, mode, bundle, env, extra, files=files if files is not None else transport_files())
    return rc, summary, rows, host_row(label, "transport_script")


def transport_parts(rc: int, rows: list, script: dict) -> dict:
    names = ["editor-transport-follow", "editor-transport-stop", "editor-transport-locate", "editor-transport-jump",
             "editor-transport-rate", "editor-transport-own-audio", "editor-transport-source"]
    reps = page_reports(rows, "transport")
    r = reps[0] if reps else {}
    segs = script.get("segments", [])
    samples = r.get("samples", [])
    if not r or r.get("visibility") != "visible" or len(segs) != 9 or len(samples) < 100:
        why = (f"the page is {r.get('visibility')}: the screen of this host is locked or its window occluded, so the page draws no frame"
               if r and r.get("visibility") != "visible" else
               f"no usable run: rc={rc}, page report {bool(r)}, {len(segs)} host segments (want 9), {len(samples)} samples")
        return {n: (False, why) for n in names}
    max_dur = r.get("max_dur") or 3.0

    def at(seg: int, age_s: float) -> list:
        """The sample nearest to `age_s` seconds after the start of the host segment."""
        w = segs[seg]["wall_ms"] + age_s * 1000.0
        return min(samples, key=lambda x: abs(x[0] - w))

    def truth_song(seg: int, sample: list) -> float:
        g = segs[seg]
        return g["song"] + ((sample[0] - g["wall_ms"]) / 1000.0 if g["playing"] else 0.0)

    def err_ms(seg: int, sample: list) -> float:
        return abs((sample[1] if sample[1] is not None else 1e9) - min(region_page_time(truth_song(seg, sample)), max_dur)) * 1000.0

    play_segs = [1, 4, 5]
    # the three sample points of the first play stretch, and every sample of all play stretches
    pts = [at(1, a) for a in (0.6, 1.1, 1.6)]
    pt_err = [err_ms(1, x) for x in pts]
    moved = pts[2][1] - pts[0][1] if None not in (pts[2][1], pts[0][1]) else 0.0
    ends = [segs[i + 1]["wall_ms"] if i + 1 < len(segs) else 1e18 for i in range(len(segs))]
    every = []
    for g in play_segs:
        for x in samples:
            age = (x[0] - segs[g]["wall_ms"]) / 1000.0
            if STABLE_FROM_S <= age and x[0] < ends[g] - 100.0:
                every.append(err_ms(g, x))
    worst = max(every) if every else 1e9
    mean = sum(every) / len(every) if every else 1e9
    if moved < 0.8:
        follow = (False, f"playhead did not move: the page time went {pts[0][1]} -> {pts[2][1]} s over 1 s of host play ({len(samples)} samples)")
    else:
        follow = (max(pt_err) <= TRANSPORT_ERR_MS and worst <= TRANSPORT_ERR_MS,
                  f"the page follows the host during play: error at the three sample points {[round(e, 1) for e in pt_err]} ms, over all "
                  f"{len(every)} samples of the stable part (from {STABLE_FROM_S} s after a start or locate to 0.1 s before the next change) of three play stretches max {worst:.1f} ms, mean {mean:.1f} ms (limit {TRANSPORT_ERR_MS:.0f} ms); "
                  f"the page time moved {moved:.2f} s in 1 s" if max(pt_err) <= TRANSPORT_ERR_MS and worst <= TRANSPORT_ERR_MS else
                  f"the playhead is off by up to {worst:.1f} ms (the three points {[round(e, 1) for e in pt_err]} ms; limit {TRANSPORT_ERR_MS:.0f} ms)")

    def held(seg_list: list, what: str) -> tuple[bool, str]:
        out = []
        for g in seg_list:
            x = at(g, 0.6)
            exp = min(region_page_time(segs[g]["song"]), max_dur)
            out.append((g, x[1], round(exp, 4), x[2]))
        ok_ = all(p is not None and abs(p - e) <= 0.003 and pl == 0 for _g, p, e, pl in out)
        return ok_, f"{what}: " + ", ".join(f"segment {g}: page {p} s (want {e}), playing={pl}" for g, p, e, pl in out)

    stop_ok, stop_ev = held([2, 6], "after a stop of the host the page stands at the host's position")
    loc_ok, loc_ev = held([3, 7, 8], "after a locate of the stopped host (the last two before and after the region) the page shows")
    before, after = at(5, -0.05), at(5, 0.5)
    jump_err = max(err_ms(5, x) for x in samples if 0.3 <= (x[0] - segs[5]["wall_ms"]) / 1000.0 < 1.1)
    step = abs((after[1] or 0.0) - (before[1] or 0.0))
    jump = (step > 1.0 and jump_err <= TRANSPORT_ERR_MS,
            f"a locate of the playing host (song 3.2+ -> 2.2): the page time stepped {step:.2f} s and is then within {jump_err:.1f} ms (limit {TRANSPORT_ERR_MS:.0f} ms)")
    log = r.get("log", [])

    def msgs_between(a: float, b: float) -> int:
        return sum(1 for t, _why in log if a <= t < b)

    pa, pb = segs[1]["wall_ms"] + 300.0, segs[2]["wall_ms"] - 100.0
    rate = msgs_between(pa, pb) / ((pb - pa) / 1000.0)
    still = msgs_between(segs[3]["wall_ms"] + 600.0, segs[4]["wall_ms"] - 100.0) + msgs_between(segs[7]["wall_ms"] + 600.0, segs[8]["wall_ms"] - 100.0)
    rate_ok = TRANSPORT_RATE[0] <= rate <= TRANSPORT_RATE[1] and still == 0
    rate_ev = (f"{rate:.1f} messages a second while the host plays (want {TRANSPORT_RATE[0]:.0f} to {TRANSPORT_RATE[1]:.0f}), {still} while it is stopped and unchanged (want 0)"
               if rate_ok else
               (f"too many updates: {rate:.1f} messages a second while the host plays (limit {TRANSPORT_RATE[1]:.0f})" if rate > TRANSPORT_RATE[1]
                else f"{rate:.1f} messages a second while the host plays (want {TRANSPORT_RATE[0]:.0f} to {TRANSPORT_RATE[1]:.0f}), {still} while stopped (want 0)"))
    closes = [x for x in rows if x.get("event") == "editor_close"]
    tcmds = closes[-1].get("transport_commands") if closes else None
    refused = str(r.get("play", "")).startswith("rejected") and r.get("wired") is True
    if not closes or tcmds is None:
        own = (False, "no editor_close row: the run did not end cleanly, the count of page commands is unknown")
    elif tcmds != 0:
        own = (False, f"the page ran {tcmds} transport command(s) while the host plays (want 0); play is {r.get('play')!r}")
    elif not refused:
        own = (False, f"the page's own play was not refused: {r.get('play')!r}, wired {r.get('wired')}")
    else:
        own = (True, f"transport commands of the page while the host plays: 0 (want 0); the page's own play is {r.get('play')!r}; the listener is wired: True")
    seen = [x for x in rows if x.get("event") == "transport_seen"]
    first = [x for x in rows if x.get("event") == "transport_page_first"]
    src = (bool(seen) and seen[0].get("source") == "clap_process" and bool(first) and first[0].get("source") == "clap_process",
           f"first transport seen by the audio thread: source {seen[0].get('source') if seen else None}, position {seen[0].get('position_seconds') if seen else None} s; "
           f"first message to the page: host {first[0].get('host_position_seconds') if first else None} s = {first[0].get('host_position_samples') if first else None} samples "
           f"at {first[0].get('sample_rate') if first else None} Hz, page time {first[0].get('page_time') if first else None} s")
    return dict(zip(names, [follow, (stop_ok, stop_ev), (loc_ok, loc_ev), jump, (rate_ok, rate_ev), own, src]))


def stage_editor_transport() -> bool:
    """Gate lines 1 to 5 (M1b5): the page follows the host transport, on the CLAP and on the plugin
    inside the VST3 bundle."""
    ok = True
    for which, bundle, suffix in (("clap", CLAP, ""), ("vst3", EMBEDDED_CLAP, "-vst3")):
        rc, _summary, rows, script = transport_run(f"editor-transport-{which}", bundle)
        ok = print_parts(suffix, transport_parts(rc, rows, script)) and ok
    return ok


def fault_reason(stage: str, parts: dict, name: str, what: str, reason: str) -> bool:
    """A fault run passes when the part fails, and for the named reason."""
    good, ev = parts.get(name, (True, "part missing"))
    return pass_fail(stage, (not good) and reason in ev, f"the fault run ({what}) makes {name} fail with '{reason}': {ev}")


def stage_editor_transport_fault() -> bool:
    ok = True
    for label, env, part, what, reason in (
        ("off", {"DS_ARA_TRANSPORT_FAULT": "off"}, "editor-transport-follow", "the hand-over is off: the audio thread stores nothing", "playhead did not move"),
        ("flood", {"DS_ARA_TRANSPORT_FAULT": "flood"}, "editor-transport-rate", "a message every 8 ms", "too many updates"),
        ("own", {"DS_ARA_EDITOR_FAULT": "page-transport"}, "editor-transport-own-audio", "the page runs a command of the app's transport", "transport command"),
    ):
        rc, _summary, rows, script = transport_run(f"editor-fault-transport-{label}", CLAP, env)
        ok = fault_reason(f"editor-transport-fault-{label}", transport_parts(rc, rows, script), part, what, reason) and ok
        if label == "own":
            # a run that ended with no editor_close row must fail the stage, and not for the reason of the fault run
            cut = [x for x in rows if x.get("event") != "editor_close"]
            good, ev = transport_parts(rc, cut, script)["editor-transport-own-audio"]
            ok = pass_fail("editor-transport-fault-noclose", (not good) and "no editor_close row" in ev and "ran" not in ev,
                           f"a run with no editor_close row fails editor-transport-own-audio for its own reason: {ev}") and ok
    return ok


def measure_playing_parts(rc: int, rows: list, summary: dict | None) -> dict:
    parts = {"editor-transport-frames": measure_parts(rc, rows, summary)["editor-measure"]}
    reps = page_reports(rows, "measure")
    r = reps[0] if reps else {}
    followed = bool(r.get("host_playing")) and (r.get("transport_msgs") or 0) >= 50
    parts["editor-transport-follows"] = (
        followed,
        f"during the measure the host played in a loop and the page followed: playing={r.get('host_playing')}, {r.get('transport_msgs')} messages, "
        f"position {r.get('host_pos')} s" if followed else
        f"the page did not follow the host during the measure: playing={r.get('host_playing')}, {r.get('transport_msgs')} messages (want 50 or more)")
    return parts


def stage_editor_transport_measure() -> bool:
    """Gate line 6: the frame time holds with the host playing (a looping transport: 8 s of scroll
    and zoom while the page follows; limit 33 ms at the 95th percentile)."""
    ok = True
    for which, bundle, suffix in (("clap", CLAP, ""), ("vst3", EMBEDDED_CLAP, "-vst3")):
        rc, summary, rows, _ = transport_run(f"editor-transport-measure-{which}", bundle, kind="loop", mode="measure",
                                             args=["--regions-all", "0,1.5,0"], files=[str(FIXTURES / f) for f in FILES])
        ok = print_parts(suffix, measure_playing_parts(rc, rows, summary), ["editor-transport-frames", "editor-transport-follows"]) and ok
    return ok


def stage_editor_transport_measure_fault() -> bool:
    ok = True
    for label, env, part, what in (
        ("slow", {"DS_ARA_EDITOR_FAULT": "slow-frames"}, "editor-transport-frames", "the page draws a slow frame every time"),
        ("off", {"DS_ARA_TRANSPORT_FAULT": "off"}, "editor-transport-follows", "the hand-over is off, the page does not follow"),
    ):
        rc, summary, rows, _ = transport_run(f"editor-fault-transport-measure-{label}", CLAP, env, kind="loop", mode="measure",
                                             args=["--regions-all", "0,1.5,0"], files=[str(FIXTURES / f) for f in FILES])
        if summary.get("window_visible") is False:
            ok = pass_fail(f"editor-transport-measure-fault-{label}", False, "the window is occluded (locked screen)") and ok
            continue
        ok = fault_part(f"editor-transport-measure-fault-{label}", measure_playing_parts(rc, rows, summary), part, what) and ok
    return ok


def none_parts(rc: int, rows: list, kind: str) -> dict:
    reps = page_reports(rows, "transport")
    r = reps[0] if reps else {}
    if not r or r.get("visibility") != "visible":
        return {"editor-transport-none": (False, f"no usable run: rc={rc}, page report {bool(r)}, page {r.get('visibility')}")}
    samples = r.get("samples", [])
    positions = sorted({x[1] for x in samples})
    playing = sum(x[2] for x in samples)
    errors = [x for x in rows if x.get("event") in ("page_report",) and x.get("kind") == "js_error"] + \
             [x for x in rows if x.get("event") in ("page_shell_report", "page_panic_caught", "page_record_failed")]
    rowset = [x["event"] for x in rows if x.get("event") in ("transport_seen", "transport_page_first")]
    good = (rc == 0 and len(samples) > 60 and positions == [0.0] and playing == 0 and r.get("messages") == 0 and r.get("wired") is True
            and not errors and not rowset)
    return {"editor-transport-none": (
        good,
        f"host with {kind}: the page stayed at {positions} s over {len(samples)} frames, playing in {playing} of them, {r.get('messages')} messages, "
        f"{len(errors)} error rows, rows {rowset or 'none'}, the listener is wired: {r.get('wired')}" if good else
        f"the playhead moved or a message came with no transport: positions {positions[:5]}, playing in {playing} frames, {r.get('messages')} messages, "
        f"{len(errors)} error rows, rows {rowset}")}


def stage_editor_transport_none() -> bool:
    """Gate line 7: a host that never makes a process call, and one whose process calls carry no
    transport, leave the page in a defined state: the playhead stays at 0, no message, no error row."""
    ok = True
    for kind in ("none", "null"):
        rc, _s, rows, _ = transport_run(f"editor-transport-{kind}", CLAP, kind=kind, arg="4")
        ok = print_parts(f"-{kind}", none_parts(rc, rows, kind)) and ok
    return ok


def stage_editor_transport_none_fault() -> bool:
    ok = True
    rc, _s, rows, _ = transport_run("editor-fault-transport-invent", CLAP, {"DS_ARA_TRANSPORT_FAULT": "invent"}, kind="none", arg="4")
    ok = fault_part("editor-transport-none-fault-invent", none_parts(rc, rows, "none"), "editor-transport-none",
                    "the plugin tells the page that the host plays when it saw no transport") and ok
    # a host that does send a transport must not pass the quiet check
    rc, _s, rows, _ = transport_run("editor-fault-transport-sends", CLAP, kind="loop", arg="4")
    return fault_part("editor-transport-none-fault-sends", none_parts(rc, rows, "loop"), "editor-transport-none",
                      "the host plays (the check for a quiet host must see it)") and ok


def deactivate_parts(rc: int, rows: list, script: dict) -> dict:
    """The host plays, then deactivates the plugin with no stopped process call: the page ends stopped."""
    reps = page_reports(rows, "transport")
    r = reps[0] if reps else {}
    segs = script.get("segments", [])
    samples = r.get("samples", [])
    if not r or r.get("visibility") != "visible" or len(segs) != 2 or len(samples) < 60:
        return {"editor-transport-deactivate": (False, f"no usable run: rc={rc}, page report {bool(r)}, page {r.get('visibility')}, {len(segs)} host segments (want 2), {len(samples)} samples")}
    end = segs[1]["wall_ms"]
    before = [x for x in samples if segs[0]["wall_ms"] + 700.0 <= x[0] < end - 100.0]
    after = [x for x in samples if x[0] >= end + 700.0]
    playing_before = sum(x[2] for x in before)
    playing_after = sum(x[2] for x in after)
    good = rc == 0 and bool(before) and playing_before == len(before) and bool(after) and playing_after == 0
    return {"editor-transport-deactivate": (
        good,
        f"the host played {len(before)} frames, then deactivated the plugin with no stopped process call: the page played in {playing_before} of them and was stopped in all {len(after)} frames from 0.7 s later"
        if good else
        f"the page kept playing after the host deactivated: playing in {playing_after} of {len(after)} frames from 0.7 s after the deactivate (before: {playing_before} of {len(before)})")}


def stage_editor_transport_deactivate() -> bool:
    """A host that stops by deactivating the plugin, with no stopped process call, must not leave the
    page running from the old position."""
    rc, _s, rows, script = transport_run("editor-transport-deactivate", CLAP, kind="deactivate", arg="5")
    return print_parts("", deactivate_parts(rc, rows, script))


def stage_editor_transport_deactivate_fault() -> bool:
    rc, _s, rows, script = transport_run("editor-fault-transport-deactivate", CLAP, {"DS_ARA_TRANSPORT_FAULT": "keep-playing"}, kind="deactivate", arg="5")
    good, ev = deactivate_parts(rc, rows, script)["editor-transport-deactivate"]
    return pass_fail("editor-transport-deactivate-fault", (not good) and "kept playing" in ev,
                     f"the fault run (a cleared slot does not stop the page) makes editor-transport-deactivate fail with 'kept playing': {ev}")


def handover_run(label: str, env: dict | None = None) -> tuple[int, str]:
    e = {"ARA_API_DIR": str(DEPS / "ARA_SDK" / "ARA_API"), "LIBCLANG_PATH": f"{CLT}/usr/lib"}
    e.update(env or {})
    return run(["cargo", "test", "--locked", "--features", "test-switches", "--lib", "transport::tests::", "--", "--test-threads=1"], env=e, log=f"{label}.log", cwd=HERE)


def stage_transport_handover() -> bool:
    """Gate line 3: the hand-over from the audio thread is lock-free. The unit tests name the mechanism:
    one sequence lock made of atomics (no mutex, no pointer: the size of the slot is the sum of its atomics),
    a reader that never sees a torn value with two writers, and no allocation in the audio thread's call."""
    rc, text = handover_run("transport-handover")
    m = re.search(r"test result: (\w+)\. (\d+) passed; (\d+) failed", text)
    needed = ["the_audio_thread_stores_only_atomics_and_a_reader_never_sees_a_torn_value", "the_audio_thread_call_allocates_nothing_and_takes_no_lock"]
    ran = all(f"transport::tests::{n} ... ok" in text for n in needed)
    return pass_fail("transport-handover", rc == 0 and bool(m) and ran,
                     f"the sequence lock of atomics (no mutex): {m.group(2) if m else '?'} unit tests passed, among them the torn-read test with two writers "
                     f"and the no-allocation test of the audio thread's call (out/transport-handover.log)")


def stage_transport_handover_fault() -> bool:
    rc, text = handover_run("fault-transport-handover", {"DS_ARA_TRANSPORT_FAULT": "alloc"})
    named = "the audio thread's call allocated" in text
    return pass_fail("transport-handover-fault", rc != 0 and named,
                     f"the fault run (the audio thread allocates in its call) makes the unit test fail with 'the audio thread's call allocated': rc={rc}, named {named}")



# ---------------------------------------------------------------- M1b7: the page moves the host
#
# The test host has an ARA playback controller (--playback full) that records every request and moves its
# transport (--transport follow), and a grid (--snap 0.5) that a position request lands on. The page
# driver (mode playback) does what a user does: a click in the ruler (S.playhead = t), the Play button
# (play()), the Stop button (pause()). The host's record says what it was asked (seconds and samples,
# the thread, the wall time); the page's samples say where the page was, with the same clock.
# The grid makes the click of the page and the answer of the host differ, so the check can tell the
# page's own move from the host's position that came back.

PB_SNAP = 0.5
PB_RATE = 48000
# what the driver does, in order: (step, request kind of the host, page time of the click or None)
PB_STEPS = [("locate-stopped", "locate", 1.2), ("play", "play", None), ("locate-playing", "locate", 0.6),
            ("stop", "stop", None), ("locate-stopped-2", "locate", 2.6)]
PB_LOOP_MS = 100.0
PB_TRACK_STEP = 0.5   # the second source lies this many seconds later in the song than the first


def playback_run(label: str, bundle: Path, scenario: str = "full", env: dict | None = None) -> tuple[int, dict, list, dict]:
    """scenario: full (all three calls), partial (no position call), ignore (the host records and does nothing),
    none (no playback controller)."""
    args = ["--transport", "follow", "--arg", scenario, "--snap", str(0.0 if scenario == "tracks" else PB_SNAP),
            "--regions-all", ",".join(str(x) for x in TRANSPORT_REGION)]
    files = transport_files()
    if scenario == "tracks":
        # M1b8: sources 0 and 1 lie on ONE DAW track (clip 1 at song 2.0 s, clip 2 of another file 0.5 s later), source 2 on a second one
        files = [*files, str(FIXTURES / FILES[1])]
        args += ["--regions-step", str(PB_TRACK_STEP), "--seq-names", "Drum A", "Drum B", "--seq-of", "0", "0", "1"]
    if scenario != "none":
        args += ["--playback", {"drag": "ignore", "late": "full", "tracks": "full"}.get(scenario, scenario)]
    if scenario == "late":
        args += ["--playback-delay", "2500"]
    rc, summary, rows = run_phase(label, "playback", bundle, env, args, files=files)
    return rc, summary, rows, host_row(label, "playback_record")


def pb_samples_of(seconds: float) -> int:
    return int(math.floor(seconds * PB_RATE + 0.5))


def pb_unusable(names: list, rc: int, rows: list, rep: dict, rec: dict, scenario: str) -> dict | None:
    """The parts all fail when the run did not happen: no page report, a hidden page, no record of the host."""
    steps = rep.get("steps", []) if rep else []
    if not rep or rep.get("visibility") != "visible" or len(steps) < 5 or len(rep.get("samples", [])) < 100 or not rec:
        why = (f"the page is {rep.get('visibility')}: the screen of this host is locked or its window occluded, so the page draws no frame"
               if rep and rep.get("visibility") != "visible" else
               f"no usable run: rc={rc}, page report {bool(rep)}, {len(steps)} steps (want 5), {len(rep.get('samples', [])) if rep else 0} samples, "
               f"host record {bool(rec)}")
        return {n: (False, why) for n in names}
    return None


def pb_first(samples: list, after_ms: float, pred) -> list | None:
    for x in samples:
        if x[0] >= after_ms and pred(x):
            return x
    return None


def playback_parts(rc: int, rows: list, rec: dict) -> dict:
    """Gate lines 1 to 4, 6 and 7 of M1b7 for the host with a full playback controller."""
    names = ["editor-playback-play", "editor-playback-stop", "editor-playback-locate", "editor-playback-loop",
             "editor-playback-thread", "editor-playback-own-audio", "editor-playback-report"]
    reps = page_reports(rows, "playback")
    rep = reps[0] if reps else {}
    bad = pb_unusable(names, rc, rows, rep, rec, "full")
    if bad:
        return bad
    a, _d, p = TRANSPORT_REGION
    samples, steps, calls = rep["samples"], rep["steps"], rep["calls"]
    reqs = rec.get("requests", [])
    kinds = [r["kind"] for r in reqs]
    want = [k for _s, k, _t in PB_STEPS]
    # one call of the shell and one request of the host per step, in the order of the driver
    seq_ok = kinds == want and [c["kind"] for c in calls] == want
    seq_ev = f"the host got {kinds} (want {want}); the shell made {[c['kind'] for c in calls]}"

    def page_of(song: float) -> float:
        return region_page_time(song)

    # --- play: the host starts, the page shows playing when the host says so
    play_i = 1
    play_ok, play_ev = False, seq_ev
    if len(reqs) > play_i and kinds[play_i] == "play" and len(calls) > play_i:
        c = calls[play_i]
        x = pb_first(samples, c["at"], lambda s: s[2] == 1)
        ms = (x[0] - c["at"]) if x else None
        play_ok = reqs[play_i]["playing_after"] is True and steps[play_i]["playing_after"] is True and ms is not None and ms <= PB_LOOP_MS
        play_ev = (f"play from the page: the host got requestStartPlayback and plays (host record playing={reqs[play_i]['playing_after']}), the page shows playing "
                   f"{ms:.0f} ms after the call (limit {PB_LOOP_MS:.0f} ms), host plays from {reqs[play_i]['landed_song']} s" if play_ok else
                   f"the host got no play request or did not start: kinds {kinds}, host playing={reqs[play_i]['playing_after']}, page playing={steps[play_i]['playing_after']}, "
                   f"page shows playing after {ms} ms")
    elif not any(k == "play" for k in kinds):
        play_ev = f"the host got no play request: {seq_ev}"
    # --- stop
    stop_i = 3
    stop_ok, stop_ev = False, seq_ev
    if len(reqs) > stop_i and kinds[stop_i] == "stop" and len(calls) > stop_i:
        c, landed = calls[stop_i], reqs[stop_i]["landed_song"]
        exp = page_of(landed)
        last = [x for x in samples if steps[stop_i]["at"] <= x[0] <= steps[stop_i]["end"] - 60.0][-1]
        x = pb_first(samples, c["at"], lambda s: s[2] == 0 and abs((s[1] or 1e9) - exp) <= 0.003)
        ms = (x[0] - c["at"]) if x else None
        stop_ok = (reqs[stop_i]["playing_after"] is False and last[2] == 0 and abs((last[1] or 1e9) - exp) <= 0.003 and ms is not None and ms <= PB_LOOP_MS)
        stop_ev = (f"stop from the page: the host got requestStopPlayback and stands at {landed} s; the page shows stopped at page time {last[1]} s (host: {exp:.4f} s) "
                   f"{ms:.0f} ms after the call (limit {PB_LOOP_MS:.0f} ms)" if stop_ok else
                   f"stop did not land: host playing={reqs[stop_i]['playing_after']}, page {last[1]} s playing={last[2]} (host position {exp:.4f} s), converged after {ms} ms")
    elif "stop" not in kinds:
        stop_ev = f"the host got no stop request: {seq_ev}"
    # --- locate: the song position at 0 samples difference, stopped (steps 0 and 4) and playing (step 2)
    loc = []
    for i, (name, kind, t) in enumerate(PB_STEPS):
        if kind != "locate" or i >= len(reqs) or reqs[i]["kind"] != "locate":
            continue
        want_s = pb_samples_of(t)
        got = reqs[i]["samples"]
        loc.append((name, t, want_s, got, (got - want_s) if got is not None else None))
    loc_ok = len(loc) == 3 and all(d == 0 for *_x, d in loc) and seq_ok
    loc_ev = ("a locate from the page lands at the mapped song position, 0 samples difference: " if loc_ok else "the locate is off by samples or missing: ") + \
             ", ".join(f"{n}: page {t} s -> want {w}, host got {g} samples (off by {d})" for n, t, w, g, d in loc) + ("" if seq_ok else f"; {seq_ev}")
    # --- loop: page locate -> host position -> page playhead, within 100 ms
    parts = []
    loop_ok = True
    for i in (0, 2, 4):
        if not (len(calls) > i and calls[i]["kind"] == "locate" and len(reqs) > i and reqs[i]["kind"] == "locate" and len(steps) > i):
            loop_ok = False
            parts.append(f"{PB_STEPS[i][0]}: no locate reached the host ({seq_ev})")
            continue
        c, name, t = calls[i], PB_STEPS[i][0], PB_STEPS[i][2]
        landed, w = reqs[i]["landed_song"], reqs[i]["wall_ms"]
        playing = steps[i]["playing_before"]
        if playing:
            x = pb_first(samples, c["at"], lambda s: s[2] == 1 and abs((s[1] or 1e9) - page_of(landed + (s[0] - w) / 1000.0)) <= 0.02)
        else:
            x = pb_first(samples, c["at"], lambda s: s[2] == 0 and abs((s[1] or 1e9) - page_of(landed)) <= 0.002)
        own = [s for s in samples if c["at"] <= s[0] and (x is None or s[0] < x[0]) and abs((s[1] or 1e9) - t) <= 0.002]
        ms = (x[0] - c["at"]) if x else None
        click_ms = (x[0] - steps[i]["at"]) if x else None
        loop_ok = loop_ok and x is not None and ms <= PB_LOOP_MS
        parts.append(f"{name}: " + (f"the page showed its own click ({t} s) in {len(own)} frames after the call, then the host position {page_of(landed):.3f} s {ms:.0f} ms after the call to the host ({click_ms:.0f} ms after the click)"
                                     if x else f"the host position did not come back to the page: it stayed at {steps[i]['pos_after']} s (host {page_of(landed):.3f} s)"))
    loop_ev = ("page locate -> host position -> page playhead within " + f"{PB_LOOP_MS:.0f} ms: " if loop_ok else
               "the loop is not closed: ") + "; ".join(parts)
    # --- thread: every call to the host on the main thread
    closes = [x for x in rows if x.get("event") == "editor_close"]
    on_main = [r["main_thread"] for r in reqs]
    cl = closes[-1] if closes else {}
    thread_ok = bool(reqs) and all(on_main) and cl.get("playback_calls_main", 0) >= len(reqs) and cl.get("playback_calls_request_thread") == 0 and bool(closes)
    thread_ev = (f"the host saw all {len(reqs)} calls on its main thread (host record), the plugin counted {cl.get('playback_calls_main')} calls on the main thread and "
                 f"{cl.get('playback_calls_request_thread')} on a request thread" if thread_ok else
                 f"a call was made off the main thread: host record {on_main}, plugin counts main {cl.get('playback_calls_main')}, request thread {cl.get('playback_calls_request_thread')}"
                 + ("" if closes else " (no editor_close row)"))
    # --- own audio: no command of the app's transport, no error row
    jserr = [x for x in rows if x.get("event") == "page_report" and x.get("kind") == "js_error"] + \
            [x for x in rows if x.get("event") in ("page_shell_report", "page_panic_caught", "page_record_failed")]
    if not closes or cl.get("transport_commands") is None:
        own = (False, "no editor_close row: the run did not end cleanly, the count of page commands is unknown")
    else:
        own_ok = cl["transport_commands"] == 0 and cl.get("command_errors", 1) == 0 and not jserr and rep.get("wired") is True
        own = (own_ok, f"transport commands of the page during play, stop and locate: {cl['transport_commands']} (want 0), command errors {cl.get('command_errors')} (want 0), "
                       f"{len(jserr)} error rows; the page starts no audio of its own, the host plays" if own_ok else
               f"the page ran {cl['transport_commands']} transport command(s) (want 0), {cl.get('command_errors')} command errors, {len(jserr)} error rows")
    # --- report rows: the offer, and the first request of each kind
    offer = [x for x in rows if x.get("event") == "playback_offer"]
    first = {k: [x for x in rows if x.get("event") == "playback_request" and x.get("kind") == k] for k in ("play", "stop", "locate")}
    loc_row = first["locate"][0] if first["locate"] else {}
    rep_ok = (bool(offer) and offer[0]["offer"].get("request_start") and offer[0]["offer"].get("request_stop") and offer[0]["offer"].get("request_set_position")
              and all(len(v) == 1 and v[0].get("thread") == "main" and v[0].get("made", 0) >= 1 for v in first.values())
              and loc_row.get("song_samples") == (reqs[0]["samples"] if reqs else None))
    rep_row = (rep_ok, f"report rows: playback_offer {offer[0]['offer'] if offer else None}; playback_request once for each of {[k for k, v in first.items() if len(v) == 1]}, "
                       f"the first locate row says song {loc_row.get('song_seconds')} s = {loc_row.get('song_samples')} samples, "
                       f"host was at {loc_row.get('host_position_seconds')} s" if rep_ok else
               f"a report row is missing or wrong: offer {offer[0]['offer'] if offer else None}, request rows by kind {({k: len(v) for k, v in first.items()})}, "
               f"locate row samples {loc_row.get('song_samples')} (host {reqs[0]['samples'] if reqs else None})")
    return dict(zip(names, [(play_ok, play_ev), (stop_ok, stop_ev), (loc_ok, loc_ev), (loop_ok, loop_ev), (thread_ok, thread_ev), own, rep_row]))


def stage_editor_playback() -> bool:
    """M1b7 gate lines 1 to 4, 6 and 7: play, stop and locate of the page move the transport of the test host
    (its ARA playback controller), on the CLAP and on the plugin inside the VST3."""
    ok = True
    for which, bundle, suffix in (("clap", CLAP, ""), ("vst3", EMBEDDED_CLAP, "-vst3")):
        label = f"editor-playback-{which}"
        rc, _summary, rows, rec = playback_run(label, bundle)
        parts = playback_parts(rc, rows, rec)
        loop = "editor-playback-loop"
        if not parts[loop][0]:
            # The limit of 100 ms is the ticket's number and stays. A miss may be the load of this machine
            # or a defect: the measure runs once more and both results are printed with the load. The loop
            # fails only if both runs miss (a defect misses every time, a loaded machine seldom twice).
            load1 = os.getloadavg()[0]
            rc2, _s2, rows2, rec2 = playback_run(label + "-again", bundle)
            second = playback_parts(rc2, rows2, rec2)[loop]
            load2 = os.getloadavg()[0]
            parts[loop] = (second[0], f"first run missed (load {load1:.1f}): {parts[loop][1]} | second run (load {load2:.1f}): {second[1]} | "
                                      f"rule: the loop passes when either run is within {PB_LOOP_MS:.0f} ms")
        ok = print_parts(suffix, parts) and ok
    return ok


def stage_editor_playback_fault() -> bool:
    ok = True
    for label, env, part, what, reason in (
        ("direct", {"DS_ARA_PLAYBACK_FAULT": "direct"}, "editor-playback-thread", "the request thread calls the host itself", "off the main thread"),
        ("skip", {"DS_ARA_PLAYBACK_FAULT": "skip"}, "editor-playback-play", "the plugin calls nothing and says it did", "no play request"),
        ("off", {"DS_ARA_TRANSPORT_FAULT": "off"}, "editor-playback-loop", "the hand-over of the position is off: the host position never comes back", "did not come back"),
        ("own", {"DS_ARA_EDITOR_FAULT": "page-transport"}, "editor-playback-own-audio", "the page runs a command of the app's transport", "transport command"),
    ):
        rc, _summary, rows, rec = playback_run(f"editor-fault-playback-{label}", CLAP, "full", env)
        ok = fault_reason(f"editor-playback-fault-{label}", playback_parts(rc, rows, rec), part, what, reason) and ok
    # a run that did not happen fails every part for its own reason (not for the reason of a fault)
    parts = playback_parts(2, [], {})
    ok = pass_fail("editor-playback-fault-norun", all((not g) and "no usable run" in ev for g, ev in parts.values()),
                   f"a run with no page report and no record of the host fails all {len(parts)} parts with 'no usable run'") and ok
    return ok


def pb_text_ok(text: str | None, *needles: str) -> bool:
    return bool(text) and all(n in text for n in needles)


def playback_none_parts(rc: int, rows: list, rec: dict) -> dict:
    """Gate line 5 and the report row of line 7: a host with no playback controller."""
    names = ["editor-playback-none-text", "editor-playback-none-calm", "editor-playback-none-report"]
    reps = page_reports(rows, "playback")
    rep = reps[0] if reps else {}
    bad = pb_unusable(names, rc, rows, rep, rec, "none")
    if bad:
        return bad
    steps = {s["name"]: s for s in rep["steps"]}
    offer = rep.get("offer") or {}
    host = "ara-host"
    t_loc, t_play = steps["locate-stopped"], steps["play"]
    # the page shows play, stop and locate as not available, with one short text that names the host
    texts = {"play": offer.get("play_text"), "stop": offer.get("stop_text"), "locate": offer.get("locate_text")}
    text_ok = (all(pb_text_ok(v, host) and len(v) <= 90 for v in texts.values())
               and t_play["play_disabled"] is True and pb_text_ok(t_play["play_title"], host)
               and pb_text_ok(t_loc["ruler_title"], host) and pb_text_ok(t_loc["msg"], host) and pb_text_ok(t_play["msg"], host)
               and pb_text_ok(steps["stop-direct"]["msg"], host))
    text_ev = (f"play, stop and locate show as not available, with a text that names the host: play button disabled with '{t_play['play_title']}', ruler '{t_loc['ruler_title']}', "
               f"message after a click '{t_loc['msg']}', after Play '{t_play['msg']}', after Stop '{steps['stop-direct']['msg']}'" if text_ok else
               f"the page does not show the three as not available with the name of the host: offer {texts}, play disabled {t_play['play_disabled']}, "
               f"play title {t_play['play_title']!r}, ruler {t_loc['ruler_title']!r}, messages {t_loc['msg']!r} / {t_play['msg']!r} / {steps['stop-direct']['msg']!r}: the text does not name the host")
    # no error row, no request to the host, the page does not stay where its own click put it, nothing plays
    closes = [x for x in rows if x.get("event") == "editor_close"]
    cl = closes[-1] if closes else {}
    jserr = [x for x in rows if x.get("event") == "page_report" and x.get("kind") == "js_error"] + \
            [x for x in rows if x.get("event") in ("page_shell_report", "page_panic_caught", "page_record_failed")]
    back = abs(t_loc["pos_after"] - 0.0) <= 0.003   # the host stands at 0: the page shows the host again after the refusal
    calm = (bool(closes) and cl.get("command_errors") == 0 and not jserr and not rec.get("requests") and cl.get("playback_calls_main") == 0
            and cl.get("transport_commands") == 0 and back and not t_play["playing_after"])
    calm_ev = (f"no error row written (command errors {cl.get('command_errors')}, {len(jserr)} error rows), the host got {len(rec.get('requests', []))} requests, "
               f"the page shows the host position again after the click ({t_loc['pos_after']} s), nothing plays" if calm else
               f"an error row was written or a call went out: command errors {cl.get('command_errors')}, {len(jserr)} error rows, host requests {len(rec.get('requests', []))}, "
               f"page after the click {t_loc['pos_after']} s (host 0.0), playing {t_play['playing_after']}" + ("" if closes else " (no editor_close row)"))
    # the report rows: what the host gave, and the first refused request of each kind
    off = [x for x in rows if x.get("event") == "playback_offer"]
    un = {k: [x for x in rows if x.get("event") == "playback_unavailable" and x.get("kind") == k] for k in ("play", "stop", "locate")}
    rep_ok = (bool(off) and off[0]["offer"].get("interface_given") is False and not any(off[0]["offer"].get(k) for k in ("request_start", "request_stop", "request_set_position"))
              and all(len(v) == 1 and pb_text_ok(v[0].get("text"), host) and v[0].get("host") == host for v in un.values()))
    rep_row = (rep_ok, f"report rows: playback_offer {off[0]['offer'] if off else None}; playback_unavailable once for each of {[k for k, v in un.items() if len(v) == 1]}, the text '{(un['locate'] or [{}])[0].get('text')}'" if rep_ok else
               f"a report row is missing or wrong: playback_offer {off[0]['offer'] if off else None}, playback_unavailable by kind {({k: len(v) for k, v in un.items()})}")
    return dict(zip(names, [(text_ok, text_ev), (calm, calm_ev), rep_row]))


def stage_editor_playback_none() -> bool:
    """M1b7 gate line 5 (and the report row of line 7): a host that gives no playback controller. The page shows
    play, stop and locate as not available with one text that names the host, and no error row is written."""
    ok = True
    for which, bundle, suffix in (("clap", CLAP, ""), ("vst3", EMBEDDED_CLAP, "-vst3")):
        rc, _summary, rows, rec = playback_run(f"editor-playback-none-{which}", bundle, "none")
        # the host with no controller records nothing: the row of the host is made only with --playback
        rec = rec or {"requests": [], "mode": "none"}
        ok = print_parts(suffix, playback_none_parts(rc, rows, rec)) and ok
    return ok


def stage_editor_playback_none_fault() -> bool:
    ok = True
    for label, env, part, what, reason in (
        ("error-row", {"DS_ARA_PLAYBACK_FAULT": "error-row"}, "editor-playback-none-calm", "a refusal is an error of the page", "an error row was written"),
        ("anonymous", {"DS_ARA_PLAYBACK_FAULT": "anonymous"}, "editor-playback-none-text", "the text does not name the host", "does not name the host"),
    ):
        rc, _summary, rows, rec = playback_run(f"editor-fault-playback-none-{label}", CLAP, "none", env)
        rec = rec or {"requests": [], "mode": "none"}
        ok = fault_reason(f"editor-playback-none-fault-{label}", playback_none_parts(rc, rows, rec), part, what, reason) and ok
    parts = playback_none_parts(2, [], {})
    ok = pass_fail("editor-playback-none-fault-norun", all((not g) and "no usable run" in ev for g, ev in parts.values()),
                   f"a run with no page report fails all {len(parts)} parts with 'no usable run'") and ok
    return ok


def playback_partial_parts(rc: int, rows: list, rec: dict) -> dict:
    """The call is not offered: a host whose interface struct ends before requestSetPlaybackPosition."""
    names = ["editor-playback-partial"]
    reps = page_reports(rows, "playback")
    rep = reps[0] if reps else {}
    bad = pb_unusable(names, rc, rows, rep, rec, "partial")
    if bad:
        return bad
    steps = {s["name"]: s for s in rep["steps"]}
    kinds = [r["kind"] for r in rec.get("requests", [])]
    off = [x for x in rows if x.get("event") == "playback_offer"]
    un = [x for x in rows if x.get("event") == "playback_unavailable"]
    closes = [x for x in rows if x.get("event") == "editor_close"]
    cl = closes[-1] if closes else {}
    host = "ara-host"
    good = (kinds == ["play", "stop"] and bool(off) and off[0]["offer"].get("request_start") and off[0]["offer"].get("request_stop")
            and off[0]["offer"].get("request_set_position") is False
            and [x["kind"] for x in un] == ["locate"] and pb_text_ok(un[0].get("text"), host, "position")
            and pb_text_ok(steps["locate-stopped"]["msg"], host, "position") and steps["play"]["playing_after"] is True and steps["stop"]["playing_after"] is False
            and bool(closes) and cl.get("command_errors") == 0)
    return {"editor-playback-partial": (
        good,
        f"the host gives start and stop, not the position: the host got {kinds}, the locate is refused once with '{(un or [{}])[0].get('text')}' (row playback_unavailable), "
        f"play and stop work, 0 command errors" if good else
        f"wrong for a host without the position call: host got {kinds} (want ['play', 'stop']), offer {off[0]['offer'] if off else None}, unavailable rows {[x['kind'] for x in un]}, "
        f"text {(un or [{}])[0].get('text')!r}, command errors {cl.get('command_errors')}: the text does not name the host or the call")}


def stage_editor_playback_partial() -> bool:
    """M1b7 gate line 5, the call is not offered: the interface struct of the host ends before requestSetPlaybackPosition."""
    rc, _summary, rows, rec = playback_run("editor-playback-partial", CLAP, "partial")
    return print_parts("", playback_partial_parts(rc, rows, rec))


def stage_editor_playback_partial_fault() -> bool:
    rc, _summary, rows, rec = playback_run("editor-fault-playback-partial", CLAP, "partial", {"DS_ARA_PLAYBACK_FAULT": "anonymous"})
    ok = fault_reason("editor-playback-partial-fault-anonymous", playback_partial_parts(rc, rows, rec), "editor-playback-partial",
                      "the text does not name the host", "does not name the host")
    parts = playback_partial_parts(2, [], {})
    return pass_fail("editor-playback-partial-fault-norun", all((not g) and "no usable run" in ev for g, ev in parts.values()),
                     "a run with no page report fails with 'no usable run'") and ok


def playback_refused_parts(rc: int, rows: list, rec: dict) -> dict:
    """The host takes the requests and does nothing: the page must not stay where its own click or play put it."""
    names = ["editor-playback-refused"]
    reps = page_reports(rows, "playback")
    rep = reps[0] if reps else {}
    bad = pb_unusable(names, rc, rows, rep, rec, "ignore")
    if bad:
        return bad
    steps = {s["name"]: s for s in rep["steps"]}
    kinds = [r["kind"] for r in rec.get("requests", [])]
    moved = [n for n in ("locate-stopped", "locate-playing", "locate-stopped-2") if abs(steps[n]["pos_after"] - 0.0) > 0.003]
    plays = steps["play"]["playing_after"] or steps["locate-playing"]["playing_after"]
    good = (not moved and not plays and "locate" in kinds and "play" in kinds and rec.get("final_song") == 0.0 and rec.get("final_playing") is False)
    return {"editor-playback-refused": (
        good,
        f"the host recorded {kinds} and did nothing (it stands at {rec.get('final_song')} s): after each locate the page shows the host position 0.0 s again "
        f"({[steps[n]['pos_after'] for n in ('locate-stopped', 'locate-playing', 'locate-stopped-2')]}), the page does not play (play said '{steps['play']['msg']}')" if good else
        f"the page stayed where its own click or play put it: positions after the locates {[(n, steps[n]['pos_after']) for n in ('locate-stopped', 'locate-playing', 'locate-stopped-2')]} "
        f"(host 0.0 s), playing after play {steps['play']['playing_after']}: the page stayed away from the host")}


def stage_editor_playback_refused() -> bool:
    """A host that ignores the requests (it records them and does not move): the page falls back to the host position."""
    rc, _summary, rows, rec = playback_run("editor-playback-refused", CLAP, "ignore")
    return print_parts("", playback_refused_parts(rc, rows, rec))


def stage_editor_playback_refused_fault() -> bool:
    rc, _summary, rows, rec = playback_run("editor-fault-playback-refused", CLAP, "ignore", {"DS_ARA_PLAYBACK_FAULT": "no-fallback"})
    ok = fault_reason("editor-playback-refused-fault-no-fallback", playback_refused_parts(rc, rows, rec), "editor-playback-refused",
                      "the shell never shows the host position again, and Play ends at once", "stayed")
    parts = playback_refused_parts(2, [], {})
    return pass_fail("editor-playback-refused-fault-norun", all((not g) and "no usable run" in ev for g, ev in parts.values()),
                     "a run with no page report fails with 'no usable run'") and ok


def playback_drag_parts(rc: int, rows: list, rec: dict) -> dict:
    """A drag in the ruler with a host that does not answer: the playhead stays under the pointer, and the host
    position is shown again only after the pointer is up."""
    name = "editor-playback-drag"
    reps = page_reports(rows, "playback")
    rep = reps[0] if reps else {}
    if not rep or rep.get("visibility") != "visible" or not rep.get("steps") or not rec:
        return {name: (False, f"no usable run: rc={rc}, page report {bool(rep)}, page {rep.get('visibility') if rep else None}, host record {bool(rec)}")}
    st = rep["steps"][0]
    during = [v for _t, v in st["during"]]
    jumped = [v for v in during if v is None or v < 0.2]   # the host stands at 0 s: a value near 0 is the fallback
    good = len(during) >= 100 and not jumped and abs(st["pos_after"]) <= 0.003 and len(rep.get("calls", [])) >= 3
    return {name: (good,
                   f"a drag of 3 s in the ruler, the host does nothing: the playhead stayed under the pointer in all {len(during)} samples "
                   f"(lowest {min(during):.2f} s), and after the pointer was up the page shows the host position again ({st['pos_after']} s); "
                   f"{len(rep.get('calls', []))} locates were sent" if good else
                   f"the playhead jumped back under the pointer: {len(jumped)} of {len(during)} samples near the host position (values {jumped[:4]}), "
                   f"after the release {st['pos_after']} s, {len(rep.get('calls', []))} locates")}


def stage_editor_playback_drag() -> bool:
    """X3: no fallback in the middle of a drag (a newer locate is out, the pointer is down)."""
    rc, _summary, rows, rec = playback_run("editor-playback-drag", CLAP, "drag")
    return print_parts("", playback_drag_parts(rc, rows, rec))


def stage_editor_playback_drag_fault() -> bool:
    rc, _summary, rows, rec = playback_run("editor-fault-playback-drag", CLAP, "drag", {"DS_ARA_PLAYBACK_FAULT": "no-drag-guard"})
    ok = fault_reason("editor-playback-drag-fault-no-guard", playback_drag_parts(rc, rows, rec), "editor-playback-drag",
                      "the shell shows the host position again under the pointer", "jumped back under the pointer")
    parts = playback_drag_parts(2, [], {})
    return pass_fail("editor-playback-drag-fault-norun", all((not g) and "no usable run" in ev for g, ev in parts.values()),
                     "a run with no page report fails with 'no usable run'") and ok


def playback_late_parts(rc: int, rows: list, rec: dict) -> dict:
    """X4: the host starts 2.5 s after the request. The page gave up at 2 s (one deadline for the whole chain);
    when the host starts, the follow shows playing and the failure text is gone."""
    name = "editor-playback-late"
    reps = page_reports(rows, "playback")
    rep = reps[0] if reps else {}
    if not rep or rep.get("visibility") != "visible" or not rep.get("gave_up") or not rec:
        return {name: (False, f"no usable run: rc={rc}, page report {bool(rep)}, page {rep.get('visibility') if rep else None}, host record {bool(rec)}")}
    gave, msg = rep["gave_up"], rep.get("final_msg") or ""
    asked = [r["kind"] for r in rec.get("requests", [])]
    good = ("playback failed" in (gave.get("msg") or "") and not gave.get("playing") and rep.get("final_playing") is True
            and "playback failed" not in msg and "play" in asked and rec.get("final_playing") is True)
    return {name: (good,
                   f"the page gave up after 2 s ('{gave.get('msg')}', playing {gave.get('playing')}); the host started 2.5 s after the request and the page then plays "
                   f"(playing {rep.get('final_playing')}), the failure text is gone ('{msg}')" if good else
                   f"after a late start the page is wrong: at the deadline '{gave.get('msg')}' playing {gave.get('playing')}; at the end playing {rep.get('final_playing')}, "
                   f"text '{msg}': the error text is stuck or the page does not follow")}


def stage_editor_playback_late() -> bool:
    """X4: a host that starts to play 2.5 s late ends with the page playing and no stuck error text."""
    rc, _summary, rows, rec = playback_run("editor-playback-late", CLAP, "late")
    return print_parts("", playback_late_parts(rc, rows, rec))


def stage_editor_playback_late_fault() -> bool:
    rc, _summary, rows, rec = playback_run("editor-fault-playback-late", CLAP, "late", {"DS_ARA_PLAYBACK_FAULT": "keep-error"})
    ok = fault_reason("editor-playback-late-fault-keep-error", playback_late_parts(rc, rows, rec), "editor-playback-late",
                      "the text of the failed play stays", "stuck")
    parts = playback_late_parts(2, [], {})
    return pass_fail("editor-playback-late-fault-norun", all((not g) and "no usable run" in ev for g, ev in parts.values()),
                     "a run with no page report fails with 'no usable run'") and ok


def playback_tracks_parts(rc: int, rows: list, rec: dict) -> dict:
    """M1b8 (#555), the owner's defect: ONE DAW track holds two sources (clip 1 at song 2.0 s, clip 2 of another file 0.5 s later), a second DAW
    track holds the third source. The page shows song time: a click inside clip 2 goes to the host as that song time (no mapping through one
    track), the follow shows it again, the playhead does not move when the selection changes from one row to the other, and when the
    host plays from there the playhead is the host position (no jump at play start), with the selection on either row."""
    name = "editor-playback-tracks"
    reps = page_reports(rows, "playback")
    rep = reps[0] if reps else {}
    if not rep or rep.get("visibility") != "visible" or len(rep.get("steps", [])) < 6 or not rec or not rep.get("calls"):
        return {name: (False, f"no usable run: rc={rc}, page report {bool(rep)}, page {rep.get('visibility') if rep else None}, host record {bool(rec)}")}
    x = rep["in_clip2"]
    want_samples = pb_samples_of(x)
    reqs = rec.get("requests", [])
    steps, calls, seen = rep["steps"], rep["calls"], rep.get("seen", [])
    kinds = [r["kind"] for r in reqs]
    loc = next((r for r in reqs if r["kind"] == "locate"), None)
    play = next((r for r in reqs if r["kind"] == "play"), None)
    call = calls[0]
    got = loc["samples"] if loc else None
    arrive = next((m for m in seen if m[0] >= call["at"] and abs(m[1] - x) <= 0.002), None)
    ms = arrive[0] - call["at"] if arrive else None
    away = [m[1] for m in seen if call["at"] <= m[0] < steps[2]["at"] and abs(m[1] - x) > 0.002 and (arrive is None or m[0] >= arrive[0] - 1)]
    n_rows = len(rep.get("rows", []))
    two_clips = n_rows == 2 and len(rep["rows"][0]["clips"]) == 2 and len(rep["rows"][0]["srcs"]) == 2
    # the frames while the host plays: the page position is the host position, with the selection on the first row and then on the second
    jump = None
    if play:
        w0, pos0 = play["wall_ms"], play["landed_song"]
        t_play, t_stop = steps[3]["at"], steps[5]["at"]
        late = [s for s in rep["samples"] if t_play + 400 <= s[0] <= t_stop and s[2] == 1 and s[1] is not None]
        errs = [abs(s[1] - (pos0 + (s[0] - w0) / 1000.0)) for s in late]
        jump = max(errs) if errs else None
    stop_pos = steps[5]["pos_after"]
    stop_host = next((r["landed_song"] for r in reqs if r["kind"] == "stop"), None)
    good = (two_clips and got == want_samples and ms is not None and ms <= PB_LOOP_MS and not away and abs(steps[1]["pos_after"] - x) <= 0.002
            and abs(steps[2]["pos_after"] - x) <= 0.002 and jump is not None and jump <= 0.05
            and stop_host is not None and abs(stop_pos - stop_host) <= 0.003)
    return {name: (good,
                   f"one DAW track with two sources and a second DAW track ({n_rows} rows, {len(rep['rows'][0]['clips']) if n_rows else 0} clips in the first): a click in clip 2 at song {x:.3f} s: "
                   f"the host got {got} samples (want {want_samples}), the follow showed it again {ms:.0f} ms after the call (limit {PB_LOOP_MS:.0f} ms), the selection changed to the first row "
                   f"and the playhead stayed at {steps[2]['pos_after']} s; play from there: the page position was the host position within {jump * 1000:.1f} ms over the whole play "
                   f"(limit 50 ms), with the selection on the other row too; after the stop the page shows {stop_pos} s, the host {stop_host} s" if good else
                   f"the page does not show song time for the DAW track: rows {n_rows} (want 2, two clips and two sources in the first: {two_clips}), host got {got} samples (want {want_samples}), "
                   f"the follow came back after {ms} ms, positions that jumped away from the click {away[:3]}, page after the click {steps[1]['pos_after']} s (want {x}), "
                   f"after selecting the first row {steps[2]['pos_after']} s (want {x}), the playhead is off the host position by up to {jump} s while the host plays (limit 0.05), "
                   f"after the stop the page shows {stop_pos} s and the host {stop_host} s"
                   + (": the playhead jumped away from the click or from the host position" if away or (jump is not None and jump > 0.05) else ""))}


def stage_editor_playback_tracks() -> bool:
    """M1b8: the page shows song time (a DAW track with two sources; the selection does not matter)."""
    rc, _summary, rows, rec = playback_run("editor-playback-tracks", CLAP, "tracks")
    return print_parts("", playback_tracks_parts(rc, rows, rec))


def stage_editor_playback_tracks_fault() -> bool:
    rc, _summary, rows, rec = playback_run("editor-fault-playback-tracks", CLAP, "tracks", {"DS_ARA_SONG_FAULT": "selected-track"})
    ok = fault_reason("editor-playback-tracks-fault-selected-track", playback_tracks_parts(rc, rows, rec), "editor-playback-tracks",
                      "the page maps the host position through the first clip of the selected row (the way of #554)", "jumped away from the click or from the host position")
    parts = playback_tracks_parts(2, [], {})
    return pass_fail("editor-playback-tracks-fault-norun", all((not g) and "no usable run" in ev for g, ev in parts.values()),
                     "a run with no page report fails with 'no usable run'") and ok


def playback_map_run(label: str, env: dict | None = None) -> tuple[int, str]:
    e = {"ARA_API_DIR": str(DEPS / "ARA_SDK" / "ARA_API"), "LIBCLANG_PATH": f"{CLT}/usr/lib"}
    e.update(env or {})
    return run(["cargo", "test", "--locked", "--features", "test-switches", "--lib", "--", *PLAYBACK_MAP_TESTS, PLAYBACK_QUEUE_TEST, "--test-threads=1"], env=e, log=f"{label}.log", cwd=HERE)


# M1b8 (#555): the page shows song time, so the locate goes to the host as the time of the page. The tests of the mapping through
# the regions (page_time, song_time, regions_for) are deleted with those functions; the stage editor-playback-tracks (the acceptance
# case: one DAW track with two sources, the selection on either row) and the unit tests below cover the same invariant.
PLAYBACK_MAP_TESTS = [
    "transport::tests::a_loop_of_the_host_is_a_jump_each_time_and_the_page_time_is_the_song_position",
    "playback::tests::a_locate_goes_to_the_host_as_the_song_position_the_page_shows_song_time",
    "playback::tests::a_locate_with_no_time_sends_nothing_and_says_why",
    "playback::tests::a_host_without_a_call_gets_no_call_and_the_text_names_the_host",
    "playback::tests::a_play_moves_a_stopped_host_first_when_the_page_stands_elsewhere",
    "playback::tests::the_queue_holds_a_few_requests_and_the_drain_job_starts_once",
    "playback::tests::a_request_queued_before_the_editor_closed_makes_no_call",
    "playback::tests::a_time_is_bounded_before_it_reaches_the_host",
    "editor::tests::requests_to_the_host_do_not_use_the_budget_of_the_app_commands",
]
PLAYBACK_QUEUE_TEST = "playback::tests::requests_run_in_the_order_of_arrival_and_the_newest_locate_wins"


def stage_playback_map() -> bool:
    """M1b7 gate line 2, since M1b8 (#555) song time: the locate goes to the host as the song position the page shows, with no
    mapping through a region or a track. The unit tests: the locate, the plan of each request (the text, the move before a play),
    the page time of the pacer."""
    rc, text = playback_map_run("playback-map")
    m = re.search(r"test result: (\w+)\. (\d+) passed; (\d+) failed", text)
    ran = all(f"{n} ... ok" in text for n in PLAYBACK_MAP_TESTS + [PLAYBACK_QUEUE_TEST])
    return pass_fail("playback-map", rc == 0 and bool(m) and ran,
                     f"{m.group(2) if m else '?'} unit tests passed: the locate is the song position, no time sends nothing, the plan of play, stop and locate "
                     f"(out/playback-map.log)")


def stage_playback_map_fault() -> bool:
    rc2, text2 = playback_map_run("fault-playback-map-queue", {"DS_ARA_PLAYBACK_FAULT": "no-supersede"})
    named2 = "the newest locate wins" in text2 and PLAYBACK_QUEUE_TEST.split("::")[-1] in text2
    return pass_fail("playback-map-fault-queue", rc2 != 0 and named2,
                     f"the fault run (no locate of the queue is dropped for a newer one) makes the queue test fail with 'the newest locate wins': rc={rc2}, named {named2}")


# ---------------------------------------------------------------- M1b11: the menu and the lanes of the page (#559)
#
# Three runs of the test host editor read what the page DREW (the pixels of the lane canvases), not what its code says:
#   hit       the host sends the double click of a real pointer into the lane of a track of the role "other" and of a drum
#             role; the page reports the pixels of both lanes. The finding of the owner: the lane was one colour (magenta,
#             yellow) with the hits as dark lines, because a track of the plugin has no colour and "undefined0d" is no colour.
#   geometry  the rows that the lines of the hits span, after a change of the zoom, of the track height and of the window
#             (across the 520 px of the low window, which the host resizes)
#   menu      the menu button of the page in host mode, used with the pointer and the keyboard, in a normal and a low window


def lane_runs(label: str, mode: str, env: dict | None = None) -> tuple[int, list, dict]:
    # M1b8: the page shows song time; these runs read the hits of every track at the same times, so every clip lies at song 0 s (1.5 s of its audio)
    rc, _summary, rows = run_phase(label, mode, CLAP, env, ["--regions-all", "0,1.5,0"] if mode in ("hit", "geometry") else None)
    reps = page_reports(rows, mode)
    return rc, rows, (reps[0] if reps else {})


def no_run(rc: int, rep: dict, names: list) -> dict | None:
    if not rep or rep.get("visibility") != "visible":
        why = (f"the page is {rep.get('visibility')}: the screen of this host is locked or its window occluded" if rep
               else f"no usable run: rc={rc}, no page report")
        return {n: (False, why) for n in names}
    return None


HIT_PARTS = ["editor-hit-pointer", "editor-hit-selected", "editor-hit-lane", "editor-hit-colour"]


def hit_parts(rc: int, rep: dict) -> dict:
    bad = no_run(rc, rep, HIT_PARTS)
    if bad:
        return bad
    tr = rep.get("tracks", [])
    if len(tr) != 2:
        return {n: (False, f"no usable run: {len(tr)} track report(s), want 2") for n in HIT_PARTS}
    roles = [t.get("role") for t in tr]
    pointer = all(t.get("pointer_added") for t in tr) and roles[0] == "other"
    sel = all(t.get("sel") and t["sel"].get("track") == t["name"] and not t.get("ln_sel") and t.get("added_flags", 0) >= 1 for t in tr)
    bad_lane = []
    for t in tr:
        sp, wv = t["after"].get("spike", {}), t["after"].get("wave", {})
        if sp.get("opaque", 1) > 0.5 or wv.get("opaque", 1) > 0.5:
            bad_lane.append(f"{t['name']} ({t['role']}): the spike lane is one opaque colour: {round(sp.get('opaque', 0) * 100)} percent opaque, "
                            f"one colour on {round(sp.get('share', 0) * 100)} percent (bucket {sp.get('top')}); the wave lane {round(wv.get('opaque', 0) * 100)} percent opaque")
    colour = all(isinstance(t.get("color"), str) and re.fullmatch(r"#[0-9a-fA-F]{6}", t["color"]) for t in tr)
    return {
        "editor-hit-pointer": (pointer, f"a double click of the pointer (NSEvents of the host) added a hit on a track of the role {roles[0]!r} (hits {tr[0]['n0']} -> {tr[0]['n1']}) "
                                        f"and on a track of the role {roles[1]!r} ({tr[1]['n0']} -> {tr[1]['n1']})" if pointer else
                               f"the pointer click did not add a hit, or the first track is not of the role other: roles {roles}, added {[t.get('pointer_added') for t in tr]}"),
        "editor-hit-selected": (sel, f"the new hit is the selected hit of its track and no range is selected: {[t.get('sel') for t in tr]}" if sel else
                                f"selection after the new hit: {[(t.get('sel'), t.get('ln_sel'), t.get('added_flags')) for t in tr]}"),
        "editor-hit-lane": (not bad_lane, "after the new hit the spike lanes are a translucent tint (opaque " + ", ".join(f"{round(t['after']['spike']['opaque'] * 100)}" for t in tr)
                            + " percent) and the wave lanes carry only the lines" if not bad_lane else "; ".join(bad_lane)),
        "editor-hit-colour": (colour, f"every track has a colour of 6 hex digits: {[t.get('color') for t in tr]}" if colour else
                              f"a track has no colour: {[t.get('color') for t in tr]}"),
    }


def stage_editor_hit() -> bool:
    if _reference() is None or not HOST.exists():
        return pass_fail("editor-hit", False, "no reference or host")
    rc, _rows, rep = lane_runs("editor-hit", "hit")
    return print_parts("", hit_parts(rc, rep))


def stage_editor_hit_fault() -> bool:
    rc, _rows, rep = lane_runs("editor-fault-hit", "hit", {"DS_ARA_EDITOR_FAULT": "no-color"})
    ok = fault_reason("editor-hit-fault-no-color", hit_parts(rc, rep), "editor-hit-lane", "the tracks have no colour and the page does not repair it", "one opaque colour")
    ok = fault_part("editor-hit-fault-no-color-colour", hit_parts(rc, rep), "editor-hit-colour", "the same run") and ok
    parts = hit_parts(2, {})
    return pass_fail("editor-hit-fault-norun", all((not g) and "no usable run" in ev for g, ev in parts.values()),
                     "a run with no page report fails with 'no usable run'") and ok


GEO_PARTS = ["editor-geometry-span", "editor-geometry-store", "editor-geometry-window"]


def geometry_parts(rc: int, rep: dict) -> dict:
    bad = no_run(rc, rep, GEO_PARTS)
    if bad:
        return bad
    steps = rep.get("steps", [])
    if len(steps) < 13:
        return {n: (False, f"no usable run: {len(steps)} steps, want 13") for n in GEO_PARTS}
    short, store, checked = [], [], 0
    for st in steps:
        n_here = 0
        for name, lanes in st["lanes"].items():
            for kind, ln in lanes.items():
                if ln.get("tiny"):
                    continue
                if abs(ln["storeH"] - ln["cssH"]) > 1.0:
                    store.append(f"{st['name']}/{name}/{kind}: store {ln['storeH']} px, box {ln['cssH']} px")
                if ln.get("maxY") is None:   # no line in this view
                    continue
                n_here += 1
                if ln["minY"] > 1.5 or ln["maxY"] < ln["cssH"] - 2.5:
                    short.append(f"{st['name']}/{name}/{kind}: the lines span {ln['minY']} to {ln['maxY']} of {ln['cssH']} px ({round((ln['maxY'] - ln['minY'] + 1) / ln['cssH'] * 100)} percent)")
        checked += n_here
        if n_here < 2:
            short.append(f"{st['name']}: only {n_here} lane(s) with a line in the view, the step proves nothing")
    lo, hi = next((s for s in steps if s["name"] == "window-low"), {}), next((s for s in steps if s["name"] == "window-high"), {})
    win_ok = bool(rep.get("went_low")) and bool(rep.get("went_high")) and lo.get("win", [0, 9999])[1] < 520 and lo.get("low") is True and hi.get("win", [0, 0])[1] >= 520 and hi.get("low") is False
    return {
        "editor-geometry-span": (not short, f"after {len(steps)} steps (zoom, track height, a row with its own height, the window below and above 520 px) the lines of the hits span the whole lane on {checked} lanes"
                                 if not short else "the lines span only part of the lane: " + "; ".join(short[:4])),
        "editor-geometry-store": (not store, "the store of every lane canvas is as high as its box" if not store else "the canvas store is not the lane size: " + "; ".join(store[:3])),
        "editor-geometry-window": (win_ok, f"the host made the window {lo.get('win')} (low: {lo.get('low')}) and then {hi.get('win')} (low: {hi.get('low')}), device pixel ratio {lo.get('dpr')}" if win_ok else
                                   f"the window did not cross the 520 px limit both ways: low {rep.get('went_low')} {lo.get('win')} {lo.get('low')}, high {rep.get('went_high')} {hi.get('win')} {hi.get('low')}"),
    }


def stage_editor_geometry() -> bool:
    if _reference() is None or not HOST.exists():
        return pass_fail("editor-geometry", False, "no reference or host")
    rc, _rows, rep = lane_runs("editor-geometry", "geometry")
    return print_parts("", geometry_parts(rc, rep))


def stage_editor_geometry_fault() -> bool:
    rc, _rows, rep = lane_runs("editor-fault-geometry", "geometry", {"DS_ARA_EDITOR_FAULT": "stale-geometry"})
    ok = fault_reason("editor-geometry-fault-stale", geometry_parts(rc, rep), "editor-geometry-span", "the page measures the rows after it drew them and does not draw again (the page before #559)", "the lines span only part")
    parts = geometry_parts(2, {})
    return pass_fail("editor-geometry-fault-norun", all((not g) and "no usable run" in ev for g, ev in parts.values()),
                     "a run with no page report fails with 'no usable run'") and ok


MENU_PARTS = ["editor-menu-button", "editor-menu-pointer", "editor-menu-keyboard", "editor-menu-low"]


def menu_parts(rc: int, rep: dict) -> dict:
    bad = no_run(rc, rep, MENU_PARTS)
    if bad:
        return bad
    if not rep.get("button"):
        why = "no menu button in the page in host mode"
        return {"editor-menu-button": (False, why), **{n: (False, why) for n in MENU_PARTS[1:]}}
    names = [g[0] for g in rep.get("groups", [])]
    holes = {k: v for k, v in (rep.get("audit") or {}).items() if v}
    # #560: File is a menu in host mode again (it holds Export); the list of groups is exact
    button = (rep.get("host_mode") is True and rep.get("panel") is True
              and names == ["Drum Spike", "File", "Edit", "View", "Transport", "Hits", "Track", "Help"] and not holes)
    st = rep.get("settings", {})
    cats = st.get("cats", [])
    pointer = (rep.get("pick_height") == "ok" and rep.get("track_height") == "large" and rep.get("pick_settings") == "ok" and st.get("visible") is True
               and "view" in cats and "keyboard" in cats and not {"audio", "audition", "pitches", "agent"} & set(cats))
    kb = rep.get("keyboard", {})
    keyboard = kb.get("opened") is True and kb.get("focus") == "Drum Spike" and kb.get("moved") == "File" and kb.get("closed") is True and kb.get("focus_back") == "hostMenuBtn"
    lo = rep.get("low", {})
    low = (lo.get("innerH", 9999) < 520 and lo.get("top", -1) >= 0 and lo.get("bottom", 9999) <= lo.get("innerH", 0) and lo.get("right", 9999) <= lo.get("innerW", 0)
           and lo.get("scrolls") is True and lo.get("last_visible") is True and lo.get("closed") is True)
    return {
        "editor-menu-button": (button, f"the page has a menu button; groups {names}; every command of the table is in the panel or in the two lists (audit {holes or 'clean'})" if button else
                               f"menu button {rep.get('button')}, panel {rep.get('panel')}, host mode {rep.get('host_mode')}, groups {names}, holes {holes}"),
        "editor-menu-pointer": (pointer, f"a click opened the menu, View > Track Height > Large set the height ({rep.get('track_height')}), Settings opened with the categories {cats}" if pointer else
                                f"pointer use failed: height {rep.get('pick_height')!r}/{rep.get('track_height')}, settings {rep.get('pick_settings')!r} visible {st.get('visible')} categories {cats}"),
        "editor-menu-keyboard": (keyboard, f"Enter opens the menu with the focus on {kb.get('focus')!r}, Down moves to {kb.get('moved')!r}, Escape closes it and the focus is back on {kb.get('focus_back')!r}" if keyboard else
                                 f"keyboard use failed: {kb}"),
        "editor-menu-low": (low, f"in the low window ({lo.get('innerH')} px high) the panel ends at {lo.get('bottom')} px, scrolls, and its last item is reachable" if low else
                            f"the low window breaks the menu: {lo}"),
    }


def stage_editor_menu() -> bool:
    if _reference() is None or not HOST.exists():
        return pass_fail("editor-menu", False, "no reference or host")
    rc, _rows, rep = lane_runs("editor-menu", "menu")
    return print_parts("", menu_parts(rc, rep))


def stage_editor_menu_fault() -> bool:
    rc, _rows, rep = lane_runs("editor-fault-menu", "menu", {"DS_ARA_EDITOR_FAULT": "no-menu"})
    ok = fault_reason("editor-menu-fault-no-menu", menu_parts(rc, rep), "editor-menu-button", "the page has no menu button", "no menu button")
    parts = menu_parts(2, {})
    return pass_fail("editor-menu-fault-norun", all((not g) and "no usable run" in ev for g, ev in parts.values()),
                     "a run with no page report fails with 'no usable run'") and ok


# ---------------------------------------------------------------- M1b13: the page follows the selection of the DAW (#562)
#
# The test host sends what a DAW sends through the ARA editor view: a selection (region sequences, playback regions, a time range) and a list
# of hidden region sequences. The page runs in the real web view; the driver reads what the page SHOWS after each step (the selected track, the
# marks, the band of the range, the view, the rows), and the pointer, the key and the menu item are the page's own. The steps are in SEL_STEPS;
# a step is `[pre action of the page@]host step`. The host has four sources (kick 3.0 s at song 0, snare 2.0 s at 3.0, tom 3.0 s at 5.0, hat
# 1.6 s at 8.0), each on its own region sequence 0 to 3, and a fifth sequence "Empty Track" that has no clip of the plugin.

SEL_ARGS = ["--seq-names", "Drum A", "Drum B", "Drum C", "Drum D", "Empty Track"]
SEL_PRE = "seq:2;hide:3"   # sent before the editor opens: the tom track is selected and the hat track is hidden
SEL_STEPS = [
    "hide:",                       # -  the DAW shows every track (the page had the hat hidden from the state at its opening)
    "seq:1",                       # 0  the DAW selects the snare track
    "seq:2,0",                     # 1  two tracks: the first in the order of the DAW (the kick) is selected, the tom is marked
    "reg:3",                       # 2  a playback region of the hat selects the hat track
    "zoom@seq:1+range:3.3,0.6",    # 3  the snare and a range of song 3.3 to 3.9; the page shows song 3.0 to 4.2 s (the page shows song time since M1b8)
    "range:6.0,1.0",               # 4  a range alone, in the song 6 to 7: no clip of the snare holds it, it is drawn all the same (song time)
    "seq:3+range:9.3,0.2",         # 5  the hat, song 9.3 to 9.5: out of the view
    "seq:4",                       # 6  a DAW track with no source of the plugin: nothing changes
    "empty",                       # 7  an empty selection: the range goes
    "hide:0,1",                    # 8  the DAW hides the kick and the snare track
    "hide:",                       # 9  and shows them again
    "pointer@seq:0",               # 10 the pointer is down in the page when the DAW selects the kick: it waits for the end of the action
    "user@seq:1",                  # 11 the user selects the tom while the pointer is down: the DAW's snare is dropped
    "key@seq:3",                   # 12 a key is down when the DAW selects the hat: it waits
    "off@seq:0+range:0.5,1.0",     # 13 the user switches the follow off in the menu, then the DAW selects the kick and a range: nothing follows
    "on@noop",                     # 14 the user switches it on again: the page takes the last selection of the host
    "flood:200",                   # 15 a drag in the ruler: 200 ranges in a tight loop; the page gets few messages and shows the last range
]
SEL_PARTS = ["editor-selection-open", "editor-selection-track", "editor-selection-several", "editor-selection-range", "editor-selection-range-view", "editor-selection-outside",
             "editor-selection-empty", "editor-selection-unknown", "editor-selection-hidden", "editor-selection-guard", "editor-selection-user", "editor-selection-switch",
             "editor-selection-flood", "editor-selection-row", "editor-selection-no-round"]


def selection_run(label: str, env: dict | None = None) -> dict:
    steps = ";".join(SEL_STEPS)
    rc, _summary, rows = run_phase(label, "selection", CLAP, env, [*SEL_ARGS, "--sel-pre", SEL_PRE, "--sel-steps", steps, "--arg", steps])
    reps = page_reports(rows, "selection")
    ready = page_reports(rows, "sel-ready")
    return {"rc": rc, "rows": rows, "rep": reps[0] if reps else {}, "ready": ready[0] if ready else {}}


def close(a, b, tol=0.02) -> bool:
    return a is not None and b is not None and abs(a - b) <= tol


def selection_parts(run: dict) -> dict:
    rep, rows = run["rep"], run["rows"]
    bad = no_run(run["rc"], rep, SEL_PARTS)
    if bad:
        return bad
    steps = rep.get("steps") or []
    if len(steps) != len(SEL_STEPS):
        return {n: (False, f"no usable run: {len(steps)} step(s) reported, want {len(SEL_STEPS)}") for n in SEL_PARTS}
    steps = steps[1:]   # the first step only shows every track again; the numbers in the comments of SEL_STEPS count from the second
    keys = track_names(FILES)
    K, S_, T_, H = keys
    st = lambda i, w="snap2": steps[i][w]
    band_ok = lambda b: b["shown"] and close(b["left"], b["want_left"], 1.0) and close(b["width"], b["want_width"], 1.5) and b["top"] <= 1 and b["bottom"] >= b["h"] - 1
    parts: dict = {}
    first = run.get("ready", {}).get("initial") or {}
    parts["editor-selection-open"] = (
        first.get("sel") == T_ and first.get("hidden") == [H] and first.get("note") == "1 track hidden in the DAW" and first.get("marked") == [],
        f"the DAW selected the tom track and hid the hat track before the editor opened: the page opens with {first.get('sel')!r} selected (want {T_!r}), hidden {first.get('hidden')} "
        f"(want {[H]}) and the text {first.get('note')!r}")
    # the page can only scroll to a track that is out of the lanes area: step 2 needs the hat row out of view before it
    pre_out = steps[2]["before"]["rows_in_view"].get(H) is False
    parts["editor-selection-track"] = (
        st(0)["sel"] == S_ and st(0)["marked"] == [] and st(2)["sel"] == H and st(2, "before").get("sel") != H and st(2)["sel_in_view"] is True and st(2)["scroll_top"] > steps[2]["before"]["scroll_top"]
        and st(2)["range"] is None and pre_out,
        f"the DAW selected the snare track and the page selected {st(0)['sel']!r} (want {S_!r}); a playback region of the hat selected {st(2)['sel']!r} (want {H!r}), "
        f"the row is inside the lanes area ({st(2)['sel_in_view']}) after the page scrolled it from {steps[2]['before']['scroll_top']} to {st(2)['scroll_top']} px")
    parts["editor-selection-several"] = (
        st(1)["sel"] == K and st(1)["marked"] == [T_] and st(2)["marked"] == [],
        f"two tracks: the first in the order of the DAW is selected ({st(1)['sel']!r}, want {K!r}), the other is marked ({st(1)['marked']}, want {[T_]}); "
        f"a later selection of one track takes the marks away ({st(2)['marked']})")
    r3 = st(3)["range"] or {}
    parts["editor-selection-range"] = (
        st(3)["sel"] == S_ and close(r3.get("t0"), 3.3) and close(r3.get("t1"), 3.9) and band_ok(st(3)["band"]) and st(3)["own_range"] is False and st(3)["hit"] is False,
        f"the DAW range song 3.3 to 3.9 on the snare track is drawn as song {r3.get('t0')} to {r3.get('t1')} (want 3.3 to 3.9); the band: {st(3)['band']}; "
        f"the page's own range selection is {st(3)['own_range']} and no hit is selected ({st(3)['hit']})")
    v5 = st(5, "snap1")["view"]
    parts["editor-selection-range-view"] = (
        steps[5]["before"]["view"][1] < 9.3 and v5[0] <= 9.3 and v5[1] >= 9.5 and st(3)["view"] == steps[3]["before"]["view"] and band_ok(st(5)["band"]),
        f"the view was {steps[5]['before']['view']} and the range 9.3 to 9.5 is out of it: the view is {v5} after the step; a range inside the view (step 3) leaves it "
        f"({steps[3]['before']['view']} then {st(3)['view']})")
    r4 = st(4)["range"] or {}
    parts["editor-selection-outside"] = (
        close(r4.get("t0"), 6.0) and close(r4.get("t1"), 7.0) and st(4)["band"]["shown"] is True and band_ok(st(4)["band"]) and st(4)["sel"] == S_,
        f"a range in no clip of the snare track is drawn all the same, in song time (range {st(4)['range']}, want 6.0 to 7.0, shown {st(4)['band']['shown']}) and keeps the selected track {st(4)['sel']!r}")
    parts["editor-selection-empty"] = (
        st(7)["range"] is None and st(7)["band"]["shown"] is False and st(7)["sel"] == H and st(5)["range"] is not None,
        f"an empty selection removes the range ({st(7)['range']}, band shown {st(7)['band']['shown']}) and keeps the selected track {st(7)['sel']!r}")
    same = ["sel", "marked", "range", "view", "hidden"]
    parts["editor-selection-unknown"] = (
        steps[6]["got_event"] is True and all(st(6, "snap1")[k] == steps[6]["before"][k] for k in same) and st(6, "snap1")["band"]["shown"] == steps[6]["before"]["band"]["shown"] is True,
        f"a DAW track with no source of the plugin: the page got the event ({steps[6]['got_event']}) and changed nothing: "
        f"{ {k: (steps[6]['before'][k], st(6, 'snap1')[k]) for k in same if st(6, 'snap1')[k] != steps[6]['before'][k]} or 'selection, marks, range, view and hidden rows are the same' }; "
        f"the plugin said none={(steps[6]['snap1'].get('last_event') or {}).get('none')} (want True: the selection names a DAW track that no track of the plugin has)")
    parts["editor-selection-hidden"] = (
        st(8)["hidden"] == [K, S_] and st(8)["rows_shown"] == [T_, H] and st(8)["note"] == "2 tracks hidden in the DAW"
        and st(9)["hidden"] == [] and st(9)["note"] == "" and st(9)["rows_shown"] == keys,
        f"hidden by the DAW: {st(8)['hidden']}, rows shown {st(8)['rows_shown']}, text {st(8)['note']!r}; after the DAW shows them again: hidden {st(9)['hidden']}, rows shown {st(9)['rows_shown']}, text {st(9)['note']!r}")
    parts["editor-selection-guard"] = (
        st(10, "snap1")["sel"] == H and st(10, "snap1")["pending"] is True and st(10)["sel"] == K
        and st(12, "snap1")["sel"] == T_ and st(12, "snap1")["pending"] is True and st(12)["sel"] == H,
        f"pointer down: the page keeps {st(10, 'snap1')['sel']!r} (pending {st(10, 'snap1')['pending']}), after the action it selects {st(10)['sel']!r} (want {H!r} then {K!r}); "
        f"a key down: {st(12, 'snap1')['sel']!r} then {st(12)['sel']!r} (want {T_!r} then {H!r})")
    parts["editor-selection-user"] = (
        st(11, "snap1")["sel"] == T_ and st(11)["sel"] == T_,
        f"the user selected {T_!r} while the pointer was down and the DAW selected the snare track: the page shows {st(11, 'snap1')['sel']!r} during and {st(11)['sel']!r} after the action (want {T_!r} both)")
    fl = steps[15]
    n_last = (fl["snap2"].get("last_event") or {}).get("n", 0)
    n_rows = len([r for r in rows if r.get("event") == "view_applied"])
    sent = fl["snap2"]["events"] - fl["before"]["events"]
    r15 = fl["snap2"].get("range") or {}
    parts["editor-selection-flood"] = (
        n_last >= 150 and sent <= 12 and n_rows <= 40 and close(r15.get("t0"), 3.995) and close(r15.get("t1"), 4.495),
        f"200 ranges in a tight loop: the page got {sent} message(s) (want at most 12), the plugin wrote {n_rows} view_applied row(s) in the whole run (want at most 40), "
        f"the numbers went up to {n_last}, and the page shows the last range {r15.get('t0')} to {r15.get('t1')} (want 3.995 to 4.495)")
    parts["editor-selection-switch"] = (
        steps[13]["menu_was"] == "true" and st(13)["follow"] is False and st(13)["sel"] == H and st(13)["marked"] == [] and st(13)["band"]["shown"] is False and st(13)["hidden"] == []
        and steps[14]["menu_was"] == "false" and st(14)["follow"] is True and st(14)["sel"] == K and band_ok(st(14)["band"]) and close((st(14)["range"] or {}).get("t0"), 0.5),
        f"switched off in the menu: nothing follows (selected {st(13)['sel']!r}, band shown {st(13)['band']['shown']}); switched on: the page takes the last selection "
        f"(selected {st(14)['sel']!r}, range {st(14)['range']}, band {st(14)['band']})")
    sel_rows = [r for r in rows if r.get("event") == "view_selection"]
    mapped = [r for r in sel_rows if r.get("range_start_s") == 3.3]
    parts["editor-selection-row"] = (
        bool(mapped) and mapped[0].get("tracks") == [S_] and mapped[0].get("sequence_names") == ["Drum B"] and close(((mapped[0].get("ranges") or [{}])[0]).get("t0"), 3.3) and len(sel_rows) >= 8,
        f"{len(sel_rows)} view_selection row(s); the row of step 3 names the DAW track {mapped[0].get('sequence_names') if mapped else None}, the track {mapped[0].get('tracks') if mapped else None} "
        f"and the ranges {mapped[0].get('ranges') if mapped else None}")
    jobs, rounds = job_rows_of(rows), [r for r in rows if r.get("event") == "round_start"]
    parts["editor-selection-no-round"] = (
        len(jobs) == 1 and len(rounds) == 1,
        f"{len(jobs)} analysis job(s) and {len(rounds)} round(s) in the whole run: a selection or a hide list of the DAW starts none")
    return parts


def stage_editor_selection() -> bool:
    if _reference() is None or not HOST.exists():
        return pass_fail("editor-selection", False, "no reference or host")
    return print_parts("", selection_parts(selection_run("editor-selection")))


def stage_editor_selection_fault() -> bool:
    ok = True
    K, S_, T_, _H = track_names(FILES)
    sel = lambda v: {"DS_ARA_SELECTION_FAULT": v}
    page = lambda v: {"DS_ARA_EDITOR_FAULT": v}
    # (run name, env, [(part, what the fault breaks, the text that the failing evidence must hold)])
    for label, env, checks in (
        ("no-track", sel("no-track"), [("editor-selection-track", "the plugin never gives the page the track", f"selected {K!r} (want {S_!r})")]),
        ("no-pull", sel("no-pull"), [("editor-selection-open", "the page that opens later is told nothing of the selection that came before", f"opens with {K!r} selected (want {T_!r})")]),
        ("no-others", sel("no-others"), [("editor-selection-several", "only the first track is given: the others are not marked", "the other is marked ([]")]),
        ("unknown-clears", sel("unknown-clears"), [("editor-selection-unknown", "a selection of a track with no source is not marked as none, so the page treats it as an empty selection and clears the range", "none=False (want True")]),
        ("keep-range", sel("keep-range"), [("editor-selection-empty", "an empty selection is not passed on: the range stays", "band shown True")]),
        ("no-row", sel("no-row"), [("editor-selection-row", "the row of a selection has the counts only", "names the DAW track None")]),
        ("round", sel("round"), [("editor-selection-no-round", "a selection of the DAW starts an analysis round", "round(s) in the whole run")]),
        ("no-throttle", sel("no-throttle"), [("editor-selection-flood", "every change is sent to the page and written as a row at once", "message(s) (want at most 12)")]),
        ("user-loses", page("sel-user-loses"), [("editor-selection-user", "the page cannot tell that the user changed the selection during the action", "after the action (want")]),
        ("no-hidden", sel("no-hidden"), [("editor-selection-hidden", "the hidden tracks are ignored", "hidden by the DAW: []")]),
        ("no-guard", page("sel-no-guard"), [("editor-selection-guard", "the page overwrites the selection while a pointer or key action runs", "(pending False)")]),
        ("own-range", page("sel-own-range"), [("editor-selection-range", "the range of the host is the page's own hit selection", "own range selection is True")]),
        ("no-scroll", page("sel-no-scroll"), [("editor-selection-range-view", "nothing scrolls the view to the range", "out of it: the view is [3, 4.2]"),
                                              ("editor-selection-track", "nothing scrolls the selected track into view", "lanes area (False)")]),
        ("no-switch", page("sel-no-switch"), [("editor-selection-switch", "the switch of the menu does not stop the follow", "band shown True")]),
        ("no-note", page("sel-no-note"), [("editor-selection-hidden", "the hidden rows have no text", "text ''")]),
    ):
        parts = selection_parts(selection_run(f"editor-fault-selection-{label}", env))
        for part, what, reason in checks:
            ok = fault_reason(f"editor-selection-fault-{label}-{part.rsplit('-', 1)[-1]}", parts, part, what, reason) and ok
    parts = selection_parts({"rc": 2, "rows": [], "rep": {}, "ready": {}})
    return pass_fail("editor-selection-fault-norun", all((not g) and "no usable run" in ev for g, ev in parts.values()),
                     "a run with no page report fails with 'no usable run'") and ok


# ---- M1b10 (#558): the tracks show the names of the DAW tracks

# The test host makes four region sequences (the DAW tracks): "Kick In" with a region name that must
# lose against it, one with no name whose region has one, and two with the same name. After the page
# reported, the host renames the first DAW track and moves the region of the second source to it.
# The names give no role (the role follows the DAW name, see the stage editor-roles): a rename or a move here
# changes no role, so it must start no round.
NAMES_ARGS = ["--seq-names", "Drum A", "-", "Drum B", "Drum B",
              "--region-names", "first take", "Snare Top Take", "-", "-",
              "--rename-seq", "0=Drum Big", "--move-region", "1=0"]
NAMES_BEFORE = ["Drum A", "Snare Top Take", "Drum B", "Drum B (2)"]
NAMES_AFTER = ["Drum Big", "Drum Big (2)", "Drum B", "Drum B (2)"]
# M1b8: the header of a row shows the name of the DAW track; two sources of one DAW track show one name (the plugin's display names keep the number
# for the file names of the export)
NAMES_AFTER_HEADER = ["Drum Big", "Drum Big", "Drum B", "Drum B (2)"]


def host_lines(label: str) -> list:
    """The JSON lines the test host printed in one run (out/host-<label>.log)."""
    path = OUT / f"host-{label}.log"
    out = []
    if path.exists():
        for ln in path.read_text().splitlines():
            if ln.startswith("{"):
                try:
                    out.append(json.loads(ln))
                except ValueError:
                    pass
    return out


def archive_core(path: Path) -> dict | None:
    """The stored archive without its document id (a fresh random id in every run)."""
    try:
        d = json.loads(path.read_text())
    except (OSError, ValueError):
        return None
    d.pop("doc_id", None)
    return d


def names_run(label: str, bundle: Path, named: bool, env: dict | None = None) -> dict:
    """One `ara-host editor` run of the mode `names`: the page reports the names it got, the host renames and
    moves (`named`; the baseline run makes no sequence at all), the page reports again after the reload."""
    archive = OUT / f"{label}.ara"
    if archive.exists():
        archive.unlink()
    args = (NAMES_ARGS if named else ["--arg", "base"]) + ["--archive", str(archive)]
    rc, summary, rows = run_phase(label, "names", bundle, env, args)
    lines = host_lines(label)
    notes = {}
    for phase in ("editor", "after-names"):
        notes[phase] = host_notes_of(lines, phase)
    return {"rc": rc, "rows": rows, "notes": notes, "archive": archive_core(archive), "named": named}


_NAMES_BASE: dict = {}


def names_base() -> dict:
    if not _NAMES_BASE:
        _NAMES_BASE.update(names_run("editor-names-base", CLAP, False))
    return _NAMES_BASE


def shown(rep: dict, keys: list) -> list:
    return [(rep.get("display") or {}).get(k) for k in keys]


def names_parts(run: dict, ref: bytes, base: dict) -> dict:
    """M1b10 gate lines 1 to 5: what the page gets as the names of the tracks, after the rename of the DAW track and
    the move of a region, and that nothing else changes."""
    rows = run["rows"]
    keys = track_names(FILES)
    first = (page_reports(rows, "names") or [{}])[0]
    after = (page_reports(rows, "names-after") or [{}])[0]
    jobs = job_rows_of(rows)
    rounds = [r for r in rows if r.get("event") == "round_start"]
    parts: dict = {}
    parts["editor-names-shown"] = (
        run["rc"] == 0 and first.get("order") == keys and shown(first, keys) == NAMES_BEFORE,
        f"the page got the display names {shown(first, keys)} for the tracks {first.get('order')} (want {NAMES_BEFORE}): the name of the "
        f"DAW track first, the name of the region when the track has none, the same name twice gets a number")
    parts["editor-names-rename"] = (
        after.get("reloaded") is True and shown(after, keys) == NAMES_AFTER,
        f"after the host renamed a DAW track and moved a region the page got {shown(after, keys)} (want {NAMES_AFTER}) once the plugin said "
        f"it reloaded ({after.get('reloaded')}, {after.get('waited_ms')} ms)")
    parts["editor-names-no-round"] = (
        len(jobs) == 1 and len(rounds) == 1,
        f"{len(jobs)} analysis job(s) and {len(rounds)} round(s) in the whole run: the rename and the move started none")
    same_keys = first.get("order") == keys and after.get("order") == keys
    try:
        events_same = (events_object_of(first["raw_events"], keys) == ref and events_object_of(after["raw_events"], keys) == ref)
    except (KeyError, ValueError):
        events_same = False
    notes_same = bool(run["notes"]["editor"]) and run["notes"]["editor"] == run["notes"]["after-names"] == base["notes"]["editor"]
    archive_same = run["archive"] is not None and run["archive"] == base["archive"]
    parts["editor-names-identity"] = (
        same_keys and events_same and notes_same and archive_same,
        f"the keys of the tracks are the same before and after ({same_keys}), the events the page reads are byte for byte the reference "
        f"before and after ({events_same}), the notes before and after are the notes of a run with no names ({notes_same}), the stored "
        f"archive (without its document id) is the archive of that run ({archive_same})")
    parts["editor-names-header"] = (
        first.get("dom_names") == NAMES_BEFORE and after.get("dom_names") == NAMES_AFTER_HEADER,
        f"the header of the page draws {first.get('dom_names')} and, after the rename and the move, {after.get('dom_names')} (want {NAMES_BEFORE} and {NAMES_AFTER_HEADER}: the two sources of one DAW track show one name)")
    rep_rows = [r for r in rows if r.get("event") == "track_names"]
    want_before = {k: v for k, v in zip(keys, NAMES_BEFORE) if k != v}
    want_after = {k: v for k, v in zip(keys, NAMES_AFTER) if k != v}
    seen = [r.get("names") for r in rep_rows]
    parts["editor-names-row"] = (
        want_before in seen and (seen[-1] if seen else None) == want_after,
        f"the report has {len(rep_rows)} track_names row(s); the first change is the names the page got first ({want_before in seen}) and the last is "
        f"the names after the rename ({(seen[-1] if seen else None) == want_after})")
    return parts


def stage_editor_names() -> bool:
    ref = _reference()
    if ref is None or not HOST.exists():
        return pass_fail("editor-names", False, "no reference or host")
    base = names_base()
    ok = True
    # M1b10 R5d: a run with no edit of the page: the archive holds the settings of every track from the END OF THE
    # ROUND (the refresh after a round, not only after a page edit)
    srcs = (base.get("archive") or {}).get("sources") or []
    ok = pass_fail("editor-names-round-settings", bool(srcs) and all(isinstance((x.get("settings") or {}).get("role"), str) for x in srcs),
                   f"{len(srcs)} sources in the archive of a run with no page edit, each with the settings of its track from the round") and ok
    for which, bundle, suffix in (("clap", CLAP, ""), ("vst3", EMBEDDED_CLAP, "-vst3")):
        run_ = names_run(f"editor-names-{which}", bundle, True)
        ok = print_parts(suffix, names_parts(run_, ref, base)) and ok
    return ok


def stage_editor_names_fault() -> bool:
    ref = _reference()
    if ref is None:
        return pass_fail("editor-names-fault", False, "no reference")
    base = names_base()
    ok = True
    for label, fault, part, what in (
        ("ignore", "ignore", "editor-names-shown", "the names of the host are ignored: the page gets the key as the name"),
        ("region-first", "region-first", "editor-names-shown", "the name of the region wins over the name of the DAW track"),
        ("no-number", "no-number", "editor-names-shown", "two DAW tracks with one name show one name"),
        ("no-rename", "no-rename", "editor-names-rename", "the rename of a DAW track is not seen"),
        ("no-reload", "no-reload", "editor-names-rename", "the page is not told about a rename and keeps the old name"),
        ("round", "round", "editor-names-no-round", "a rename and a move start an analysis round"),
        ("key", "key", "editor-names-identity", "the key of a track follows the name of the DAW track"),
    ):
        run_ = names_run(f"editor-names-fault-{label}", CLAP, True, {"DS_ARA_NAMES_FAULT": fault})
        parts = names_parts(run_, ref, base)
        ok = fault_part(f"editor-names-fault-{label}", parts, part, what) and ok
        if label in ("ignore", "no-reload"):
            ok = fault_part(f"editor-names-fault-{label}-header", parts, "editor-names-header", what + " (the drawn header text)") and ok
    return ok


# ---- M1b8 (#555): the page shows song time; one row is one DAW track

# Mode `song`. The test host makes two DAW tracks: sources 0 and 1 lie on the first (clips at song 0 s and 2 s), sources 2 and 3 on the second
# (4 s and 6 s); every clip is 1.5 s of its file. The page asks for the rows. Then the host renames the first DAW track, moves the region of
# source 3 to it, moves the clip of source 1 to song 20 s and trims the clip of source 2 to the slice 0.5 s to 1.5 s.
SONG_ARGS = ["--seq-names", "Kick", "Snare", "--seq-of", "0", "0", "1", "1", "--regions-all", "0,1.5,0", "--regions-step", "2",
             "--rename-seq", "0=Kick In", "--move-region", "3=0", "--shift-region", "1=20", "--trim-region", "2=0.5,1.0"]


SONG_SHIFT_ARGS = ["--seq-names", "Kick", "Snare", "--seq-of", "0", "0", "1", "1", "--regions-all", "0,1.5,0", "--regions-step", "2", "--shift-region", "1=20"]


def song_run(label: str, env: dict | None = None, args: list | None = None) -> dict:
    rc, _summary, rows = run_phase(label, "song", CLAP, env, args or SONG_ARGS)
    return {"rc": rc, "rows": rows}


def song_shift_parts(run: dict) -> dict:
    """The host moves one clip in the song and nothing else (no analysis round): the rows must show it, from the live regions."""
    name = "editor-song-shift"
    after = (page_reports(run["rows"], "song-after") or [None])[0]
    if after is None:
        return {name: (False, f"no usable run (rc={run['rc']}, {len(page_reports(run['rows'], 'song-after'))} report(s))")}
    keys = track_names(FILES)
    s1 = song_clip(after["rows"]["rows"][0], keys[1]) if after["rows"]["rows"] else None
    jobs = job_rows_of(run["rows"])
    return {name: (s1 == [(0.0, 1.5, 20.0, 1.0)],
                   f"after the host moved the clip of source 1 from song 2 s to 20 s (no new analysis round: {len(jobs)} job(s) in the run) the row shows it at {s1} (want [(0.0, 1.5, 20.0, 1.0)])"
                   + (" (start_play is old: the rows were made from the last analysis round, not from the live regions)" if s1 == [(0.0, 1.5, 2.0, 1.0)] else ""))}


def song_clip(row: dict, key: str) -> list:
    return [(round(c["a"], 6), round(c["b"], 6), round(c["s"], 6), round(c["k"], 6)) for c in row.get("clips", []) if c.get("src") == key]


def song_parts(run: dict) -> dict:
    rows = run["rows"]
    first = (page_reports(rows, "song") or [None])[0]
    after = (page_reports(rows, "song-after") or [None])[0]
    names = ("editor-song-rows", "editor-song-live", "editor-song-id", "editor-song-event", "editor-song-draw", "editor-song-cut")
    if first is None or after is None:
        why = f"no usable run (rc={run['rc']}, {len(page_reports(rows, 'song'))} first and {len(page_reports(rows, 'song-after'))} second report)"
        return {n: (False, why) for n in names}
    keys = track_names(FILES)
    a, b = first["rows"]["rows"], after["rows"]["rows"]
    parts: dict = {}
    one = len(a) == 2 and a[0]["srcs"] == keys[:2] and a[1]["srcs"] == keys[2:]
    clips0 = [(c["src"], c["s"], c["b"] - c["a"], c["k"]) for c in a[0]["clips"]] if a else []
    parts["editor-song-rows"] = (
        one and clips0 == [(keys[0], 0.0, 1.5, 1.0), (keys[1], 2.0, 1.5, 1.0)] and a[0]["name"] == "Kick",
        f"{len(a)} row(s) for two DAW tracks (want 2); the sources of the rows are {[r['srcs'] for r in a]} (want {[keys[:2], keys[2:]]}), the clips of "
        f"the first row are {clips0} (want both sources at their song positions in ONE row), its name is {a[0]['name'] if a else None!r}")
    ok_after = len(b) == 2 and b[0]["srcs"] == [keys[0], keys[1], keys[3]] and b[1]["srcs"] == [keys[2]]
    s1 = song_clip(b[0], keys[1]) if b else None
    s2 = song_clip(b[1], keys[2]) if len(b) > 1 else None
    old = " (start_play is old: the rows were made from the last analysis round, not from the live regions)" if s1 == [(0.0, 1.5, 2.0, 1.0)] else ""
    parts["editor-song-live"] = (
        ok_after and s1 == [(0.0, 1.5, 20.0, 1.0)] and s2 == [(0.5, 1.5, 4.5, 1.0)] and b[0]["name"] == "Kick In",
        f"after the host renamed, moved, shifted and trimmed: the sources of the rows are {[r['srcs'] for r in b]}, the clip of source 1 is {s1} "
        f"(want [(0.0, 1.5, 20.0, 1.0)]){old}, the trimmed clip of source 2 is {s2} (want [(0.5, 1.5, 4.5, 1.0)]), the name is {b[0]['name'] if b else None!r}")
    parts["editor-song-id"] = (
        len(a) == 2 and len(b) == 2 and a[0]["id"] == b[0]["id"] and a[1]["id"] == b[1]["id"],
        f"the ids of the rows before {[r['id'] for r in a]} and after {[r['id'] for r in b]} (the rename, the move of a region to the row and "
        f"the move in time keep the id, so the selection and the scroll of the page stay)")
    ev = after.get("last_event") or {}
    parts["editor-song-event"] = (
        after.get("events", 0) >= 1 and ev.get("rows") == after["rows"]["rows"] and ev.get("gen", 0) >= 1,
        f"the page got {after.get('events')} event(s) plugin-regions after the first answer; the last one carries the rows of the final answer "
        f"({ev.get('rows') == after['rows']['rows']}) and the change counter {ev.get('gen')}")
    # what the page DRAWS (the pixels of the hit lines in the lanes), written independently of the page's own mapping
    pa, pb = first.get("page") or {}, after.get("page") or {}
    ra = pa.get("rows") or {}
    row_a, row_b = (a[0]["id"] if a else None), (a[1]["id"] if len(a) > 1 else None)
    h0, h1 = (ra.get(row_a) or {}).get("hits", []), (ra.get(row_b) or {}).get("hits", [])
    out0 = (ra.get(row_a) or {}).get("outside", [])
    by_src = lambda hits, k: [h for h in hits if h["src"] == k]
    miss = [(h["src"][-12:], round(h["song"], 3)) for h in h0 + h1 if not h["drawn"]]
    stray = [(h["src"][-12:], round(h["song"], 3)) for h in out0 + (ra.get(row_b) or {}).get("outside", []) if h["drawn"]]
    second = by_src(h0, keys[1])
    parts["editor-song-draw"] = (
        bool(h0) and bool(second) and bool(h1) and not miss and not stray and pa.get("shown") == [keys[0], keys[2]] and pa.get("headers") == ["Kick", "Snare"]
        and pa.get("merged") == [keys[1], keys[3]],
        f"the page draws {len(h0)} hit(s) in the row of the first DAW track ({len(second)} of them of the SECOND source of that track, in song time) and {len(h1)} in the row of the second; "
        f"hit lines missing at song {miss}; hit lines at the song time of a hit that lies outside every clip {stray}; rows shown {pa.get('shown')} (want {[keys[0], keys[2]]}), "
        f"headers {pa.get('headers')} (want ['Kick', 'Snare']), rows hidden as merged {pa.get('merged')} (want {[keys[1], keys[3]]})"
        + ("" if not second or all(h["drawn"] for h in second) else f": the second source of the DAW track is not drawn at its song position (clip at {a[0]['clips'][1]['s'] if a else None} s)"))
    rb = pb.get("rows") or {}
    cut = (rb.get(b[1]["id"]) or {}).get("hits", []) if len(b) > 1 else []
    parts["editor-song-cut"] = (
        bool(cut) and all(h["drawn"] for h in cut),
        f"after the host trimmed the clip of source 2 to its slice 0.5 s to 1.5 s and put it at song 4.5 s: {len(cut)} hit(s) of the slice drawn at {[round(h['song'], 3) for h in cut]} "
        f"(all drawn: {all(h['drawn'] for h in cut)}, want True)"
        + ("" if all(h["drawn"] for h in cut) else ": the cut start of the clip is ignored"))
    return parts


def song_edit_run(label: str, arg: str, env: dict | None = None) -> dict:
    """Mode `song` with `--arg edit` (a double click in the second clip of the first DAW track and one in the gap) or `--arg base` (none)."""
    rc, _summary, rows = run_phase(label, "song", CLAP, env, ["--seq-names", "Kick", "Snare", "--seq-of", "0", "0", "1", "1", "--regions-all", "0,1.5,0", "--regions-step", "2", "--arg", arg, *(["--wait-sync"] if arg == "edit" else [])])
    return {"rc": rc, "rows": rows, "notes": host_notes_of(host_lines(label), "after-song")}


def song_edit_parts(edit: dict, base: dict) -> dict:
    names = ("editor-song-edit", "editor-song-store")
    rep = (page_reports(edit["rows"], "song-after") or [None])[0]
    if rep is None or not rep.get("edit"):
        why = f"no usable run (rc={edit['rc']}, {len(page_reports(edit['rows'], 'song-after'))} report(s))"
        return {n: (False, why) for n in names}
    e = rep["edit"]
    keys = track_names(FILES)
    sr = 48000
    src = e["src"]
    times = rep.get("page", {}).get("rows", {})
    # the page's own copy of the hits of the second source (the driver reports the count; the times come from the host notes below)
    added = e["n_after"] - e["n_before"]
    gap_ok = "outside every region" in (e.get("gap_msg") or "") and e["home_after"] == e["home_before"]
    parts: dict = {}
    # the double click is a pixel position: the stored source time is the one of that pixel (within 0.02 s of the wanted time), and the note is its sample
    stored = [t for t in e.get("added_at", []) if abs(t - e["want_src_t"]) <= 0.02]
    want_sample = round(stored[0] * sr) if stored else round(e["want_src_t"] * sr)
    en, bn = edit["notes"], base["notes"]
    changed = sorted(k for k in set(en) | set(bn) if (en.get(k) or []) != (bn.get(k) or []))
    new_s = sorted(set(en.get(changed[0]) or []) - set(bn.get(changed[0]) or [])) if len(changed) == 1 else []
    song_sample = round(e["want_song_t"] * sr)
    parts["editor-song-edit"] = (
        added == 1 and gap_ok and len(new_s) == 1 and abs(new_s[0] - want_sample) <= 1,
        f"a double click at song {e['want_song_t']:.3f} s in the second clip of the first DAW track adds {added} hit (want 1) to the SECOND source, stored at source time {e['want_src_t']:.3f} s = sample "
        f"{want_sample} (to the pixel: the stored time is {stored}); the host notes of the sources that changed ({changed}) gained the samples {new_s} (want one within 1 sample of {want_sample}; the song time would be sample {song_sample}); "
        f"a double click in the gap between the clips adds none and says {e.get('gap_msg')!r}"
        + ("" if added == 1 and new_s and abs(new_s[0] - want_sample) <= 1 else ": no hit was stored at the source time (the song time was stored, or none at all)"))
    parts["editor-song-store"] = (
        bool(bn) and len(changed) == 1 and len(new_s) == 1,
        f"compared with a run that made no edit, one source changed in the notes for the host ({changed}, {len(new_s)} new note(s)); no other source changed (want exactly one changed); "
        f"no stored time of the project, no note and no export is in song time: the offset is a display value only")
    return parts


def stage_editor_song_rows() -> bool:
    if not HOST.exists():
        return pass_fail("editor-song-rows", False, "no host")
    ok = print_parts("", song_parts(song_run("editor-song-rows")))
    ok = print_parts("", song_shift_parts(song_run("editor-song-shift", args=SONG_SHIFT_ARGS))) and ok
    return print_parts("", song_edit_parts(song_edit_run("editor-song-edit", "edit"), song_edit_run("editor-song-base", "base"))) and ok


def stage_editor_song_rows_fault() -> bool:
    ok = True
    run_ = song_run("editor-song-fault-round-regions", {"DS_ARA_SONG_FAULT": "round-regions"}, SONG_SHIFT_ARGS)
    ok = fault_reason("editor-song-fault-round-regions", song_shift_parts(run_), "editor-song-shift",
                      "the rows are made from the regions of the last round: a clip moved in the DAW is not seen", "start_play is old")
    for label, part, what, reason in (
        ("source-x", "editor-song-draw", "the page draws the hit lines at source time, not at song time", "is not drawn at its song position"),
        ("home-only", "editor-song-draw", "the page draws only the first source of a DAW track", "is not drawn at its song position"),
        ("no-cut-start", "editor-song-cut", "the mapping ignores where a clip starts in its source", "the cut start of the clip is ignored"),
    ):
        parts = song_parts(song_run(f"editor-song-fault-{label}", {"DS_ARA_SONG_FAULT": label}))
        ok = fault_reason(f"editor-song-fault-{label}", parts, part, what, reason) and ok
    base = song_edit_run("editor-song-fault-store-base", "base")
    parts = song_edit_parts(song_edit_run("editor-song-fault-store-song", "edit", {"DS_ARA_SONG_FAULT": "store-song"}), base)
    ok = fault_reason("editor-song-fault-store-song", parts, "editor-song-edit", "the page stores the song time of a hit it adds", "no hit was stored at the source time") and ok
    parts = song_parts({"rc": 2, "rows": []})
    ok = pass_fail("editor-song-fault-norun", all((not g) and "no usable run" in ev for g, ev in parts.values()),
                   "a run with no page report fails with 'no usable run'") and ok
    return ok


# ---- M1d1 (#575): the page in a DAW shows no standalone leftovers

# Mode `leftovers`: the four DAW tracks of the names run (no rename, no move). The page reports every text that names a track, the dirty dot and the
# export dialog; the host then replaces the audio of the hat (source 3) and the page reports the status line that it shows after the reload.
LEFT_ARGS = NAMES_ARGS[:10]


def leftovers_run(label: str, env: dict | None = None) -> dict:
    new = alt_hat()
    rc, summary, rows = run_phase(label, "leftovers", CLAP, env, [*LEFT_ARGS, "--update-content-after", f"3:{new}"])
    return {"rc": rc, "summary": summary, "rows": rows}


def leftovers_parts(run: dict) -> dict:
    rows = run["rows"]
    first = (page_reports(rows, "leftovers") or [None])[0]
    after = (page_reports(rows, "leftovers-after") or [None])[0]
    keys = track_names(FILES)
    if first is None or after is None:
        why = f"no usable run (rc={run['rc']}, {len(page_reports(rows, 'leftovers'))} first and {len(page_reports(rows, 'leftovers-after'))} second report)"
        return {n: (False, why) for n in ("editor-leftovers-names", "editor-leftovers-page", "editor-leftovers-audio")}
    parts: dict = {}
    a, b = first.get("a"), first.get("b")
    da, db = NAMES_BEFORE[0], NAMES_BEFORE[2]
    texts = {k: first.get(k) or "" for k in ("progress", "info", "sel", "undo", "follower", "status", "export_sel")}
    texts["follow_menu"] = " | ".join(first.get("follow_menu") or [])
    holds_key = sorted(k for k, t in texts.items() if any(x in t for x in keys))
    want = {"info": db, "undo": f"Undo Delete Hit ({db}, 1.500 s)", "status": f"{db}: 12 hits", "export_sel": f"Selected track (1): {db}"}
    wrong = sorted(k for k, v in want.items() if texts[k] != v)
    if not texts["sel"].startswith(f"{db}:"):
        wrong.append("sel")
    if not texts["follower"].startswith(f"{da} follows {db}:"):
        wrong.append("follower")
    if db not in texts["progress"]:
        wrong.append("progress")
    if sorted(first.get("follow_menu") or []) != sorted([NAMES_BEFORE[0], NAMES_BEFORE[1], NAMES_BEFORE[3]]):
        wrong.append("follow_menu")
    parts["editor-leftovers-names"] = (
        not holds_key and not wrong and a == keys[0] and b == keys[2],
        f"the page showed the DAW names where it names a track ({list(texts)}); texts that hold the key of a track: {holds_key or 'none'} "
        f"(the page holds the key in these places: {holds_key}); texts that are not the expected DAW text: {wrong or 'none'}; progress {texts['progress']!r}, "
        f"info line {texts['info']!r}, undo {texts['undo']!r}, export {texts['export_sel']!r}, Follow menu {first.get('follow_menu')}")
    note = first.get("export_note") or ""
    parts["editor-leftovers-page"] = (
        first.get("dirty_dot_hidden") is True and "start of the session" in note and "later part" not in note and first.get("dest_visible") is False,
        f"in the DAW page: the unsaved dot hidden {first.get('dirty_dot_hidden')}, the export note {note!r} (drop hint and no development note), "
        f"destination row visible {first.get('dest_visible')} (want collapsed)")
    jobs = job_rows_of(rows)
    hat = keys[3]
    changed = [j.get("audio_changed") for j in jobs]
    msg = after.get("msg") or ""
    want_msg = f"The DAW changed the audio of {NAMES_BEFORE[3]}. The hits were detected again. The hits you added, moved or edited are kept."
    parts["editor-leftovers-audio"] = (
        msg == want_msg and len(jobs) == 2 and changed == [[], [hat]],
        f"after the host replaced the audio of the hat the page said {msg!r} (want {want_msg!r}) after {after.get('waited_ms')} ms (no audio message if empty); "
        f"{len(jobs)} job row(s), the tracks whose changed audio each round analysed again: {changed} (want [[], [{hat!r}]])")
    return parts


def stage_editor_leftovers() -> bool:
    if _reference() is None or not HOST.exists():
        return pass_fail("editor-leftovers", False, "no reference or host")
    return print_parts("", leftovers_parts(leftovers_run("editor-leftovers")))


def stage_editor_leftovers_fault() -> bool:
    ok = True
    for label, env, part, what, reason in (
        ("key", {"DS_ARA_EDITOR_FAULT": "left-key"}, "editor-leftovers-names", "the label helper returns the key of the track", "holds the key"),
        ("dot", {"DS_ARA_EDITOR_FAULT": "left-dot"}, "editor-leftovers-page", "the unsaved dot shows in a DAW", "the unsaved dot hidden False"),
        ("dest", {"DS_ARA_EDITOR_FAULT": "left-dest"}, "editor-leftovers-page", "the destination row is not collapsed", "destination row visible True"),
        ("no-audio-reason", {"DS_ARA_LEFTOVER_FAULT": "no-audio-reason"}, "editor-leftovers-audio", "the round names no track whose audio changed", "no audio message if empty"),
    ):
        parts = leftovers_parts(leftovers_run(f"editor-leftovers-fault-{label}", env))
        ok = fault_reason(f"editor-leftovers-fault-{label}", parts, part, what, reason) and ok
    parts = leftovers_parts({"rc": 2, "rows": [], "summary": {}})
    return pass_fail("editor-leftovers-fault-norun", all((not g) and "no usable run" in ev for g, ev in parts.values()),
                     "a run with no page report fails with 'no usable run'") and ok


# ---- M1d4 (#578): clear states at the first open

# Mode `firstopen`: the editor opens BEFORE the audio (the host adds the four sources after the page booted, as `open-first` does) and the page samples what it
# shows every 100 ms: the line over the lanes, the strip or card of each row, the lanes text, and the answer of plugin_host_states. Four runs:
# `flow` (a slow read, a slow round), `offline` (one source gets no samples access for a while, then late), `failed` (one track fails for good: Retry runs
# only that track) and `stale` (a content update whose reads fail: the failed row keeps its hits, the user edits it, Retry brings the new audio).
FO_NAMES = ["Kick Room", "Snare Top", "Tom One", "Overhead Wide"]   # the DAW names of the four sources, by index
FO_ARGS = ["--seq-names", *FO_NAMES]


def firstopen_run(label: str, scenario: str, env: dict | None = None, extra: list | None = None) -> dict:
    late = ["--late-files", *fixture_args(FILES), "--arg", scenario, *FO_ARGS, *(extra or [])]
    rc, summary, rows = run_phase(label, "firstopen", CLAP, {"DS_ARA_RETRY_MS": "200", **(env or {})}, late, files=[])
    reps = page_reports(rows, "firstopen")
    return {"rc": rc, "rows": rows, "rep": reps[0] if reps else None, "scenario": scenario}


def fo_flow(label: str, env: dict | None = None, page_fault: str = "") -> dict:
    e = {"DS_ARA_SCENARIO": "slow-setup", **(env or {})}
    if page_fault:
        e["DS_ARA_EDITOR_FAULT"] = page_fault
    return firstopen_run(label, "flow", e, ["--slow-read", "1:8000"])


def fo_offline(label: str, env: dict | None = None) -> dict:
    return firstopen_run(label, "offline", {"DS_ARA_FIRSTOPEN_MS": "wait=3000", "DS_ARA_SCENARIO": "audio-deadline-ms:25000", **(env or {})},
                         ["--no-access", "2", "--enable-access-after", "14000:2"])


def fo_limit(label: str, env: dict | None = None) -> dict:
    return firstopen_run(label, "limit", {"DS_ARA_FIRSTOPEN_MS": "read=2500", **(env or {})}, ["--slow-read", "1:9000"])


def fo_limit_parts(run: dict) -> dict:
    bad = fo_unusable(run, ("editor-firstopen-limit",))
    if bad:
        return bad
    tl, rep = run["rep"]["timeline"], run["rep"]
    nm = FO_NAMES[1]
    text = f"{nm}: reading or analysing took too long. Press Retry."
    seen = fo_first(tl, lambda s: any(r["text"] == text and r["retry"] for r in s.get("rows", [])))
    reading = fo_first(tl, lambda s: fo_row_text(s, f"{nm}: reading the audio"))
    ans = rep.get("answers") or [{}]
    refused = bool(ans) and ans[0].get("started") == [] and "still working" in json.dumps(ans[0].get("refused"))
    last = tl[-1]
    return {"editor-firstopen-limit": (
        seen is not None and reading is not None and reading["t"] < seen["t"] and seen["t"] - reading["t"] >= 1500 and refused and (last.get("line") or "").startswith("All 4 tracks are ready") and rep.get("finished") is True,
        f"a read with no progress for 2.5 s: the row says reading, then {'the reason and Retry' if seen else 'never failed: still reading after the limit'} ({(seen or {}).get('t')} ms, reading since {(reading or {}).get('t')} ms); "
        f"Retry while the read still runs starts no second job ({refused}: {[a.get('started') for a in ans]}); the read is not cancelled: the document ends as {last.get('line')!r} (the real state comes back)")}


def fo_failed(label: str, env: dict | None = None) -> dict:
    return firstopen_run(label, "failed", {"DS_ARA_SCENARIO": "corrupt-wav:1", **(env or {})})


def fo_stale(label: str, env: dict | None = None) -> dict:
    return firstopen_run(label, "stale", env, ["--update-content-after", f"3:{alt_hat()}", "--update-fail-reads", "4"])


def fo_unusable(run: dict, names: tuple) -> dict | None:
    if run["rep"] is None or not run["rep"].get("timeline"):
        why = f"no usable run (rc={run['rc']}, no firstopen report)"
        return {n: (False, why) for n in names}
    return None


def fo_texts(s: dict) -> list:
    """Every text of one sample that the user reads: the line and each row's strip or card."""
    return [s.get("line") or ""] + [r["text"] for r in s.get("rows", [])]


def fo_first(tl: list, pred) -> dict | None:
    return next((s for s in tl if pred(s)), None)


def fo_row_text(s: dict, text: str) -> bool:
    return any(r["text"] == text for r in s.get("rows", []))


def fo_changes(tl: list) -> int:
    n = 0
    prev: dict = {"line": None, "rows": {}}
    for s in tl:
        if s.get("line") != prev["line"]:
            n += 1
        rows = {r["row"]: r["text"] for r in s.get("rows", [])}
        n += sum(1 for k, v in rows.items() if prev["rows"].get(k) != v)
        prev = {"line": s.get("line"), "rows": rows}
    return n


def fo_lines(tl: list) -> int:
    seen, last = 0, None
    for s in tl:
        if s.get("line") != last:
            seen += 1
            last = s.get("line")
    return seen


def flow_parts(run: dict) -> dict:
    names = ("editor-firstopen-reading", "editor-firstopen-analysing", "editor-firstopen-done", "editor-firstopen-once", "editor-firstopen-never-empty")
    bad = fo_unusable(run, names)
    if bad:
        return bad
    tl, rep = run["rep"]["timeline"], run["rep"]
    keys = track_names(FILES)
    parts: dict = {}
    reading = fo_first(tl, lambda s: fo_row_text(s, f"{FO_NAMES[1]}: reading the audio"))
    n_reading = fo_first(tl, lambda s: re.fullmatch(r"Reading the audio: \d of 4 tracks\.", s.get("line") or ""))
    parts["editor-firstopen-reading"] = (
        reading is not None and n_reading is not None,
        f"while the host reads the audio of the second source slowly the page shows {FO_NAMES[1]!r} as {'reading the audio' if reading else 'no reading state: no row said reading'} "
        f"and the line {(n_reading or {}).get('line')!r} (want 'Reading the audio: n of 4 tracks.'); lines seen: {sorted({s.get('line') for s in tl if s.get('line')})[:6]}")
    ana = [s for s in tl if re.fullmatch(r"Analysing: \d of 4 tracks\.", s.get("line") or "")]
    ns = [int(re.search(r"(\d) of", s["line"]).group(1)) for s in ana]
    texts = [t for s in tl for t in fo_texts(s)]
    holds = sorted({k for k in keys for t in texts if k in t})
    named = all(any(n in t for t in texts) for n in FO_NAMES)
    parts["editor-firstopen-analysing"] = (
        bool(ana) and ns == sorted(ns) and not holds and named,
        f"the page showed {len(ana)} sample(s) of 'Analysing: n of 4 tracks.' (n never goes back: {ns == sorted(ns)}); every row text names the DAW track ({named}: {FO_NAMES}); "
        f"texts that hold the machine name of a source: {holds or 'none'} (holds the key if listed)" + ("" if ana else "; no analysing line was shown"))
    last = tl[-1]
    want = f"All 4 tracks are ready: {last.get('hits')} hits."
    parts["editor-firstopen-done"] = (
        last.get("line") == want and last.get("hits", 0) > 0 and rep.get("finished") is True,
        f"at the end the line says {last.get('line')!r} (want {want!r}, the hits that the page holds); finished {rep.get('finished')}; strips left: {[r['text'] for r in last.get('rows', [])]}")
    lines, writes = fo_lines(tl), (tl[-1].get("line_writes") or 0)
    parts["editor-firstopen-once"] = (
        writes <= lines + 2,
        f"the line was written {writes} times for {lines} different texts (want at most {lines + 2}: a change is written once, not at every poll; announced more than once if more)")
    empty = [s["t"] for s in tl if not (s.get("line") or "").strip()]
    old = [s["t"] for s in tl if "put the Drum Spike plugin on a drum track" in (s.get("lanes") or "")]
    parts["editor-firstopen-never-empty"] = (
        not empty and not old,
        f"{len(tl)} samples: samples with no text on the line: {empty[:5] or 'none'}; samples with the old empty text: {old[:5] or 'none'}")
    return parts


def offline_parts(run: dict) -> dict:
    names = ("editor-firstopen-waiting", "editor-firstopen-offline", "editor-firstopen-retry-offline", "editor-firstopen-late")
    bad = fo_unusable(run, names)
    if bad:
        return bad
    tl, rep = run["rep"]["timeline"], run["rep"]
    nm = FO_NAMES[2]
    parts: dict = {}
    wait = fo_first(tl, lambda s: fo_row_text(s, f"{nm}: waiting for the audio from your DAW"))
    first_line = next((s.get("line") for s in tl), None)
    parts["editor-firstopen-waiting"] = (
        wait is not None,
        f"a source that the host gave no audio shows {nm!r} as waiting for the audio from your DAW"
        + ("" if wait else f": never shown: the old empty state (the line says {first_line!r}, lanes {tl[len(tl) // 2].get('lanes', '')[:60]!r})"))
    off_text = f"{nm}: the DAW gives no audio for this track. In the DAW, make sure the clip is on a track and its audio file is present, then press Retry."
    off = fo_first(tl, lambda s: any(r["text"] == off_text and r["retry"] for r in s.get("rows", [])))
    gap = (off["t"] - wait["t"]) if (off and wait) else None
    parts["editor-firstopen-offline"] = (
        off is not None and gap is not None and gap >= 1500,
        f"after the wait limit (3 s) the row says what to do in the DAW and has Retry: {'seen' if off else 'never offline: still waiting after the limit'}; "
        f"{gap} ms after the waiting state (want at least 1500); the line then says {(off or {}).get('line')!r}")
    ans = rep.get("answers") or [{}, {}]
    ask = fo_first(tl, lambda s: s["t"] >= rep.get("press_at", 0) and fo_row_text(s, f"{nm}: asking the DAW for the audio"))
    off2 = fo_first(tl, lambda s: ask is not None and s["t"] > ask["t"] and any(r["text"] == off_text for r in s.get("rows", [])))
    two = (len(ans) == 2 and len(ans[0].get("started", [])) == 1 and ans[1].get("started") == [])
    ans2 = rep.get("answers2") or [{}]
    again = ans2[0].get("started") == [rep.get("pressed")]
    parts["editor-firstopen-retry-offline"] = (
        ask is not None and off2 is not None and two and again,
        f"after Retry the row says 'asking the DAW for the audio' ({'seen' if ask else 'never'}) and goes offline again after the wait limit ({'seen' if off2 else 'never'}); "
        f"two presses in one go: started {[a.get('started') for a in ans]} (want one start and one refusal); a press after the wait limit started {ans2[0].get('started')} "
        f"(want the row again: {'a dead end if it started nothing' if not again else 'ok'}; refused: {ans2[0].get('refused')})")
    last = tl[-1]
    last_off = max((s["t"] for s in tl if any(r["text"] == off_text for r in s.get("rows", []))), default=0)
    left = [s for s in tl if s["t"] > last_off]
    first_left = left[0]["t"] if left else None
    parts["editor-firstopen-late"] = (
        bool(left) and first_left is not None and first_left >= 12000 and (last.get("line") or "").startswith("All 4 tracks are ready") and rep.get("finished") is True,
        f"when the DAW gives the audio late (at 14 s) the row leaves offline (at {first_left} ms, not before the audio came) and the document ends as {last.get('line')!r} (the real state comes back)")
    return parts


def failed_parts(run: dict) -> dict:
    names = ("editor-firstopen-failed", "editor-firstopen-retry-one")
    bad = fo_unusable(run, names)
    if bad:
        return bad
    tl, rep, rows = run["rep"]["timeline"], run["rep"], run["rows"]
    nm = FO_NAMES[1]
    key = track_names(FILES)[1]
    parts: dict = {}
    fs = fo_first(tl, lambda s: any(r["text"].startswith(f"{nm}: ") and r["retry"] for r in s.get("rows", [])))
    text = next((r["text"] for r in (fs or {}).get("rows", []) if r["text"].startswith(f"{nm}: ")), "")
    parts["editor-firstopen-failed"] = (
        fs is not None and "the analysis did not finish" in text and text.endswith("Press Retry.") and "1 of 4 tracks needs attention." == fs.get("line") and not fs.get("retry_all"),
        f"a track that failed shows its reason in plain words and Retry: {text!r} (want the reason 'the analysis did not finish' and 'Press Retry.' - no reason if it says only failed); "
        f"line {(fs or {}).get('line')!r}; Retry all shown {(fs or {}).get('retry_all')} (want no, only one row failed)")
    ans = rep.get("answers") or [{}, {}]
    before, after = rep.get("before") or {}, rep.get("after") or {}
    others = [k for k in before if k != key]
    same = bool(others) and all(before.get(k) == after.get(k) for k in others)
    i = next((n for n, r in enumerate(rows) if r.get("event") == "state_retry"), None)
    later = rows[i:] if i is not None else []
    analysed = sorted({t for r in later if r.get("event") == "job" for t in (r.get("analysed") or [])})
    rounds = sum(1 for r in later if r.get("event") == "round_start")
    edited = rep.get("edited") or {}
    kept = False
    if edited.get("name") and after.get(edited["name"]):
        try:
            kept = any(abs(e["t"] - edited["to"]) < 1e-4 for e in json.loads(after[edited["name"]])["events"])
        except (ValueError, KeyError):
            kept = False
    started = [a.get("started") for a in ans]
    parts["editor-firstopen-retry-one"] = (
        started == [[rep.get("pressed")], []] and same and kept and set(analysed) <= {key} and rounds == 1,
        f"Retry started {started} (want only the failed row, then a refusal for the second press); the events of the {len(others)} other tracks are byte for byte the same before and after ({same}), "
        f"the hit that the user moved on a finished track is still there ({kept}); the round after the press analysed {analysed} (want only {key!r}: other tracks touched if more) "
        f"in {rounds} round(s) (want 1)")
    return parts


def stale_parts(run: dict) -> dict:
    names = ("editor-firstopen-stale", "editor-firstopen-handles")
    bad = fo_unusable(run, names)
    if bad:
        return bad
    tl, rep = run["rep"]["timeline"], run["rep"]
    nm = FO_NAMES[3]
    key = track_names(FILES)[3]
    parts: dict = {}
    fs = fo_first(tl, lambda s: any(r["text"].startswith(f"{nm}: ") and r["retry"] for r in s.get("rows", [])))
    text = next((r["text"] for r in (fs or {}).get("rows", []) if r["text"].startswith(f"{nm}: ")), "")
    edited = rep.get("edited") or {}
    after = rep.get("after") or {}
    kept = False
    try:
        kept = any(abs(e["t"] - edited["to"]) < 1e-4 for e in json.loads(after[edited["name"]])["events"])
    except (ValueError, KeyError, TypeError):
        kept = False
    held = (fs or {}).get("events_n", {}).get(key, 0)
    parts["editor-firstopen-stale"] = (
        fs is not None and text.endswith("The hits shown are from before.") and held > 0 and edited.get("name") == key and kept and (tl[-1].get("line") or "").startswith("All 4 tracks are ready"),
        f"a row whose new audio could not be read keeps its {held} hits on the page and says {text!r} (want '... The hits shown are from before.'); "
        f"the hit that the user moved on that row is still in the events after Retry brought the new audio ({kept}); the document ends as {tl[-1].get('line')!r}")
    ready = [s["t"] for s in tl for h in (s.get("handles") or [])
             if h["midi"] == "ready" and any(p["id"] == h["id"] and p["state"] != "done" for p in (s.get("plugin") or {}).get("rows", []))]
    parts["editor-firstopen-handles"] = (
        fs is not None and not ready,
        f"a row that is not done has no ready drag handle: samples with a ready handle on a row that is not done: {ready[:5] or 'none'} (ready handle before done if listed)")
    return parts


def stage_editor_firstopen() -> bool:
    if _reference() is None or not HOST.exists():
        return pass_fail("editor-firstopen", False, "no reference or host")
    ok = print_parts("", flow_parts(fo_flow("editor-firstopen-flow")))
    ok = print_parts("", offline_parts(fo_offline("editor-firstopen-offline"))) and ok
    ok = print_parts("", failed_parts(fo_failed("editor-firstopen-failed"))) and ok
    ok = print_parts("", fo_limit_parts(fo_limit("editor-firstopen-limit"))) and ok
    return print_parts("", stale_parts(fo_stale("editor-firstopen-stale"))) and ok


def stage_editor_firstopen_fault() -> bool:
    ok = True
    flow = {"left-key": ({"DS_ARA_FIRSTOPEN_FAULT": "left-key"}, "editor-firstopen-analysing", "a row names the machine name of its source", "holds the key"),
            "no-reading": ({"DS_ARA_FIRSTOPEN_FAULT": "no-reading"}, "editor-firstopen-reading", "a read shows as waiting", "no reading state"),
            "old-empty": ({"DS_ARA_FIRSTOPEN_FAULT": "old-empty"}, "editor-firstopen-reading", "a source with no track gives no state: the old empty page", "no reading state")}
    for label, (env, part, what, reason) in flow.items():
        ok = fault_reason(f"editor-firstopen-fault-{label}", flow_parts(fo_flow(f"editor-firstopen-fault-{label}", env)), part, what, reason) and ok
    ok = fault_reason("editor-firstopen-fault-announce", flow_parts(fo_flow("editor-firstopen-fault-announce", None, "first-announce")), "editor-firstopen-once",
                      "the page writes the line at every poll", "announced more than once") and ok
    ok = fault_reason("editor-firstopen-fault-old-empty-waiting", offline_parts(fo_offline("editor-firstopen-fault-old-empty", {"DS_ARA_FIRSTOPEN_FAULT": "old-empty"})),
                      "editor-firstopen-waiting", "a source with no track gives no state", "the old empty state") and ok
    ok = fault_reason("editor-firstopen-fault-no-limit", offline_parts(fo_offline("editor-firstopen-fault-no-limit", {"DS_ARA_FIRSTOPEN_FAULT": "no-limit"})),
                      "editor-firstopen-offline", "no time limit fires", "still waiting after the limit") and ok
    ok = fault_reason("editor-firstopen-fault-retry-dead-end", offline_parts(fo_offline("editor-firstopen-fault-retry-dead-end", {"DS_ARA_FIRSTOPEN_FAULT": "retry-dead-end"})),
                      "editor-firstopen-retry-offline", "a press of Retry never ends", "a dead end if it started nothing") and ok
    ok = fault_reason("editor-firstopen-fault-no-reason", failed_parts(fo_failed("editor-firstopen-fault-no-reason", {"DS_ARA_FIRSTOPEN_FAULT": "no-reason"})),
                      "editor-firstopen-failed", "a failed row says no reason", "no reason if it says only failed") and ok
    ok = fault_reason("editor-firstopen-fault-retry-all", failed_parts(fo_failed("editor-firstopen-fault-retry-all", {"DS_ARA_FIRSTOPEN_FAULT": "retry-all-tracks"})),
                      "editor-firstopen-retry-one", "Retry runs for every track", "other tracks touched") and ok
    ok = fault_reason("editor-firstopen-fault-handle-early", stale_parts(fo_stale("editor-firstopen-fault-handle-early", {"DS_ARA_FIRSTOPEN_FAULT": "handle-early"})),
                      "editor-firstopen-handles", "a drag handle is ready before its row is done", "ready handle before done") and ok
    ok = fault_reason("editor-firstopen-fault-no-limit-read", fo_limit_parts(fo_limit("editor-firstopen-fault-no-limit-read", {"DS_ARA_FIRSTOPEN_FAULT": "no-limit"})),
                      "editor-firstopen-limit", "no time limit fires", "still reading after the limit") and ok
    parts = flow_parts({"rc": 2, "rows": [], "rep": None})
    return pass_fail("editor-firstopen-fault-norun", all((not g) and "no usable run" in ev for g, ev in parts.values()), "a run with no page report fails with 'no usable run'") and ok


# ---- M1b10 item 8: the role of a track follows the name of the DAW track

# The sources have machine names (32 hex characters). The DAW tracks are named like a kick, a snare, a hi-hat
# and a track with no role in its name. The host then renames the last one to a snare name (a track whose role is
# the automatic guess changes), or the user sets a role by hand and the host renames that track (the role stays).
ROLE_HEX = ["3f9a1c7e5b2d4a8f9c0e1d2b3a4f5e6d", "7b1e9d3c5a2f4e8b0c6d1a3f5e7b9c2d",
            "c4d2e6a8b0f1394a5c7e9b1d3f5a7c9e", "0a2c4e6f8b1d3f5a7c9e0b2d4f6a8c1e"]
ROLE_SEQS = ["Kick In", "Snare Top", "Hi-Hat", "Mystery"]
ROLES_BEFORE = [("kick", 36), ("snare", 38), ("hat", 42), ("other", 36)]


def roles_run(label: str, hand: bool, env: dict | None = None) -> dict:
    keys = [safe_name(stem_of(h)) for h in ROLE_HEX]
    rename = "0=Hi-Hat Top" if hand else "3=Snare Bottom"
    args = ["--names", *ROLE_HEX, "--seq-names", *ROLE_SEQS, "--rename-seq", rename,
            "--arg", "roles-hand" if hand else "roles"]
    rc, summary, rows = run_phase(label, "names", CLAP, env, args)
    return {"rc": rc, "rows": rows, "keys": keys}


def got_roles(rep: dict, keys: list) -> list:
    return [((rep.get("roles") or {}).get(k) or {}) for k in keys]


def roles_parts(run: dict, hand: bool) -> dict:
    rows, keys = run["rows"], run["keys"]
    first = (page_reports(rows, "names") or [{}])[0]
    after = (page_reports(rows, "names-after") or [{}])[0]
    rounds = [r for r in rows if r.get("event") == "round_start"]
    b, a = got_roles(first, keys), got_roles(after, keys)
    pair = lambda lst: [(r.get("role"), r.get("note")) for r in lst]
    parts: dict = {}
    if not hand:
        auto = got_roles({"roles": first.get("roles_auto")}, keys)
        parts["editor-roles-daw"] = (
            run["rc"] == 0 and first.get("order") == keys and pair(auto) == ROLES_BEFORE and all(r.get("role_source") == "guess" for r in auto[:3]),
            f"four sources with machine names on DAW tracks named {ROLE_SEQS} got the roles and default notes {pair(auto)} (want {ROLES_BEFORE})")
        parts["editor-roles-rename"] = (
            pair(a) == ROLES_BEFORE[:3] + [("snare", 38)] and pair(b) == ROLES_BEFORE,
            f"after the DAW track with no role in its name was renamed to a snare name the roles are {pair(a)} (want the first three unchanged and 'snare', 38)")
        parts["editor-roles-one-round"] = (
            len(rounds) == 2,
            f"{len(rounds)} round(s) in the whole run (want 2: the first analysis and one for the changed role)")
    else:
        parts["editor-roles-hand"] = (
            run["rc"] == 0 and bool(b) and b[0].get("role") == "cymbal" and b[0].get("role_source") == "user"
            and bool(a) and a[0].get("role") == "cymbal" and pair(a[1:]) == ROLES_BEFORE[1:],
            f"the user set the role of the first track to cymbal ({pair(b)[:1]}); the DAW track was renamed to a hi-hat name; the roles are now {pair(a)} (want cymbal first, the others unchanged)")
    return parts


def stage_editor_roles() -> bool:
    if not HOST.exists():
        return pass_fail("editor-roles", False, "no host")
    ok = print_parts("", roles_parts(roles_run("editor-roles", False), False))
    return print_parts("", roles_parts(roles_run("editor-roles-hand", True), True)) and ok


def stage_editor_roles_fault() -> bool:
    ok = True
    for label, fault, hand, part, what in (
        ("source", "role-source", False, "editor-roles-daw", "the role guess reads the name of the audio source only: hex names give 'other'"),
        ("frozen", "role-frozen", False, "editor-roles-rename", "the role is guessed once and a rename of the DAW track never changes it"),
        ("hand", "role-hand", True, "editor-roles-hand", "a new DAW name overwrites the role that the user set"),
    ):
        run_ = roles_run(f"editor-roles-fault-{label}", hand, {"DS_ARA_NAMES_FAULT": fault})
        ok = fault_part(f"editor-roles-fault-{label}", roles_parts(run_, hand), part, what) and ok
    return ok


# ---- M1b10 N10: a restore of the archive puts back what the user set

def restore_settings_parts(first_run: dict, run: dict, archive: Path, keys: list) -> dict:
    """The user set a role, a note and a volume by hand in the page (run 1, archive stored); a new document
    restores that archive (run 2): the same role, role_source, note and volume, the stored hits, and no guess and
    no round because of it."""
    rows1 = first_run["rows"]
    after = (page_reports(rows1, "names-after") or [{}])[0]
    want = {k: (r.get("role"), r.get("note"), r.get("role_source")) for k, r in (after.get("roles") or {}).items()}
    try:
        doc = tracks_of_doc(doc_of_run(run["cache"]))
        arch = json.loads(archive.read_text())
    except (OSError, ValueError, IndexError, StopIteration):
        doc, arch = {}, {}
    got = {k: (t.get("role"), t.get("note"), t.get("role_source")) for k, t in doc.items()}
    first_key = keys[0]
    rounds = [r for r in run["rows"] if r.get("event") in ("round_start", "job")]
    held = [s for s in arch.get("sources", []) if s.get("settings")]
    return {
        "restore-settings": (
            got == want and want.get(first_key) == ("cymbal", 60, "user") and len(held) == len(keys),
            f"the archive holds the settings of {len(held)} of {len(keys)} tracks; the restored document has {got} (want {want}): the role, the note and "
            f"role_source the user set, nothing guessed"),
        "restore-volume": (
            (doc.get(first_key) or {}).get("volume_db") == -3,
            f"the volume the user set ({(doc.get(first_key) or {}).get('volume_db')}) is back"),
        "restore-hits": (
            bool(arch.get("events")) and {k: (t.get("events") or []) for k, t in doc.items()} == arch.get("events"),
            "the hits of the restored document are the hits of the archive, edited ones too"),
        "restore-no-round": (
            not rounds,
            f"{len(rounds)} round or job row(s) in the run that restored: the restore makes no round"),
    }


def restore_settings_run(label: str, env: dict | None = None) -> tuple[dict, dict, Path, list]:
    keys = [safe_name(stem_of(h)) for h in ROLE_HEX]
    archive = OUT / f"{label}.ara"
    if archive.exists():
        archive.unlink()
    base = ["--names", *ROLE_HEX, "--seq-names", *ROLE_SEQS]
    rc, summary, rows = run_phase(f"{label}-store", "names", CLAP, None, [*base, "--arg", "settings", "--archive", str(archive)])
    first = {"rc": rc, "rows": rows}
    cache = f"cache-{label}-restore"
    # --enable-access: the host reads the audio of the restored sources and pumps 3 s, longer than the settle time of
    # the plugin: a round that the restore wrongly started would show in the report
    rc2, text, lines = host_run(f"{label}-restore", CLAP, ["restore", "--files", *fixture_args(FILES), "--archive", str(archive), *base, "--enable-access"], cache, extra=env)
    run = {"rc": rc2, "rows": report_rows(f"{label}-restore"), "cache": cache}
    return first, run, archive, keys

def late_names_run(label: str, archive: Path, store_cache: str, env: dict | None = None) -> dict:
    """The names arrive AFTER the restore (a real host may make them late): the sequences are made with no name, then the
    host names two of them. One automatic track gets a snare name, the track with the hand-set role gets a hi-hat name."""
    keys = [safe_name(stem_of(h)) for h in ROLE_HEX]
    # the cache folder of the run that stored the archive stays (a session keeps its cache): a restore into an empty cache
    # that then analyses finds the cached WAV records changed (observed: "offline (changed)", see the design note)
    cache = store_cache
    args = ["restore", "--files", *fixture_args(FILES), "--archive", str(archive), "--names", *ROLE_HEX,
            "--seq-names", "-", "-", "-", "-", "--rename-seq", "3=Snare Bottom", "--rename-seq", "0=Hi-Hat Top",
            "--enable-access", "--linger-ms", "8000"]
    rc, text, lines = host_run(label, CLAP, args, cache, extra=env, keep_cache=True)
    return {"rc": rc, "rows": report_rows(label), "cache": cache, "keys": keys}


def late_names_parts(run: dict, archive: Path) -> dict:
    keys = run["keys"]
    try:
        doc = tracks_of_doc(doc_of_run(run["cache"]))
        arch = json.loads(archive.read_text())
    except (OSError, ValueError, IndexError, StopIteration):
        doc, arch = {}, {}
    held = {s.get("track"): s.get("settings") or {} for s in arch.get("sources", [])}
    rounds = [r for r in run["rows"] if r.get("event") == "round_start"]
    t0, t3 = doc.get(keys[0]) or {}, doc.get(keys[3]) or {}
    params_same = all((doc.get(k) or {}).get("params") == (held.get(k) or {}).get("params") for k in keys)
    return {"restore-late-names": (
        run["rc"] == 0 and len(rounds) == 1 and (t0.get("role"), t0.get("note"), t0.get("role_source"), t0.get("volume_db")) == ("cymbal", 60, "user", -3)
        and (t3.get("role"), t3.get("note")) == ("snare", 38) and params_same,
        f"names that came after the restore: {len(rounds)} round (want 1, with the restored parameters); the hand-set track is "
        f"{(t0.get('role'), t0.get('note'), t0.get('role_source'), t0.get('volume_db'))} (want cymbal, 60, user, -3: a hi-hat name changes nothing), the "
        f"automatic track with a snare name is {(t3.get('role'), t3.get('note'))} (want snare, 38), parameters of all tracks the stored ones ({params_same})")}


def stage_restore_settings() -> bool:
    if not HOST.exists():
        return pass_fail("restore-settings", False, "no host")
    first, run, archive, keys = restore_settings_run("restore-settings")
    ok = print_parts("", restore_settings_parts(first, run, archive, keys))
    late = late_names_run("restore-late-names", archive, "cache-restore-settings-store")
    return print_parts("", late_names_parts(late, archive)) and ok


def stage_restore_settings_fault() -> bool:
    first, run, archive, keys = restore_settings_run("restore-settings-fault", {"DS_ARA_NAMES_FAULT": "restore-guess"})
    ok = fault_part("restore-settings-fault", restore_settings_parts(first, run, archive, keys), "restore-settings",
                    "the restore guesses the role again and drops the settings of the user")
    # the hand-set role must survive a name that arrives after the restore: the fault run lets the name overwrite it
    ok_archive = OUT / "restore-settings-fault.ara"
    late = late_names_run("restore-late-names-fault", ok_archive, "cache-restore-settings-fault-store", {"DS_ARA_NAMES_FAULT": "role-hand"})
    return fault_part("restore-late-names-fault", late_names_parts(late, ok_archive), "restore-late-names",
                      "a name that arrives after the restore overwrites the role the user set") and ok


# ---------------------------------------------------------------------------- M1b12 (#561): keys

HK_FLAG_CMD = 1 << 20
# the steps of the host, in order: (label, the key the page sees or None when the page must not see it, what the page makes
# of the key, the group of an absent command). The check proves the keys are what the stage says they are.
HK_TABLE = [
    ("space", "Space", "page", None), ("space-repeat", None, None, None), ("page-key", "KeyG", "page", None),
    ("page-add-hit", "KeyH", "page", None), ("absent-key", "KeyU", "absent", "audio"), ("absent-key-cmd", "KeyE", "absent", "files"),
    ("no-command", "KeyY", "none", None), ("no-command-shift", "KeyY", "none", None), ("no-command-cmd", "KeyJ", "none", None),
    ("no-command-option", "KeyX", "none", None), ("page-key-cmd", "KeyZ", "page", None), ("escape", "Escape", "page", None),
    ("return", "Enter", "page", None), ("arrow-left", "ArrowLeft", "page", None), ("function-key", "F5", "none", None),
    ("control-key", "KeyL", "none", None), ("cmd-s", "KeyS", "absent", ""), ("cmd-w", "KeyW", "absent", ""),
    ("slow-delete", "Backspace", "none", None), ("slow-move", "ArrowLeft", "none", None),
    ("marker-field-on", "F13", "none", None), ("field-letter", "KeyU", "absent", "audio"), ("field-space", "Space", "page", None),
    ("field-cmd-absent", "KeyE", "absent", "files"), ("field-cmd-none", "KeyJ", "none", None),
    ("field-option-at", "KeyL", "none", None), ("field-option-lbracket", "Digit5", "none", None),
    ("field-option-rbracket", "Digit6", "none", None), ("field-delete", "Backspace", "page", None), ("field-undo", "KeyZ", "page", None),
    ("field-tab", "Tab", "page", None), ("field-tab-2", "Tab", "page", None), ("field-tab-3", "Tab", "page", None), ("field-tab-4", "Tab", "page", None), ("after-tab", "KeyY", "none", None), ("marker-field-off", "F14", "none", None),
    ("menu-action", None, None, None), ("system-hide", None, None, None), ("system-quit", None, None, None),
    ("click-out", None, None, None), ("given-back", None, None, None), ("mod-down", None, None, None), ("click-in", None, None, None),
    ("mod-up", None, None, None), ("taken", "KeyQ", "none", None), ("click-overlay", None, None, None), ("overlay-key", None, None, None), ("click-in-again", None, None, None),
    ("marker-end", "F12", "none", None),
]
HK_STEPS = [t[0] for t in HK_TABLE]
HK_PAGE_CODES = [t[1] for t in HK_TABLE if t[1]]
HK_PAGE_CLASS = [(t[2], t[3]) for t in HK_TABLE if t[1]]
HK_SYSTEM_STEPS = {"system-hide": "Host Hide", "system-quit": "Host Quit"}   # Cmd+H, Cmd+Q: the exception list


def hostkeys_run(label: str, bundle: Path, env: dict | None = None) -> tuple[int, dict, list, dict, dict]:
    """One run of the `hostkeys` mode: real key events posted to the window of the test host, a host with a playback
    controller. Returns the rc, the summary, the rows of the plugin, the host's playback record and its key record."""
    args = ["--transport", "follow", "--arg", "full", "--snap", str(PB_SNAP), "--playback", "full",
            "--regions-all", ",".join(str(x) for x in TRANSPORT_REGION)]
    rc, summary, rows = run_phase(label, "hostkeys", bundle, env, args, files=transport_files())
    return rc, summary, rows, host_row(label, "playback_record"), host_row(label, "key_record")


def hk_unusable(names: list, rc: int, rep: dict, krec: dict) -> dict | None:
    """The parts all fail when the run did not happen: no host key record, not every step posted, or a hidden page. A run
    whose page sent no report (the last key never reached it) is a run: the parts of the host record are still judged."""
    posted = [x["step"] for x in krec.get("posted", [])] if krec else []
    if not krec or posted != HK_STEPS or (rep and rep.get("visibility") != "visible"):
        why = (f"the page is {rep.get('visibility')}: the screen of this host is locked or its window occluded"
               if rep and rep.get("visibility") != "visible" else
               f"no usable run: rc={rc}, page report {bool(rep)}, host key record {bool(krec)}, steps posted {len(posted)} of {len(HK_STEPS)}")
        return {n: (False, why) for n in names}
    return None


def hk_leaks(krec: dict) -> list:
    """The key events that reached the host and are not on the exception list. The two system keys may reach it (any
    event of them), and the key after the click out of the plugin (step given-back) is the host's by design."""
    out = []
    for k in krec.get("seen", []):
        if k["step"] in HK_SYSTEM_STEPS and k["via"] == "menu" and k["title"] == HK_SYSTEM_STEPS[k["step"]]:
            continue
        if k["step"] in HK_SYSTEM_STEPS and k["via"] in ("window", "view") and k["kind"] == "keyUp":
            continue   # the key up of a key whose down went to the host belongs to the host
        if k["kind"] == "flagsChanged" and k["via"] in ("window", "view"):
            continue   # a change of the modifier keys alone runs no command: the host gets it
        if k["step"] in ("given-back", "overlay-key") and k["via"] in ("view", "window", "focus-view") and k["code"] == 12 and not k["flags"]:
            continue
        if k["step"] == "menu-action" and k["via"] == "menu" and k["title"] == "Host Save":
            continue
        out.append(k)
    return out


def hk_show(k: dict) -> str:
    mods = ("Cmd+" if k["flags"] & HK_FLAG_CMD else "") + ("Option+" if k["flags"] & (1 << 19) else "")
    what = f"{mods}{k['chars']}" if k["chars"] else (repr(k["title"]) if k["title"] else str(k["code"]))
    return f"{k['step']}: {k['kind']} {what} via {k['via']}"


def hk_step_keys(krec: dict, *steps: str) -> list:
    return [k for k in krec.get("seen", []) if k["step"] in steps and k["kind"] != "flagsChanged"]


def hostkeys_parts(rc: int, rows: list, rec: dict, krec: dict) -> dict:
    names = ["editor-keys-classes", "editor-keys-host-record", "editor-keys-system", "editor-keys-space", "editor-keys-text",
             "editor-keys-field", "editor-keys-focus", "editor-keys-undo", "editor-keys-tab", "editor-keys-action", "editor-keys-slow",
             "editor-keys-modifier", "editor-keys-overlay"]
    reps = page_reports(rows, "hostkeys")
    rep = reps[0] if reps else {}
    bad = hk_unusable(names, rc, rep, krec)
    if bad:
        return bad
    keys = rep.get("keys", [])
    codes = [k["code"] for k in keys]
    leaks = hk_leaks(krec)
    stops = [r for r in rows if r.get("event") == "key_stop"]
    passes = [r for r in rows if r.get("event") == "key_pass"]
    by_where: dict = {}
    for r in stops:
        by_where[r["where"]] = by_where.get(r["where"], 0) + 1
    # the key the page saw at each step that reaches it, by step label
    page_steps = [t[0] for t in HK_TABLE if t[1]]
    by_step = dict(zip(page_steps, keys)) if len(keys) == len(page_steps) else {}
    # --- classes: the keys are the kinds of key the ticket names, and the page saw them in order
    groups = {g["group"]: g["say"] for g in rep.get("groups", [])}
    cls = [(k["cls"].get("kind"), k["cls"].get("group")) for k in keys]
    cls_ok = codes == HK_PAGE_CODES
    wrong = []
    if cls_ok:
        for i, ((kind, grp), (wk, wg)) in enumerate(zip(cls, HK_PAGE_CLASS)):
            if kind != wk or (wk == "absent" and grp != wg):
                wrong.append(f"{page_steps[i]}: {HK_PAGE_CODES[i]} is {kind}/{grp}, want {wk}/{wg}")
    cls_ok = cls_ok and not wrong
    cls_ev = (f"the page saw the {len(keys)} keys that stay in the plugin, in order: a page command (Space, G, H, Cmd+Z, Escape, Return, arrow, Tab, Delete), an absent command "
              f"(U, Cmd+E, Cmd+S, Cmd+W), no command (Y, Shift+Y, Cmd+J, Option+X, F5, Control+L), and the same with a text field in focus, "
              f"Option+L, Option+5 and Option+6 (the German at sign and brackets) typed into the field"
              if cls_ok else f"the page did not see the keys it should, in order: saw {codes} (want {HK_PAGE_CODES}); {wrong}")
    # --- host record: nothing but the exception list (and the keys after the click out)
    rec_ok = not leaks and bool(krec.get("posted"))
    rec_ev = (f"the host's window, its content view (an ANCESTOR of the plugin view that takes every key without Command as a key command of its own, and refuses the first "
              f"responder) and its menu (its key commands) saw {len(krec.get('seen', []))} key event(s): the two system keys, the key after the click outside, one menu action; "
              f"the plugin took {len(stops)} events ({by_where}) and passed {len(passes)} system key(s) on"
              if rec_ok else f"{len(leaks)} key event(s) reached the host: {'; '.join(hk_show(k) for k in leaks[:8])}")
    # --- system keys
    sys_seen = {k["step"]: k for k in krec.get("seen", []) if k["via"] == "menu" and k["step"] in ("system-hide", "system-quit")}
    sys_ok = (sys_seen.get("system-hide", {}).get("title") == "Host Hide" and sys_seen.get("system-quit", {}).get("title") == "Host Quit"
              and len(passes) == 2)
    sys_ev = (f"Cmd+H ran the host's 'Host Hide' and Cmd+Q its 'Host Quit' (the exception list); the plugin passed {len(passes)} keys on and the page saw neither"
              if sys_ok else f"a system key did not reach the host: menu records {[(k['step'], k['title']) for k in sys_seen.values()]}, plugin passed {len(passes)} (want 2)")
    # --- space: one play request through the page, no key; a held space bar and a space typed into a field give none
    reqs = rec.get("requests", [])
    kinds = [r["kind"] for r in reqs]
    space_page = by_step.get("space", {})
    space_key = hk_step_keys(krec, "space", "space-repeat", "field-space")
    calls = rep.get("calls", [])
    plays = [c for c in calls if c["kind"] == "play"]
    t0 = space_page.get("at", 0)
    near = bool(plays) and abs(plays[0]["at"] - t0) <= 1000
    held = [c for c in calls if t0 + 400 <= c["at"] <= t0 + 2500]   # the held space bar comes 0.9 s after the first
    fs = by_step.get("field-space", {})
    in_field = [c for c in calls if fs and abs(c["at"] - fs.get("at", 0)) <= 1500]
    space_ok = (kinds.count("play") == 1 and len(plays) == 1 and near and not space_key and space_page.get("prevented") is True
                and not held and not in_field and bool(fs) and fs.get("value") == "u "
                and any(r["kind"] == "play" and r.get("playing_after") is True for r in reqs))
    space_ev = (f"the space bar gave exactly one play request to the host (host requests {kinds}: the play within {plays[0]['at'] - t0 if plays else None} ms of the key), through "
                f"B.hostTransport; a held space bar (repeating key down) gave no second request, a space typed into the text field gave none and typed its space "
                f"({fs.get('value')!r}); no space key reached the host" if space_ok else
                f"the space bar did not behave: the host got {kinds}, the shell made {[c['kind'] for c in calls]}, the play call is "
                f"{(plays[0]['at'] - t0) if plays else None} ms from the space bar, page prevented={space_page.get('prevented')}, space key events at the host {len(space_key)}, "
                f"{len(held)} call(s) while the space bar is held, {len(in_field)} call(s) for the space in the field, field value {fs.get('value')!r}"
                + (" (no play request)" if "play" not in kinds else ""))
    # --- text
    say = set(groups.values())
    bad_text = []
    texts_ok = len(keys) == len(HK_PAGE_CLASS)
    if not texts_ok:
        bad_text = [f"the page saw {len(keys)} keys, want {len(HK_PAGE_CLASS)}: the text cannot be matched to its key (the page sent {'a' if rep else 'no'} report)"]
    for i, k in enumerate(keys[:len(HK_PAGE_CLASS)]):
        typing = k.get("field") and not (k.get("meta") or k.get("ctrl"))   # a letter typed into a field is the field's
        want = (groups.get(HK_PAGE_CLASS[i][1]) or "the host owns this") if HK_PAGE_CLASS[i][0] == "absent" and not typing else None
        msg = k.get("msg") or ""
        if want is not None and msg != want:
            bad_text.append(f"{page_steps[i]}: shows {msg!r}, want {want!r}")
        if want is None and msg in say:
            bad_text.append(f"{page_steps[i]}: shows {msg!r}, want no group text")
    text_ok = not bad_text
    text_ev = (f"a key of an absent command showed the text of its group ({groups.get('audio')!r} for U, {groups.get('files')!r} for Cmd+E, 'the host owns this' for Cmd+S and Cmd+W, "
               f"also with a field in focus); the page keys, the keys of no command and a letter typed into the field showed none" if text_ok else
               f"the text of the key of an absent command is missing or wrong: {bad_text}")
    # --- field: no key reaches the host, the field takes its characters (also Option+L, Option+5, Option+6 of a German layout) and undoes
    field_steps = [t[0] for t in HK_TABLE if t[0].startswith("field-")]
    in_host = hk_step_keys(krec, *field_steps)
    val = {st: (by_step.get(st) or {}).get("value") for st in field_steps}
    want_val = {"field-letter": "u", "field-space": "u ", "field-option-at": "u @", "field-option-lbracket": "u @[", "field-option-rbracket": "u @[]", "field-delete": "u @["}
    off = {st: (val.get(st), w) for st, w in want_val.items() if val.get(st) != w}
    undone = val.get("field-undo") is not None and val.get("field-undo") != "u @["
    field_ok = not in_host and not off and undone and rep.get("host_mode") is True
    field_ev = (f"with a text field of the page in focus {len(field_steps)} keys were posted (letter, space, Cmd+E, Cmd+J, Option+L, Option+5, Option+6, Delete, Cmd+Z): the field took its characters "
                f"('u', 'u ', 'u @', 'u @[', 'u @[]'), Delete took the last one, Cmd+Z undid the typing (value {val.get('field-undo')!r}); 0 key events reached the host" if field_ok else
                f"with a text field in focus a key reached the host or the field did not behave: {len(in_host)} host events ({'; '.join(hk_show(k) for k in in_host[:4])}), "
                f"values off (got, want) {off}, Cmd+Z {'undid' if undone else 'did not undo'} the typing (value {val.get('field-undo')!r})")
    # --- focus: a REAL click outside gives the keys to the host (its views refuse the first responder), a real click in the page takes them back
    fr = {x["step"]: x["class"] for x in krec.get("first_responder", [])}
    given = [k for k in krec.get("seen", []) if k["step"] == "given-back" and k["kind"] != "keyUp"]
    taken = hk_step_keys(krec, "taken")
    page_q = [k for k in keys if k["code"] == "KeyQ"]
    web_after_in = "WebView" in fr.get("click-in", "")
    web_after_out = "WebView" in fr.get("click-out", "")
    focus_ok = bool(given) and not taken and len(page_q) == 1 and not web_after_out and web_after_in
    focus_ev = (f"a real mouse click outside the plugin (the host's view refuses the first responder) took the keys from the page: the first responder was {fr.get('click-out')}, the key Q reached the "
                f"host ({given[0]['via']}) and the page saw 0; a real click in the page made the web view the first responder again ({fr.get('click-in')}) and the same key reached the page ({len(page_q)}), the host 0 times"
                if focus_ok else
                f"the keys did not follow the clicks: after the click outside the first responder was {fr.get('click-out')!r} and Q reached the host {len(given)} time(s) (want 1); after the click in the page the first "
                f"responder was {fr.get('click-in')!r} and Q reached the host {len(taken)} time(s) (want 0) and the page {len(page_q)} time(s) (want 1)")
    # --- undo: the page really undid the hit it had added
    add, und = by_step.get("page-add-hit", {}), by_step.get("page-key-cmd", {})
    undo_ok = (add.get("undo0") == 0 and add.get("undo") == 1 and und.get("undo0") == 1 and und.get("undo") == 0 and und.get("redo") == 1
               and und.get("prevented") is True)
    undo_ev = (f"the key H added a hit (the page's undo stack 0 -> 1) and Cmd+Z undid it (stack 1 -> 0, redo stack 0 -> 1, message {und.get('msg')!r}): the page ran its own command on the key, the host's undo ran 0 times"
               if undo_ok else f"the page did not undo its hit: add {add.get('undo0')}->{add.get('undo')}, undo {und.get('undo0')}->{und.get('undo')} redo {und.get('redo0')}->{und.get('redo')}")
    # --- tab: Tab at the last element of the page does not leave the plugin
    tab_after = [fr.get(st, "") for st in ("field-tab-4", "after-tab")]
    after = by_step.get("after-tab", {})
    tab_ok = all("WebView" in c for c in tab_after) and after.get("code") == "KeyY" and not hk_step_keys(krec, "field-tab", "field-tab-2", "field-tab-3", "field-tab-4", "after-tab")
    tab_ev = (f"Four Tab keys in the text field, which is the last element of the page, left the focus in the plugin (first responder {tab_after[0]} after the fourth, {tab_after[1]} after the next key); the next key went to the page, not to the host"
              if tab_ok else f"the Tab key left the plugin: first responder after Tab {tab_after[0]!r}, after the next key {tab_after[1]!r}; host events at those steps {len(hk_step_keys(krec, 'field-tab', 'field-tab-2', 'field-tab-3', 'field-tab-4', 'after-tab'))}")
    # --- action: a menu action of the host with no target still reaches the host's views while the plugin has the focus
    sent = [x for x in krec.get("first_responder", []) if x["step"] == "menu-action-sent"]
    action_seen = hk_step_keys(krec, "menu-action")
    action_ok = bool(action_seen) and action_seen[0]["title"] == "Host Save" and bool(sent) and sent[0]["class"] == "true"
    action_ev = ("a menu action with no target ('Host Save', sent down the responder chain from the first responder, the web view, as the application does for a menu item) reached the host's view; "
                 "the shield answers an action only while a fresh key event of the plugin is being handled" if action_ok else
                 f"the menu action did not reach the host's view while the plugin has the focus: sent {[x['class'] for x in sent]}, host records {len(action_seen)}: the shield swallowed it")
    # --- slow: a key that the page does not use, while the main thread of the host is blocked for 400 ms; the commands that WebKit derives
    # from it (delete a word, move a word) must not reach a host view above the plugin
    acts = [k for k in krec.get("seen", []) if k["via"] == "action"]
    slow_keys = hk_step_keys(krec, "slow-delete", "slow-move")
    slow_page = [by_step.get(st) for st in ("slow-delete", "slow-move")]
    slow_ok = not acts and not slow_keys and all(slow_page)
    slow_ev = ("two keys the page does not use (Option+Delete, Option+Left) were posted, each followed by a 400 ms stall of the host's main thread: the page saw both, and no editing command "
               "(deleteWordBackward:, moveWordLeft:, cancelOperation:, deleteBackward:) reached the host view above the plugin" if slow_ok else
               f"an editing command or a key reached the host after a stalled main thread: commands {[k['title'] for k in acts]}, keys {[hk_show(k) for k in slow_keys]}, page saw {[bool(x) for x in slow_page]}: the command was not taken, it reached the host")
    # --- modifier: Command down outside (the host has it), a click in the page, Command up: the host must get the release
    mod_down = [k for k in krec.get("seen", []) if k["step"] == "mod-down" and k["kind"] == "flagsChanged"]
    mod_up = [k for k in krec.get("seen", []) if k["step"] == "mod-up" and k["kind"] == "flagsChanged"]
    mod_ok = bool(mod_down) and bool(mod_up) and (mod_up[0]["flags"] & HK_FLAG_CMD) == 0 and mod_up[0]["via"] == "view"
    mod_ev = ("Command went down while the host had the keys (the host window got it), the click in the page took the focus, and the release of Command went up the responder chain to a host VIEW above the plugin (not only the window): the host does not keep a stuck "
              "modifier" if mod_ok else
              f"the host did not get the release of Command after a click in the plugin: modifier down seen {len(mod_down)}, modifier up seen {len(mod_up)} (via {[k['via'] for k in mod_up]}): the release did not reach the host")
    # --- overlay: a host view over a part of the plugin is outside it; the click on it gives the keys back
    over_given = [k for k in krec.get("seen", []) if k["step"] == "overlay-key" and k["kind"] != "keyUp"]
    over_ok = bool(over_given) and "WebView" not in fr.get("click-overlay", "WebView") and "WebView" in fr.get("taken", "")
    over_ev = (f"a click on a host view that lies over a part of the plugin took the keys from the page (first responder {fr.get('click-overlay')}), the next key reached the host"
               if over_ok else f"the click on the overlay did not give the keys back: first responder {fr.get('click-overlay')!r}, host got the key {len(over_given)} time(s): the overlay counted as inside the plugin")
    return dict(zip(names, [(cls_ok, cls_ev), (rec_ok, rec_ev), (sys_ok, sys_ev), (space_ok, space_ev), (text_ok, text_ev), (field_ok, field_ev),
                            (focus_ok, focus_ev), (undo_ok, undo_ev), (tab_ok, tab_ev), (action_ok, action_ev), (slow_ok, slow_ev), (mod_ok, mod_ev),
                            (over_ok, over_ev)]))


def stage_editor_keys() -> bool:
    """M1b12 gate lines 2, 3, 4, 5 and 6: with the focus in the plugin view no key reaches the host except the exception
    list; the space bar plays through the page; an absent command's key shows its group text; a click outside gives
    the keys back. Real key events posted to the window of the test host, on the CLAP and on the CLAP in the VST3."""
    ok = True
    for which, bundle, suffix in (("clap", CLAP, ""), ("vst3", EMBEDDED_CLAP, "-vst3")):
        rc, _s, rows, rec, krec = hostkeys_run(f"editor-keys-{which}", bundle)
        ok = print_parts(suffix, hostkeys_parts(rc, rows, rec, krec)) and ok
    return ok


def stage_editor_keys_fault() -> bool:
    ok = True
    for label, env, part, what, reason in (
        ("pass", {"DS_ARA_EDITOR_FAULT": "keys-pass"}, "editor-keys-host-record", "the rule is off: every key goes on to the host", "reached the host"),
        ("pass-field", {"DS_ARA_EDITOR_FAULT": "keys-pass"}, "editor-keys-field", "the rule is off: a key typed with a field in focus goes on", "reached the host"),
        ("pass-focus", {"DS_ARA_EDITOR_FAULT": "keys-pass"}, "editor-keys-focus", "the rule is off: the key after a click in the page goes on", "did not follow the clicks"),
        ("no-shield", {"DS_ARA_EDITOR_FAULT": "no-shield"}, "editor-keys-host-record", "the shield passes an unused key on to the host's view", "reached the host"),
        ("no-monitor", {"DS_ARA_EDITOR_FAULT": "no-monitor"}, "editor-keys-host-record", "no monitor: a key with Command goes to the host's menu first", "reached the host"),
        ("no-focus", {"DS_ARA_EDITOR_FAULT": "no-focus"}, "editor-keys-host-record", "the web view is not the first responder: the keys go to the host's view", "reached the host"),
        ("no-click-out", {"DS_ARA_EDITOR_FAULT": "no-click-out"}, "editor-keys-focus", "a click outside the plugin does not take the first responder from the web view", "did not follow the clicks"),
        ("no-repeat-guard", {"DS_ARA_EDITOR_FAULT": "no-repeat-guard"}, "editor-keys-space", "a held space bar repeats its key down to the page", "did not behave"),
        ("no-field-undo", {"DS_ARA_EDITOR_FAULT": "no-field-undo"}, "editor-keys-field", "Cmd+Z in a text field does not undo", "did not undo"),
        ("old-freshness", {"DS_ARA_EDITOR_FAULT": "old-freshness"}, "editor-keys-slow", "the 0.25 s rule counted from the creation of the event (round 2)", "it reached the host"),
        ("no-forward-modifier", {"DS_ARA_EDITOR_FAULT": "no-forward-modifier"}, "editor-keys-modifier", "the plugin takes a key up and a modifier change and does not give them to the host", "did not reach the host"),
        ("rect-inside", {"DS_ARA_EDITOR_FAULT": "rect-inside"}, "editor-keys-overlay", "the click is inside by the rectangle of the shield", "counted as inside"),
        ("swallow-actions", {"DS_ARA_EDITOR_FAULT": "swallow-actions"}, "editor-keys-action", "the shield answers every menu action of the host", "swallowed it"),
        ("no-system", {"DS_ARA_EDITOR_FAULT": "no-system-keys"}, "editor-keys-system", "the system keys are taken too", "did not reach the host"),
        ("no-text", {"DS_ARA_EDITOR_FAULT": "no-key-text"}, "editor-keys-text", "the page does not show the text of an absent command's key", "missing or wrong"),
        ("space", {"DS_ARA_PLAYBACK_FAULT": "skip"}, "editor-keys-space", "the plugin makes no call to the host and says it did", "no play request"),
    ):
        rc, _s, rows, rec, krec = hostkeys_run(f"editor-fault-keys-{label}", CLAP, env)
        ok = fault_reason(f"editor-keys-fault-{label}", hostkeys_parts(rc, rows, rec, krec), part, what, reason) and ok
    parts = hostkeys_parts(2, [], {}, {})
    ok = pass_fail("editor-keys-fault-norun", all((not g) and "no usable run" in ev for g, ev in parts.values()),
                   f"a run with no page report and no host key record fails all {len(parts)} parts with 'no usable run'") and ok
    return ok


def twin_parts(rc: int, rows_a: list, rows_b: list, summary: dict) -> dict:
    ready_a = [r for r in rows_a if r.get("event") == "page_ready"]
    ready_b = [r for r in rows_b if r.get("event") == "page_ready"]
    names = [r.get("class") for rows in (rows_a, rows_b) for r in rows if r.get("event") == "key_shield"]
    return {
        "editor-keys-twin-open": (
            rc == 0 and len(ready_a) == 1 and len(ready_b) == 1,
            f"two copies of the plugin in ONE process (the CLAP, then the CLAP of the VST3 bundle, the first still loaded) each opened an editor and the page was ready: "
            f"{len(ready_a)} and {len(ready_b)} (rc={rc}, {summary})" if rc == 0 and ready_a and ready_b else
            f"the second copy of the plugin did not open its editor: rc={rc}, page ready {len(ready_a)} and {len(ready_b)}, {summary}: the class of the key shield could not be made twice"),
        "editor-keys-twin-names": (
            len(names) == 2 and len(set(names)) == 2 and all(str(n).startswith("DrumSpikeAraKeyShield_") for n in names),
            f"each copy made its own key shield class: {names}"),
    }


def stage_editor_keys_twin() -> bool:
    """M1b12 K3: the Objective-C class of the key shield has a name that is unique in the process. Two copies of the
    plugin (the CLAP and the CLAP inside the VST3 bundle) in one test host process both open an editor."""
    ok = True
    for label, env in (("editor-keys-twin", {}),):
        rc, summary, rows_a = editor_run(label, "ready", CLAP, 1, env, ["--twin", str(EMBEDDED_CLAP), "--twin-out", str(OUT / f"report-{label}-b.jsonl"),
                                                                       "--twin-cache", str(OUT / f"cache-{label}-b")], timeout=600)
        rows_b = report_rows(f"{label}-b")
        ok = print_parts("", twin_parts(rc, rows_a, rows_b, summary)) and ok
    return ok


def stage_editor_keys_twin_fault() -> bool:
    rc, summary, rows_a = editor_run("editor-fault-keys-twin", "ready", CLAP, 1, {"DS_ARA_EDITOR_FAULT": "fixed-class-name"},
                                     ["--twin", str(EMBEDDED_CLAP), "--twin-out", str(OUT / "report-editor-fault-keys-twin-b.jsonl"),
                                      "--twin-cache", str(OUT / "cache-editor-fault-keys-twin-b")], timeout=600)
    rows_b = report_rows("editor-fault-keys-twin-b")
    return fault_reason("editor-keys-twin-fault", twin_parts(rc, rows_a, rows_b, summary), "editor-keys-twin-open",
                        "the key shield class has one fixed name", "did not open its editor")



# ---- M1c1 (#560): the delivery: the export in song time, the folder rule and the file drag

DELIVERY_REGION = (0.5, 2.0, 10.0)   # every source: the slice [0.5, 2.5) of the source sits at song 10 s
DELIVERY_RATES = {safe_name(stem_of(n)): r for n, r, *_ in SPEC}


def midi_note_times(path: Path) -> list:
    """The times in seconds of the note-ons of a standard MIDI file, through its own tempo events."""
    return midi_parse(path)[0]


def midi_parse(path: Path) -> tuple:
    """`(note times in seconds, tempo events in microseconds per quarter, time signatures)` of a standard MIDI file."""
    b = path.read_bytes()
    assert b[:4] == b"MThd"
    ppq = int.from_bytes(b[12:14], "big")
    pos, tempi, notes, sigs = 14, [(0, 500000)], [], []
    while pos < len(b):
        assert b[pos:pos + 4] == b"MTrk"
        n = int.from_bytes(b[pos + 4:pos + 8], "big")
        i, end, tick, status = pos + 8, pos + 8 + n, 0, 0
        while i < end:
            d = 0
            while True:
                c = b[i]
                i += 1
                d = (d << 7) | (c & 0x7F)
                if not c & 0x80:
                    break
            tick += d
            if b[i] == 0xFF:
                kind, ln = b[i + 1], b[i + 2]
                if kind == 0x51:
                    tempi.append((tick, int.from_bytes(b[i + 3:i + 3 + ln], "big")))
                if kind == 0x58:
                    sigs.append((tick, b[i + 3], 1 << b[i + 4]))
                i += 3 + ln
                continue
            if b[i] & 0x80:
                status = b[i]
                i += 1
            hi = status & 0xF0
            if hi in (0xC0, 0xD0):
                i += 1
            else:
                if hi == 0x90 and b[i + 1] > 0:
                    notes.append(tick)
                i += 2
        pos = end
    tempi.sort()
    out = []
    for nt in notes:
        t, t0, us = 0.0, 0, tempi[0][1]
        for tk, u in tempi[1:]:
            if tk >= nt:
                break
            t += (tk - t0) * us / (ppq * 1e6)
            t0, us = tk, u
        out.append(t + (nt - t0) * us / (ppq * 1e6))
    return sorted(out), [u for _, u in tempi[1:]], sigs


def wav_spikes(path: Path) -> dict:
    """Rate, length, the `bext` time reference and the first sample of each spike of a spike WAV."""
    import struct
    b = path.read_bytes()
    pos, fmt, tref, data = 12, None, None, b""
    while pos + 8 <= len(b):
        cid, n = b[pos:pos + 4], struct.unpack("<I", b[pos + 4:pos + 8])[0]
        body = b[pos + 8:pos + 8 + n]
        if cid == b"fmt ":
            fmt = struct.unpack("<HHIIHH", body[:16])
        elif cid == b"bext":
            tref = struct.unpack("<II", body[338:346])
            tref = tref[0] | (tref[1] << 32)
        elif cid == b"data":
            data = body
        pos += 8 + n + (n & 1)
    tag, ch, rate, _, _, bits = fmt
    step = bits // 8
    if tag == 3 or (tag == 0xFFFE and bits == 32 and False):
        xs = struct.unpack(f"<{len(data) // 4}f", data)
    else:
        xs = [int.from_bytes(data[i:i + step], "little", signed=True) / float(1 << (bits - 1)) for i in range(0, len(data) - step + 1, step * ch)]
    starts, quiet = [], 10 ** 9
    for i, x in enumerate(xs):
        if abs(x) > 0.05 and quiet >= 200:
            starts.append(i)
        quiet = 0 if abs(x) > 0.05 else quiet + 1
    return {"rate": rate, "frames": len(xs) // (ch if tag == 3 else 1), "tref": tref, "starts": starts}


def wav_samples(path: Path) -> dict:
    """Rate, channels, the `bext` time reference and the first channel of a WAV file as floats (PCM 16/24/32 or float 32)."""
    import struct
    b = path.read_bytes()
    pos, fmt, tref, data = 12, None, None, b""
    while pos + 8 <= len(b):
        cid, n = b[pos:pos + 4], struct.unpack("<I", b[pos + 4:pos + 8])[0]
        body = b[pos + 8:pos + 8 + n]
        if cid == b"fmt ":
            fmt = struct.unpack("<HHIIHH", body[:16])
        elif cid == b"bext":
            t = struct.unpack("<II", body[338:346])
            tref = t[0] | (t[1] << 32)
        elif cid == b"data":
            data = body
        pos += 8 + n + (n & 1)
    tag, ch, rate, _, _, bits = fmt
    step = bits // 8
    if tag == 3:
        raw = struct.unpack(f"<{len(data) // 4}f", data)
        xs = list(raw[0::ch])
    else:
        xs = [int.from_bytes(data[i:i + step], "little", signed=True) / float(1 << (bits - 1)) for i in range(0, len(data) - step + 1, step * ch)]
    return {"rate": rate, "ch": ch, "tref": tref, "xs": xs}


def song_delivery_run(label: str, env: dict | None = None, pressed: bool = True, region: tuple | None = DELIVERY_REGION, shift: float | None = None, extra: list | None = None) -> dict:
    out_dir = OUT / f"delivery-out-{label}"
    if out_dir.exists():
        shutil.rmtree(out_dir)
    out_dir.mkdir(parents=True)
    (out_dir / "sub").mkdir()   # a folder that exists below the chosen one: the rule must still refuse it
    e = {"DS_ARA_PICK_DIR": str(out_dir)}
    if pressed:
        e["DS_ARA_DRAG_PRESSED"] = "1"
    e.update(env or {})
    args = ["--regions-all", ",".join(str(x) for x in region)] if region else ["--no-regions"]
    if shift is not None:   # the host moves every region in playback time after the page has booted: no new analysis round
        args += ["--shift-region", *[f"{i}={shift}" for i in range(4)]]
    rc, summary, rows = run_phase(f"delivery-{label}", "delivery", CLAP, e, args + (extra or []) + ["--arg", str(OUT)])
    reps = page_reports(rows, "delivery")
    return {"rc": rc, "rows": rows, "rep": reps[0] if reps else {}, "dir": out_dir.resolve(), "summary": summary}


def delivery_parts(d: dict) -> dict:
    r, rows, folder = d["rep"], d["rows"], d["dir"]
    parts: dict = {}
    names = r.get("names") or []
    parts["delivery-ran"] = (d["rc"] == 0 and bool(r) and len(names) == 4 and "prepare_error" not in r,
                             f"the host ran the delivery driver (rc={d['rc']}), {len(names)} tracks, {r.get('prepare_error')}")
    parts["delivery-notempo"] = ((r.get("tempo") or {}).get("source") == "none" and (r.get("tempo") or {}).get("bpm") == 120,
                                 f"no tempo from the test host: the page is told {r.get('tempo')}")
    parts["delivery-before-choice"] = ((r.get("before_choice") or {}).get("code") == 403,
                                       f"a dest outside the document folder before any choice: {r.get('before_choice')}")
    parts["delivery-folder"] = (r.get("folder") is not None and Path(str(r.get("folder"))).resolve() == folder,
                                f"the sheet (answered by DS_ARA_PICK_DIR) gave {r.get('folder')}")
    parts["delivery-dest-rule"] = (((r.get("subfolder") or {}).get("code") == 403) and ((r.get("other") or {}).get("code") == 403),
                                   f"a subfolder of the chosen folder: {r.get('subfolder')}; another folder: {r.get('other')}")
    ex = r.get("exported") or {}
    files = [Path(f) for f in ex.get("files", [])]
    mids = [f for f in files if f.suffix == ".mid"]
    wavs = [f for f in files if f.suffix == ".wav"]
    parts["delivery-files"] = (len(mids) == 4 and len(wavs) == 4 and all(f.parent.resolve() == folder and f.is_file() for f in files)
                               and not ex.get("errors") and "outside_song" in ex,
                               f"{len(mids)} MIDI and {len(wavs)} WAV files in the chosen folder, errors {ex.get('errors')}")
    bad_midi, bad_spike, bad_ref, n_notes, n_spikes = [], [], [], 0, 0
    region = d.get("region")
    for key in names:
        stem = (r.get("display") or {}).get(key, key)
        rate = DELIVERY_RATES.get(key, 48000)
        times = [e["t"] for e in (r.get("events") or {}).get(key, [])]
        if region:
            want = sorted(region[2] + (t - region[0]) for t in times if region[0] <= t < region[0] + region[1])
        else:
            want = sorted(times)
        mid, wav = folder / f"{stem}_spike.mid", folder / f"{stem}_spike.wav"
        if not (mid.is_file() and wav.is_file()):
            bad_midi.append(f"{stem}: no file")
            continue
        got = midi_note_times(mid)
        n_notes += len(got)
        if len(got) != len(want) or any(abs(g - w) * rate > 1.0 for g, w in zip(got, sorted(want))):
            bad_midi.append((stem, len(got), len(want), [round((g - w) * rate, 2) for g, w in zip(got, want)][:6]))
        sp = wav_spikes(wav)
        n_spikes += len(sp["starts"])
        idx = [round(w * sp["rate"]) for w in want]
        if sp["starts"] != idx:
            bad_spike.append((stem, sp["starts"][:6], idx[:6]))
        if sp["tref"] != 0:
            bad_ref.append((stem, sp["tref"]))
    parts["delivery-midi-positions"] = (not bad_midi and n_notes > 0,
                                        f"{n_notes} note-ons within 1 sample of the song position of their hit; differences: {bad_midi}")
    parts["delivery-spike-positions"] = (not bad_spike and n_spikes > 0,
                                         f"{n_spikes} spikes at 0 samples difference from the song position; differences: {bad_spike}")
    parts["delivery-time-reference"] = (not bad_ref and n_spikes > 0, f"the bext time reference of every spike file is 0; differences: {bad_ref}")
    # M1c2 (#579): the audio spikes are in song time as the synthetic ones: same file length and rate, time reference 0, and each snippet is
    # the source audio of its hit (a mono source: sample for sample) at the song sample of the hit; every sample outside the snippets is zero
    ea = r.get("exported_audio") or {}
    src_of = {safe_name(stem_of(n)): FIXTURES / n for n, *_ in SPEC}
    bad_audio, n_snip = [], 0
    for key in names:
        stem = (r.get("display") or {}).get(key, key)
        rate = DELIVERY_RATES.get(key, 48000)
        aw, sw = folder / f"{stem}_audio_spike.wav", folder / f"{stem}_spike.wav"
        if not (aw.is_file() and sw.is_file()):
            bad_audio.append(f"{stem}: no file")
            continue
        a, sp = wav_samples(aw), wav_samples(sw)
        if (a["rate"], len(a["xs"]), a["tref"]) != (sp["rate"], len(sp["xs"]), 0):
            bad_audio.append((stem, "rate/length/time reference", a["rate"], len(a["xs"]), a["tref"], sp["rate"], len(sp["xs"])))
            continue
        src = wav_samples(src_of[key]) if key in src_of else None
        pairs = []
        for t in [e["t"] for e in (r.get("events") or {}).get(key, [])]:
            if region:
                if region[0] <= t < region[0] + region[1]:
                    pairs.append((t, region[2] + (t - region[0])))
            else:
                pairs.append((t, t))
        covered = [False] * len(a["xs"])
        window = int(0.061 * rate) + 2
        for ts, tg in pairs:
            i, j = round(ts * rate), round(tg * rate)
            for k in range(j, min(j + window, len(covered))):
                covered[k] = True
            n_snip += 1
            seg = a["xs"][j:j + window]
            if not any(seg):
                bad_audio.append((stem, "no snippet at", j))
            elif src and src["ch"] == 1 and any(abs(x - y) > 1e-6 for x, y in zip(a["xs"][j:j + 40], src["xs"][i:i + 40])):
                bad_audio.append((stem, "snippet is not the source audio of the hit", j, i))
        stray = sum(1 for x, c in zip(a["xs"], covered) if x != 0.0 and not c)
        if stray:
            bad_audio.append((stem, "nonzero samples outside the snippets", stray))
    parts["delivery-audio-positions"] = (not bad_audio and n_snip > 0 and not ea.get("errors") and ea.get("audio") == len(names),
                                         f"{n_snip} audio snippets at their song samples, the source audio of their hit, exact zero elsewhere, the length, rate and time reference 0 of the "
                                         f"synthetic files; {ea.get('audio')} audio files; differences: {bad_audio[:4]}")
    single = r.get("exported_single") or {}
    if region and d.get("check_left"):
        # a trim in the host starts a new analysis round (M1b4), so the hits are those of the live slice: every note of the files lies inside
        # the slice at its song position, and the trim left fewer hits than the session had before it
        before = sum(len(v) for v in (r.get("events_before") or {}).values())
        lo, hi = region[2], region[2] + region[1]
        notes = [t for k in names for t in midi_note_times(folder / f"{(r.get('display') or {}).get(k, k)}_spike.mid")]
        parts["delivery-trim-range"] = (bool(notes) and all(lo - 1e-4 <= t <= hi + 1e-4 for t in notes) and len(notes) < before,
                                        f"{len(notes)} notes of the trimmed files (of {before} hits before the trim) all lie in the live slice, song {lo} to {hi} s")
    parts["delivery-single-file"] = (bool(single.get("midi_file")) and Path(single["midi_file"]).is_file(),
                                     f"the one multi-track file: {single.get('midi_file')}")
    taken = [Path(f) for f in (r.get("taken") or {}).get("files", [])]
    started = [x for x in rows if x.get("event") == "drag_started"]
    pb = {str(Path(p).resolve()) for x in started for p in x.get("pasteboard", [])}
    # a gate run starts NO system drag session (round 3 of #579): the plugin runs the guards, makes the items, writes them to a private
    # pasteboard and reads them back; every row says session "test". A real drag into a DAW is a hand check.
    parts["delivery-drag-pasteboard"] = (r.get("drag_ok") is True and len(taken) == 8 and bool(started)
                                         and pb == {str(f.resolve()) for f in taken} and all(x.get("session") == "test" for x in started),
                                         f"the drag command ran its guards and made the items (ok={r.get('drag_ok')} {r.get('drag_error')}); the private pasteboard holds "
                                         f"{len(pb)} file URLs, the {len(taken)} files of one MIDI and one WAV per track; sessions {sorted({x.get('session') for x in started})} (no system session)")
    parts["delivery-drag-guard"] = ((r.get("drag_outside") or {}).get("code") == 403,
                                    f"a drag of a file outside Exports/: {r.get('drag_outside')}")
    closes = [x for x in rows if x.get("event") == "editor_close"]
    parts["delivery-main-thread"] = (bool(closes) and closes[-1].get("off_main") == 0 and closes[-1].get("command_errors", 0) >= 0,
                                     f"the sheet and the drag ran on the main thread: off_main {closes[-1].get('off_main') if closes else None}")
    return parts


def ara_parts(d: dict, want_ara: bool) -> dict:
    """E2 of round 3: the content access controller of the test host (two tempos, two bar signatures) or one with no tempo content."""
    r, rows = d["rep"], d["rows"]
    c = (d.get("summary") or {}).get("content") or {}
    row = next((x for x in rows if x.get("event") == "musical_context" and x.get("controller") is True), {})
    t = r.get("tempo") or {}
    parts: dict = {}
    if want_ara:
        parts["delivery-ara-source"] = (t.get("source") == "ara" and t.get("steps") == 1 and t.get("bpm") == 120 and row.get("tempo_available") is True
                                        and row.get("entries") == 3 and row.get("bars") == 2,
                                        f"the page is told {t}; the row musical_context: entries {row.get('entries')}, bars {row.get('bars')}, reason {row.get('reason')!r}")
        parts["delivery-ara-reader"] = (c.get("created", 0) > 0 and c.get("created") == c.get("destroyed") and c.get("events") == 5 * (c.get("created", 0) // 2) and c.get("off_main") == 0,
                                        f"the plugin made {c.get('created')} readers and gave back {c.get('destroyed')}, read {c.get('events')} events, {c.get('off_main')} calls off the main thread")
        folder = d["dir"]
        mids = sorted(folder.glob("*_spike.mid"))
        tempi, sigs, bad = [], [], []
        for m in mids:
            _, tempi, sigs = midi_parse(m)
            if tempi != [500000, 600000] or sorted({(n, dn) for _, n, dn in sigs}) != [(3, 4), (4, 4)]:
                bad.append((m.name, tempi, sigs))
        parts["delivery-ara-midi"] = (bool(mids) and not bad,
                                      f"{len(mids)} MIDI files carry the host map: tempo events 500000 and 600000 microseconds per quarter and the signatures 4/4 and 3/4; differences {bad}")
    else:
        parts["delivery-ara-fallback"] = (t.get("source") != "ara" and row.get("tempo_available") is False and row.get("reason") == "no-tempo-content"
                                          and c.get("available", 0) >= 1 and c.get("created") == 0 and c.get("events") == 0,
                                          f"a controller with no tempo content: source {t.get('source')}, reason {row.get('reason')!r}, {c.get('available')} availability questions, {c.get('created')} readers")
    return parts


def _with_region(d: dict) -> dict:
    d["region"] = DELIVERY_REGION
    return d


def delivery_ara_parts() -> bool:
    ok = print_parts("-ara", ara_parts(dd := _with_region(song_delivery_run("clap-ara", extra=["--musical", "tempo"])), True) | {k: v for k, v in delivery_parts(dd).items() if k in ("delivery-midi-positions", "delivery-spike-positions")})
    ok = print_parts("-ara-none", ara_parts(song_delivery_run("clap-ara-none", extra=["--musical", "no-tempo"]), False)) and ok
    return ok


def stage_delivery() -> bool:
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("delivery", False, "no host or bundle: run check.sh build")
    d = song_delivery_run("clap")
    d["region"] = DELIVERY_REGION
    ok = print_parts("", delivery_parts(d))
    # D1 of the review: a clip moved in the host (playback time only, no new round) exports at its NEW song position
    dm = song_delivery_run("clap-moved", shift=30.0)
    dm["region"] = (DELIVERY_REGION[0], DELIVERY_REGION[1], 30.0)
    ok = print_parts("-moved", delivery_parts(dm), ["delivery-ran", "delivery-midi-positions", "delivery-spike-positions", "delivery-audio-positions"]) and ok
    # E1 of round 3: a region trimmed in the host exports through its LIVE slice
    dt = song_delivery_run("clap-trimmed", extra=["--trim-region", *[f"{i}=1.0,1.0" for i in range(4)]])
    dt["region"] = (1.0, 1.0, 10.5)
    dt["check_left"] = True
    ok = print_parts("-trimmed", delivery_parts(dt), ["delivery-ran", "delivery-midi-positions", "delivery-spike-positions", "delivery-audio-positions", "delivery-trim-range"]) and ok
    ok = delivery_ara_parts() and ok
    d2 = song_delivery_run("clap-noregion", region=None)
    d2["region"] = None
    ok = print_parts("-no-region", delivery_parts(d2), ["delivery-ran", "delivery-midi-positions", "delivery-spike-positions", "delivery-audio-positions"]) and ok
    return ok


def stage_delivery_release() -> bool:
    """No mouse button down: the drag command refuses (no items made)."""
    d = song_delivery_run("release", pressed=False)
    r = d["rep"]
    started = [x for x in d["rows"] if x.get("event") == "drag_started"]
    return pass_fail("delivery-drag-release", r.get("drag_ok") is False and (r.get("drag_error") or {}).get("code") == 409 and not started,
                     f"with no button down the drag answered {r.get('drag_error')} and no items were made ({len(started)} rows)")


def stage_delivery_fault() -> bool:
    ok = True
    for label, env, pressed, part, what in (
        ("source-time", {"DS_ARA_DELIVERY_FAULT": "source-time"}, True, "delivery-midi-positions", "the files keep the time of the source"),
        ("any-dest", {"DS_ARA_EDITOR_FAULT": "any-dest"}, True, "delivery-before-choice", "a dest outside the document folder is written without a choice"),
        ("prefix-dest", {"DS_ARA_EDITOR_FAULT": "prefix-dest"}, True, "delivery-dest-rule", "a subfolder of the chosen folder passes"),
        ("any-path", {"DS_ARA_EDITOR_FAULT": "any-path"}, True, "delivery-drag-guard", "a drag takes a path outside Exports/"),
        ("invent-map", {"DS_ARA_DELIVERY_FAULT": "invent-map"}, True, "delivery-notempo", "the plugin invents a tempo map"),
    ):
        d = song_delivery_run(f"fault-{label}", env, pressed)
        d["region"] = DELIVERY_REGION
        ok = fault_part(f"delivery-fault-{label}", delivery_parts(d), part, what) and ok
        if label == "source-time":
            ok = fault_part("delivery-fault-source-time-audio", delivery_parts(d), "delivery-audio-positions", "the audio spikes keep the time of the source") and ok
    d = song_delivery_run("fault-stale-regions", {"DS_ARA_DELIVERY_FAULT": "stale-regions"}, shift=30.0)
    d["region"] = (DELIVERY_REGION[0], DELIVERY_REGION[1], 30.0)
    ok = fault_part("delivery-fault-stale-regions", delivery_parts(d), "delivery-midi-positions", "the song position of the last round is used: a moved clip exports at its old place") and ok
    ok = fault_part("delivery-fault-stale-regions-audio", delivery_parts(d), "delivery-audio-positions", "a moved clip exports its audio spikes at the old place") and ok
    d = song_delivery_run("fault-keep-reader", {"DS_ARA_DELIVERY_FAULT": "keep-reader"}, extra=["--musical", "tempo"])
    ok = fault_part("delivery-fault-keep-reader", ara_parts(d, True), "delivery-ara-reader", "the plugin does not give the reader back") and ok
    d = song_delivery_run("fault-no-button-check", {"DS_ARA_EDITOR_FAULT": "no-button-check"}, pressed=False)
    r = d["rep"]
    ok = pass_fail("delivery-fault-no-button-check", r.get("drag_ok") is True,
                   f"the fault run (no check of the mouse button) makes the drag items with no button down (a test session, no system drag): ok={r.get('drag_ok')}") and ok
    d = song_delivery_run("fault-test-row-real", {"DS_ARA_EDITOR_FAULT": "test-row-real"})
    d["region"] = DELIVERY_REGION
    ok = fault_part("delivery-fault-test-row-real", delivery_parts(d), "delivery-drag-pasteboard", "a drag row that says session real (the check that no gate run starts a system session can fail; the fault only changes the row)") and ok
    return ok


def export_dialog_run(label: str, env: dict | None = None) -> dict:
    """M1c2 (#579): the REAL Export dialog in the REAL page of the plugin, driven from the host-mode menu (mode exportdialog), in two
    editors of one document (`--cycles 2`). The sheet is answered by DS_ARA_PICK_DIR with a folder below out/."""
    pick = OUT / f"exdialog-pick-{label}"
    if pick.exists():
        shutil.rmtree(pick)
    pick.mkdir(parents=True)
    pick = pick.resolve()
    e = {"DS_ARA_PICK_DIR": str(pick), "DS_ARA_DRAG_PRESSED": "1", "DS_ARA_EDITOR_SCENARIO": "rm-exports"}   # rm-exports: Exports/ is gone at the first open
    e.update(env or {})
    rc, summary, rows = editor_run(f"exdialog-{label}", "exportdialog", CLAP, 2, e,
                                   ["--regions-all", ",".join(str(x) for x in DELIVERY_REGION), "--arg", str(pick)], timeout=400)
    return {"rc": rc, "summary": summary, "rows": rows, "reps": page_reports(rows, "exportdialog"), "pick": pick,
            "cache": (OUT / f"cache-exdialog-{label}").resolve()}


def export_dialog_parts(d: dict) -> dict:
    reps, rows, pick = d["reps"], d["rows"], d["pick"]
    parts: dict = {}
    r1 = reps[0] if reps else {}
    r2 = reps[1] if len(reps) > 1 else {}

    def step(r: dict, label: str) -> dict:
        return next((x for x in r.get("steps", []) if x.get("label") == label), {})

    def files_of(folder: Path) -> tuple[list, list]:
        if not folder.is_dir():
            return [], []
        return sorted(folder.glob("*.mid")), sorted(folder.glob("*.wav"))

    parts["export-dialog-ran"] = (d["rc"] == 0 and len(reps) == 2 and all(r.get("menu") == "ok" and r.get("menu2") == "ok" and "error" not in r for r in reps),
                                  f"the host ran the driver in 2 editors (rc={d['rc']}), {len(reps)} reports; the dialog opened from File > Export...: "
                                  f"{[(r.get('menu'), r.get('menu2')) for r in reps]}")
    default = Path(str(r1.get("def") or "/none"))
    opened = [step(r1, "open"), step(r1, "reopen")]
    parts["export-dialog-no-folder-error"] = (
        all(s.get("open") and s.get("problem") == "" and s.get("dest") == str(default) and s.get("dest_ok") is True and "cannot be used" not in (s.get("problem") or "") for s in opened)
        and default.name == "Exports",
        f"the dialog opens with the default folder and no folder error, at the first open and after a close: {[(s.get('dest'), s.get('problem')) for s in opened]}")
    tile_ok = lambda t, n=4: bool(t) and t.get("state") == "ready" and re.fullmatch(rf"{n} files", t.get("text", "")) is not None
    parts["export-dialog-tiles"] = (all(tile_ok(s.get("midi")) and tile_ok(s.get("spikes")) for s in opened),
                                    f"both drag tiles are ready with a file count: {[(s.get('midi'), s.get('spikes')) for s in opened]}")
    parts["export-dialog-export"] = (
        all(s.get("export_finished") is True and (s.get("result") or "").startswith("Exported 4 MIDI and 4 spike files") for s in (step(r1, "export1"), step(r1, "export2"))),
        f"Export wrote the files at the first open and after a close: {[(step(r1, k).get('export_finished'), step(r1, k).get('result')) for k in ('export1', 'export2')]}")
    mids, wavs = files_of(default)
    made = [x for x in rows if x.get("event") == "default_dest_made"]
    parts["export-dialog-folder-missing"] = (len(made) == 1 and made[0].get("command") in ("export_existing", "plugin_deliver_plan"),   # #589: the replace count of the dialog is the plan command now

                                             f"Exports/ did not exist (the scenario removes the empty folder) when the dialog first named it: the plugin made it once ({len(made)} rows, by {[x.get('command') for x in made]})")
    parts["export-dialog-files"] = (len(mids) == 4 and len(wavs) == 4, f"{len(mids)} MIDI and {len(wavs)} WAV files in {default.name}/ of the document")
    drag = step(r1, "drag")
    started = [x for x in rows if x.get("event") == "drag_started"]
    pb = {str(Path(p).resolve()) for x in started for p in x.get("pasteboard", [])}
    parts["export-dialog-drag"] = (str(drag.get("msg", "")).startswith("dragged 4 files") and bool(started) and all(x.get("session") == "test" for x in started)
                                   and pb and all(Path(p).parent.resolve() in (default.resolve(), (default / "Dragged").resolve()) for p in pb) and len(pb) == 4,   # #589: a take links the prepared files into Exports/Dragged/
                                   f"a press and a move on the Drag MIDI tile ran the drag command up to the items (no system session): message {drag.get('msg')!r}, "
                                   f"{len(started)} test sessions, private pasteboard {len(pb)} files, all in Exports/ or Exports/Dragged/")
    ch = step(r1, "chosen")
    cm, cw = files_of(pick)
    cw = [f for f in cw if not f.name.endswith("_audio_spike.wav")]   # the type Both writes the audio files there too
    parts["export-dialog-choose"] = (str(Path(ch.get("dest", "")).resolve()) == str(pick) and ch.get("problem") == "" and tile_ok(ch.get("midi")) and tile_ok(ch.get("spikes"))
                                     and step(r1, "export3").get("export_finished") is True and len(cm) == 4 and len(cw) == 4,
                                     f"after Choose the dialog shows {ch.get('dest')!r}, no error, ready tiles; Export wrote {len(cm)} MIDI and {len(cw)} WAV files there")
    both = step(r1, "both")
    ex4 = step(r1, "export4")
    aw = [f for f in files_of(pick)[1] if f.name.endswith("_audio_spike.wav")]
    parts["export-dialog-audio"] = (both.get("spike_audio_visible") is True and both.get("spike_both_visible") is True and "later part" not in (both.get("note") or "")
                                    and tile_ok(both.get("midi")) and re.fullmatch(r"8 files", (both.get("spikes") or {}).get("text", "")) is not None
                                    and ex4.get("export_finished") is True and (ex4.get("result") or "").startswith("Exported 4 MIDI and 4 spike and 4 audio spike files")
                                    and len(aw) == 4 and all(wav_samples(f)["tref"] == 0 for f in aw),
                                    f"in the dialog of a DAW the spike types Audio and Both are shown ({both.get('spike_audio_visible')}, {both.get('spike_both_visible')}), "
                                    f"the note has no 'later part', Both gives 8 spike files on the tile and Export writes {len(aw)} audio spike files with time reference 0: {ex4.get('result')!r}")
    o2 = step(r2, "open")
    parts["export-dialog-remembered"] = (str(Path(o2.get("dest", "")).resolve()) == str(pick) and o2.get("problem") == "" and o2.get("dest_ok") is True
                                         and tile_ok(o2.get("midi")) and tile_ok(o2.get("spikes"), 8) and step(r2, "export1").get("export_finished") is True,
                                         f"a new editor of the document opens with the folder chosen before (and the spike type Both that the project keeps, 8 spike files): dest {o2.get('dest')!r}, problem {o2.get('problem')!r}")
    lists = sorted(d["cache"].glob("documents/*/export-folders.json"))
    try:
        listed = [str(Path(x).resolve()) for x in json.loads(lists[0].read_text()).get("folders", [])] if lists else []
    except ValueError:
        listed = []
    inside = sorted(d["cache"].glob("documents/*/project/**/export-folders.json")) + sorted(d["cache"].glob("documents/*/service/**/export-folders.json"))
    parts["export-dialog-list"] = (listed == [str(pick)] and not inside,
                                   f"the plugin keeps the chosen folder in the folder of the document, outside project/ and service/: {listed}, copies inside {len(inside)}")
    closes = [x for x in rows if x.get("event") == "editor_close"]
    parts["export-dialog-main-thread"] = (bool(closes) and closes[-1].get("off_main") == 0,
                                          f"the sheet and the drag ran on the main thread: off_main {closes[-1].get('off_main') if closes else None}")
    return parts


def stage_export_dialog() -> bool:
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("export-dialog", False, "no host or bundle: run check.sh build")
    return print_parts("", export_dialog_parts(export_dialog_run("clap")))


def stage_export_dialog_fault() -> bool:
    d = export_dialog_run("fault-old-dest", {"DS_ARA_EDITOR_FAULT": "old-dest"})
    parts = export_dialog_parts(d)
    ok = fault_part("export-dialog-fault-old-dest", parts, "export-dialog-no-folder-error",
                    "the default folder is refused as a dest (the rule before #579): the dialog says the folder cannot be used")
    d = export_dialog_run("fault-no-remember", {"DS_ARA_EDITOR_FAULT": "no-remember"})
    return fault_part("export-dialog-fault-no-remember", export_dialog_parts(d), "export-dialog-remembered",
                      "the chosen folder is not kept: a new editor opens with the default folder") and ok


# ---------------------------------------------------------------- M1c4 (#589): the export dialog makes one file per DAW track

def export_rows_run(label: str, env: dict | None = None, stuck: bool = False, late: bool = False, unload: bool = False) -> dict:
    """The REAL Export dialog in the REAL page (mode exportrows) on the host of the stage deliver (a DAW track with clips of three audio files, one with one
    file). The folder sheet is answered by DS_ARA_PICK_DIR; `other` is a folder that the plugin must refuse (it is empty at the end), `pick/sub` exists for the
    fault run prefix-dest."""
    base = OUT / f"exrows-{label}"
    if base.exists():
        shutil.rmtree(base)
    pick, other = base / "pick", base / "other"
    for d in (pick / "sub", other):
        d.mkdir(parents=True)
    pick, other = pick.resolve(), other.resolve()
    e = {"DS_ARA_PICK_DIR": str(pick), "DS_ARA_DRAG_PRESSED": "1"}
    e.update(env or {})
    arg = "stuck" if stuck else "late" if late else "unload" if unload else f"{other};{pick}"
    rc, summary, rows = editor_run(f"exrows-{label}", "exportrows", CLAP, 1, e, [*DELIVER_ARGS, "--arg", arg], timeout=500)
    reps = page_reports(rows, "exportrows")
    return {"rc": rc, "rep": reps[0] if reps else {}, "rows": rows, "pick": pick, "other": other, "base": base.resolve(),
            "cache": (OUT / f"cache-exrows-{label}").resolve()}


def export_rows_parts(d: dict) -> dict:
    r, pick, other = d["rep"], d["pick"], d["other"]
    parts: dict = {}
    steps = {x.get("label"): x for x in r.get("steps", [])}
    exports = next(iter(sorted(d["cache"].glob("documents/*/project/Exports"))), d["cache"] / "none")
    top = lambda f: sorted(x for x in f.glob("*") if x.is_file()) if f.is_dir() else []
    rows = r.get("rows") or []
    names = {x["id"]: x["name"] for x in rows}
    parts["export-rows-ran"] = (d["rc"] == 0 and bool(r) and r.get("menu") == "ok" and len(rows) == 2 and steps.get("export1", {}).get("export_finished") is True
                                and (steps["export1"].get("result") or "").startswith("Exported 2 MIDI and 2 spike files"),
                                f"the host ran the driver (rc={d['rc']}), the dialog opened from the menu: {r.get('menu')!r}, {len(rows)} DAW tracks, result {steps.get('export1', {}).get('result')!r}")
    want_names = {"Kick_spike.mid", "Snare_spike.mid", "Kick_spike.wav", "Snare_spike.wav", "Drums_merged.mid"}
    got_names = {f.name for f in top(exports)}
    parts["export-rows-files"] = (got_names == want_names,
                                  f"Export wrote ONE file per DAW track and kind (3 audio files in one track, 1 in the other: 2 MIDI, 2 spike) and the merged MIDI file: {sorted(got_names)}, want {sorted(want_names)}")
    take = [Path(f) for f in ((r.get("take") or {}).get("v") or {}).get("files", [])]
    merged = [Path(f) for f in ((r.get("take_merged") or {}).get("v") or {}).get("files", [])]
    diff = [t.name for t in [*take, *merged] if not (exports / re.sub(r" \d+(\.\w+)$", r"\1", t.name)).is_file() or (exports / re.sub(r" \d+(\.\w+)$", r"\1", t.name)).read_bytes() != t.read_bytes()]
    parts["export-rows-bytes"] = (len(take) == 4 and len(merged) == 1 and not diff,
                                  f"the files of the dialog are byte-equal to the files that the handles give ({len(take) + len(merged)} compared, different: {diff})")
    want = deliver_expected(r)
    bad = []
    for f in sorted(exports.glob("*_spike.mid")) if exports.is_dir() else []:
        row = next((i for i, n in names.items() if f.name == f"{n}_spike.mid"), None)
        got = midi_note_times(f)
        w = want.get(row, [])
        if row is None or len(got) != len(w) or any(abs(a - b) > 2 / 48000 for a, b in zip(got, w)):
            bad.append((f.name, len(got), len(w)))
    parts["export-rows-hits"] = (not bad and sorted(len(v) for v in want.values()) == [3, 6],
                                 f"the MIDI file of the DAW track with three audio files holds the hits of all its clips at their song positions; wrong: {bad}; hits per track {[len(v) for v in want.values()]}")
    one, sel, st = steps.get("single", {}), steps.get("selected", {}), steps.get("open", {})
    mg = exports / "Drums_merged.mid"
    parts["export-rows-single"] = (mg.is_file() and midi_tracks(mg) == 2 and "one midi track with all drums" in (one.get("single_label") or "").lower()
                                   and "one midi track per" not in (one.get("single_label") or "").lower() and "merged MIDI" in (steps.get("export_single", {}).get("result") or ""),
                                   f"the single file mode is the merged MIDI file (one note track: {midi_tracks(mg) if mg.is_file() else None} tracks incl. the tempo track), labelled {one.get('single_label')!r}; result {steps.get('export_single', {}).get('result')!r}")
    parts["export-rows-selected"] = (sel.get("single_disabled") is True and "needs All tracks" in (sel.get("single_title") or "") and (sel.get("sel_text") or "").startswith("Selected track (1): Kick"),
                                     f"with one selected DAW track the single mode is off with its reason: disabled {sel.get('single_disabled')}, {sel.get('single_title')!r}, {sel.get('sel_text')!r}")
    parts["export-rows-words"] = (st.get("all_text") == "All tracks (2)" and not any(x.get("body") for x in r.get("steps", [])),
                                  f"the dialog counts DAW tracks ({st.get('all_text')!r}) and no text says 'source': {[x.get('label') for x in r.get('steps', []) if x.get('body')]}")
    after = deliver_expected(r, (rows[0]["id"], [(r.get("edit") or {}).get("song", 0.0)])) if rows and r.get("edit") else {}
    pk = top(pick)
    f_after = pick / "Kick_spike.mid"
    got = midi_note_times(f_after) if f_after.is_file() else []
    w = after.get(rows[0]["id"], []) if rows else []
    spike = pick / "Kick_spike.wav"
    peaks = set()
    if spike.is_file():
        sw = wav_samples(spike)
        peaks = {i for i, v in enumerate(sw["xs"]) if v > 0.5}
        miss = [t for t in w if round(t * sw["rate"]) not in peaks]
    else:
        miss = list(w)
    parts["export-rows-wait"] = (bool((r.get("edit") or {}).get("ok")) and len(got) == len(w) > 0 and all(abs(a - b) <= 2 / 48000 for a, b in zip(got, w)) and not miss,
                                 f"an edit and Export at once: the dialog waited for the new set, the new hit is in the MIDI file and the spike file ({len(got)} notes, want {len(w)}, spikes missing {len(miss)}), never the old file")
    audios = [f for f in pk if f.name.endswith("_audio_spike.wav")]
    parts["export-rows-chosen"] = (str(Path(steps.get("chosen", {}).get("dest", "")).resolve()) == str(pick) and len(audios) == 2 and (pick / "Snare_spike.mid").is_file()
                                   and "audio spike" in (steps.get("export_both", {}).get("result") or ""),
                                   f"after Choose the files are in the chosen folder ({[f.name for f in pk]}), the type Both wrote {len(audios)} audio spike files: {steps.get('export_both', {}).get('result')!r}")
    fg = r.get("forged") or {}
    code = lambda k: (fg.get(k) or {}).get("code") if not (fg.get(k) or {}).get("ok", True) else 200
    escaped = [p for p in (d["base"] / "escape.mid", pick / "escape.mid", exports / "escape.mid", pick.parent / "escape.mid") if p.exists()]
    parts["export-rows-forged-row"] = (code("row") == 400 and code("plan_row") == 400,
                                       f"a row id that is no live row is refused: export {code('row')}, plan {code('plan_row')} (want 400, 400)")
    parts["export-rows-forged-name"] = (code("name") == 400 and not escaped,
                                        f"a name in an item is refused ({code('name')}, want 400) and no file lands outside the folder: {[str(p) for p in escaped]}")
    leaked = top(other) + (top(pick / "sub") if (pick / "sub").is_dir() else [])
    parts["export-rows-forged-dest"] = (code("dest") == 403 and code("sub") == 403 and code("path") == 403 and not leaked,
                                        f"a folder that is not the default or chosen one, a subfolder of the chosen one and a path key are refused (403): {code('dest')}, {code('sub')}, {code('path')}; written there: {[f.name for f in leaked]}")
    tmps = [f for folder in (exports, pick) for f in folder.glob(".*.tmp")] if exports.is_dir() else []
    parts["export-rows-no-tmp"] = (not tmps, f"no temporary file is left behind: {[f.name for f in tmps]}")
    return parts


def export_rows_stuck_parts(d: dict) -> dict:
    r = d["rep"]
    stuck = {x.get("label"): x for x in r.get("steps", [])}.get("stuck", {})
    exports = next(iter(sorted(d["cache"].glob("documents/*/project/Exports"))), d["cache"] / "none")
    written = sorted(f.name for f in exports.glob("*") if f.is_file()) if exports.is_dir() else []
    names = [x["name"] for x in r.get("rows", [])]
    msg = stuck.get("msg") or ""
    return {"export-rows-stuck": (stuck.get("export_finished") is True and "is not ready after" in msg and any(n in msg for n in names) and "nothing was written" in msg and not written,
                                  f"a set that never gets ready: the dialog names the DAW track, writes nothing and stops ({msg!r}; files written {written})")}


def export_rows_late_parts(d: dict) -> dict:
    """A hit is added while the export copies (the fault slow-copy keeps the copy open for 2.5 s): the check of the hits after the copy refuses the old files, the dialog
    waits and delivers the new set. The MIDI file of the first DAW track must hold the new hit."""
    r = d["rep"]
    exports = next(iter(sorted(d["cache"].glob("documents/*/project/Exports"))), d["cache"] / "none")
    rows = (r.get("regions") or {}).get("rows", [])
    f = exports / "Kick_spike.mid"
    want = deliver_expected(r, (rows[0]["id"], [(r.get("edit") or {}).get("song", 0.0)])).get(rows[0]["id"], []) if rows and r.get("edit") else []
    got = midi_note_times(f) if f.is_file() else []
    late = {x.get("label"): x for x in r.get("steps", [])}.get("late", {})
    tmps = [x.name for x in exports.glob(".*.tmp")] if exports.is_dir() else []
    sig = r.get("signal") or {}
    # #597: the edit waits for a signal of the plugin (the copy is done, the late check not yet), not for a fixed time; the signal was false before the press
    signalled = isinstance(sig.get("ms"), int) and sig["ms"] >= 0 and sig.get("before_copied") is False
    return {"export-rows-late": (signalled and bool((r.get("edit") or {}).get("ok")) and late.get("export_finished", True) is not False and len(got) == len(want) > 0
                                 and all(abs(a - b) <= 2 / 48000 for a, b in zip(got, want)) and not tmps,
                                 f"a hit added while the export copies, after the signal of the plugin that the copy is done (signal after {sig.get('ms')} ms, false before the press: {sig.get('before_copied')}): the check after the copy refused the old files and the new set was delivered ({len(got)} notes, want {len(want)}; temporary files {tmps}; result {late.get('result')!r})")}


def export_rows_unload_parts(d: dict) -> dict:
    """#597: the host unloads the library while the export copies (slow-copy-long keeps the copy open 8 s): the unload waits for the request thread of the copy, which stops at its
    checkpoint; no thread of the export runs after the unload, no file is written or replaced, no temporary file is left."""
    r = d["rep"]
    exports = next(iter(sorted(d["cache"].glob("documents/*/project/Exports"))), d["cache"] / "none")
    rows = [x for x in d["rows"] if x.get("event") == "prepare_unloaded"]
    row = rows[-1] if rows else {}
    written = sorted(f.name for f in exports.glob("*") if f.is_file()) if exports.is_dir() else []
    tmps = [f.name for f in exports.glob(".*.tmp")] if exports.is_dir() else []
    sig = (r.get("unload") or {}).get("signal_ms")
    before = r.get("before") or {}
    return {"export-rows-unload": (isinstance(sig, int) and sig >= 0 and before.get("copied") is False and row.get("exports_at_start") == 2
                                   and row.get("exports_after") == 0 and row.get("ended") is True and (row.get("ms") or 99999) < 4000 and not written and not tmps,
                                   f"the library was unloaded while a copy ran (signal after {sig} ms, false before the press: {before.get('copied')}): the unload saw {row.get('exports_at_start')} export guard(s) (want 2: the request thread of the page and the export itself), "
                                   f"waited {row.get('ms')} ms (limit 4000), {row.get('exports_after')} still ran after it (want 0), ended {row.get('ended')}; files written {written}, temporary files {tmps}")}


def stage_export_rows() -> bool:
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("export-rows", False, "no host or bundle: run check.sh build")
    ok = print_parts("", export_rows_parts(export_rows_run("clap")))
    d = export_rows_run("stuck", {"DS_ARA_PREPARE_FAULT": "forever,no-watchdog"}, stuck=True)
    ok = print_parts("", export_rows_stuck_parts(d)) and ok
    d = export_rows_run("late", {"DS_ARA_PREPARE_FAULT": "slow-copy"}, late=True)
    ok = print_parts("", export_rows_late_parts(d)) and ok
    d = export_rows_run("unload", {"DS_ARA_PREPARE_FAULT": "slow-copy-long"}, unload=True)
    return print_parts("", export_rows_unload_parts(d)) and ok


def stage_export_rows_fault() -> bool:
    d = export_rows_run("fault-no-late-check", {"DS_ARA_PREPARE_FAULT": "slow-copy,no-late-check"}, late=True)
    ok = fault_part("export-rows-fault-no-late-check", export_rows_late_parts(d), "export-rows-late", "the hits are not checked again after the copy: the old files are delivered although a hit changed")
    d = export_rows_run("fault-no-signal", {"DS_ARA_PREPARE_FAULT": "slow-copy,no-copied-signal"}, late=True)
    ok = fault_part("export-rows-fault-no-signal", export_rows_late_parts(d), "export-rows-late", "the plugin never says that the copy is done: the stage finds no signal and does not edit at a guessed time") and ok
    d = export_rows_run("fault-no-thread-guard", {"DS_ARA_PREPARE_FAULT": "slow-copy-long,no-thread-guard"}, unload=True)
    ok = fault_part("export-rows-fault-no-thread-guard", export_rows_unload_parts(d), "export-rows-unload", "the request thread of the export is not counted: the unload sees one guard, not two") and ok
    d = export_rows_run("fault-no-export-wait", {"DS_ARA_PREPARE_FAULT": "slow-copy-long,no-export-wait"}, unload=True)
    ok = fault_part("export-rows-fault-no-export-wait", export_rows_unload_parts(d), "export-rows-unload", "the unload does not wait for a running copy (the state before #597): a thread of the export still runs after it") and ok
    for label, env, part, what in (
        ("per-source", {"DS_ARA_PREPARE_FAULT": "per-source"}, "export-rows-files", "one file per audio file, as the old dialog made them: the file set is not one file per DAW track"),
        ("stale-ok", {"DS_ARA_PREPARE_FAULT": "stale-ok,no-late-check"}, "export-rows-wait", "a copy that ignores the state of the hits, before and after the copy, writes the old file after an edit"),
        ("any-dest", {"DS_ARA_EDITOR_FAULT": "any-dest"}, "export-rows-forged-dest", "any folder is allowed: a file lands in a folder that was never chosen"),
        ("prefix-dest", {"DS_ARA_EDITOR_FAULT": "prefix-dest"}, "export-rows-forged-dest", "a subfolder of the chosen folder is allowed"),
        ("trust-name", {"DS_ARA_PREPARE_FAULT": "trust-name"}, "export-rows-forged-name", "a name that the page sends is used: a file lands outside the folder"),
        ("any-row", {"DS_ARA_PREPARE_FAULT": "any-row"}, "export-rows-forged-row", "a row id that is no live row is accepted"),
    ):
        d = export_rows_run(f"fault-{label}", env)
        ok = fault_part(f"export-rows-fault-{label}", export_rows_parts(d), part, what) and ok
        for p in (d["base"] / "escape.mid", d["pick"] / "escape.mid"):
            p.unlink(missing_ok=True)
    return ok


# ---------------------------------------------------------------- M1c3 (#576): the files made before the user asks, and the handles

DELIVER_ARGS = ["--seq-names", "Kick", "Snare", "--seq-of", "0", "0", "1", "0", "--regions-all", "0,1.5,0", "--regions-step", "2"]


def deliver_run(label: str, env: dict | None = None, args: list | None = None) -> dict:
    e = {"DS_ARA_DRAG_PRESSED": "1"}
    e.update(env or {})
    rc, summary, rows = run_phase(f"deliver-{label}", "deliver", CLAP, e, args or DELIVER_ARGS)
    reps = page_reports(rows, "deliver")
    return {"rc": rc, "rows": rows, "rep": reps[0] if reps else {}, "cache": (OUT / f"cache-deliver-{label}").resolve(), "rows_label": f"deliver-{label}"}


def host_rows(label: str, event: str) -> list:
    """Every row the test host printed to its stdout (out/host-<label>.log) for `event`."""
    out: list = []
    path = OUT / f"host-{label}.log"
    if path.exists():
        for ln in path.read_text(errors="replace").splitlines():
            if ln.startswith("{") and f'"{event}"' in ln:
                try:
                    out.append(json.loads(ln))
                except ValueError:
                    pass
    return out


def deliver_close_run(label: str, env: dict | None = None) -> dict:
    """#599: the run `close` (two editor cycles, no drag, no press in the second): the host watches the prepared folder on the disk, see the mode `deliver` of the host."""
    rc, summary, rows = editor_run(f"deliver-{label}", "deliver", CLAP, 2, env or {}, DELIVER_ARGS + ["--arg", "close"], timeout=300)
    return {"rc": rc, "rows": rows, "watch": {w.get("phase"): w for w in host_rows(f"deliver-{label}", "prepared_watch")}}


def deliver_close_parts(d: dict) -> dict:
    """#599 (D1): the prepared AUDIO sets are gone from the disk after the last editor closed (the MIDI sets stay) and are back after the next open, with no press."""
    w, rows = d["watch"], d["rows"]
    o, c, b = w.get("open", {}), w.get("closed", {}), w.get("reopened", {})
    released = [r for r in rows if r.get("event") == "prepare_released" and r.get("why") == "closed"]
    reopened = [r for r in rows if r.get("event") == "prepare_reopened"]
    parts: dict = {}
    parts["deliver-close-ran"] = (d["rc"] == 0 and bool(o) and bool(c) and bool(b),
                                  f"the host ran two editor cycles (rc={d['rc']}) and watched the prepared folder: {sorted(w)}")
    parts["deliver-close-gone"] = (o.get("wav") == 4 and o.get("mid") == 3 and c.get("wav") == 0 and c.get("bytes") == 0 and c.get("mid") == 3 and len(released) >= 1 and released[0].get("removed") == 4 and released[0].get("failed") == 0,
                                   f".prepared/ held {o.get('wav')} wav sets ({o.get('bytes')} bytes) and {o.get('mid')} MIDI files with the editor open; {c.get('ms')} ms after the close it holds {c.get('wav')} wav files ({c.get('bytes')} bytes) and {c.get('mid')} MIDI files; "
                                   f"the plugin says removed {released[0].get('removed') if released else None}, failed {released[0].get('failed') if released else None}")
    parts["deliver-close-back"] = (b.get("wav") == o.get("wav") == 4 and b.get("mid") == 3 and len(reopened) >= 1 and reopened[0].get("sets") == 4,
                                   f"after the next open (no press) the folder holds {b.get('wav')} wav sets again after {b.get('ms')} ms (want {o.get('wav')}); the plugin says every wanted set is ready again {reopened[0].get('ms') if reopened else None} ms after the open")
    return parts


def deliver_expected(rep: dict, extra: tuple | None = None) -> dict:
    """`extra`: (row id, [song times]) hits that an edit added."""
    """The song times (seconds, sorted) of the hits of each row, from the rows and the stored hits that the page reported."""
    out = {}
    ev = rep.get("events") or {}
    for r in (rep.get("regions") or {}).get("rows", []):
        ts = []
        for c in r["clips"]:
            ts += [c["s"] + (e["t"] - c["a"]) * c["k"] for e in ev.get(c["src"], []) if c["a"] <= e["t"] < c["b"]]
        if extra and extra[0] == r["id"]:
            ts += list(extra[1])
        out[r["id"]] = sorted(ts)
    return out


def file_of(files: list, base: str, ext: str):
    """The delivered file of a set: `<base><ext>`, or numbered (`<base> 2<ext>`) when an older delivered file of that name exists."""
    return next((f for f in files if re.fullmatch(rf"{re.escape(base)}(?: \d+)?{re.escape(ext)}", f.name)), None)


def midi_tracks(path: Path) -> int:
    return int.from_bytes(path.read_bytes()[10:12], "big")


def deliver_parts(d: dict) -> dict:
    """The run `noedit`: the files as they were made (rows, wav kinds, merged file, states, drag, folder, handles)."""
    r = d["rep"]
    parts: dict = {}
    rows = (r.get("regions") or {}).get("rows", [])
    names = {x["id"]: x["name"] for x in rows}
    parts["deliver-ran"] = (d["rc"] == 0 and bool(r) and len(rows) == 2 and r.get("t_midi", -1) > 0 and "dom3" in r,
                            f"the host ran the driver (rc={d['rc']}), {len(rows)} rows, MIDI ready after {r.get('t_midi')} ms")
    want = deliver_expected(r)
    take = r.get("take_midi") or {}
    files = [Path(f) for f in (take.get("v") or {}).get("files", [])]
    bad = []
    for f in files:
        got = midi_note_times(f) if f.is_file() else []
        row = next((i for i, n in names.items() if file_of([f], f"{n}_spike", ".mid")), None)
        w = want.get(row, [])
        if row is None or len(got) != len(w) or any(abs(a - b) > 2 / 48000 for a, b in zip(got, w)):
            bad.append((f.name, len(got), len(w)))
    parts["deliver-rows"] = (take.get("ok") is True and len(files) == len(rows) == 2 and not bad and sorted(len(v) for v in want.values()) == [3, 6]
                             and len({f.name for f in files}) == 2,
                             f"ONE MIDI file per DAW track ({len(files)} files for {len(rows)} rows, named {[f.name for f in files]}) with the hits of all clips of the "
                             f"row at their song positions (within 2 samples); wrong: {bad}; hits per row {[len(v) for v in want.values()]} (three sources and one)")
    all_files = [Path(f) for f in ((r.get("take_all") or {}).get("v") or {}).get("files", [])]
    spikes = [f for f in all_files if f.name.endswith("_spike.wav") and not f.name.endswith("_audio_spike.wav")]
    audios = [f for f in all_files if f.name.endswith("_audio_spike.wav")]
    bad_s, bad_a = [], []
    for f in spikes:
        w = wav_samples(f)
        row = next((i for i, n in names.items() if file_of([f], f"{n}_spike", ".wav")), None)
        peaks = {i for i, v in enumerate(w["xs"]) if v > 0.5}
        miss = [t for t in want.get(row, []) if round(t * w["rate"]) not in peaks]
        if row is None or miss or w["tref"] != 0:
            bad_s.append((f.name, len(miss), w["tref"]))
    for f in audios:
        w = wav_samples(f)
        row = next((i for i, n in names.items() if file_of([f], f"{n}_audio_spike", ".wav")), None)
        quiet = [t for t in want.get(row, []) if not any(abs(v) > 0 for v in w["xs"][round(t * w["rate"]):round(t * w["rate"]) + 48])]
        if row is None or quiet or w["tref"] != 0:
            bad_a.append((f.name, len(quiet), w["tref"]))
    parts["deliver-wav"] = (len(spikes) == 2 and len(audios) == 2 and not bad_s and not bad_a and r.get("t_spike", -1) > 0 and r.get("t_audio", -1) > 0,
                            f"one spike file and one audio spike file per DAW track, each hit at its song sample, time reference 0 ({len(spikes)} + {len(audios)} files; wrong spikes {bad_s}, "
                            f"wrong audio {bad_a}); first use made them ready after {r.get('t_spike')} ms and {r.get('t_audio')} ms")
    sizes, bad_size = [], []
    for f in spikes:
        w = wav_samples(f)
        row = next((i for i, n in names.items() if file_of([f], f"{n}_spike", ".wav")), None)
        clips = next((x["clips"] for x in rows if x["id"] == row), [])
        end = max((c["s"] + (c["b"] - c["a"]) * c["k"] for c in clips), default=0)
        n = math.ceil(end * w["rate"] - 1e-9)
        size = f.stat().st_size
        sizes.append((f.name, size, w["rate"], n))
        # the length is the end of the last clip in the song, in float 32: the file is 4 bytes a frame and a header of less than 2000 bytes
        if not (len(w["xs"]) in (n, n + 1) and 4 * n <= size <= 4 * n + 2000):
            bad_size.append((f.name, size, n))
    parts["deliver-size"] = (len(sizes) == 2 and not bad_size,
                             f"the spike files have the computed size (4 bytes x ceil(song end x rate) + a header under 2000): {sizes} (name, bytes, rate, frames), wrong {bad_size}; "
                             f"ready {r.get('t_spike')} ms after the press for {len(rows)} rows (song end {(r.get('regions') or {}).get('song_end')} s); audio spikes ready {r.get('t_audio')} ms")
    mg = [Path(f) for f in ((r.get("take_merged") or {}).get("v") or {}).get("files", [])]
    total = sum(len(v) for v in want.values())
    n_mg = len(midi_note_times(mg[0])) if mg and mg[0].is_file() else None
    parts["deliver-merged"] = (len(mg) == 1 and mg[0].name == "Drums_merged.mid" and mg[0].is_file() and midi_tracks(mg[0]) == 2 and n_mg == total,
                               f"one merged MIDI file {[m.name for m in mg]} with the tempo track and ONE note track ({midi_tracks(mg[0]) if mg and mg[0].is_file() else None} tracks, want 2) "
                               f"holding all {total} hits ({n_mg})")
    s1, s2 = r.get("state1") or {}, r.get("state2") or {}
    st1 = [(x["midi"]["state"], x["spike"]["state"], x["audio"]["state"]) for x in s1.get("rows", [])]
    refused = r.get("take_spike_off") or {}
    parts["deliver-state"] = (st1 == [("ready", "off", "off")] * 2 and refused.get("ok") is False and "not prepared yet" in str(refused.get("msg"))
                              and all(x["spike"]["state"] == x["audio"]["state"] == "ready" for x in s2.get("rows", [])) and bool(s2.get("rows")),
                              f"after the round MIDI is ready and the wav kinds say press to prepare ({st1}); a take of a kind that is off is refused ({refused.get('msg')!r}); after the first use all are ready")
    started = [x for x in d["rows"] if x.get("event") == "drag_started"]
    prep_dirs = {str(Path(f).parent) for f in all_files}
    parts["deliver-drag"] = (r.get("drag") == "ok" and bool(started) and all(x.get("session") == "test" for x in started) and len(started[-1].get("files", [])) == len(all_files) == 6
                             and all(Path(x).parent.name == "Dragged" and Path(x).parent.parent.name == "Exports" for x in started[-1].get("files", [])) and len(prep_dirs) == 1,
                             f"the drag of all files through the page: {r.get('drag')}; the drag command got {len(started[-1].get('files', [])) if started else 0} files (want 6), all in Exports/Dragged/, "
                             f"as a private pasteboard (no system session)")
    expo = Path(next(iter(prep_dirs))).parent if prep_dirs else None
    # #599: the folder as the host saw it while the editor was still open (the closing editor and the unload remove the audio sets, so the disk after the run holds the MIDI sets only)
    seen = [w for w in host_rows(d["rows_label"], "prepared_watch") if w.get("phase") == "open"] if d.get("rows_label") else []
    prep = list(seen[0].get("names") or []) if seen else []
    parts["deliver-prepared-folder"] = (len(prep) == 7 and not any(n.endswith(".tmp") for n in prep),
                                        f".prepared/ holds the 7 sets and no temporary file: {prep}")
    loose = sorted(p.name for p in expo.iterdir() if p.is_file()) if expo and expo.is_dir() else ["no Exports/"]
    delivered = sorted(p.name for p in (expo / "Dragged").iterdir()) if expo and (expo / "Dragged").is_dir() else []
    parts["deliver-exports-clean"] = (loose == [] and len(delivered) == 7,
                                      f"the worker wrote no file in Exports/ itself (files there: {loose}); the takes linked 7 files into Exports/Dragged/ ({delivered})")
    d1, d2 = r.get("dom1") or {}, r.get("dom2") or {}
    strips = list((d1.get("rows") or {}).values())
    parts["deliver-handles"] = (len(strips) == 2 and all([h["kind"] for h in s_] == ["midi", "spike", "audio"] for s_ in strips)
                                and all([h["state"] for h in s_] == ["ready", "off", "off"] for s_ in strips)
                                and [h["kind"] for h in d1.get("bar", [])] == ["midi", "spike", "all", "merged"]
                                and [h["state"] for h in d2.get("bar", [])] == ["ready"] * 4 and d1.get("hint") == "Drop at the start of the session, not at the clip",
                                f"two DAW tracks have a strip of three handles ({[[h['state'] for h in s_] for s_ in strips]}), the toolbar has four ({[h['kind'] for h in d1.get('bar', [])]}) and "
                                f"all are ready after the first use ({[h['state'] for h in d2.get('bar', [])]}); the hint {d1.get('hint')!r}")
    return parts


def deliver_edit_parts(d: dict) -> dict:
    """The run with the edit: a take right after it is stale, the sets are made again by themselves, only the row of the edit."""
    r = d["rep"]
    rows = (r.get("regions") or {}).get("rows", [])
    names = {x["id"]: x["name"] for x in rows}
    stale = r.get("take_stale") or {}
    ed = r.get("edit") or {}
    after = deliver_expected(r, (rows[0]["id"], ed.get("songs", [])) if rows else None)
    ta = [Path(f) for f in ((r.get("take_after") or {}).get("v") or {}).get("files", [])]
    first = file_of(ta, names[rows[0]["id"]] + "_spike", ".mid") if rows else None
    spk = file_of(ta, names[rows[0]["id"]] + "_spike", ".wav") if rows else None
    sw = wav_samples(spk) if spk and spk.is_file() else {"rate": 48000, "xs": [], "tref": None}
    peaks = {i for i, v in enumerate(sw["xs"]) if v > 0.5}
    miss = [t for t in (after[rows[0]["id"]] if rows else []) if round(t * sw["rate"]) not in peaks]
    got = midi_note_times(first) if first and first.is_file() else []
    mg = [Path(f) for f in ((r.get("take_merged_after") or {}).get("v") or {}).get("files", [])]
    n_mg = len(midi_note_times(mg[0])) if mg and mg[0].is_file() else None
    parts = {}
    parts["deliver-stale"] = (ed.get("ok") is True and stale.get("ok") is False and "preparing" in str(stale.get("msg"))
                              and r.get("t_after", -1) > 0 and bool(rows) and len(got) == len(after[rows[0]["id"]]) and all(any(abs(g - t) < 2 / 48000 for g in got) for t in ed.get("songs", [0]))
                              and n_mg == sum(len(v) for v in after.values()),
                              f"a take right after an edit is refused as stale ({stale.get('msg')!r}); the set becomes ready by itself after {r.get('t_after')} ms and holds the new hit "
                              f"({len(got)} notes, want {len(after[rows[0]['id']]) if rows else None}); the merged file has {n_mg} notes (want {sum(len(v) for v in after.values())})")
    c0, c1 = r.get("count_before") or {}, r.get("count_after") or {}
    jobs = c1.get("jobs", 0) - c0.get("jobs", 0)
    parts["deliver-spike-after"] = (spk is not None and not miss and sw["tref"] == 0 and bool(peaks),
                                    f"the spike file of the edited row after the edit (delivered as {spk.name if spk else None}) has a spike at every hit, the new ones too ({len(miss)} missing of {len(after[rows[0]['id']]) if rows else 0})")
    keep = sorted(d["cache"].glob("documents/*/project/Exports/Kick_spike.mid"))
    plan = b"USER FILE Kick_spike.mid"
    now = keep[0].read_bytes() if keep else None
    parts["deliver-user-file"] = (now == plan, f"a file of the user, Exports/Kick_spike.mid with known bytes, is the same after the preparation, takes and edits ({now!r}, want {plan!r})")
    parts["deliver-delivered-keeps"] = (len({f.name for f in ta}) == len(ta) and any(re.search(r" \d+\.", f.name) for f in ta) and (first is not None and first.stat().st_size > 0),
                                        f"after the edit the take delivered NEW numbered files and left the first ones as they were ({sorted(f.name for f in ta)})")
    parts["deliver-cost"] = (jobs == 4, f"three edits in one DAW track, 400 ms apart, make only its own sets, once: {jobs} jobs (MIDI, spike, audio of that row and the merged file = 4; every row would be 7, a pass for each edit 12)")
    return parts


def deliver_watchdog_parts(d: dict) -> dict:
    r = d["rep"]
    st = [x["midi"] for x in (r.get("state1") or {}).get("rows", [])]
    return {"deliver-watchdog": (bool(st) and all(x["state"] == "error" and "no answer" in x["why"] for x in st),
                                 f"a job that never ends: after the watchdog the handles say error, never preparing for ever: {[(x['state'], x['why']) for x in st]}")}


def deliver_hostile_parts(d: dict) -> dict:
    r = d["rep"]
    cache = d["cache"]
    outside = [p for p in cache.glob("documents/*/project/Exports/*") if p.is_file() and p.name.startswith((".", "evil"))] if cache.exists() else []
    outside += [p for p in cache.glob("documents/*/project/*") if p.is_file() and p.suffix != ".drumspike"]
    prepared = [p for p in cache.glob("documents/*/project/Exports/.prepared/*") if p.is_file()]
    state = [x["midi"]["state"] for x in (r.get("state1") or {}).get("rows", [])]
    return {"deliver-hostile-name": (len(prepared) >= 1 and not outside and not any("/" in p.name for p in prepared) and state == ["ready", "ready"],
                                     f"DAW tracks named '../evil' and '/abs': {len(prepared)} files stay inside .prepared/, none outside ({[p.name for p in outside]}), states {state}")}


def deliver_names_parts(d: dict) -> dict:
    r = d["rep"]
    files = [Path(f) for f in ((r.get("take_all") or {}).get("v") or {}).get("files", [])]
    names = sorted(f.name for f in files)
    want = sorted(["Snare_spike.mid", "Snare_audio_spike.mid", "Snare_spike.wav", "Snare_audio_spike.wav", "Snare (2)_audio_spike.wav", "Snare_audio_audio_spike.wav"])
    low = {n.lower() for n in names}
    return {"deliver-file-names": (names == want and len(low) == 6,
                                   f"rows 'Snare' and 'Snare_audio': the six delivered files have six different names without case, the later audio file is numbered: {names} (want {want})")}


def deliver_late_parts(d: dict) -> dict:
    """Round 3 (P1): a new row whose spike file has the name of another row's audio file must not change that audio set."""
    r = d["rep"]
    l1 = [Path(f) for f in ((r.get("late1") or {}).get("v") or {}).get("files", [])]
    l2 = [Path(f) for f in ((r.get("late2") or {}).get("v") or {}).get("files", [])]
    a = l1[0].read_bytes() if l1 and l1[0].is_file() else None
    audio2 = l2[0] if len(l2) == 2 else None
    spike2 = l2[1] if len(l2) == 2 else None
    ok = (a is not None and audio2 is not None and audio2.is_file() and spike2 is not None and spike2.is_file()
          and audio2.read_bytes() == a and spike2.read_bytes() != a and l1[0].read_bytes() == a
          and re.fullmatch(r"Snare \(2\)_audio_spike( \d+)?\.wav", audio2.name) is not None)
    return {"deliver-name-clash": (ok, f"rows {r.get('late_rows')}: the audio set of 'Snare' is renamed and made again (delivered as {audio2.name if audio2 else None}), its bytes are still the audio "
                                       f"({audio2.read_bytes() == a if audio2 and audio2.is_file() and a is not None else None}); the spike of 'Snare_audio' has other bytes "
                                       f"({spike2.name if spike2 else None}); the file delivered before is unchanged")}


def stage_deliver() -> bool:
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("deliver", False, "no host or bundle: run check.sh build")
    ok = print_parts("", deliver_parts(deliver_run("base", None, DELIVER_ARGS + ["--arg", "noedit"])))
    ok = print_parts("", deliver_edit_parts(deliver_run("edit", {"DS_ARA_PREPARE_FAULT": "user-file", "DS_ARA_PREPARE_TRACE": "1"}))) and ok
    ok = print_parts("", deliver_late_parts(deliver_run("late", {"DS_ARA_PREPARE_FAULT": "late-row"}, ["--seq-names", "Snare", "Snare_audio", "--seq-of", "0", "0", "1", "0", "--regions-all", "0,1.5,0", "--regions-step", "2", "--arg", "late"]))) and ok
    ok = print_parts("", deliver_names_parts(deliver_run("names", None, ["--seq-names", "Snare", "Snare_audio", "--seq-of", "0", "0", "1", "0", "--regions-all", "0,1.5,0", "--regions-step", "2", "--arg", "noedit"]))) and ok
    ok = print_parts("", deliver_watchdog_parts(deliver_run("watchdog", {"DS_ARA_PREPARE_FAULT": "forever", "DS_ARA_PREPARE_WAIT_MS": "3000"}, DELIVER_ARGS + ["--arg", "short"]))) and ok
    ok = print_parts("", deliver_close_parts(deliver_close_run("close"))) and ok
    return print_parts("", deliver_hostile_parts(deliver_run("hostile", None, ["--seq-names", "../evil", "/abs", "--seq-of", "0", "0", "1", "0", "--regions-all", "0,1.5,0", "--regions-step", "2", "--arg", "short"]))) and ok


def stage_deliver_fault() -> bool:
    ok = True
    for label, fault, parts_of, args, part, what in (
        ("per-source", "per-source", deliver_parts, DELIVER_ARGS + ["--arg", "noedit"], "deliver-rows", "one file per source, as the dialog makes them: the sources of a DAW track give several files"),
        ("two-tracks", "two-tracks", deliver_parts, DELIVER_ARGS + ["--arg", "noedit"], "deliver-merged", "the merged file has one MIDI track per drum"),
        ("stale-ok", "stale-ok,user-file", deliver_edit_parts, None, "deliver-stale", "a take ignores the state of the hits: an old set is taken"),
        ("link-exports", "user-file,link-exports", deliver_edit_parts, None, "deliver-user-file", "the old behaviour: the worker links its file into Exports/ and replaces a file of the user"),
        ("all-rows", "all-rows,user-file", deliver_edit_parts, None, "deliver-cost", "every row is made again after an edit in one row"),
        ("no-debounce", "no-debounce,user-file", deliver_edit_parts, None, "deliver-cost", "the files are made again at every edit, with no quiet time"),
    ):
        parts = parts_of(deliver_run(f"fault-{label}", {"DS_ARA_PREPARE_FAULT": fault}, args))
        ok = fault_part(f"deliver-fault-{label}", parts, part, what) and ok
    d = deliver_run("fault-late", {"DS_ARA_PREPARE_FAULT": "late-row,fp-no-name,no-held-check"}, ["--seq-names", "Snare", "Snare_audio", "--seq-of", "0", "0", "1", "0", "--regions-all", "0,1.5,0", "--regions-step", "2", "--arg", "late"])
    ok = fault_part("deliver-fault-fp-no-name", deliver_late_parts(d), "deliver-name-clash", "the final name is not in fp and a name held by another set is overwritten: the audio set of one row delivers the spike content of another") and ok
    d = deliver_run("fault-names", {"DS_ARA_PREPARE_FAULT": "no-file-numbering"}, ["--seq-names", "Snare", "Snare_audio", "--seq-of", "0", "0", "1", "0", "--regions-all", "0,1.5,0", "--regions-step", "2", "--arg", "noedit"])
    ok = fault_part("deliver-fault-no-file-numbering", deliver_names_parts(d), "deliver-file-names", "two sets get one file name: the later one replaces the first") and ok
    d = deliver_run("fault-watchdog", {"DS_ARA_PREPARE_FAULT": "forever,no-watchdog", "DS_ARA_PREPARE_WAIT_MS": "3000"}, DELIVER_ARGS + ["--arg", "short"])
    ok = fault_part("deliver-fault-no-watchdog", deliver_watchdog_parts(d), "deliver-watchdog", "a job that never ends and no watchdog: the handle stays preparing") and ok
    d = deliver_close_run("fault-no-release", {"DS_ARA_PREPARE_FAULT": "no-release"})
    ok = fault_part("deliver-fault-no-release", deliver_close_parts(d), "deliver-close-gone", "the last editor closes and the audio sets stay on the disk") and ok
    d = deliver_close_run("fault-no-reopen", {"DS_ARA_PREPARE_FAULT": "no-reopen"})
    ok = fault_part("deliver-fault-no-reopen", deliver_close_parts(d), "deliver-close-back", "the editor opens again and the audio sets are not made again (the handles would stay preparing)") and ok
    d = deliver_run("fault-raw-name", {"DS_ARA_PREPARE_FAULT": "raw-name"}, ["--seq-names", "../evil", "/abs", "--seq-of", "0", "0", "1", "0", "--regions-all", "0,1.5,0", "--regions-step", "2", "--arg", "short"])
    ok = fault_part("deliver-fault-raw-name", deliver_hostile_parts(d), "deliver-hostile-name", "the row name goes into the file name as it is: the name reaches the path and the plugin refuses to write it (the state is an error, no file outside the folder)") and ok
    return ok


def drag_source_check(src: Path) -> tuple[bool, str]:
    """Round 3 of #579: no gate run may start a system drag session, post an event that leaves the process or touch the general pasteboard.
    A grep-level read of the plugin source: the call of the crate `drag` is in exactly one function, behind the test branch."""
    def code_lines(f: Path):
        for i, ln in enumerate(f.read_text().splitlines(), 1):
            t = ln.strip()
            if t and not t.startswith("//"):
                yield i, ln

    problems: list[str] = []
    calls = []
    for f in sorted(src.rglob("*.rs")):
        lines = list(code_lines(f))
        all_lines = f.read_text().splitlines()
        in_tests = len(all_lines) + 1
        for i, ln in enumerate(all_lines, 1):
            if ln.startswith("#[cfg(test)]"):
                in_tests = i
                break
        for i, ln in lines:
            if i >= in_tests:
                break
            rel = f.relative_to(src)
            if "drag::start_drag(" in ln:
                fn = next((all_lines[j].split("fn ", 1)[1].split("(")[0].split("<")[0] for j in range(i - 1, -1, -1) if re.match(r"\s*(pub )?(unsafe )?fn ", all_lines[j])), "?")
                guarded = fn != "?" and any("drag_is_test()" in all_lines[k] for k in range(i - 1 - 14, i - 1))
                calls.append((str(rel), i, fn, guarded))
            for bad in ("release_test_drag", "generalPasteboard", "NSPasteboardNameGeneral", "NSWorkspace", "CGEventPost", "CGEventCreate", "CGPostMouse",
                        "beginDraggingSession", "NSDraggingSession"):
                if bad in ln:
                    problems.append(f"{rel}:{i}: {bad}")
            if "NSPasteboardNameDrag" in ln and not (str(rel) == "deliver.rs"):
                problems.append(f"{rel}:{i}: the drag pasteboard is read outside deliver.rs")
            if "postEvent_atStart" in ln and not str(rel).startswith("bin/"):
                problems.append(f"{rel}:{i}: an event is posted in the plugin")
    if len(calls) != 1 or calls[0][0] != "deliver.rs" or calls[0][2] != "start_drag_system" or not calls[0][3]:
        problems.append(f"the system drag call must be once, in deliver.rs fn start_drag_system, behind drag_is_test(): found {calls}")
    return not problems, f"{len(calls)} call(s) of drag::start_drag {calls}; problems: {problems}"


def stage_drag_source() -> bool:
    ok, ev = drag_source_check(HERE / "src")
    return pass_fail("drag-source", ok, f"no gate run can start a system drag session, post an event out of the process or touch the general pasteboard: {ev}")


def stage_drag_source_fault() -> bool:
    plant = OUT / "drag-source-fault"
    if plant.exists():
        shutil.rmtree(plant)
    shutil.copytree(HERE / "src", plant)
    ed = plant / "editor.rs"
    ed.write_text("fn planted(h: &H) { drag::start_drag(h, a, b, c, d); }\n" + ed.read_text())
    ok, ev = drag_source_check(plant)
    return pass_fail("drag-source-fault", not ok, f"the fault run (a second call of the system drag in editor.rs) fails the source check: {ev}")



# ---------------------------------------------------------------- #577 (M1d3): the user defaults and presets of the plugin
#
# Every host process of these stages gets its own `DS_ARA_USER_DIR` below out/ (guard_user_dir); a stage that would reach the real folder
# fails before it starts. The cargo unit tests are the stage `crate`; the page is proved by tests/e2e_app/test_host_defaults.py.

DEFAULTS_KICK = "m1a_kick_48k_mono"
DEFAULTS_FILE = {
    "v": 1,
    "roles": {
        "kick": {"notes": [35]},
        "snare": {"note": 41, "params": {"min_gap_ms": 33}},
        "tom": {"notes": [50, 48]},
    },
}


def plant_defaults(label: str, content) -> Path:
    """The folder of the user defaults of one run (below out/), with `defaults.json` made from `content` (a text or an object)."""
    d = OUT / f"user-{label}"
    if d.exists() or d.is_symlink():
        shutil.rmtree(d)
    d.mkdir(parents=True)
    if content is not None:
        (d / "defaults.json").write_text(content if isinstance(content, str) else json.dumps(content), encoding="utf-8")
    return d


def defaults_session(label: str, content, extra: dict | None = None) -> tuple[int, dict, list]:
    """A fresh session of the four fixtures with the defaults of `content`. Returns (rc, tracks of the project document, report rows)."""
    d = plant_defaults(label, content)
    rc, _text, _lines = host_run(
        label, CLAP, ["session", "--files", *fixture_args(FILES), "--wait", HOST_WAIT], f"cache-{label}", extra={"DS_ARA_USER_DIR": str(d), **(extra or {})}
    )
    docs = project_docs(f"cache-{label}")
    return rc, (tracks_of_doc(docs[0]) if len(docs) == 1 else {}), report_rows(label)


def role_track(tracks: dict, role: str) -> dict:
    return next((v for v in tracks.values() if v.get("role") == role), {})


def new_track_facts(label: str, extra: dict | None = None) -> tuple[bool, str]:
    rc, tracks, _rows = defaults_session(label, DEFAULTS_FILE, extra)
    kick, snare, tom, hat = (role_track(tracks, r) for r in ("kick", "snare", "tom", "hat"))
    parts = {
        "rc": rc == 0 and len(tracks) == len(FILES),
        "kick": kick.get("note") == 35,
        "snare": snare.get("note") == 41 and (snare.get("params") or {}).get("min_gap_ms") == 33,
        "tom": tom.get("note") == 50,
        "hat": hat.get("note") == 42 and (hat.get("params") or {}).get("min_gap_ms") != 33,
    }
    bad = [k for k, v in parts.items() if not v]
    return not bad, (
        f"rc={rc}, {len(tracks)} tracks: kick note {kick.get('note')} (want 35), snare note {snare.get('note')} min_gap {(snare.get('params') or {}).get('min_gap_ms')} "
        f"(want 41, 33), tom note {tom.get('note')} (want 50), hat note {hat.get('note')} (want 42, no default); wrong: {bad or 'none'}"
    )


def restored_equals_stored(label: str, archive: Path, extra: dict | None = None) -> tuple[bool, str]:
    """A restore with a defaults file that would change every role: each restored track equals what the archive stored."""
    d = plant_defaults(label, {"v": 1, "roles": {r: {"notes": [99], "params": {"min_gap_ms": 99}} for r in ("kick", "snare", "tom", "hat", "cymbal", "other")}})
    rc, _text, _lines = host_run(
        label, CLAP, ["restore", "--files", *fixture_args(FILES), "--archive", str(archive)], f"cache-{label}", extra={"DS_ARA_USER_DIR": str(d), **(extra or {})}
    )
    docs = project_docs(f"cache-{label}")
    tracks = tracks_of_doc(docs[0]) if len(docs) == 1 else {}
    stored = {s["track"]: s.get("settings") or {} for s in json.loads(archive.read_text())["sources"] if s.get("track")}
    wrong = []
    for name, st in stored.items():
        got = tracks.get(name) or {}
        if got.get("note") != st.get("note") or got.get("role") != st.get("role") or (st.get("params") and got.get("params") != st.get("params")):
            wrong.append(name)
    with_settings = sum(1 for st in stored.values() if st.get("note") is not None)
    return rc == 0 and bool(stored) and with_settings > 0 and not wrong, (
        f"rc={rc}, {len(stored)} tracks stored ({with_settings} with settings), restored tracks that differ from the archive: {wrong or 'none'}"
    )


def stage_defaults_new_track() -> bool:
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("defaults-new-track", False, "no host or no CLAP: run check.sh build first")
    ok, ev = new_track_facts("defaults-new-track")
    ok = pass_fail("defaults-new-track", ok, "new tracks start with the notes and settings of the file, a role with no entry keeps the built-in values: " + ev)
    ok_store, archive, why = archive_store()
    if not ok_store:
        return pass_fail("defaults-new-track-restore", False, why)
    ok2, ev2 = restored_equals_stored("defaults-new-track-restore", archive)
    return pass_fail("defaults-new-track-restore", ok2, "a restore never reads the file: " + ev2) and ok


def stage_defaults_new_track_fault() -> bool:
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("defaults-new-track-fault", False, "no host or no CLAP: run check.sh build first")
    ok_store, archive, why = archive_store()
    if not ok_store:
        return pass_fail("defaults-new-track-fault", False, why)
    res = {}
    for fault in ("always", "session-loses"):
        res[fault] = restored_equals_stored(f"defaults-fault-{fault}", archive, {"DS_ARA_DEFAULTS_FAULT": fault})
    return pass_fail(
        "defaults-new-track-fault",
        all(not v[0] for v in res.values()),
        "the fault runs (a restore applies the defaults; the defaults overwrite the stored settings) were refused by the check: "
        + "; ".join(f"{k}: {v[1]}" for k, v in res.items()),
    )


HOSTILE_PARTIAL = {"v": 1, "roles": {"kick": {"note": 300}, "snare": {"notes": [41]}, "tom": {"notes": [1, 2, 3, 4, 5, 6, 7, 8, 9]}}, "follow": {"mode": "x"}}


def hostile_defaults_run(label: str, how: str, extra: dict | None = None) -> tuple[bool, str]:
    content = {"not-json": "{{{ not json", "big": json.dumps({"v": 1, "pad": "x" * 300_000}), "partial": HOSTILE_PARTIAL, "link": None}[how]
    d = OUT / f"user-{label}"
    if how == "link":
        content = None
    rc, tracks, rows = None, {}, []
    if how == "link":
        plant_defaults(label, None)
        (OUT / f"user-{label}" / "real.json").write_text(json.dumps(DEFAULTS_FILE))
        (OUT / f"user-{label}" / "defaults.json").symlink_to(OUT / f"user-{label}" / "real.json")
        rc, _t, _l = host_run(label, CLAP, ["session", "--files", *fixture_args(FILES), "--wait", HOST_WAIT], f"cache-{label}", extra={"DS_ARA_USER_DIR": str(d), **(extra or {})})
        docs = project_docs(f"cache-{label}")
        tracks, rows = (tracks_of_doc(docs[0]) if len(docs) == 1 else {}), report_rows(label)
    else:
        rc, tracks, rows = defaults_session(label, content, extra)
    kick, snare, tom = (role_track(tracks, r) for r in ("kick", "snare", "tom"))
    invalid = [r for r in rows if r.get("event") == "defaults_invalid"]
    leaked = [r for r in invalid if "/" in json.dumps(r) or "Users" in json.dumps(r)]
    parts = {
        "rc": rc == 0 and len(tracks) == len(FILES),
        "kick-built-in": kick.get("note") == 36,
        "tom-built-in": tom.get("note") == 48,
        "snare": snare.get("note") == (41 if how == "partial" else 38),
        "one-row": len(invalid) == 1,
        "no-path": not leaked,
    }
    bad = [k for k, v in parts.items() if not v]
    return not bad, f"{how}: rc={rc}, kick {kick.get('note')}, snare {snare.get('note')}, tom {tom.get('note')}, {len(invalid)} defaults_invalid row(s); wrong: {bad or 'none'}"


def stage_defaults_hostile() -> bool:
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("defaults-hostile", False, "no host or no CLAP: run check.sh build first")
    ok = True
    for how in ("not-json", "big", "partial", "link"):
        good, ev = hostile_defaults_run(f"defaults-hostile-{how}", how)
        ok = pass_fail(f"defaults-hostile-{how}", good, "built-in values for the bad fields, the good ones stay, one report row, the plugin runs on: " + ev) and ok
    return ok


def stage_defaults_hostile_fault() -> bool:
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("defaults-hostile-fault", False, "no host or no CLAP: run check.sh build first")
    good, ev = hostile_defaults_run("defaults-hostile-trust", "partial", {"DS_ARA_DEFAULTS_FAULT": "trust"})
    return pass_fail("defaults-hostile-fault", not good, f"the fault run (no range check) was refused by the check: {ev}")


def real_path_facts(label: str, extra: dict | None = None) -> tuple[bool, str]:
    home = OUT / f"home-{label}"
    if home.exists():
        shutil.rmtree(home)
    home.mkdir(parents=True)
    rc, _text, _lines = host_run(
        label, CLAP, ["session", "--files", *fixture_args(HOME_FILES), "--wait", HOST_WAIT], f"cache-{label}",
        extra={"HOME": str(home), "DS_ARA_USER_DIR": USER_DIR_NONE, **(extra or {})},
    )
    refused = [r for r in report_rows(label) if r.get("event") == "defaults_path_refused"]
    folder = home / "Library" / "Application Support" / "DrumSpike ARA"
    made = [str(p.relative_to(home)) for p in home.rglob("*") if "DrumSpike ARA" in str(p)]
    return rc == 0 and bool(refused) and not made and not folder.exists(), (
        f"rc={rc}, {len(refused)} defaults_path_refused row(s) (want at least 1), files or folders of the plugin below HOME: {made or 'none'}"
    )


def stage_defaults_real_path() -> bool:
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("defaults-real-path", False, "no host or no CLAP: run check.sh build first")
    ok, ev = real_path_facts("defaults-real-path")
    ok = pass_fail("defaults-real-path", ok, "a gate build without DS_ARA_USER_DIR has no folder and says so: " + ev)
    # the guard of the runner: a value outside out/ is refused before a process starts
    real = str(Path.home() / "Library" / "Application Support" / "DrumSpike ARA")
    link = OUT / "ara-host-link"
    if link.is_symlink() or link.exists():
        link.unlink()
    link.symlink_to(HOST)
    refused = {}
    for how, cmd in (("host path", [str(HOST)]), ("a link to the host", [str(link)]), ("relative host name", ["./" + Path(HOST).name]), ("cargo", ["cargo", "test"])):
        try:
            guard_user_dir(cmd, {"DS_ARA_USER_DIR": real}, "x")
            refused[how] = False
        except AssertionError:
            refused[how] = True
    return pass_fail("defaults-real-path-guard", all(refused.values()), f"the runner refuses a folder of the user defaults outside out/ however the command is written: {refused}") and ok


def stage_defaults_real_path_fault() -> bool:
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("defaults-real-path-fault", False, "no host or no CLAP: run check.sh build first")
    ok, ev = real_path_facts("defaults-real-path-fault", {"DS_ARA_DEFAULTS_FAULT": "real-path"})
    return pass_fail("defaults-real-path-fault", not ok, f"the fault run (the real path resolves; HOME is below out/) was refused by the check: {ev}")


def race_workers(label: str, n: int, extra: dict) -> tuple[list, Path]:
    d = plant_defaults(label, {"v": 1})
    env = {**os.environ, **no_switch_env(), "DS_ARA_USER_DIR": str(d), "ARA_API_DIR": str(DEPS / "ARA_SDK" / "ARA_API"), "LIBCLANG_PATH": f"{CLT}/usr/lib",
           "CARGO_TARGET_DIR": str(HERE / "target"), **extra}
    cmd = ["cargo", "test", "--locked", "--features", "test-switches", "--lib", "race_worker", "--", "--ignored", "--exact", "userdefaults::tests::race_worker"]
    procs = [subprocess.Popen(cmd, cwd=HERE, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True) for _ in range(n)]
    outs = [p.communicate(timeout=900)[0] for p in procs]
    return [(p.returncode, o) for p, o in zip(procs, outs)], d


def no_switch_env() -> dict:
    return {k: v for k, v in os.environ.items() if k not in {e["name"] for e in load_switches() if e["kind"] != "check"}}


def writers_facts(label: str, extra: dict) -> tuple[bool, str]:
    runs, d = race_workers(label, 4, extra)
    try:
        v = json.loads((d / "defaults.json").read_text())
        final = v.get("v") == 1 and bool(v.get("roles"))
    except (ValueError, OSError):
        final = False
    failed = [i for i, (rc, _o) in enumerate(runs) if rc != 0]
    return not failed and final, f"4 processes of 150 saves each: {len(failed)} saw a half file or failed, the final file is valid JSON with roles: {final}"


def stage_defaults_writers() -> bool:
    ok, ev = writers_facts("defaults-writers", {})
    return pass_fail("defaults-writers", ok, "last writer wins and no reader sees half a file: " + ev)


def stage_defaults_writers_fault() -> bool:
    ok, ev = writers_facts("defaults-writers-fault", {"DS_ARA_DEFAULTS_FAULT": "in-place"})
    return pass_fail("defaults-writers-fault", not ok, f"the fault run (the file is written in place) was refused by the check: {ev}")


def load_plugin_files():
    import importlib.util

    spec = importlib.util.spec_from_file_location("plugin_files", HERE.parent.parent / "tests" / "plugin_files.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def stage_defaults_standalone_control() -> bool:
    """Gate line 6 (c): the comparison of the standalone test can see a write of the plugin. A plugin process saves roles into the planted folder; the
    comparison of tests/plugin_files.py must name the change (a control that cannot fire would prove nothing)."""
    pf = load_plugin_files()
    d = plant_defaults("defaults-standalone-control", {"v": 1, "roles": {"snare": {"notes": [99]}}})
    (d / "Presets").mkdir()
    (d / "Presets" / "Sentinel.json").write_text('{"v":1,"name":"Sentinel","roles":{}}')
    before = pf.snapshot(d)
    quiet = pf.changed(before, pf.snapshot(d))
    env = {**no_switch_env(), "DS_ARA_USER_DIR": str(d), "ARA_API_DIR": str(DEPS / "ARA_SDK" / "ARA_API"), "LIBCLANG_PATH": f"{CLT}/usr/lib", "CARGO_TARGET_DIR": str(HERE / "target")}
    rc, text = run(["cargo", "test", "--locked", "--features", "test-switches", "--lib", "--", "--ignored", "--exact", "userdefaults::tests::race_worker"], env=env, log="defaults-standalone-control.log", cwd=HERE)
    seen = pf.changed(before, pf.snapshot(d))
    return pass_fail(
        "defaults-standalone-control",
        rc == 0 and quiet == [] and any(s.endswith("defaults.json") for s in seen),
        f"nothing touched: {quiet}; after a plugin process saved roles, the comparison names: {seen[:3]} (rc={rc})",
    )


def archive_with_export(label: str, archive: Path, export) -> Path:
    blob = json.loads(archive.read_text())
    if export == "keep":
        pass
    elif export is None:
        blob.pop("export", None)
    else:
        blob["export"] = export
    out = OUT / f"{label}.araarchive"
    out.write_text(json.dumps(blob), encoding="utf-8")
    return out


def export_facts(label: str, archive: Path, export, extra: dict | None = None) -> dict:
    a = archive_with_export(label, archive, export)
    rc, _text, _lines = host_run(label, CLAP, ["restore", "--files", *fixture_args(FILES), "--archive", str(a)], f"cache-{label}", extra=extra)
    docs = project_docs(f"cache-{label}")
    doc = json.loads(docs[0].read_text()) if len(docs) == 1 else {}
    return {"rc": rc, "last": doc.get("export_last"), "comp": doc.get("export_compensate"), "rows": report_rows(label)}


def stage_doc_archive_export() -> bool:
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("doc-archive-export", False, "no host or no CLAP: run check.sh build first")
    ok_store, archive, why = archive_store()
    if not ok_store:
        return pass_fail("doc-archive-export", False, why)
    good = export_facts("doc-archive-export-good", archive, {"formats": ["midi", "spikes"], "midi_mode": "single", "compensate": False, "dest": "/etc", "scope": "selected", "future": 1})
    last = good["last"] or {}
    ok = pass_fail(
        "doc-archive-export",
        good["rc"] == 0 and last.get("formats") == ["midi", "spikes"] and last.get("midi_mode") == "single" and last.get("compensate") is False and not last.get("dest") and good["comp"] is False,
        f"the export settings of the archive are back in the project; dest and scope of the archive are not taken: {last}",
    )
    old = export_facts("doc-archive-export-old", archive, None)
    ok = pass_fail("doc-archive-export-old", old["rc"] == 0 and old["last"] is None, f"an archive without the field restores as today (no export settings invented): {old['last']}") and ok
    bad = export_facts("doc-archive-export-hostile", archive, {"formats": ["midi", "../../x"], "midi_mode": 7, "compensate": "yes"})
    refused = [r for r in bad["rows"] if r.get("event") == "restore_refused" and r.get("what") == "export settings"]
    return pass_fail("doc-archive-export-hostile", bad["rc"] == 0 and bad["last"] is None and len(refused) == 1, f"hostile values are range-checked like a hostile page: nothing set ({bad['last']}), {len(refused)} refusal row(s)") and ok


def stage_doc_archive_export_fault() -> bool:
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("doc-archive-export-fault", False, "no host or no CLAP: run check.sh build first")
    ok_store, archive, why = archive_store()
    if not ok_store:
        return pass_fail("doc-archive-export-fault", False, why)
    f = export_facts("doc-archive-export-drop", archive, {"formats": ["midi"], "midi_mode": "single", "compensate": True}, {"DS_ARA_DEFAULTS_FAULT": "export-drop"})
    return pass_fail("doc-archive-export-fault", f["last"] is None, f"the fault run (the export settings are not restored) was seen by the check: export_last {f['last']}")


def unit_with_fault(name: str, filt: str, fault_env: dict) -> tuple[int, str]:
    env = {**no_switch_env(), "ARA_API_DIR": str(DEPS / "ARA_SDK" / "ARA_API"), "LIBCLANG_PATH": f"{CLT}/usr/lib", "CARGO_TARGET_DIR": str(HERE / "target"), **fault_env}
    return run(["cargo", "test", "--locked", "--features", "test-switches", "--lib", filt], env=env, log=f"{name}.log", cwd=HERE)


def stage_defaults_units() -> bool:
    """The unit tests of the rules (kept session values, hostile files, the notes of a role, the stale sets after a note) pass."""
    ok = True
    for name, filt in (
        ("kept", "held_without"), ("kept-gone", "the_kept_values_of_a_removed_source"), ("kept-round", "a_source_with_no_result_that_is_not_in_a_round"), ("kept-used", "a_used_entry_is_removed"), ("export-merge", "an_older_read_of_the_export"), ("twin-names", "a_shown_name_that_collides"), ("restore-late", "a_restore_after_a_first_round"),
        ("userdefaults", "userdefaults"), ("service-defaults", "new_toms"), ("prepare-note", "a_changed_note"),
    ):
        rc, text = unit_with_fault(f"defaults-unit-{name}", filt, {})
        passed = sum(int(m) for m in re.findall(r"test result: ok\. (\d+) passed", text))
        ok = pass_fail(f"defaults-unit-{name}", rc == 0 and passed > 0, f"cargo test {filt}: rc={rc}, {passed} passed") and ok
    return ok


def stage_defaults_units_fault() -> bool:
    """Each fault of a unit rule makes its test fail."""
    ok = True
    for name, filt, env in (
        ("kept", "held_without", {"DS_ARA_DEFAULTS_FAULT": "lose-kept"}),
        ("hostile", "a_hostile_file", {"DS_ARA_DEFAULTS_FAULT": "trust"}),
        ("one-role", "a_save_changes_one_role", {"DS_ARA_DEFAULTS_FAULT": "all-roles"}),
        ("note-stale", "a_changed_note", {"DS_ARA_PREPARE_FAULT": "fp-no-note"}),
        ("preset-clash", "a_name_whose_file_name", {"DS_ARA_DEFAULTS_FAULT": "preset-overwrite"}),
        ("backup", "a_failed_backup", {"DS_ARA_DEFAULTS_FAULT": "backup-ignore"}),
        ("kept-gone", "the_kept_values_of_a_removed_source", {"DS_ARA_DEFAULTS_FAULT": "keep-gone"}),
        ("kept-round", "a_source_with_no_result_that_is_not_in_a_round", {"DS_ARA_DEFAULTS_FAULT": "kept-by-round"}),
        ("kept-stale", "a_used_entry_is_removed", {"DS_ARA_DEFAULTS_FAULT": "kept-stale"}),
    ):
        rc, _text = unit_with_fault(f"defaults-unit-fault-{name}", filt, env)
        ok = pass_fail(f"defaults-unit-fault-{name}", rc != 0, f"the fault {env} made `cargo test {filt}` fail (rc={rc})") and ok
    return ok



def layer_run(label: str, filt: str, fault: dict | None = None, with_dir: bool = True) -> tuple[int, str]:
    """The unit tests of the command layer (`defaults_command` in editor.rs) with the folder of this run below out/ (S6)."""
    d = OUT / f"user-{label}"
    env = {**no_switch_env(), "ARA_API_DIR": str(DEPS / "ARA_SDK" / "ARA_API"), "LIBCLANG_PATH": f"{CLT}/usr/lib", "CARGO_TARGET_DIR": str(HERE / "target"), **(fault or {})}
    if with_dir:
        env["DS_ARA_USER_DIR"] = str(d)
    guard_user_dir(["cargo"], env, label)
    return run(["cargo", "test", "--locked", "--features", "test-switches", "--lib", filt, "--", "--ignored", "--nocapture"], env=env, log=f"{label}.log", cwd=HERE)


def stage_editor_defaults() -> bool:
    """The commands of the page through the real command layer: get, save a role, a preset, a colliding name (refused, the text reaches the page), delete, a hostile name,
    and the answer without a folder."""
    rc, text = layer_run("editor-defaults", "defaults_layer_get_save")
    # the tests are #[ignore]d outside this stage and refuse to run without the folder: a run without DS_ARA_USER_DIR must FAIL (a filter that cannot fire)
    rc_none, _t = layer_run("editor-defaults-nodir", "defaults_layer_get_save", with_dir=False)
    ran = "defaults_layer_get_save_preset_collision_delete_and_hostile_name ... ok" in text
    ok = pass_fail("editor-defaults-needs-dir", rc_none != 0, f"the same test without DS_ARA_USER_DIR fails, it does not pass early (rc={rc_none})")
    ok = pass_fail("editor-defaults", rc == 0 and ran, f"get, save role, preset save, collision refused, delete, hostile name `../x`: rc={rc}, the test ran and passed: {ran}") and ok
    rc2, text2 = layer_run("editor-defaults-nofolder", "defaults_layer_answers", with_dir=False)
    ran2 = "defaults_layer_answers_without_a_folder_and_never_panics ... ok" in text2
    return pass_fail("editor-defaults-nofolder", rc2 == 0 and ran2, f"with no folder the page gets `nofolder` and a save says why: rc={rc2}, passed: {ran2}") and ok


def stage_editor_defaults_fault() -> bool:
    ok = True
    for name, fault, why in (
        ("all-roles", {"DS_ARA_DEFAULTS_FAULT": "all-roles"}, "Save as default writes every role, so the file holds more than the saved role"),
        ("trust", {"DS_ARA_DEFAULTS_FAULT": "trust"}, "the range check is off, so a note 300 of a hostile file reaches the page"),
        ("overwrite", {"DS_ARA_DEFAULTS_FAULT": "preset-overwrite"}, "a colliding file name replaces the other preset without a question"),
    ):
        rc, text = layer_run(f"editor-defaults-fault-{name}", "defaults_layer_get_save", fault)
        ok = pass_fail(f"editor-defaults-fault-{name}", rc != 0 and "panicked" in text, f"the fault {fault} ({why}) made the command-layer test fail (rc={rc})") and ok
    return ok


def export_store_facts(label: str, extra: dict | None = None) -> tuple[bool, str]:
    """S10: the plugin sets the export settings as the page does (scenario), the archive is stored, and a NEW process restores it."""
    archive = OUT / f"{label}.araarchive"
    if archive.exists():
        archive.unlink()
    want = {"formats": ["midi", "spikes"], "midi_mode": "single", "compensate": False}
    rc, _text, _lines = host_run(
        f"{label}-store", CLAP, ["store", "--files", *fixture_args(FILES), "--archive", str(archive)], f"cache-{label}-store",
        extra={"DS_ARA_SCENARIO": "export-last:" + json.dumps(want), **(extra or {})},
    )
    stored = json.loads(archive.read_text()).get("export") if archive.exists() else None
    back = export_facts(f"{label}-restore", archive, "keep") if archive.exists() else {"rc": 1, "last": None, "comp": None}
    last = back["last"] or {}
    good = rc == 0 and stored == want and back["rc"] == 0 and last.get("formats") == want["formats"] and last.get("midi_mode") == "single" and last.get("compensate") is False
    return good, f"store rc={rc}, the archive holds export {stored}, a new process restored {last.get('formats')} {last.get('midi_mode')} compensate {last.get('compensate')}"


def stage_doc_archive_export_store() -> bool:
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("doc-archive-export-store", False, "no host or no CLAP: run check.sh build first")
    ok, ev = export_store_facts("doc-archive-export-store")
    return pass_fail("doc-archive-export-store", ok, "an export setting reaches the archive and comes back in a new process: " + ev)


def stage_doc_archive_export_store_fault() -> bool:
    if not HOST.exists() or not CLAP.exists():
        return pass_fail("doc-archive-export-store-fault", False, "no host or no CLAP: run check.sh build first")
    ok, ev = export_store_facts("doc-archive-export-store-drop", {"DS_ARA_DEFAULTS_FAULT": "export-drop"})
    return pass_fail("doc-archive-export-store-fault", not ok, f"the fault run (the store drops the export settings) was refused by the check: {ev}")


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
    "doc-rounds": stage_doc_rounds,
    "doc-rounds-fault": stage_doc_rounds_fault,
    "doc-two": stage_doc_two,
    "doc-two-fault": stage_doc_two_fault,
    "doc-archive": stage_doc_archive,
    "doc-archive-export": stage_doc_archive_export,
    "doc-archive-export-fault": stage_doc_archive_export_fault,
    "defaults-new-track": stage_defaults_new_track,
    "defaults-new-track-fault": stage_defaults_new_track_fault,
    "defaults-hostile": stage_defaults_hostile,
    "defaults-hostile-fault": stage_defaults_hostile_fault,
    "defaults-real-path": stage_defaults_real_path,
    "defaults-real-path-fault": stage_defaults_real_path_fault,
    "defaults-writers": stage_defaults_writers,
    "defaults-writers-fault": stage_defaults_writers_fault,
    "defaults-standalone-control": stage_defaults_standalone_control,
    "editor-defaults": stage_editor_defaults,
    "editor-defaults-fault": stage_editor_defaults_fault,
    "doc-archive-export-store": stage_doc_archive_export_store,
    "doc-archive-export-store-fault": stage_doc_archive_export_store_fault,
    "defaults-units": stage_defaults_units,
    "defaults-units-fault": stage_defaults_units_fault,
    "doc-archive-fault": stage_doc_archive_fault,
    "doc-destroy": stage_doc_destroy,
    "doc-destroy-fault": stage_doc_destroy_fault,
    "doc-close": stage_doc_close,
    "doc-close-fault": stage_doc_close_fault,
    "doc-id-hostile": stage_doc_id_hostile,
    "doc-id-hostile-fault": stage_doc_id_hostile_fault,
    "doc-same-id": stage_doc_same_id,
    "doc-same-id-fault": stage_doc_same_id_fault,
    "doc-restore-busy": stage_doc_restore_busy,
    "doc-restore-busy-fault": stage_doc_restore_busy_fault,
    "doc-store-window": stage_doc_store_window,
    "doc-store-window-fault": stage_doc_store_window_fault,
    "doc-store-early-fault": stage_doc_store_early_fault,
    "doc-restore-repair": stage_doc_restore_repair,
    "doc-restore-repair-fault": stage_doc_restore_repair_fault,
    "doc-readd": stage_doc_readd,
    "doc-readd-fault": stage_doc_readd_fault,
    "doc-readd-unknown-fault": stage_doc_readd_unknown_fault,
    "doc-retry": stage_doc_retry,
    "doc-retry-fault": stage_doc_retry_fault,
    "doc-rerequest": stage_doc_rerequest,
    "doc-rerequest-fault": stage_doc_rerequest_fault,
    "doc-errors": stage_doc_errors,
    "doc-errors-fault": stage_doc_errors_fault,
    "doc-regions-change": stage_doc_regions_change,
    "doc-regions-change-fault": stage_doc_regions_change_fault,
    "doc-panic": stage_doc_panic,
    "doc-source-names": stage_doc_source_names,
    "doc-source-names-fault": stage_doc_source_names_fault,
    "doc-region-move": stage_doc_region_move,
    "doc-region-move-fault": stage_doc_region_move_fault,
    "doc-late-audio": stage_doc_late_audio,
    "doc-late-audio-fault": stage_doc_late_audio_fault,
    "doc-late-audio-arrives": stage_doc_late_audio_arrives,
    "doc-late-audio-arrives-fault": stage_doc_late_audio_arrives_fault,
    "doc-panic-fault": stage_doc_panic_fault,
    "no-request": stage_no_request,
    "no-request-fault": stage_no_request_fault,
    "request-order": stage_request_order,
    "request-order-fault": stage_request_order_fault,
    "no-request-group": stage_no_request_group,
    "no-request-group-fault": stage_no_request_group_fault,
    "no-request-restored": stage_no_request_restored,
    "no-request-restored-fault": stage_no_request_restored_fault,
    "no-request-update": stage_no_request_update,
    "no-request-update-fault": stage_no_request_update_fault,
    "no-request-retry": stage_no_request_retry,
    "no-request-retry-fault": stage_no_request_retry_fault,
    "no-request-retry-limit": stage_no_request_retry_limit,
    "no-request-retry-limit-fault": stage_no_request_retry_limit_fault,
    "no-request-cancel": stage_no_request_cancel,
    "no-request-cancel-fault": stage_no_request_cancel_fault,
    "no-request-spawn": stage_no_request_spawn,
    "no-request-spawn-fault": stage_no_request_spawn_fault,
    "no-request-update-between": stage_no_request_update_between,
    "no-request-update-between-fault": stage_no_request_update_between_fault,
    "no-request-update-failed-read": stage_no_request_update_failed_read,
    "no-request-update-failed-read-fault": stage_no_request_update_failed_read_fault,
    "no-request-files": stage_no_request_files,
    "no-request-files-fault": stage_no_request_files_fault,
    "no-request-update-failed-for-good": stage_no_request_update_failed_for_good,
    "no-request-update-failed-for-good-fault": stage_no_request_update_failed_for_good_fault,
    "no-request-update-twice": stage_no_request_update_twice,
    "no-request-update-twice-fault": stage_no_request_update_twice_fault,
    "state": stage_state,
    "state-fault": stage_state_fault,
    "editor-no-request": stage_editor_no_request,
    "editor-no-request-fault": stage_editor_no_request_fault,
    "switches": stage_switches,
    "switches-fault": stage_switches_fault,
    "no-unwrap": stage_no_unwrap,
    "no-unwrap-fault": stage_no_unwrap_fault,
    "doc-home": stage_doc_home,
    "doc-home-fault": stage_doc_home_fault,
    "conformance": stage_conformance,
    "files": stage_files,
    "pluginval": stage_pluginval,
    "pluginval-fault": stage_pluginval_fault,
    "bundle-version": stage_bundle_version,
    "bundle-version-fault": stage_bundle_version_fault,
    "bundle-sign": stage_bundle_sign,
    "bundle-sign-fault": stage_bundle_sign_fault,
    "delivery-switches": stage_delivery_switches,
    "delivery-switches-fault": stage_delivery_switches_fault,
    "validate": stage_validate,
    "universal": stage_universal,
    "standalone": stage_standalone,
    "crate": stage_crate,
    "workspace": stage_workspace,
    "make-fixture": stage_make_fixture,
    "editor-open": stage_editor_open,
    "editor-fault": stage_editor_fault,
    "editor-gui": stage_editor_gui,
    "editor-gui-fault": stage_editor_gui_fault,
    "editor-shell": stage_editor_shell,
    "editor-shell-fault": stage_editor_shell_fault,
    "editor-edit": stage_editor_edit,
    "editor-edit-fault": stage_editor_edit_fault,
    "editor-threads": stage_editor_threads,
    "editor-threads-fault": stage_editor_threads_fault,
    "editor-follow": stage_editor_follow,
    "editor-follow-fault": stage_editor_follow_fault,
    "editor-refuse": stage_editor_refuse,
    "editor-refuse-fault": stage_editor_refuse_fault,
    "editor-race": stage_editor_race,
    "editor-race-fault": stage_editor_race_fault,
    "editor-page-copy": stage_editor_page_copy,
    "editor-page-copy-fault": stage_editor_page_copy_fault,
    "editor-vst3-view": stage_editor_vst3_view,
    "editor-vst3-view-fault": stage_editor_vst3_view_fault,
    "editor-edit-sync": stage_editor_edit_sync,
    "editor-edit-sync-fault": stage_editor_edit_sync_fault,
    "editor-edit-destroy": stage_editor_edit_destroy,
    "editor-edit-destroy-fault": stage_editor_edit_destroy_fault,
    "editor-regions-keep": stage_editor_regions_keep,
    "editor-regions-keep-fault": stage_editor_regions_keep_fault,
    "editor-race-destroy": stage_editor_race_destroy,
    "editor-race-destroy-fault": stage_editor_race_destroy_fault,
    "editor-shell-report": stage_editor_shell_report,
    "editor-shell-report-fault": stage_editor_shell_report_fault,
    "editor-shell-functions": stage_editor_shell_functions,
    "editor-shell-functions-fault": stage_editor_shell_functions_fault,
    "transport-handover": stage_transport_handover,
    "transport-handover-fault": stage_transport_handover_fault,
    "editor-transport": stage_editor_transport,
    "editor-transport-fault": stage_editor_transport_fault,
    "editor-transport-measure": stage_editor_transport_measure,
    "editor-transport-measure-fault": stage_editor_transport_measure_fault,
    "editor-transport-none": stage_editor_transport_none,
    "editor-transport-none-fault": stage_editor_transport_none_fault,
    "editor-transport-deactivate": stage_editor_transport_deactivate,
    "editor-transport-deactivate-fault": stage_editor_transport_deactivate_fault,
    "playback-map": stage_playback_map,
    "playback-map-fault": stage_playback_map_fault,
    "editor-playback": stage_editor_playback,
    "editor-playback-fault": stage_editor_playback_fault,
    "editor-playback-none": stage_editor_playback_none,
    "editor-playback-none-fault": stage_editor_playback_none_fault,
    "editor-playback-partial": stage_editor_playback_partial,
    "editor-playback-partial-fault": stage_editor_playback_partial_fault,
    "editor-playback-tracks": stage_editor_playback_tracks,
    "editor-names": stage_editor_names,
    "editor-song-rows": stage_editor_song_rows,
    "editor-song-rows-fault": stage_editor_song_rows_fault,
    "delivery": stage_delivery,
    "drag-source": stage_drag_source,
    "drag-source-fault": stage_drag_source_fault,
    "deliver": stage_deliver,
    "deliver-fault": stage_deliver_fault,
    "export-dialog": stage_export_dialog,
    "export-rows": stage_export_rows,
    "export-rows-fault": stage_export_rows_fault,
    "export-dialog-fault": stage_export_dialog_fault,
    "delivery-release": stage_delivery_release,
    "delivery-fault": stage_delivery_fault,
    "editor-names-fault": stage_editor_names_fault,
    "editor-leftovers": stage_editor_leftovers,
    "editor-leftovers-fault": stage_editor_leftovers_fault,
    "editor-firstopen": stage_editor_firstopen,
    "editor-firstopen-fault": stage_editor_firstopen_fault,
    "editor-roles": stage_editor_roles,
    "restore-settings": stage_restore_settings,
    "restore-settings-fault": stage_restore_settings_fault,
    "editor-roles-fault": stage_editor_roles_fault,
    "editor-playback-tracks-fault": stage_editor_playback_tracks_fault,
    "editor-playback-drag": stage_editor_playback_drag,
    "editor-playback-drag-fault": stage_editor_playback_drag_fault,
    "editor-playback-late": stage_editor_playback_late,
    "editor-playback-late-fault": stage_editor_playback_late_fault,
    "editor-playback-refused": stage_editor_playback_refused,
    "editor-playback-refused-fault": stage_editor_playback_refused_fault,
    "editor-hit": stage_editor_hit,
    "editor-hit-fault": stage_editor_hit_fault,
    "editor-geometry": stage_editor_geometry,
    "editor-geometry-fault": stage_editor_geometry_fault,
    "editor-menu": stage_editor_menu,
    "editor-selection": stage_editor_selection,
    "editor-selection-fault": stage_editor_selection_fault,
    "editor-menu-fault": stage_editor_menu_fault,
    "editor-keys": stage_editor_keys,
    "editor-keys-fault": stage_editor_keys_fault,
    "editor-keys-twin": stage_editor_keys_twin,
    "editor-keys-twin-fault": stage_editor_keys_twin_fault,
}
ORDER = [
    "fixture", "reference", "tables", "tables-fault", "identity-clap", "identity-vst3", "regions", "archive",
    "archive-paths-fault", "archive-restore-pathname",
    "threads", "doc-service", "doc-service-fault", "doc-tracks", "doc-tracks-fault", "doc-rounds", "doc-rounds-fault", "doc-two", "doc-two-fault",
    "doc-archive", "doc-archive-fault", "doc-archive-export", "doc-archive-export-fault", "doc-archive-export-store", "doc-archive-export-store-fault", "editor-defaults", "editor-defaults-fault", "defaults-units", "defaults-units-fault", "defaults-new-track", "defaults-new-track-fault", "defaults-hostile", "defaults-hostile-fault", "defaults-real-path", "defaults-real-path-fault", "defaults-writers", "defaults-writers-fault", "defaults-standalone-control", "doc-destroy", "doc-destroy-fault", "doc-close", "doc-close-fault", "doc-home", "doc-home-fault",
    "doc-id-hostile", "doc-id-hostile-fault", "doc-same-id", "doc-same-id-fault", "doc-restore-busy", "doc-restore-busy-fault", "doc-store-window", "doc-store-window-fault", "doc-store-early-fault", "doc-restore-repair", "doc-restore-repair-fault", "doc-readd", "doc-readd-fault", "doc-readd-unknown-fault",
    "doc-retry", "doc-retry-fault", "doc-rerequest", "doc-rerequest-fault", "doc-errors", "doc-errors-fault",
    "doc-regions-change", "doc-regions-change-fault", "doc-panic", "doc-panic-fault",
    "doc-source-names", "doc-source-names-fault", "doc-region-move", "doc-region-move-fault", "doc-late-audio", "doc-late-audio-fault", "doc-late-audio-arrives", "doc-late-audio-arrives-fault",
    "no-request", "no-request-fault", "request-order", "request-order-fault", "no-request-group", "no-request-group-fault",
    "no-request-restored", "no-request-restored-fault",
    "no-request-update", "no-request-update-fault", "no-request-retry", "no-request-retry-fault", "no-request-retry-limit", "no-request-retry-limit-fault",
    "no-request-cancel", "no-request-cancel-fault", "no-request-spawn", "no-request-spawn-fault",
    "no-request-update-between", "no-request-update-between-fault", "no-request-update-failed-read", "no-request-update-failed-read-fault",
    "no-request-update-failed-for-good", "no-request-update-failed-for-good-fault", "no-request-update-twice", "no-request-update-twice-fault",
    "no-request-files", "no-request-files-fault", "state", "state-fault",
    "editor-open", "editor-fault", "editor-gui", "editor-gui-fault", "editor-shell", "editor-shell-fault",
    "editor-edit", "editor-edit-fault", "editor-threads", "editor-threads-fault",
    "editor-follow", "editor-follow-fault", "editor-refuse", "editor-refuse-fault", "editor-race", "editor-race-fault", "editor-page-copy", "editor-page-copy-fault", "editor-vst3-view", "editor-vst3-view-fault",
    "editor-edit-sync", "editor-edit-sync-fault", "editor-edit-destroy", "editor-edit-destroy-fault", "editor-regions-keep", "editor-regions-keep-fault", "editor-race-destroy", "editor-race-destroy-fault", "editor-shell-functions", "editor-shell-functions-fault", "editor-shell-report", "editor-shell-report-fault", "editor-no-request", "editor-no-request-fault",
    "transport-handover", "transport-handover-fault", "editor-transport", "editor-transport-fault",
    "editor-transport-measure", "editor-transport-measure-fault", "editor-transport-none", "editor-transport-none-fault",
    "editor-transport-deactivate", "editor-transport-deactivate-fault",
    "playback-map", "playback-map-fault", "editor-playback", "editor-playback-fault", "editor-playback-none", "editor-playback-none-fault",
    "editor-playback-partial", "editor-playback-partial-fault", "editor-playback-refused", "editor-playback-refused-fault",
    "editor-playback-drag", "editor-playback-drag-fault", "editor-playback-late", "editor-playback-late-fault",
    "editor-playback-tracks", "editor-playback-tracks-fault",
    "editor-hit", "editor-hit-fault", "editor-geometry", "editor-geometry-fault", "editor-menu", "editor-menu-fault",
    "editor-selection", "editor-selection-fault",
    "editor-keys", "editor-keys-fault", "editor-keys-twin", "editor-keys-twin-fault",
    "editor-names", "editor-names-fault", "editor-song-rows", "editor-song-rows-fault", "editor-leftovers", "editor-leftovers-fault", "editor-firstopen", "editor-firstopen-fault", "editor-roles", "editor-roles-fault", "restore-settings", "restore-settings-fault",
    "drag-source", "drag-source-fault", "delivery", "delivery-release", "delivery-fault", "export-dialog", "export-dialog-fault", "export-rows", "export-rows-fault", "deliver", "deliver-fault",
    "switches", "switches-fault", "no-unwrap", "no-unwrap-fault",
    "delivery-switches", "delivery-switches-fault", "bundle-version", "bundle-version-fault",
    "conformance", "files", "pluginval", "pluginval-fault", "validate", "universal", "bundle-sign", "bundle-sign-fault", "standalone",
    "crate", "workspace",
]


def main() -> int:
    stages = sys.argv[1:] or ["all"]
    if stages == ["write-readme"]:
        write_readme()
        print("README.md: the switches block is made from switches.json")
        return 0
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
