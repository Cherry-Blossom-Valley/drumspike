"""Check a self-test report (P3-7a) and print its numbers.

    uv run python apps/desktop/selftest/check_report.py REPORT.json --project PROJECT_FOLDER

Fails (exit 1) when the report did not pass, or when a MIDI file in `PROJECT_FOLDER/Exports/` differs from the
reference file for the events the page held: the retired server's file (pretty_midi), frozen in
tests/golden/frozen/midi.json, else the core's MIDI writer (P3-12). Only the run on the machine
that has the project can do the second check, so it lives here and not in the app."""
import argparse
import json
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(ROOT / "tests"))


def midi_reference(scenario, name, events, note):
    """(bytes, source) of the reference MIDI file for these events: the file the retired server wrote
    (engine._py_export_midi, pretty_midi), frozen in tests/golden/frozen/midi.json (P3-12), when the frozen run had
    the same events; else the file of the core's MIDI writer (`target/release/parity midi`), which equals the server's
    bytes on every frozen event list (tools/parity/check_midi.py)."""
    import frozen
    inputs = {"events": events, "note": note, "ppq": 24000, "tempo": 120.0}
    key = f"selftest/{scenario}/{name}/{frozen.digest(inputs)[:16]}"   # one entry per event list
    try:
        return bytes.fromhex(frozen.store("midi").ref(key, inputs)["hex"]), "frozen"
    except frozen.FrozenError:
        pass
    exe = ROOT / "target" / "release" / "parity"
    if not exe.is_file():
        raise SystemExit(f"no frozen MIDI file for {name} and no {exe} (cargo build --release -p drumspike-devtools)")
    with tempfile.TemporaryDirectory() as tmp:
        job, out = Path(tmp) / "job.json", Path(tmp) / "ref.mid"
        job.write_text(json.dumps(inputs))
        subprocess.run([str(exe), "midi", str(job), str(out)], check=True, capture_output=True)
        return out.read_bytes(), "core"


SOURCES = {}   # reference source ("frozen" or "core") -> number of tracks, for the summary line


def midi_mismatches(report, project):
    """[(track, why)] for every track whose exported file differs from the reference file."""
    bad = []
    for name, t in report.get("tracks", {}).items():
        if "events" not in t or "midi" not in t:
            bad.append((name, "no events or no midi record in the report"))
            continue
        out = Path(project) / t["midi"]["path"]
        ref, src = midi_reference(report.get("scenario"), name, [{"t": a, "v": b} for a, b in t["events"]], t["note"])
        SOURCES[src] = SOURCES.get(src, 0) + 1
        if not out.is_file():
            bad.append((name, f"{out} missing"))
        elif out.read_bytes() != ref:
            bad.append((name, f"bytes differ from the reference ({src})"))
    return bad


def play_lines(p):
    """The numbers of the play step (P3-5b): underruns, playhead error, mute latency, seeks."""
    if not p:
        return []
    e, mu = p.get("playhead_error_ms", {}), p.get("mute", {})
    lines = [f"  play: {p['seconds']} s, {p['callbacks']} callbacks, underruns {p['underruns']} ({p['underrun_frames']} frames), "
             f"stream errors {p['stream_errors']}, xruns {p.get('stream_xruns')}, notices {p.get('stream_warnings')}, device reopens {p['reopens']}, pre-gain peak {p['pre_gain_peak']:.3f}, "
             f"master after {p['master_after']}"]
    if e.get("n"):
        lines.append(f"  play: drawn playhead vs transport (n={e['n']}): median {e['median']} ms, p95 {e['p95']} ms, "
                     f"max {e['max']} ms; signed median {e.get('signed_median')} ms (+ = drawn ahead), model only: signed median {e.get('model_signed_median')} ms, p95 {e.get('model_abs_p95')} ms (including the drawing work of the frame: median {e.get('incl_frame_work_median')} ms, p95 {e.get('incl_frame_work_p95')} ms, max {e.get('incl_frame_work_max')} ms; frame interval median {p.get('frame_ms_median')} ms, p95 {p.get('frame_ms_p95')} ms) (status round trip median {e['status_rtt_median']} ms, max {e['status_rtt_max']} ms)")
    else:
        lines.append(f"  play: drawn playhead not measured: {e.get('note')}")
    lines.append(f"  play: {mu.get('n')} mute/solo changes, block latency median {mu.get('latency_ms_median')} ms, "
                 f"max {mu.get('latency_ms_max')} ms; inbound max {mu.get('inbound_ms_max')} ms; round trip max {mu.get('rtt_ms_max')} ms; inbound + block (click to heard) max {mu.get('total_ms_max')} ms")
    lines.append(f"  play: seek until playing again {p.get('seek_settle_ms')} ms")
    if p.get("meters"):
        mt = p["meters"]
        lines.append(f"  play: meter messages {mt.get('messages')} ({mt.get('per_s')} per s), master meter peak {mt.get('master_peak_max')}")
    return lines


