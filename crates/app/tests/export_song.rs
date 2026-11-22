//! #560 (M1c1): the song time base of an export, and the proof that the standalone export did not change.

mod common;

use common::*;
use drumspike_app::export::{ExportOpts, MidiMode};
use drumspike_app::export_song::{ExportTime, SongRegion, SongTrack};
use drumspike_core::tempo::{TempoMap, TempoStep};
use serde_json::{Value, json};
use std::sync::Arc;

fn opened(tag: &str) -> (drumspike_app::Service, String, std::path::PathBuf, std::path::PathBuf) {
    let root = tmp(tag);
    let dir = project(
        &root,
        "Song",
        &[kick_spec("Kick_In", "kick", 36), kick_spec("Snare_Top", "snare", 38)],
    );
    let svc = service_dsp();
    let sid = svc.project_open(dir.to_str().unwrap()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    svc.events_set(
        &sid,
        "Kick_In",
        &json!([{"t": 0.5, "v": 64}, {"t": 1.0, "v": 90}, {"t": 2.0, "v": 70}]),
    )
    .unwrap();
    svc.events_set(&sid, "Snare_Top", &json!([{"t": 0.75, "v": 80}, {"t": 1.5, "v": 100}]))
        .unwrap();
    (svc, sid, dir, root)
}

fn sha_of_dir(dir: &std::path::Path) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap())
        .filter(|e| e.path().is_file())
        .map(|e| {
            (
                e.file_name().to_string_lossy().into_owned(),
                file_sha256(e.path()).unwrap(),
            )
        })
        .collect();
    v.sort();
    v
}

/// The bytes of the standalone export (no `ExportTime` object): per track files and the one multi-track
/// file, on the default tempo map and on a map with two tempos. The hashes were made on the base commit
/// (main 70f6646) before any change of #560.
#[test]
fn the_standalone_export_writes_the_same_bytes_as_before_the_song_time_base() {
    let (svc, sid, dir, root) = opened("song_golden");
    let mut got = Vec::new();
    for (label, map) in [
        ("default", None),
        ("map", Some(json!({"bpm": 100.0, "steps": [{"t": 1.0, "bpm": 120.0}]}))),
    ] {
        if let Some(m) = &map {
            svc.tempo_map_set(&sid, m).unwrap();
        }
        for mode in ["per_track", "single"] {
            let out = root.join(format!("out_{label}_{mode}"));
            std::fs::create_dir(&out).unwrap();
            let o = ExportOpts::from_args(&json!({"dest": out.to_str().unwrap(), "midi_mode": mode})).unwrap();
            let r = svc.export_all_opts(&sid, &o, None).unwrap();
            assert!(r.get("outside_song").is_none(), "no song key in a standalone answer");
            for (n, h) in sha_of_dir(&out) {
                got.push(format!("{label}/{mode}/{n} {h}"));
            }
        }
    }
    let _ = dir;
    assert_eq!(got.join("\n"), GOLDEN.trim());
    std::fs::remove_dir_all(&root).unwrap();
}

const GOLDEN: &str = "
default/per_track/Kick_In_spike.mid ae41ae9695723f2f928b6fc56a9a69fdd9aeaa7fd1eb55347cacb3357199c3a7
default/per_track/Kick_In_spike.wav 72f897e475d92e58920691c61ffba438edb4eb55ff43784ab777acc446e60663
default/per_track/Snare_Top_spike.mid 5583728d38ebb00cdfb481b60958bbbd24cce3499e1b5680c1a2412b63a2a9b7
default/per_track/Snare_Top_spike.wav e86fb899bfc8b71c4503620727c44df75a07b1135f98af6e12c11d5a8a7ef25c
default/single/Kick_In_spike.wav 72f897e475d92e58920691c61ffba438edb4eb55ff43784ab777acc446e60663
default/single/Snare_Top_spike.wav e86fb899bfc8b71c4503620727c44df75a07b1135f98af6e12c11d5a8a7ef25c
default/single/Song_drums.mid 0bf2d43b708060fcb06792f91b7696eb166bd90ec59b6a585bf1fcbc037f6c76
map/per_track/Kick_In_spike.mid 03deaf0487680994ee1c97dc15bec0e5109b06908cf880a253fb62410f12e55f
map/per_track/Kick_In_spike.wav 72f897e475d92e58920691c61ffba438edb4eb55ff43784ab777acc446e60663
map/per_track/Snare_Top_spike.mid eb28ee01aef55bcbdaa5aa52eed44df363e34ad039efe2584f6f4f4a02b05758
map/per_track/Snare_Top_spike.wav e86fb899bfc8b71c4503620727c44df75a07b1135f98af6e12c11d5a8a7ef25c
map/single/Kick_In_spike.wav 72f897e475d92e58920691c61ffba438edb4eb55ff43784ab777acc446e60663
map/single/Snare_Top_spike.wav e86fb899bfc8b71c4503620727c44df75a07b1135f98af6e12c11d5a8a7ef25c
map/single/Song_drums.mid 7c58b449f1a2f11e7033044a2950d03ff74502e16dd7977303c586a532a892e6
";

