//! P4-8 stage B: the tempo detection on synthetic hit trains with a known map.
//!
//! Each train is built bar by bar from a pattern of eighth notes (kick and snare), with a
//! seeded jitter, dropped hits and swing. The checks name the known tempo, step and bar 1.

use drumspike_core::tempo_detect::{detect, DetectParams, Hit, Role};

/// A small deterministic random source (xorshift64*).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
    }

    /// A normal value (Box-Muller).
    fn normal(&mut self) -> f64 {
        let u = self.next().max(1e-12);
        let v = self.next();
        (-2.0 * u.ln()).sqrt() * (2.0 * std::f64::consts::PI * v).cos()
    }
}

/// One section: bars at a tempo with a pattern. A pattern is one string per role over the
/// sixteenth notes of one bar (`x` = hit at velocity 100, `g` = ghost note at 25 to 45), e.g.
/// kick `x.......x.x.....`.
struct Section {
    bars: usize,
    bpm: f64,
    beats: usize,
    kick: &'static str,
    snare: &'static str,
}

const ROCK_K: &str = "x.......x.x.....";
const ROCK_S: &str = "....x.......x...";

fn rock(bars: usize, bpm: f64) -> Section {
    Section {
        bars,
        bpm,
        beats: 4,
        kick: ROCK_K,
        snare: ROCK_S,
    }
}

struct Train {
    start: f64,
    jitter_ms: f64,
    drop: f64,
    /// Where the second eighth of a beat falls, as a share of the beat (0.5 straight).
    swing: f64,
    seed: u64,
}

impl Default for Train {
    fn default() -> Self {
        Train {
            start: 1.0,
            jitter_ms: 4.0,
            drop: 0.0,
            swing: 0.5,
            seed: 7,
        }
    }
}

/// The hits of the sections, and the times of each section's first bar line.
fn build(train: &Train, sections: &[Section]) -> (Vec<Hit>, Vec<f64>) {
    let mut rng = Rng(train.seed);
    // the ghost velocities come from their own source: the times stay those of round 1
    let mut vel = Rng(train.seed ^ 0x9E37_79B9_7F4A_7C15);
    let mut hits = Vec::new();
    let mut starts = Vec::new();
    let mut t = train.start;
    for s in sections {
        starts.push(t);
        let beat = 60.0 / s.bpm;
        let slots = s.kick.len();
        let per_beat = slots / s.beats;
        for _ in 0..s.bars {
            for (role, pat) in [(Role::Kick, s.kick), (Role::Snare, s.snare)] {
                for (i, c) in pat.chars().enumerate() {
                    if !matches!(c, 'x' | 'g') || rng.next() < train.drop {
                        continue;
                    }
                    let b = (i / per_beat) as f64;
                    let sub = (i % per_beat) as f64 / per_beat as f64;
                    // Swing moves the second eighth (and the sixteenths around it).
                    let pos = if per_beat >= 2 && (sub - 0.5).abs() < 1e-9 {
                        train.swing
                    } else {
                        sub
                    };
                    let jit = rng.normal() * train.jitter_ms / 1000.0;
                    let v = if c == 'g' { 25.0 + 20.0 * vel.next() } else { 100.0 };
                    hits.push(Hit {
                        v,
                        ..Hit::new(t + (b + pos) * beat + jit, role)
                    });
                }
            }
            t += s.beats as f64 * beat;
        }
    }
    hits.sort_by(|a, b| a.t.total_cmp(&b.t));
    (hits, starts)
}

fn params() -> DetectParams {
    DetectParams::default()
}

#[test]
fn a_steady_train_gives_its_tempo_bar_1_and_no_step() {
    let (hits, starts) = build(&Train::default(), &[rock(64, 120.0)]);
    let d = detect(&hits, &params()).unwrap();
    assert!((d.bpm - 120.0).abs() < 0.05, "bpm {}", d.bpm);
    assert!(d.steps.is_empty(), "steps {:?}", d.steps);
    assert_eq!(d.meter, (4, 4));
    assert!(
        (d.bar1_s - starts[0]).abs() < 0.01,
        "bar1 {} want {}",
        d.bar1_s,
        starts[0]
    );
    assert!(d.confidence > 0.5, "confidence {}", d.confidence);
    assert!(d.on_beat > 0.75 && d.jitter_ms < 8.0, "{} {}", d.on_beat, d.jitter_ms);
    assert_eq!(d.map.steps.len(), 0);
    assert!((d.map.bpm - 120.0).abs() < 0.05);
}

