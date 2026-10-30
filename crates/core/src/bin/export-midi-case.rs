//! `export-midi-case`: write the MIDI file of one export case for `tools/check_midi_timing.py`
//! (ticket #434). The check exercises the real Rust export; it shells out to this binary so the
//! Python never re-implements the file format. Not product code.
//!
//! Usage: `export-midi-case <job.json>` with
//! `{"kind": "single"|"multi", "events": [{"t", "v"}...], "note": int, "ppq": int,
//!   "map": <TempoMap JSON or null for the default map>, "legacy_tempo": float,
//!   "markers": [{"t", "note"}...] (multi only), "tracks": [{"name", "note", "events"}] (multi only),
//!   "path": "..."}`
//! and it writes the file at `path`, printing nothing on success.

use drumspike_core::midi::{MidiEvent, MidiMarker, MidiTrack, export_midi_map, export_multi_midi};
use drumspike_core::tempo::TempoMap;
use std::io::Read;
use std::path::Path;

fn events(v: &serde_json::Value) -> Vec<MidiEvent> {
    v.as_array()
        .expect("events is a list")
        .iter()
        .map(|e| MidiEvent {
            t: e["t"].as_f64().expect("t"),
            v: e.get("v").and_then(|v| v.as_i64()).unwrap_or(100),
        })
        .collect()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 1 {
        eprintln!("usage: export-midi-case <job.json>");
        std::process::exit(2);
    }
    let mut s = String::new();
    std::fs::File::open(&args[0])
        .and_then(|mut f| f.read_to_string(&mut s))
        .expect("read the job");
    let job: serde_json::Value = serde_json::from_str(&s).expect("parse the job");
    let note = job["note"].as_i64().expect("note");
    let ppq = job["ppq"].as_u64().expect("ppq") as u16;
    let legacy_tempo = job["legacy_tempo"].as_f64().expect("legacy_tempo");
    let map = match &job["map"] {
        serde_json::Value::Null => TempoMap::default(),
        v => TempoMap::from_json(v).expect("map"),
    };
    let path = Path::new(job["path"].as_str().expect("path"));
    let r = if job["kind"] == "multi" {
        let tracks: Vec<MidiTrack> = job["tracks"]
            .as_array()
            .expect("tracks")
            .iter()
            .map(|t| MidiTrack {
                name: t["name"].as_str().expect("name"),
                note: t["note"].as_i64().expect("note"),
                events: Box::leak(events(&t["events"]).into_boxed_slice()),
            })
            .collect();
        let markers: Vec<MidiMarker> = job
            .get("markers")
            .and_then(|m| m.as_array())
            .map(|ms| {
                ms.iter()
                    .map(|m| MidiMarker {
                        t: m["t"].as_f64().expect("t"),
                        note: m.get("note").and_then(|n| n.as_str()),
                    })
                    .collect()
            })
            .unwrap_or_default();
        export_multi_midi(&tracks, &markers, path, ppq, &map, legacy_tempo)
    } else {
        export_midi_map(&events(&job["events"]), note, path, ppq, &map, legacy_tempo)
    };
    if let Err(e) = r {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