struct Fixed {
    tracks: Vec<(String, SongTrack)>,
    map: Option<TempoMap>,
}
impl ExportTime for Fixed {
    fn track(&self, name: &str) -> Option<SongTrack> {
        self.tracks.iter().find(|(n, _)| n == name).map(|(_, t)| t.clone())
    }
    fn tempo(&self) -> Option<TempoMap> {
        self.map.clone()
    }
}

fn read_notes(path: &std::path::Path) -> Vec<f64> {
    use midly::{MetaMessage, MidiMessage, Smf, Timing, TrackEventKind};
    let bytes = std::fs::read(path).unwrap();
    let smf = Smf::parse(&bytes).unwrap();
    let Timing::Metrical(ppq) = smf.header.timing else {
        panic!("timing")
    };
    let ppq = f64::from(ppq.as_int());
    // tempo events from track 0, in ticks
    let mut tempi: Vec<(u64, f64)> = vec![(0, 500_000.0)];
    let mut notes = Vec::new();
    for track in &smf.tracks {
        let mut tick = 0u64;
        for e in track {
            tick += u64::from(e.delta.as_int());
            match e.kind {
                TrackEventKind::Meta(MetaMessage::Tempo(t)) => tempi.push((tick, f64::from(t.as_int()))),
                TrackEventKind::Midi {
                    message: MidiMessage::NoteOn { vel, .. },
                    ..
                } if vel.as_int() > 0 => {
                    notes.push(tick);
                }
                _ => {}
            }
        }
    }
    tempi.dedup_by_key(|t| t.0);
    notes
        .into_iter()
        .map(|nt| {
            let (mut s, mut t0, mut us) = (0.0, 0u64, tempi[0].1);
            for &(tk, u) in tempi.iter().skip(1) {
                if tk >= nt {
                    break;
                }
                s += (tk - t0) as f64 * us / (ppq * 1e6);
                t0 = tk;
                us = u;
            }
            s + (nt - t0) as f64 * us / (ppq * 1e6)
        })
        .collect()
}

