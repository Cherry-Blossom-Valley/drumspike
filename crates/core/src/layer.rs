//! P4-7a and P4-7b: the one-shot layer of a track. The engineer gives a track one one-shot sample
//! (WAV or AIFF); the app plays it at every hit and renders it as `Exports/<name>_layer.wav`.
//!
//! * [`read_one_shot`]: a WAV (the core reader) or AIFF/AIFC file (big-endian PCM 8 to 32 bit,
//!   AIFC `sowt`, `fl32`, `fl64`), kept planar with 1 or 2 channels (more are folded to mono).
//! * [`one_shot_onset`]: the one-shot's own onset, by the refinement rule of the analysis
//!   ([`RefineRule::CURRENT`]): the coarse time is 6 ms before the first sample at 25 % of the peak
//!   (where the detector reports a hit), the rule moves it to the attack. The render puts this
//!   sample on the hit, not the file start.
//! * [`sinc_resample`] and [`frac_delayed`]: a windowed sinc (Blackman, [`SINC_HALF`] zero crossings
//!   each side) for the rate change and the sub-sample delay.
//! * [`Curve`]: the gain of a hit from its velocity (default linear in dB over [`DEFAULT_RANGE_DB`]).
//! * P4-7b: [`xcorr_hit`] cross-correlates the one-shot's attack with the close mic around a hit
//!   (normalised, so the height is 0 to 1); [`AlignParams`] says how long the attack is and how far
//!   the search reaches; [`majority_sign`] is the polarity of the track.
//! * [`render_layer`]: the layer signal of a track, each hit's onset at its position (whole or
//!   fractional samples; a fractional start goes through [`frac_delayed`], quantised to
//!   1/[`FRAC_STEPS`] sample).

use std::collections::HashMap;
use std::path::Path;

use crate::io::MappedWav;
use crate::refine::{RefineRule, RefineWindow, refine_rule};

/// The longest one-shot the app takes (seconds).
pub const MAX_ONE_SHOT_S: f64 = 20.0;
/// Zero crossings of the windowed sinc on each side.
pub const SINC_HALF: usize = 16;
/// The fractional delays of a render are quantised to 1/`FRAC_STEPS` sample.
pub const FRAC_STEPS: u32 = 64;
/// The default range of the dB velocity curve: velocity 127 is 0 dB, velocity 0 is -24 dB.
pub const DEFAULT_RANGE_DB: f64 = 24.0;
/// The detector reports a hit about this long before the attack; the one-shot's coarse time sits
/// this long before its first sample at 25 % of the peak.
pub const COARSE_LEAD_S: f64 = 0.006;
/// The level of the first sample that counts for the coarse time (a share of the peak).
pub const COARSE_SHARE: f32 = 0.25;
/// Silence put in front of a one-shot for the onset rule (its pre-attack level and margins).
pub const ONSET_PAD_S: f64 = 0.030;

/// A one-shot sample: rate and the channels (planar; 1 or 2).
#[derive(Debug, Clone, PartialEq)]
pub struct OneShot {
    /// Sample rate in Hz.
    pub sr: u32,
    /// The channels, each `frames` long.
    pub chans: Vec<Vec<f32>>,
}

impl OneShot {
    /// Channels (1 or 2).
    #[must_use]
    pub fn channels(&self) -> usize {
        self.chans.len()
    }

    /// Frames.
    pub fn frames(&self) -> usize {
        self.chans.first().map_or(0, Vec::len)
    }

    /// The mean of the channels (the signal the onset and the alignment read).
    #[must_use]
    pub fn mono(&self) -> Vec<f32> {
        mono_of(&self.chans)
    }

    /// The one-shot at `to` Hz ([`sinc_resample`] per channel; the same samples when the rate is `to`).
    #[must_use]
    pub fn resampled(&self, to: u32) -> OneShot {
        if to == self.sr {
            return self.clone();
        }
        OneShot {
            sr: to,
            chans: self.chans.iter().map(|c| sinc_resample(c, self.sr, to)).collect(),
        }
    }

    /// The channels interleaved (frame by frame).
    #[must_use]
    pub fn interleaved(&self) -> Vec<f32> {
        let (n, k) = (self.frames(), self.channels());
        let mut out = Vec::with_capacity(n * k);
        for i in 0..n {
            for c in &self.chans {
                out.push(c[i]);
            }
        }
        out
    }
}