#[test]
fn one_step_is_found_on_its_bar_line_with_both_tempos() {
    let (hits, starts) = build(&Train::default(), &[rock(32, 120.0), rock(32, 100.0)]);
    let d = detect(&hits, &params()).unwrap();
    assert!((d.bpm - 120.0).abs() < 0.1, "bpm {}", d.bpm);
    assert_eq!(d.steps.len(), 1, "steps {:?}", d.steps);
    let s = d.steps[0];
    assert!((s.bpm - 100.0).abs() < 0.1, "step {:?}", s);
    assert!((s.t - starts[1]).abs() < 0.01, "step at {} want {}", s.t, starts[1]);
    assert_eq!(s.bar, 33);
    assert_eq!(s.bars, 32);
    assert!(s.confidence > 0.5, "{s:?}");
}

#[test]
fn a_slow_ballad_with_sixteenth_kicks_is_not_called_double_tempo() {
    // 67 BPM, kick on sixteenths around the beat (as the Session A session), snare on 2 and 4.
    let s = Section {
        bars: 40,
        bpm: 67.0,
        beats: 4,
        kick: "x.x.....x.xx..x.",
        snare: "....x.......x...",
    };
    let (hits, _) = build(&Train::default(), &[s]);
    let d = detect(&hits, &params()).unwrap();
    assert!(
        (d.bpm - 67.0).abs() < 0.05,
        "bpm {} alternatives {:?}",
        d.bpm,
        d.alternatives
    );
}

#[test]
fn a_fast_punk_beat_is_not_called_half_tempo() {
    let s = Section {
        bars: 64,
        bpm: 180.0,
        beats: 4,
        kick: "x.......x.......",
        snare: "....x.......x...",
    };
    let (hits, _) = build(&Train::default(), &[s]);
    let d = detect(&hits, &params()).unwrap();
    assert!(
        (d.bpm - 180.0).abs() < 0.1,
        "bpm {} alternatives {:?}",
        d.bpm,
        d.alternatives
    );
}

#[test]
fn a_sparse_kick_and_missing_hits_still_give_the_tempo() {
    let s = Section {
        bars: 48,
        bpm: 96.0,
        beats: 4,
        kick: "x...............",
        snare: "....x.......x...",
    };
    let train = Train {
        drop: 0.3,
        seed: 11,
        ..Train::default()
    };
    let (hits, _) = build(&train, &[s]);
    let d = detect(&hits, &params()).unwrap();
    assert!(
        (d.bpm - 96.0).abs() < 0.1,
        "bpm {} alternatives {:?}",
        d.bpm,
        d.alternatives
    );
    assert!(d.steps.is_empty(), "{:?}", d.steps);
}

#[test]
fn swung_eighths_do_not_move_the_tempo_or_bar_1() {
    let s = Section {
        bars: 48,
        bpm: 110.0,
        beats: 4,
        kick: "x.x.....x.x.....",
        snare: "....x.x.....x...",
    };
    let train = Train {
        swing: 2.0 / 3.0,
        ..Train::default()
    };
    let (hits, starts) = build(&train, &[s]);
    let d = detect(&hits, &params()).unwrap();
    assert!((d.bpm - 110.0).abs() < 0.05, "bpm {}", d.bpm);
    assert!((d.bar1_s - starts[0]).abs() < 0.01, "bar1 {}", d.bar1_s);
}

#[test]
fn a_new_tempo_for_fewer_than_min_bars_is_no_step() {
    // 3 bars at 140 inside 120: no step with the 4-bar rule; with min_bars 2 both steps appear.
    let secs = || [rock(24, 120.0), rock(3, 140.0), rock(24, 120.0)];
    let (hits, starts) = build(&Train::default(), &secs());
    let d = detect(&hits, &params()).unwrap();
    assert!(d.steps.is_empty(), "4-bar rule: {:?}", d.steps);
    // P4-8 fix round 1: the 3 bars the map cannot hold are reported
    assert_eq!(d.excursions.len(), 1, "{:?}", d.excursions);
    let x = d.excursions[0];
    assert!(
        (x.t - starts[1]).abs() < 0.01 && (x.bpm - 140.0).abs() < 0.2 && x.bars == 3,
        "{x:?}"
    );
    assert!((x.map_bpm - d.map.bpm).abs() < 1e-9);
    let d2 = detect(
        &hits,
        &DetectParams {
            min_bars: 2,
            ..params()
        },
    )
    .unwrap();
    assert_eq!(d2.steps.len(), 2, "min_bars 2: {:?}", d2.steps);
    assert!(d2.excursions.is_empty(), "no probe at min_bars 2");
    assert!((d2.steps[0].t - starts[1]).abs() < 0.01 && (d2.steps[0].bpm - 140.0).abs() < 0.2);
    assert!((d2.steps[1].t - starts[2]).abs() < 0.01 && (d2.steps[1].bpm - 120.0).abs() < 0.2);
}

