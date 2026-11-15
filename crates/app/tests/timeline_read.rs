//! #519 (DAW plugin P4): the timeline of a segmented track is assembled one time per
//! analysis run, not on every read — the analysis and the session steps share one assembled
//! signal per distinct segment file (a per-file decode counter in the test build proves one
//! decode per file for the whole job). And `"segments": null` in a document is a plain
//! track. The cache holds no stale audio: a changed segment file or a moved segment
//! assembles again.

mod common;

use common::*;
use drumspike_app::Service;
use drumspike_core::project::{Segment, add_track, build_source, load_project, new_project, save_project};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;

const SR: u32 = 48_000;

/// One track of the session: name, role, signal and segments (`(start, offset, length)`).
type SegTrack<'a> = (&'a str, &'a str, Vec<f32>, Vec<(f64, f64, f64)>);

/// A project folder with the given segmented tracks (like `segments.rs`): the WAV files go
/// to `<root>/daw/<name>.wav`, the segments share the file's record.
fn project_segments(root: &std::path::Path, project_name: &str, tracks: &[SegTrack]) -> PathBuf {
    let mut p = new_project(root, project_name).unwrap();
    for (name, role, x, segs) in tracks {
        let wav = root.join("daw").join(format!("{name}.wav"));
        write_wav(&wav, SR, x);
        let rec = build_source(&wav, &p.dir, "referenced").unwrap();
        let tr = add_track(&mut p, name, Some(rec.clone()), role, 36, Some("average")).unwrap();
        tr.segments = Some(
            segs.iter()
                .map(|&(start, offset, length)| Segment {
                    start,
                    offset,
                    length,
                    source: Some(Some(rec.clone())),
                    extra: Default::default(),
                })
                .collect(),
        );
        tr.source = None;
    }
    save_project(&mut p).unwrap();
    p.dir
}

fn patch_doc(dir: &std::path::Path, project_name: &str, f: impl FnOnce(&mut Value)) {
    let doc = dir.join(format!("{project_name}.drumspike"));
    let text = String::from_utf8(std::fs::read(&doc).unwrap()).unwrap();
    let mut v: Value = serde_json::from_str(&text).unwrap();
    f(&mut v);
    std::fs::write(&doc, serde_json::to_string(&v).unwrap()).unwrap();
}