/// The mean of planar channels.
fn mono_of(chans: &[Vec<f32>]) -> Vec<f32> {
    match chans.len() {
        0 => Vec::new(),
        1 => chans[0].clone(),
        k => (0..chans[0].len())
            .map(|i| chans.iter().map(|c| c[i]).sum::<f32>() / k as f32)
            .collect(),
    }
}

/// Planar channels from interleaved samples; more than 2 channels are folded to their mean.
fn planar(inter: &[f32], ch: usize) -> Vec<Vec<f32>> {
    let n = inter.len() / ch.max(1);
    let mut chans: Vec<Vec<f32>> = (0..ch).map(|c| (0..n).map(|i| inter[i * ch + c]).collect()).collect();
    if ch > 2 {
        chans = vec![mono_of(&chans)];
    }
    chans
}

/// Read a one-shot: a WAV file (RIFF/WAVE, the core reader) or an AIFF/AIFC file. The error text
/// names what is wrong (format, length, an empty file).
pub fn read_one_shot(path: &Path) -> Result<OneShot, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let shot = if bytes.len() >= 12 && &bytes[0..4] == b"FORM" {
        read_aiff(&bytes)?
    } else {
        let wav = MappedWav::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let info = &wav.info;
        let ch = info.channels as usize;
        let data = &wav.bytes()[info.data_offset..info.data_offset + info.n_frames * info.block_align];
        let mut inter = vec![0.0f32; info.n_frames * ch];
        let n = info.decode_channels_into(data, &mut inter);
        inter.truncate(n * ch);
        OneShot {
            sr: info.sample_rate,
            chans: planar(&inter, ch),
        }
    };
    if shot.frames() == 0 || shot.sr == 0 {
        return Err(format!("{}: the file holds no audio", path.display()));
    }
    if shot.frames() as f64 > MAX_ONE_SHOT_S * f64::from(shot.sr) {
        return Err(format!(
            "{}: a one-shot is at most {MAX_ONE_SHOT_S} s long",
            path.display()
        ));
    }
    Ok(shot)
}

/// The 80-bit IEEE extended number of an AIFF `COMM` chunk (the sample rate).
#[must_use]
pub fn ext80_to_f64(b: &[u8]) -> f64 {
    if b.len() < 10 {
        return 0.0;
    }
    let sign = if b[0] & 0x80 != 0 { -1.0 } else { 1.0 };
    let exp = (i32::from(b[0] & 0x7F) << 8) | i32::from(b[1]);
    let mut m = 0u64;
    for &x in &b[2..10] {
        m = (m << 8) | u64::from(x);
    }
    if exp == 0 && m == 0 {
        return 0.0;
    }
    sign * (m as f64) * 2f64.powi(exp - 16383 - 63)
}

/// One sample of an AIFF data chunk to a float.
type SampleConv = Box<dyn Fn(&[u8]) -> f32>;