#[test]
fn a_new_tempo_for_exactly_min_bars_is_a_step() {
    let (hits, starts) = build(&Train::default(), &[rock(24, 120.0), rock(4, 132.0), rock(24, 120.0)]);
    let d = detect(&hits, &params()).unwrap();
    assert_eq!(d.steps.len(), 2, "{:?}", d.steps);
    assert!((d.steps[0].t - starts[1]).abs() < 0.01, "{:?}", d.steps);
    assert_eq!(d.steps[0].bars, 4);
    assert!(d.excursions.is_empty(), "{:?}", d.excursions);
}

#[test]
fn a_waltz_is_three_four_and_a_backbeat_is_four_four() {
    let w = Section {
        bars: 64,
        bpm: 150.0,
        beats: 3,
        kick: "x...........",
        snare: "....x...x...",
    };
    let (hits, starts) = build(&Train::default(), &[w]);
    let d = detect(&hits, &params()).unwrap();
    assert_eq!(d.meter, (3, 4), "r3 {} r4 {}", d.meter_r3, d.meter_r4);
    assert!((d.bpm - 150.0).abs() < 0.1, "bpm {}", d.bpm);
    assert!((d.bar1_s - starts[0]).abs() < 0.01, "bar1 {}", d.bar1_s);
    let (hits, _) = build(&Train::default(), &[rock(32, 120.0)]);
    assert_eq!(detect(&hits, &params()).unwrap().meter, (4, 4));
}

#[test]
fn too_few_hits_and_bad_parameters_are_refused() {
    let hits: Vec<Hit> = (0..5).map(|i| Hit::new(i as f64 * 0.5, Role::Kick)).collect();
    assert!(detect(&hits, &params()).unwrap_err().contains("at least 8"));
    let (hits, _) = build(&Train::default(), &[rock(8, 120.0)]);
    for bad in [
        DetectParams {
            min_bars: 0,
            ..params()
        },
        DetectParams {
            min_step_pct: f64::NAN,
            ..params()
        },
        DetectParams {
            bpm_range: (100.0, 150.0),
            ..params()
        },
    ] {
        assert!(detect(&hits, &bad).is_err(), "{bad:?}");
    }
}

#[test]
fn the_suggested_map_is_a_valid_map() {
    let (hits, _) = build(&Train::default(), &[rock(16, 120.0), rock(16, 90.0)]);
    let d = detect(&hits, &params()).unwrap();
    d.map.validate().unwrap();
    assert_eq!(d.map.steps.len(), d.steps.len());
    assert_eq!(d.map.meter, d.meter);
}

/// One row of the error table: the detection against the known map of a train.
fn row(name: &str, train: &Train, sections: &[Section]) -> (f64, bool, f64, f64, f64) {
    let (hits, starts) = build(train, sections);
    let d = detect(&hits, &params()).unwrap();
    let bpm_err = (d.bpm - sections[0].bpm).abs();
    // Known steps: where the tempo changes from one section to the next.
    let mut want: Vec<(f64, f64)> = Vec::new();
    // A section shorter than the 4-bar rule is no step (the map cannot hold it).
    let short = sections.iter().any(|s| s.bars < 4);
    for k in 1..sections.len() {
        if !short && sections[k].bpm != sections[k - 1].bpm {
            want.push((starts[k], sections[k].bpm));
        }
    }
    // No false alarm: only the train with a short section has an excursion.
    assert_eq!(d.excursions.is_empty(), !short, "{name}: {:?}", d.excursions);
    let same = d.steps.len() == want.len();
    let (mut step_bpm, mut step_ms) = (0.0f64, 0.0f64);
    if same {
        for (s, w) in d.steps.iter().zip(&want) {
            step_bpm = step_bpm.max((s.bpm - w.1).abs());
            step_ms = step_ms.max((s.t - w.0).abs() * 1000.0);
        }
    }
    let bar1_ms = (d.bar1_s - starts[0]).abs() * 1000.0;
    println!(
        "| {name} | {:.3} | {:.4} | {}/{} | {:.4} | {:.2} | {:.2} | {:.2} | {:.3} |",
        d.bpm,
        bpm_err,
        d.steps.len(),
        want.len(),
        step_bpm,
        step_ms,
        bar1_ms,
        d.jitter_ms,
        d.confidence
    );
    (bpm_err, same, step_bpm, step_ms, bar1_ms)
}

