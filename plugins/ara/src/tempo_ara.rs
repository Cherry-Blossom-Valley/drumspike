//! M1c1 (#560): the tempo map of the host in song seconds.
//!
//! The ARA musical context gives the tempo as sync points `(time in song seconds, quarter position)`: the tempo
//! between two points is constant, before the first point it is the tempo of the first section, after the last
//! point that of the last section. The bar signatures give the meter and the position of bar 1 (the first one).
//! [`tempo_from_ara`] turns them into a `TempoMap` of drumspike-core (steps at song seconds), which the MIDI
//! export of the app writes into the file. The design is `docs/design/daw-plugin/m1c1-delivery.md`, section 2.

use drumspike_core::tempo::{MeterChange, TempoMap, TempoStep};

/// A tempo sync point of the host.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Entry {
    /// Song time in seconds.
    pub t: f64,
    /// The same moment in quarter notes.
    pub q: f64,
}

/// A bar signature of the host: valid from `q` (quarter notes) on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bar {
    pub num: i32,
    pub den: i32,
    pub q: f64,
}

/// What the conversion made.
#[derive(Debug, Clone, PartialEq)]
pub struct Converted {
    pub map: TempoMap,
    /// Short reasons for what was left out or changed (for the report and the page).
    pub notes: Vec<&'static str>,
}

/// The least relative difference of two tempos that makes a step.
const SAME: f64 = 1e-9;
/// The shortest gap between two steps (`MIN_STEP_GAP_S` of the core).
const MIN_GAP_S: f64 = 0.001;

fn section_bpm(a: Entry, b: Entry) -> f64 {
    60.0 * (b.q - a.q) / (b.t - a.t)
}

/// The song time of a quarter position: linear inside a section, the first and the last section run on.
fn time_of_quarter(e: &[Entry], q: f64) -> f64 {
    let i = e.partition_point(|x| x.q <= q).clamp(1, e.len() - 1);
    let (a, b) = (e[i - 1], e[i]);
    a.t + (q - a.q) * (b.t - a.t) / (b.q - a.q)
}

fn valid_meter(num: i32, den: i32) -> bool {
    (1..=64).contains(&num) && matches!(den, 1 | 2 | 4 | 8 | 16 | 32 | 64)
}

/// Convert the sync points and the bar signatures of the host to a tempo map in song seconds.
///
/// # Errors
///
/// A short reason: `entries` (fewer than 2 points, or points that do not grow in time and quarters) or
/// `tempo-range` (a tempo outside 4 to 1000 BPM).
pub fn tempo_from_ara(entries: &[Entry], bars: &[Bar]) -> Result<Converted, &'static str> {
    let mut e: Vec<Entry> = entries.to_vec();
    e.sort_by(|a, b| a.t.total_cmp(&b.t));
    if e.len() < 2 || e.iter().any(|x| !x.t.is_finite() || !x.q.is_finite()) {
        return Err("entries");
    }
    if e.windows(2).any(|w| w[1].t <= w[0].t || w[1].q <= w[0].q) {
        return Err("entries");
    }
    let n = e.len();
    // the tempo of section i (between point i and i + 1), i in 0..n-1
    let tempo: Vec<f64> = (0..n - 1).map(|i| section_bpm(e[i], e[i + 1])).collect();
    if tempo.iter().any(|b| !(4.0..=1000.0).contains(b)) {
        return Err("tempo-range");
    }
    let mut notes = Vec::new();
    // the tempo at song time 0: the section that holds it
    let at0 = e.partition_point(|x| x.t <= 0.0).clamp(1, n - 1) - 1;
    let mut map = TempoMap {
        bpm: tempo[at0],
        ..TempoMap::default()
    };
    // later boundaries: point i (1..=n-2) ends section i-1 and starts section i
    let mut cur = tempo[at0];
    let mut merged = false;
    for i in 1..n - 1 {
        if e[i].t <= 0.0 || tempo[i] == cur || ((tempo[i] - cur).abs() / cur) < SAME {
            continue;
        }
        let last_t = map.steps.last().map_or(0.0, |s| s.t);
        if e[i].t - last_t < MIN_GAP_S {
            // too close to the step before: the later tempo wins
            merged = true;
            match map.steps.last_mut() {
                Some(s) => s.bpm = tempo[i],
                None => map.bpm = tempo[i],
            }
        } else {
            map.steps.push(TempoStep { t: e[i].t, bpm: tempo[i] });
        }
        cur = tempo[i];
    }
    if merged {
        notes.push("steps-merged");
    }
    // the bar signatures: the first one is bar 1
    let mut b: Vec<Bar> = bars.to_vec();
    b.sort_by(|x, y| x.q.total_cmp(&y.q));
    if let Some(first) = b.first().copied() {
        if valid_meter(first.num, first.den) && first.q.is_finite() {
            map.meter = (first.num as u8, first.den as u8);
            let bar1 = time_of_quarter(&e, first.q);
            if bar1 < 0.0 {
                notes.push("bar1-before-start");
            } else {
                map.bar1_s = bar1;
            }
            let (mut bar, mut cur_q, mut cur_m) = (1u32, first.q, (first.num, first.den));
            for s in b.iter().skip(1) {
                let len = 4.0 * f64::from(cur_m.0) / f64::from(cur_m.1);
                let bars_between = (s.q - cur_q) / len;
                if !valid_meter(s.num, s.den) || (bars_between - bars_between.round()).abs() > 1e-6 || bars_between < 0.5 {
                    notes.push("bars-not-whole");
                    break;
                }
                bar += bars_between.round() as u32;
                map.changes.push(MeterChange { bar, num: s.num as u8, den: s.den as u8 });
                cur_q = s.q;
                cur_m = (s.num, s.den);
            }
        } else {
            notes.push("bars-invalid");
        }
    }
    map.validate().map_err(|_| "tempo-range")?;
    Ok(Converted { map, notes })
}

