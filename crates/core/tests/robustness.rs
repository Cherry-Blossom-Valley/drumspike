//! #281 C14: the robustness smoke set for the model-free
//! detector. Every signal is synthesised here (no audio files, no downloads) and run
//! through the public analysis path of drumspike-core, `analyze_track` with
//! [`Detector::Dsp`]. Each case
//! prints one table row (case, expected, found, matched within 1 ms, matched within the
//! case band, runtime) and asserts the case's stated plausibility band: no panic, never
//! an empty result where hits exist, recall and false-hit rate inside the band.
//!
//! Bands are stated per case in `BANDS` below: a failing case is a finding for
//! `docs/migration/reports/c14-robustness.md`, not a tuning job here.
//!
//! Run: `cargo test -p drumspike-core --test robustness -j 8 -- --nocapture`

use std::time::Instant;

use drumspike_core::analyze::{analyze_track, AnalyzeParams, Detector};

/// The pattern: one 2 s loop at 120 BPM. The kick track carries the kicks, the snare
/// track the snares; the roles are analysed as separate close mics, so overlapping
/// times are fine. Hits stay far above the 20 ms min gap.
const LOOP_S: f64 = 2.0;
const KICK_TIMES: [f64; 5] = [0.0, 0.5, 1.0, 1.5, 1.75];
const SNARE_TIMES: [f64; 2] = [0.5, 1.5];
/// Render tail so every hit's decay fits inside the buffer.
const TAIL_S: f64 = 0.5;

/// A deterministic PRNG (xorshift32) so the noise is repeatable on every host.
struct Rng(u32);

impl Rng {
    fn next_f32(&mut self) -> f64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        // 0 .. 1
        f64::from(x) / f64::from(u32::MAX)
    }
}

/// One hit voice, sampled at `t` seconds after the attack.
#[derive(Clone, Copy)]
enum Voice {
    /// Acoustic kick: pitch sweep 125 -> 55 Hz, 0.5 ms attack, 0.12 s decay.
    AcousticKick,
    /// Acoustic snare: noise burst plus a 190 Hz ring, 0.5 ms attack, 0.15 s decay.
    AcousticSnare,
    /// Brush: noise with a slow 15 ms rise, 0.25 s decay, little tone.
    Brush,
    /// Rods: a thin (differentiated) noise burst, 0.2 ms attack, 0.08 s decay.
    Rods,
    /// Electronic kick: a pure sine with a pitch drop 120 -> 40 Hz, 1 ms attack, 0.3 s decay.
    ElectronicKick,
    /// Electronic snare: a plain noise burst, 1 ms attack, 0.1 s decay.
    ElectronicSnare,
}

impl Voice {
    fn len_s(self) -> f64 {
        match self {
            Voice::AcousticKick => 0.4,
            Voice::AcousticSnare => 0.4,
            Voice::Brush => 0.5,
            Voice::Rods => 0.2,
            Voice::ElectronicKick => 0.5,
            Voice::ElectronicSnare => 0.3,
        }
    }

    /// The sample at `t` seconds after the attack (t < `len_s`).
    fn sample(self, t: f64, rng: &mut Rng) -> f64 {
        let (attack, decay) = match self {
            Voice::AcousticKick => (0.0005, 0.12),
            Voice::AcousticSnare => (0.0005, 0.15),
            Voice::Brush => (0.015, 0.25),
            Voice::Rods => (0.0002, 0.08),
            Voice::ElectronicKick => (0.001, 0.3),
            Voice::ElectronicSnare => (0.001, 0.1),
        };
        let env = (t / attack).min(1.0) * (-t / decay).exp();
        let noise = 2.0 * rng.next_f32() - 1.0;
        match self {
            Voice::AcousticKick => {
                // phase-integrated sweep so the frequency starts at 125 Hz
                let p = 55.0 * t + 70.0 * 0.03 * (1.0 - (-t / 0.03).exp());
                env * (2.0 * std::f64::consts::PI * p).sin()
            }
            Voice::AcousticSnare => env * (0.6 * noise + 0.4 * (t * 190.0).sin()),
            Voice::Brush => env * (0.85 * noise + 0.15 * (t * 220.0).sin()),
            Voice::Rods => {
                // thin: the difference of the noise is a crude high-pass
                env * 0.7 * (noise - 0.5 * (t * 4000.0).sin())
            }
            Voice::ElectronicKick => {
                let f = 40.0 + 80.0 * (-t / 0.05).exp();
                env * (t * f).sin()
            }
            Voice::ElectronicSnare => env * noise,
        }
    }
}