#[test]
fn the_synthetic_error_table_stays_within_half_a_bpm() {
    println!("| train | bpm | error | steps found/known | step bpm error | step time error ms | bar 1 error ms | jitter ms | confidence |");
    println!("|---|---|---|---|---|---|---|---|---|");
    let click = Train::default();
    let human = Train {
        jitter_ms: 15.0,
        drop: 0.3,
        seed: 3,
        ..Train::default()
    };
    let swing = Train {
        swing: 2.0 / 3.0,
        jitter_ms: 8.0,
        ..Train::default()
    };
    let mut rows = Vec::new();
    for bpm in [60.0, 80.0, 100.0, 120.0, 140.0, 160.0, 180.0, 200.0] {
        rows.push(row(&format!("rock {bpm} click"), &click, &[rock(48, bpm)]));
        rows.push(row(
            &format!("rock {bpm} jitter 15 ms, 30 % dropped"),
            &human,
            &[rock(48, bpm)],
        ));
    }
    for bpm in [90.0, 140.0] {
        let s = Section {
            bars: 48,
            bpm,
            beats: 4,
            kick: "x.x.....x.x.....",
            snare: "....x.x.....x...",
        };
        rows.push(row(&format!("swing {bpm}"), &swing, &[s]));
    }
    rows.push(row("step 120 to 100", &click, &[rock(32, 120.0), rock(32, 100.0)]));
    rows.push(row("step 100 to 126", &human, &[rock(16, 100.0), rock(32, 126.0)]));
    rows.push(row(
        "steps 90, 110, 95",
        &click,
        &[rock(16, 90.0), rock(16, 110.0), rock(16, 95.0)],
    ));
    rows.push(row(
        "step 120 to 123 (2.5 %)",
        &click,
        &[rock(24, 120.0), rock(24, 123.0)],
    ));
    rows.push(row(
        "3 bars at 140 in 120 (no step by the 4-bar rule)",
        &click,
        &[rock(24, 120.0), rock(3, 140.0), rock(24, 120.0)],
    ));
    rows.push(row(
        "ballad 67, sixteenth kicks",
        &click,
        &[Section {
            bars: 40,
            bpm: 67.0,
            beats: 4,
            kick: "x.x.....x.xx..x.",
            snare: "....x.......x...",
        }],
    ));
    rows.push(row(
        "sparse kick 96, 30 % dropped",
        &human,
        &[Section {
            bars: 48,
            bpm: 96.0,
            beats: 4,
            kick: "x...............",
            snare: "....x.......x...",
        }],
    ));
    rows.push(row(
        "waltz 150",
        &click,
        &[Section {
            bars: 64,
            bpm: 150.0,
            beats: 3,
            kick: "x...........",
            snare: "....x...x...",
        }],
    ));
    for (i, r) in rows.iter().enumerate() {
        assert!(r.1, "row {i}: step count");
        if i == 22 {
            // The 3-bar excursion: one tempo cannot hold the shifted bars after it (reported).
            continue;
        }
        assert!(r.0 < 0.5, "row {i}: bpm error {}", r.0);
        assert!(r.2 < 0.5 && r.3 < 20.0, "row {i}: step {} BPM {} ms", r.2, r.3);
        assert!(r.4 < 20.0, "row {i}: bar 1 {} ms", r.4);
    }
}

// ---------------------------------------------------------------- P4-8 fix round 2

/// The distance of `t` from the nearest bar line of the grid `b0 + k * bar`.
fn off_bar(t: f64, b0: f64, bar: f64) -> f64 {
    let x = (t - b0).rem_euclid(bar);
    x.min(bar - x)
}

const PUNK_K: &str = "x.......x.......";