def memory_lines(mem):
    """The numbers of the memory step (P3-6b): footprints of the WebContent and GPU processes, open to drawn, zoom sweeps."""
    if not mem:
        return []
    e = mem.get("engine", {})
    lines = [f"  memory: window {mem.get('window')}",
             f"  memory: engine {'worker' if e.get('remote') else 'page'} ({e.get('renderer')}), {mem.get('process_count')} new WebContent process(es)",
             f"  memory: WebContent footprint peak {mem.get('web_content_peak_mb')} MB (first {mem.get('web_content_first_mb')}, last {mem.get('web_content_last_mb')}, "
             f"RSS peak {mem.get('web_content_rss_peak_mb')}), GPU process peak {mem.get('gpu_peak_mb')} MB, peak numbers held by the page {mem.get('page_peak_numbers_max')}",
             (f"  memory: target {mem['target']['web_content_mb']} MB for the WebContent footprint: {'MET' if mem['target']['met'] else 'NOT MET'} over the whole run "
              f"(lowest sample {'under' if mem['target']['met_at_rest'] else 'not under'} the target)" if mem.get("target") else "  memory: no target set"),
             f"  memory: open to drawn (warm cache) {mem.get('open_to_drawn_ms')} ms, median {mem.get('open_to_drawn_median_ms')}, max {mem.get('open_to_drawn_max_ms')}"]
    for row in mem.get("samples", []):
        lines.append(f"    {row['t_s']:>7.1f} s {row['tag']:<22} WebContent {row['web_content_mb']:>7} MB  GPU {row['gpu_mb']:>7} MB  textures {row.get('textures')}  lane canvases with a store {row.get('canvases_with_store')}"
                     + (f"  strip {row['strip']}" if row.get("strip") else "")
                     + (f"  top: {row['top']}" if row.get("top") and row["tag"] in ("after_open", "after_reopen", "play_30s", "zoom_back") else ""))
    for d in mem.get("diag", []):
        lines.append(f"    diag {d['tag']:<32} WebContent {d['web_content_mb']:>7} MB  top: {d.get('top')}")
    for d in mem.get("diag_redraw", []):
        lines.append(f"    diag redraw at 30 Hz: {d['tag']:<30} " + (f"WebContent {d['web_content_mb']} MB" if "web_content_mb" in d else f"while running {d['while_running_mb']} MB, 5 s after {d['after_5s_mb']} MB"))
    for ab in mem.get("coalesce_ab", []):
        lines.append(f"  memory coalesce A/B: one fetch per track at a time {ab['coalesce']}: {ab.get('issued')} window requests in a 2 s sweep, {ab.get('drain_ms')} ms to answer the last one after the sweep, p99 {ab['p99_ms']} max {ab['max_ms']} ms, frames over 30 ms {ab['over_30ms']}, "
                     f"{ab['fetches']} window fetches ({ab['fetched_mb']} MB, median {ab['fetch_ms_median']} ms, max {ab['fetch_ms_max']} ms)")
    if "play" in mem:
        lines.append(f"  memory play = {json.dumps(mem['play'])}")
    for key in ("zoom_sweeps_before_play", "zoom_sweeps_after_play", "zoom_sweeps_after_transport_close"):
        for i, w in enumerate(mem.get(key, [])):
            lines.append(f"  memory {key}[{i}]: n {w.get('n')}, p50 {w.get('p50_ms')} p95 {w.get('p95_ms')} p99 {w.get('p99_ms')} max {w.get('max_ms')} ms, "
                         f"JS p99 {w.get('js_p99_ms')} max {w.get('js_max_ms')} ms, frames over 30 ms {w.get('over_30ms')} (JS under 5 ms: {w.get('over_30ms_js_under_5ms')}, with a window fetch in flight: {w.get('long_with_fetch')}), "
                         f"lane posts {w.get('lane_posts')}, window fetches {w.get('fetches')} ({w.get('fetched_mb')} MB, median {w.get('fetch_ms_median')} ms, max {w.get('fetch_ms_max')} ms)")
    return lines