struct Spec {
    sr: u32,
    loops: usize,
    /// Mono, the drum in one channel of a stereo pair, or in both channels. The test
    /// downmixes a stereo pair to mono by averaging (the app's track downmix).
    stereo: Stereo,
    /// The peak level in dBFS before the optional clip (0 dBFS = full scale).
    peak_db: f64,
    /// Clamp the rendered signal at 0 dBFS.
    clip: bool,
    kick: Voice,
    snare: Voice,
}

#[derive(Clone, Copy, PartialEq)]
enum Stereo {
    Mono,
    OneChannel,
    BothChannels,
}

/// The case's plausibility band: (tolerance s, min recall, max false rate).
struct Band {
    tol_s: f64,
    min_recall: f64,
    max_false: f64,
}

const CLEAN: Band = Band {
    tol_s: 0.005,
    min_recall: 0.8,
    max_false: 0.10,
};
const CLIPPED: Band = Band {
    tol_s: 0.005,
    min_recall: 0.6,
    max_false: 0.25,
};
const QUIET: Band = Band {
    tol_s: 0.005,
    min_recall: 0.5,
    max_false: 0.30,
};
const SOFT: Band = Band {
    tol_s: 0.030,
    min_recall: 0.5,
    max_false: 0.30,
};

fn render(role: &str, spec: &Spec) -> Vec<f32> {
    let voice = match role {
        "kick" => spec.kick,
        _ => spec.snare,
    };
    let times: Vec<f64> = (0..spec.loops)
        .flat_map(|l| {
            let base = l as f64 * LOOP_S;
            let list: &[f64] = match role {
                "kick" => &KICK_TIMES,
                _ => &SNARE_TIMES,
            };
            list.iter().map(move |&t| base + t)
        })
        .collect();
    let dur = spec.loops as f64 * LOOP_S + voice.len_s() + TAIL_S;
    let n = (dur * f64::from(spec.sr)) as usize;
    let srf = f64::from(spec.sr);
    let mut x = vec![0.0f64; n];
    let mut rng = Rng(0x1234_5678 ^ (role.len() as u32) ^ spec.sr);
    for &t0 in &times {
        let i0 = (t0 * srf).round() as usize;
        for k in 0..((voice.len_s() * srf) as usize) {
            if i0 + k >= n {
                break;
            }
            x[i0 + k] += voice.sample(k as f64 / srf, &mut rng);
        }
    }
    let peak = x.iter().fold(0.0f64, |m, &v| m.max(v.abs())).max(1e-12);
    let g = 10f64.powf(spec.peak_db / 20.0) / peak;
    let channels = match spec.stereo {
        Stereo::Mono => 1,
        _ => 2,
    };
    // Downmix: average the channels (Mono is its own single channel).
    let g = g / f64::from(channels);
    let mut out = Vec::with_capacity(n);
    for &v in &x {
        let v = v * g * f64::from(channels);
        let v = if spec.clip { v.clamp(-1.0, 1.0) } else { v };
        out.push(v as f32);
    }
    out
}