#[test]
fn fast_tempos_give_the_tempo_or_its_exact_half() {
    // M2: up to round 1 a 3-beat or 5-beat period won through its "backbeat" (rock 220 gave 55)
    println!("| train | detected | error | steps | top alternative |");
    println!("|---|---|---|---|---|");
    for bpm in [200.0, 205.0, 210.0, 220.0, 230.0, 240.0, 250.0, 260.0] {
        for (name, kick, jitter_ms, drop, seed) in [
            ("rock click", ROCK_K, 4.0, 0.0, 7),
            ("punk click", PUNK_K, 4.0, 0.0, 7),
            ("rock, jitter 15 ms, 30 % dropped", ROCK_K, 15.0, 0.3, 3),
        ] {
            let train = Train {
                start: 2.0,
                jitter_ms,
                drop,
                seed,
                ..Train::default()
            };
            let s = Section {
                bars: 48,
                bpm,
                beats: 4,
                kick,
                snare: ROCK_S,
            };
            let (hits, starts) = build(&train, &[s]);
            let d = detect(&hits, &params()).unwrap();
            let a = d.alternatives[0];
            println!(
                "| {name} {bpm} | {:.3} | {:.4} | {} | {} ({}) |",
                d.bpm,
                (d.bpm - bpm).abs(),
                d.steps.len(),
                a.bpm,
                a.score
            );
            let full = (d.bpm - bpm).abs() < 0.1;
            assert!(
                full || (d.bpm - bpm / 2.0).abs() < 0.05,
                "{name} {bpm}: {} BPM, alternatives {:?}",
                d.bpm,
                d.alternatives
            );
            assert!(d.steps.is_empty(), "{name} {bpm}: {:?}", d.steps);
            if full {
                let bar = 240.0 / bpm;
                assert!(
                    off_bar(d.bar1_s, starts[0], bar) < 0.02,
                    "{name} {bpm}: bar 1 {}",
                    d.bar1_s
                );
            }
        }
    }
}

#[test]
fn funk_with_ghost_notes_keeps_the_beat_in_every_seed() {
    // M3: kicks on 1, the "a" of 1, the "and" of 3 and the "e" of 4, ghost notes on six sixteenths, 8 ms jitter
    for bpm in [96.0, 100.0, 104.0] {
        for seed in 1..=9u64 {
            let s = Section {
                bars: 48,
                bpm,
                beats: 4,
                kick: "x..x......x..x..",
                snare: ".g.gx.g..g.gx..g",
            };
            let train = Train {
                jitter_ms: 8.0,
                seed,
                ..Train::default()
            };
            let (hits, starts) = build(&train, &[s]);
            let d = detect(&hits, &params()).unwrap();
            println!(
                "funk {bpm} seed {seed}: {:.3} BPM, steps {}, bar 1 off by {:.1} ms",
                d.bpm,
                d.steps.len(),
                off_bar(d.bar1_s, starts[0], 240.0 / bpm) * 1000.0
            );
            assert!((d.bpm - bpm).abs() < 0.1, "funk {bpm} seed {seed}: {} BPM", d.bpm);
            assert!(d.steps.is_empty(), "funk {bpm} seed {seed}: {:?}", d.steps);
            assert!(
                off_bar(d.bar1_s, starts[0], 240.0 / bpm) < 0.02,
                "funk {bpm} seed {seed}: bar 1 {}",
                d.bar1_s
            );
        }
    }
}

/// Kick on 1 and 3, snare on 2 and 4, 32 bars at 110 BPM from 2.0 s. `pickup`: the two beats
/// before bar 1 hold a snare fill (sixteenths) or the groove's beats 3 and 4 (kick, snare).
/// `crash`: a cymbal hit on beat 1 of every 4th bar.
fn one_and_three(pickup: &str, crash: bool) -> (Vec<Hit>, f64, f64) {
    let beat = 60.0 / 110.0;
    let bar1 = 2.0;
    let mut hits = Vec::new();
    match pickup {
        "fill" => {
            for i in 0..8 {
                hits.push(Hit::new(bar1 - 2.0 * beat + i as f64 * beat / 4.0, Role::Snare));
            }
        }
        "groove" => {
            hits.push(Hit::new(bar1 - 2.0 * beat, Role::Kick));
            hits.push(Hit::new(bar1 - beat, Role::Snare));
        }
        _ => {}
    }
    for bar in 0..32 {
        let t = bar1 + bar as f64 * 4.0 * beat;
        for b in 0..4 {
            let role = if b % 2 == 0 { Role::Kick } else { Role::Snare };
            hits.push(Hit::new(t + b as f64 * beat, role));
        }
        if crash && bar % 4 == 0 {
            hits.push(Hit::new(t + 0.002, Role::Cymbal));
        }
    }
    hits.sort_by(|a, b| a.t.total_cmp(&b.t));
    (hits, bar1, beat)
}