#[test]
fn a_song_time_export_puts_the_hits_at_their_song_positions_in_the_midi_and_the_spike_files() {
    let (svc, sid, _dir, root) = opened("song_files");
    // kick: one region [0.25, 1.25) at song 10 s, a second copy [1.75, 2.5) at song 20 s: the hits 0.5 and 1.0 sit in
    // the first, 2.0 in the second. The 120/100 BPM map has whole-microsecond quarters.
    svc.set_export_time(Some(Arc::new(Fixed {
        tracks: vec![
            (
                "Kick_In".into(),
                SongTrack {
                    regions: vec![
                        SongRegion {
                            start_mod: 0.25,
                            dur_mod: 1.0,
                            start_play: 10.0,
                            dur_play: 1.0,
                        },
                        SongRegion {
                            start_mod: 1.75,
                            dur_mod: 0.75,
                            start_play: 20.0,
                            dur_play: 0.75,
                        },
                    ],
                    stem: Some("Kick Drum".into()),
                    hide_all: false,
                },
            ),
            (
                "Snare_Top".into(),
                SongTrack {
                    regions: vec![],
                    stem: Some("Snare".into()),
                    hide_all: false,
                },
            ),
        ],
        map: Some(TempoMap {
            bpm: 120.0,
            steps: vec![TempoStep { t: 15.0, bpm: 100.0 }],
            ..TempoMap::default()
        }),
    })));
    let out = root.join("out");
    std::fs::create_dir(&out).unwrap();
    let o = ExportOpts::from_args(&json!({"dest": out.to_str().unwrap()})).unwrap();
    let r = svc.export_all_opts(&sid, &o, None).unwrap();
    assert!(r["errors"].as_array().unwrap().is_empty(), "{r}");
    let names: Vec<String> = sha_of_dir(&out).into_iter().map(|x| x.0).collect();
    assert_eq!(
        names,
        [
            "Kick Drum_spike.mid",
            "Kick Drum_spike.wav",
            "Snare_spike.mid",
            "Snare_spike.wav"
        ]
    );
    assert_eq!(r["outside_song"], json!({}), "every kick hit is in a region");

    // MIDI: each note within 1 sample (48 kHz) of its song position
    let kick = read_notes(&out.join("Kick Drum_spike.mid"));
    let want = [10.25, 10.75, 20.25];
    assert_eq!(kick.len(), 3);
    for (g, w) in kick.iter().zip(want) {
        assert!((g - w).abs() * 48_000.0 <= 1.0, "note at {g}, hit at {w}");
    }
    // the snare has no region: identity
    let snare = read_notes(&out.join("Snare_spike.mid"));
    for (g, w) in snare.iter().zip([0.75, 1.5]) {
        assert!((g - w).abs() * 48_000.0 <= 1.0, "note at {g}, hit at {w}");
    }
    // spike file: 0 samples of difference, length to the end of the last region, time reference 0
    let w = drumspike_core::io::MappedWav::open(&out.join("Kick Drum_spike.wav")).unwrap();
    assert_eq!(w.info.n_frames, 996_000, "song end 20.75 s");
    assert_eq!(w.info.bext.as_ref().map(|b| b.time_reference), Some(0));
    let x = w.decode_mono(drumspike_core::io::MixMode::Ffmpeg).unwrap();
    let mut peaks = Vec::new();
    for t in [10.25_f64, 10.75, 20.25] {
        let i = (t * 48_000.0).round() as usize;
        assert!(x[i].abs() > 0.1, "a spike at sample {i}");
        peaks.push(i);
    }
    let stray = x
        .iter()
        .enumerate()
        .filter(|(i, v)| v.abs() > 0.1 && !peaks.iter().any(|p| i.abs_diff(*p) < 200))
        .count();
    assert_eq!(stray, 0, "only the three spikes are loud");
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_song_time_export_counts_the_hits_it_leaves_out_and_names_the_files_after_the_host() {
    let (svc, sid, _dir, root) = opened("song_out");
    svc.set_export_time(Some(Arc::new(Fixed {
        tracks: vec![(
            "Kick_In".into(),
            SongTrack {
                // holds 0.5 only; 1.0 and 2.0 are in no region
                regions: vec![SongRegion {
                    start_mod: 0.0,
                    dur_mod: 0.75,
                    start_play: 5.0,
                    dur_play: 0.75,
                }],
                stem: None,
                hide_all: false,
            },
        )],
        map: None,
    })));
    let out = root.join("o2");
    std::fs::create_dir(&out).unwrap();
    let o = ExportOpts {
        dest: Some(out.to_str().unwrap().into()),
        tracks: Some(vec!["Kick_In".into()]),
        midi_mode: MidiMode::Single,
        ..ExportOpts::default()
    };
    let r = svc.export_all_opts(&sid, &o, None).unwrap();
    assert_eq!(
        r["outside_song"],
        json!({"Kick_In": {"no_region": 2, "before_start": 0}})
    );
    assert!(out.join("Song_drums.mid").is_file());
    let _: Value = r;
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_track_the_host_has_no_source_for_loses_every_hit_and_a_region_before_song_zero_makes_no_empty_spike_file() {
    let (svc, sid, _dir, root) = opened("song_edge");
    svc.set_export_time(Some(Arc::new(Fixed {
        tracks: vec![
            // no source of the host: every hit is left out and counted, the files keep the host's name
            (
                "Kick_In".into(),
                SongTrack {
                    regions: vec![],
                    stem: Some("Kick Host".into()),
                    hide_all: true,
                },
            ),
            // a region that ends before song 0: no spike file, and the answer says why
            (
                "Snare_Top".into(),
                SongTrack {
                    regions: vec![SongRegion {
                        start_mod: 0.0,
                        dur_mod: 0.5,
                        start_play: -1.0,
                        dur_play: 0.5,
                    }],
                    stem: Some("Snare Host".into()),
                    hide_all: false,
                },
            ),
        ],
        map: None,
    })));
    let out = root.join("o3");
    std::fs::create_dir(&out).unwrap();
    let o = ExportOpts::from_args(&json!({"dest": out.to_str().unwrap()})).unwrap();
    let r = svc.export_all_opts(&sid, &o, None).unwrap();
    assert_eq!(r["outside_song"]["Kick_In"], json!({"no_region": 3, "before_start": 0}));
    assert!(out.join("Kick Host_spike.mid").is_file() && out.join("Kick Host_spike.wav").is_file());
    assert!(!out.join("Snare Host_spike.wav").exists(), "no 0-frame file");
    let errs = r["errors"].as_array().unwrap();
    assert!(
        errs.iter()
            .any(|e| e["message"].as_str().unwrap_or("").contains("no spike file")),
        "{errs:?}"
    );
    std::fs::remove_dir_all(&root).unwrap();
}
