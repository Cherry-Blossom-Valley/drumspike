//! #441: the MIDI export's ticks per quarter note by tempo and rate, the sample-anchored
//! note-ons, and the stated bound. Pure export checks through `drumspike_core`; the gate runs
//! them with `cargo test -p drumspike-app`.

use drumspike_core::midi::{MAX_PPQ, MidiEvent, export_midi_map, export_ppq, worst_error_samples};
use drumspike_core::tempo::{TempoMap, TempoStep, TickMap};

/// xorshift64*: the same hits on every run, no rand crate.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
}

/// The note-on ticks of a written file, in file order (by tick, then note and velocity).
fn note_ons(bytes: &[u8]) -> Vec<u64> {
    use midly::{Format, Timing};
    let smf = midly::Smf::parse(bytes).expect("a standard MIDI file");
    assert_eq!(smf.header.format, Format::Parallel);
    assert!(matches!(smf.header.timing, Timing::Metrical(p) if p.as_int() > 0));
    let mut at = 0u64;
    let mut ons = vec![];
    for e in &smf.tracks[1] {
        at += u64::from(e.delta.as_int());
        if let midly::TrackEventKind::Midi {
            message: midly::MidiMessage::NoteOn { vel, .. },
            ..
        } = e.kind
            && vel.as_int() > 0
        {
            ons.push(at);
        }
    }
    ons
}

/// 1000 hits at random samples: hit `t` sits within half a sample of its intended sample
/// `round(t * rate)` (never exactly on the tie), at times after 0 s, in random order.
fn hits(rate: u32) -> Vec<f64> {
    let mut rng = Rng(0x441);
    (0..1000)
        .map(|_| {
            let s = 1 + rng.next() % (300 * u64::from(rate));
            let frac = ((rng.next() % 1000) as f64 + 0.5) / 1000.0 - 0.5;
            (s as f64 + frac) / f64::from(rate)
        })
        .collect()
}

/// The same hits in time order: the file sorts its note-ons by tick, which follows the time.
fn sorted_hits(rate: u32) -> Vec<f64> {
    let mut hs = hits(rate);
    hs.sort_by(|a, b| a.partial_cmp(b).expect("finite hits"));
    hs
}

/// The bytes of the single-note export, through the public file writer (a scratch file, one
/// per call: the tests run in parallel).
fn export_bytes(evs: &[MidiEvent], ppq: u16, map: &TempoMap, rate: u32) -> Vec<u8> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("drumspike-441-export-{}-{n}.mid", std::process::id()));
    export_midi_map(evs, 36, &p, ppq, map, 120.0, rate).expect("the export writes");
    let bytes = std::fs::read(&p).expect("the file is there");
    let _ = std::fs::remove_file(&p);
    bytes
}

/// The worst distance between the file's note-on and its hit's intended sample
/// `round(t * rate)`, each tick converted back with the exact tempo of the map.
fn measured_worst(bytes: &[u8], ppq: u16, map: &TempoMap, hs: &[f64], rate: u32) -> f64 {
    let tm = TickMap::new(ppq, map).expect("the export's own tick map");
    note_ons(bytes)
        .iter()
        .zip(hs)
        .map(|(tick, t)| {
            let want = (t * f64::from(rate)).round();
            (tm.time(*tick as i64) * f64::from(rate) - want).abs()
        })
        .fold(0.0_f64, f64::max)
}

#[test]
fn the_ticks_per_quarter_follow_the_rate_and_the_exact_tempo() {
    // 48 kHz at 120 BPM is the old fixed 24,000: the golden bytes do not move
    assert_eq!(export_ppq(48_000, &TempoMap::default(), 120.0), 24_000);
    assert_eq!(export_ppq(44_100, &TempoMap::default(), 120.0), 22_050);
    // 96 kHz at 120 BPM wants 48,000, beyond the standard's 15 bits
    assert_eq!(export_ppq(96_000, &TempoMap::default(), 120.0), MAX_PPQ);
    // a tempo whose ideal is not a whole number of ticks takes the largest grid
    let m92 = TempoMap {
        bpm: 92.0,
        ..TempoMap::default()
    };
    assert_eq!(export_ppq(48_000, &m92, 120.0), MAX_PPQ);
    let m90 = TempoMap {
        bpm: 90.0,
        ..TempoMap::default()
    };
    assert_eq!(export_ppq(48_000, &m90, 120.0), 32_000);
    // the default map's effective tempo is the legacy tempo the file writes
    assert_eq!(export_ppq(48_000, &TempoMap::default(), 90.0), 32_000);
    assert_eq!(export_ppq(48_000, &TempoMap::default(), 118.0), MAX_PPQ);
}