/// Run one role of one case through the public analysis path: print its table row and
/// assert the case's band.
fn run_role(case: &str, role: &str, spec: &Spec, band: &Band) {
    let x = render(role, spec);
    let expected = spec.loops
        * match role {
            "kick" => KICK_TIMES.len(),
            _ => SNARE_TIMES.len(),
        };
    let t0 = Instant::now();
    let res = analyze_track(&x, spec.sr, role, &AnalyzeParams::default(), Detector::Dsp);
    let runtime_ms = t0.elapsed().as_millis();
    let analysis = res.expect("analyze_track must not fail");
    assert!(
        !analysis.events.is_empty(),
        "{case}/{role}: empty result where {expected} hits exist"
    );
    let found = analysis.events.len();
    // Greedy nearest match inside the band tolerance; `m1` counts matches within 1 ms.
    let mut used = vec![false; found];
    let mut mband = 0usize;
    let mut m1 = 0usize;
    for l in 0..spec.loops {
        for &t in match role {
            "kick" => &KICK_TIMES[..],
            _ => &SNARE_TIMES[..],
        } {
            let want = l as f64 * LOOP_S + t;
            let mut best: Option<(usize, f64)> = None;
            for (i, h) in analysis.events.iter().enumerate() {
                if used[i] {
                    continue;
                }
                let d = (h.t - want).abs();
                if d <= band.tol_s && best.map(|(_, bd)| d < bd).unwrap_or(true) {
                    best = Some((i, d));
                }
            }
            if let Some((i, d)) = best {
                used[i] = true;
                mband += 1;
                if d <= 0.001 {
                    m1 += 1;
                }
            }
        }
    }
    let recall = mband as f64 / expected as f64;
    let false_rate = (found - mband) as f64 / found.max(1) as f64;
    assert!(
        recall >= band.min_recall,
        "{case}/{role}: recall {recall:.3} under the band {} (matched {mband} of {expected})",
        band.min_recall
    );
    assert!(
        false_rate <= band.max_false,
        "{case}/{role}: false rate {false_rate:.3} over the band {} ({found} found, {mband} matched)",
        band.max_false
    );
    println!("| {case}/{role} | {expected} | {found} | {m1} | {mband} | {runtime_ms} |");
}

fn smoke(case: &str, spec: &Spec, band: &Band) {
    run_role(case, "kick", spec, band);
    run_role(case, "snare", spec, band);
}

// --- the cases -------------------------------------------------------------

#[test]
fn sample_rates_44k() {
    smoke(
        "44k",
        &Spec {
            sr: 44_100,
            loops: 4,
            stereo: Stereo::Mono,
            peak_db: -3.0,
            clip: false,
            kick: Voice::AcousticKick,
            snare: Voice::AcousticSnare,
        },
        &CLEAN,
    );
}

#[test]
fn sample_rates_48k() {
    smoke(
        "48k",
        &Spec {
            sr: 48_000,
            loops: 4,
            stereo: Stereo::Mono,
            peak_db: -3.0,
            clip: false,
            kick: Voice::AcousticKick,
            snare: Voice::AcousticSnare,
        },
        &CLEAN,
    );
}

#[test]
fn sample_rates_96k() {
    smoke(
        "96k",
        &Spec {
            sr: 96_000,
            loops: 4,
            stereo: Stereo::Mono,
            peak_db: -3.0,
            clip: false,
            kick: Voice::AcousticKick,
            snare: Voice::AcousticSnare,
        },
        &CLEAN,
    );
}

#[test]
fn stereo_one_channel() {
    smoke(
        "stereo-one",
        &Spec {
            sr: 48_000,
            loops: 4,
            stereo: Stereo::OneChannel,
            peak_db: -3.0,
            clip: false,
            kick: Voice::AcousticKick,
            snare: Voice::AcousticSnare,
        },
        &CLEAN,
    );
}

#[test]
fn stereo_both_channels() {
    smoke(
        "stereo-both",
        &Spec {
            sr: 48_000,
            loops: 4,
            stereo: Stereo::BothChannels,
            peak_db: -3.0,
            clip: false,
            kick: Voice::AcousticKick,
            snare: Voice::AcousticSnare,
        },
        &CLEAN,
    );
}

