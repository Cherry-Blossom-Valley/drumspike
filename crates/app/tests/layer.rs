//! P4-7a and P4-7b: the one-shot layer of a track through `drumspike_app::Service`: set, get, the
//! render (`export_layer`, read back sample by sample), the per-hit alignment with planted lags and
//! an inverted polarity, and the voices of the transport on the null device (silent). The test names
//! start with the command name; `command_tests.rs` checks that.

mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use common::*;
use drumspike_app::Service;
use drumspike_core::io::{Bext, WavFormat, WavSpec, read_wav_bytes, write_wav as write_core};
use drumspike_core::layer::{OneShot, render_layer};
use drumspike_core::transport::device::Output;
use serde_json::{Value, json};

/// A drum-like one-shot: `pre` samples of silence, a decaying 150 Hz tone and a short noise burst.
fn shot(sr: u32, pre: usize, secs: f64, seed: u64) -> Vec<f32> {
    let mut s = seed;
    let n = (secs * sr as f64) as usize;
    (0..n)
        .map(|i| {
            if i < pre {
                return 0.0;
            }
            s = s
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let u = ((s >> 33) as f64 / (1u64 << 31) as f64) * 2.0 - 1.0;
            let t = (i - pre) as f64 / sr as f64;
            (0.6 * (2.0 * std::f64::consts::PI * 150.0 * t).sin() * (-t / 0.05).exp() + 0.4 * u * (-t / 0.003).exp())
                as f32
        })
        .collect()
}

fn wav(path: &Path, sr: u32, chans: &[Vec<f32>], bext: Option<&Bext>) {
    let n = chans[0].len();
    let inter: Vec<f32> = (0..n).flat_map(|i| chans.iter().map(move |c| c[i])).collect();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let spec = WavSpec {
        sample_rate: sr,
        channels: chans.len() as u16,
        format: WavFormat::Float32,
        extensible: false,
        channel_mask: 0,
    };
    write_core(path, &spec, &inter, bext).unwrap();
}

/// Read a float WAV: (rate, channels planar).
fn read_back(path: &Path) -> (u32, Vec<Vec<f32>>, Option<u64>) {
    let (bytes, info) = read_wav_bytes(path).unwrap();
    let ch = info.channels as usize;
    let d = &bytes[info.data_offset..info.data_offset + info.n_frames * info.block_align];
    let x: Vec<f32> = d.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect();
    let chans = (0..ch)
        .map(|c| x.iter().skip(c).step_by(ch).copied().collect())
        .collect();
    (info.sample_rate, chans, info.time_reference())
}

