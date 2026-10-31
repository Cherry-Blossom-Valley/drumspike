//! #281 item 10: the model-free role suggestion from a track's audio (`role_from_audio`).
//!
//! Rules on band energy, spectral centroid, decay time and onset rate; no learned
//! model, no training data. Returns [`Role::Kick`], [`Role::Snare`],
//! [`Role::Tom`], [`Role::Hat`] or [`Role::Cymbal`], or `None` for anything else
//! ("other": pink noise, continuous material, nothing decisive).
//!
//! Per-hit measurement (ticket #393, the coordinator's direction): the onsets are
//! found on the energy envelope, each loud hit gets a 0-400 ms window, and every hit
//! is described by its own features ([`hit_features`]). `role_features` aggregates
//! them as medians over the loud hits (within 12 dB of the loudest onset; the mic's
//! own instrument is louder on its own mic than the bleed of the others). The key
//! per-hit features:
//!
//! - High-band (4-16 kHz) decay: the time from the early peak of the hit's 4-16 kHz
//!   envelope to -20 dB under it. A cymbal stays within about 20 dB of its peak for
//!   hundreds of ms; a snare, tom or closed hat drops by 20 dB within 100-200 ms.
//!   Plus the energy ratio 150-400 ms vs 0-50 ms of the same band.
//! - The fraction below 200 Hz: a tom's fundamental dominates its body; a snare has
//!   its body at 150-300 Hz plus a broad wire-noise band.
//! - Whole file: the energy-weighted band fractions over the active frames, and the
//!   share of active frames whose 4-16 kHz energy stays within 20 dB of the file's
//!   high-band peak (the wash between the hits of a cymbal mic).
//!
//! The `role-from-audio` dev binary (same crate) runs it over a WAV file for the
//! `tools/eval` scripts.

use rustfft::{FftPlanner, num_complex::Complex};

/// A drum role the classifier can suggest. The string is the app's role name
/// (`crates/app/src/roles.rs`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Kick drum.
    Kick,
    /// Snare drum.
    Snare,
    /// Tom.
    Tom,
    /// Hi-hat.
    Hat,
    /// Cymbal (ride, crash, overhead-like).
    Cymbal,
}

impl Role {
    /// The app's role name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Kick => "kick",
            Role::Snare => "snare",
            Role::Tom => "tom",
            Role::Hat => "hat",
            Role::Cymbal => "cymbal",
        }
    }
}

/// The features `role_from_audio` decides on (exposed for the eval report and tests).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoleFeatures {
    /// Median fraction of the per-hit spectral energy below 150 Hz.
    pub b_low: f64,
    /// Median per-hit fraction in 150-800 Hz.
    pub b_mid: f64,
    /// Median per-hit fraction in 800-4000 Hz.
    pub b_upper: f64,
    /// Median per-hit fraction in 4000-12000 Hz.
    pub b_high: f64,
    /// Median per-hit fraction above 12000 Hz.
    pub b_top: f64,
    /// Median energy-weighted spectral centroid of a hit window, Hz.
    pub centroid: f64,
    /// Inter-quartile range of the per-hit 4-12 kHz fraction across the loud hits.
    pub iqr_high: f64,
    /// Median per-hit peak-to-smoothed-mean ratio of the 80-800 Hz spectrum (a tom rings
    /// on its fundamental; a snare is noise).
    pub tonal_lo: f64,
    /// Whole-file fraction of the active-frame energy below 150 Hz.
    pub w_low: f64,
    /// Whole-file fraction in 150-800 Hz.
    pub w_mid: f64,
    /// Whole-file fraction in 800-4000 Hz.
    pub w_upper: f64,
    /// Whole-file fraction in 4000-12000 Hz.
    pub w_high: f64,
    /// Whole-file fraction above 12000 Hz.
    pub w_top: f64,
    /// Median decay time of a hit to -12 dB of its onset peak, s.
    pub decay_s: f64,
    /// Median per-hit fraction below 200 Hz over the 0-400 ms hit window (a tom's
    /// fundamental lives here, a snare's body sits above it).
    pub hit_low200: f64,
    /// Median per-hit time for the 4-16 kHz envelope to fall 20 dB from its early peak
    /// inside the hit window, s (a cymbal rings, a snare or closed hat drops fast).
    pub t20_hi: f64,
    /// Median per-hit ratio of 4-16 kHz energy, 150-400 ms over 0-50 ms after the onset.
    pub ratio_hi: f64,
    /// Share of the active frames whose 4-16 kHz energy stays within 20 dB of the
    /// file's high-band peak (the wash between the hits of a cymbal mic).
    pub hi_sustain: f64,
    /// Onsets per second (energy-envelope rises of 6 dB or more).
    pub onsets_per_s: f64,
    /// Peak-to-mean ratio of the energy envelope (crest); steady material scores low.
    pub crest: f64,
}

