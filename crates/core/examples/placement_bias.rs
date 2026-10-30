//! Synthetic onset error of the placement (#101): one-shots at exact onset samples on a noise floor.
//! Prints the error (refined time in samples minus the true onset) per role and rate.
//! Run: cargo run --release -p drumspike-core --example placement_bias
use drumspike_core::refine::{
    refine_foot_tuned, refine_rule_peak, BackStep, FootCap, FootTune, Overrun, RefineRule, RefineWindow, FM_PEAK_DB,
};
use std::f64::consts::PI;

const ROLES: [&str; 4] = ["kick", "snare", "tom", "hat"];
const RATES: [u32; 3] = [44_100, 48_000, 96_000];
const FLOORS_DB: [f64; 2] = [-60.0, -90.0];
const POLARITIES: [f64; 2] = [1.0, -1.0];
/// Attack lengths (s) of the one-shot: 0 is a sharp start at full level, else a raised-cosine ramp
/// from the true onset sample up to full level over this length.
const ATTACKS_S: [f64; 3] = [0.0, 0.0005, 0.001];
const SEEDS: u64 = 30;
const AMP: f64 = 0.5;
/// The detector reports hits about 6 ms before the attack (refine.rs, RefineWindow docs).
const T0_BEFORE_S: f64 = 0.006;

struct Lcg(u64);

impl Lcg {
    /// Uniform in [-1, 1).
    fn uni(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
    }
}

/// One one-shot of `role` starting at sample `onset` (a sharp start at full level), on a uniform noise
/// floor of `floor_db` (rms), with first polarity `pol`. `level` is `(floor_db, pol)`.
fn shot(role: usize, sr: u32, n: usize, onset: usize, level: (f64, f64), attack_s: f64, rng: &mut Lcg) -> Vec<f32> {
    let (floor_db, pol) = level;
    let srf = sr as f64;
    let mut x = vec![0f32; n];
    let a = 10f64.powf(floor_db / 20.0) * 3f64.sqrt();
    for v in x.iter_mut() {
        *v = (a * rng.uni()) as f32;
    }
    let mut prev_noise = 0.0;
    for (k, slot) in x.iter_mut().enumerate().skip(onset) {
        let t = (k - onset) as f64 / srf;
        let v = match role {
            0 => (2.0 * PI * 60.0 * t).cos() * (-t / 0.10).exp(),
            1 => rng.uni() * (-t / 0.06).exp() + 0.3 * (2.0 * PI * 190.0 * t).cos() * (-t / 0.10).exp(),
            2 => {
                0.8 * (2.0 * PI * 120.0 * t).cos() * (-t / 0.25).exp()
                    + 0.3 * (2.0 * PI * 180.0 * t).cos() * (-t / 0.15).exp()
            }
            _ => {
                let nz = rng.uni();
                let d = nz - prev_noise;
                prev_noise = nz;
                d * (-t / 0.03).exp()
            }
        };
        let ramp = if attack_s > 0.0 && t < attack_s {
            0.5 - 0.5 * (PI * t / attack_s).cos()
        } else {
            1.0
        };
        *slot = (pol * AMP * v * ramp) as f32;
    }
    x
}

fn pct(v: &mut [f64], p: f64) -> f64 {
    v.sort_by(|a, b| a.total_cmp(b));
    let i = ((v.len() - 1) as f64 * p).round() as usize;
    v[i]
}

/// The foot of the attack (owner rule, #101): the last floor-level sample before the first sample
/// that leaves the floor. The floor is uniform noise in [-a, a], so a sample leaves it when |x| > a.
fn foot_of(x: &[f32], a: f64) -> usize {
    let dep = x
        .iter()
        .position(|v| (v.abs() as f64) > a * (1.0 + 1e-6))
        .expect("the shot leaves the floor");
    dep - 1
}

