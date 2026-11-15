//! #528 (DAW plugin P5): the waveform of a segmented track comes from its timeline, and the
//! readers without a timeline answer say so in plain words.
//!
//! The gate: a single full-file segment gives peak data byte-identical to the plain track,
//! two segments with a gap give peaks on the timeline positions (the gap is silence, each
//! segment's bins are that part of the file, the test prints the bins it compared), a
//! changed segment rebuilds the peaks (no stale cache), velocity recompute and reanalyze
//! selection read the timeline, and every other reader of the source answers a 400 that
//! names the reason (never "offline (missing)").
mod common;

use common::*;
use drumspike_app::Service;
use drumspike_core::project::{Segment, add_track, build_source, load_project, new_project, save_project};
use serde_json::{Value, json};

const SR: u32 = 48_000;
const NOT_AVAILABLE: &str = "not available for a track made of segments";

/// A track of the project: name, role, its audio files (file `i` of the segments; one file:
/// a plain track or segments of the same file), the timeline segments
/// (`(start, offset, length)`) and extra keys (layer, sampler).
struct SegSpec {
    name: &'static str,
    role: &'static str,
    files: Vec<Vec<f32>>,
    segs: Vec<(f64, f64, f64)>,
    extra: Value,
}

impl SegSpec {
    /// A plain track with the file as its source.
    fn plain(name: &'static str, role: &'static str, x: Vec<f32>) -> SegSpec {
        SegSpec {
            name,
            role,
            files: vec![x],
            segs: Vec::new(),
            extra: json!({}),
        }
    }

    /// A segmented track; every segment plays its own file (`files[i]`, the last file for
    /// the segments beyond it) from `offset` for `length` at `start`.
    fn segmented(name: &'static str, role: &'static str, files: Vec<Vec<f32>>, segs: Vec<(f64, f64, f64)>) -> SegSpec {
        SegSpec {
            name,
            role,
            files,
            segs,
            extra: json!({}),
        }
    }
}

/// A project folder with the given tracks; a track with segments carries them and no
/// top-level source, the WAV files go to `<root>/daw/<name>[_i].wav`.
fn project_segments(root: &std::path::Path, project_name: &str, tracks: &[SegSpec]) -> std::path::PathBuf {
    let mut p = new_project(root, project_name).unwrap();
    for s in tracks {
        let mut recs = Vec::new();
        for (i, x) in s.files.iter().enumerate() {
            let file = if i == 0 {
                root.join("daw").join(format!("{}.wav", s.name))
            } else {
                root.join("daw").join(format!("{}_{}.wav", s.name, i))
            };
            write_wav(&file, SR, x);
            recs.push(build_source(&file, &p.dir, "referenced").unwrap());
        }
        let tr = add_track(&mut p, s.name, Some(recs[0].clone()), s.role, 36, Some("average")).unwrap();
        if !s.segs.is_empty() {
            tr.segments = Some(
                s.segs
                    .iter()
                    .enumerate()
                    .map(|(i, &(start, offset, length))| Segment {
                        start,
                        offset,
                        length,
                        source: Some(Some(recs[i.min(recs.len() - 1)].clone())),
                        extra: Default::default(),
                    })
                    .collect(),
            );
            tr.source = None;
        }
        if let Some(extra) = s.extra.as_object() {
            for (k, v) in extra {
                tr.extra.insert(k.clone(), v.clone());
            }
        }
    }
    save_project(&mut p).unwrap();
    p.dir
}

/// Sets the file's mtime to `UNIX_EPOCH + secs` (sub-second, for the whole-second key test).
fn set_mtime(path: &std::path::Path, secs: f64) {
    let f = std::fs::OpenOptions::new().write(true).open(path).unwrap();
    f.set_times(
        std::fs::FileTimes::new().set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs_f64(secs)),
    )
    .unwrap();
}

