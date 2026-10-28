//! P4-7c: the sampler calibration through `drumspike_app::Service`: the calibration MIDI file, the
//! bounce measured against planted delays and polarities, and the MIDI exports that place each note at
//! the aligned attack minus the offset (per track, Export All, the multi-track file; the lags of the
//! layer's alignment; a note before 0 clamped with a warning). The test names start with the command
//! name; `command_tests.rs` checks that.

mod common;

use std::path::{Path, PathBuf};

use common::*;
use drumspike_app::export::ExportOpts;
use drumspike_app::Service;
use drumspike_core::io::{write_wav as write_core, WavFormat, WavSpec};
use drumspike_core::transport::device::Output;
use midly::{MidiMessage, Smf, TrackEventKind};
use serde_json::{json, Value};

const SR: u32 = 48_000;

/// A drum-like shot: `pre` samples of silence, a decaying 150 Hz tone and a short noise burst.
fn shot(pre: usize, secs: f64, seed: u64) -> Vec<f32> {
    let mut s = seed;
    let n = (secs * SR as f64) as usize;
    (0..n)
        .map(|i| {
            if i < pre {
                return 0.0;
            }
            s = s
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let u = ((s >> 33) as f64 / (1u64 << 31) as f64) * 2.0 - 1.0;
            let t = (i - pre) as f64 / SR as f64;
            (0.6 * (2.0 * std::f64::consts::PI * 150.0 * t).sin() * (-t / 0.05).exp() + 0.4 * u * (-t / 0.003).exp())
                as f32
        })
        .collect()
}

fn wav(path: &Path, x: &[f32]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let spec = WavSpec {
        sample_rate: SR,
        channels: 1,
        format: WavFormat::Float32,
        extensible: false,
        channel_mask: 0,
    };
    write_core(path, &spec, x, None).unwrap();
}

/// A bounce of the sampler: its transient `delay` samples after the calibration note at 2.000 s.
fn bounce(path: &Path, delay: usize, sign: f32) {
    let x: Vec<f32> = shot(2 * SR as usize + delay, 4.0, 1)
        .iter()
        .map(|v| sign * 0.5 * v)
        .collect();
    wav(path, &x);
}

/// The hits of the test track: seconds, and (0.005 s) one so early that a sampler offset puts it before 0.
const HITS: [f64; 5] = [0.005, 0.5, 1.5, 2.5, 3.5];