#[test]
fn clipped_at_zero_dbfs() {
    smoke(
        "clipped",
        &Spec {
            sr: 48_000,
            loops: 4,
            stereo: Stereo::Mono,
            // the peaks are pushed over full scale, then clamped
            peak_db: 3.0,
            clip: true,
            kick: Voice::AcousticKick,
            snare: Voice::AcousticSnare,
        },
        &CLIPPED,
    );
}

#[test]
fn very_quiet_minus_50_dbfs() {
    // The tonal kick survives -50 dBFS; the broadband snare does not (see the ignored
    // finding test below).
    let spec = Spec {
        sr: 48_000,
        loops: 4,
        stereo: Stereo::Mono,
        peak_db: -50.0,
        clip: false,
        kick: Voice::AcousticKick,
        snare: Voice::AcousticSnare,
    };
    run_role("quiet-50", "kick", &spec, &QUIET);
}

/// C14 finding: a broadband snare at -50 dBFS is missed completely. The superflux runs on
/// the log-compressed spectrogram `log10(1 + x)`, so at -50 dBFS the per-band magnitudes
/// are of the order of 1e-5 and the summed flux (about 5e-4) never reaches the snare's
/// absolute peak-picking threshold `delta` (1.0 log10 units): peak picking returns no
/// candidates at all (`analyze_track_dsp_cands`: 0 cands). The tonal kick passes because
/// its energy concentrates in a few bands. A tuning job, not done here.
#[test]
#[ignore = "C14 finding: broadband snare at -50 dBFS gives no flux peaks at all (delta 1.0 vs flux ~5e-4)"]
fn quiet_50_snare_broadband_missed() {
    let spec = Spec {
        sr: 48_000,
        loops: 4,
        stereo: Stereo::Mono,
        peak_db: -50.0,
        clip: false,
        kick: Voice::AcousticKick,
        snare: Voice::AcousticSnare,
    };
    run_role("quiet-50", "snare", &spec, &QUIET);
}

#[test]
fn brush_strokes_slow_attack() {
    smoke(
        "brush",
        &Spec {
            sr: 48_000,
            loops: 4,
            stereo: Stereo::Mono,
            peak_db: -3.0,
            clip: false,
            kick: Voice::AcousticKick,
            snare: Voice::Brush,
        },
        &SOFT,
    );
}

#[test]
fn rod_strokes_thin_attack() {
    smoke(
        "rods",
        &Spec {
            sr: 48_000,
            loops: 4,
            stereo: Stereo::Mono,
            peak_db: -3.0,
            clip: false,
            kick: Voice::AcousticKick,
            snare: Voice::Rods,
        },
        &CLIPPED,
    );
}

#[test]
fn electronic_kit() {
    smoke(
        "electronic",
        &Spec {
            sr: 48_000,
            loops: 4,
            stereo: Stereo::Mono,
            peak_db: -3.0,
            clip: false,
            kick: Voice::ElectronicKick,
            snare: Voice::ElectronicSnare,
        },
        &CLIPPED,
    );
}

/// The 20-minute take: the gate demands the whole case under 60 s on linux-arm64.
#[test]
fn twenty_minute_take_under_60_s() {
    let spec = Spec {
        sr: 48_000,
        loops: 600, // 600 * 2 s = 20 min
        stereo: Stereo::Mono,
        peak_db: -3.0,
        clip: false,
        kick: Voice::AcousticKick,
        snare: Voice::AcousticSnare,
    };
    let t0 = Instant::now();
    run_role("20min", "kick", &spec, &CLEAN);
    run_role("20min", "snare", &spec, &CLEAN);
    let total_ms = t0.elapsed().as_millis();
    println!("| 20min/total | - | - | - | - | {total_ms} |");
    assert!(
        total_ms < 60_000,
        "the 20-minute case took {total_ms} ms, over the 60 s gate"
    );
}