fn open(svc: &Service, dir: &std::path::Path) -> String {
    svc.project_open(&dir.display().to_string()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string()
}

fn peaks_bin(dir: &std::path::Path, name: &str) -> Vec<u8> {
    std::fs::read(dir.join("Analysis").join(name).join("peaks.bin")).unwrap()
}

/// Builds the peak cache of `name` (`open_peaks` builds a missing or stale cache) and
/// returns the peak level with the smallest block size: `(block, bins)`.
fn smallest_level(svc: &Service, sid: &str, name: &str) -> (usize, usize) {
    let info = svc.peaks_info(sid, name).unwrap();
    let levels = info["levels"].as_object().unwrap();
    let (block, n) = levels
        .iter()
        .map(|(k, v)| (k.parse::<usize>().unwrap(), v["n"].as_u64().unwrap() as usize))
        .min_by_key(|(b, _)| *b)
        .unwrap();
    (block, n)
}

/// Bins of `dur` seconds at the block size `block` (full blocks only).
fn bins_of(dur: f64, block: usize) -> i64 {
    ((dur * f64::from(SR)) / block as f64).floor() as i64
}

/// Gate 1: a track made of one segment that covers the full file gives peak data that is
/// byte-identical to the peak data of the same file as a plain track.
#[test]
fn a_single_full_file_segment_gives_peaks_byte_identical_to_the_plain_track() {
    let (root_plain, root_seg) = (tmp("p5_peaks_plain"), tmp("p5_peaks_seg"));
    let x = kick_signal(&ONSETS, 3.5, SR);
    let dur = x.len() as f64 / f64::from(SR);
    let dir_plain = project_segments(&root_plain, "Plain", &[SegSpec::plain("k", "kick", x.clone())]);
    let dir_seg = project_segments(
        &root_seg,
        "Seg",
        &[SegSpec::segmented("k", "kick", vec![x.clone()], vec![(0.0, 0.0, dur)])],
    );
    let svc = service_dsp();
    let sid_plain = open(&svc, &dir_plain);
    let sid_seg = open(&svc, &dir_seg);
    let a = smallest_level(&svc, &sid_plain, "k");
    let b = smallest_level(&svc, &sid_seg, "k");
    assert_eq!(a, b, "the same audio, the same peak levels");
    assert!(a.1 > 0, "the levels hold {} bins", a.1);
    assert_eq!(
        peaks_bin(&dir_plain, "k"),
        peaks_bin(&dir_seg, "k"),
        "the peak data of the full-file segment is the plain track's, byte for byte"
    );
}

/// Gate 2: a track of two segments with a gap gives peaks on the timeline positions: the
/// bins inside the gap are silence and the bins of each segment are the bins of that part
/// of the file. Prints the number of bins compared and fails on 0.
#[test]
fn two_segments_with_a_gap_give_peaks_on_the_timeline_positions() {
    let (root_seg, root_a, root_b) = (tmp("p5_gap_seg"), tmp("p5_gap_a"), tmp("p5_gap_b"));
    // the places are whole blocks of the peak levels (the smallest ones are 480 and 512
    // samples): the segment starts on a bin boundary, so the bins compare exactly
    let (xa, xb) = (kick_signal(&ONSETS, 2.24, SR), kick_signal(&[0.5, 1.0], 1.6, SR));
    let (dur_a, dur_b) = (xa.len() as f64 / f64::from(SR), xb.len() as f64 / f64::from(SR));
    let start_b = 2.56; // the gap: 0.32 s of silence between the segments
    let dir_seg = project_segments(
        &root_seg,
        "Seg",
        &[SegSpec::segmented(
            "s",
            "kick",
            vec![xa.clone(), xb.clone()],
            vec![(0.0, 0.0, dur_a), (start_b, 0.0, dur_b)],
        )],
    );
    let dir_a = project_segments(&root_a, "A", &[SegSpec::plain("a", "kick", xa)]);
    let dir_b = project_segments(&root_b, "B", &[SegSpec::plain("b", "kick", xb)]);
    let svc = service_dsp();
    let sid_seg = open(&svc, &dir_seg);
    let sid_a = open(&svc, &dir_a);
    let sid_b = open(&svc, &dir_b);
    let (block, _) = smallest_level(&svc, &sid_seg, "s");
    let bins_a = bins_of(dur_a, block);
    let bins_b = bins_of(dur_b, block);
    let (b0, b1) = (bins_of(start_b, block), bins_of(start_b, block) + bins_b);

    let tile = |sid: &str, name: &str, from: i64, to: i64| svc.peaks_tile(sid, name, block, from, to).unwrap();

    // the gap is silence: min, max and rms are 0 in every bin
    let gap_tile = tile(&sid_seg, "s", bins_a, b0);
    assert_eq!(
        gap_tile.len(),
        12 + 12 * ((b0 - bins_a) as usize),
        "the gap tile holds the gap's bins"
    );
    assert!(
        b0 - bins_a > 0,
        "the gap spans {} bins and the test fails on 0",
        b0 - bins_a
    );
    for chunk in gap_tile[12..].as_chunks::<4>().0 {
        assert_eq!(chunk, &[0, 0, 0, 0], "a bin of the gap is silence");
    }

    // each segment's bins are the bins of that part of the file (here: from its first sample)
    assert!(
        bins_a > 0 && bins_b > 0,
        "the test compared {bins_a} + {bins_b} bins and fails on 0"
    );
    let a_seg = tile(&sid_seg, "s", 0, bins_a);
    let a_plain = tile(&sid_a, "a", 0, bins_a);
    let first = a_seg[12..].iter().zip(a_plain[12..].iter()).position(|(x, y)| x != y);
    assert!(
        first.is_none(),
        "segment A's timeline bins differ from the file's bins: first differing byte index {:?} (block {block}, bins {bins_a}, seg len {} plain len {})",
        first,
        a_seg.len(),
        a_plain.len()
    );
    let b_seg = tile(&sid_seg, "s", b0, b1);
    let b_plain = tile(&sid_b, "b", 0, bins_b);
    let first_b = b_seg[12..].iter().zip(b_plain[12..].iter()).position(|(x, y)| x != y);
    assert!(
        first_b.is_none(),
        "segment B's timeline bins differ from the file's bins: first differing byte index {first_b:?}"
    );
    println!("compared {bins_a} bins of segment A and {bins_b} bins of segment B, plus the gap's silence bins");
}

/// Gate 3: the peak cache has no stale data — after a segment changes (its place or its
/// file), the next peak build gives the new timeline.
#[test]
fn a_changed_segment_rebuilds_the_peaks() {
    let root = tmp("p5_stale");
    let (xa, xb) = (kick_signal(&ONSETS, 2.0, SR), kick_signal(&[0.5, 1.0], 1.5, SR));
    let dur_a = xa.len() as f64 / f64::from(SR);
    let dir = project_segments(
        &root,
        "Seg",
        &[SegSpec::segmented(
            "s",
            "kick",
            vec![xa],
            vec![(0.0, 0.0, dur_a), (3.0, 0.0, 1.0)],
        )],
    );
    // the file the second segment is re-cut onto below
    write_wav(&root.join("daw").join("b.wav"), SR, &xb);

    let svc = service_dsp();
    let sid = open(&svc, &dir);
    let _ = smallest_level(&svc, &sid, "s");
    let before = peaks_bin(&dir, "s");
    svc.project_close(&sid, true).unwrap();

    // the segment moves: another timeline, another cache
    let mut p = load_project(&dir).unwrap();
    p.doc.track_mut("s").unwrap().segments.as_mut().unwrap()[1].start = 5.0;
    save_project(&mut p).unwrap();
    let sid2 = open(&svc, &dir);
    let _ = smallest_level(&svc, &sid2, "s");
    let after_move = peaks_bin(&dir, "s");
    assert_ne!(before, after_move, "a moved segment rebuilds the peaks");
    svc.project_close(&sid2, true).unwrap();

    // the segment is re-cut onto the other file: another cache again
    let mut p = load_project(&dir).unwrap();
    let seg = &mut p.doc.track_mut("s").unwrap().segments.as_mut().unwrap()[1];
    seg.source = Some(Some(
        build_source(&root.join("daw").join("b.wav"), &p.dir, "referenced").unwrap(),
    ));
    seg.length = xb.len() as f64 / f64::from(SR);
    save_project(&mut p).unwrap();
    let sid3 = open(&svc, &dir);
    let _ = smallest_level(&svc, &sid3, "s");
    let after_file = peaks_bin(&dir, "s");
    assert_ne!(after_move, after_file, "a re-filed segment rebuilds the peaks");
}

/// Gate (also in this ticket, #531): a changed offset alone and a changed length alone
/// are each another peak key — the next peak build gives the new timeline.
#[test]
fn a_changed_offset_or_length_alone_rebuilds_the_peaks() {
    let root = tmp("p6_stale_off_len");
    let (xa, xb) = (kick_signal(&ONSETS, 2.0, SR), kick_signal(&[0.5, 1.0], 1.5, SR));
    let dur_a = xa.len() as f64 / f64::from(SR);
    let dir = project_segments(
        &root,
        "Seg",
        &[SegSpec::segmented(
            "s",
            "kick",
            vec![xa, xb.clone()],
            vec![(0.0, 0.0, dur_a), (3.0, 0.0, 1.0)],
        )],
    );
    let svc = service_dsp();
    let sid = open(&svc, &dir);
    let _ = smallest_level(&svc, &sid, "s");
    let before = peaks_bin(&dir, "s");
    svc.project_close(&sid, true).unwrap();

    // the offset alone changes: the second segment plays another part of its file
    let mut p = load_project(&dir).unwrap();
    p.doc.track_mut("s").unwrap().segments.as_mut().unwrap()[1].offset = 0.25;
    save_project(&mut p).unwrap();
    let sid2 = open(&svc, &dir);
    let _ = smallest_level(&svc, &sid2, "s");
    let after_offset = peaks_bin(&dir, "s");
    assert_ne!(before, after_offset, "a changed offset alone rebuilds the peaks");
    svc.project_close(&sid2, true).unwrap();

    // the length alone changes: the second segment is shorter
    let mut p = load_project(&dir).unwrap();
    p.doc.track_mut("s").unwrap().segments.as_mut().unwrap()[1].length = 0.5;
    save_project(&mut p).unwrap();
    let sid3 = open(&svc, &dir);
    let _ = smallest_level(&svc, &sid3, "s");
    let after_length = peaks_bin(&dir, "s");
    assert_ne!(after_offset, after_length, "a changed length alone rebuilds the peaks");
    let _ = xb;
}

/// Gate (also in this ticket, #531): the peak key of a segmented track reads the file time
/// in full, not whole seconds — a file touched within one second is another key, like the
/// timeline cache's key. (A replaced file is caught by the source record's head hash, so
/// the key is checked on a same-content touch, via the written `key.json`.)
#[test]
fn a_file_touched_within_one_second_is_another_peak_key() {
    let root = tmp("p6_mtime");
    let (xa, xb) = (kick_signal(&ONSETS, 2.0, SR), kick_signal(&[0.5], 1.5, SR));
    let dur_a = xa.len() as f64 / f64::from(SR);
    let dir = project_segments(
        &root,
        "Seg",
        &[SegSpec::segmented(
            "s",
            "kick",
            vec![xa, xb],
            vec![(0.0, 0.0, dur_a), (3.0, 0.0, 1.5)],
        )],
    );
    // the same content and a mtime within one second: only the full time tells them apart
    let bwav = root.join("daw").join("s_1.wav");
    set_mtime(&bwav, 1_000_000.4);
    let svc = service_dsp();
    let sid = open(&svc, &dir);
    let _ = smallest_level(&svc, &sid, "s");
    let key1 = std::fs::read_to_string(dir.join("Analysis").join("s").join("key.json")).unwrap();
    svc.project_close(&sid, true).unwrap();

    set_mtime(&bwav, 1_000_000.7);
    let sid2 = open(&svc, &dir);
    let _ = smallest_level(&svc, &sid2, "s");
    let key2 = std::fs::read_to_string(dir.join("Analysis").join("s").join("key.json")).unwrap();
    assert_ne!(
        key1, key2,
        "a file touched within one second is another peak key (the key reads the full mtime)"
    );
}

/// The segmented fixture of the reader table: a project with a plain track `k` and a
/// segmented track `s` (two segments of one file with a gap) that carries a layer and a
/// sampler calibration, so every reader that would touch their audio reaches its source
/// read.
fn segmented_with_extras() -> (Service, String, std::path::PathBuf) {
    let root = tmp("p5_readers");
    let x = kick_signal(&ONSETS, 2.0, SR);
    let dur = x.len() as f64 / f64::from(SR);
    let dir = project_segments(
        &root,
        "Seg",
        &[
            SegSpec::plain("k", "kick", x.clone()),
            SegSpec {
                name: "s",
                role: "kick",
                files: vec![x],
                segs: vec![(0.0, 0.0, dur / 2.0), (1.5, dur / 2.0, dur / 2.0)],
                extra: json!({
                    "layer": {"source": {"path": "daw/missing-shot.wav", "relative": "daw/missing-shot.wav"},
                              "onset": 0, "sr": SR},
                    "sampler": {"offset_samples": 0.0, "sr": SR}
                }),
            },
        ],
    );
    let svc = service_dsp();
    let sid = open(&svc, &dir);
    (svc, sid, dir)
}

fn assert_not_available(row: &str, e: drumspike_app::AppError) {
    assert_eq!(e.code, 400, "row {row}: not a 400: {e:?}");
    assert!(
        e.message.contains(NOT_AVAILABLE),
        "row {row}: the error does not name the reason: {}",
        e.message
    );
    assert!(
        !e.message.contains("offline"),
        "row {row}: the error still says offline: {}",
        e.message
    );
}

/// Gate 4: every reader without a timeline answer says so in plain words: a 400 that names
/// the reason, never "offline (missing)". Prints the number of readers it checked and
/// fails on 0.
#[test]
fn the_readers_without_a_timeline_answer_not_available() {
    let (svc, sid, _dir) = segmented_with_extras();
    let mut checked = 0usize;

    // the layer view's mic path
    checked += 1;
    assert_not_available("layer", svc.track_layer_get(&sid, "s").unwrap_err());

    // the sampler's alignment in the MIDI export (compensate)
    checked += 1;
    assert_not_available(
        "sampler",
        svc.export_midi_with(&sid, "s", 36, &[], None, 120.0, None, true)
            .unwrap_err(),
    );

    // the WAV export
    checked += 1;
    assert_not_available("export wav", svc.export_spikes(&sid, "s", None).unwrap_err());

    // the export jobs (Export All): the answer reports the reader's 400
    checked += 1;
    let all = svc.export_all(&sid, None).unwrap();
    assert!(
        serde_json::to_string(&all).unwrap().contains(NOT_AVAILABLE),
        "export all does not name the reason: {all}"
    );

    // the transport plan
    checked += 1;
    assert_not_available(
        "transport plan",
        svc.transport_play_window(&sid, None, None, None, None, None)
            .unwrap_err(),
    );

    // the role re-pick reads the timeline now (P6a, `segments_timeline.rs`); the sample
    // window and an edit with snap moved there too. The review helpers — confidence, the
    // histograms, the Learn scores, a Learn mark and a follower — read the timeline now
    // as well (#532, `segments_readers.rs`).

    println!("checked {checked} readers without a timeline answer");
    assert!(checked >= 5, "the test checked {checked} readers and fails on 0");
}

/// Gate 4 (timeline rows): the velocity recompute reads the timeline — a hit of a segment
/// gets the velocity of that part of the file, a hit in the gap the silence's.
#[test]
fn the_velocity_recompute_reads_the_timeline() {
    let (root_seg, root_plain) = (tmp("p5_vel_seg"), tmp("p5_vel_plain"));
    // the hits are far apart, so the velocity window of one never reaches the other
    let x = kick_signal(&[0.25, 2.0], 3.0, SR);
    let half = 1.5; // the first segment is the file's first half, the second the rest
    let dir_seg = project_segments(
        &root_seg,
        "Seg",
        &[SegSpec::segmented(
            "s",
            "kick",
            vec![x.clone()],
            vec![(0.0, 0.0, half), (2.0, half, half)],
        )],
    );
    let dir_plain = project_segments(&root_plain, "Plain", &[SegSpec::plain("p", "kick", x.clone())]);
    let svc = service_dsp();
    let sid_seg = open(&svc, &dir_seg);
    let sid_plain = open(&svc, &dir_plain);
    // the file's hits at 0.25 and 2.0: the second is on the timeline of the second segment
    // at 2.0 + (2.0 - 1.5) = 2.5; the hit at 1.0 is inside the gap
    let events_seg = [json!({"t": 0.25}), json!({"t": 2.5}), json!({"t": 1.0})];
    let events_plain = [json!({"t": 0.25}), json!({"t": 2.0})];
    let out_seg = svc.velocities_recompute(&sid_seg, "s", Some(&events_seg)).unwrap();
    let out_plain = svc.velocities_recompute(&sid_plain, "p", Some(&events_plain)).unwrap();
    let v = |out: &Value, i: usize| out["events"][i]["v"].as_u64().unwrap();
    let (v_first, v_second, v_gap) = (v(&out_seg, 0), v(&out_seg, 1), v(&out_seg, 2));
    assert_eq!(v_first, v(&out_plain, 0), "the first segment's hit is the file's hit");
    assert_eq!(v_second, v(&out_plain, 1), "the second segment's hit is the file's hit");
    assert!(
        v_gap < v_first,
        "a hit in the gap gets the silence's velocity ({v_gap} < {v_first})"
    );
}

/// Gate 4 (timeline rows): the reanalyze selection reads the timeline — a selection inside
/// a segment re-picks the hits of that part of the file at the timeline positions.
#[test]
fn the_reanalyze_selection_reads_the_timeline() {
    let root = tmp("p5_reanalyze");
    let x = kick_signal(&[0.25, 0.75], 1.5, SR);
    let dur = x.len() as f64 / f64::from(SR);
    let half = dur / 2.0;
    let dir = project_segments(
        &root,
        "Seg",
        &[SegSpec::segmented(
            "s",
            "kick",
            vec![x.clone()],
            vec![(0.0, 0.0, half), (1.0, half, half)],
        )],
    );
    let svc = service_dsp();
    let sid = open(&svc, &dir);
    let out = svc.reanalyze_selection(&sid, "s", 0.0, half, false).unwrap();
    let times: Vec<f64> = out["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["t"].as_f64())
        .collect();
    assert!(
        times.iter().any(|&t| (t - 0.25).abs() < 0.05),
        "the re-pick found the hit of the first segment on the timeline: {times:?}"
    );
    assert!(
        times.iter().all(|&t| t < half + 0.2),
        "the re-pick stayed in the selection's timeline range: {times:?}"
    );
}
