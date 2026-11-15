//! #531 (DAW plugin P6a): the sample view, an edit with snap and the role re-pick of a
//! segmented track read its timeline.
//!
//! The gate: a window inside a segment is byte-identical to the file's samples, a window
//! in the gap is silence, a window across an edge is the file up to the edge and silence
//! after it (the test prints the samples it compared and fails on 0), a single full-file
//! segment gives windows byte-identical to the plain track, an added hit with snap near a
//! stroke of the second segment lands on the same stroke as on a plain track at the
//! timeline position (stored time = file time - offset + start, 0 samples difference), a
//! snap in the gap finds no stroke and keeps its time, and a role change re-picks on the
//! timeline positions.

mod common;

use common::*;
use drumspike_app::Service;
use drumspike_core::project::{Segment, add_track, build_source, new_project, save_project};
use serde_json::{Value, json};

const SR: u32 = 48_000;

/// A project with one track: `segs` empty is a plain track with `files[0]` as its source,
/// else a segmented track with one `(start, offset, length)` per segment (each segment
/// plays its own file, `files[i]`). The WAV files go to `<root>/daw/<name>[_i].wav`.
fn track_project(
    root: &std::path::Path,
    project_name: &str,
    name: &str,
    role: &str,
    files: &[Vec<f32>],
    segs: &[(f64, f64, f64)],
) -> std::path::PathBuf {
    let mut p = new_project(root, project_name).unwrap();
    let mut recs = Vec::new();
    for (i, x) in files.iter().enumerate() {
        let file = if i == 0 {
            root.join("daw").join(format!("{name}.wav"))
        } else {
            root.join("daw").join(format!("{name}_{i}.wav"))
        };
        write_wav(&file, SR, x);
        recs.push(build_source(&file, &p.dir, "referenced").unwrap());
    }
    let tr = add_track(&mut p, name, Some(recs[0].clone()), role, 36, Some("average")).unwrap();
    if !segs.is_empty() {
        tr.segments = Some(
            segs.iter()
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
    save_project(&mut p).unwrap();
    p.dir
}

fn open(svc: &Service, dir: &std::path::Path) -> String {
    svc.project_open(&dir.display().to_string()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string()
}

/// The samples of a `samples_window` answer: `(start, rate, samples)`.
fn window(bytes: &[u8]) -> (u32, u32, &[u8]) {
    let start = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
    let rate = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    (start, rate, &bytes[8..])
}

/// The two-segment timeline of the reader tests: `xa` plays at timeline 0 for 1.0 s, the
/// gap is 1.0 s, `xb` plays from timeline 2.0 for its full length. Returns the files and
/// the segment rows.
fn two_segments() -> (Vec<f32>, Vec<f32>, Vec<(f64, f64, f64)>) {
    let xa = kick_signal(&[0.25, 0.5], 1.0, SR);
    let xb = kick_signal(&[0.25, 0.5], 1.0, SR);
    let dur_a = xa.len() as f64 / f64::from(SR);
    let dur_b = xb.len() as f64 / f64::from(SR);
    (xa, xb, vec![(0.0, 0.0, dur_a), (2.0, 0.0, dur_b)])
}

/// Gate 1: a window inside a segment is byte-identical to the same samples of the file; a
/// window inside the gap is silence; a window across a segment edge has the file samples
/// up to the edge and silence after it. Prints the samples compared, fails on 0.
#[test]
fn the_sample_window_of_a_segmented_track_reads_the_timeline() {
    let (root_seg, root_a, root_b) = (tmp("p6_window_seg"), tmp("p6_window_a"), tmp("p6_window_b"));
    let (xa, xb, segs) = two_segments();
    let dir_seg = track_project(&root_seg, "Seg", "s", "kick", &[xa.clone(), xb.clone()], &segs);
    let dir_a = track_project(&root_a, "A", "a", "kick", &[xa], &[]);
    let dir_b = track_project(&root_b, "B", "b", "kick", &[xb], &[]);
    let svc = service_dsp();
    let sid_seg = open(&svc, &dir_seg);
    let sid_a = open(&svc, &dir_a);
    let sid_b = open(&svc, &dir_b);

    let mut compared = 0i64;

    // inside segment A: byte-identical to the file's samples
    let inside = svc.samples_window(&sid_seg, "s", 1000, 2000).unwrap();
    let plain = svc.samples_window(&sid_a, "a", 1000, 2000).unwrap();
    let (start, rate, samples) = window(&inside);
    let (_, _, file_samples) = window(&plain);
    assert_eq!((start, rate), (1000, SR));
    assert_eq!(samples, file_samples, "a window inside segment A is the file's samples");
    compared += samples.len() as i64 / 4;

    // inside the gap (timeline 1.0 s to 2.0 s): silence
    let gap = svc.samples_window(&sid_seg, "s", 48_100, 48_200).unwrap();
    let (start, _, samples) = window(&gap);
    assert_eq!(start, 48_100);
    assert_eq!(samples.len() / 4, 100, "the gap window holds its 100 samples");
    assert!(samples.iter().all(|&b| b == 0), "a window inside the gap is silence");

    // across the edge of segment A (timeline sample 48_000): the file's samples up to the
    // edge, silence after it
    let edge = svc.samples_window(&sid_seg, "s", 47_990, 48_010).unwrap();
    let file = svc.samples_window(&sid_a, "a", 47_990, 48_010).unwrap();
    let (start, _, samples) = window(&edge);
    let (_, _, file_samples) = window(&file);
    assert_eq!(start, 47_990);
    assert_eq!(samples.len(), 20 * 4, "the window holds its 20 samples across the edge");
    assert_eq!(
        &samples[..10 * 4],
        &file_samples[..10 * 4],
        "the file samples up to the edge"
    );
    assert!(samples[10 * 4..].iter().all(|&b| b == 0), "silence after the edge");
    compared += 20;

    // inside segment B: byte-identical to the file's samples (the window holds the file
    // samples at the same place in the segment, the header start is the timeline position)
    let inside_b = svc.samples_window(&sid_seg, "s", 96_100, 96_200).unwrap();
    let file_b = svc.samples_window(&sid_b, "b", 100, 200).unwrap();
    let (start, _, samples) = window(&inside_b);
    let (_, _, file_samples) = window(&file_b);
    assert_eq!((start, samples.len()), (96_100, file_samples.len()));
    assert_eq!(samples, file_samples, "a window inside segment B is the file's samples");
    compared += samples.len() as i64 / 4;

    println!("compared {compared} samples against the files' samples and fails on 0");
    assert!(compared > 0, "the test compared {compared} samples and fails on 0");
}

/// Gate 2: a track made of one segment that covers the full file gives sample windows
/// byte-identical to the windows of the same file as a plain track.
#[test]
fn a_full_file_segment_gives_windows_byte_identical_to_the_plain_track() {
    let (root_plain, root_seg) = (tmp("p6_window_plain"), tmp("p6_window_full"));
    let x = kick_signal(&ONSETS, 3.5, SR);
    let dur = x.len() as f64 / f64::from(SR);
    let dir_plain = track_project(&root_plain, "Plain", "k", "kick", &[x.clone()], &[]);
    let dir_seg = track_project(&root_seg, "Seg", "k", "kick", &[x], &[(0.0, 0.0, dur)]);
    let svc = service_dsp();
    let sid_plain = open(&svc, &dir_plain);
    let sid_seg = open(&svc, &dir_seg);
    let mut compared = 0i64;
    for (s, e) in [(0i64, 1000i64), (1000, 96_000), (12_345, 54_321), (160_000, 200_000)] {
        let a = svc.samples_window(&sid_seg, "k", s, e).unwrap();
        let b = svc.samples_window(&sid_plain, "k", s, e).unwrap();
        assert_eq!(a, b, "window [{s}, {e}) of the full-file segment is the plain track's");
        compared += (a.len() as i64 - 8) / 4;
    }
    println!("compared {compared} samples of full-file-segment windows against the plain track");
    assert!(compared > 0, "the test compared {compared} samples and fails on 0");
}

/// Gate 3: an added hit with snap near a stroke of the second segment lands on the same
/// stroke as on a plain track of that file, at the timeline position: stored time =
/// file time - offset + start, 0 samples difference. A snap in the gap finds no stroke
/// and keeps the given time.
#[test]
fn an_edit_with_snap_lands_on_the_timeline_stroke_and_the_gap_keeps_its_time() {
    let (root_seg, root_plain) = (tmp("p6_snap_seg"), tmp("p6_snap_plain"));
    // the file: strokes at 0.25 and 0.75; the segment plays the file's [0.5, 2.5) part at
    // timeline 2.0, so the file stroke at 0.75 is a timeline stroke at 2.25
    let x = kick_signal(&[0.25, 0.75], 2.5, SR);
    let plain_dir = track_project(&root_plain, "Plain", "k", "kick", &[x.clone()], &[]);
    let seg_dir = track_project(&root_seg, "Seg", "s", "kick", &[x], &[(0.0, 0.0, 0.5), (2.0, 0.5, 2.0)]);
    let svc = service_dsp();
    let sid_plain = open(&svc, &plain_dir);
    let sid_seg = open(&svc, &seg_dir);

    // the plain track: the add at 0.75 - 3 ms snaps to the stroke at 0.75
    let r = svc
        .events_edit(
            &sid_plain,
            "k",
            &[json!({"op": "add", "t": 0.75 - 0.003, "snap": true})],
        )
        .unwrap();
    let s = r["snaps"].as_array().unwrap();
    assert_eq!(s[0]["snapped"], json!(true), "{r}");
    let t_file = s[0]["t"].as_f64().unwrap();
    assert!(
        (t_file - 0.75).abs() < 0.005,
        "the plain add snapped to the stroke, got {t_file}"
    );

    // the segmented track: the same stroke is at timeline 0.75 - 0.5 + 2.0 = 2.25; the add
    // at 2.25 - 3 ms snaps to it — the same file stroke, at the timeline position
    let r = svc
        .events_edit(&sid_seg, "s", &[json!({"op": "add", "t": 2.25 - 0.003, "snap": true})])
        .unwrap();
    let s = r["snaps"].as_array().unwrap();
    assert_eq!(s[0]["snapped"], json!(true), "{r}");
    let t_timeline = s[0]["t"].as_f64().unwrap();
    let diff_samples = ((t_timeline - (t_file - 0.5 + 2.0)) * f64::from(SR)).round() as i64;
    assert_eq!(
        diff_samples, 0,
        "stored time = file time - offset + start, 0 samples difference: {t_timeline} vs {t_file}"
    );
    let ev = svc.events_get(&sid_seg, "s").unwrap()["events"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        ev.iter().any(|e| e["t"].as_f64() == Some(t_timeline)),
        "the snapped time is stored: {ev:?}"
    );

    // a snap in the gap (timeline 0.5 to 2.0) finds no stroke and keeps the given time
    let r = svc
        .events_edit(&sid_seg, "s", &[json!({"op": "add", "t": 1.25, "snap": true})])
        .unwrap();
    let s = r["snaps"].as_array().unwrap();
    assert_eq!(
        s[0],
        json!({"click": 1.25, "t": 1.25, "v": 100, "snapped": false}),
        "a snap in the gap keeps its time: {r}"
    );
}

/// Gate 4: a role change on a segmented track answers `reanalyzed: true`, and the events
/// are on timeline positions — a hit of the second segment sits where the file's stroke
/// is, at file time - offset + start (at least one hit in the second segment).
#[test]
fn a_role_change_repicks_a_segmented_track_on_the_timeline_positions() {
    let (root_seg, root_plain) = (tmp("p6_role_seg"), tmp("p6_role_plain"));
    // the file: strokes at 0.25, 0.75, 1.25; the segment plays the file's [1.0, 2.5) part
    // at timeline 2.0, so the file strokes at 1.25 are timeline strokes at 2.25
    let x = kick_signal(&[0.25, 0.75, 1.25], 2.5, SR);
    let plain_dir = track_project(&root_plain, "Plain", "k", "kick", &[x.clone()], &[]);
    let seg_dir = track_project(&root_seg, "Seg", "s", "kick", &[x], &[(0.0, 0.0, 1.0), (2.0, 1.0, 1.5)]);
    let svc = service_dsp();
    let sid_plain = open(&svc, &plain_dir);
    let sid_seg = open(&svc, &seg_dir);
    svc.analyze_start(&sid_seg, Some(vec!["s".into()]), None, None).unwrap();
    wait_done(&svc, &sid_seg);
    svc.analyze_start(&sid_plain, Some(vec!["k".into()]), None, None)
        .unwrap();
    wait_done(&svc, &sid_plain);

    let role = svc
        .track_set_role(&sid_seg, "s", "snare", None, None, None, None)
        .unwrap();
    assert_eq!(role["reanalyzed"], json!(true), "the role change re-picks: {role}");

    let times_of = |sid: &str, name: &str| -> Vec<f64> {
        svc.events_get(sid, name).unwrap()["events"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter_map(|e| e["t"].as_f64())
            .collect()
    };
    let seg_times = times_of(&sid_seg, "s");
    let plain_times = times_of(&sid_plain, "k");
    assert!(
        seg_times.iter().any(|&t| t >= 2.0),
        "at least one hit in the second segment: {seg_times:?}"
    );
    let mut checked = 0usize;
    for &t in &seg_times {
        // a hit at timeline t comes from the file stroke at t - start + offset (the second
        // segment), or at t itself (the first segment, offset 0, start 0). The attack at the
        // segment boundary itself (the silence-to-signal jump at 2.0) is a real onset of the
        // timeline, not a stroke of the file, so it is not checked against the file's strokes.
        let in_second = t >= 2.0 + 0.1;
        let want = if in_second { t - 2.0 + 1.0 } else { t };
        assert!(
            !in_second || plain_times.iter().any(|p| (p - want).abs() * f64::from(SR) <= 2.0),
            "every hit inside the second segment is on a timeline position of the file's strokes: {t} (file time {want}) against {plain_times:?}"
        );
        if in_second {
            checked += 1;
        }
    }
    assert!(
        checked > 0,
        "the test checked {checked} hits in the second segment and fails on 0"
    );
    println!("checked {checked} re-picked hits in the second segment on the timeline positions");
}