/// The features of one hit's 0-400 ms window (exposed for the per-hit eval).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoleHitFeatures {
    /// The hit's onset time, s.
    pub t_s: f64,
    /// The frame-envelope level at the onset, dB under the file's peak.
    pub env_db: f64,
    /// Band fractions of the window's energy: <150, 150-800, 800-4k, 4-12k, >12k.
    pub bands: [f64; 5],
    /// The window's fraction below 200 Hz.
    pub low200: f64,
    /// Energy-weighted spectral centroid of the window, Hz.
    pub centroid: f64,
    /// Time for the 4-16 kHz envelope to fall 20 dB from its early peak, s.
    pub t20_hi: f64,
    /// 4-16 kHz energy ratio, 150-400 ms over 0-50 ms after the onset.
    pub ratio_hi: f64,
    /// Peak-to-smoothed-mean ratio of the 80-800 Hz spectrum (a tom rings, a snare is
    /// noise).
    pub tonal_lo: f64,
}

const FRAME_MS: f64 = 46.4; // the frame length at 44.1 kHz (2048 samples), scaled to the rate
const HIT_WINDOW_S: f64 = 0.4; // the per-hit window after the onset (0-400 ms, #393)
const LOUD_DB: f64 = 12.0; // the hits within this of the loudest onset carry the spectrum

/// `role_from_audio`: the role the audio suggests, or `None` for anything else.
///
/// This is the hint mode of ticket #393 (the coordinator's decision): the suggestion is
/// shown only when the file name gives no role, so a wrong hint costs more than no hint.
/// A role is suggested only when its confidence reaches that role's own bar
/// ([`role_threshold`]); a role with no bar (never 90 % precise on ENST) is never
/// suggested.
#[must_use]
pub fn role_from_audio(samples: &[f32], sr: u32) -> Option<Role> {
    let (role, confidence) = role_from_audio_confidence(samples, sr);
    role.filter(|r| role_threshold(*r).is_some_and(|bar| confidence >= bar))
}

/// The per-role confidence bars of ticket #393 (the coordinator's decision 2026-10-25:
/// per-role thresholds instead of one). Each bar is the lowest threshold on a 0.05 grid
/// at which the role's suggestions reach a precision of 0.90 or more on the ENST-Drums
/// mic files, all three drummers pooled (1538 files, the vote-margin confidence of the
/// working tree at the time of the decision; the run is
/// `~/.cache/drumspike/role-from-audio/dump3.jsonl` on linux-arm64). A role that never reaches
/// 0.90 precision on that run has no bar and is never suggested.
///
/// - kick 0.05: 94 % precise (203/215), from 0.05 up.
/// - tom 0.60: 91 % precise (195/215).
/// - snare: never above 84 % at any threshold (the snare and tom votes trade places);
///   no bar.
/// - hat: never above 65 %; no bar.
/// - cymbal: the wash veto never fired on the run (0 suggestions at any threshold);
///   no bar.
#[must_use]
pub fn role_threshold(role: Role) -> Option<f64> {
    match role {
        Role::Kick => Some(0.05),
        Role::Tom => Some(0.60),
        Role::Snare | Role::Hat | Role::Cymbal => None,
    }
}

/// The suggested role with its confidence: the vote margin between the best and the
/// second role, or the wash veto strength for a cymbal. `None` has confidence 0.
#[must_use]
pub fn role_from_audio_confidence(samples: &[f32], sr: u32) -> (Option<Role>, f64) {
    if samples.is_empty() {
        return (None, 0.0);
    }
    let a = analyze(samples, sr);
    let hits = hit_list(&a);
    if hits.is_empty() {
        return (None, 0.0);
    }
    let f = features_from(&a, &hits);
    classify(&f, &hits)
}