/// An AIFF or AIFC file (FORM chunk): `COMM` and `SSND`, big-endian PCM of 8 to 32 bits, AIFC
/// `NONE`, `sowt` (little-endian 16 bit), `fl32` and `fl64`.
fn read_aiff(b: &[u8]) -> Result<OneShot, String> {
    let kind = &b[8..12];
    if kind != b"AIFF" && kind != b"AIFC" {
        return Err("not an AIFF file".into());
    }
    let be16 = |o: usize| i16::from_be_bytes([b[o], b[o + 1]]);
    let be32 = |o: usize| u32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
    let (mut comm, mut ssnd) = (None, None);
    let mut pos = 12usize;
    while pos + 8 <= b.len() {
        let id = &b[pos..pos + 4];
        let size = be32(pos + 4) as usize;
        let body = pos + 8;
        let end = (body + size).min(b.len());
        if id == b"COMM" {
            comm = Some((body, end));
        } else if id == b"SSND" {
            ssnd = Some((body, end));
        }
        pos = body + size + (size & 1);
    }
    let (c0, c1) = comm.ok_or("AIFF without a COMM chunk")?;
    if c1 - c0 < 18 {
        return Err("AIFF COMM chunk too short".into());
    }
    let ch = be16(c0).max(0) as usize;
    let frames = be32(c0 + 2) as usize;
    let bits = be16(c0 + 6).max(0) as usize;
    let sr = ext80_to_f64(&b[c0 + 8..c0 + 18]).round();
    let comp: &[u8] = if kind == b"AIFC" && c1 - c0 >= 22 {
        &b[c0 + 18..c0 + 22]
    } else {
        b"NONE"
    };
    if ch == 0 || !(1.0..=1e6).contains(&sr) {
        return Err("AIFF with no channels or a bad rate".into());
    }
    let (s0, s1) = ssnd.ok_or("AIFF without an SSND chunk")?;
    if s1 - s0 < 8 {
        return Err("AIFF SSND chunk too short".into());
    }
    let data = &b[(s0 + 8 + be32(s0) as usize).min(s1)..s1];
    let (width, conv): (usize, SampleConv) = match comp {
        b"NONE" | b"twos" => match bits.div_ceil(8) {
            1 => (1, Box::new(|s: &[u8]| f32::from(s[0] as i8) / 128.0)),
            2 => (
                2,
                Box::new(|s: &[u8]| f32::from(i16::from_be_bytes([s[0], s[1]])) / 32768.0),
            ),
            3 => (
                3,
                Box::new(|s: &[u8]| (i32::from_be_bytes([s[0], s[1], s[2], 0]) >> 8) as f32 / 8_388_608.0),
            ),
            4 => (
                4,
                Box::new(|s: &[u8]| i32::from_be_bytes([s[0], s[1], s[2], s[3]]) as f32 / 2_147_483_648.0),
            ),
            _ => return Err(format!("AIFF with {bits} bits per sample")),
        },
        b"sowt" if bits <= 16 => (
            2,
            Box::new(|s: &[u8]| f32::from(i16::from_le_bytes([s[0], s[1]])) / 32768.0),
        ),
        b"fl32" | b"FL32" => (4, Box::new(|s: &[u8]| f32::from_be_bytes([s[0], s[1], s[2], s[3]]))),
        b"fl64" | b"FL64" => (
            8,
            Box::new(|s: &[u8]| {
                let mut a = [0u8; 8];
                a.copy_from_slice(&s[..8]);
                f64::from_be_bytes(a) as f32
            }),
        ),
        other => {
            return Err(format!(
                "AIFC compression {:?} is not supported",
                String::from_utf8_lossy(other)
            ));
        }
    };
    let n = frames.min(data.len() / (width * ch));
    let inter: Vec<f32> = data[..n * width * ch].chunks_exact(width).map(&*conv).collect();
    Ok(OneShot {
        sr: sr as u32,
        chans: planar(&inter, ch),
    })
}

/// The onset of a one-shot (mono, `sr`), in samples: the refinement rule of the analysis
/// ([`RefineRule::CURRENT`], no neighbours, the default window) from a coarse time
/// [`COARSE_LEAD_S`] before the first sample at [`COARSE_SHARE`] of the peak. A silent file has
/// its onset at 0. The result is a sample of the file.
#[must_use]
pub fn one_shot_onset(x: &[f32], sr: u32) -> usize {
    let peak = x.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    if peak <= 0.0 || x.is_empty() || sr == 0 {
        return 0;
    }
    // the rule reads a pre-attack level before the attack: a one-shot that starts with its
    // attack gets that much silence in front (and the result is moved back)
    let pad = (ONSET_PAD_S * f64::from(sr)).round() as usize;
    let mut padded = vec![0.0f32; pad];
    padded.extend_from_slice(x);
    let first = padded
        .iter()
        .position(|v| v.abs() >= COARSE_SHARE * peak)
        .unwrap_or(pad);
    let t0 = (first as f64 / f64::from(sr) - COARSE_LEAD_S).max(0.0);
    let r = refine_rule(&padded, t0, sr, &[], RefineWindow::DEFAULT, RefineRule::CURRENT);
    let i = (r.t * f64::from(sr)).round() - pad as f64;
    if i.is_finite() && i >= 0.0 {
        (i as usize).min(x.len() - 1)
    } else {
        0
    }
}

/// The Blackman window at `u` in -1..1 (0 outside).
fn blackman(u: f64) -> f64 {
    if u.abs() >= 1.0 {
        return 0.0;
    }
    let a = std::f64::consts::PI * (u + 1.0);
    0.42 - 0.5 * a.cos() + 0.08 * (2.0 * a).cos()
}

/// `sin(pi t) / (pi t)`.
fn sinc(t: f64) -> f64 {
    if t.abs() < 1e-12 {
        1.0
    } else {
        let p = std::f64::consts::PI * t;
        p.sin() / p
    }
}

