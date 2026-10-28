//! Shared render of the A5 level kit (kick + acoustic snare voices, `level_calib.rs`
//! and `level_probe.rs`). Keep in sync with `robustness.rs`.

pub const LOOP_S: f64 = 2.0;
pub const KICK_TIMES: [f64; 5] = [0.0, 0.5, 1.0, 1.5, 1.75];
pub const SNARE_TIMES: [f64; 2] = [0.5, 1.5];
pub const TAIL_S: f64 = 0.5;
pub const LOOPS: usize = 4;

/// A deterministic PRNG (xorshift32), the same construction as `robustness.rs`.
pub struct Rng(pub u32);

impl Rng {
    pub fn next_f32(&mut self) -> f64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        f64::from(x) / f64::from(u32::MAX)
    }
}

/// One hit voice, sampled at `t` seconds after the attack; the acoustic pair of
/// `robustness.rs`.
#[derive(Clone, Copy)]
pub enum Voice {
    AcousticKick,
    AcousticSnare,
}

impl Voice {
    pub fn len_s(self) -> f64 {
        match self {
            Voice::AcousticKick => 0.4,
            Voice::AcousticSnare => 0.4,
        }
    }

    pub fn sample(self, t: f64, rng: &mut Rng) -> f64 {
        let (attack, decay) = match self {
            Voice::AcousticKick => (0.0005, 0.12),
            Voice::AcousticSnare => (0.0005, 0.15),
        };
        let env = (t / attack).min(1.0) * (-t / decay).exp();
        let noise = 2.0 * rng.next_f32() - 1.0;
        match self {
            Voice::AcousticKick => {
                let p = 55.0 * t + 70.0 * 0.03 * (1.0 - (-t / 0.03).exp());
                env * (2.0 * std::f64::consts::PI * p).sin()
            }
            Voice::AcousticSnare => env * (0.6 * noise + 0.4 * (t * 190.0).sin()),
        }
    }
}

pub fn render(role: &str, peak_db: f64) -> Vec<f32> {
    let voice = if role == "kick" {
        Voice::AcousticKick
    } else {
        Voice::AcousticSnare
    };
    let times: Vec<f64> = (0..LOOPS)
        .flat_map(|l| {
            let base = l as f64 * LOOP_S;
            let list: &[f64] = if role == "kick" { &KICK_TIMES } else { &SNARE_TIMES };
            list.iter().map(move |&t| base + t)
        })
        .collect();
    let sr = 48_000u32;
    let n = ((LOOPS as f64 * LOOP_S + voice.len_s() + TAIL_S) * f64::from(sr)) as usize;
    let srf = f64::from(sr);
    let mut x = vec![0.0f64; n];
    let mut rng = Rng(0x1234_5678 ^ (role.len() as u32) ^ sr);
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
    let g = 10f64.powf(peak_db / 20.0) / peak;
    x.iter().map(|&v| (v * g) as f32).collect()
}

#[allow(dead_code)]
pub fn role_bands(role: &str) -> std::ops::Range<usize> {
    let (lo, hi) = match role {
        "kick" => (20.0, 4_000.0),
        "snare" => (100.0, 300.0),
        _ => (20.0, 20_000.0),
    };
    drumspike_core::detect::onset::band_range(lo, hi)
}
