"""P3-9a: the exit flow E2, E3, E4, E6, E7, E8 and E9 through the app's commands.

    cargo build -p drumspike-devtools --bin ds-bridge
    uv run --project . python tools/parity/exit_flow.py <session_dir> <work_dir> \\
        [--bridge target/debug/ds-bridge] [--json out.json]

<session_dir> is the output of tools/parity/make_session.py (30 tracks, 96 kHz, 24 bit,
5 minutes). <work_dir> must be new or empty and outside the repository: the script clones the
session's WAV files into <work_dir>/audio (APFS clones, `cp -c`), so the generated folder never
moves. The app side is `ds-bridge` (the same `Service` methods as the Tauri commands, one HTTP
call per command). The server side was server.py in-process (FastAPI TestClient) on clones of
the project folder; P3-12 froze its answers per step in tests/golden/frozen/exit_flow.json (track
order, status, source modes, a digest of the (t, v, rise) list per track, the SHA-256 of each MIDI
file, the SHA-256 of each spike file of drumspike_core.write_spike_wav), keyed by the SHA-256 of
the session manifest and the digest of the app's events. Nothing is written into the repository.

Steps and rules (exit table of docs/archive/phase3-desktop-app-prompt.md):
E2  project_new: the folder holds the document and the 4 subfolders; the server opens it.
E3  tracks_scan + tracks_add (referenced, with the peak cache): 30 / 30 online, every record
    `referenced`, 30 peaks.bin; the time of tracks_add; the SHA-256 of the 30 sources before
    the first step and after the last step are equal.
E4  analyze_start for all tracks: 30 / 30 done with the DSP detector; the time; the events
    equal (t, v, rise) the server's Rust mode (DRUMSPIKE_RUST=1) on a clone of the project.
E6  project_save: a new inode for the document (rename), no `.tmp` file, the previous document
    bytes in Backups/; the server (Python mode) opens it with the same events.
E7  close, move the audio folder, reopen: 30 / 30 offline; one relink_search of the parent
    folder relinks 30 / 30; events unchanged.
E8  project_save_as with copy_audio: 30 files in Audio Files/, every record `copied`; after the
    original audio folder is renamed the copy opens with 30 / 30 online (app and server).
E9  export_all: 30 MIDI files byte-identical to the server's POST /api/export of the same
    events; 30 spike WAVs byte-identical to drumspike_core.write_spike_wav, bext time reference
    equal to the source's (the generated files have no bext chunk: None on both sides).
Exit status 1 when a rule fails.
"""
import argparse
import hashlib
import json
import os
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(ROOT / "tests"))
sys.path.insert(0, str(ROOT / "tools" / "parity"))
SUBDIRS = ("Audio Files", "Analysis", "Exports", "Backups")
import frozen  # noqa: E402

STORE = frozen.store("exit_flow")


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 22), b""):
            h.update(block)
    return h.hexdigest()


def ev_key(events):
    """The event fields the exit rows compare: t, v, rise."""
    return [(e.get("t"), e.get("v"), e.get("rise")) for e in events]


def events_digest(events):
    return frozen.digest(ev_key(events))


def server_frozen(step, mode, op, session_sha, extra=None):
    """The frozen answer of the retired server for one step (P3-12)."""
    inputs = {"session": session_sha, "step": step, "mode": mode, "op": op, **(extra or {})}
    return STORE.ref(f"{step}/{mode}/{op}", inputs)


# ---------- the app side (ds-bridge) ----------