def summary(report):
    m = report["metrics"]
    lines = [f"scenario {report['scenario']}: {'PASS' if report['pass'] else 'FAIL'}"]
    if isinstance(m.get("keys"), dict):   # #206: keys tested N, passed P, failed F per preset
        for p, s in m["keys"].items():
            lines.append(f"  keys {p}: keys tested {s['tested']}, passed {s['passed']}, failed {len(s['failed'])}")
            for f in s["failed"][:10]:
                lines.append(f"    FAIL {f}")
    for s in report["steps"]:
        lines.append(f"  {s['step']:<11} {'ok  ' if s['ok'] else 'FAIL'} {s['ms']:>9.1f} ms" + ("" if s["ok"] else f"  {s.get('error')}"))
    for key in ("cold_start_ms", "open_ms", "lane_progress", "analyze_ms", "hits_total", "tracks_done", "detectors",
                "menu", "menu_checks", "menu_window", "menu_new_items", "menu_tempo_map", "finder_open_ms", "menu_triggered", "quit_cancel", "quit_save_discard", "play_start_ms",
                "play_device", "play_tracks", "play_hits", "consolidate_ms", "save_as_copy", "drop_to_list_ms", "accept_to_lanes_ms", "export_ms", "project_name", "roles", "tracks_flow"):
        if key in m:
            lines.append(f"  {key} = {m[key]}")
    for key in ("frames_scroll", "frames_zoom", "frames_during_analyze", "frames_layout_reads", "frames_drawn",
                "edit_gl_draws", "edit_frames", "edit_layout_reads"):   # P3-14d: the frame scheduler's counters
        if key in m:
            lines.append(f"  {key} = {json.dumps(m[key])}")
    r = m.get("render")
    if r:
        lines.append(f"  render: {r['lanes']} lanes ({r.get('visible_lanes')} on screen), {r['gl'].get('renderer')}, warm-up {r['warm_ms']} ms")
        for key in ("playback_app", "playback_every", "playback_all_app", "playback_all_every"):
            if key not in r:   # P4-2: a pan_only run has none
                continue
            p = r[key]
            lines.append(f"  {key}: fps median {p['fps_median']} (mean {p['fps_mean']}), JS per frame p50 {p['js_p50_ms']} / p95 {p['js_p95_ms']} / "
                         f"max {p['js_max_ms']} ms, {p['lane_redraws']} lane redraws in {p['frames']} frames, frames over 20 ms {p['frame']['over_20ms']}"
                         + (f", transport: start {p['start_ms']} ms, {p['tracks']} tracks, underruns {p['underruns']}, master {p['master']}"
                            if p.get("transport") else ", self-test clock"))
        pan = r.get("pan")
        if pan:   # P4-2: frames over 16.7 ms, missed frames (25 ms or longer), JS p95, draws per segment; zoom steps
            for key in ("idle", "steady_right", "steady_left", "burst", "scroll"):
                if key in pan:
                    q = pan[key]
                    lines.append(f"  pan {key}: {q['n']} frames, fps {q['fps']}, over 16.7 ms {q['over_16_7']}, missed {q['missed']}, max {q.get('max_ms')} ms, "
                                 f"JS p95 {q['js_p95_ms']} ms, view draws {q['view_draws']}, GL posts {q['gl_posts']}" + (f", tiles {json.dumps(q['tiles'])}" if q.get("tiles") else ""))
            lines.append(f"  pan zoom_steps = {json.dumps(pan.get('zoom_steps'))}")
        if "frames_zoom" not in r:   # a pan_only run
            return "\n".join(lines + [f"  error: {e}" for e in report.get("errors", [])])
        lines.append(f"  render profile (ms per redraw, all 30 lanes) = {json.dumps(r.get('profile_ms_per_redraw'))}")
        lines.append(f"  render frames_zoom = {json.dumps(r['frames_zoom'])}, window requests in the sweep: {r.get('zoom_window_requests')}")
        if "frames_idle" in r:
            lines.append(f"  render frames_idle (no change) = {json.dumps(r['frames_idle'])}")
        lines.append(f"  render agreement_2d = {json.dumps(r['agreement_2d'])}")
        if "layout_reads_in_frames" in r:
            lines.append(f"  render layout reads in the frames = {json.dumps(r['layout_reads_in_frames'])}, frames drawn = {json.dumps(r.get('frames_drawn'))}")
        if "track_heights" in r:   # P3-14c
            lines.append(f"  render track_heights = {json.dumps(r['track_heights'])}")
    lines += play_lines(m.get("play"))
    lines += memory_lines(m.get("memory"))
    lines += [f"  error: {e}" for e in report.get("errors", [])]
    return "\n".join(lines)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("report")
    ap.add_argument("--project", help="the project folder; checks the exported MIDI files against the reference")
    a = ap.parse_args(argv)
    report = json.loads(Path(a.report).read_text())
    print(summary(report))
    ok = bool(report["pass"])
    if a.project and report.get("tracks"):
        bad = midi_mismatches(report, a.project)
        done = len(report["tracks"]) - len(bad)
        print(f"  MIDI byte-identical to the reference: {done} / {len(report['tracks'])} "
              f"(reference: {', '.join(f'{k} {v}' for k, v in sorted(SOURCES.items())) or 'none'})")
        for name, why in bad:
            print(f"    {name}: {why}")
        ok = ok and not bad
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