#[test]
fn bar_1_of_a_kick_on_1_and_3_groove_does_not_depend_on_the_first_hit() {
    // M4: up to round 1 a tie of the kicks on 1 and 3 went to whichever beat came first
    let (hits, bar1, beat) = one_and_three("none", false);
    let d = detect(&hits, &params()).unwrap();
    assert!((d.bar1_s - bar1).abs() < 0.005, "no pickup: {}", d.bar1_s);
    // a two-beat snare fill before bar 1: the fill ends on beat 1, the first kick is on beat 1
    let (hits, bar1, beat2) = one_and_three("fill", false);
    let d = detect(&hits, &params()).unwrap();
    assert!(
        (d.bar1_s - bar1).abs() < 0.005,
        "fill pickup: bar 1 {} want {bar1}",
        d.bar1_s
    );
    assert!(
        (d.bar1_alternative_s - (bar1 + 2.0 * beat2)).abs() < 0.005,
        "{}",
        d.bar1_alternative_s
    );
    assert!(
        d.bar1_backbeat > 0.9 && d.bar1_downbeat > 0.0 && d.bar1_downbeat < 0.5,
        "{d:?}"
    );
    assert!((d.bar1_confidence - d.bar1_backbeat * d.bar1_downbeat).abs() < 0.002);
    // the groove from beat 3: the first kick says beat 3, kicks tie, so bar 1 is a guess with a
    // low confidence and the right answer as the alternative ...
    let (hits, bar1, _) = one_and_three("groove", false);
    let d = detect(&hits, &params()).unwrap();
    assert!(d.bar1_confidence < 0.3, "{}", d.bar1_confidence);
    let picked = [d.bar1_s, d.bar1_alternative_s];
    assert!(
        picked.iter().any(|t| off_bar(*t, bar1, 4.0 * beat) < 0.005),
        "{picked:?}"
    );
    // ... and a named cymbal track with a crash on beat 1 every 4 bars decides it
    let (hits, bar1, _) = one_and_three("groove", true);
    let d = detect(&hits, &params()).unwrap();
    assert!(off_bar(d.bar1_s, bar1, 4.0 * beat) < 0.005, "crash: bar 1 {}", d.bar1_s);
    assert!(d.bar1_downbeat > 0.5, "{}", d.bar1_downbeat);
}

/// A seeded groove for the bar-1 reliability table: tempo, pattern, jitter, dropped hits, a
/// pickup before bar 1, fills and crash hits drawn from the seed. Returns the hits, the time of
/// bar 1 and the beat.
fn groove(seed: u64) -> (Vec<Hit>, f64, f64) {
    let mut r = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let bpm = 70.0 + 100.0 * r.next();
    let beat = 60.0 / bpm;
    let pats = [
        ("x.......x.x.....", "....x.......x..."),
        ("x.......x.......", "....x.......x..."),
        ("x...............", "....x.......x..."),
        ("x..x......x..x..", ".g.gx.g..g.gx..g"),
        ("x.x.....x.......", "....x.......x..."),
        ("x...x...x...x...", "....x.......x..."),
        ("x.........x.....", "....x.......x..."),
        ("x.........x...x.", "....x.......x..."),
    ];
    let (kp, sp) = pats[(r.next() * pats.len() as f64) as usize % pats.len()];
    let jitter = 0.003 + 0.012 * r.next();
    let drop = 0.3 * r.next();
    let bars = 24 + (r.next() * 40.0) as i64;
    let pickup = (r.next() * 4.0) as i64;
    let fill_pickup = r.next() < 0.5;
    let fills = r.next() < 0.5;
    let crash = r.next() < 0.3;
    let bar1 = 0.5 + 4.0 * beat;
    let mut hits = Vec::new();
    let mut add = |t: f64, role: Role, v: f64, r: &mut Rng| {
        if r.next() >= drop {
            hits.push(Hit {
                v,
                ..Hit::new(t + jitter * r.normal(), role)
            });
        }
    };
    for bar in -1..bars {
        let t0 = bar1 + bar as f64 * 4.0 * beat;
        let fill = (bar == -1 && fill_pickup) || (fills && bar % 8 == 7);
        for i in 0..16usize {
            let t = t0 + i as f64 * beat / 4.0;
            if bar == -1 && (i as i64) < 4 * (4 - pickup) {
                continue;
            }
            if fill && i >= 8 {
                add(t, Role::Snare, 100.0, &mut r);
                continue;
            }
            if kp.as_bytes()[i] == b'x' {
                add(t, Role::Kick, 100.0, &mut r);
            }
            match sp.as_bytes()[i] {
                b'x' => add(t, Role::Snare, 100.0, &mut r),
                b'g' => add(t, Role::Snare, 35.0, &mut r),
                _ => {}
            }
        }
        if crash && bar >= 0 && bar % 4 == 0 {
            add(t0, Role::Cymbal, 110.0, &mut r);
        }
    }
    hits.sort_by(|a, b| a.t.total_cmp(&b.t));
    (hits, bar1, beat)
}