/// Resample `x` from `from` to `to` Hz with a windowed sinc (Blackman, [`SINC_HALF`] zero crossings
/// of the lower band edge on each side; downwards the cut-off is the new Nyquist). The output is
/// `round(len * to / from)` long; output sample `m` is the input at `m * from / to`.
#[must_use]
pub fn sinc_resample(x: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || from == 0 || to == 0 {
        return x.to_vec();
    }
    let ratio = f64::from(to) / f64::from(from);
    let fc = ratio.min(1.0);
    let half = SINC_HALF as f64 / fc;
    let n_out = (x.len() as f64 * ratio).round() as usize;
    (0..n_out)
        .map(|m| {
            let pos = m as f64 / ratio;
            let lo = (pos - half).ceil().max(0.0) as usize;
            let hi = ((pos + half).floor() as usize).min(x.len().saturating_sub(1));
            let mut acc = 0.0f64;
            for (k, &v) in x.iter().enumerate().take(hi + 1).skip(lo) {
                let t = pos - k as f64;
                acc += f64::from(v) * fc * sinc(fc * t) * blackman(t / half);
            }
            acc as f32
        })
        .collect()
}

/// `x` delayed by `frac` (0 to 1) of a sample with a windowed sinc ([`SINC_HALF`] taps each side).
/// The result is `len + 2 * SINC_HALF` long and its sample `j` is `x` at `j - SINC_HALF - frac`:
/// put it [`SINC_HALF`] samples before the whole-sample start.
#[must_use]
pub fn frac_delayed(x: &[f32], frac: f64) -> Vec<f32> {
    let h = SINC_HALF as i64;
    let taps: Vec<f64> = (-h..=h)
        .map(|d| sinc(d as f64 - frac) * blackman((d as f64 - frac) / h as f64))
        .collect();
    let n = x.len() as i64 + 2 * h;
    (0..n)
        .map(|j| {
            // y(n) = sum_k x[k] h(n - frac - k), n = j - h
            let nn = j - h;
            let mut acc = 0.0f64;
            for (ti, d) in (-h..=h).enumerate() {
                let k = nn - d;
                if k >= 0 && (k as usize) < x.len() {
                    acc += f64::from(x[k as usize]) * taps[ti];
                }
            }
            acc as f32
        })
        .collect()
}

/// The gain of a hit from its velocity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Curve {
    /// Linear in dB: velocity 127 is 0 dB, velocity 0 is `-range_db` (the default, [`DEFAULT_RANGE_DB`]).
    Db {
        /// The range in dB (0 to 96).
        range_db: f64,
    },
    /// Linear in amplitude: `v / 127`.
    Linear,
    /// Every hit at the layer's level.
    Fixed,
}

impl Default for Curve {
    fn default() -> Self {
        Curve::Db {
            range_db: DEFAULT_RANGE_DB,
        }
    }
}

impl Curve {
    /// The linear gain at velocity `v` (clamped to 0 to 127).
    #[must_use]
    pub fn gain(&self, v: i64) -> f64 {
        let v = v.clamp(0, 127) as f64;
        match *self {
            Curve::Db { range_db } => 10f64.powf(-range_db.clamp(0.0, 96.0) * (1.0 - v / 127.0) / 20.0),
            Curve::Linear => v / 127.0,
            Curve::Fixed => 1.0,
        }
    }

    /// The curve of a JSON object `{kind: "db" | "linear" | "fixed", range_db?}`; `None` for another value.
    #[must_use]
    pub fn from_json(v: &serde_json::Value) -> Option<Curve> {
        match v.get("kind").and_then(|k| k.as_str())? {
            "db" => {
                let r = match v.get("range_db") {
                    None | Some(serde_json::Value::Null) => DEFAULT_RANGE_DB,
                    Some(x) => x.as_f64().filter(|r| r.is_finite() && (0.0..=96.0).contains(r))?,
                };
                Some(Curve::Db { range_db: r })
            }
            "linear" => Some(Curve::Linear),
            "fixed" => Some(Curve::Fixed),
            _ => None,
        }
    }

    /// The JSON object of the curve.
    #[must_use]
    pub fn to_json(&self) -> serde_json::Value {
        match *self {
            Curve::Db { range_db } => serde_json::json!({"kind": "db", "range_db": range_db}),
            Curve::Linear => serde_json::json!({"kind": "linear"}),
            Curve::Fixed => serde_json::json!({"kind": "fixed"}),
        }
    }
}