class Bridge:
    def __init__(self, binary, env):
        self.p = subprocess.Popen([binary, "--port", "0"], stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                  env=env, text=True, cwd=ROOT)
        line = self.p.stdout.readline()
        if "listening on" not in line:
            raise SystemExit(f"ds-bridge did not start: {line!r}")
        self.url = line.strip().split()[-1].rstrip("/")

    def __call__(self, cmd, **args):
        req = urllib.request.Request(f"{self.url}/invoke/{cmd}", data=json.dumps(args).encode(),
                                     headers={"Content-Type": "application/json"}, method="POST")
        try:
            with urllib.request.urlopen(req, timeout=900) as r:
                body = r.read()
        except urllib.error.HTTPError as e:
            raise RuntimeError(f"{cmd}: HTTP {e.code} {e.read()[:400]!r}") from None
        return json.loads(body)

    def stop(self):
        self.p.terminate()
        self.p.wait(10)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("session")
    ap.add_argument("work")
    ap.add_argument("--bridge", default=str(ROOT / "target" / "debug" / "ds-bridge"))
    ap.add_argument("--json")
    a = ap.parse_args()
    from check_midi import bext_time_reference
    from check_spike import source_format

    session, work = Path(a.session).resolve(), Path(a.work).resolve()
    if ROOT in work.parents:
        raise SystemExit("the work folder must be outside the repository")
    if work.exists() and any(work.iterdir()):
        raise SystemExit(f"{work} is not empty")
    manifest = json.loads((session / "session.json").read_text())
    session_sha = sha256(session / "session.json")
    rows, bad = {}, []

    def need(row, cond, msg):
        if not cond:
            bad.append(f"{row}: {msg}")
            print(f"  FAIL {row}: {msg}")

    audio = work / "audio"
    audio.mkdir(parents=True)
    wavs = sorted(p for p in session.glob("*.wav"))
    for w in wavs:
        subprocess.run(["cp", "-c", str(w), str(audio / w.name)], check=True)
    srcs = sorted(audio.glob("*.wav"))
    t0 = time.perf_counter()
    sha_before = {p.name: sha256(p) for p in srcs}
    print(f"{len(srcs)} sources cloned into {audio}, SHA-256 in {time.perf_counter() - t0:.1f} s")

    env = dict(os.environ, DRUMSPIKE_HOME=str(work / "app-home"), DRUMSPIKE_PROJECTS=str(work / "projects"),
               DRUMSPIKE_AUDIO="null", DRUMSPIKE_AUTOSAVE_SECONDS="1000000",
               DRUMSPIKE_REFINE_RULE="1",   # P4-12: the frozen E4 events are the engine's refinement (version 1)
               DRUMSPIKE_ROLL_RULE="0",     # P4-19: and the engine had no roll fill
               DRUMSPIKE_FLAM_RULE="0",     # P4-13 stage B: nor the flam rule
               DRUMSPIKE_KICK_RESCUE="0",   # P4-10 stage 5: nor the kick rescue
               DRUMSPIKE_SNARE_RESCUE="0")  # P4-10 stage 7: nor the snare rescue
    br = Bridge(a.bridge, env)
    scratch = work / "scratch"
    scratch.mkdir()
    try:
        info = br("app_info")
        print(f"bridge {br.url}: core {info.get('core_version')}, detector {info.get('detector')}, model ok {(info.get('model') or {}).get('ok')}")

        # E2 new project
        t = time.perf_counter()
        s = br("project_new", name="Exit Run", dir=str(work / "projects"))
        e2_ms = (time.perf_counter() - t) * 1000
        sid, folder, doc = s["sid"], Path(s["folder"]), Path(s["document"])
        subs = [d for d in SUBDIRS if (folder / d).is_dir()]
        srv = server_frozen("E2", "python", "open", session_sha)
        rows["E2"] = {"ms": round(e2_ms, 1), "document": doc.is_file(), "subfolders": subs, "server_tracks": len(srv["order"])}
        need("E2", doc.is_file() and len(subs) == 4 and srv["order"] == [], f"{rows['E2']}")
        print("E2", rows["E2"])

        # E3 import referenced with the peak cache
        t = time.perf_counter()
        scan = br("tracks_scan", paths=[str(audio)])
        scan_ms = (time.perf_counter() - t) * 1000
        files = scan["files"]
        t = time.perf_counter()
        add = br("tracks_add", sid=sid, paths=[f["path"] for f in files], roles=[f["role"] for f in files],
                 notes=[f["note"] for f in files], copy=False)
        add_ms = (time.perf_counter() - t) * 1000
        tg = br("tracks_get", sid=sid)
        online = [n for n, tr in tg["tracks"].items() if tr.get("status") == "ok"]
        peaks = list((folder / "Analysis").glob("*/peaks.bin"))
        rows["E3"] = {"scan_ms": round(scan_ms, 1), "tracks_add_ms": round(add_ms, 1),
                      "peaks_ms": add.get("peaks_ms"), "added": len(add["added"]), "rejected": len(add.get("rejected") or []),
                      "online": len(online), "peaks_bin": len(peaks),
                      "roles": sorted({f["role"] for f in files})}
        print("E3", rows["E3"])

        # E4 analyse all
        t = time.perf_counter()
        br("analyze_start", sid=sid)
        while True:
            st = br("analyze_status", sid=sid)
            if st["state"] == "done":
                break
            time.sleep(0.05)
        e4_s = time.perf_counter() - t
        done = [n for n, x in st["tracks"].items() if x["state"] == "done"]
        dets = sorted({x.get("detector") for x in st["tracks"].values()})
        app_events = {n: br("events_get", sid=sid, name=n)["events"] for n in tg["order"]}
        hits = sum(len(v) for v in app_events.values())
        # the known hits of make_session.py: a detected hit within 2 ms of a known hit counts
        truth = {t_["file"].rsplit(".", 1)[0]: t_["hit_times_s"] for t_ in manifest["tracks"]}
        found = known = 0
        for n, evs in app_events.items():
            ts = sorted(e["t"] for e in evs)
            for h in truth.get(n, []):
                known += 1
                found += any(abs(x - h) <= 0.002 for x in ts)

        # E6 save (before the server's analysis, so that the server reads the saved document)
        doc_before = doc.read_bytes()
        ino_before = doc.stat().st_ino
        backups_before = set(os.listdir(folder / "Backups"))
        t = time.perf_counter()
        br("project_save", sid=sid)
        e6_ms = (time.perf_counter() - t) * 1000
        new_backups = sorted(set(os.listdir(folder / "Backups")) - backups_before)
        prev_in_backups = any((folder / "Backups" / b).read_bytes() == doc_before for b in new_backups)
        tmp_left = [p.name for p in folder.rglob("*.tmp")]
        saved = json.loads(doc.read_text())
        modes = {n: tr["source"]["mode"] for n, tr in saved["tracks"].items()}
        rows["E3"]["referenced"] = sum(m == "referenced" for m in modes.values())
        need("E3", len(add["added"]) == 30 and len(online) == 30 and len(peaks) == 30
             and rows["E3"]["referenced"] == 30, f"{rows['E3']}")

        app_digests = {n: events_digest(app_events[n]) for n in tg["order"]}
        srv_open = server_frozen("E6", "python", "open", session_sha, {"events": app_digests})
        srv_same = sum(srv_open["events"][n] == app_digests[n] for n in tg["order"])
        rows["E6"] = {"ms": round(e6_ms, 1), "new_inode": doc.stat().st_ino != ino_before, "tmp_left": tmp_left,
                      "new_backups": new_backups, "previous_document_in_backups": prev_in_backups,
                      "server_status_ok": sum(v == "ok" for v in srv_open["status"].values()),
                      "server_events_equal": srv_same}
        need("E6", rows["E6"]["new_inode"] and not tmp_left and prev_in_backups and srv_same == 30
             and rows["E6"]["server_status_ok"] == 30, f"{rows['E6']}")
        print("E6", rows["E6"])

        srv_an = server_frozen("E4", "rust", "analyze", session_sha)
        equal = [n for n in tg["order"] if srv_an["events"][n] == app_digests[n]]
        diff = [n for n in tg["order"] if n not in equal]
        rows["E4"] = {"seconds": round(e4_s, 2), "done": len(done), "detectors": dets, "hits": hits,
                      "known_hits_found_2ms": f"{found} / {known}",
                      "server_rust_detectors": srv_an["detectors"],
                      "events_equal_to_server_rust": len(equal), "differ": diff[:5]}
        need("E4", len(done) == 30 and dets == ["rust-dsp"] and len(equal) == 30, f"{rows['E4']}")
        print("E4", rows["E4"])

        # E7 relink after the audio folder moved
        br("project_close", sid=sid)
        moved = work / "audio-moved"
        audio.rename(moved)
        t = time.perf_counter()
        s = br("project_open", path=str(folder))
        open_ms = (time.perf_counter() - t) * 1000
        sid = s["sid"]
        tg = br("tracks_get", sid=sid)
        offline = [n for n, tr in tg["tracks"].items() if tr.get("status") != "ok"]
        rl = br("relink_search", sid=sid, folder=str(work))
        tg2 = br("tracks_get", sid=sid)
        online2 = [n for n, tr in tg2["tracks"].items() if tr.get("status") == "ok"]
        ev2 = {n: br("events_get", sid=sid, name=n)["events"] for n in tg2["order"]}
        same = sum(ev_key(ev2[n]) == ev_key(app_events[n]) for n in tg2["order"])
        by = {}
        for f in rl["found"]:
            by[f["by"]] = by.get(f["by"], 0) + 1
        br("project_save", sid=sid)
        rows["E7"] = {"reopen_ms": round(open_ms, 1), "offline_after_move": len(offline), "relinked": rl["relinked"],
                      "found_by": by, "ambiguous": len(rl["ambiguous"]), "missing": len(rl["missing"]),
                      "search_ms": round(rl["elapsed_ms"], 1), "online_after": len(online2), "events_unchanged": same}
        need("E7", len(offline) == 30 and rl["relinked"] == 30 and len(online2) == 30 and same == 30, f"{rows['E7']}")
        print("E7", rows["E7"])

        # E8 Save As with copied audio
        t = time.perf_counter()
        r = br("project_save_as", sid=sid, name="Exit Run Copy", dir=str(work / "projects"), copy_audio=True)
        while True:
            js = br("job_status", sid=sid)
            if js.get("state") in ("done", "cancelled", "error", "failed"):
                break
            time.sleep(0.05)
        e8_s = time.perf_counter() - t
        summary = js.get("result") or {}
        copy_folder = Path(summary.get("folder") or work / "projects" / "Exit Run Copy")
        copy_doc = Path(summary.get("document") or next(copy_folder.glob("*.drumspike")))
        n_audio = len([p for p in (copy_folder / "Audio Files").iterdir() if p.suffix == ".wav"])
        cdoc = json.loads(copy_doc.read_text())
        copied = sum(tr["source"]["mode"] == "copied" for tr in cdoc["tracks"].values())
        br("project_close", sid=sid, force=True)
        moved2 = work / "audio-moved-again"
        moved.rename(moved2)
        s = br("project_open", path=str(copy_folder))
        sid = s["sid"]
        tg3 = br("tracks_get", sid=sid)
        online3 = [n for n, tr in tg3["tracks"].items() if tr.get("status") == "ok"]
        ev3 = {n: br("events_get", sid=sid, name=n)["events"] for n in tg3["order"]}
        same3 = sum(ev_key(ev3[n]) == ev_key(app_events[n]) for n in tg3["order"])
        srv_copy = server_frozen("E8", "python", "open", session_sha,
                                 {"events": app_digests})
        rows["E8"] = {"job_state": js.get("state"), "seconds": round(e8_s, 2), "first_answer": sorted(r)[:4],
                      "audio_files": n_audio, "copied_records": copied, "online_after_rename": len(online3),
                      "events_unchanged": same3,
                      "server_online": sum(v == "ok" for v in srv_copy["status"].values())}
        need("E8", n_audio == 30 and copied == 30 and len(online3) == 30 and same3 == 30
             and rows["E8"]["server_online"] == 30, f"{rows['E8']}")
        print("E8", rows["E8"])

        # E9 export all on the copy, against the server and the core
        t = time.perf_counter()
        ex = br("export_all", sid=sid)
        e9_ms = (time.perf_counter() - t) * 1000
        exp = copy_folder / "Exports"
        app_midi = {n: sha256(exp / f"{n}_spike.mid") for n in tg3["order"] if (exp / f"{n}_spike.mid").exists()}
        srv_ex = server_frozen("E9", "python", "export", session_sha, {"events": app_digests})
        srv_ex_rust = server_frozen("E9", "rust", "export", session_sha, {"events": app_digests})
        midi_same = sum(app_midi.get(n) == srv_ex["midi"][n] for n in tg3["order"])
        midi_same_rust = sum(app_midi.get(n) == srv_ex_rust["midi"][n] for n in tg3["order"])
        spike_same = tref_same = 0
        rates = set()
        for n in tg3["order"]:
            src = copy_folder / "Audio Files" / f"{n}.wav"
            if not src.exists():
                src = Path(tg3["tracks"][n].get("path") or "")
            fmt, sr, _ = source_format(src)
            rates.add(sr)
            times = [e["t"] for e in ev3[n]]
            want = STORE.ref(f"E9/spike/{n}", {"session": session_sha, "times": frozen.digest(times), "format": fmt})
            mine = exp / f"{n}_spike.wav"
            spike_same += mine.exists() and sha256(mine) == want
            tref_same += mine.exists() and bext_time_reference(str(mine)) == bext_time_reference(str(src))
        rows["E9"] = {"ms": round(e9_ms, 1), "command_ms": ex.get("elapsed_ms"), "midi": ex.get("midi"), "spikes": ex.get("spikes"),
                      "errors": len(ex.get("errors") or []), "midi_equal_server_python": midi_same,
                      "midi_equal_server_rust": midi_same_rust, "spike_equal_core": spike_same,
                      "bext_time_reference_equal": tref_same, "spike_rates": sorted(rates)}
        need("E9", ex.get("midi") == 30 and ex.get("spikes") == 30 and midi_same == 30 and spike_same == 30
             and tref_same == 30 and rates == {manifest["sr"]}, f"{rows['E9']}")
        print("E9", rows["E9"])
        br("project_close", sid=sid, force=True)
    finally:
        br.stop()

    moved2.rename(audio)   # the clone folder gets its first name back for the hash
    sha_after = {p.name: sha256(p) for p in sorted(audio.glob("*.wav"))}
    unchanged = sum(sha_after.get(k) == v for k, v in sha_before.items())
    rows["E3"]["sources_sha256_unchanged"] = f"{unchanged} / {len(sha_before)}"
    need("E3", unchanged == len(sha_before) == 30, f"sources changed: {unchanged} / {len(sha_before)}")
    print("sources SHA-256 unchanged", rows["E3"]["sources_sha256_unchanged"])
    out = {"pass": not bad, "rows": rows, "failures": bad, "bridge": a.bridge, "app_info": info}
    if a.json:
        Path(a.json).write_text(json.dumps(out, indent=1))
    print("PASS" if not bad else f"FAIL ({len(bad)})")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
