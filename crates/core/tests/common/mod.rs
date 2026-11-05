//! The shared synthetic renderer of the drumspike-core integration tests: the voices,
//! the spec and the renderer of `robustness.rs`, used there and by `invariance.rs`.
//! Every signal is synthesised here (no audio files, no downloads).

#![allow(dead_code)]

/// The pattern: one 2 s loop at 120 BPM. The kick track carries the kicks, the snare
/// track the snares; the roles are analysed as separate close mics, so overlapping
/// times are fine. Hits stay far above the 20 ms min gap.
pub const LOOP_S: f64 = 2.0;
pub const KICK_TIMES: [f64; 5] = [0.0, 0.5, 1.0, 1.5, 1.75];
pub const SNARE_TIMES: [f64; 2] = [0.5, 1.5];
/// Render tail so every hit's decay fits inside the buffer.
pub const TAIL_S: f64 = 0.5;

/// A deterministic PRNG (xorshift32) so the noise is repeatable on every host.
pub struct Rng(pub u32);

impl Rng {
    pub fn next_f32(&mut self) -> f64 {
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
pub enum Voice {
    /// Acoustic kick: pitch sweep 125 -> 55 Hz, 0.5 ms attack, 0.12 s decay.
    AcousticKick,
    /// Acoustic snare: noise burst plus a 190 Hz ring, 0.5 ms attack, 0.15 s decay.
    AcousticSnare,
    /// Acoustic tom: a lower pitch sweep 180 -> 90 Hz with a longer body, 0.5 ms
    /// attack, 0.25 s decay.
    AcousticTom,
    /// Hi-hat: a thin metallic burst (differentiated noise against a 7 kHz ring),
    /// 0.2 ms attack, 0.06 s decay.
    Hat,
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
    pub fn len_s(self) -> f64 {
        match self {
            Voice::AcousticKick => 0.4,
            Voice::AcousticSnare => 0.4,
            Voice::AcousticTom => 0.5,
            Voice::Hat => 0.2,
            Voice::Brush => 0.5,
            Voice::Rods => 0.2,
            Voice::ElectronicKick => 0.5,
            Voice::ElectronicSnare => 0.3,
        }
    }

    /// The sample at `t` seconds after the attack (t < `len_s`).
    pub fn sample(self, t: f64, rng: &mut Rng) -> f64 {
        let (attack, decay) = match self {
            Voice::AcousticKick => (0.0005, 0.12),
            Voice::AcousticSnare => (0.0005, 0.15),
            Voice::AcousticTom => (0.0005, 0.25),
            Voice::Hat => (0.0002, 0.06),
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
            Voice::AcousticTom => {
                // phase-integrated sweep so the frequency starts at 180 Hz, plus a
                // little noise for the stick contact
                let p = 90.0 * t + 90.0 * 0.03 * (1.0 - (-t / 0.03).exp());
                env * (0.9 * (2.0 * std::f64::consts::PI * p).sin() + 0.1 * noise)
            }
            Voice::Hat => {
                // thin metallic: the differentiated noise is a crude high-pass
                env * 0.8 * (noise - 0.6 * (t * 7000.0).sin())
            }
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

pub struct Spec {
    pub sr: u32,
    pub loops: usize,
    /// Mono, the drum in one channel of a stereo pair, or in both channels. The test
    /// downmixes a stereo pair to mono by averaging (the app's track downmix).
    pub stereo: Stereo,
    /// The peak level in dBFS before the optional clip (0 dBFS = full scale).
    pub peak_db: f64,
    /// Clamp the rendered signal at 0 dBFS.
    pub clip: bool,
    pub kick: Voice,
    pub snare: Voice,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Stereo {
    Mono,
    OneChannel,
    BothChannels,
}

pub fn render(role: &str, spec: &Spec) -> Vec<f32> {
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

/// Render an arbitrary hit list (time in seconds, voice) into `total_s` seconds of
/// mono audio, normalised so the signal's peak sits at `peak_db` dBFS. Used by
/// `invariance.rs`, which composes and transforms whole signals.
pub fn render_hits(sr: u32, hits: &[(f64, Voice)], total_s: f64, peak_db: f64) -> Vec<f32> {
    let n = (total_s * f64::from(sr)) as usize;
    let srf = f64::from(sr);
    let mut x = vec![0.0f64; n];
    let mut rng = Rng(0x1234_5678);
    for &(t0, voice) in hits {
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