/// The settings of the per-hit cross-correlation (P4-7b).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AlignParams {
    /// Length of the one-shot's attack that is correlated (seconds; the card's 5 to 20 ms).
    pub attack_s: f64,
    /// The attack window starts this long before the one-shot's onset (seconds).
    pub pre_s: f64,
    /// The largest lag tried on each side of the hit (seconds).
    pub max_lag_s: f64,
}

impl Default for AlignParams {
    fn default() -> Self {
        AlignParams {
            attack_s: 0.010,
            pre_s: 0.001,
            max_lag_s: 0.003,
        }
    }
}

/// The cross-correlation of one hit: the best lag and height for each sign (P4-7b).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HitXcorr {
    /// Lag (samples, sub-sample) of the largest positive correlation: the one-shot's onset sits at `hit + lag`.
    pub pos_lag: f64,
    /// Its height (0 to 1).
    pub pos_r: f64,
    /// Lag of the largest negative correlation (the one-shot inverted).
    pub neg_lag: f64,
    /// Its height as a positive number (0 to 1).
    pub neg_r: f64,
    /// The best lag sits on the search edge: the true lag may be outside the window.
    pub pos_edge: bool,
    /// The same for the inverted match.
    pub neg_edge: bool,
}

impl HitXcorr {
    /// The sign of the better match: +1 (as is) or -1 (inverted); +1 on a tie.
    #[must_use]
    pub fn sign(&self) -> i8 {
        if self.neg_r > self.pos_r { -1 } else { 1 }
    }

    /// The lag and the height for polarity `sign` (+1 or -1).
    #[must_use]
    pub fn for_sign(&self, sign: i8) -> (f64, f64) {
        if sign < 0 {
            (self.neg_lag, self.neg_r)
        } else {
            (self.pos_lag, self.pos_r)
        }
    }

    /// The lag for `sign` with its edge flag (an edge lag is uncertain).
    #[must_use]
    pub fn for_sign_edge(&self, sign: i8) -> (f64, f64, bool) {
        if sign < 0 {
            (self.neg_lag, self.neg_r, self.neg_edge)
        } else {
            (self.pos_lag, self.pos_r, self.pos_edge)
        }
    }
}

/// The attack template of a one-shot (mono, at the mic's rate) for [`xcorr_hit`]: the samples from
/// `pre` before the onset, [`AlignParams::attack_s`] long (shorter at the end of the file), and `pre`
/// in samples.
#[must_use]
pub fn attack_template(x: &[f32], onset: usize, sr: u32, p: &AlignParams) -> (Vec<f32>, usize) {
    let pre = ((p.pre_s * f64::from(sr)).round() as usize).min(onset);
    let len = ((p.attack_s * f64::from(sr)).round() as usize).max(1);
    let a0 = onset - pre;
    let a1 = (a0 + len).min(x.len());
    (x[a0..a1].to_vec(), pre)
}

/// Parabolic interpolation of a peak at index `i` of `y`: (offset -0.5 to 0.5, height).
fn parabolic(y: &[f64], i: usize) -> (f64, f64) {
    if i == 0 || i + 1 >= y.len() {
        return (0.0, y[i]);
    }
    let (a, b, c) = (y[i - 1], y[i], y[i + 1]);
    let den = a - 2.0 * b + c;
    if den >= 0.0 {
        return (0.0, b);
    }
    let d = (0.5 * (a - c) / den).clamp(-0.5, 0.5);
    (d, b - 0.25 * (a - c) * d)
}