/// The tempo the export uses, and where it came from (the answer of `plugin_host_tempo`).
#[derive(Debug, Clone, PartialEq)]
pub struct Answer {
    /// The map in song seconds; `None`: the project's own map (the default, 120 BPM).
    pub map: Option<TempoMap>,
    /// `ara` (the timeline of the host), `transport` (one tempo of the process call), `transport-varies` (the same, and
    /// the host's tempo changed: the file does not follow it) or `none`.
    pub source: &'static str,
    pub bpm: f64,
    pub steps: usize,
    /// Why the ARA timeline was not used, or what the conversion changed.
    pub reason: String,
}

/// The order of the sources: the ARA timeline, then the tempo of the process call, then nothing.
#[must_use]
pub fn choose(ara: Option<&Converted>, ara_reason: &str, transport: Option<(f64, bool)>) -> Answer {
    if let Some(c) = ara {
        return Answer {
            map: Some(c.map.clone()),
            source: "ara",
            bpm: c.map.bpm,
            steps: c.map.steps.len(),
            reason: c.notes.join(","),
        };
    }
    if let Some((bpm, varies)) = transport.filter(|(b, _)| (4.0..=1000.0).contains(b)) {
        return Answer {
            map: Some(TempoMap { bpm, ..TempoMap::default() }),
            source: if varies { "transport-varies" } else { "transport" },
            bpm,
            steps: 0,
            reason: ara_reason.to_string(),
        };
    }
    Answer { map: None, source: "none", bpm: 120.0, steps: 0, reason: ara_reason.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use drumspike_core::midi::export_ppq;
    use drumspike_core::tempo::TickMap;

    fn e(t: f64, q: f64) -> Entry {
        Entry { t, q }
    }

    /// 120 BPM for 10 s (20 quarters), then 100 BPM.
    fn two_tempos() -> Vec<Entry> {
        vec![e(0.0, 0.0), e(10.0, 20.0), e(40.0, 70.0)]
    }

    #[test]
    fn two_tempos_become_a_start_tempo_and_one_step() {
        let c = tempo_from_ara(&two_tempos(), &[]).unwrap();
        assert_eq!(c.map.bpm, 120.0);
        assert_eq!(c.map.steps, vec![TempoStep { t: 10.0, bpm: 100.0 }]);
        assert!(c.notes.is_empty());
    }

    #[test]
    fn equal_sections_make_no_step_and_the_first_section_runs_back_before_song_zero() {
        let c = tempo_from_ara(&[e(-2.0, -4.0), e(0.0, 0.0), e(4.0, 8.0)], &[]).unwrap();
        assert_eq!((c.map.bpm, c.map.steps.len()), (120.0, 0));
        // the tempo at song 0 is the section that holds it, here the second one
        let c = tempo_from_ara(&[e(-4.0, -4.0), e(-1.0, -1.0), e(3.0, 7.0)], &[]).unwrap();
        assert_eq!((c.map.bpm, c.map.steps.len()), (120.0, 0), "the second section holds song 0");
    }

    #[test]
    fn a_step_closer_than_a_millisecond_merges_into_the_one_before() {
        let c = tempo_from_ara(&[e(0.0, 0.0), e(5.0, 10.0), e(5.0005, 10.0004), e(9.0, 20.0)], &[]).unwrap();
        assert!(c.notes.contains(&"steps-merged"));
        assert_eq!(c.map.steps.len(), 1);
        assert!(c.map.validate().is_ok());
    }

    #[test]
    fn bad_points_and_a_tempo_out_of_range_are_refused_with_a_reason() {
        assert_eq!(tempo_from_ara(&[e(0.0, 0.0)], &[]), Err("entries"));
        assert_eq!(tempo_from_ara(&[e(0.0, 0.0), e(0.0, 1.0)], &[]), Err("entries"));
        assert_eq!(tempo_from_ara(&[e(0.0, 0.0), e(1.0, 0.01)], &[]), Err("tempo-range"));
    }

    #[test]
    fn bar_signatures_give_the_meter_the_changes_and_the_place_of_bar_one() {
        let bars = [
            Bar { num: 4, den: 4, q: 4.0 },
            Bar { num: 3, den: 4, q: 20.0 },
            Bar { num: 7, den: 8, q: 29.0 },
        ];
        let c = tempo_from_ara(&two_tempos(), &bars).unwrap();
        assert_eq!(c.map.meter, (4, 4));
        assert!((c.map.bar1_s - 2.0).abs() < 1e-12, "quarter 4 at 120 BPM is 2 s");
        assert_eq!(
            c.map.changes,
            vec![MeterChange { bar: 5, num: 3, den: 4 }, MeterChange { bar: 8, num: 7, den: 8 }]
        );
    }

    #[test]
    fn bar_one_before_the_start_stays_at_zero_and_a_split_bar_drops_the_rest() {
        let c = tempo_from_ara(&two_tempos(), &[Bar { num: 4, den: 4, q: -8.0 }, Bar { num: 3, den: 4, q: 1.0 }]).unwrap();
        assert_eq!(c.map.bar1_s, 0.0);
        assert!(c.notes.contains(&"bar1-before-start") && c.notes.contains(&"bars-not-whole"));
        assert!(c.map.changes.is_empty());
    }

    #[test]
    fn the_sources_come_in_order_ara_then_the_process_call_then_nothing() {
        let c = tempo_from_ara(&two_tempos(), &[]).unwrap();
        assert_eq!(choose(Some(&c), "", Some((90.0, false))).source, "ara");
        let a = choose(None, "no-controller", Some((97.3, false)));
        assert_eq!((a.source, a.bpm, a.steps), ("transport", 97.3, 0));
        assert_eq!(a.map.as_ref().unwrap().bpm, 97.3);
        assert_eq!(choose(None, "x", Some((97.3, true))).source, "transport-varies");
        let n = choose(None, "no-controller", None);
        assert_eq!((n.source, n.map), ("none", None));
        assert_eq!(choose(None, "x", Some((0.5, false))).source, "none", "a tempo out of range is no tempo");
    }

    #[test]
    fn each_note_reads_back_within_one_sample_of_its_hit_at_the_session_rate() {
        let c = tempo_from_ara(&two_tempos(), &[]).unwrap();
        for rate in [44_100u32, 48_000] {
            let ppq = export_ppq(rate, &c.map, 120.0);
            let tm = TickMap::new(ppq, &c.map).unwrap();
            let mut worst = 0.0f64;
            for k in 0..4000 {
                let t = f64::from(k) * 0.01234 + 0.5; // hits over 49 s, across the step at 10 s
                let sample = (t * f64::from(rate)).round();
                let hit = sample / f64::from(rate);
                let back = tm.time(tm.tick(hit));
                worst = worst.max((back - hit).abs() * f64::from(rate));
            }
            assert!(worst <= 1.0, "{rate} Hz: worst {worst} samples");
        }
    }

    #[test]
    fn a_tempo_whose_quarter_is_not_whole_microseconds_reports_its_bound() {
        // 97.3 BPM: the bound is what worst_error_samples measures, and it is at most the tick grid
        let c = tempo_from_ara(&[e(0.0, 0.0), e(60.0, 97.3), e(120.0, 194.6)], &[]).unwrap();
        assert!((c.map.bpm - 97.3).abs() < 1e-9);
        let ppq = export_ppq(48_000, &c.map, 120.0);
        let ev: Vec<drumspike_core::midi::MidiEvent> =
            (0..100).map(|k| drumspike_core::midi::MidiEvent { t: f64::from(k) * 0.37, v: 90 }).collect();
        let w = drumspike_core::midi::worst_error_samples(&ev, ppq, &c.map, 120.0, 48_000).unwrap();
        assert!(w <= 1.0, "the grid is finer than 2 samples at 97 BPM: {w}");
    }
}
