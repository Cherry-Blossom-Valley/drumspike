//! #281 C15: the detector speed bound. A 5-minute,
//! 8-track kit (kick, snare, 3 toms, hat, 2 overheads) at 48 kHz, synthesised
//! deterministically like `tests/robustness.rs` (xorshift32 noise, fixed hit
//! times), runs through the public analysis path `analyze_track` with
//! [`Detector::Dsp`] on all 8 tracks.
//!
//! Threading: the app analyses one track at a time on the single job thread
//! (`crates/app/src/jobs.rs`: one worker per `analyze_start`, one analysis at a
//! time in the whole app), so this test runs the 8 tracks sequentially and the
//! wall times are per track plus the total.
//!
//! Run: `cargo test -p drumspike-core --release --test speed -- --ignored --nocapture`
//! (ignored by default: the default `cargo test -p drumspike-core` run stays fast).

use std::time::Instant;

use drumspike_core::analyze::{AnalyzeParams, Detector, analyze_track};

/// The session: 5 minutes at 120 BPM (150 bars of a 2 s loop).
const LOOPS: usize = 150;
const LOOP_S: f64 = 2.0;
const SR: u32 = 48_000;
/// The gate: the whole 8-track session must analyse under this wall time in a
/// release build on linux-arm64.
const TOTAL_LIMIT_MS: u128 = 10_000;
/// Render tail so every hit's decay fits inside the buffer.
const TAIL_S: f64 = 2.0;

/// The kit: one track per entry. The roles are the app's close-mic roles; the
/// two overheads are `cymbal` roles with a ride voice.
const KIT: [(&str, Voice, &[f64]); 8] = [
    ("kick", Voice::Kick, &[0.0, 0.5, 1.0, 1.5, 1.75]),
    ("snare", Voice::Snare, &[0.5, 1.5]),
    ("tom1", Voice::Tom(220.0, 110.0), &[0.75, 1.25]),
    ("tom2", Voice::Tom(165.0, 90.0), &[0.25, 1.625]),
    ("tom3", Voice::Tom(130.0, 75.0), &[1.875]),
    ("hat", Voice::Hat, &[0.0, 0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 1.75]),
    ("oh-l", Voice::Ride, &[0.0, 1.0, 1.75]),
    ("oh-r", Voice::Ride, &[0.0, 1.0, 1.75]),
];

/// A deterministic PRNG (xorshift32) so the noise is repeatable on every host.
struct Rng(u32);

impl Rng {
    fn next_f64(&mut self) -> f64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        f64::from(x) / f64::from(u32::MAX)
    }
}

/// One hit voice, sampled at `t` seconds after the attack.
#[derive(Clone, Copy)]
enum Voice {
    /// Acoustic kick: pitch sweep 125 -> 55 Hz, 0.5 ms attack, 0.12 s decay.
    Kick,
    /// Acoustic snare: noise burst plus a 190 Hz ring, 0.5 ms attack, 0.15 s decay.
    Snare,
    /// Rack/floor tom: a pitch sweep from `f0` to `f1`, 0.5 ms attack, 0.25 s decay.
    Tom(f64, f64),
    /// Hat: a short noise burst with a crude high-pass (the difference of the noise),
    /// 0.2 ms attack, 0.05 s decay.
    Hat,
    /// Overhead (ride): a long noise wash with a slow 5 ms rise, 1.2 s decay.
    Ride,
}

impl Voice {
    fn len_s(self) -> f64 {
        match self {
            Voice::Kick => 0.4,
            Voice::Snare => 0.4,
            Voice::Tom(..) => 0.6,
            Voice::Hat => 0.15,
            Voice::Ride => 1.8,
        }
    }

    fn sample(self, t: f64, rng: &mut Rng) -> f64 {
        let (attack, decay) = match self {
            Voice::Kick => (0.0005, 0.12),
            Voice::Snare => (0.0005, 0.15),
            Voice::Tom(..) => (0.0005, 0.25),
            Voice::Hat => (0.0002, 0.05),
            Voice::Ride => (0.005, 1.2),
        };
        let env = (t / attack).min(1.0) * (-t / decay).exp();
        let noise = 2.0 * rng.next_f64() - 1.0;
        match self {
            Voice::Kick => {
                let p = 55.0 * t + 70.0 * 0.03 * (1.0 - (-t / 0.03).exp());
                env * (2.0 * std::f64::consts::PI * p).sin()
            }
            Voice::Snare => env * (0.6 * noise + 0.4 * (t * 190.0).sin()),
            Voice::Tom(f0, f1) => {
                let p = f1 * t + (f0 - f1) * 0.03 * (1.0 - (-t / 0.03).exp());
                env * (0.7 * (2.0 * std::f64::consts::PI * p).sin() + 0.3 * noise)
            }
            Voice::Hat => env * 0.7 * (noise - 0.5 * (t * 8000.0).sin()),
            Voice::Ride => env * (0.7 * noise + 0.3 * (t * 3000.0).sin()),
        }
    }
}

/// Render one track: the loop pattern repeated `LOOPS` times, peak-normalised to
/// -3 dBFS, mono f32 at 48 kHz.
fn render(role: &str, voice: Voice, times: &[f64]) -> Vec<f32> {
    let srf = f64::from(SR);
    let dur = LOOPS as f64 * LOOP_S + voice.len_s() + TAIL_S;
    let n = (dur * srf) as usize;
    let mut x = vec![0.0f64; n];
    let mut rng = Rng(0x1234_5678 ^ (role.len() as u32) ^ SR);
    for l in 0..LOOPS {
        let base = l as f64 * LOOP_S;
        for &t in times {
            let i0 = ((base + t) * srf).round() as usize;
            for k in 0..(voice.len_s() * srf) as usize {
                if i0 + k >= n {
                    break;
                }
                x[i0 + k] += voice.sample(k as f64 / srf, &mut rng);
            }
        }
    }
    let peak = x.iter().fold(0.0f64, |m, &v| m.max(v.abs())).max(1e-12);
    let g = 10f64.powf(-3.0 / 20.0) / peak;
    x.iter().map(|&v| (v * g) as f32).collect()
}

#[test]
#[ignore = "speed gate: run in release with --ignored (the default cargo test run stays fast)"]
fn five_minute_8_track_session_under_10_s() {
    let mut rows: Vec<(String, u128)> = Vec::new();
    let total = Instant::now();
    for (role, voice, times) in KIT {
        let x = render(role, voice, times);
        let t0 = Instant::now();
        let res = analyze_track(&x, SR, role, &AnalyzeParams::default(), Detector::Dsp);
        let ms = t0.elapsed().as_millis();
        let analysis = res.expect("analyze_track must not fail");
        assert!(!analysis.events.is_empty(), "{role}: empty result where hits exist");
        println!(
            "| {:5} | {} hits | {:>6} ms | {} samples |",
            role,
            analysis.events.len(),
            ms,
            x.len()
        );
        rows.push((role.to_string(), ms));
    }
    let total_ms = total.elapsed().as_millis();
    println!("| TOTAL  | 8 tracks | {total_ms:>6} ms |");
    for (role, ms) in &rows {
        println!("  {role:5}: {ms:>6} ms");
    }
    println!("  total: {total_ms:>6} ms (limit {TOTAL_LIMIT_MS} ms)");
    assert!(
        total_ms <= TOTAL_LIMIT_MS,
        "5-minute 8-track session took {total_ms} ms, over the {TOTAL_LIMIT_MS} ms bound"
    );
}
