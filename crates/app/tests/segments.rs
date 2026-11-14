//! #508 (DAW plugin P1): a track as segments on a timeline. The gate: a single full-file
//! segment analyses byte-identically to the plain track, two segments with a gap give the
//! hits of each segment at `timeline start + offset in the segment` exact to the sample (a
//! hit in the gap or a trimmed part is absent), the session rules run on timeline positions
//! (the flam rule on a shifted session marks the same hits), and an old project file (no
//! `segments`) loads and analyses byte-identically.
mod common;

use common::*;
use drumspike_app::Service;
use drumspike_core::project::{Segment, add_track, build_source, load_project, new_project, save_project};
use serde_json::Value;

const SR: u32 = 48_000;

/// A track of [`project_segments`]: name, role, its signal and its timeline segments
/// (`(start, offset, length)`; empty: a plain track with the file as its source).
struct SegSpec {
    name: &'static str,
    role: &'static str,
    x: Vec<f32>,
    segs: Vec<(f64, f64, f64)>,
}

/// A project folder with the given tracks. A track with segments carries them (with the
/// shared record) and no top-level source, like a DAW clip list; the WAV files go to
/// `<root>/daw/<name>.wav`.
fn project_segments(root: &std::path::Path, project_name: &str, tracks: &[SegSpec]) -> std::path::PathBuf {
    let mut p = new_project(root, project_name).unwrap();
    for s in tracks {
        let wav = root.join("daw").join(format!("{}.wav", s.name));
        write_wav(&wav, SR, &s.x);
        let rec = build_source(&wav, &p.dir, "referenced").unwrap();
        let tr = add_track(&mut p, s.name, Some(rec.clone()), s.role, 36, Some("average")).unwrap();
        if !s.segs.is_empty() {
            tr.segments = Some(
                s.segs
                    .iter()
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
    }
    save_project(&mut p).unwrap();
    p.dir
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

/// Gate 1: a track made of one segment that covers the full file gives an event list that is
/// byte-identical to the same file as a normal track.
#[test]
fn a_single_full_file_segment_is_byte_identical_to_the_plain_track() {
    let (root, root_seg) = (tmp("seg_full_plain"), tmp("seg_full_seg"));
    let x = kick_signal(&ONSETS, 3.5, SR);
    let dur = x.len() as f64 / f64::from(SR);
    let dir = project(&root, "Plain", &[kick_spec("k in", "kick", 36)]);
    let dir_seg = project_segments(
        &root_seg,
        "Seg",
        &[SegSpec {
            name: "k in",
            role: "kick",
            x: x.clone(),
            segs: vec![(0.0, 0.0, dur)],
        }],
    );
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let svc_seg = service_dsp();
    svc_seg.set_home(root_seg.join("home"));
    let sid = analyze_all(&svc, &dir);
    let sid_seg = analyze_all(&svc_seg, &dir_seg);
    let a = serde_json::to_string(&events(&svc, &sid, "k in")).unwrap();
    let b = serde_json::to_string(&events(&svc_seg, &sid_seg, "k in")).unwrap();
    assert!(!a.is_empty() && a != "[]", "no hits at all: {a}");
    assert_eq!(a, b, "the segmented track's events differ from the plain track's");
}

/// Gate 2: two segments of one file with a gap: the hits of each segment sit at
/// `timeline start + offset in the segment`, exact to the sample; a hit in the gap is absent.
#[test]
fn two_segments_with_a_gap_place_their_hits_on_the_timeline() {
    let (root, root_seg) = (tmp("seg_gap_full"), tmp("seg_gap_seg"));
    // onsets 1 s apart: every cut lands in written silence (a kick's tail ends 0.4 s after
    // its onset, the next onset is 1.0 s later)
    let onsets = [0.25, 1.25, 2.25, 3.25];
    let x = kick_signal(&onsets, 4.0, SR);
    // segment A: file 0..1 s at timeline 0; segment B: file 2..4 s at timeline 3
    // (timeline gap 1..3: the hit at 1.25 is absent)
    let dir_seg = project_segments(
        &root_seg,
        "Seg",
        &[SegSpec {
            name: "k in",
            role: "kick",
            x: x.clone(),
            segs: vec![(0.0, 0.0, 1.0), (3.0, 2.0, 2.0)],
        }],
    );
    let dir = project(
        &root,
        "Full",
        &[Spec {
            name: "k in",
            role: "kick",
            note: 36,
            x: x.clone(),
            sr: SR,
        }],
    );
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let svc_seg = service_dsp();
    svc_seg.set_home(root_seg.join("home"));
    let sid_full = analyze_all(&svc, &dir);
    let sid_seg = analyze_all(&svc_seg, &dir_seg);
    let full = at_samples(&times(&events(&svc, &sid_full, "k in")));
    let seg = at_samples(&times(&events(&svc_seg, &sid_seg, "k in")));
    // expected: every full-file hit inside a segment, moved by `start - offset`, exact to
    // the sample; the others (in the gap or a trimmed part) absent
    let mut expected = Vec::new();
    for s in full {
        let f = s as f64 / f64::from(SR);
        if f < 1.0 {
            expected.push(s);
        } else if (2.0..4.0).contains(&f) {
            expected.push(((f - 2.0 + 3.0) * f64::from(SR)).round() as i64);
        }
    }
    expected.sort_unstable();
    let mut got = seg.clone();
    got.sort_unstable();
    assert!(!expected.is_empty(), "the full-file track found no hits");
    assert_eq!(got, expected, "segmented hits are not the shifted segment hits");
    // and no hit in the gap
    for t in &seg {
        let f = *t as f64 / f64::from(SR);
        assert!(!((1.0..3.0).contains(&f)), "a hit in the gap at {f}s");
    }
}

/// Gate 3: the session rules run on timeline positions. The flam rule on a session where the
/// snare and its sources sit at timeline start 10 s (one segment each; the tom stays plain
/// at 0) marks the same hits as the unshifted session, shifted by exactly 10 s.
#[test]
fn the_flam_rule_marks_the_same_hits_on_shifted_tracks() {
    let (root, root_seg) = (tmp("seg_flam"), tmp("seg_flam_shift"));
    let (x_snare, _mains) = flam_track(3);
    let (x_kick, x_hat, x_tom) = (other_track(5, 60.0), other_track(7, 0.0), few_track(9));
    let dir = project(
        &root,
        "Flams",
        &[
            spec("s top", "snare", x_snare.clone()),
            spec("s samp", "snare", x_snare.clone()),
            spec("k in", "kick", x_kick.clone()),
            spec("hh", "hat", x_hat.clone()),
            spec("t1", "tom", x_tom.clone()),
        ],
    );
    let shift = 10.0;
    let span = |x: &[f32]| x.len() as f64 / f64::from(SR);
    let dir_seg = project_segments(
        &root_seg,
        "Flams",
        &[
            SegSpec {
                name: "s top",
                role: "snare",
                x: x_snare.clone(),
                segs: vec![(shift, 0.0, span(&x_snare))],
            },
            SegSpec {
                name: "s samp",
                role: "snare",
                x: x_snare.clone(),
                segs: vec![(shift, 0.0, span(&x_snare))],
            },
            SegSpec {
                name: "k in",
                role: "kick",
                x: x_kick.clone(),
                segs: vec![(shift, 0.0, span(&x_kick))],
            },
            SegSpec {
                name: "hh",
                role: "hat",
                x: x_hat.clone(),
                segs: vec![(shift, 0.0, span(&x_hat))],
            },
            SegSpec {
                name: "t1",
                role: "tom",
                x: x_tom,
                segs: Vec::new(),
            },
        ],
    );
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let svc_seg = service_dsp();
    svc_seg.set_home(root_seg.join("home"));
    let sid = analyze_all(&svc, &dir);
    let sid_seg = analyze_all(&svc_seg, &dir_seg);
    let unshifted = at_samples(&flams(&svc, &sid, "s top"));
    let shifted = at_samples(&flams(&svc_seg, &sid_seg, "s top"));
    assert_eq!(
        unshifted.len(),
        shifted.len(),
        "the shifted session marks a different number of flams"
    );
    let mut want: Vec<i64> = unshifted
        .iter()
        .map(|t| t + (shift * f64::from(SR)).round() as i64)
        .collect();
    want.sort_unstable();
    let mut got = shifted;
    got.sort_unstable();
    assert_eq!(got, want, "the shifted flam hits are not the unshifted ones + 10 s");
}

fn flams(svc: &Service, sid: &str, name: &str) -> Vec<f64> {
    events(svc, sid, name)
        .iter()
        .filter(|e| e["flam"] == true)
        .map(|e| e["t"].as_f64().unwrap())
        .collect()
}

/// Gate 4: a project file written before this change (no `segments` key) loads and analyses
/// to byte-identical events, and the document round-trips byte-identically.
#[test]
fn an_old_project_file_loads_and_analyses_byte_identically() {
    let (root, root_plain) = (tmp("seg_old"), tmp("seg_old_plain"));
    let dir = project(&root, "Old", &[kick_spec("k in", "kick", 36)]);
    let doc = dir.join("Old.drumspike");
    let text = String::from_utf8(std::fs::read(&doc).unwrap()).unwrap();
    let d = drumspike_core::Document::from_json(&text).unwrap();
    for (_, tr) in d.tracks() {
        assert!(tr.segments.is_none(), "an old track got segments");
        assert!(tr.segments().is_empty());
    }
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    let sid = analyze_all(&svc, &dir);
    let ev = serde_json::to_string(&events(&svc, &sid, "k in")).unwrap();
    assert!(!ev.is_empty() && ev != "[]");
    // the same audio as a plain track in a fresh project: identical events
    let dir2 = project(&root_plain, "Old2", &[kick_spec("k in", "kick", 36)]);
    let svc2 = service_dsp();
    svc2.set_home(root_plain.join("home"));
    let sid2 = analyze_all(&svc2, &dir2);
    assert_eq!(
        ev,
        serde_json::to_string(&events(&svc2, &sid2, "k in")).unwrap(),
        "the old project's events differ from a plain track of the same audio"
    );
    // and the document round-trips: load + save rewrites the same document (the save's
    // `modified` aside, no `segments` key appears and no key moves)
    let mut p = load_project(&dir).unwrap();
    save_project(&mut p).unwrap();
    let after = String::from_utf8(std::fs::read(&doc).unwrap()).unwrap();
    let strip = |s: &str| -> Value {
        let mut v: Value = serde_json::from_str(s).unwrap();
        v.as_object_mut().unwrap().remove("modified");
        v
    };
    assert_eq!(strip(&text), strip(&after), "the document changed through load + save");
    assert!(
        !after.contains("\"segments\""),
        "a saved old document gained a segments key"
    );
}

// ---------- the flam fixtures of tests/flam.rs (copied; a test binary cannot import another) ----------

fn spec(name: &'static str, role: &'static str, x: Vec<f32>) -> Spec {
    let note = match role {
        "kick" => 36,
        "hat" => 42,
        _ => 38,
    };
    Spec {
        name,
        role,
        note,
        x,
        sr: SR,
    }
}

/// A deterministic standard normal source (LCG and Box-Muller).
struct Normal(u64);

impl Normal {
    fn uniform(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 11) as f64 + 0.5) / (1u64 << 53) as f64
    }
    fn next(&mut self) -> f64 {
        let (a, b) = (self.uniform(), self.uniform());
        (-2.0 * a.ln()).sqrt() * (2.0 * std::f64::consts::PI * b).cos()
    }
}

/// The DRT kit's snare: a 190 Hz body (40 ms decay) and noise (`decay` s), 250 ms long.
fn snare(rng: &mut Normal, decay: f64) -> Vec<f32> {
    (0..SR as usize / 4)
        .map(|i| {
            let t = i as f64 / f64::from(SR);
            let body = (2.0 * std::f64::consts::PI * 190.0 * t).sin() * (-t / 0.04).exp();
            let noise = rng.next() * (-t / decay).exp() * 0.6;
            ((body + noise) / 1.6) as f32
        })
        .collect()
}

fn place(buf: &mut [f32], s: &[f32], at_s: f64, db: f64) {
    let at = (at_s * f64::from(SR)).round() as usize;
    let g = 10f64.powf(db / 20.0) as f32;
    for (i, v) in s.iter().enumerate() {
        if at + i < buf.len() {
            buf[at + i] += g * v;
        }
    }
}

/// A snare track: a backbeat every second at -6 dB from 1 s; at 3, 6 and 9 s a close pair
/// instead (the flam). Returns the signal.
fn flam_track(seed: u64) -> (Vec<f32>, Vec<f64>) {
    let mut rng = Normal(seed);
    let mut x = vec![0.0f32; 12 * SR as usize];
    let mut mains = Vec::new();
    for i in 0..10 {
        let t = 1.0 + f64::from(i);
        if i % 3 == 2 {
            let g = snare(&mut rng, 0.008);
            place(&mut x, &g, t, -8.0);
            let m = snare(&mut rng, 0.06);
            place(&mut x, &m, t + 0.03, -6.0);
            mains.push(t + 0.03);
        } else {
            let s = snare(&mut rng, 0.06);
            place(&mut x, &s, t, -6.0);
        }
    }
    for v in &mut x {
        *v += 1e-4 * rng.next() as f32;
    }
    (x, mains)
}

/// A kick or hat track: a hit every 0.5 s from 1.25 s (never at a flam).
fn other_track(seed: u64, hz: f64) -> Vec<f32> {
    let mut rng = Normal(seed);
    let mut x = vec![0.0f32; 12 * SR as usize];
    for i in 0..20 {
        let at = ((1.25 + 0.5 * f64::from(i)) * f64::from(SR)) as usize;
        for k in 0..(SR as usize / 10) {
            let t = k as f64 / f64::from(SR);
            let v = if hz > 0.0 {
                (2.0 * std::f64::consts::PI * hz * t).sin() * (-t / 0.05).exp()
            } else {
                rng.next() * (-t / 0.015).exp() * 0.3
            };
            x[at + k] += (0.5 * v) as f32;
        }
    }
    x
}

/// A tom track with 3 hits: too few strong hits for a transfer gain.
fn few_track(seed: u64) -> Vec<f32> {
    let mut rng = Normal(seed);
    let mut x = vec![0.0f32; 12 * SR as usize];
    for i in 0..3 {
        let at = ((1.25 + 3.0 * f64::from(i)) * f64::from(SR)) as usize;
        for k in 0..(SR as usize / 10) {
            let t = k as f64 / f64::from(SR);
            x[at + k] += (0.4 * rng.next() * (-t / 0.02).exp()) as f32;
        }
    }
    x
}