/// Cross-correlate the attack `a` (from [`attack_template`], whose onset is `pre` samples into it)
/// with the close mic `mic` around the hit sample `hit`, lags `-max_lag..=max_lag`. Normalised
/// (cosine of the two windows), so each height is -1 to 1; the best lag of each sign is refined to a
/// sub-sample lag by parabolic interpolation. `None` when the attack is silent or the mic window
/// is silent at every lag.
#[must_use]
pub fn xcorr_hit(mic: &[f32], hit: i64, a: &[f32], pre: usize, max_lag: usize) -> Option<HitXcorr> {
    let l = a.len();
    let an: f64 = a.iter().map(|v| f64::from(*v) * f64::from(*v)).sum::<f64>().sqrt();
    if l == 0 || an <= 0.0 {
        return None;
    }
    let r = max_lag as i64;
    // the mic samples from hit - pre - max_lag, zero outside the file
    let s0 = hit - pre as i64 - r;
    let seg_len = l + 2 * max_lag;
    let seg: Vec<f64> = (0..seg_len as i64)
        .map(|k| {
            let i = s0 + k;
            if i >= 0 && (i as usize) < mic.len() {
                f64::from(mic[i as usize])
            } else {
                0.0
            }
        })
        .collect();
    let mut sq = vec![0.0f64; seg_len + 1];
    for (k, v) in seg.iter().enumerate() {
        sq[k + 1] = sq[k] + v * v;
    }
    let n_lags = 2 * max_lag + 1;
    let mut y = vec![0.0f64; n_lags];
    let mut any = false;
    for (j, out) in y.iter_mut().enumerate() {
        let e = sq[j + l] - sq[j];
        if e <= 1e-20 {
            continue;
        }
        any = true;
        let dot: f64 = a.iter().zip(&seg[j..j + l]).map(|(p, q)| f64::from(*p) * q).sum();
        *out = dot / (an * e.sqrt());
    }
    if !any {
        return None;
    }
    let best = |s: f64| -> (f64, f64, bool) {
        let ys: Vec<f64> = y.iter().map(|v| s * v).collect();
        let (mut bi, mut bv) = (max_lag, f64::NEG_INFINITY);
        for (i, v) in ys.iter().enumerate() {
            // ties go to the lag nearest 0
            let closer = (i as i64 - r).abs() < (bi as i64 - r).abs();
            if *v > bv || (*v == bv && closer) {
                bi = i;
                bv = *v;
            }
        }
        // a best at the search edge: the true lag may be outside the window (the parabolic
        // cannot extend past the array) - the lag is uncertain
        let edge = bi == 0 || bi == n_lags - 1;
        let (d, h) = parabolic(&ys, bi);
        (bi as f64 - r as f64 + d, h.clamp(0.0, 1.0), edge)
    };
    let (pos_lag, pos_r, pos_edge) = best(1.0);
    let (neg_lag, neg_r, neg_edge) = best(-1.0);
    Some(HitXcorr {
        pos_lag,
        pos_r,
        neg_lag,
        neg_r,
        pos_edge,
        neg_edge,
    })
}

/// The polarity of a track from its hits: -1 when more hits match better inverted than as is,
/// else +1. Returns (sign, hits as is, hits inverted).
#[must_use]
pub fn majority_sign(hits: &[HitXcorr]) -> (i8, usize, usize) {
    let neg = hits.iter().filter(|h| h.sign() < 0).count();
    let pos = hits.len() - neg;
    (if neg > pos { -1 } else { 1 }, pos, neg)
}

/// The layer signal of a track: `n_frames` samples per channel of `shot` (at the output rate),
/// one copy per hit `(pos, gain)` with its onset `onset` (samples of `shot`, fractional when the one-shot
/// was resampled: P4-7a fix 7) at the sample position `pos`
/// (fractional: the copy goes through [`frac_delayed`], quantised to 1/[`FRAC_STEPS`] sample).
/// Copies overlap and add (no cut); a copy that starts before the file or runs past its end is cut.
#[must_use]
pub fn render_layer(n_frames: usize, shot: &OneShot, onset: f64, hits: &[(f64, f32)]) -> Vec<Vec<f32>> {
    let mut out = vec![vec![0.0f32; n_frames]; shot.channels()];
    let mut delayed: HashMap<u32, Vec<Vec<f32>>> = HashMap::new();
    for &(pos, gain) in hits {
        if !pos.is_finite() || gain == 0.0 {
            continue;
        }
        let start = pos - onset;
        let mut i0 = start.floor();
        let mut q = ((start - i0) * f64::from(FRAC_STEPS)).round() as u32;
        if q == FRAC_STEPS {
            q = 0;
            i0 += 1.0;
        }
        let (src, base): (&Vec<Vec<f32>>, i64) = if q == 0 {
            (&shot.chans, i0 as i64)
        } else {
            let v = delayed.entry(q).or_insert_with(|| {
                shot.chans
                    .iter()
                    .map(|c| frac_delayed(c, f64::from(q) / f64::from(FRAC_STEPS)))
                    .collect()
            });
            (&*v, i0 as i64 - SINC_HALF as i64)
        };
        for (o, s) in out.iter_mut().zip(src) {
            let k0 = (-base).max(0) as usize;
            for (k, v) in s.iter().enumerate().skip(k0) {
                let i = base + k as i64;
                if i as usize >= n_frames {
                    break;
                }
                o[i as usize] += gain * v;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests;