/// The features of one clip (see the module docs).
#[must_use]
pub fn role_features(samples: &[f32], sr: u32) -> RoleFeatures {
    let f = RoleFeatures::zeroed();
    let n = samples.len();
    if n == 0 {
        return f;
    }
    let a = analyze(samples, sr);
    let hits = hit_list(&a);
    if hits.is_empty() {
        return f;
    }
    features_from(&a, &hits)
}

/// The aggregate features of a clip from its analysis and loud hits.
fn features_from(a: &Analysis, hits: &[RoleHitFeatures]) -> RoleFeatures {
    let mut f = RoleFeatures::zeroed();
    let median = |mut v: Vec<f64>| -> f64 {
        if v.is_empty() {
            return 0.0;
        }
        v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        v[v.len() / 2]
    };
    f.b_low = median(hits.iter().map(|h| h.bands[0]).collect());
    f.b_mid = median(hits.iter().map(|h| h.bands[1]).collect());
    f.b_upper = median(hits.iter().map(|h| h.bands[2]).collect());
    f.b_high = median(hits.iter().map(|h| h.bands[3]).collect());
    f.b_top = median(hits.iter().map(|h| h.bands[4]).collect());
    f.centroid = median(hits.iter().map(|h| h.centroid).collect());
    f.tonal_lo = median(hits.iter().map(|h| h.tonal_lo).collect());
    f.hit_low200 = median(hits.iter().map(|h| h.low200).collect());
    f.t20_hi = median(hits.iter().map(|h| h.t20_hi).collect());
    f.ratio_hi = median(hits.iter().map(|h| h.ratio_hi).collect());
    // IQR of the per-hit 4-12 kHz fraction (an own-instrument mic hears one spectrum).
    if hits.len() >= 4 {
        let mut v: Vec<f64> = hits.iter().map(|h| h.bands[3]).collect();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let q = |p: f64| -> f64 {
            let i = ((v.len() - 1) as f64 * p).round() as usize;
            v[i]
        };
        f.iqr_high = q(0.75) - q(0.25);
    }

    // Whole-file band fractions over the active frames (within 30 dB of the peak).
    let peak_env = a.peak_env;
    let bins = a.mags[0].len();
    let e0 = ((150.0 / a.bin_hz) as usize).min(bins - 1);
    let e1 = ((800.0 / a.bin_hz) as usize).min(bins - 1);
    let e2 = ((4000.0 / a.bin_hz) as usize).min(bins - 1);
    let e3 = ((12000.0 / a.bin_hz) as usize).min(bins - 1);
    let bin4k = e2;
    let bin16k = ((16000.0 / a.bin_hz) as usize).min(bins);
    let active_min = peak_env * 10.0_f64.powf(-30.0 / 20.0);
    let mut band_w = [0.0_f64; 5];
    let mut active = 0.0_f64;
    let mut hi_peak = 0.0_f64;
    let mut hi_vals: Vec<f64> = Vec::new();
    let mut hi_far = 0.0_f64;
    // Frames within 100 ms of an onset carry the attack; the wash is measured only
    // between the hits, so a short bright hit file does not look sustained.
    let attack_frames = (0.1 / a.hop_s).round().max(1.0) as usize;
    // A frame is "near an attack" when an onset lies within the attack window around
    // it; the wash is measured only between the hits, so a short bright hit file does
    // not look sustained.
    let near_onset = |i: usize| {
        let i = i as isize;
        let af = attack_frames as isize;
        a.onsets.iter().any(|&o| (o as isize - i).abs() <= af)
    };
    for (i, (spec, e)) in a.mags.iter().zip(a.env.iter()).enumerate() {
        if *e < active_min {
            continue;
        }
        let power: Vec<f64> = spec.iter().map(|m| m * m).collect();
        let total: f64 = power.iter().sum();
        if total <= 0.0 {
            continue;
        }
        active += 1.0;
        let hi: f64 = power[bin4k..bin16k].iter().sum();
        if !near_onset(i) {
            hi_vals.push(hi);
            hi_far += 1.0;
            hi_peak = hi_peak.max(hi);
        }
        band_w[0] += power[..e0].iter().sum::<f64>() / total;
        band_w[1] += power[e0..e1].iter().sum::<f64>() / total;
        band_w[2] += power[e1..e2].iter().sum::<f64>() / total;
        band_w[3] += power[e2..e3].iter().sum::<f64>() / total;
        band_w[4] += power[e3..].iter().sum::<f64>() / total;
    }
    let act = active.max(1.0);
    f.w_low = band_w[0] / act;
    f.w_mid = band_w[1] / act;
    f.w_upper = band_w[2] / act;
    f.w_high = band_w[3] / act;
    f.w_top = band_w[4] / act;
    // The wash: far-from-attack frames whose high band stays within 20 dB of the
    // file's high-band peak (a cymbal mic; a drum mic drops between the hits).
    if hi_peak > 0.0 && hi_far > 0.0 {
        let floor = hi_peak * 0.01;
        f.hi_sustain = hi_vals.iter().filter(|&&h| h >= floor).count() as f64 / hi_far;
    }

    // Decay: per onset, the time until the envelope falls 12 dB under its local peak
    // (or the next onset, or the end).
    let target_drop = 12.0;
    let mut decays: Vec<f64> = Vec::new();
    for (oi, &o) in a.onsets.iter().enumerate() {
        let local_peak = a.env_db[o..]
            .iter()
            .take((0.15 / a.hop_s).max(1.0) as usize)
            .copied()
            .fold(f64::NEG_INFINITY, f64::max);
        let limit = local_peak - target_drop;
        let end = a.onsets.get(oi + 1).copied().unwrap_or(a.env_db.len());
        let mut t = None;
        for (j, v) in a.env_db.iter().enumerate().take(end).skip(o + 1) {
            if *v <= limit {
                t = Some((j - o) as f64 * a.hop_s);
                break;
            }
        }
        if let Some(t) = t {
            decays.push(t);
        }
    }
    f.decay_s = median(decays); // 0.0 when no hit ever decays (busy or sustained)

    f.onsets_per_s = a.onsets.len() as f64 / a.duration.max(1e-9);
    let mean_env = a.env.iter().sum::<f64>() / a.env.len() as f64;
    f.crest = peak_env / mean_env.max(1e-12);
    f
}