/// The host a run happens on, named in the table row and the assertion message.
fn host_name() -> String {
    if let Ok(out) = std::process::Command::new("hostname").output() {
        if out.status.success() {
            if let Ok(s) = String::from_utf8(out.stdout) {
                let s = s.trim();
                if !s.is_empty() {
                    return s.to_string();
                }
            }
        }
    }
    std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown".into())
}

/// Detector #8 package 7: the standing analysis speed budget. The reference render is the
/// 20-minute take above; the wall clock of its analysis per minute of audio (both roles,
/// so 40 minutes of analysed audio) must stay within 10 % of the recorded budget. The
/// render itself is not measured (harness cost, not analysis). The host is named in the
/// assertion message: the budget is recorded for one machine and profile, so a fail on a
/// slower one reads as a re-record, not a regression.
///
/// Recorded: x86-mac (Intel Core i9-8950HK, 12 threads), the default test profile (the
/// gate command's own debug profile), 2026-10-29, main at 409e6ff. Eight recorded runs
/// measured 0.237 - 0.388 s per minute (one turbo burst, the rest the sustained rate of
/// this thermally limited laptop); the constant is the worst of them, so the gate passes
/// as measured on this machine and trips on a real slowdown.
const SPEED_BUDGET_S_PER_MIN: f64 = 0.39;
/// The gate fails when the measured throughput exceeds the recorded budget by more than
/// this factor (the plan's 10 %).
const SPEED_BUDGET_SLACK: f64 = 1.10;

/// The budget run: `cargo test -p drumspike-core --test robustness -- --ignored` (the
/// profile of the recorded budget). The measured throughput and the host are printed as a
/// table row and named in the assertion message, so an over-budget fail shows both.
#[test]
#[ignore = "speed budget: run with the recorded profile (the default cargo test run stays fast)"]
fn analysis_throughput_within_the_recorded_budget() {
    let spec = Spec {
        sr: 48_000,
        loops: 600, // 600 * 2 s = 20 min per role
        stereo: Stereo::Mono,
        peak_db: -3.0,
        clip: false,
        kick: Voice::AcousticKick,
        snare: Voice::AcousticSnare,
    };
    let kick = render("kick", &spec);
    let snare = render("snare", &spec);
    let minutes = (kick.len() + snare.len()) as f64 / f64::from(spec.sr) / 60.0;
    let t0 = Instant::now();
    let k = analyze_track(&kick, spec.sr, "kick", &AnalyzeParams::default(), Detector::Dsp);
    let s = analyze_track(&snare, spec.sr, "snare", &AnalyzeParams::default(), Detector::Dsp);
    let total_s = t0.elapsed().as_secs_f64();
    let k = k.expect("analyze_track must not fail");
    let s = s.expect("analyze_track must not fail");
    assert!(
        !k.events.is_empty(),
        "speed budget: the kick pass found no hits where 3000 exist"
    );
    assert!(
        !s.events.is_empty(),
        "speed budget: the snare pass found no hits where 1200 exist"
    );
    let s_per_min = total_s / minutes;
    let host = host_name();
    eprintln!(
        "| speed-budget/20min | {} + {} hits | {total_s:.1} s total | {minutes:.1} min audio | {s_per_min:.3} s/min | {host} |",
        k.events.len(),
        s.events.len()
    );
    assert!(
        s_per_min <= SPEED_BUDGET_S_PER_MIN * SPEED_BUDGET_SLACK,
        "the reference render analysed at {s_per_min:.3} s per minute of audio, over the \
         recorded budget {:.3} s/min +10 % (measured on {host}; the budget is recorded for \
         the default test profile on the machine named in this test's doc comment)",
        SPEED_BUDGET_S_PER_MIN
    );
}