fn main() {
    // `placement_bias foot` measures the version 4 rule, `foot5` the version 5 rule (#127); no argument measures
    // the analysis rule (CURRENT).
    // `placement_bias tuned <cap samples> <keep|clamp>` measures a version 5 foot with other cap knobs (#127).
    let args: Vec<String> = std::env::args().collect();
    let rule = match args.get(1).map(String::as_str) {
        Some("foot") => RefineRule::Foot,
        Some("foot5") => RefineRule::FootCapped,
        _ => RefineRule::CURRENT,
    };
    let tuned = (args.get(1).map(String::as_str) == Some("tuned")).then(|| FootTune {
        cap: FootCap::Samples(args[2].parse().expect("cap samples")),
        overrun: if args[3] == "clamp" {
            Overrun::Clamp
        } else {
            Overrun::Keep
        },
    });
    println!(
        "placement error in samples (refined time minus the true onset, and minus the foot), rule {:?}, peak {} dB",
        rule, FM_PEAK_DB
    );
    println!(
        "pooled over floors {:?} dBFS, polarities +1/-1, {} seeds each",
        FLOORS_DB, SEEDS
    );
    println!("| rate | role | attack ms | n | onset median | p10 | p90 | foot median | p10 | p90 | early (<-1 vs foot) | moved | kept | busy | separate |");
    println!("|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|");
    for &sr in &RATES {
        let n = (sr as usize) * 3 / 5;
        for (ri, role) in ROLES.iter().enumerate() {
            for &attack in &ATTACKS_S {
                let mut errs: Vec<f64> = Vec::new();
                let mut ferrs: Vec<f64> = Vec::new();
                let (mut moved, mut kept, mut busy, mut sep) = (0, 0, 0, 0);
                for &floor in &FLOORS_DB {
                    let a = 10f64.powf(floor / 20.0) * 3f64.sqrt();
                    for &pol in &POLARITIES {
                        for seed in 0..SEEDS {
                            let mut rng = Lcg(0x9E37_79B9_7F4A_7C15 ^ ((seed + 1) * 0x1000_0000_0001));
                            let onset = (0.3 * sr as f64) as usize + (seed as usize) * 3;
                            let x = shot(ri, sr, n, onset, (floor, pol), attack, &mut rng);
                            let foot = foot_of(&x, a);
                            let t0 = onset as f64 / sr as f64 - T0_BEFORE_S;
                            let r = match tuned {
                                Some(tune) => {
                                    refine_foot_tuned(&x, t0, sr, &[], RefineWindow::DEFAULT, FM_PEAK_DB, tune)
                                }
                                None => refine_rule_peak(&x, t0, sr, &[], RefineWindow::DEFAULT, rule, FM_PEAK_DB),
                            };
                            errs.push(r.t * sr as f64 - onset as f64);
                            ferrs.push(r.t * sr as f64 - foot as f64);
                            match r.back {
                                BackStep::Moved => moved += 1,
                                BackStep::Kept | BackStep::Off => kept += 1,
                                BackStep::Busy => busy += 1,
                                BackStep::Separate => sep += 1,
                            }
                        }
                    }
                }
                let med = pct(&mut errs.clone(), 0.5);
                let p10 = pct(&mut errs.clone(), 0.1);
                let p90 = pct(&mut errs.clone(), 0.9);
                let fmed = pct(&mut ferrs.clone(), 0.5);
                let fp10 = pct(&mut ferrs.clone(), 0.1);
                let fp90 = pct(&mut ferrs.clone(), 0.9);
                let early = ferrs.iter().filter(|&&e| e < -1.0).count();
                println!(
                    "| {} | {} | {} | {} | {:+.2} | {:+.2} | {:+.2} | {:+.2} | {:+.2} | {:+.2} | {} | {} | {} | {} | {} |",
                    sr, role, attack * 1000.0, errs.len(), med, p10, p90, fmed, fp10, fp90, early, moved, kept, busy, sep
                );
            }
        }
    }
}