/// The per-hit features of every loud hit of the clip (the eval's per-hit mode).
#[must_use]
pub fn hit_features(samples: &[f32], sr: u32) -> Vec<RoleHitFeatures> {
    if samples.is_empty() {
        return Vec::new();
    }
    hit_list(&analyze(samples, sr))
}

/// The frame analysis shared by `role_features` and `hit_features`.
struct Analysis {
    mags: Vec<Vec<f64>>,
    env: Vec<f64>,
    env_db: Vec<f64>,
    hop_s: f64,
    duration: f64,
    onsets: Vec<usize>,
    loud: Vec<usize>,
    bin_hz: f64,
    peak_env: f64,
}

fn analyze(samples: &[f32], sr: u32) -> Analysis {
    let sr = f64::from(sr.max(1));
    let n = samples.len();
    let frame = ((FRAME_MS * sr / 1000.0).round() as usize).next_power_of_two().max(256);
    let hop = frame / 4;
    let norm = 1.0 / samples.iter().map(|s| s.abs()).fold(0.0_f32, f32::max).max(1e-30);

    // The magnitude spectra of the frames (Hann window), and the frame RMS envelope.
    let mut planner = FftPlanner::<f64>::new();
    let fft = planner.plan_fft_forward(frame);
    let window: Vec<f64> = (0..frame)
        .map(|i| 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / frame as f64).cos())
        .collect();
    let mut mags: Vec<Vec<f64>> = Vec::new();
    let mut env: Vec<f64> = Vec::new();
    let mut pos = 0;
    while pos + frame <= n {
        mags.push(frame_mags(fft.as_ref(), &window, &samples[pos..pos + frame], norm));
        env.push(frame_rms(&samples[pos..pos + frame], norm));
        pos += hop;
    }
    if mags.is_empty() {
        // Shorter than one frame: one zero-padded frame, no envelope beyond it.
        let mut padded = samples.to_vec();
        padded.resize(frame, 0.0);
        mags.push(frame_mags(fft.as_ref(), &window, &padded[..frame], norm));
        env.push(frame_rms(samples, norm));
    }

    let hop_s = hop as f64 / sr;
    let duration = n as f64 / sr;

    // Onsets on the frame envelope (dB domain, hop-resolution). A hit shows up as a rise
    // of 6 dB or more over the previous ~46 ms (the frame smoothing of the RMS needs the
    // longer window; a single-frame step would miss overlapping hits).
    let env_db: Vec<f64> = env.iter().map(|e| 20.0 * e.max(1e-12).log10()).collect();
    let peak_env = env.iter().copied().fold(0.0, f64::max);
    let peak_db = 20.0 * peak_env.max(1e-12).log10();
    let gate_db = peak_db - 45.0; // ignore the noise floor
    let rise_frames = (0.046 / hop_s).max(1.0) as usize;
    let mut onsets: Vec<usize> = Vec::new();
    let mut i = rise_frames;
    while i < env_db.len() {
        let before = env_db[i - rise_frames..i].iter().copied().fold(f64::INFINITY, f64::min);
        if env_db[i] > gate_db && env_db[i] - before >= 6.0 {
            onsets.push(i);
            i += (0.05 / hop_s).max(1.0) as usize; // 50 ms refractory
        } else {
            i += 1;
        }
    }

    // The loudest hits only: the mic's own instrument is louder on its own mic than the
    // bleed of the others. A 12 dB window keeps enough hits for a median.
    let mut loud: Vec<usize> = Vec::new();
    if !onsets.is_empty() {
        let max_hit = onsets.iter().filter_map(|&o| env.get(o)).copied().fold(0.0, f64::max);
        let floor = max_hit * 10.0_f64.powf(-LOUD_DB / 20.0);
        loud.extend(onsets.iter().copied().filter(|&o| env[o] >= floor));
    }

    let bins = mags[0].len();
    let bin_hz = sr / (2.0 * bins as f64);
    Analysis {
        mags,
        env,
        env_db,
        hop_s,
        duration,
        onsets,
        loud,
        bin_hz,
        peak_env,
    }
}

