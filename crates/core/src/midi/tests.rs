use super::*;

fn ev(t: f64, v: i64) -> MidiEvent {
    MidiEvent { t, v }
}

#[test]
fn ticks_follow_pretty_midi() {
    let ts = tick_scale(DEFAULT_PPQ, DEFAULT_TEMPO);
    assert_eq!(ts, 60.0 / (120.0 * 24000.0));
    assert_eq!(time_to_tick(0.5, ts), 24_000);
    assert_eq!(time_to_tick(0.300_000_000_000_000_04, ts), 14_400);
    assert_eq!(time_to_tick(2.675, ts), 128_400);
    assert_eq!(time_to_tick(0.0, ts), 0);
    assert_eq!(time_to_tick(-1.0, ts), 0);
    assert_eq!(time_to_tick(1e-9, ts), 0);
    // ties to even: 0.5 ticks -> 0, 1.5 ticks -> 2
    assert_eq!(time_to_tick(0.5 * ts, ts), 0);
    assert_eq!(time_to_tick(1.5 * ts, ts), 2);
    assert_eq!(tempo_meta(DEFAULT_PPQ, DEFAULT_TEMPO), 500_000);
    assert_eq!(tempo_meta(480, 90.0), 666_666);
}

#[test]
fn note_events_carry_tick_note_velocity_and_duration() {
    let n = note_events(
        &[ev(0.5, 90), ev(0.52, 100), ev(0.0, 1)],
        36,
        DEFAULT_PPQ,
        DEFAULT_TEMPO,
    )
    .unwrap();
    assert_eq!(
        n,
        vec![
            NoteEvent {
                tick: 24_000,
                note: 36,
                velocity: 90,
                duration: 2_400
            },
            NoteEvent {
                tick: 24_960,
                note: 36,
                velocity: 100,
                duration: 2_400
            },
            NoteEvent {
                tick: 0,
                note: 36,
                velocity: 1,
                duration: 2_400
            },
        ]
    );
    assert_eq!(
        note_events(&[ev(0.5, 128)], 36, 24000, 120.0),
        Err(MidiError::Velocity(128))
    );
    assert_eq!(
        note_events(&[ev(0.5, -1)], 36, 24000, 120.0),
        Err(MidiError::Velocity(-1))
    );
    assert_eq!(note_events(&[ev(0.5, 1)], 200, 24000, 120.0), Err(MidiError::Note(200)));
    // S2-1: a late hit is a valid file; only a time without a tick is refused
    assert_eq!(
        note_events(&[ev(1e5, 1)], 36, 24000, 120.0).unwrap()[0].tick,
        4_800_000_000
    );
    assert!(matches!(note_events(&[ev(f64::NAN, 1)], 36, 24000, 120.0), Err(MidiError::Tick(t)) if t.is_nan()));
    assert_eq!(
        note_events(&[ev(f64::INFINITY, 1)], 36, 24000, 120.0),
        Err(MidiError::Tick(f64::INFINITY))
    );
    assert_eq!(
        note_events(&[ev(1e30, 1)], 36, 24000, 120.0),
        Err(MidiError::Tick(1e30))
    );
    assert!(matches!(note_events(&[], 36, 0, 120.0), Err(MidiError::Timing { .. })));
    assert!(matches!(
        note_events(&[], 36, 24000, 0.0),
        Err(MidiError::Timing { .. })
    ));
}

/// The bytes `engine.export_midi` wrote for [{"t": 0.5, "v": 90}, {"t": 0.52}, {"t": 0.0, "v": 1}],
/// note 36 (`pretty_midi` 0.2.11, mido 1.3.3), captured while building this module.
const PYTHON_FILE: &str = "4d54686400000006000100025dc04d54726b0000001300ff510307a12000ff58040402180801ff2f00\
4d54726b0000002a00ff03067370696b657300c900009924019260240081a860245a874024648b2024008740240001ff2f00";

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

#[test]
fn bytes_match_the_python_export() {
    let b = midi_bytes(
        &[ev(0.5, 90), ev(0.52, 100), ev(0.0, 1)],
        36,
        DEFAULT_PPQ,
        DEFAULT_TEMPO,
    )
    .unwrap();
    assert_eq!(b, unhex(PYTHON_FILE));
}