#[test]
fn the_bar_1_confidence_is_calibrated_on_seeded_grooves() {
    // M4: accuracy of bar 1 per confidence bin; the page shows this confidence
    let edges = [0.0, 0.2, 0.4, 0.6, 0.8, 1.01];
    let mut n = [0usize; 5];
    let mut ok = [0usize; 5];
    for seed in 1..=240u64 {
        let (hits, bar1, beat) = groove(seed);
        let d = detect(&hits, &params()).unwrap();
        let right = (d.bpm * beat / 60.0 - 1.0).abs() < 0.01 && off_bar(d.bar1_s, bar1, 4.0 * beat) < beat / 4.0;
        if !right && d.bar1_confidence >= 0.6 {
            println!(
                "WRONG seed {seed}: true {:.2} got {:.2}, bar 1 off {:.2} beats, bb {} db {} meter {:?}",
                60.0 / beat,
                d.bpm,
                (d.bar1_s - bar1).rem_euclid(4.0 * beat) / beat,
                d.bar1_backbeat,
                d.bar1_downbeat,
                d.meter
            );
        }
        let k = edges
            .windows(2)
            .position(|w| d.bar1_confidence >= w[0] && d.bar1_confidence < w[1])
            .unwrap();
        n[k] += 1;
        ok[k] += usize::from(right);
    }
    println!("| bar 1 confidence | trains | bar 1 right | accuracy |");
    println!("|---|---|---|---|");
    for k in 0..5 {
        println!(
            "| {:.1} to {:.1} | {} | {} | {:.2} |",
            edges[k],
            edges[k + 1].min(1.0),
            n[k],
            ok[k],
            ok[k] as f64 / n[k].max(1) as f64
        );
    }
    let acc = |k: usize| ok[k] as f64 / n[k].max(1) as f64;
    // a confidence of 0.6 or more is right at least 9 times in 10; under 0.4 it is a coin flip
    assert!(n[3] + n[4] >= 100, "{n:?}");
    assert!(
        acc(3) >= 0.9 && acc(4) >= 0.9,
        "bins 0.6 to 1: {} of {}, {} of {}",
        ok[3],
        n[3],
        ok[4],
        n[4]
    );
    assert!(acc(0) < 0.7 && acc(1) < 0.7, "low bins: {ok:?} of {n:?}");
}

#[test]
fn a_six_eight_train_is_not_four_four_with_full_confidence() {
    // L5: kick on 1, snare on 4 of six eighths; the beat is the dotted quarter
    let s = Section {
        bars: 48,
        bpm: 60.0,
        beats: 2,
        kick: "x.....",
        snare: "...x..",
    };
    let (hits, _) = build(&Train::default(), &[s]);
    let d = detect(&hits, &params()).unwrap();
    println!(
        "6/8: {:.2} BPM, meter {:?} confidence {} feel {:?} r2 {} r3 {} r4 {}",
        d.bpm, d.meter, d.meter_confidence, d.meter_feel, d.meter_r2, d.meter_r3, d.meter_r4
    );
    assert!(d.meter_confidence <= 0.5, "{}", d.meter_confidence);
    if (d.bpm - 60.0).abs() < 0.5 {
        assert_eq!(d.meter_feel, Some("two-beat"));
    }
    // a rock beat keeps a clear 4/4: the kick on 3+ makes the bar 4 beats long
    let (hits, _) = build(&Train::default(), &[rock(48, 120.0)]);
    let d = detect(&hits, &params()).unwrap();
    assert_eq!((d.meter, d.meter_feel), ((4, 4), None));
    assert!(d.meter_confidence > 0.5, "{}", d.meter_confidence);
}

#[test]
fn one_isolated_far_hit_does_not_move_the_fit() {
    // L6: 16 bars of rock at 120 from 10 s, 15 ms jitter, 30 % dropped; stray hits 50 ms off the
    // grid (on the beat for the fit). Up to round 1: 0.02 to 0.07 BPM and up to 20 ms of bar 1.
    let grid = |t: f64| 10.0 + ((t - 10.0) / 0.5).round() * 0.5;
    for seed in [3u64, 5, 9] {
        let train = Train {
            start: 10.0,
            jitter_ms: 15.0,
            drop: 0.3,
            seed,
            ..Train::default()
        };
        let (hits, _) = build(&train, &[rock(16, 120.0)]);
        let base = detect(&hits, &params()).unwrap();
        let last = hits.last().unwrap().t;
        for off in [0.05, -0.05] {
            for (what, stray) in [
                (
                    "a kick 8 s before the song",
                    vec![Hit::new(grid(2.0) + off, Role::Kick)],
                ),
                (
                    "a kick and a snare 20 s after the song",
                    vec![
                        Hit::new(grid(last + 20.0) + off, Role::Kick),
                        Hit::new(grid(last + 20.5) + off, Role::Snare),
                    ],
                ),
            ] {
                let mut h = hits.clone();
                h.extend(stray);
                let d = detect(&h, &params()).unwrap();
                let (db, b1) = (d.bpm - base.bpm, (d.bar1_s - base.bar1_s) * 1000.0);
                println!("seed {seed}, {what}, {off} s off: {db:.4} BPM, bar 1 moved {b1:.2} ms");
                assert!(db.abs() < 0.005, "seed {seed}, {what}, {off}: {db} BPM");
                assert!(b1.abs() < 1.0, "seed {seed}, {what}, {off}: bar 1 {b1} ms");
            }
        }
    }
}