fn frame_mags(fft: &dyn rustfft::Fft<f64>, window: &[f64], chunk: &[f32], norm: f32) -> Vec<f64> {
    let mut buf: Vec<Complex<f64>> = chunk
        .iter()
        .zip(window.iter())
        .map(|(s, w)| Complex::new(f64::from(*s * norm) * w, 0.0))
        .collect();
    let half = buf.len() / 2;
    buf.resize(buf.len(), Complex::new(0.0, 0.0));
    fft.process(&mut buf);
    (0..half).map(|k| buf[k].norm()).collect()
}

fn frame_rms(chunk: &[f32], norm: f32) -> f64 {
    let sum: f64 = chunk
        .iter()
        .map(|s| {
            let s = f64::from(*s) * f64::from(norm);
            s * s
        })
        .sum();
    (sum / chunk.len().max(1) as f64).sqrt()
}

/// The per-hit features of the analysis' loud hits.
fn hit_list(a: &Analysis) -> Vec<RoleHitFeatures> {
    let bins = a.mags[0].len();
    let bin_hz = a.bin_hz;
    let e0 = ((150.0 / bin_hz) as usize).min(bins - 1);
    let e1 = ((800.0 / bin_hz) as usize).min(bins - 1);
    let bin4k = ((4000.0 / bin_hz) as usize).min(bins - 1);
    let e3 = ((12000.0 / bin_hz) as usize).min(bins - 1);
    let bin200 = ((200.0 / bin_hz) as usize).min(bins - 1);
    let bin16k = ((16000.0 / bin_hz) as usize).min(bins);
    let lo_band = ((80.0 / bin_hz) as usize).min(bins - 1)..((800.0 / bin_hz) as usize).min(bins);
    let hit_frames = (HIT_WINDOW_S / a.hop_s).round().max(1.0) as usize;

    let mut hits = Vec::new();
    for &o in &a.loud {
        // The window ends at the next onset, loud or not: a repeating burst must not
        // spill the next hit's attack into this hit's late band.
        let next = a
            .onsets
            .iter()
            .filter(|&&x| x > o)
            .min()
            .copied()
            .unwrap_or(a.mags.len());
        let end = (o + hit_frames).min(next).min(a.mags.len());
        let mut band = [0.0_f64; 5];
        let mut cent_num = 0.0;
        let mut cent_den = 0.0;
        let mut total_acc = 0.0;
        let mut low200 = 0.0;
        let mut hi_env: Vec<f64> = Vec::with_capacity(end - o);
        let mut spec_lo: Vec<Vec<f64>> = Vec::new();
        for spec in &a.mags[o..end] {
            let power: Vec<f64> = spec.iter().map(|m| m * m).collect();
            let total: f64 = power.iter().sum();
            if total <= 0.0 {
                continue;
            }
            total_acc += total;
            band[0] += power[..e0].iter().sum::<f64>();
            band[1] += power[e0..e1].iter().sum::<f64>();
            band[2] += power[e1..bin4k].iter().sum::<f64>();
            band[3] += power[bin4k..e3].iter().sum::<f64>();
            band[4] += power[e3..].iter().sum::<f64>();
            low200 += power[..bin200].iter().sum::<f64>();
            hi_env.push(power[bin4k..bin16k].iter().sum::<f64>());
            for (k, p) in power.iter().enumerate() {
                cent_num += k as f64 * bin_hz * p;
                cent_den += p;
            }
            spec_lo.push(power[lo_band.clone()].to_vec());
        }
        if total_acc <= 0.0 || hi_env.is_empty() {
            continue;
        }
        let bands = band.map(|b| b / total_acc);
        let centroid = if cent_den > 0.0 { cent_num / cent_den } else { 0.0 };
        // High-band decay inside the window: the time from the early peak (the first
        // 100 ms, so a later crash in the window cannot reset the peak) of the 4-16 kHz
        // envelope to -20 dB under it, plus the late/early energy ratio 150-400 ms vs
        // 0-50 ms.
        let win_s = (end - o) as f64 * a.hop_s;
        let early_n = (((0.1 / a.hop_s).round() as usize).max(1)).min(hi_env.len());
        let (peak_hi, peak_i) = hi_env[..early_n]
            .iter()
            .enumerate()
            .fold((0.0_f64, 0), |acc, (i, &p)| if p > acc.0 { (p, i) } else { acc });
        let t20 = if peak_hi > 0.0 {
            let limit = peak_hi * 0.01; // -20 dB in power
            match hi_env[peak_i..].iter().position(|&p| p <= limit) {
                Some(j) => (peak_i + j) as f64 * a.hop_s,
                None => win_s,
            }
        } else {
            win_s
        };
        let early_end = ((0.05 / a.hop_s).round().max(1.0) as usize).min(hi_env.len());
        let late_start = ((0.15 / a.hop_s).round() as usize).min(hi_env.len());
        let early: f64 = hi_env[..early_end].iter().sum();
        let late: f64 = hi_env[late_start..].iter().sum();
        let ratio_hi = if early > 0.0 { late / early } else { 0.0 };
        // Low-band tonal peakiness: the max of the hit spectrum over its 3-bin smoothed
        // mean in 80-800 Hz (a tom rings on its fundamental; a snare is noise).
        let tonal_lo = if spec_lo.len() > 1 {
            let mut acc = vec![0.0; spec_lo[0].len()];
            for p in &spec_lo {
                for (a2, v) in acc.iter_mut().zip(p) {
                    *a2 += v;
                }
            }
            let k = spec_lo.len() as f64;
            let p: Vec<f64> = acc.iter().map(|v| v / k).collect();
            tonal_ratio(&p)
        } else {
            tonal_ratio(&spec_lo[0])
        };
        hits.push(RoleHitFeatures {
            t_s: o as f64 * a.hop_s,
            env_db: a.env_db.get(o).copied().unwrap_or(f64::NEG_INFINITY),
            bands,
            low200: low200 / total_acc,
            centroid,
            t20_hi: t20,
            ratio_hi,
            tonal_lo,
        });
    }
    hits
}