/// `engine.export_midi` for [{"t": 0.5, "v": 90}, {"t": 123456.123456789, "v": 50}, {"t": 0.52}],
/// note 36: the gap to the late hit is a 5-byte quantity (mido 1.3.3), captured for S2-1.
const PYTHON_LONG_GAP: &str = "4d54686400000006000100025dc04d54726b0000001300ff510307a12000ff58040402180801ff2f00\
4d54726b0000002e00ff03067370696b657300c90081bb4099245a874024648b202400874024009689d6984624329260240001ff2f00";

#[test]
fn a_gap_beyond_28_bits_is_written_like_mido() {
    let ev3 = [ev(0.5, 90), ev(123_456.123_456_789, 50), ev(0.52, 100)];
    let b = midi_bytes(&ev3, 36, DEFAULT_PPQ, DEFAULT_TEMPO).unwrap();
    assert_eq!(b, unhex(PYTHON_LONG_GAP));
    let n = note_events(&ev3, 36, DEFAULT_PPQ, DEFAULT_TEMPO).unwrap();
    assert_eq!(n[1].tick, 5_925_893_926); // round(123456.123456789 * 48000)
}

#[test]
fn both_writers_agree_where_midly_can_write() {
    let cases: [&[MidiEvent]; 4] = [
        &[],
        &[ev(0.5, 90), ev(0.52, 100), ev(0.0, 1)],
        &[ev(1.0, 100), ev(1.05, 100), ev(1.05, 7), ev(3000.0, 127)],
        &[ev(5000.0, 64), ev(0.001, 2)],
    ];
    for (ppq, tempo) in [(DEFAULT_PPQ, DEFAULT_TEMPO), (480, 90.0), (960, 133.7)] {
        for evs in cases {
            let notes = note_events(evs, 38, ppq, tempo).unwrap();
            let msgs = track1_messages(&notes);
            let t0 = default_track0(tempo_meta(ppq, tempo));
            let a = midly_bytes(&msgs, ppq, &t0).unwrap();
            assert_eq!(a, write_smf(&msgs, ppq, &t0), "ppq {ppq} tempo {tempo}");
            assert_eq!(a, midi_bytes(evs, 38, ppq, tempo).unwrap());
        }
    }
}

#[test]
fn vlq_follows_mido() {
    let enc = |v: u64| {
        let mut o = Vec::new();
        push_vlq(&mut o, v);
        o
    };
    assert_eq!(enc(0), vec![0x00]);
    assert_eq!(enc(0x7F), vec![0x7F]);
    assert_eq!(enc(0x80), vec![0x81, 0x00]);
    assert_eq!(enc(0x0FFF_FFFF), vec![0xFF, 0xFF, 0xFF, 0x7F]);
    assert_eq!(enc(0x1000_0000), vec![0x81, 0x80, 0x80, 0x80, 0x00]);
}

#[test]
fn note_off_sorts_before_note_on_at_the_same_tick() {
    // 50 ms apart: the first note's off and the second note's on share a tick
    let b = midi_bytes(&[ev(1.0, 100), ev(1.05, 100)], 36, DEFAULT_PPQ, DEFAULT_TEMPO).unwrap();
    let smf = Smf::parse(&b).unwrap();
    let kinds: Vec<(u32, u8)> = smf.tracks[1]
        .iter()
        .filter_map(|e| match e.kind {
            TrackEventKind::Midi {
                message: MidiMessage::NoteOn { vel, .. },
                ..
            } => Some((e.delta.as_int(), vel.as_int())),
            _ => None,
        })
        .collect();
    assert_eq!(kinds, vec![(48_000, 100), (2_400, 0), (0, 100), (2_400, 0)]);
}