fn open(svc: &Service, dir: &Path) -> String {
    svc.project_open(dir.to_str().unwrap()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string()
}

/// A project with one kick track `k in` and the given hits (time, velocity); a silent service.
fn session(tag: &str, x: Vec<f32>, sr: u32, hits: &[(f64, i64)]) -> (Service, String, PathBuf) {
    let root = tmp(tag);
    let dir = project(
        &root,
        "Song",
        &[Spec {
            name: "k in",
            role: "kick",
            note: 36,
            x,
            sr,
        }],
    );
    let svc = service_dsp();
    svc.set_output_select(Output::Null { sample_rate: None });
    svc.set_master_gain(0.0);
    let sid = open(&svc, &dir);
    let ev: Vec<Value> = hits.iter().map(|(t, v)| json!({"t": t, "v": v})).collect();
    svc.events_set(&sid, "k in", &json!(ev)).unwrap();
    (svc, sid, root)
}

const HITS: [(f64, i64); 4] = [(0.25, 127), (0.75, 100), (1.25, 64), (1.75, 1)];

#[test]
fn track_layer_set_reads_the_file_measures_the_onset_and_stores_the_layer() {
    let (svc, sid, root) = session("layer_set", vec![0.0; 96_000], 48_000, &HITS);
    let f = root.join("shots").join("kick one.wav");
    wav(&f, 48_000, &[shot(48_000, 480, 0.3, 1)], None);
    // a setting before a file: 409
    assert_eq!(
        svc.track_layer_set(&sid, "k in", &json!({"level_db": -3}))
            .unwrap_err()
            .code,
        409
    );
    let r = svc
        .track_layer_set(&sid, "k in", &json!({"path": f.to_str().unwrap()}))
        .unwrap();
    let l = &r["layer"];
    let onset = l["onset"].as_u64().unwrap();
    assert!((475..=490).contains(&onset), "onset {onset} for the attack at 480");
    assert_eq!((l["sr"].clone(), l["channels"].clone()), (json!(48_000), json!(1)));
    assert_eq!(
        (
            l["level_db"].clone(),
            l["polarity"].clone(),
            l["on"].clone(),
            l["align"].clone()
        ),
        (json!(0.0), json!("normal"), json!(true), json!(false))
    );
    assert_eq!(l["curve"], json!({"kind": "db", "range_db": 24.0}));
    assert_eq!(r["status"], "ok");
    assert_eq!(r["file"], "kick one.wav");
    assert!((r["onset_ms"].as_f64().unwrap() - onset as f64 / 48.0).abs() < 1e-3);
    assert_eq!(svc.project_get(&sid).unwrap()["dirty"], true);
    // settings, one at a time; the file stays
    let r = svc
        .track_layer_set(
            &sid,
            "k in",
            &json!({"level_db": -80, "polarity": "invert", "on": false, "curve": {"kind": "fixed"}, "align": true}),
        )
        .unwrap();
    assert_eq!(r["layer"]["level_db"], json!(-60.0), "clamped");
    assert_eq!(
        (
            r["layer"]["polarity"].clone(),
            r["layer"]["on"].clone(),
            r["layer"]["align"].clone()
        ),
        (json!("invert"), json!(false), json!(true))
    );
    assert_eq!(r["layer"]["onset"], json!(onset));
    // the layer is in tracks_get (the page reads it there) and survives a save and a reopen
    assert_eq!(svc.tracks_get(&sid).unwrap()["tracks"]["k in"]["layer"], r["layer"]);
    svc.project_save(&sid).unwrap();
    let dir = PathBuf::from(svc.project_get(&sid).unwrap()["folder"].as_str().unwrap());
    svc.project_close(&sid, true).unwrap();
    let sid = open(&svc, &dir);
    assert_eq!(svc.track_layer_get(&sid, "k in").unwrap()["layer"], r["layer"]);
    // a new file keeps the settings, an AIFF file is taken
    let aif = root.join("shots").join("snare.aif");
    let be: Vec<u8> = shot(44_100, 0, 0.1, 2)
        .iter()
        .flat_map(|v| ((v * 32767.0) as i16).to_be_bytes())
        .collect();
    std::fs::write(&aif, aiff(&be, 1, 44_100)).unwrap();
    let r2 = svc
        .track_layer_set(&sid, "k in", &json!({"path": aif.to_str().unwrap()}))
        .unwrap();
    assert_eq!(
        (r2["layer"]["sr"].clone(), r2["layer"]["polarity"].clone()),
        (json!(44_100), json!("invert"))
    );
    // wrong values: 400
    for bad in [
        json!({"polarity": "up"}),
        json!({"level_db": "x"}),
        json!({"curve": {"kind": "exp"}}),
        json!({"on": 1}),
        json!({"path": 3}),
    ] {
        assert_eq!(svc.track_layer_set(&sid, "k in", &bad).unwrap_err().code, 400, "{bad}");
    }
    std::fs::write(root.join("x.wav"), b"not audio").unwrap();
    assert_eq!(
        svc.track_layer_set(&sid, "k in", &json!({"path": root.join("x.wav").to_str().unwrap()}))
            .unwrap_err()
            .code,
        400
    );
    assert_eq!(svc.track_layer_set(&sid, "nope", &json!({})).unwrap_err().code, 400);
    // null (and "") removes the layer
    let r = svc.track_layer_set(&sid, "k in", &json!({"path": null})).unwrap();
    assert_eq!(r["layer"], Value::Null);
    assert!(svc.tracks_get(&sid).unwrap()["tracks"]["k in"].get("layer").is_none());
    svc.track_layer_set(&sid, "k in", &json!({"path": f.to_str().unwrap()}))
        .unwrap();
    assert_eq!(
        svc.track_layer_set(&sid, "k in", &json!({"path": ""})).unwrap()["layer"],
        Value::Null
    );
}

/// A 16-bit mono or stereo AIFF file.
fn aiff(be: &[u8], ch: i16, sr: u32) -> Vec<u8> {
    let frames = (be.len() / 2 / ch as usize) as u32;
    let mut comm = Vec::new();
    comm.extend_from_slice(&ch.to_be_bytes());
    comm.extend_from_slice(&frames.to_be_bytes());
    comm.extend_from_slice(&16i16.to_be_bytes());
    // the rate as an 80-bit extended number (exponent 16383 + 15, the rate in the top bits)
    let e = 16383 + 15u16;
    comm.extend_from_slice(&e.to_be_bytes());
    comm.extend_from_slice(&((sr as u64) << 48).to_be_bytes());
    let mut ssnd = vec![0u8; 8];
    ssnd.extend_from_slice(be);
    let mut body = b"AIFF".to_vec();
    for (id, c) in [(b"COMM", comm), (b"SSND", ssnd)] {
        body.extend_from_slice(id);
        body.extend_from_slice(&(c.len() as u32).to_be_bytes());
        body.extend_from_slice(&c);
    }
    let mut out = b"FORM".to_vec();
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(&body);
    out
}

#[test]
fn track_layer_get_reports_a_missing_file_and_finds_a_moved_one() {
    let (svc, sid, root) = session("layer_get", vec![0.0; 96_000], 48_000, &HITS);
    assert_eq!(svc.track_layer_get(&sid, "k in").unwrap()["layer"], Value::Null);
    let f = root.join("shots").join("one.wav");
    wav(&f, 48_000, &[shot(48_000, 100, 0.2, 3)], None);
    svc.track_layer_set(&sid, "k in", &json!({"path": f.to_str().unwrap()}))
        .unwrap();
    let dir = PathBuf::from(svc.project_get(&sid).unwrap()["folder"].as_str().unwrap());
    // moved away: missing, the export says so
    let away = root.join("one.wav");
    std::fs::rename(&f, &away).unwrap();
    let r = svc.track_layer_get(&sid, "k in").unwrap();
    assert_eq!(
        (r["status"].clone(), r["path"].clone()),
        (json!("missing"), Value::Null)
    );
    assert_eq!(r["file"], "one.wav");
    assert_eq!(svc.export_layer(&sid, "k in").unwrap_err().code, 409);
    // into the project's Audio Files: found again (the source resolution order)
    let moved = dir.join("Audio Files").join("one.wav");
    std::fs::create_dir_all(moved.parent().unwrap()).unwrap();
    std::fs::rename(&away, &moved).unwrap();
    let r = svc.track_layer_get(&sid, "k in").unwrap();
    assert_eq!(r["status"], "ok");
    assert_eq!(r["path"], json!(moved.display().to_string()));
}

#[test]
fn export_layer_to_writes_the_chosen_file_and_refuses_the_one_shot_in_use() {
    let sr = 48_000u32;
    let root = tmp("layer_export_to");
    let dir = project(
        &root,
        "Song",
        &[Spec {
            name: "k in",
            role: "kick",
            note: 36,
            x: vec![0.0; 2 * sr as usize],
            sr,
        }],
    );
    let src = root.join("daw").join("k in.wav");
    wav(&src, sr, &[vec![0.0; 2 * sr as usize]], None);
    let svc = service_dsp();
    let sid = open(&svc, &dir);
    svc.project_relink(&sid, "k in", src.to_str().unwrap(), true).unwrap();
    let ev: Vec<Value> = HITS.iter().map(|(t, v)| json!({"t": t, "v": v})).collect();
    svc.events_set(&sid, "k in", &json!(ev)).unwrap();
    let f = root.join("shots").join("st.wav");
    wav(&f, sr, &[shot(sr, 240, 0.2, 4)], None);
    svc.track_layer_set(&sid, "k in", &json!({"path": f.to_str().unwrap()}))
        .unwrap();
    // the chosen file is written, and nothing else is
    let want = root.join("picked").join("k in layer.wav");
    std::fs::create_dir_all(want.parent().unwrap()).unwrap();
    svc.export_layer_to(&sid, "k in", want.to_str()).unwrap();
    assert!(want.is_file());
    // a save over the one-shot in use is refused, and the one-shot stays as it was
    let before = std::fs::read(&f).unwrap();
    assert_eq!(svc.export_layer_to(&sid, "k in", f.to_str()).unwrap_err().code, 409);
    assert_eq!(std::fs::read(&f).unwrap(), before);
    // a relative path is 400, a folder that does not exist is 404
    assert_eq!(
        svc.export_layer_to(&sid, "k in", Some("rel.wav")).unwrap_err().code,
        400
    );
    let missing = root.join("nope").join("x.wav");
    assert_eq!(
        svc.export_layer_to(&sid, "k in", missing.to_str()).unwrap_err().code,
        404
    );
}

#[test]
fn export_layer_places_the_onset_on_every_hit_sample() {
    let sr = 48_000u32;
    let bext = Bext {
        description: "take".into(),
        originator: "Pro Tools".into(),
        originator_reference: String::new(),
        origination_date: "2026-10-14".into(),
        origination_time: "10:00:00".into(),
        time_reference: 1_234_567,
        version: 1,
        coding_history: String::new(),
    };
    let root = tmp("layer_export");
    let dir = project(
        &root,
        "Song",
        &[Spec {
            name: "k in",
            role: "kick",
            note: 36,
            x: vec![0.0; 2 * sr as usize],
            sr,
        }],
    );
    // the source gets a bext chunk (the layer file takes its time reference)
    let src = root.join("daw").join("k in.wav");
    wav(&src, sr, &[vec![0.0; 2 * sr as usize]], Some(&bext));
    let svc = service_dsp();
    let sid = open(&svc, &dir);
    svc.project_relink(&sid, "k in", src.to_str().unwrap(), true).unwrap();
    let mut ev: Vec<Value> = HITS.iter().map(|(t, v)| json!({"t": t, "v": v})).collect();
    ev.push(json!({"t": 1.5, "v": 100, "cut": true}));
    ev.push(json!({"t": 5.0, "v": 100}));
    svc.events_set(&sid, "k in", &json!(ev)).unwrap();
    let left = shot(sr, 240, 0.2, 4);
    let right: Vec<f32> = left.iter().map(|v| -0.5 * v).collect();
    let f = root.join("shots").join("st.wav");
    wav(&f, sr, &[left.clone(), right.clone()], None);
    let onset = svc
        .track_layer_set(&sid, "k in", &json!({"path": f.to_str().unwrap()}))
        .unwrap()["layer"]["onset"]
        .as_u64()
        .unwrap() as usize;
    let r = svc.export_layer(&sid, "k in").unwrap();
    assert_eq!(
        (
            r["sr"].clone(),
            r["n_samples"].clone(),
            r["channels"].clone(),
            r["format"].clone()
        ),
        (json!(sr), json!(2 * sr), json!(2), json!("float32"))
    );
    assert_eq!(
        (r["onset"].clone(), r["outside"].clone(), r["aligned"].clone()),
        (json!(onset), json!(1), json!(false))
    );
    assert_eq!(r["time_reference"], json!(1_234_567));
    let path = PathBuf::from(r["path"].as_str().unwrap());
    assert!(path.ends_with("Exports/k in_layer.wav"));
    let (rate, out, tref) = read_back(&path);
    assert_eq!(
        (rate, out.len(), out[0].len(), tref),
        (sr, 2, 2 * sr as usize, Some(1_234_567))
    );
    // each hit: the one-shot's onset sample on the hit's sample, the gain of the dB curve
    let db = |v: i64| 10f64.powf(-24.0 * (1.0 - v as f64 / 127.0) / 20.0) as f32;
    for (t, v) in HITS {
        let i = (t * sr as f64).round() as usize;
        let g = db(v);
        for k in [0usize, 1, 2, 100, 5_000] {
            assert_eq!(out[0][i - onset + k], g * left[k], "hit {t} sample {k}");
            assert_eq!(out[1][i - onset + k], g * right[k], "right, hit {t} sample {k}");
        }
        assert_eq!(out[0][i], g * left[onset], "the onset on the hit");
    }
    // the cut hit has no layer
    let c = (1.5 * sr as f64) as usize;
    assert_eq!(out[0][c - onset + 300], 0.0);
    // inverted, fixed curve, -6 dB
    svc.track_layer_set(
        &sid,
        "k in",
        &json!({"polarity": "invert", "curve": {"kind": "fixed"}, "level_db": -6}),
    )
    .unwrap();
    let r = svc.export_layer(&sid, "k in").unwrap();
    assert_eq!(r["sign"], -1);
    let (_, out, _) = read_back(&path);
    let g = -(10f64.powf(-6.0 / 20.0)) as f32;
    for (t, _) in HITS {
        let i = (t * sr as f64).round() as usize;
        assert_eq!(out[0][i], g * left[onset], "inverted at {t}");
    }
    // a one-shot at 44.1 kHz on a 48 kHz track: resampled once, the onset at the track's rate.
    // The onset must map to a whole 48 kHz sample (441 -> 480) for the exact-sample check.
    let f2 = root.join("shots").join("low.wav");
    wav(&f2, 44_100, &[shot(44_100, 442, 0.2, 5)], None);
    svc.track_layer_set(
        &sid,
        "k in",
        &json!({"path": f2.to_str().unwrap(), "polarity": "normal"}),
    )
    .unwrap();
    let r = svc.export_layer(&sid, "k in").unwrap();
    assert_eq!(r["channels"], 1);
    let on48 = r["onset"].as_u64().unwrap() as usize;
    let l441 = svc.track_layer_get(&sid, "k in").unwrap()["layer"]["onset"]
        .as_u64()
        .unwrap() as f64;
    assert_eq!(on48, (l441 * 48_000.0 / 44_100.0).round() as usize);
    let (_, out, _) = read_back(&path);
    let rs = OneShot {
        sr: 44_100,
        chans: vec![shot(44_100, 442, 0.2, 5)],
    }
    .resampled(48_000);
    let i = (0.25 * sr as f64).round() as usize;
    // the level (-6 dB) and the fixed curve stay from before
    assert_eq!(out[0][i], (10f64.powf(-6.0 / 20.0) as f32) * rs.chans[0][on48]);
    // no layer: 409; another track's file names: 409 (the P4-4 owner rule)
    svc.track_layer_set(&sid, "k in", &json!({"path": null})).unwrap();
    assert_eq!(svc.export_layer(&sid, "k in").unwrap_err().code, 409);
}

#[test]
fn export_layer_places_the_overridden_onset_on_every_hit_sample() {
    // P4-40e (the Gate of #20): an onset override moves every hit by exactly the difference; the
    // measured onset is kept and Reset (onset null) renders the first render again. Planted fault:
    // a render that ignores the override leaves every voice on the measured onset and fails the
    // out2 asserts below.
    let sr = 48_000u32;
    let (svc, sid, root) = session("layer_override", vec![0.0; 2 * sr as usize], sr, &HITS);
    let f = root.join("shots").join("kick.wav");
    wav(&f, sr, &[shot(sr, 480, 0.2, 1)], None);
    let onset = svc
        .track_layer_set(&sid, "k in", &json!({"path": f.to_str().unwrap()}))
        .unwrap()["layer"]["onset"]
        .as_u64()
        .unwrap() as usize;
    let i0 = (0.25 * sr as f64).round() as usize; // the first hit, velocity 127: the level itself
    let db = |v: i64| 10f64.powf(-24.0 * (1.0 - v as f64 / 127.0) / 20.0) as f32;
    let left = shot(sr, 480, 0.2, 1);

    let render = || {
        let r = svc.export_layer(&sid, "k in").unwrap();
        read_back(Path::new(r["path"].as_str().unwrap())).1.remove(0)
    };
    let out1 = render();
    assert_eq!(out1[i0], db(127) * left[onset], "the measured onset on the hit");

    // the override: 961 samples later; the answer keeps the measured onset and names the override
    let r = svc
        .track_layer_set(&sid, "k in", &json!({"onset": onset as u64 + 961}))
        .unwrap();
    assert_eq!(
        (
            r["layer"]["onset"].as_u64().unwrap(),
            r["layer"]["onset_set"].as_u64().unwrap()
        ),
        (onset as u64, onset as u64 + 961)
    );
    assert!((r["onset_ms"].as_f64().unwrap() - 1000.0 * (onset + 961) as f64 / sr as f64).abs() < 0.01);
    let d = 961usize;
    let out2 = render();
    for (t, v) in HITS {
        let i = (t * sr as f64).round() as usize;
        // every hit's voice sits the difference later
        for k in [0usize, 1, 2, 100, 5_000] {
            assert_eq!(
                out2[i - onset - d + k],
                db(v) * left[k],
                "hit {t} sample {k} at the override"
            );
        }
        // the old content moved: the position that held left[k] now holds left[k + d] (the hits are
        // 0.5 s apart, the one-shot 0.2 s: no overlap)
        assert_eq!(
            out2[i - onset + 2],
            db(v) * left[d + 2],
            "hit {t}: the voice moved by the difference"
        );
    }

    // Reset: the measured onset again, the first render again
    let r = svc.track_layer_set(&sid, "k in", &json!({"onset": null})).unwrap();
    assert_eq!(r["layer"]["onset_set"], Value::Null);
    assert_eq!(r["layer"]["onset"].as_u64().unwrap(), onset as u64);
    let out3 = render();
    for (t, v) in HITS {
        let i = (t * sr as f64).round() as usize;
        assert_eq!(
            out3[i - onset + 2],
            db(v) * left[2],
            "hit {t}: Reset renders the measured onset"
        );
    }
    // a new one-shot clears the override (the measured onset of the new file is used)
    let f2 = root.join("shots").join("low.wav");
    wav(&f2, 44_100, &[shot(44_100, 441, 0.2, 5)], None);
    let r = svc
        .track_layer_set(&sid, "k in", &json!({"path": f2.to_str().unwrap(), "onset": 1000}))
        .unwrap();
    assert_eq!(r["layer"]["onset_set"].as_u64().unwrap(), 1000);
    let r = svc
        .track_layer_set(&sid, "k in", &json!({"path": f.to_str().unwrap()}))
        .unwrap();
    assert_eq!(r["layer"]["onset_set"], Value::Null, "a new path clears the override");
    assert_eq!(r["layer"]["onset"].as_u64().unwrap(), onset as u64);
    // an onset at or after the end of the file is a 400
    assert_eq!(
        svc.track_layer_set(&sid, "k in", &json!({"onset": 9_600}))
            .unwrap_err()
            .code,
        400
    );
}

/// The mic of the alignment test: the one-shot at each hit plus a planted lag, with `sign`.
fn planted_mic(sr: u32, n: usize, x: &[f32], onset: usize, hits: &[(f64, f64)], sign: f32) -> Vec<f32> {
    let s = OneShot {
        sr,
        chans: vec![x.to_vec()],
    };
    let pos: Vec<(f64, f32)> = hits
        .iter()
        .map(|(t, lag)| ((t * sr as f64).round() + lag, sign * 0.8))
        .collect();
    let mut mic = render_layer(n, &s, onset as f64, &pos).remove(0);
    for (i, m) in mic.iter_mut().enumerate() {
        *m += 0.003 * ((i as f64 * 0.21).sin() as f32);
    }
    mic
}

/// An aperiodic one-shot: `pre` samples of silence, a 0.3 ms rise and a 2.5 ms exponential decay.
fn pulse(sr: u32, pre: usize, secs: f64) -> Vec<f32> {
    (0..(secs * sr as f64) as usize)
        .map(|i| {
            if i < pre {
                return 0.0;
            }
            let t = (i - pre) as f64 / sr as f64;
            ((t / 0.0003).min(1.0) * (-t / 0.0025).exp()) as f32
        })
        .collect()
}

#[test]
fn track_layer_align_searches_the_signed_correlation_and_flags_a_low_match_uncertain() {
    // the mic holds an aperiodic one-shot upside down: with polarity normal the search maximises the signed
    // correlation (an absolute value would find the inverted match at the planted lag: the planted fault), so the
    // match is low and the hit is flagged uncertain (a low match is applied, only an edge lag is not); with polarity invert
    // the same hit matches well
    let sr = 48_000u32;
    let x = pulse(sr, 480, 0.25);
    let onset = drumspike_core::layer::one_shot_onset(&x, sr);
    let planted = [(0.25, 6.0), (0.75, 0.0)];
    let mic = planted_mic(sr, 2 * sr as usize, &x, onset, &planted, -1.0);
    let hits: Vec<(f64, i64)> = planted.iter().map(|(t, _)| (*t, 127)).collect();
    let (svc, sid, root) = session("layer_signed", mic, sr, &hits);
    let f = root.join("shots").join("k.wav");
    wav(&f, sr, std::slice::from_ref(&x), None);
    let p = f.to_str().unwrap();
    svc.track_layer_set(&sid, "k in", &json!({"path": p, "polarity": "normal", "align": true}))
        .unwrap();
    let a = svc.track_layer_align(&sid, "k in").unwrap();
    for r in a["hits"].as_array().unwrap() {
        assert!(r["match"].as_f64().unwrap() < 0.5, "signed search, normal: {r}");
        // flagged uncertain by the low match; only an edge lag is not applied (`applied` is `!edge`)
        assert_eq!(r["uncertain"], true, "{r}");
        assert_eq!(r["applied"].as_bool().unwrap(), !r["edge"].as_bool().unwrap(), "{r}");
    }
    // inverted polarity: the signed correlation is high, the lag is applied
    svc.track_layer_set(&sid, "k in", &json!({"polarity": "invert", "align": true}))
        .unwrap();
    let a = svc.track_layer_align(&sid, "k in").unwrap();
    let rows = a["hits"].as_array().unwrap();
    assert!(
        rows.iter()
            .all(|r| r["match"].as_f64().unwrap() > 0.9 && r["applied"] == true),
        "{a}"
    );
    assert!((rows[0]["lag"].as_f64().unwrap() - 6.0).abs() < 0.5, "{a}");
}

#[test]
fn track_layer_align_recovers_planted_lags_and_the_polarity() {
    let sr = 48_000u32;
    let x = shot(sr, 480, 0.25, 9);
    let onset = drumspike_core::layer::one_shot_onset(&x, sr);
    let planted = [(0.25, 5.0), (0.75, -7.5), (1.25, 12.0), (1.75, 0.0), (2.25, -30.25)];
    let mic = planted_mic(sr, 3 * sr as usize, &x, onset, &planted, -1.0);
    let hits: Vec<(f64, i64)> = planted.iter().map(|(t, _)| (*t, 127)).collect();
    let (svc, sid, root) = session("layer_align", mic.clone(), sr, &hits);
    let f = root.join("shots").join("k.wav");
    wav(&f, sr, std::slice::from_ref(&x), None);
    assert_eq!(svc.track_layer_align(&sid, "k in").unwrap_err().code, 409, "no layer");
    svc.track_layer_set(&sid, "k in", &json!({"path": f.to_str().unwrap(), "polarity": "auto"}))
        .unwrap();
    let a = svc.track_layer_align(&sid, "k in").unwrap();
    assert_eq!(
        (
            a["polarity"].clone(),
            a["sign"].clone(),
            a["inverted"].clone(),
            a["flips"].clone()
        ),
        (json!("inverted"), json!(-1), json!(5), json!([]))
    );
    assert_eq!(a["n"], 5);
    assert_eq!(
        a["max_lag_ms"], 5.0,
        "the answer reports the kick's window, as the render uses it"
    );
    for (row, (_, lag)) in a["hits"].as_array().unwrap().iter().zip(planted) {
        let got = row["lag"].as_f64().unwrap();
        assert!((got - lag).abs() < 0.2, "planted {lag}, found {got}");
        assert!(row["match"].as_f64().unwrap() > 0.97, "{row}");
        assert_eq!(row["sign"], -1);
        assert_eq!(row["uncertain"], false);
    }
    // normal polarity: the lags are those of the as-is sign, the match is low
    svc.track_layer_set(&sid, "k in", &json!({"polarity": "normal"}))
        .unwrap();
    let n = svc.track_layer_align(&sid, "k in").unwrap();
    assert_eq!(n["sign"], 1);
    assert!(n["median_match"].as_f64().unwrap() < a["median_match"].as_f64().unwrap());
    // the aligned render (auto polarity) sums with the mic to nearly nothing: it is the mic inverted
    // back, sub-sample aligned
    svc.track_layer_set(
        &sid,
        "k in",
        &json!({"polarity": "auto", "align": true, "curve": {"kind": "fixed"}, "level_db": 20.0*0.8f64.log10()}),
    )
    .unwrap();
    let r = svc.export_layer(&sid, "k in").unwrap();
    assert_eq!((r["aligned"].clone(), r["sign"].clone()), (json!(true), json!(-1)));
    let (_, out, _) = read_back(Path::new(r["path"].as_str().unwrap()));
    for (t, _) in planted {
        let i = (t * sr as f64).round() as usize;
        let (mut e, mut p) = (0.0f64, 0.0f64);
        for k in i - 100..i + 2_000 {
            e += ((out[0][k] - mic[k]) as f64).powi(2);
            p += (mic[k] as f64).powi(2);
        }
        assert!(e / p < 1e-3, "hit {t}: residual {}", e / p);
    }
    // planted fault: without the alignment the residual is large at the lagged hits
    svc.track_layer_set(&sid, "k in", &json!({"align": false})).unwrap();
    let r = svc.export_layer(&sid, "k in").unwrap();
    let (_, out, _) = read_back(Path::new(r["path"].as_str().unwrap()));
    let i = (1.25 * sr as f64).round() as usize;
    let (mut e, mut p) = (0.0f64, 0.0f64);
    for k in i - 100..i + 2_000 {
        e += ((out[0][k] - mic[k]) as f64).powi(2);
        p += (mic[k] as f64).powi(2);
    }
    assert!(e / p > 0.05, "the 12-sample lag is not corrected: {}", e / p);
}

fn wait_for(what: &str, secs: f64, mut f: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs_f64(secs);
    while !f() {
        assert!(Instant::now() < end, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn track_layer_set_gives_the_playing_transport_its_voices_at_both_speeds() {
    let sr = 48_000u32;
    let (svc, sid, root) = session("layer_play", kick_signal(&ONSETS, 3.5, sr), sr, &HITS);
    let f = root.join("shots").join("k.wav");
    wav(&f, sr, &[shot(sr, 480, 0.2, 7)], None);
    let onset = svc
        .track_layer_set(&sid, "k in", &json!({"path": f.to_str().unwrap()}))
        .unwrap()["layer"]["onset"]
        .as_u64()
        .unwrap() as i64;
    svc.transport_play(&sid, Some(0.0), None, None).unwrap();
    let st = svc.transport_status();
    assert_eq!(st["layers"], json!([{"name": "k in", "voices": 4}]));
    wait_for("the four hits", 10.0, || {
        svc.transport_status()["pos"].as_f64().unwrap() > 1.9
    });
    let st = svc.transport_status();
    assert_eq!(st["layer_voices"], 4);
    // the last voice: the one-shot's first sample at the hit's frame minus the onset
    assert_eq!(st["layer_last_start"], json!((1.75 * sr as f64).round() as i64 - onset));
    // off while open: the transport drops the voices at once
    svc.track_layer_set(&sid, "k in", &json!({"on": false})).unwrap();
    assert_eq!(svc.transport_status()["layers"], json!([]));
    let _ = svc.transport_stop(false);
    // on again, half speed: the voices at twice the frame
    svc.track_layer_set(&sid, "k in", &json!({"on": true})).unwrap();
    svc.transport_play_at(&sid, Some(1.6), None, None, Some(0.5)).unwrap();
    wait_for("the last hit at half speed", 10.0, || {
        svc.transport_status()["pos"].as_f64().unwrap() > 1.8
    });
    let st = svc.transport_status();
    assert_eq!(st["layer_voices"], 5);
    assert_eq!(
        st["layer_last_start"],
        json!(2 * ((1.75 * sr as f64).round() as i64 - onset))
    );
    let _ = svc.transport_stop(true);
}

/// A session with the layer file `shots/one.wav` set on `k in`; returns (service, sid, root, the file).
fn layered_session(tag: &str) -> (Service, String, PathBuf, PathBuf) {
    let sr = 48_000u32;
    let (svc, sid, root) = session(tag, kick_signal(&ONSETS, 3.5, sr), sr, &HITS);
    let f = root.join("shots").join("one.wav");
    wav(&f, sr, &[shot(sr, 480, 0.2, 5)], None);
    svc.track_layer_set(&sid, "k in", &json!({"path": f.to_str().unwrap()}))
        .unwrap();
    (svc, sid, root, f)
}

fn wait_copy_job(svc: &Service, sid: &str) {
    wait_for("the copy job", 60.0, || svc.job_status(sid)["state"] != "running");
    assert_eq!(svc.job_status(sid)["state"], "done", "{}", svc.job_status(sid));
}

#[test]
fn project_save_as_copy_takes_the_layer_file_into_the_project_and_rewrites_its_record() {
    // P4-7a fix 6
    let (svc, sid, root, f) = layered_session("layer_saveas");
    let out = root.join("out");
    svc.project_save_as(&sid, Some("Copy"), Some(out.to_str().unwrap()), true, None)
        .unwrap();
    wait_copy_job(&svc, &sid);
    let audio = out.join("Copy").join("Audio Files");
    let r = svc.track_layer_get(&sid, "k in").unwrap();
    assert_eq!(r["status"], "ok");
    assert_eq!(r["path"], json!(audio.join("one.wav").display().to_string()), "{r}");
    assert!(audio.join("one.wav").is_file(), "the layer file is in the project");
    // the original goes away (planted fault: without the copy the layer is missing now)
    std::fs::remove_file(&f).unwrap();
    assert_eq!(svc.track_layer_get(&sid, "k in").unwrap()["status"], "ok");
    svc.project_save(&sid).unwrap();
    svc.export_layer(&sid, "k in").unwrap();
    // and the saved document has the new record
    svc.project_close(&sid, true).unwrap();
    let sid2 = svc.project_open(out.join("Copy").to_str().unwrap()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(svc.track_layer_get(&sid2, "k in").unwrap()["status"], "ok");
}

#[test]
fn project_consolidate_takes_the_layer_file_into_the_project_and_rewrites_its_record() {
    // P4-7a fix 6
    let (svc, sid, root, f) = layered_session("layer_consol");
    let dir = PathBuf::from(svc.project_get(&sid).unwrap()["folder"].as_str().unwrap());
    svc.project_consolidate(&sid, None).unwrap();
    wait_copy_job(&svc, &sid);
    let r = svc.track_layer_get(&sid, "k in").unwrap();
    assert_eq!(
        r["path"],
        json!(dir.join("Audio Files").join("one.wav").display().to_string()),
        "{r}"
    );
    std::fs::remove_dir_all(root.join("shots")).unwrap();
    assert!(!f.exists());
    assert_eq!(svc.track_layer_get(&sid, "k in").unwrap()["status"], "ok", "{r}");
}

#[test]
fn relink_search_finds_a_moved_layer_file_and_the_track_list_flags_a_missing_one() {
    // P4-7a fix 6
    let (svc, sid, root, f) = layered_session("layer_relink");
    let moved = root.join("elsewhere").join("deep");
    std::fs::create_dir_all(&moved).unwrap();
    std::fs::rename(&f, moved.join("one.wav")).unwrap();
    let t = svc.tracks_get(&sid).unwrap();
    assert_eq!(
        t["tracks"]["k in"]["layer_status"], "missing",
        "the bar can list the layer file"
    );
    let r = svc
        .relink_search(&sid, root.join("elsewhere").to_str().unwrap())
        .unwrap();
    assert_eq!(
        (r["offline"].clone(), r["relinked"].clone()),
        (json!(1), json!(1)),
        "{r}"
    );
    assert_eq!(r["found"][0]["track"], "k in");
    assert_eq!(r["found"][0]["layer"], true);
    assert_eq!(svc.track_layer_get(&sid, "k in").unwrap()["status"], "ok");
    assert_eq!(svc.tracks_get(&sid).unwrap()["tracks"]["k in"]["layer_status"], "ok");
    // the settings of the layer stay
    assert_eq!(svc.track_layer_get(&sid, "k in").unwrap()["layer"]["on"], true);
    // a search with nothing offline looks at nothing
    assert_eq!(svc.relink_search(&sid, root.to_str().unwrap()).unwrap()["offline"], 0);
}

#[test]
fn export_layer_is_sub_sample_exact_for_a_one_shot_at_another_rate() {
    // P4-7a fix 7: the one-shot (44.1 kHz) resampled to the track's 48 kHz has its onset at a fractional
    // sample; the render uses it, not its rounding
    let sr = 48_000u32;
    let (svc, sid, root) = session(
        "layer_rate",
        vec![0.0; 3 * sr as usize],
        sr,
        &[(0.25, 127), (0.75, 127)],
    );
    let x = shot(44_100, 333, 0.2, 11);
    let f = root.join("shots").join("cd.wav");
    wav(&f, 44_100, std::slice::from_ref(&x), None);
    let r = svc
        .track_layer_set(
            &sid,
            "k in",
            &json!({"path": f.to_str().unwrap(), "curve": {"kind": "fixed"}}),
        )
        .unwrap();
    let onset = r["layer"]["onset"].as_u64().unwrap() as f64;
    let exact = onset * f64::from(sr) / 44_100.0;
    assert!(
        exact.fract().abs() > 0.05 && exact.fract() < 0.95,
        "the test needs a fractional onset: {exact}"
    );
    let out = svc.export_layer(&sid, "k in").unwrap();
    let (_, got, _) = read_back(Path::new(out["path"].as_str().unwrap()));
    let rs = OneShot {
        sr: 44_100,
        chans: vec![x],
    }
    .resampled(sr);
    let want = render_layer(3 * sr as usize, &rs, exact, &[(12_000.0, 1.0), (36_000.0, 1.0)]);
    assert!(
        got[0] == want[0],
        "the render is the resampled one-shot at the exact onset"
    );
}

#[test]
fn track_layer_align_marks_an_edge_hit_uncertain_and_reports_no_lag_for_it() {
    // the true lag (400 samples) is outside the kick's 5 ms window: the best lag in it sits on the edge;
    // the render does not apply it, so the answer says edge, uncertain, lag 0
    let sr = 48_000u32;
    // an aperiodic attack (a decaying pulse): its correlation falls away from the true lag, so a lag outside
    // the window gives a best lag on the window's edge (a periodic body gives an interior peak)
    let x = pulse(sr, 480, 0.25);
    let onset = drumspike_core::layer::one_shot_onset(&x, sr);
    let planted = [(0.25, 400.0), (0.75, 0.0)];
    let mic = planted_mic(sr, 2 * sr as usize, &x, onset, &planted, 1.0);
    let hits: Vec<(f64, i64)> = planted.iter().map(|(t, _)| (*t, 127)).collect();
    let (svc, sid, root) = session("layer_edge", mic, sr, &hits);
    let f = root.join("shots").join("k.wav");
    wav(&f, sr, std::slice::from_ref(&x), None);
    svc.track_layer_set(
        &sid,
        "k in",
        &json!({"path": f.to_str().unwrap(), "polarity": "normal", "align": true}),
    )
    .unwrap();
    let a = svc.track_layer_align(&sid, "k in").unwrap();
    let rows = a["hits"].as_array().unwrap();
    assert_eq!(rows[0]["edge"], true, "{a}");
    assert_eq!(
        (rows[0]["lag"].clone(), rows[0]["uncertain"].clone()),
        (json!(0.0), json!(true))
    );
    assert_eq!(
        (rows[1]["edge"].clone(), rows[1]["uncertain"].clone()),
        (json!(false), json!(false))
    );
    assert_eq!(a["uncertain"], 1);
}

/// Campaign 0.9.6 S3 (#117): a failed layer render left a folder named like the target file
/// (`Exports/<track>_layer.wav/` plus a `<track>_layer.wav.drumspike` sidecar), and every later
/// render failed with 400 "the path is a folder" until the folder was removed by hand. The next
/// render now removes the leftover and succeeds, on the default target...
#[test]
fn export_layer_clears_a_leftover_folder_at_the_default_target_and_renders() {
    let (svc, sid, root) = session("layer_leftover_default", vec![0.0; 96_000], 48_000, &HITS);
    let f = root.join("shots").join("kick one.wav");
    wav(&f, 48_000, &[shot(48_000, 480, 0.3, 1)], None);
    svc.track_layer_set(&sid, "k in", &json!({"path": f.to_str().unwrap()}))
        .unwrap();
    let dir = PathBuf::from(svc.project_get(&sid).unwrap()["folder"].as_str().unwrap());
    let exports = dir.join("Exports");
    let folder = exports.join("k in_layer.wav");
    // plant the leftover the failure produces: the folder plus its sidecar
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(exports.join("k in_layer.wav.drumspike"), b"{}").unwrap();
    let r = svc.export_layer(&sid, "k in").unwrap();
    assert_eq!(r["path"], json!(folder.display().to_string()));
    assert!(folder.is_file(), "the render wrote the file over the leftover");
    assert!(
        !exports.join("k in_layer.wav.drumspike").exists(),
        "the sidecar of the leftover is gone too"
    );
}

/// ... and on an explicit `path` (the save dialog's file).
#[test]
fn export_layer_clears_a_leftover_folder_at_the_chosen_path_and_renders() {
    let (svc, sid, root) = session("layer_leftover_picked", vec![0.0; 96_000], 48_000, &HITS);
    let f = root.join("shots").join("kick one.wav");
    wav(&f, 48_000, &[shot(48_000, 480, 0.3, 1)], None);
    svc.track_layer_set(&sid, "k in", &json!({"path": f.to_str().unwrap()}))
        .unwrap();
    let want = root.join("picked").join("k in layer.wav");
    std::fs::create_dir_all(&want).unwrap();
    std::fs::write(root.join("picked").join("k in layer.wav.drumspike"), b"{}").unwrap();
    svc.export_layer_to(&sid, "k in", want.to_str()).unwrap();
    assert!(want.is_file());
    assert!(!root.join("picked").join("k in layer.wav.drumspike").exists());
}

/// A folder with real content at the target belongs to the engineer: it is not deleted, and the
/// 400 names the path and what to do. An empty folder without the sidecar is not provably a
/// render leftover and is left alone as well.
#[test]
fn export_layer_refuses_a_foreign_folder_at_the_target_without_touching_it() {
    let (svc, sid, root) = session("layer_foreign_folder", vec![0.0; 96_000], 48_000, &HITS);
    let f = root.join("shots").join("kick one.wav");
    wav(&f, 48_000, &[shot(48_000, 480, 0.3, 1)], None);
    svc.track_layer_set(&sid, "k in", &json!({"path": f.to_str().unwrap()}))
        .unwrap();
    let dir = PathBuf::from(svc.project_get(&sid).unwrap()["folder"].as_str().unwrap());
    let exports = dir.join("Exports");
    // a folder with content at the default target
    let folder = exports.join("k in_layer.wav");
    std::fs::create_dir_all(folder.join("notes")).unwrap();
    std::fs::write(folder.join("notes").join("readme.txt"), b"mine").unwrap();
    let e = svc.export_layer(&sid, "k in").unwrap_err();
    assert_eq!(e.code, 400);
    assert!(e.message.contains("the path is a folder"), "{e}");
    assert!(e.message.contains("remove the folder"), "{e}");
    assert_eq!(
        std::fs::read(folder.join("notes").join("readme.txt")).unwrap(),
        b"mine",
        "the foreign folder is untouched"
    );
    // an empty folder without the sidecar at an explicit target: refused, not deleted
    let want = root.join("picked").join("k in layer.wav");
    std::fs::create_dir_all(&want).unwrap();
    let e = svc.export_layer_to(&sid, "k in", want.to_str()).unwrap_err();
    assert_eq!(e.code, 400);
    assert!(want.is_dir(), "the empty folder is left alone");
    // a folder holding a foreign project document at the default target: refused, not deleted
    let docs = exports.join("k in_layer.wav");
    std::fs::create_dir_all(&docs).unwrap();
    std::fs::write(docs.join("Song.drumspike"), b"{\"format\": 1}").unwrap();
    let e = svc.export_layer(&sid, "k in").unwrap_err();
    assert_eq!(e.code, 400);
    assert!(e.message.contains("the path is a folder"), "{e}");
    assert_eq!(
        std::fs::read(docs.join("Song.drumspike")).unwrap(),
        b"{\"format\": 1}",
        "the folder with the foreign document is untouched"
    );
    assert!(docs.is_dir());
}
