//! #532 (DAW plugin P6b): the review helpers of the page read a segmented track's
//! timeline: confidence (and its Training variant), the histograms, the Learn scores, a
//! Learn mark and a follower track.
//!
//! The gate: a single full-file segment gives every reader the same answer as the same
//! file as a plain track (the JSON answers compare equal, the test prints the number of
//! readers it compared and fails on 0), a hit of the second segment of a two-segment
//! timeline sits at file time - offset + start (0 samples difference; each check names
//! its reader and the test fails on 0 comparisons), Forget This and Learn This act on
//! the same hits as on a plain track of that file part, and a follower of a segmented
//! track derives its hits from the timeline — a segmented follower too. The rows these
//! readers had in the not-available test of #528 (`segments_peaks.rs`) moved here.

mod common;

use common::*;
use drumspike_app::Service;
use drumspike_core::project::{Segment, add_track, build_source, new_project, save_project};
use serde_json::{Value, json};

const SR: u32 = 48_000;

/// One track of the project: name, role, its audio files (file `i` of the segments; one
/// file: a plain track or segments of the same file) and the timeline segments
/// (`(start, offset, length)` per segment).
type TrackSpec<'a> = (&'a str, &'a str, &'a [Vec<f32>], &'a [(f64, f64, f64)]);