/// A session with a kick track `k in` (the one-shot at each hit, the attack `lag` samples after the hit) and the
/// hits. A silent service.
fn session(tag: &str, lag: usize) -> (Service, String, PathBuf) {
    let root = tmp(tag);
    let mut x = vec![0.0f32; 5 * SR as usize];
    for t in HITS {
        let at = (t * SR as f64).round() as usize + lag;
        for (i, v) in shot(0, 0.4, 1).iter().enumerate() {
            x[at + i] += *v;
        }
    }
    let dir = project(
        &root,
        "Song",
        &[Spec {
            name: "k in",
            role: "kick",
            note: 36,
            x,
            sr: SR,
        }],
    );
    let svc = service_dsp();
    svc.set_output_select(Output::Null { sample_rate: None });
    svc.set_master_gain(0.0);
    let sid = svc.project_open(dir.to_str().unwrap()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    let ev: Vec<Value> = HITS.iter().map(|t| json!({"t": t, "v": 100})).collect();
    svc.events_set(&sid, "k in", &json!(ev)).unwrap();
    (svc, sid, root)
}

/// The note-ons of a MIDI file: (track name or index, tick, key, velocity).
fn notes(path: &str) -> Vec<(String, u64, u8, u8)> {
    let bytes = std::fs::read(path).unwrap();
    let smf = Smf::parse(&bytes).unwrap();
    let mut out = Vec::new();
    for (i, tr) in smf.tracks.iter().enumerate() {
        let mut name = i.to_string();
        let mut tick = 0u64;
        for e in tr {
            tick += e.delta.as_int() as u64;
            match e.kind {
                TrackEventKind::Meta(midly::MetaMessage::TrackName(n)) => {
                    name = String::from_utf8_lossy(n).into_owned()
                }
                TrackEventKind::Midi {
                    message: MidiMessage::NoteOn { key, vel },
                    ..
                } if vel.as_int() > 0 => out.push((name.clone(), tick, key.as_int(), vel.as_int())),
                _ => {}
            }
        }
    }
    out
}

fn ticks(path: &str) -> Vec<u64> {
    notes(path).into_iter().map(|n| n.1).collect()
}

fn set_bounce(svc: &Service, sid: &str, path: &Path) -> Value {
    svc.track_sampler_set(sid, "k in", &json!({"path": path.to_str().unwrap()}))
        .unwrap()
}

/// The note ticks the hits would have without any calibration.
fn plain_ticks() -> Vec<u64> {
    HITS.iter().map(|t| (t * SR as f64).round() as u64).collect()
}

#[test]
fn sampler_calibration_midi_to_writes_the_chosen_file_and_refuses_a_folder_or_a_missing_folder() {
    let (svc, sid, root) = session("cal_midi_to", 0);
    let want = root.join("picked").join("k in calibration.mid");
    std::fs::create_dir_all(want.parent().unwrap()).unwrap();
    let r = svc
        .sampler_calibration_midi_to(&sid, "k in", None, want.to_str())
        .unwrap();
    assert_eq!(Path::new(r["path"].as_str().unwrap()), want);
    assert_eq!(
        notes(want.to_str().unwrap()),
        vec![("spikes".to_string(), 96_000, 36, 100)]
    );
    // the save is one file: a folder is 400, a relative path 400, a folder that does not exist 404
    let code = |p: &str| {
        svc.sampler_calibration_midi_to(&sid, "k in", None, Some(p))
            .unwrap_err()
            .code
    };
    assert_eq!(code(root.to_str().unwrap()), 400);
    assert_eq!(code("rel.mid"), 400);
    assert_eq!(code(root.join("nope").join("x.mid").to_str().unwrap()), 404);
}

#[test]
fn sampler_calibration_midi_writes_one_note_on_bar_2_in_the_destination() {
    let (svc, sid, root) = session("cal_midi", 0);
    let dest = root.join("out");
    std::fs::create_dir_all(&dest).unwrap();
    let r = svc
        .sampler_calibration_midi(&sid, "k in", Some(dest.to_str().unwrap()))
        .unwrap();
    let path = r["path"].as_str().unwrap();
    assert_eq!(Path::new(path), dest.join("k in_calibration.mid"));
    assert_eq!(notes(path), vec![("spikes".to_string(), 96_000, 36, 100)]);
    assert_eq!(
        (r["t"].as_f64(), r["tick"].as_u64(), r["note"].as_i64()),
        (Some(2.0), Some(96_000), Some(36))
    );
    let text = r["text"].as_str().unwrap();
    assert!(
        text.contains("session start") && text.contains("bounce") && text.contains("load the bounce"),
        "{text}"
    );
    // no dest: Exports/ of the project; a bad dest: 400; the track's own note
    svc.track_set_note(&sid, "k in", 35).unwrap();
    let r = svc.sampler_calibration_midi(&sid, "k in", None).unwrap();
    assert!(r["path"].as_str().unwrap().contains("Exports"));
    assert_eq!(notes(r["path"].as_str().unwrap())[0].2, 35);
    assert_eq!(
        svc.sampler_calibration_midi(&sid, "k in", Some("rel"))
            .unwrap_err()
            .code,
        400
    );
    assert_eq!(svc.sampler_calibration_midi(&sid, "nope", None).unwrap_err().code, 400);
    // a tempo map: bar 2 through the map (90 BPM, bar 1 at 1 s: 1 s + 4 beats of 2/3 s)
    svc.tempo_map_set(&sid, &json!({"bpm": 90.0, "bar1_s": 1.0})).unwrap();
    let r = svc.sampler_calibration_midi(&sid, "k in", None).unwrap();
    assert!(
        (r["t"].as_f64().unwrap() - (1.0 + 4.0 * 60.0 / 90.0)).abs() < 1e-4,
        "{}",
        r["t"]
    );
    let tick = r["tick"].as_u64().unwrap();
    assert_eq!(ticks(r["path"].as_str().unwrap()), vec![tick]);
}

#[test]
fn track_sampler_set_measures_a_planted_delay_and_polarity_and_keeps_them() {
    let (svc, sid, root) = session("sampler_set", 0);
    let shots = root.join("shots");
    // nothing set: all null
    let g = svc.track_sampler_get(&sid, "k in").unwrap();
    assert_eq!(
        (g["sampler"].clone(), g["offset_ms"].clone(), g["polarity"].clone()),
        (Value::Null, Value::Null, Value::Null)
    );
    // the reference: no delay (the onset rule has its own bias, the same for every delay)
    bounce(&shots.join("b0.wav"), 0, 1.0);
    let base = set_bounce(&svc, &sid, &shots.join("b0.wav"))["sampler"]["offset_samples"]
        .as_f64()
        .unwrap();
    assert!(base.abs() <= 24.0, "bias {base}");
    for d in [48usize, 312, 1_000, 2_400] {
        let p = shots.join(format!("b{d}.wav"));
        bounce(&p, d, 1.0);
        let r = set_bounce(&svc, &sid, &p);
        let s = &r["sampler"];
        assert_eq!(s["offset_samples"].as_f64().unwrap() - base, d as f64, "delay {d}");
        assert_eq!(
            (s["sr"].clone(), r["polarity"].clone(), r["file"].clone()),
            (json!(48_000), json!("normal"), json!(format!("b{d}.wav")))
        );
        assert!(s["match"].as_f64().unwrap() > 0.9, "{s}");
        assert!(s["measured_at"].as_f64().unwrap() > 1.0e9);
        assert!((r["offset_ms"].as_f64().unwrap() - (base + d as f64) / 48.0).abs() < 1e-3);
    }
    // an inverted bounce
    let p = shots.join("inv.wav");
    bounce(&p, 312, -1.0);
    let r = set_bounce(&svc, &sid, &p);
    assert_eq!(r["polarity"], "inverted");
    assert_eq!(
        r["sampler"]["offset_samples"].as_f64().unwrap() - base,
        312.0,
        "the delay does not depend on the sign"
    );
    // the page reads it in tracks_get; it survives a save and a reopen; the project is dirty
    assert_eq!(svc.tracks_get(&sid).unwrap()["tracks"]["k in"]["sampler"], r["sampler"]);
    svc.project_save(&sid).unwrap();
    let dir = svc.project_get(&sid).unwrap()["folder"].as_str().unwrap().to_string();
    svc.project_close(&sid, true).unwrap();
    let sid2 = svc.project_open(&dir).unwrap()["sid"].as_str().unwrap().to_string();
    assert_eq!(svc.track_sampler_get(&sid2, "k in").unwrap()["sampler"], r["sampler"]);
    // a silent bounce and a missing file are 400s and leave the old value
    wav(&shots.join("silent.wav"), &vec![0.0; 48_000]);
    assert_eq!(
        svc.track_sampler_set(
            &sid2,
            "k in",
            &json!({"path": shots.join("silent.wav").to_str().unwrap()})
        )
        .unwrap_err()
        .code,
        400
    );
    assert_eq!(
        svc.track_sampler_set(&sid2, "k in", &json!({"path": "/no/such.wav"}))
            .unwrap_err()
            .code,
        400
    );
    assert_eq!(svc.track_sampler_get(&sid2, "k in").unwrap()["sampler"], r["sampler"]);
    // an empty path removes it
    let r = svc.track_sampler_set(&sid2, "k in", &json!({"path": ""})).unwrap();
    assert_eq!(r["sampler"], Value::Null);
    assert!(svc.tracks_get(&sid2).unwrap()["tracks"]["k in"]
        .get("sampler")
        .is_none());
}

#[test]
fn track_sampler_set_as_one_shot_sets_the_layer_with_the_polarity() {
    let (svc, sid, root) = session("sampler_shot", 0);
    let p = root.join("shots").join("inv.wav");
    bounce(&p, 480, -1.0);
    let r = svc
        .track_sampler_set(&sid, "k in", &json!({"path": p.to_str().unwrap(), "one_shot": true}))
        .unwrap();
    assert_eq!(r["layer"]["layer"]["polarity"], "invert");
    assert_eq!(r["layer"]["file"], "inv.wav");
    let p2 = root.join("shots").join("norm.wav");
    bounce(&p2, 480, 1.0);
    let r = svc
        .track_sampler_set(&sid, "k in", &json!({"path": p2.to_str().unwrap(), "one_shot": true}))
        .unwrap();
    assert_eq!(r["layer"]["layer"]["polarity"], "normal");
    // without one_shot the layer is not touched
    let (svc, sid, root) = session("sampler_shot2", 0);
    let p = root.join("shots").join("b.wav");
    bounce(&p, 480, 1.0);
    let r = set_bounce(&svc, &sid, &p);
    assert_eq!(r["layer"], Value::Null);
    assert!(svc.tracks_get(&sid).unwrap()["tracks"]["k in"].get("layer").is_none());
}

/// Calibrate `k in` with a bounce `d` samples late; returns the stored offset in samples.
fn calibrate(svc: &Service, sid: &str, root: &Path, d: usize) -> f64 {
    let p = root.join("shots").join(format!("cal{d}.wav"));
    bounce(&p, d, 1.0);
    set_bounce(svc, sid, &p)["sampler"]["offset_samples"].as_f64().unwrap()
}

fn expect(offset: f64) -> Vec<u64> {
    plain_ticks()
        .iter()
        .map(|t| (*t as f64 - offset).round().max(0.0) as u64)
        .collect()
}

#[test]
fn track_sampler_get_then_every_midi_export_moves_the_notes_by_the_offset() {
    let (svc, sid, root) = session("sampler_midi", 0);
    let dest = root.join("out");
    std::fs::create_dir_all(&dest).unwrap();
    let d = dest.to_str().unwrap();
    // before a calibration: unchanged
    let r = svc
        .export_midi_stored_to(&sid, "k in", None, None, None, None, Some(d))
        .unwrap();
    assert_eq!(ticks(r["path"].as_str().unwrap()), plain_ticks());
    assert!(r.get("sampler").is_none());
    let off = calibrate(&svc, &sid, &root, 480);
    assert!((off - 480.0).abs() <= 24.0);
    // per track (the page's and the agent's call), with the clamped first note
    let r = svc
        .export_midi_stored_to(&sid, "k in", None, None, None, None, Some(d))
        .unwrap();
    assert_eq!(ticks(r["path"].as_str().unwrap()), expect(off));
    assert_eq!(
        ticks(r["path"].as_str().unwrap())[0],
        0,
        "0.005 s is 240 samples: before 0"
    );
    assert_eq!(r["sampler"]["clamped"], 1);
    assert_eq!(r["sampler"]["aligned"], false);
    assert!((r["sampler"]["offset_ms"].as_f64().unwrap() - off / 48.0).abs() < 1e-3);
    assert!(
        r["warnings"][0].as_str().unwrap().contains("before the start"),
        "{}",
        r["warnings"]
    );
    // the option off: unchanged again
    let r = svc
        .export_midi_stored_with(&sid, "k in", None, None, None, None, Some(d), false)
        .unwrap();
    assert_eq!(ticks(r["path"].as_str().unwrap()), plain_ticks());
    // events passed in (the page sends its copy) and cut hits left out
    let ev = [json!({"t": 1.0, "v": 90}), json!({"t": 2.0, "cut": true, "v": 90})];
    let r = svc
        .export_midi_stored_to(&sid, "k in", Some(36), Some(&ev), None, None, Some(d))
        .unwrap();
    assert_eq!(
        ticks(r["path"].as_str().unwrap()),
        vec![(48_000.0 - off).round() as u64]
    );
    // Export All, per track files
    let o = ExportOpts::from_args(&json!({"dest": d, "formats": ["midi"]})).unwrap();
    let r = svc.export_all_opts(&sid, &o, None).unwrap();
    assert_eq!(ticks(r["files"][0].as_str().unwrap()), expect(off));
    assert_eq!(r["sampler"]["k in"]["clamped"], 1);
    assert_eq!(r["warnings"].as_array().unwrap().len(), 1);
    // ... the multi-track file
    let o = ExportOpts::from_args(&json!({"dest": d, "formats": ["midi"], "midi_mode": "single"})).unwrap();
    let r = svc.export_all_opts(&sid, &o, None).unwrap();
    assert_eq!(ticks(r["midi_file"].as_str().unwrap()), expect(off));
    assert_eq!(r["sampler"]["k in"]["notes"], 5);
    // ... the drag files of the Export dialog
    let o = ExportOpts::from_args(&json!({"formats": ["midi"]})).unwrap();
    let r = svc.export_drag_prepare(&sid, &o).unwrap();
    assert_eq!(ticks(r["midi"][0].as_str().unwrap()), expect(off));
    // ... and with compensate off in the options: unchanged
    let o = ExportOpts::from_args(&json!({"formats": ["midi"], "compensate": false})).unwrap();
    assert!(!o.compensate);
    let r = svc.export_drag_prepare(&sid, &o).unwrap();
    assert_eq!(ticks(r["midi"][0].as_str().unwrap()), plain_ticks());
    assert_eq!(
        ExportOpts::from_args(&json!({"compensate": "yes"})).unwrap_err().code,
        400
    );
    // a negative offset (the sampler plays early) moves the notes later
    let early = shot_early(&svc, &sid, &root);
    assert!(early < 0.0);
    let r = svc
        .export_midi_stored_to(&sid, "k in", None, None, None, None, Some(d))
        .unwrap();
    assert_eq!(ticks(r["path"].as_str().unwrap()), expect(early));
}

/// A bounce whose transient is before the note: offset below 0.
fn shot_early(svc: &Service, sid: &str, root: &Path) -> f64 {
    let p = root.join("shots").join("early.wav");
    let x: Vec<f32> = shot(2 * SR as usize - 960, 4.0, 1);
    wav(&p, &x);
    set_bounce(svc, sid, &p)["sampler"]["offset_samples"].as_f64().unwrap()
}

#[test]
fn track_sampler_set_with_a_layer_alignment_the_notes_follow_the_aligned_attack() {
    // the mic's attack sits 7 samples after the hit time: the alignment lag is about +7
    let (svc, sid, root) = session("sampler_align", 7);
    let dest = root.join("out");
    std::fs::create_dir_all(&dest).unwrap();
    let d = dest.to_str().unwrap();
    let off = calibrate(&svc, &sid, &root, 600);
    let lay = root.join("shots").join("one.wav");
    wav(&lay, &shot(240, 0.3, 1));
    svc.track_layer_set(&sid, "k in", &json!({"path": lay.to_str().unwrap(), "align": true}))
        .unwrap();
    let al = svc.track_layer_align(&sid, "k in").unwrap();
    let lags: Vec<f64> = al["hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["lag"].as_f64().unwrap())
        .collect();
    assert!(lags.iter().skip(1).all(|l| (l - 7.0).abs() < 1.5), "{lags:?}");
    let r = svc
        .export_midi_stored_to(&sid, "k in", None, None, None, None, Some(d))
        .unwrap();
    assert_eq!(r["sampler"]["aligned"], true);
    let got = ticks(r["path"].as_str().unwrap());
    for (k, t) in plain_ticks().iter().enumerate() {
        let want = (*t as f64 + lags[k] - off).max(0.0);
        assert!((got[k] as f64 - want).abs() <= 1.0, "note {k}: {} for {want}", got[k]);
    }
    // the notes of the later hits are about 7 samples after the notes without the alignment
    let plain = expect(off);
    assert!(
        (got[3] as f64 - plain[3] as f64 - 7.0).abs() <= 1.5,
        "{} {}",
        got[3],
        plain[3]
    );
    // Align off: the notes are on the hits again
    svc.track_layer_set(&sid, "k in", &json!({"align": false})).unwrap();
    let r = svc
        .export_midi_stored_to(&sid, "k in", None, None, None, None, Some(d))
        .unwrap();
    assert_eq!(r["sampler"]["aligned"], false);
    assert_eq!(ticks(r["path"].as_str().unwrap()), plain);
}