#[test]
fn every_hit_lands_on_its_sample_where_the_grid_allows_it_else_within_the_bound() {
    let mut checked = 0usize;
    for rate in [44_100u32, 48_000, 96_000] {
        for bpm in 40..=240 {
            let map = TempoMap {
                bpm: f64::from(bpm),
                ..TempoMap::default()
            };
            let ppq = export_ppq(rate, &map, 120.0);
            let hs = sorted_hits(rate);
            let evs = events(&hs);
            let bytes = export_bytes(&evs, ppq, &map, rate);
            let ons = note_ons(&bytes);
            assert_eq!(ons.len(), 1000, "one note per hit at {rate} Hz, {bpm} BPM");
            checked += ons.len();
            let tm = TickMap::new(ppq, &map).expect("the export's own tick map");
            let bound = worst_error_samples(&evs, ppq, &map, 120.0, rate).expect("the map is valid");
            let ideal = 60.0 * f64::from(rate) / f64::from(bpm);
            for (tick, &t) in ons.iter().zip(&hs) {
                let want = (t * f64::from(rate)).round() as i64;
                let got = tm.time(*tick as i64) * f64::from(rate);
                if ideal <= f64::from(MAX_PPQ) {
                    assert_eq!(
                        got.round() as i64,
                        want,
                        "{rate} Hz {bpm} BPM ppq {ppq}: tick {tick} reads sample {}, want {want}",
                        got.round()
                    );
                } else {
                    assert!(
                        (got - want as f64).abs() <= bound + 1.0e-6,
                        "{rate} Hz {bpm} BPM ppq {ppq}: error {} over the bound {bound}",
                        (got - want as f64).abs()
                    );
                }
            }
        }
    }
    assert!(checked > 0, "the sweep checked no notes");
    println!("checked {checked} note-ons over 603 tempo and rate pairs");
}

#[test]
fn a_three_tempo_map_takes_the_largest_grid_and_holds_its_bound() {
    let rate = 48_000u32;
    let map = TempoMap {
        bpm: 120.0,
        steps: vec![TempoStep { t: 10.0, bpm: 100.0 }, TempoStep { t: 20.0, bpm: 150.0 }],
        ..TempoMap::default()
    };
    // differing ideals (24000, 28800, 19200): the largest grid gives the lowest worst error
    assert_eq!(export_ppq(rate, &map, 120.0), MAX_PPQ);
    // three equal tempos share one whole ideal: that ideal
    let same = TempoMap {
        bpm: 90.0,
        steps: vec![TempoStep { t: 10.0, bpm: 90.0 }, TempoStep { t: 20.0, bpm: 90.0 }],
        ..TempoMap::default()
    };
    assert_eq!(export_ppq(rate, &same, 120.0), 32_000);

    let hs = sorted_hits(rate);
    let evs = events(&hs);
    let ppq = export_ppq(rate, &map, 120.0);
    let bytes = export_bytes(&evs, ppq, &map, rate);
    assert_eq!(note_ons(&bytes).len(), 1000);
    // every segment's ideal is at most the grid: all 1000 note-ons sit on their sample
    let tm = TickMap::new(ppq, &map).expect("the export's own tick map");
    for (tick, &t) in note_ons(&bytes).iter().zip(&hs) {
        let want = (t * f64::from(rate)).round() as i64;
        assert_eq!(
            (tm.time(*tick as i64) * f64::from(rate)).round() as i64,
            want,
            "tick {tick} off its sample at the three-tempo map"
        );
    }
    // and the returned bound is the measured worst error over the same note-ons
    let bound = worst_error_samples(&evs, ppq, &map, 120.0, rate).expect("the map is valid");
    let worst = measured_worst(&bytes, ppq, &map, &hs, rate);
    assert!((worst - bound).abs() < 1.0e-9, "bound {bound} vs measured {worst}");
}

#[test]
fn the_returned_bound_is_the_measured_worst_error() {
    let rate = 48_000u32;
    // a grid coarser than a sample: 48 kHz at 40 BPM wants 72,000 ticks per quarter
    let map = TempoMap {
        bpm: 40.0,
        ..TempoMap::default()
    };
    let ppq = export_ppq(rate, &map, 120.0);
    assert_eq!(ppq, MAX_PPQ);
    let hs = sorted_hits(rate);
    let evs = events(&hs);
    let bytes = export_bytes(&evs, ppq, &map, rate);
    let bound = worst_error_samples(&evs, ppq, &map, 120.0, rate).expect("the map is valid");
    let worst = measured_worst(&bytes, ppq, &map, &hs, rate);
    assert!((worst - bound).abs() < 1.0e-9, "bound {bound} vs measured {worst}");
    // the exact case measures zero (to float dust): 48 kHz at 120 BPM, one tick is one sample
    let exact = TempoMap::default();
    let ppq = export_ppq(rate, &exact, 120.0);
    assert_eq!(ppq, 24_000);
    let bytes = export_bytes(&evs, ppq, &exact, rate);
    let bound = worst_error_samples(&evs, ppq, &exact, 120.0, rate).unwrap();
    assert!(bound < 1.0e-6, "the exact grid leaves {bound} samples");
    let worst = measured_worst(&bytes, ppq, &exact, &hs, rate);
    assert!((worst - bound).abs() < 1.0e-9, "bound {bound} vs measured {worst}");
}

fn events(hs: &[f64]) -> Vec<MidiEvent> {
    hs.iter().map(|&t| MidiEvent { t, v: 100 }).collect()
}