/// A project with the named tracks: `segs` empty is a plain track with `files[0]` as its
/// source, else a segmented track with one `(start, offset, length)` per segment (each
/// segment plays its own file, `files[i]`). The WAV files go to `<root>/daw/<name>[_i].wav`.
fn track_project(root: &std::path::Path, project_name: &str, tracks: &[TrackSpec]) -> std::path::PathBuf {
    let mut p = new_project(root, project_name).unwrap();
    for (name, role, files, segs) in tracks {
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

/// The stored hits of a track as `(t, v)` rows in stored order.
fn events(svc: &Service, sid: &str, name: &str) -> Vec<(f64, u64)> {
    svc.events_get(sid, name).unwrap()["events"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|e| (e["t"].as_f64().unwrap_or(0.0), e["v"].as_u64().unwrap_or(0)))
        .collect()
}

/// The times of a follower's derived hits, in seconds.
fn follow_times(svc: &Service, sid: &str, name: &str) -> Vec<f64> {
    events(svc, sid, name).into_iter().map(|(t, _)| t).collect()
}

/// The JSON of an answer with the volatile keys dropped (none is expected; the scrub keeps
/// the comparison robust if one is added later).
fn stable(v: &Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(
            m.iter()
                .filter(|(k, _)| !(k.contains("ms") || k.as_str() == "ts" || k.contains("elapsed")))
                .map(|(k, v)| (k.clone(), stable(v)))
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(stable).collect()),
        other => other.clone(),
    }
}

/// The file of the reader tests: strokes at 0.25, 0.75 and 1.25 s, silent around the cuts of
/// the two-segment timeline (file 0.5, 1.0 and 1.5): a stroke tail that would reach across a
/// cut goes to zero before it, so the cut is a hard onset of neither track. The two-segment
/// timeline plays its first half at timeline 0 and its `[0.5, 1.5)` part at timeline 2.0, so
/// the file strokes 0.75 and 1.25 are timeline strokes at 2.25 and 2.75.
fn reader_file() -> Vec<f32> {
    let mut x = kick_signal(&[0.25, 0.75, 1.25], 2.0, SR);
    let srf = f64::from(SR);
    // silent from just before each cut of the two-segment timeline (file 0.5, 1.0, 1.5) up to
    // the next stroke, so a cut is a hard onset of neither track and no tail resumes after it
    for (a, b) in [(0.45f64, 0.70), (0.95, 1.20), (1.45, 2.0)] {
        x[(a * srf) as usize..(b * srf) as usize].fill(0.0);
    }
    x
}

/// The two-segment timeline of the reader tests: the first segment is the file's first
/// half (offset 0, start 0), the second its `[0.5, 1.5)` part at start 2.0 — offset not 0
/// and start not equal to offset, with a gap from 1.0 to 2.0.
fn two_segments() -> Vec<(f64, f64, f64)> {
    vec![(0.0, 0.0, 1.0), (2.0, 0.5, 1.0)]
}

/// One file as the single-element file list of a track.
fn one(x: &Vec<f32>) -> &[Vec<f32>] {
    std::slice::from_ref(x)
}

/// Gate 1: a track made of one segment that covers the full file gives every reader the
/// same answer as the same file as a plain track. Prints the number of readers compared
/// and fails on 0.
#[test]
fn a_single_full_file_segment_gives_every_reader_the_plain_track_s_answer() {
    let x = reader_file();
    let dur = x.len() as f64 / f64::from(SR);
    let dir_plain = track_project(
        &tmp("p6b_id_plain"),
        "Plain",
        &[("s", "kick", one(&x), &[]), ("f", "kick", one(&x), &[])],
    );
    let dir_seg = track_project(
        &tmp("p6b_id_seg"),
        "Seg",
        &[("s", "kick", one(&x), &[(0.0, 0.0, dur)]), ("f", "kick", one(&x), &[])],
    );
    let svc = service_dsp();
    let sid_plain = open(&svc, &dir_plain);
    let sid_seg = open(&svc, &dir_seg);
    for (sid, track) in [(&sid_plain, "s"), (&sid_seg, "s")] {
        svc.analyze_start(sid, Some(vec![track.into()]), None, None).unwrap();
        wait_done(&svc, sid);
    }

    let mut compared = 0usize;
    let same = |a: Value, b: Value, reader: &str| {
        assert_eq!(stable(&a), stable(&b), "{reader}: the answers differ: {a} vs {b}");
    };

    // confidence, and its Training variant
    for training in [false, true] {
        let a = svc.track_confidence(&sid_plain, "s", None, training).unwrap();
        let b = svc.track_confidence(&sid_seg, "s", None, training).unwrap();
        let n = b["confidence"].as_array().map_or(0, Vec::len);
        assert!(n > 0, "confidence compared {n} hits and fails on 0");
        same(
            a,
            b,
            if training {
                "confidence (training)"
            } else {
                "confidence"
            },
        );
        compared += 1;
    }

    // histograms
    let a = svc.track_histograms(&sid_plain, "s", None).unwrap();
    let b = svc.track_histograms(&sid_seg, "s", None).unwrap();
    same(a, b, "histograms");
    compared += 1;

    // the Learn scores
    let a = svc.track_learn_scores(&sid_plain, "s").unwrap();
    let b = svc.track_learn_scores(&sid_seg, "s").unwrap();
    assert_eq!(a["state"], json!("ok"), "{a}");
    same(a, b, "learn scores");
    compared += 1;

    // a Learn mark (the examples and their placed times)
    let a = svc
        .track_learn_mark(&sid_plain, "s", "this", Some((0.2, 0.3)), None)
        .unwrap();
    let b = svc
        .track_learn_mark(&sid_seg, "s", "this", Some((0.2, 0.3)), None)
        .unwrap();
    assert_eq!(a["marked"], json!(true), "{a}");
    same(a, b, "learn mark");
    compared += 1;

    // a follower track (its derived hits)
    svc.track_set_follow(&sid_plain, "f", Some("s")).unwrap();
    svc.track_set_follow(&sid_seg, "f", Some("s")).unwrap();
    let a = json!(events(&svc, &sid_plain, "f"));
    let b = json!(events(&svc, &sid_seg, "f"));
    let n = b.as_array().map_or(0, Vec::len);
    assert!(n > 0, "follow compared {n} hits and fails on 0");
    same(a, b, "follow");
    compared += 1;

    println!("compared {compared} readers on the full-file segment and the plain track");
    assert!(compared >= 6, "the test compared {compared} readers and fails on 0");
}

/// Gate 2: a hit of the second segment sits at file time - offset + start, 0 samples
/// difference, in every reader that returns or stores times. Each check names its reader;
/// the test prints the number of times it compared and fails on 0.
#[test]
fn the_readers_give_the_timeline_positions() {
    let x = reader_file();
    let segs = two_segments();
    let dir_plain = track_project(&tmp("p6b_pos_plain"), "Plain", &[("s", "kick", one(&x), &[])]);
    let dir_seg = track_project(
        &tmp("p6b_pos_seg"),
        "Seg",
        &[("s", "kick", one(&x), &segs), ("f", "kick", one(&x), &[])],
    );
    let svc = service_dsp();
    let sid_plain = open(&svc, &dir_plain);
    let sid_seg = open(&svc, &dir_seg);
    for sid in [&sid_plain, &sid_seg] {
        svc.analyze_start(sid, Some(vec!["s".into()]), None, None).unwrap();
        wait_done(&svc, sid);
    }
    // the hits of the second segment: the timeline times and the file times they come from
    let plain_times: Vec<f64> = events(&svc, &sid_plain, "s").into_iter().map(|(t, _)| t).collect();
    let seg_times: Vec<f64> = events(&svc, &sid_seg, "s").into_iter().map(|(t, _)| t).collect();
    let second: Vec<(f64, f64)> = seg_times
        .iter()
        .filter(|&&t| t >= 2.0 + 0.1)
        .map(|&t| (t, t - 2.0 + 0.5))
        .filter(|(_, f)| plain_times.iter().any(|&p| (p - f).abs() < 0.05))
        .collect();
    let mut compared = 0usize;

    // the histograms: the level of the timeline hit is the file part's level
    let ev_of = |times: &[f64]| times.iter().map(|&t| json!({"t": t})).collect::<Vec<Value>>();
    let level = |out: &Value, i: usize| out["loudness"]["levels"][i].as_f64().unwrap();
    let seg_lv = svc
        .track_histograms(&sid_seg, "s", Some(&ev_of(&[second[0].0])))
        .unwrap();
    let plain_lv = svc
        .track_histograms(&sid_plain, "s", Some(&ev_of(&[second[0].1])))
        .unwrap();
    assert!(
        (level(&seg_lv, 0) - level(&plain_lv, 0)).abs() < 1e-9,
        "histograms: the second segment's hit has the file part's level ({} vs {})",
        level(&seg_lv, 0),
        level(&plain_lv, 0)
    );
    compared += 1;

    // confidence: the same hit rates the same on both tracks
    let rate = |out: &Value, i: usize| {
        (
            out["confidence"][i]["c"].as_f64().unwrap(),
            out["confidence"][i]["doubt"].as_bool(),
        )
    };
    let seg_c = svc
        .track_confidence(&sid_seg, "s", Some(&ev_of(&[second[0].0])), false)
        .unwrap();
    let plain_c = svc
        .track_confidence(&sid_plain, "s", Some(&ev_of(&[second[0].1])), false)
        .unwrap();
    assert_eq!(
        rate(&seg_c, 0),
        rate(&plain_c, 0),
        "confidence: the hit's rating differs"
    );
    compared += 1;

    // the Learn scores: the candidates of the second segment sit at the timeline positions
    let cands = |sid: &str, lo: f64, hi: f64| -> Vec<f64> {
        let out = svc.track_learn_scores(sid, "s").unwrap();
        assert_eq!(out["state"], json!("ok"), "{out}");
        out["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|r| r[0].as_f64())
            .filter(|&t| t >= lo && t <= hi)
            .collect()
    };
    let seg_ts = cands(&sid_seg, 2.0 + 0.1, 3.0 - 0.1);
    let plain_ts = cands(&sid_plain, 0.5 + 0.1, 1.5 - 0.1);
    assert!(
        !seg_ts.is_empty(),
        "learn scores: no candidate of the second segment to compare (seg {seg_ts:?} plain {plain_ts:?})"
    );
    assert_eq!(
        seg_ts.len(),
        plain_ts.len(),
        "learn scores: a different candidate count for the file part: seg {seg_ts:?} plain {plain_ts:?}"
    );
    for (s, p) in seg_ts.iter().zip(&plain_ts) {
        let diff = ((s - (p - 0.5 + 2.0)) * f64::from(SR)).round() as i64;
        assert_eq!(
            diff, 0,
            "learn scores: the candidate is {diff} samples off the timeline position"
        );
        compared += 1;
    }

    // a Learn mark stores the hit's timeline position
    let mark = |sid: &str, range: (f64, f64)| {
        svc.track_learn_mark(sid, "s", "this", Some(range), None).unwrap()["mark"]["at"]
            .as_array()
            .cloned()
            .unwrap()
            .into_iter()
            .map(|v| v.as_f64().unwrap())
            .collect::<Vec<f64>>()
    };
    let seg_at = mark(&sid_seg, (2.7, 2.9));
    let plain_at = mark(&sid_plain, (1.2, 1.3));
    let n_at = seg_at.len().min(plain_at.len());
    assert!(n_at > 0, "learn mark: no examples to compare");
    for (s, p) in seg_at.iter().zip(&plain_at).take(n_at) {
        let diff = ((s - (p - 0.5 + 2.0)) * f64::from(SR)).round() as i64;
        assert_eq!(
            diff, 0,
            "learn mark: the example sits {diff} samples off the timeline position ({s} vs file {p})"
        );
        compared += 1;
    }

    // a follower derives its hits at the source's timeline positions
    svc.track_set_follow(&sid_seg, "f", Some("s")).unwrap();
    let derived = follow_times(&svc, &sid_seg, "f");
    let want: Vec<f64> = seg_times
        .iter()
        .map(|t| (t * f64::from(SR)).round() / f64::from(SR))
        .collect();
    let n_follow = derived.len().min(want.len());
    assert!(n_follow > 0, "follow: no derived hits to compare");
    for (d, w) in derived.iter().zip(&want).take(n_follow) {
        let diff = ((d - w) * f64::from(SR)).round() as i64;
        assert_eq!(
            diff, 0,
            "follow: the derived hit is {diff} samples off the timeline hit"
        );
        compared += 1;
    }

    println!(
        "compared {compared} times on the timeline positions (histograms, confidence, learn scores, learn mark, follow)"
    );
    assert!(compared > 0, "the test compared {compared} times and fails on 0");
}

/// Gate 3: Learn This and Forget This on a hit of the second segment act on the same hits
/// as on a plain track of that file part: the same count of examples Learn This adds, at
/// the same times, the same count of hits the Learn filter takes out, and the same count
/// of examples Forget This removes (with the hits its sweep takes, none here: the range
/// resolves to hits). The hit lists compare away from the segment edges (the fixture keeps
/// them silent anyway); the test prints how many hits it compared and fails on 0.
#[test]
fn learn_this_and_forget_this_act_on_the_same_hits_as_the_plain_track() {
    let x = reader_file();
    let segs = two_segments();
    let dir_plain = track_project(&tmp("p6b_learn_plain"), "Plain", &[("s", "kick", one(&x), &[])]);
    let dir_seg = track_project(&tmp("p6b_learn_seg"), "Seg", &[("s", "kick", one(&x), &segs)]);
    let svc = service_dsp();
    let sid_plain = open(&svc, &dir_plain);
    let sid_seg = open(&svc, &dir_seg);
    for sid in [&sid_plain, &sid_seg] {
        svc.analyze_start(sid, Some(vec!["s".into()]), None, None).unwrap();
        wait_done(&svc, sid);
    }

    // The stored hits of the second segment, mapped to file times (timeline - 2.0 + 0.5),
    // against the plain track's hits in the same file part. Hits within 0.1 s of the segment
    // edges (timeline 2.0 and 3.0; the file's 0.5 and 1.5) are dropped.
    let hits = |sid: &str| -> Vec<f64> { events(&svc, sid, "s").into_iter().map(|(t, _)| t).collect() };
    let compare = |what: &str| {
        let mut seg: Vec<f64> = hits(&sid_seg)
            .into_iter()
            .filter(|&t| (2.1..=2.9).contains(&t))
            .map(|t| t - 1.5)
            .collect();
        let mut plain: Vec<f64> = hits(&sid_plain).into_iter().filter(|&t| (0.6..=1.4).contains(&t)).collect();
        seg.sort_by(f64::total_cmp);
        plain.sort_by(f64::total_cmp);
        assert!(
            !seg.is_empty(),
            "{what}: no hit away from the segment edges to compare (seg {seg:?} plain {plain:?})"
        );
        assert_eq!(
            seg.len(),
            plain.len(),
            "{what}: a different hit count: seg {seg:?} plain {plain:?}"
        );
        for (s, p) in seg.iter().zip(&plain) {
            let diff = ((s - p) * f64::from(SR)).round() as i64;
            assert_eq!(
                diff, 0,
                "{what}: the hit is {diff} samples off the plain track's ({s} vs {p})"
            );
        }
        println!("{what}: compared {} hits away from the segment edges", seg.len());
    };

    // Learn This on the second segment's hit (timeline 2.75 = file 1.25 - 0.5 + 2.0): one
    // example on each track at the same hit, the same count of hits the filter takes out
    let seg = svc
        .track_learn_mark(&sid_seg, "s", "this", Some((2.7, 2.9)), None)
        .unwrap();
    let plain = svc
        .track_learn_mark(&sid_plain, "s", "this", Some((1.2, 1.3)), None)
        .unwrap();
    assert_eq!(seg["added"], json!(1), "{seg}");
    assert_eq!(plain["added"], json!(1), "{plain}");
    let seg_at = seg["mark"]["at"].as_array().cloned().unwrap();
    let plain_at = plain["mark"]["at"].as_array().cloned().unwrap();
    assert_eq!(seg_at.len(), plain_at.len(), "{seg} vs {plain}");
    for (s, p) in seg_at.iter().zip(&plain_at) {
        let (s, p) = (s.as_f64().unwrap(), p.as_f64().unwrap());
        let diff = ((s - (p - 0.5 + 2.0)) * f64::from(SR)).round() as i64;
        assert_eq!(
            diff, 0,
            "Learn This adds the example {diff} samples off the plain track's"
        );
    }
    assert_eq!(
        seg["learn_out_after"], plain["learn_out_after"],
        "a different count of hits the Learn filter takes out: {seg} vs {plain}"
    );
    compare("after Learn This");

    // Forget This over the same hit removes the example and the hit it supported on both tracks
    let seg = svc
        .track_learn_forget_at(&sid_seg, "s", Some((2.7, 2.9)), None)
        .unwrap();
    let plain = svc
        .track_learn_forget_at(&sid_plain, "s", Some((1.2, 1.3)), None)
        .unwrap();
    assert_eq!(seg["removed"], plain["removed"], "{seg} vs {plain}");
    assert_eq!(seg["removed"], json!(1), "forget removed {} examples", seg["removed"]);
    assert_eq!(seg["swept"], plain["swept"], "{seg} vs {plain}");
    compare("after Forget This");

    // A Learn This with no hit in the range (a K-add) places its hit at the same position of
    // the file part's audio: the seg track's timeline and the plain track's file agree to the
    // sample. (Main refuses the read: the hit comes from the track's audio.)
    let seg = svc
        .track_learn_mark(&sid_seg, "s", "this", Some((2.05, 2.2)), None)
        .unwrap();
    let plain = svc
        .track_learn_mark(&sid_plain, "s", "this", Some((0.55, 0.7)), None)
        .unwrap();
    assert!(!seg["added_hit"].is_null(), "{seg}");
    let seg_hit = seg["added_hit"].as_f64().unwrap();
    let plain_hit = plain["added_hit"]
        .as_f64()
        .unwrap_or_else(|| panic!("no added hit: {plain}"));
    let diff = ((seg_hit - (plain_hit - 0.5 + 2.0)) * f64::from(SR)).round() as i64;
    assert_eq!(diff, 0, "the K-add places its hit {diff} samples off the plain track's");
    compare("after the K-add");
}

/// Gate 4: a follower of a segmented track derives its hits from the timeline — a
/// segmented follower too.
#[test]
fn a_follower_of_a_segmented_track_derives_its_hits_from_the_timeline() {
    let x = reader_file();
    let segs = two_segments();
    let dir = track_project(
        &tmp("p6b_follow"),
        "Seg",
        &[
            ("s", "kick", one(&x), &segs),
            ("fp", "kick", one(&x), &[]),
            ("fs", "kick", one(&x), &segs),
        ],
    );
    let svc = service_dsp();
    let sid = open(&svc, &dir);
    svc.analyze_start(&sid, Some(vec!["s".into()]), None, None).unwrap();
    wait_done(&svc, &sid);

    let out = svc.track_set_follow(&sid, "fp", Some("s")).unwrap();
    assert!(out["n_events"].as_u64().unwrap_or(0) > 0, "{out}");
    let out = svc.track_set_follow(&sid, "fs", Some("s")).unwrap();
    assert!(out["n_events"].as_u64().unwrap_or(0) > 0, "{out}");

    // both followers hold the source's hits at the timeline positions, sample-exact
    let source: Vec<f64> = events(&svc, &sid, "s").into_iter().map(|(t, _)| t).collect();
    let srf = f64::from(SR);
    for name in ["fp", "fs"] {
        let derived = follow_times(&svc, &sid, name);
        assert_eq!(
            derived.len(),
            source.len(),
            "{name}: a different hit count than the source"
        );
        for (d, s) in derived.iter().zip(&source) {
            let diff = ((d - s) * srf).round() as i64;
            assert_eq!(
                diff, 0,
                "{name}: the derived hit is {diff} samples off the source's timeline hit"
            );
        }
    }
    // the segmented follower's velocities come from its own timeline (the same audio), so
    // they match the source's; the plain follower reads another layout and may not
    let v_of = |name: &str| {
        events(&svc, &sid, name)
            .into_iter()
            .map(|(_, v)| v)
            .collect::<Vec<u64>>()
    };
    assert_eq!(
        v_of("fs"),
        v_of("s"),
        "the segmented follower's velocities differ from the source's"
    );
}

/// Gate 5: a training record of a second-segment hit carries the audio of the timeline
/// position: the window of the record holds the stroke at timeline 2.75 (file 1.25 - 0.5
/// + 2.0), loud and with its peak at the stroke — not the zeros past the file's end, which
///   the file position would read.
#[test]
fn a_training_record_of_a_second_segment_hit_carries_the_timeline_audio() {
    let x = reader_file();
    let segs = two_segments();
    let root = tmp("p6b_training_seg");
    let dir = track_project(&root, "Seg", &[("s", "kick", one(&x), &segs)]);
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let sid = open(&svc, &dir);
    svc.training_status(&sid, Some(true)).unwrap();
    svc.analyze_start(&sid, Some(vec!["s".into()]), None, None).unwrap();
    wait_done(&svc, &sid);

    let r = svc
        .track_learn_mark_training(&sid, "s", "this", Some((2.7, 2.9)), None, true)
        .unwrap();
    assert_eq!(r["marked"], json!(true), "{r}");

    // 40 ms before and 10 ms after at 48 kHz (the record's window of training.rs)
    let (n_before, n_after) = (1920usize, 480usize);
    let dir = root.join("home").join("training");
    let mut lines = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("no training log at {}: {e}", dir.display()))
        .flatten()
        .flat_map(|f| {
            std::fs::read_to_string(f.path())
                .unwrap()
                .lines()
                .map(str::to_string)
                .collect::<Vec<_>>()
        });
    let line = lines.next().unwrap_or_else(|| panic!("no training record"));
    let rec: Value = serde_json::from_str(&line).unwrap();
    let win = rec["audio"]["at_from"].as_array().cloned().unwrap_or_default();
    assert_eq!(win.len(), n_before + n_after, "the record's audio window: {rec}");
    let peak = win
        .iter()
        .map(|v| v.as_f64().unwrap_or(0.0).abs())
        .fold(0.0f64, f64::max);
    assert!(peak > 0.05, "the record's audio window is silent ({peak}): {rec}");
    let at = win
        .iter()
        .enumerate()
        .max_by(|a, b| {
            a.1.as_f64()
                .unwrap_or(0.0)
                .abs()
                .total_cmp(&b.1.as_f64().unwrap_or(0.0).abs())
        })
        .map_or(usize::MAX, |(i, _)| i);
    assert!(
        (n_before - 240..=n_before + 240).contains(&at),
        "the window's peak sits {at} samples from the stroke (expected near {n_before}): {rec}"
    );
}

/// Gate 6: a Learn mark with a second analysed close mic that is segmented reads that mic's
/// timeline: the loud snare hit at timeline 1.75 (file 1.25 - 0.5 + 1.0) is the bleed of the
/// kick's weak rise (a 2 to 6 dB rise over the hum, no candidate, no 6 dB rise) — the log
/// names the bleed at the snare's timeline hit and K never places its hit on the bled rise;
/// the snare's plain file at the timeline time would not rule the rise out. The timeline
/// here has no gap, so the hum never steps at a join.
#[test]
fn a_learn_mark_reads_the_timeline_of_a_second_segmented_close_mic() {
    // no gap: the second segment follows the first at once (timeline 1.0 = file 0.5), both
    // cuts land on a zero crossing of the 300 Hz hum
    let segs = [(0.0, 0.0, 1.0), (1.0, 0.5, 1.0)];
    // the kick's ghost: a soft stroke at file 1.25 (timeline 1.75), 4 dB over a steady hum
    let srf = f64::from(SR);
    let ghost = kick_signal(&[1.25], 2.0, SR);
    let g = 10f32.powf(-58.0 / 20.0);
    let hum = 10f32.powf(-62.0 / 20.0);
    let kick: Vec<f32> = ghost
        .iter()
        .enumerate()
        .map(|(i, b)| g * b + hum * (2.0 * std::f32::consts::PI * 300.0 * i as f32 / srf as f32).sin())
        .collect();
    // the snare: one loud hit at the same place
    let snare = kick_signal(&[1.25], 2.0, SR);
    let dir = track_project(
        &tmp("p6b_bleed_seg"),
        "Seg",
        &[("s", "kick", one(&kick), &segs), ("o", "snare", one(&snare), &segs)],
    );
    let svc = service_dsp();
    let sid = open(&svc, &dir);
    svc.analyze_start(&sid, None, None, None).unwrap();
    wait_done(&svc, &sid);
    // premise: the snare holds the hit at the timeline position
    let o: Vec<f64> = events(&svc, &sid, "o").into_iter().map(|(t, _)| t).collect();
    assert!(
        o.iter().any(|t| (t - 1.75).abs() < 0.02),
        "premise: no snare hit at 1.75: {o:?}"
    );

    let r = svc.track_learn_mark(&sid, "s", "this", Some((1.7, 1.9)), None).unwrap();
    assert_eq!(
        r["from"],
        json!("hand"),
        "premise: a weak rise, not a hit or candidate: {r}"
    );
    // the snare's timeline hit rules its rise out: the log names the bleed at 1.75 (with the
    // snare's level over the rise's own), and K never places the hit on the bled rise
    let log = r["k_rise"]["pieces"].as_array().cloned().unwrap_or_default();
    let bled = log
        .iter()
        .find(|p| p["why"] == json!("bleed"))
        .unwrap_or_else(|| panic!("the rise log names no bleed: {log:?}"));
    let bleed_t = bled["bleed_t"].as_f64().unwrap();
    assert!(
        (bleed_t - 1.75).abs() <= drumspike_app::learn::BLEED_REACH,
        "the bleed is not at the snare's timeline hit: {bled}"
    );
    assert!(
        bled["bleed_level"].as_f64().unwrap() > bled["level"].as_f64().unwrap(),
        "the bleed is not the louder hit: {bled}"
    );
    for p in &log {
        if matches!(p["why"].as_str(), Some("taken")) {
            let t = p["t"].as_f64().unwrap_or(0.0);
            assert!(
                (t - 1.75).abs() > drumspike_app::learn::BLEED_REACH,
                "K took the bled rise at {t}: {log:?}"
            );
        }
    }
}