impl RoleFeatures {
    fn zeroed() -> Self {
        RoleFeatures {
            b_low: 0.0,
            b_mid: 0.0,
            b_upper: 0.0,
            b_high: 0.0,
            b_top: 0.0,
            centroid: 0.0,
            iqr_high: 0.0,
            tonal_lo: 0.0,
            w_low: 0.0,
            w_mid: 0.0,
            w_upper: 0.0,
            w_high: 0.0,
            w_top: 0.0,
            decay_s: 0.0,
            hit_low200: 0.0,
            t20_hi: 0.0,
            ratio_hi: 0.0,
            hi_sustain: 0.0,
            onsets_per_s: 0.0,
            crest: 0.0,
        }
    }
}

/// max / smoothed-mean of a spectrum slice.
fn tonal_ratio(power: &[f64]) -> f64 {
    if power.len() < 5 {
        return 0.0;
    }
    let n = power.len();
    let mean = power.iter().sum::<f64>() / n as f64;
    let max = (0..n)
        .map(|i| {
            let lo = i.saturating_sub(1);
            let hi = (i + 1).min(n - 1);
            (power[lo] + power[i] + power[hi]) / 3.0
        })
        .fold(0.0, f64::max);
    max / mean.max(1e-30)
}

/// The per-hit vote of one loud hit: kick, snare, tom or hat. Cymbal is decided at the
/// file level (the wash between the hits), so a hit never votes cymbal.
fn hit_vote(h: &RoleHitFeatures) -> Role {
    let bright = h.bands[3] + h.bands[4];
    // Kick: the low band dominates, everything under 200 Hz, and the centroid under
    // ~130 Hz. A tom's fundamental keeps the centroid higher (its body sits at 150-400
    // Hz) and its low band is never this dominant - the tom->kick confusion of the
    // file-median rules (#393) separates here on the centroid and the b_low floor.
    if h.bands[0] >= 0.65 && h.low200 >= 0.9 && h.centroid <= 130.0 {
        return Role::Kick;
    }
    // Hat: the hit itself is bright (4 kHz and up carry a real share).
    if bright >= 0.12 {
        return Role::Hat;
    }
    // Tom against snare on where the body sits: a tom's fundamental is under 200 Hz,
    // a snare's body above it. The grey zone (0.12-0.35) goes by the low-band share.
    if h.low200 >= 0.35 || (h.low200 > 0.12 && h.bands[0] >= 0.1) {
        return Role::Tom;
    }
    Role::Snare
}