#[test]
fn gated_alternatives_carry_their_real_score() {
    // L7: an alternative under the comb gate keeps the score it would have without the gate
    let mut seen = 0;
    for bpm in [60.0, 80.0, 100.0, 120.0, 140.0, 160.0, 180.0, 200.0] {
        let (hits, _) = build(&Train::default(), &[rock(48, bpm)]);
        let d = detect(&hits, &params()).unwrap();
        for a in d.alternatives.iter().filter(|a| a.gated) {
            assert!(a.score > 0.0 && a.comb > 0.0 && a.comb < 0.6, "{bpm}: {a:?}");
            seen += 1;
        }
        for a in d.alternatives.iter().filter(|a| !a.gated) {
            assert!(a.comb >= 0.6, "{bpm}: {a:?}");
        }
    }
    assert!(seen > 0, "no gated alternative in the rock trains");
}

#[test]
fn a_tempo_within_its_error_bar_of_a_whole_bpm_is_suggested_as_that() {
    // R: a played 126 is suggested as 126, not 125.99x
    let human = Train {
        jitter_ms: 15.0,
        drop: 0.3,
        seed: 3,
        ..Train::default()
    };
    let (hits, _) = build(&human, &[rock(48, 126.0)]);
    let d = detect(&hits, &params()).unwrap();
    assert_eq!(d.map.bpm, 126.0, "fitted {} error bar {}", d.bpm, d.bpm_error_bar);
    assert!((d.bpm - 126.0).abs() <= d.bpm_error_bar && d.bpm != 126.0);
    // a tempo off the whole numbers still suggests the nearest whole BPM (#472); the fit
    // stays in `d.bpm` as a diagnostic
    let (hits, _) = build(&Train::default(), &[rock(48, 125.37)]);
    let d = detect(&hits, &params()).unwrap();
    assert_eq!(d.map.bpm, 125.0, "{}", d.map.bpm);
    assert!((d.bpm - 125.37).abs() < 0.005, "{}", d.bpm);
    // steps too: 100 to 126
    let (hits, _) = build(&human, &[rock(16, 100.0), rock(32, 126.0)]);
    let d = detect(&hits, &params()).unwrap();
    assert_eq!((d.map.bpm, d.map.steps.len()), (100.0, 1));
    assert_eq!(d.map.steps[0].bpm, 126.0);
}

#[test]
fn hits_after_the_end_are_dropped_and_a_far_span_is_refused() {
    // M1: one kick at 1e7 s made the tracker walk 2e7 beats (812 MB, 2 s)
    let (hits, _) = build(&Train::default(), &[rock(48, 120.0)]);
    let base = detect(&hits, &params()).unwrap();
    let mut far = hits.clone();
    far.push(Hit::new(1e7, Role::Kick));
    let with_end = DetectParams {
        duration_s: Some(120.0),
        ..params()
    };
    let d = detect(&far, &with_end).unwrap();
    assert_eq!((d.late, d.isolated), (1, 0));
    assert_eq!(d.map, base.map);
    // without a duration the lone far hit is isolated and left out
    let t = std::time::Instant::now();
    let d = detect(&far, &params()).unwrap();
    assert_eq!((d.late, d.isolated), (0, 1));
    assert_eq!(d.map, base.map);
    assert!(t.elapsed().as_secs_f64() < 5.0, "{:?}", t.elapsed());
    // two far hits are no isolated hit: the span limit refuses them at once
    let mut two = hits.clone();
    two.extend([Hit::new(1e5, Role::Kick), Hit::new(1e5 + 0.5, Role::Snare)]);
    let e = detect(&two, &params()).unwrap_err();
    assert!(e.contains("span"), "{e}");
    assert_eq!(detect(&two, &with_end).unwrap().late, 2);
}