#[test]
fn export_writes_the_file_and_an_empty_list_is_a_valid_file() {
    let dir = std::env::temp_dir().join(format!("drumspike-core-midi-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("spikes.mid");
    export_midi(&[], 38, &p, DEFAULT_PPQ, DEFAULT_TEMPO).unwrap();
    let bytes = std::fs::read(&p).unwrap();
    let smf = Smf::parse(&bytes).unwrap();
    assert_eq!(smf.header.format, Format::Parallel);
    assert_eq!(smf.header.timing, Timing::Metrical(u15::new(24_000)));
    assert_eq!(smf.tracks.len(), 2);
    assert_eq!(smf.tracks[1].len(), 3, "name, program change, end of track");
    std::fs::remove_dir_all(&dir).unwrap();
}

// ---------- P4-40g: the review markers ----------

/// The marker events of track 0: (abs tick, text) in file order.
fn markers_of(b: &[u8]) -> Vec<(u64, String)> {
    let smf = midly::Smf::parse(b).unwrap();
    let (mut tick, mut out) = (0u64, Vec::new());
    for e in &smf.tracks[0] {
        tick += e.delta.as_int() as u64;
        if let TrackEventKind::Meta(MetaMessage::Marker(t)) = e.kind {
            out.push((tick, String::from_utf8_lossy(t).into_owned()));
        }
    }
    out
}

#[test]
fn the_multi_track_file_writes_the_review_markers_at_their_ticks() {
    let a = [ev(0.5, 90)];
    let tr = [MidiTrack {
        name: "Kick",
        note: 36,
        events: &a,
    }];
    let markers = [
        MidiMarker {
            t: 1.0,
            note: Some("verse"),
        },
        MidiMarker { t: 3.0, note: None },
    ];
    // the default map: 120 BPM is 48,000 ticks per second; the list is written in the given order
    let bytes = multi_midi_bytes(&tr, &markers, DEFAULT_PPQ, &TempoMap::default(), DEFAULT_TEMPO).unwrap();
    assert_eq!(
        markers_of(&bytes),
        vec![(48_000, "verse".into()), (144_000, "Marker 2".into())]
    );
    // through a tempo map: 120 BPM to 8 s, then 100 BPM (40,000 ticks per second)
    let map = map3();
    let markers = [
        MidiMarker {
            t: 0.5,
            note: Some("verse"),
        },
        MidiMarker { t: 9.0, note: None },
    ];
    let bytes = multi_midi_bytes(&tr, &markers, DEFAULT_PPQ, &map, 120.0).unwrap();
    assert_eq!(
        markers_of(&bytes),
        vec![
            (24_000, "verse".into()),
            (96_000, "Bar 1".into()),
            (384_000 + 40_000, "Marker 2".into())
        ]
    );
    // a time that is no time is refused
    let bad = [MidiMarker {
        t: f64::NAN,
        note: None,
    }];
    assert!(matches!(
        multi_midi_bytes(&tr, &bad, DEFAULT_PPQ, &TempoMap::default(), 120.0),
        Err(MidiError::Tick(_))
    ));
}

#[test]
fn the_multi_track_file_holds_no_review_markers_when_the_export_has_none() {
    let a = [ev(0.5, 90)];
    let tr = [MidiTrack {
        name: "Kick",
        note: 36,
        events: &a,
    }];
    let none: [MidiMarker; 0] = [];
    let bytes = multi_midi_bytes(&tr, &none, DEFAULT_PPQ, &TempoMap::default(), DEFAULT_TEMPO).unwrap();
    assert_eq!(markers_of(&bytes), vec![], "no marker events without markers");
    let bytes = multi_midi_bytes(&tr, &none, DEFAULT_PPQ, &map3(), 120.0).unwrap();
    assert_eq!(
        markers_of(&bytes),
        vec![(96_000, "Bar 1".into())],
        "only the map's own Bar 1"
    );
}

// ---------- P4-8: the tempo map ----------

use crate::tempo::{MeterChange, TempoStep, import_midi};

fn map3() -> TempoMap {
    // ticks are exact: 120 BPM is 48,000 ticks per second, 100 BPM 40,000, 150 BPM 60,000
    TempoMap {
        bpm: 120.0,
        steps: vec![
            TempoStep { t: 8.0, bpm: 100.0 },
            TempoStep { t: 16.0, bpm: 150.0 },
            TempoStep { t: 24.0, bpm: 80.0 },
        ],
        meter: (4, 4),
        changes: vec![
            MeterChange { bar: 5, num: 3, den: 4 },
            MeterChange { bar: 9, num: 6, den: 8 },
        ],
        bar1_s: 2.0,
    }
}

/// The worst note-on error of a file in ticks, read the way the independent checker does:
/// only the file's own tempo events, in whole numbers (a time is `sum(ticks * us) / (ppq * 1e6)`).
/// Also returns the number of notes. Plants a failure when a hit has no note.
fn worst_error_ticks(bytes: &[u8], hits: &[f64]) -> (f64, usize) {
    let smf = Smf::parse(bytes).unwrap();
    let ppq = match smf.header.timing {
        Timing::Metrical(p) => p.as_int() as i128,
        _ => panic!("not metrical"),
    };
    let mut tempos: Vec<(i128, i128)> = vec![]; // (tick, us)
    let mut at = 0i128;
    for e in &smf.tracks[0] {
        at += e.delta.as_int() as i128;
        if let TrackEventKind::Meta(MetaMessage::Tempo(us)) = e.kind {
            tempos.push((at, us.as_int() as i128));
        }
    }
    if tempos.first().is_none_or(|t| t.0 != 0) {
        tempos.insert(0, (0, 500_000));
    }
    let mut ons = vec![];
    let mut at = 0i128;
    for e in &smf.tracks[1] {
        at += e.delta.as_int() as i128;
        if let TrackEventKind::Midi {
            message: MidiMessage::NoteOn { vel, .. },
            ..
        } = e.kind
            && vel.as_int() > 0
        {
            ons.push(at);
        }
    }
    assert_eq!(ons.len(), hits.len(), "one note per hit");
    let mut worst = 0f64;
    for (tick, hit) in ons.iter().zip(hits) {
        // numerator of the note's time in units of 1 / (ppq * 1e6) s
        let mut num = 0i128;
        let mut us_here = tempos[0].1;
        for (i, (t0, us)) in tempos.iter().enumerate() {
            let end = tempos.get(i + 1).map_or(i128::MAX, |n| n.0);
            if *tick > *t0 {
                num += (tick.min(&end) - t0) * us;
            }
            if *tick >= *t0 {
                us_here = *us;
            }
        }
        let hit_num = hit * (ppq as f64) * 1e6;
        let err = ((num as f64 - hit_num) / us_here as f64).abs();
        worst = worst.max(err);
    }
    (worst, ons.len())
}

#[test]
fn the_default_map_writes_the_bytes_of_the_old_export() {
    let evs = [ev(0.5, 90), ev(0.52, 100), ev(0.0, 1)];
    let b = midi_bytes_map(&evs, 36, DEFAULT_PPQ, &TempoMap::default(), DEFAULT_TEMPO).unwrap();
    assert_eq!(b, unhex(PYTHON_FILE));
    // a legacy tempo and ppq: the ticks and the written tempo are the same tempo (#434). The
    // tempo 90 BPM does not come out in whole microseconds (666666.67), so the file holds the
    // rounded 666667 and the notes sit on the ticks of that tempo, not of the exact 90.
    let b = midi_bytes_map(&evs, 36, 480, &TempoMap::default(), 90.0).unwrap();
    let smf = Smf::parse(&b).unwrap();
    let mut tempos = smf.tracks[0].iter().scan(0u32, |t, e| {
        *t += e.delta.as_int();
        Some((*t, e.kind))
    });
    assert_eq!(
        tempos.find_map(|(t, k)| match k {
            TrackEventKind::Meta(MetaMessage::Tempo(us)) => Some((t, us.as_int())),
            _ => None,
        }),
        Some((0, 666_667))
    );
    // the notes sit on the ticks of the written tempo (the file sorts them by tick)
    let hits = [0.0, 0.5, 0.52];
    assert!(
        worst_error_ticks(&b, &hits).0 <= 0.5 + 1e-6,
        "the notes follow the written tempo"
    );
    let long = [ev(0.5, 90), ev(123_456.123_456_789, 50), ev(0.52, 100)];
    assert_eq!(
        midi_bytes_map(&long, 36, DEFAULT_PPQ, &TempoMap::default(), DEFAULT_TEMPO).unwrap(),
        unhex(PYTHON_LONG_GAP)
    );
}

/// #434: the drift case. The single-tempo export (the default map, a caller tempo of 118 BPM)
/// writes notes whose ticks follow the exact 118.000 (#466) and whose `set_tempo` is the
/// nearest whole microsecond. A DAW that ignores the file's tempo and applies the user's
/// 118.000 keeps every note within 1 ms over 6 minutes; the round trip through the file's own
/// (integer) tempo events is bounded by the rounding mismatch of that integer, which grows
/// with time (about 12 ticks over 6 minutes at 118 BPM, the documented integer-set_tempo
/// limit; #466 moved this error from the DAW side to the file's own reading).
#[test]
fn a_single_tempo_export_round_trips_through_its_own_events_and_holds_the_user_tempo() {
    let ppq = 24_000;
    let hits: Vec<f64> = (0..600).map(|i| 2.025 + i as f64 * (60.0 / 118.0)).collect(); // 2 s to 6 min
    let evs: Vec<MidiEvent> = hits.iter().map(|&t| ev(t, 90)).collect();
    let b = midi_bytes_map(&evs, 36, ppq, &TempoMap::default(), 118.0).unwrap();
    let (worst, n) = worst_error_ticks(&b, &hits);
    assert_eq!(n, 600);
    assert!(
        worst <= integer_tempo_round_trip_ticks(118.0, ppq, 307.0, 0),
        "round trip through the file's own events: {worst} ticks"
    );
    let daw = worst_daw_error_s(&b, 118.0, &hits);
    assert!(daw <= 1.0e-3, "a DAW at 118.000: {daw} s over 6 minutes");
    // the map path with the same tempo behaves the same
    let map = TempoMap {
        bpm: 118.0,
        ..TempoMap::default()
    };
    let b = midi_bytes_map(&evs, 36, ppq, &map, 120.0).unwrap();
    assert!(worst_error_ticks(&b, &hits).0 <= integer_tempo_round_trip_ticks(118.0, ppq, 307.0, 0));
    assert!(worst_daw_error_s(&b, 118.0, &hits) <= 1.0e-3);
    // the multi-track file with the caller tempo too
    let tracks = [MidiTrack {
        name: "spikes",
        note: 36,
        events: &evs,
    }];
    let b = multi_midi_bytes(&tracks, &[], ppq, &TempoMap::default(), 118.0).unwrap();
    assert!(worst_error_ticks(&b, &hits).0 <= integer_tempo_round_trip_ticks(118.0, ppq, 307.0, 0));
    assert!(worst_daw_error_s(&b, 118.0, &hits) <= 1.0e-3);
}

/// #466: the round trip through the file's own whole-microsecond `set_tempo` cannot be tighter
/// than the rounding of those integers against the exact tempos: half a tick per segment
/// boundary (`boundaries` steps after the first, each step tick within half a tick of the
/// exact step time) plus half a tick of note rounding plus `t_max` seconds of the relative
/// mismatch of the worst tempo `bpm`, in ticks of the written tempo.
fn integer_tempo_round_trip_ticks(bpm: f64, ppq: u16, t_max: f64, boundaries: usize) -> f64 {
    let exact = 6e7 / bpm;
    let us = tempo_us(bpm).expect("in range") as f64;
    (boundaries as f64 + 1.0) * 0.5 + t_max * (us - exact).abs() / exact * (ppq as f64 * 1e6) / us
}

/// The worst error of the notes against a DAW that ignores the file's tempo events and applies
/// `bpm`: `|tick * 60 / (bpm * ppq) - hit|` in seconds.
fn worst_daw_error_s(bytes: &[u8], bpm: f64, hits: &[f64]) -> f64 {
    let smf = Smf::parse(bytes).unwrap();
    let ppq = match smf.header.timing {
        Timing::Metrical(p) => p.as_int(),
        _ => panic!("not metrical"),
    };
    let mut ons = vec![];
    let mut at = 0u64;
    for e in &smf.tracks[1] {
        at += e.delta.as_int() as u64;
        if let TrackEventKind::Midi {
            message: MidiMessage::NoteOn { vel, .. },
            ..
        } = e.kind
            && vel.as_int() > 0
        {
            ons.push(at);
        }
    }
    assert_eq!(ons.len(), hits.len(), "one note per hit");
    ons.iter().zip(hits).fold(0f64, |worst, (&tick, &hit)| {
        worst.max((tick as f64 * 60.0 / (bpm * ppq as f64) - hit).abs())
    })
}

#[test]
fn three_tempo_steps_and_a_meter_change_round_trip_with_every_note_within_half_a_tick() {
    let map = map3();
    let hits: Vec<f64> = (0..400).map(|i| 0.013 + i as f64 * 0.1111).collect(); // 0.013 s to 44.4 s
    let evs: Vec<MidiEvent> = hits.iter().map(|&t| ev(t, 90)).collect();
    let b = midi_bytes_map(&evs, 38, DEFAULT_PPQ, &map, DEFAULT_TEMPO).unwrap();
    let (worst, n) = worst_error_ticks(&b, &hits);
    assert_eq!(n, 400);
    assert!(worst <= 0.5 + 1e-6, "worst note error {worst} ticks");
    let got = import_midi(&b).unwrap();
    assert_eq!(got.map, map, "the map comes back as it went in");
    assert_eq!(
        (got.ppq, got.tempo_events, got.meter_events, got.snapped),
        (24_000, 4, 3, 0)
    );
    // the second leg: export the imported map again, the bytes are the same
    assert_eq!(midi_bytes_map(&evs, 38, DEFAULT_PPQ, &got.map, 120.0).unwrap(), b);
}

#[test]
fn a_step_off_the_tick_grid_comes_back_within_half_a_tick() {
    let map = TempoMap {
        steps: vec![
            TempoStep {
                t: 7.123_456,
                bpm: 90.0,
            },
            TempoStep {
                t: 19.000_001,
                bpm: 133.0,
            },
        ],
        ..TempoMap::default()
    };
    let hits = [0.5, 7.0, 7.123_456, 7.2, 19.0, 19.000_001, 30.0];
    let evs: Vec<MidiEvent> = hits.iter().map(|&t| ev(t, 80)).collect();
    let b = midi_bytes_map(&evs, 36, DEFAULT_PPQ, &map, 120.0).unwrap();
    // #466: the ticks sit on the exact tempos of the steps; the round trip through the file's
    // own integer set_tempo carries the rounding mismatch of those integers (worst at 90 BPM).
    assert!(worst_error_ticks(&b, &hits).0 <= integer_tempo_round_trip_ticks(90.0, DEFAULT_PPQ, 30.0, 2));
    let got = import_midi(&b).unwrap().map;
    assert_eq!(got.steps.len(), 2);
    for (a, b) in got.steps.iter().zip(&map.steps) {
        // #466: the step tick sits on the exact-tempo grid, so the time read back through the
        // file's integer set_tempo carries that integer's rounding mismatch: half a tick of the
        // step placement plus 19 s of the 90 BPM integer's 5e-7 relative mismatch.
        assert!(
            (a.t - b.t).abs() <= 0.5 * 666_667.0 / 24_000e6 + 19.0 * 0.5e-6 + 1e-9,
            "{} vs {}",
            a.t,
            b.t
        );
        assert_eq!(
            a.bpm,
            (6e7 / ((6e7f64 / b.bpm).round())),
            "the tempo is the file's whole microseconds"
        );
    }
}

#[test]
fn a_song_whose_bar_1_is_not_at_0_s_keeps_it_through_a_marker() {
    let map = TempoMap {
        bar1_s: 3.5,
        ..TempoMap::default()
    };
    let b = midi_bytes_map(&[ev(4.0, 100)], 36, DEFAULT_PPQ, &map, 120.0).unwrap();
    let smf = Smf::parse(&b).unwrap();
    let marker = smf.tracks[0]
        .iter()
        .scan(0u32, |t, e| {
            *t += e.delta.as_int();
            Some((*t, e.kind))
        })
        .find_map(|(t, k)| match k {
            TrackEventKind::Meta(MetaMessage::Marker(m)) => Some((t, m.to_vec())),
            _ => None,
        });
    assert_eq!(
        marker,
        Some((168_000, b"Bar 1".to_vec())),
        "3.5 s at 48,000 ticks per second"
    );
    assert_eq!(import_midi(&b).unwrap().map, map);
    // bar 1 at 0 s: no marker, as the old file
    let plain = midi_bytes_map(
        &[ev(4.0, 100)],
        36,
        DEFAULT_PPQ,
        &TempoMap {
            bpm: 90.0,
            ..TempoMap::default()
        },
        120.0,
    )
    .unwrap();
    assert!(!plain.windows(5).any(|w| w == b"Bar 1"));
}

#[test]
fn a_hit_before_bar_1_and_a_hit_before_0_s_keep_their_place() {
    let map = TempoMap {
        bar1_s: 4.0,
        steps: vec![TempoStep { t: 2.0, bpm: 100.0 }],
        ..TempoMap::default()
    };
    // 1.0 s and 3.0 s are before bar 1 (the second also after the step); -0.25 s is before the audio
    let hits = [1.0, 3.0, 5.0];
    let evs = [ev(-0.25, 100), ev(1.0, 100), ev(3.0, 100), ev(5.0, 100)];
    let b = midi_bytes_map(&evs, 36, DEFAULT_PPQ, &map, 120.0).unwrap();
    let notes = {
        let smf = Smf::parse(&b).unwrap();
        let mut at = 0u64;
        let mut v = vec![];
        for e in &smf.tracks[1] {
            at += e.delta.as_int() as u64;
            if let TrackEventKind::Midi {
                message: MidiMessage::NoteOn { vel, .. },
                ..
            } = e.kind
                && vel.as_int() > 0
            {
                v.push(at);
            }
        }
        v
    };
    // 120 BPM to 2 s (48,000 ticks per second), then 100 BPM (40,000 per second)
    assert_eq!(notes, vec![0, 48_000, 96_000 + 40_000, 96_000 + 120_000]);
    let (worst, _) = worst_error_ticks(&b, &[0.0, hits[0], hits[1], hits[2]]);
    assert!(worst <= 0.5 + 1e-6, "{worst}");
    assert_eq!(import_midi(&b).unwrap().map, map);
}

#[test]
fn a_map_that_cannot_be_written_is_refused() {
    let bad = TempoMap {
        bpm: 2.0,
        ..TempoMap::default()
    };
    assert!(matches!(
        midi_bytes_map(&[], 36, DEFAULT_PPQ, &bad, 120.0),
        Err(MidiError::Map(_))
    ));
    assert!(matches!(
        midi_bytes_map(
            // a time whose ticks saturate i64 has no place in the file (beyond `MAX_TICK`)
            &[ev(1e18, 1)],
            36,
            DEFAULT_PPQ,
            &TempoMap {
                bpm: 100.0,
                ..TempoMap::default()
            },
            120.0
        ),
        Err(MidiError::Tick(_))
    ));
}

#[test]
fn a_foreign_file_without_a_marker_puts_bar_1_at_0_and_snaps_a_meter_change_to_the_next_bar() {
    // 4/4 at tick 0, 3/4 at tick 100,000 (bar 2 starts at 96,000: the event is 4,000 ticks into bar 2)
    let t0 = [
        (0u64, Meta0::Tempo(600_000)),
        (0, Meta0::Signature(4, 2)),
        (100_000, Meta0::Signature(3, 2)),
    ];
    let b = write_file(&track1_messages(&[]), 24_000, &t0).unwrap();
    let got = import_midi(&b).unwrap();
    assert_eq!(got.map.bpm, 100.0);
    assert_eq!(got.map.changes, vec![MeterChange { bar: 3, num: 3, den: 4 }]);
    assert_eq!(got.snapped, 1);
    assert!(import_midi(b"MThd nope").is_err());
    let smpte = [b"MThd".as_slice(), &[0, 0, 0, 6, 0, 0, 0, 1, 0xE7, 0x28]].concat();
    assert!(import_midi(&smpte).is_err());
}

#[test]
fn export_midi_map_writes_the_bytes_of_midi_bytes_map() {
    let dir = std::env::temp_dir().join(format!("drumspike-core-midimap-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("map.mid");
    let evs = [ev(0.5, 90), ev(9.0, 70)];
    let map = map3();
    export_midi_map(&evs, 36, &p, DEFAULT_PPQ, &map, 120.0).unwrap();
    assert_eq!(
        std::fs::read(&p).unwrap(),
        midi_bytes_map(&evs, 36, DEFAULT_PPQ, &map, 120.0).unwrap()
    );
    let notes = note_events_tm(&evs, 36, &TickMap::new(DEFAULT_PPQ, &map).unwrap()).unwrap();
    assert_eq!((notes[0].tick, notes[1].tick), (24_000, 384_000 + 40_000));
    assert_eq!(
        note_events_tm(&evs, 200, &TickMap::new(DEFAULT_PPQ, &map).unwrap()),
        Err(MidiError::Note(200))
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Reads a file back: the tracks as (name, [(tick, note, velocity)] of the note ons) and the tempo events of track 0.
type NoteOns = Vec<(u64, u8, u8)>;

fn read_back(b: &[u8]) -> (Vec<(String, NoteOns)>, Vec<u32>) {
    let smf = midly::Smf::parse(b).unwrap();
    assert_eq!(smf.header.format, Format::Parallel);
    let mut tempos = Vec::new();
    let mut tracks = Vec::new();
    for (i, tr) in smf.tracks.iter().enumerate() {
        let (mut tick, mut name, mut ons) = (0u64, String::new(), Vec::new());
        for e in tr {
            tick += e.delta.as_int() as u64;
            match e.kind {
                TrackEventKind::Meta(MetaMessage::Tempo(us)) if i == 0 => tempos.push(us.as_int()),
                TrackEventKind::Meta(MetaMessage::TrackName(n)) => name = String::from_utf8_lossy(n).into_owned(),
                TrackEventKind::Midi {
                    message: MidiMessage::NoteOn { key, vel },
                    ..
                } if vel.as_int() > 0 => ons.push((tick, key.as_int(), vel.as_int())),
                _ => {}
            }
        }
        if i > 0 {
            tracks.push((name, ons));
        }
    }
    (tracks, tempos)
}

#[test]
fn a_multi_track_file_has_one_track_per_drum_on_its_own_note_and_the_default_tempo() {
    let (a, b) = ([ev(0.5, 90), ev(1.0, 100)], [ev(0.25, 70)]);
    let tr = [
        MidiTrack {
            name: "Kick In",
            note: 36,
            events: &a,
        },
        MidiTrack {
            name: "Snare",
            note: 38,
            events: &b,
        },
    ];
    let bytes = multi_midi_bytes(&tr, &[], DEFAULT_PPQ, &TempoMap::default(), DEFAULT_TEMPO).unwrap();
    let (tracks, tempos) = read_back(&bytes);
    assert_eq!(tempos, vec![500_000]);
    assert_eq!(tracks.len(), 2);
    assert_eq!(
        tracks[0],
        ("Kick In".to_string(), vec![(24_000, 36, 90), (48_000, 36, 100)])
    );
    assert_eq!(tracks[1], ("Snare".to_string(), vec![(12_000, 38, 70)]));
    // the notes of a track are those of the per-track file
    let single = note_events(&a, 36, DEFAULT_PPQ, DEFAULT_TEMPO).unwrap();
    assert_eq!(single[0].tick, tracks[0].1[0].0);
}

#[test]
fn a_multi_track_file_takes_the_tempo_events_and_ticks_of_the_map() {
    let a = [ev(0.5, 90), ev(9.0, 70)];
    let tr = [MidiTrack {
        name: "Kick",
        note: 36,
        events: &a,
    }];
    let map = map3();
    let (tracks, tempos) = read_back(&multi_midi_bytes(&tr, &[], DEFAULT_PPQ, &map, 120.0).unwrap());
    assert_eq!(tempos.len(), 4, "the start tempo and three steps");
    assert_eq!(tempos[0], 500_000);
    assert_eq!((tracks[0].1[0].0, tracks[0].1[1].0), (24_000, 384_000 + 40_000));
    // the same ticks as the per-track file
    let one = note_events_tm(&a, 36, &TickMap::new(DEFAULT_PPQ, &map).unwrap()).unwrap();
    assert_eq!(one[1].tick, tracks[0].1[1].0);
}

#[test]
fn a_multi_track_file_refuses_a_bad_note_a_bad_velocity_and_writes_the_path() {
    let a = [ev(0.5, 90)];
    let bad = [MidiTrack {
        name: "x",
        note: 200,
        events: &a,
    }];
    assert_eq!(
        multi_midi_bytes(&bad, &[], DEFAULT_PPQ, &TempoMap::default(), 120.0),
        Err(MidiError::Note(200))
    );
    let loud = [ev(0.5, 300)];
    let bad = [MidiTrack {
        name: "x",
        note: 36,
        events: &loud,
    }];
    assert_eq!(
        multi_midi_bytes(&bad, &[], DEFAULT_PPQ, &TempoMap::default(), 120.0),
        Err(MidiError::Velocity(300))
    );
    let none: [MidiTrack; 0] = [];
    let dir = std::env::temp_dir().join(format!("drumspike-core-multi-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("m.mid");
    export_multi_midi(&none, &[], &p, DEFAULT_PPQ, &TempoMap::default(), 120.0).unwrap();
    assert_eq!(
        std::fs::read(&p).unwrap(),
        multi_midi_bytes(&none, &[], DEFAULT_PPQ, &TempoMap::default(), 120.0).unwrap()
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