/// The rules on the features and the per-hit votes. The None gate first (steady material
/// is not a drum), then the file-level wash veto into cymbal, then the vote shares of the
/// loud hits. The confidence is the smaller of the best vote share and the margin to the
/// second share (a file whose hits disagree is no hint), or the wash share for a cymbal.
fn classify(f: &RoleFeatures, hits: &[RoleHitFeatures]) -> (Option<Role>, f64) {
    // Steady material (pink noise, a hum, room tone) has no hits at all.
    if f.onsets_per_s <= 0.0 || f.crest < 1.3 {
        return (None, 0.0);
    }
    let bright_hit = f.b_high + f.b_top;
    // Cymbal veto: the wash between RARE hits keeps high-band energy in the whole-file
    // view (a cymbal mic ringing between hits; #393). The onset-rate gate keeps the
    // bleed of busy close-mic files from counting as a wash: on the ENST close mics the
    // between-hit high band stays up too (room bleed), but those files have 2-4 onsets
    // per second. Only a clip with actual high-band content counts.
    let bright = bright_hit >= 0.02 || f.w_high + f.w_top >= 0.02;
    if bright && f.hi_sustain >= 0.5 && f.onsets_per_s <= 1.0 {
        return (Some(Role::Cymbal), f.hi_sustain);
    }
    // The vote shares of the loud hits over the four drum roles.
    let mut votes = [0.0_f64; 4];
    for h in hits {
        votes[match hit_vote(h) {
            Role::Kick => 0,
            Role::Snare => 1,
            Role::Tom => 2,
            _ => 3,
        }] += 1.0;
    }
    let total: f64 = votes.iter().sum();
    if total <= 0.0 {
        return (None, 0.0);
    }
    let mut order = [0, 1, 2, 3];
    order.sort_by(|&a, &b| votes[b].total_cmp(&votes[a]));
    let best = order[0];
    let role = [Role::Kick, Role::Snare, Role::Tom, Role::Hat][best];
    let share = votes[best] / total;
    let margin = (votes[best] - votes[order[1]]) / total;
    (Some(role), share.min(margin))
}

#[cfg(test)]
mod tests;