fn open(svc: &Service, dir: &std::path::Path) -> String {
    svc.project_open(&dir.display().to_string()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string()
}

fn analyze_all(svc: &Service, dir: &std::path::Path) -> String {
    let sid = open(svc, dir);
    svc.analyze_start(&sid, None, None, None).unwrap();
    wait_done(svc, &sid);
    sid
}

fn events(svc: &Service, sid: &str, name: &str) -> Vec<Value> {
    svc.events_get(sid, name).unwrap()["events"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

fn times(ev: &[Value]) -> Vec<f64> {
    ev.iter().map(|e| e["t"].as_f64().unwrap()).collect()
}

fn at_samples(ts: &[f64]) -> Vec<i64> {
    ts.iter().map(|t| (t * f64::from(SR)).round() as i64).collect()
}

/// Gate 1: a session of segmented tracks runs the analysis with the session steps (cross
/// bleed, ghost gate, tom bleed, flam), and every distinct segment file is decoded exactly
/// once for the whole job. Red on main: the timeline was assembled (and every file
/// re-decoded) on every read, and the counter counted 0.
#[test]
fn one_decode_per_distinct_segment_file_for_the_whole_job() {
    let root = tmp("519_session");
    let x = kick_signal(&ONSETS, 3.5, SR);
    let dur = x.len() as f64 / f64::from(SR);
    let dir = project_segments(
        &root,
        "Session",
        &[
            ("k in", "kick", x.clone(), vec![(0.0, 0.0, dur)]),
            ("s in", "snare", x.clone(), vec![(0.0, 0.0, dur)]),
            ("t1", "tom", x.clone(), vec![(0.0, 0.0, dur)]),
        ],
    );
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let sid = analyze_all(&svc, &dir);
    assert_eq!(wait_done(&svc, &sid)["state"], "done");
    let counts: HashMap<String, usize> = svc
        .decode_counts()
        .into_iter()
        .map(|(p, n)| (p.file_name().unwrap().to_string_lossy().into_owned(), n))
        .collect();
    println!("decodes per segment file for the whole job: {counts:?}");
    assert!(!counts.is_empty(), "no decodes were counted at all");
    for name in ["k in", "s in", "t1"] {
        assert_eq!(
            counts.get(&format!("{name}.wav")),
            Some(&1),
            "{name}: the file was not decoded exactly once: {counts:?}"
        );
    }
    std::fs::remove_dir_all(&root).unwrap();
}

/// Gate 2: a track with `"segments": null` and a top-level source analyses as a plain track,
/// byte-identically to the same track without the key; the document loads and saves
/// unchanged. Red on main: the analysis answered `segments are malformed`.
#[test]
fn a_null_segments_key_with_a_source_is_a_plain_track() {
    let (root, root_plain) = (tmp("519_null"), tmp("519_null_plain"));
    let dir = project(&root, "Null", &[kick_spec("k in", "kick", 36)]);
    patch_doc(&dir, "Null", |v| {
        v["tracks"]["k in"]["segments"] = Value::Null;
    });
    // the document loads and saves unchanged (the key stays `null` in `extra`)
    let mut p = load_project(&dir).unwrap();
    assert_eq!(
        p.doc.track("k in").unwrap().extra.get("segments"),
        Some(&Value::Null),
        "the null key was dropped on load"
    );
    let doc = dir.join("Null.drumspike");
    let before = String::from_utf8(std::fs::read(&doc).unwrap()).unwrap();
    save_project(&mut p).unwrap();
    let after = String::from_utf8(std::fs::read(&doc).unwrap()).unwrap();
    let strip = |s: &str| -> Value {
        let mut v: Value = serde_json::from_str(s).unwrap();
        v.as_object_mut().unwrap().remove("modified");
        v
    };
    assert_eq!(
        strip(&before),
        strip(&after),
        "the document changed through load + save"
    );
    // the events are byte-identical to the same track without the key
    let dir_plain = project(&root_plain, "Plain", &[kick_spec("k in", "kick", 36)]);
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let svc_plain = service_dsp();
    svc_plain.set_home(root_plain.join("home"));
    let sid = analyze_all(&svc, &dir);
    let sid_plain = analyze_all(&svc_plain, &dir_plain);
    let a = serde_json::to_string(&events(&svc, &sid, "k in")).unwrap();
    let b = serde_json::to_string(&events(&svc_plain, &sid_plain, "k in")).unwrap();
    assert!(!a.is_empty() && a != "[]", "no hits at all: {a}");
    assert_eq!(a, b, "the null-segments track's events differ from the plain track's");
    std::fs::remove_dir_all(&root).unwrap();
    std::fs::remove_dir_all(&root_plain).unwrap();
}

/// Gate 4: the cache holds no stale audio. After the segment's file is replaced (another
/// length and other onsets), the next analysis decodes the file again (counter 2) and gets
/// the shifted hits. Red on main: the counter counted 0.
#[test]
fn a_changed_segment_file_is_read_again_by_the_next_analysis() {
    let root = tmp("519_stale_file");
    let x = kick_signal(&ONSETS, 3.5, SR);
    let dur = x.len() as f64 / f64::from(SR);
    let dir = project_segments(&root, "Stale", &[("k in", "kick", x, vec![(0.0, 0.0, dur)])]);
    let wav = root.join("daw").join("k in.wav");
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let sid = analyze_all(&svc, &dir);
    let first = times(&events(&svc, &sid, "k in"));
    assert!(!first.is_empty(), "no hits at all");
    // the segment's file changes: other onsets, another length; the segment's record is
    // rebuilt like a relink would, the project is closed and opened again
    let shifted: Vec<f64> = ONSETS.iter().map(|t| t + 0.5).collect();
    write_wav(&wav, SR, &kick_signal(&shifted, 4.0, SR));
    svc.project_close(&sid, true).unwrap();
    let mut p = load_project(&dir).unwrap();
    let rec = build_source(&wav, &p.dir, "referenced").unwrap();
    p.doc.track_mut("k in").unwrap().segments.as_mut().unwrap()[0].source = Some(Some(rec));
    save_project(&mut p).unwrap();
    let sid2 = open(&svc, &dir);
    svc.analyze_start(&sid2, None, None, None).unwrap();
    let st = wait_done(&svc, &sid2);
    assert_eq!(st["tracks"]["k in"]["state"], "done", "{st}");
    let counts: HashMap<String, usize> = svc
        .decode_counts()
        .into_iter()
        .map(|(p, n)| (p.file_name().unwrap().to_string_lossy().into_owned(), n))
        .collect();
    println!("decodes after the file changed: {counts:?}");
    assert_eq!(
        counts.get("k in.wav"),
        Some(&2),
        "the changed file was not decoded again: {counts:?}"
    );
    let second = times(&events(&svc, &sid2, "k in"));
    assert_eq!(
        at_samples(&second),
        at_samples(&first)
            .iter()
            .map(|t| t + (0.5 * f64::from(SR)) as i64)
            .collect::<Vec<_>>(),
        "the second analysis did not read the new timeline"
    );
    std::fs::remove_dir_all(&root).unwrap();
}

/// Gate 4, placement: after a segment's `start` changes, the next analysis reads the new
/// timeline (the hits sit at the new positions). Red on main: the counter counted 0.
#[test]
fn a_moved_segment_is_assembled_again_by_the_next_analysis() {
    let root = tmp("519_stale_move");
    let x = kick_signal(&ONSETS, 3.5, SR);
    let dur = x.len() as f64 / f64::from(SR);
    let dir = project_segments(&root, "Move", &[("k in", "kick", x, vec![(0.0, 0.0, dur)])]);
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let sid = analyze_all(&svc, &dir);
    let first = times(&events(&svc, &sid, "k in"));
    assert!(!first.is_empty(), "no hits at all");
    // the segment moves on the timeline: the document is patched and re-opened
    svc.project_close(&sid, true).unwrap();
    patch_doc(&dir, "Move", |v| {
        v["tracks"]["k in"]["segments"][0]["start"] = serde_json::json!(2.0);
    });
    let sid2 = open(&svc, &dir);
    svc.analyze_start(&sid2, None, None, None).unwrap();
    wait_done(&svc, &sid2);
    let counts: HashMap<String, usize> = svc
        .decode_counts()
        .into_iter()
        .map(|(p, n)| (p.file_name().unwrap().to_string_lossy().into_owned(), n))
        .collect();
    println!("decodes after the segment moved: {counts:?}");
    assert_eq!(
        counts.get("k in.wav"),
        Some(&1),
        "the unchanged file was decoded more than once: {counts:?}"
    );
    let second = times(&events(&svc, &sid2, "k in"));
    assert_eq!(
        at_samples(&second),
        at_samples(&first)
            .iter()
            .map(|t| t + (2.0 * f64::from(SR)) as i64)
            .collect::<Vec<_>>(),
        "the second analysis did not read the new timeline"
    );
    std::fs::remove_dir_all(&root).unwrap();
}
