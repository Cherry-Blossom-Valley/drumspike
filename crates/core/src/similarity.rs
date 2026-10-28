//! P4-6b / #293: the Learn similarity, DSP-only (`docs/migration/concept-learn-mode.md`, sections 3.1 and 4.1).
//!
//! The model-based version (P4-6b, `patch-1` and `mfcc-1`) had a fourth block, the five class
//! activations of the model. The model is gone (#281 D16), so the DSP-only recipes `patch-2` and
//! `mfcc-2` have three blocks: each detection candidate (and each Learn example) gets a feature vector
//! taken from the native-rate mono signal at the candidate's own anchor ([`anchor`]: the refinement's
//! steepest rise near the candidate, without the neighbour clipping):
//!
//! | Block | [`Recipe::Patch`] (option A, `patch-2`) | [`Recipe::Mfcc`] (option C, `mfcc-2`, the ablation) |
//! |---|---|---|
//! | attack | 4 frames of about 11 ms (Hann), 5 ms hop from 2 ms before the anchor, 32 log bands 60 Hz to 16 kHz | the same frames, 40 mel bands, DCT, c1 to c13 |
//! | body | 60 ms from 10 ms after the anchor (Hann), 1/6-octave bands from 40 Hz to 16 kHz | the same window, 40 mel bands, DCT, c1 to c19 |
//! | envelope | 12 steps of 10 ms from the anchor: the energy in dB | the same |
//!
//! Each audio block is in dB with a floor 80 dB under its maximum, its mean removed and L2-normalised
//! ([`Blocks`]), so the level of a hit does not count. The weights ([`Weights`]) scale the blocks
//! before the whole vector is normalised ([`weigh`]); the similarity of two weighted vectors is
//! their dot product, the cosine ([`cosine`]).
//!
//! The score of a candidate against the positive examples `P` and the negative examples `N`
//! ([`score`], concept 3.1): `sp` the mean of the `min(3, |P|)` highest similarities to `P`, `sn` the
//! same against `N` (0 without negatives), `clamp((1 + sp - max(sn, 0)) / 2, 0, 1)`.
//!
//! Window lengths are in ms and band edges in Hz, so features of 44.1, 48 and 96 kHz tracks compare.
//! Magnitude spectra only: the polarity of a mic does not matter. A window past the end of the
//! signal is zero-padded.

use std::sync::Arc;

use rayon::prelude::*;
use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};

use crate::detect::peaks::FPS;
use crate::refine::{refine_sr, RefineWindow};

/// The feature recipe: the name a track's `learn.method` stores.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Recipe {
    /// Option A: log-band spectra (`patch-2`).
    Patch,
    /// Option C: MFCC-like cepstra in the two spectral blocks (`mfcc-2`).
    Mfcc,
}

impl Recipe {
    /// The method name of the recipe.
    pub fn name(self) -> &'static str {
        match self {
            Recipe::Patch => "patch-2",
            Recipe::Mfcc => "mfcc-2",
        }
    }

    /// The recipe of a method name, `None` for an unknown one.
    pub fn parse(name: &str) -> Option<Recipe> {
        match name {
            "patch-2" => Some(Recipe::Patch),
            "mfcc-2" => Some(Recipe::Mfcc),
            _ => None,
        }
    }

    /// The length of each block: attack, body, envelope.
    pub fn dims(self) -> [usize; 3] {
        match self {
            Recipe::Patch => [ATTACK_FRAMES * ATTACK_BANDS, body_edges_patch().len() - 1, ENV_STEPS],
            Recipe::Mfcc => [ATTACK_FRAMES * MFCC_ATTACK, MFCC_BODY, ENV_STEPS],
        }
    }

    /// The length of a feature vector.
    pub fn len(self) -> usize {
        self.dims().iter().sum()
    }

    /// Always false: a recipe has blocks (clippy's `len_without_is_empty`).
    pub fn is_empty(self) -> bool {
        self.len() == 0
    }
}

/// Attack frames per candidate.
pub const ATTACK_FRAMES: usize = 4;
/// Attack frame length (seconds; 512 samples at 48 kHz).
pub const ATTACK_WIN_S: f64 = 0.0107;
/// Attack frame hop (seconds).
pub const ATTACK_HOP_S: f64 = 0.005;
/// The first attack frame starts this far from the anchor (seconds).
pub const ATTACK_START_S: f64 = -0.002;
/// Log bands of an attack frame (option A).
pub const ATTACK_BANDS: usize = 32;
/// Lowest and highest attack band edge (Hz).
pub const ATTACK_LO_HZ: f64 = 60.0;
/// The highest band edge of every spectral block (Hz; lower when the Nyquist frequency is lower).
pub const TOP_HZ: f64 = 16_000.0;
/// The body window starts this far after the anchor (seconds) ...
pub const BODY_START_S: f64 = 0.010;
/// ... and is this long (seconds).
pub const BODY_LEN_S: f64 = 0.060;
/// The lowest body band edge (Hz); the bands are 1/6 octave wide.
pub const BODY_LO_HZ: f64 = 40.0;
/// Envelope steps from the anchor.
pub const ENV_STEPS: usize = 12;
/// Envelope step length (seconds).
pub const ENV_STEP_S: f64 = 0.010;
/// The floor of a block under its maximum (dB).
pub const DB_RANGE: f32 = 80.0;
/// The number of nearest examples a score averages (concept 3.1: `k = min(3, |P|)`).
pub const K_NEAREST: usize = 3;
/// Mel bands of the option C blocks.
pub const MEL_BANDS: usize = 40;
const MFCC_ATTACK: usize = 13;
const MFCC_BODY: usize = 19;

/// The weights of the three blocks. The model-based weights (`attack` 1, `body` 1, `env` 0.5,
/// `act` 0.5, P4-6b, Session A only) lost their activation block with #293; these three are the same
/// values until the DSP-only weights are tuned on the real sessions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Weights {
    /// The attack spectra.
    pub attack: f32,
    /// The body spectrum.
    pub body: f32,
    /// The energy envelope.
    pub env: f32,
}

impl Weights {
    /// The weights the app uses.
    pub const DEFAULT: Weights = Weights {
        attack: 1.0,
        body: 1.0,
        env: 0.5,
    };

    /// The three weights in block order.
    pub fn as_array(&self) -> [f32; 3] {
        [self.attack, self.body, self.env]
    }
}

impl Default for Weights {
    fn default() -> Self {
        Weights::DEFAULT
    }
}

fn body_edges_patch() -> Vec<f64> {
    let mut e = vec![BODY_LO_HZ];
    loop {
        let next = e[e.len() - 1] * 2f64.powf(1.0 / 6.0);
        if next > TOP_HZ * 1.0001 {
            break;
        }
        e.push(next);
    }
    e
}

fn log_edges(lo: f64, hi: f64, n: usize) -> Vec<f64> {
    (0..=n).map(|i| lo * (hi / lo).powf(i as f64 / n as f64)).collect()
}

fn mel(f: f64) -> f64 {
    2595.0 * (1.0 + f / 700.0).log10()
}

fn mel_inv(m: f64) -> f64 {
    700.0 * (10f64.powf(m / 2595.0) - 1.0)
}

fn mel_edges(lo: f64, hi: f64, n: usize) -> Vec<f64> {
    let (a, b) = (mel(lo), mel(hi));
    (0..=n).map(|i| mel_inv(a + (b - a) * i as f64 / n as f64)).collect()
}

/// For each band `[edges[i], edges[i+1])` the FFT bins whose centre falls in it; an empty band
/// takes the bin nearest to its geometric centre. Bands above the Nyquist frequency take the top bin.
fn band_bins(edges: &[f64], sr: u32, n_fft: usize) -> Vec<(usize, usize)> {
    let hz = f64::from(sr) / n_fft as f64;
    let top = n_fft / 2;
    edges
        .windows(2)
        .map(|w| {
            let lo = (w[0] / hz).ceil() as usize;
            let hi = ((w[1] / hz).ceil() as usize).min(top + 1);
            if lo < hi {
                (lo, hi)
            } else {
                let c = (((w[0] * w[1]).sqrt() / hz).round() as usize).min(top);
                (c, c + 1)
            }
        })
        .collect()
}

/// A spectral analysis of one window length: the FFT plan, the window and the band bins.
struct Spectrum {
    fft: Arc<dyn Fft<f32>>,
    win: Vec<f32>,
    n_fft: usize,
    bands: Vec<(usize, usize)>,
}

impl Spectrum {
    fn new(planner: &mut FftPlanner<f32>, sr: u32, len_s: f64, edges: &[f64]) -> Spectrum {
        let n = ((len_s * f64::from(sr)).round() as usize).max(2);
        let n_fft = n.next_power_of_two();
        let win = (0..n)
            .map(|i| (0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / (n - 1) as f64).cos()) as f32)
            .collect();
        Spectrum {
            fft: planner.plan_fft_forward(n_fft),
            win,
            n_fft,
            bands: band_bins(edges, sr, n_fft),
        }
    }

    /// The band powers in dB of the window starting at sample `start` (may be negative).
    fn bands_db(&self, x: &[f32], start: i64, buf: &mut Vec<Complex<f32>>) -> Vec<f32> {
        buf.clear();
        buf.resize(self.n_fft, Complex::new(0.0, 0.0));
        for (i, w) in self.win.iter().enumerate() {
            let j = start + i as i64;
            if j >= 0 && (j as usize) < x.len() {
                buf[i] = Complex::new(x[j as usize] * w, 0.0);
            }
        }
        self.fft.process(buf);
        self.bands
            .iter()
            .map(|&(a, b)| {
                let p: f32 = buf[a..b].iter().map(|c| c.norm_sqr()).sum::<f32>() / (b - a) as f32;
                10.0 * (p + 1e-20).log10()
            })
            .collect()
    }
}

/// The feature extractor of one signal rate and recipe (the plans and band tables, made once).
pub struct Extractor {
    sr: u32,
    recipe: Recipe,
    attack: Spectrum,
    body: Spectrum,
}

impl Extractor {
    /// The extractor for signals of rate `sr`.
    pub fn new(sr: u32, recipe: Recipe) -> Extractor {
        let top = TOP_HZ.min(f64::from(sr) / 2.0 * 0.95);
        let mut planner = FftPlanner::new();
        let (a_edges, b_edges) = match recipe {
            Recipe::Patch => {
                let mut b = body_edges_patch();
                for e in &mut b {
                    *e = e.min(top);
                }
                (log_edges(ATTACK_LO_HZ, top, ATTACK_BANDS), b)
            }
            Recipe::Mfcc => (
                mel_edges(ATTACK_LO_HZ, top, MEL_BANDS),
                mel_edges(BODY_LO_HZ, top, MEL_BANDS),
            ),
        };
        Extractor {
            sr,
            recipe,
            attack: Spectrum::new(&mut planner, sr, ATTACK_WIN_S, &a_edges),
            body: Spectrum::new(&mut planner, sr, BODY_LEN_S, &b_edges),
        }
    }

    /// The recipe of this extractor.
    pub fn recipe(&self) -> Recipe {
        self.recipe
    }

    fn at(&self, t: f64) -> i64 {
        (t * f64::from(self.sr)).round() as i64
    }

    /// The three blocks at the anchor `t` (seconds) of `x`.
    pub fn blocks(&self, x: &[f32], t: f64) -> Blocks {
        let mut buf = Vec::new();
        let mut attack = Vec::with_capacity(self.recipe.dims()[0]);
        for f in 0..ATTACK_FRAMES {
            let start = self.at(t + ATTACK_START_S + f as f64 * ATTACK_HOP_S);
            let db = self.attack.bands_db(x, start, &mut buf);
            match self.recipe {
                Recipe::Patch => attack.extend(db),
                Recipe::Mfcc => attack.extend(cepstrum(&floor_db(db), MFCC_ATTACK)),
            }
        }
        let body_db = self.body.bands_db(x, self.at(t + BODY_START_S), &mut buf);
        let body = match self.recipe {
            Recipe::Patch => body_db,
            Recipe::Mfcc => cepstrum(&floor_db(body_db), MFCC_BODY),
        };
        let step = (ENV_STEP_S * f64::from(self.sr)).round().max(1.0) as i64;
        let a0 = self.at(t);
        let env: Vec<f32> = (0..ENV_STEPS as i64)
            .map(|s| {
                let (mut e, mut n) = (0f64, 0usize);
                for j in a0 + s * step..a0 + (s + 1) * step {
                    if j >= 0 && (j as usize) < x.len() {
                        e += f64::from(x[j as usize]).powi(2);
                    }
                    n += 1;
                }
                (10.0 * (e / n.max(1) as f64 + 1e-20).log10()) as f32
            })
            .collect();
        Blocks {
            attack: normalise(&attack, self.recipe == Recipe::Patch),
            body: normalise(&body, self.recipe == Recipe::Patch),
            env: normalise(&env, true),
        }
    }
}

/// The floor of [`DB_RANGE`] under the maximum, for the cepstrum input.
fn floor_db(mut v: Vec<f32>) -> Vec<f32> {
    let top = v.iter().cloned().fold(f32::MIN, f32::max);
    for x in &mut v {
        *x = x.max(top - DB_RANGE);
    }
    v
}

/// DCT-II of `v` (orthonormal scaling), coefficients 1 to `n` (c0, the level, dropped).
fn cepstrum(v: &[f32], n: usize) -> Vec<f32> {
    let m = v.len() as f64;
    (1..=n)
        .map(|k| {
            let s: f64 = v
                .iter()
                .enumerate()
                .map(|(i, &x)| f64::from(x) * (std::f64::consts::PI * k as f64 * (i as f64 + 0.5) / m).cos())
                .sum();
            (s * (2.0 / m).sqrt()) as f32
        })
        .collect()
}

/// A block in dB: the floor [`DB_RANGE`] under its maximum (when `db`), the mean removed,
/// L2-normalised; all zeros when it has no shape (a flat block, silence).
pub fn normalise(v: &[f32], db: bool) -> Vec<f32> {
    if v.is_empty() {
        return Vec::new();
    }
    let top = v.iter().cloned().fold(f32::MIN, f32::max);
    let w: Vec<f32> = v.iter().map(|&x| if db { x.max(top - DB_RANGE) } else { x }).collect();
    let mean = w.iter().sum::<f32>() / w.len() as f32;
    let c: Vec<f32> = w.iter().map(|x| x - mean).collect();
    let n = c.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n < 1e-6 {
        vec![0.0; c.len()]
    } else {
        c.iter().map(|x| x / n).collect()
    }
}

/// The three normalised blocks of one candidate.
#[derive(Debug, Clone, PartialEq)]
pub struct Blocks {
    /// The attack spectra (or cepstra), frame after frame.
    pub attack: Vec<f32>,
    /// The body spectrum (or cepstrum).
    pub body: Vec<f32>,
    /// The energy envelope.
    pub env: Vec<f32>,
}

impl Blocks {
    /// The blocks one after the other (the cache layout, [`Recipe::len`] values).
    pub fn flat(&self) -> Vec<f32> {
        let mut v = Vec::with_capacity(self.attack.len() + self.body.len() + self.env.len());
        v.extend(&self.attack);
        v.extend(&self.body);
        v.extend(&self.env);
        v
    }
}

/// The anchor of a detector time `t`: the refinement's steepest rise in its window (`-2..+12` ms,
/// [`RefineWindow::DEFAULT`]) without neighbours, so it does not depend on which neighbours survive.
pub fn anchor(x: &[f32], sr: u32, t: f64) -> f64 {
    refine_sr(x, t, sr, &[], RefineWindow::DEFAULT).0
}

/// The flat feature vectors ([`Blocks::flat`]) of the candidates at the detector times `times`
/// (seconds) in `x`. Runs on all cores.
pub fn candidate_features(x: &[f32], sr: u32, times: &[f64], recipe: Recipe) -> Vec<Vec<f32>> {
    let ex = Extractor::new(sr, recipe);
    times
        .par_iter()
        .map(|&t| ex.blocks(x, anchor(x, sr, t)).flat())
        .collect()
}

/// The mean of the flat vectors `feats` (`n x recipe.len()`, one after the other): the track's
/// typical candidate, which [`centred`] takes off. Zeros for no vector.
pub fn track_mean(feats: &[f32], recipe: Recipe) -> Vec<f32> {
    let dim = recipe.len();
    let mut m = vec![0f32; dim];
    let n = feats.len() / dim;
    for row in feats.chunks_exact(dim) {
        for (a, v) in m.iter_mut().zip(row) {
            *a += v;
        }
    }
    if n > 0 {
        for a in &mut m {
            *a /= n as f32;
        }
    }
    m
}

/// A flat vector relative to the track's typical candidate ([`track_mean`]): each block minus the
/// mean's block, L2-normalised again (a block with nothing left is zeros). Every candidate on one
/// mic shares the mic's colour (its spectral tilt, the bleed it always hears), which makes all
/// cosines high; without it the cosine measures how a candidate differs from the typical one.
pub fn centred(flat: &[f32], mean: &[f32], recipe: Recipe) -> Vec<f32> {
    let mut out = Vec::with_capacity(flat.len());
    let mut i = 0;
    for d in recipe.dims() {
        let end = (i + d).min(flat.len());
        let block: Vec<f32> = flat[i..end]
            .iter()
            .zip(&mean[i..end.min(mean.len())])
            .map(|(v, m)| v - m)
            .collect();
        let n = block.iter().map(|v| v * v).sum::<f32>().sqrt();
        out.extend(block.iter().map(|v| if n > 1e-6 { v / n } else { 0.0 }));
        i += d;
    }
    out
}

/// A flat vector with the block weights applied, normalised as a whole (all zeros stay zeros).
pub fn weigh(flat: &[f32], recipe: Recipe, w: &Weights) -> Vec<f32> {
    let dims = recipe.dims();
    let ws = w.as_array();
    let mut out = Vec::with_capacity(flat.len());
    let mut i = 0;
    for (d, wb) in dims.iter().zip(ws) {
        out.extend(flat[i..(i + d).min(flat.len())].iter().map(|v| v * wb));
        i += d;
    }
    let n = out.iter().map(|v| v * v).sum::<f32>().sqrt();
    if n > 1e-9 {
        for v in &mut out {
            *v /= n;
        }
    }
    out
}

/// The cosine of two weighted vectors ([`weigh`]): their dot product.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// The mean of the `min(K_NEAREST, n)` highest similarities of `c` to `examples`; 0 without examples.
pub fn nearest_mean(c: &[f32], examples: &[&[f32]]) -> f32 {
    if examples.is_empty() {
        return 0.0;
    }
    let mut s: Vec<f32> = examples.iter().map(|e| cosine(c, e)).collect();
    s.sort_by(|a, b| b.total_cmp(a));
    let k = K_NEAREST.min(s.len());
    s[..k].iter().sum::<f32>() / k as f32
}

/// The score of section 3.1 of a weighted vector `c` against the positive and negative examples.
pub fn score(c: &[f32], pos: &[&[f32]], neg: &[&[f32]]) -> f32 {
    let sp = nearest_mean(c, pos);
    let sn = if neg.is_empty() { 0.0 } else { nearest_mean(c, neg) };
    ((1.0 + sp - sn.max(0.0)) / 2.0).clamp(0.0, 1.0)
}

/// The scores of the examples themselves, each without itself (leave one out): `(this, not)`.
/// A "this" example with no other positive scores against the negatives only.
pub fn example_scores(pos: &[&[f32]], neg: &[&[f32]]) -> (Vec<f32>, Vec<f32>) {
    fn others<'a>(v: &[&'a [f32]], i: usize) -> Vec<&'a [f32]> {
        v.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, e)| *e).collect()
    }
    let this = (0..pos.len()).map(|i| score(pos[i], &others(pos, i), neg)).collect();
    let not = (0..neg.len()).map(|i| score(neg[i], pos, &others(neg, i))).collect();
    (this, not)
}

/// Bins of the Similarity histogram (concept 3.2).
pub const HIST_BINS: usize = 50;
/// The automatic threshold ignores scores under this (concept 3.2: the search starts at 0.3).
pub const AUTO_LO: f32 = 0.3;
/// The threshold when nothing better is known (concept 3.2, fewer than 4 candidates).
pub const DEFAULT_THRESHOLD: f32 = 0.5;

/// The automatic threshold looks for the emptiest bins this far on each side of the boundary.
pub const AUTO_WINDOW: f32 = 0.05;

fn median(v: &[f32]) -> f32 {
    let mut s = v.to_vec();
    s.sort_by(f32::total_cmp);
    let n = s.len();
    if n % 2 == 1 {
        s[n / 2]
    } else {
        (s[n / 2 - 1] + s[n / 2]) / 2.0
    }
}

/// The automatic threshold (concept 3.2, "mind the gap"), from the scores `cands` of the candidates
/// that pass the Sensitivity (the solid bars) and the "this" examples' own scores (each without itself,
/// [`example_scores`]):
///
/// 1. Two groups, the hits and the rest, by 2-means on the scores of [`AUTO_LO`] and more: the hits'
///    centre starts at the median "this" score, the rest's at the median of the scores 0.1 or more
///    under it; the boundary is the middle of the two centres.
/// 2. The gap: in the histogram of [`HIST_BINS`] bins, the widest run of the lowest count within
///    [`AUTO_WINDOW`] of the boundary; the threshold is its middle (an empty stretch is the lowest count).
///
/// No score 0.1 under the examples (nothing that looks like bleed): [`AUTO_LO`], every candidate stays.
/// Fewer than 4 candidates or no "this" score: [`DEFAULT_THRESHOLD`]. The "not" scores do not move it:
/// a forced "not" example goes anyway, and a negative that sounds like the hits (bleed of the same drum,
/// a hit the reference lacks) must not drag the threshold into the hits.
pub fn auto_threshold(cands: &[f32], this: &[f32], _not: &[f32]) -> f32 {
    if this.is_empty() || cands.len() < 4 {
        return DEFAULT_THRESHOLD;
    }
    let s: Vec<f32> = cands.iter().copied().filter(|v| *v >= AUTO_LO).collect();
    let mut ct = median(this);
    let below: Vec<f32> = s.iter().copied().filter(|v| *v < ct - 0.1).collect();
    if below.is_empty() {
        return AUTO_LO;
    }
    let mut cf = median(&below);
    let mean = |v: &mut dyn Iterator<Item = f32>| {
        let (sum, n) = v.fold((0f32, 0usize), |(a, n), x| (a + x, n + 1));
        (n > 0).then(|| sum / n as f32)
    };
    for _ in 0..30 {
        let b = (ct + cf) / 2.0;
        match (
            mean(&mut s.iter().copied().filter(|v| *v >= b)),
            mean(&mut s.iter().copied().filter(|v| *v < b)),
        ) {
            (Some(t), Some(f)) => (ct, cf) = (t, f),
            _ => break,
        }
    }
    let b = (ct + cf) / 2.0;
    let bin = |v: f32| ((v.clamp(0.0, 1.0) * HIST_BINS as f32) as usize).min(HIST_BINS - 1);
    let mut h = [0usize; HIST_BINS];
    for &v in &s {
        h[bin(v)] += 1;
    }
    let (b0, b1) = (bin(b - AUTO_WINDOW), bin(b + AUTO_WINDOW));
    let low = h[b0..=b1].iter().copied().min().expect("a non-empty window");
    let (mut best, mut run): (Option<(usize, usize)>, Option<usize>) = (None, None);
    for (i, &c) in h.iter().enumerate().take(b1 + 1).skip(b0) {
        if c == low {
            let st = *run.get_or_insert(i);
            if best.is_none_or(|(x, y)| i - st > y - x) {
                best = Some((st, i));
            }
        } else {
            run = None;
        }
    }
    let (x, y) = best.expect("the window holds its lowest bin");
    ((x + y + 1) as f32 / 2.0) / HIST_BINS as f32
}

/// P4-6c: the suggested threshold with "not" examples only: a candidate goes when its similarity to the
/// nearest "not" examples is 0.9 or more (score `(1 - sn) / 2` under 0.05), so in practice the marked hit
/// and its near copies.
pub const SUGGEST_NOT_ONLY: f32 = 0.05;
/// P4-6c: the suggested threshold with both kinds of examples: a candidate goes when its similarity to the
/// nearest "not" examples exceeds the one to the nearest "this" examples by 0.2 or more (score under 0.4).
pub const SUGGEST_BOTH: f32 = 0.4;

/// P4-6c: the threshold the Learn filter uses while the track's `learn.auto` is set (concept 3.2: it follows
/// the examples until the engineer drags it). `cands` are the scores of the candidates that pass the
/// Sensitivity, `this` and `not` the examples' own scores ([`example_scores`]):
///
/// * no example: [`DEFAULT_THRESHOLD`] (every candidate scores exactly that and stays);
/// * "not" examples only: [`SUGGEST_NOT_ONLY`];
/// * "this" examples only: [`auto_threshold`];
/// * both: [`SUGGEST_BOTH`].
pub fn suggest_threshold(cands: &[f32], this: &[f32], not: &[f32]) -> f32 {
    match (this.is_empty(), not.is_empty()) {
        (true, true) => DEFAULT_THRESHOLD,
        (true, false) => SUGGEST_NOT_ONLY,
        (false, true) => auto_threshold(cands, this, not),
        (false, false) => SUGGEST_BOTH,
    }
}

/// The Learn filter on the detector's hits (concept 3.3: after the detection, before the role
/// filter's dedup): a hit `(time, note)` stays when the score of its frame is at or above
/// `threshold` (`scores` holds `(frame, score)` sorted by frame; a frame without a score stays).
/// Then the forced examples: the hit nearest to each `not` time within `reach` seconds goes, and a
/// `this` time without a hit within `reach` comes back as `(time, note)`. Sorted by time and note.
pub fn filter_detected(
    detected: &[(f64, u8)],
    scores: &[(usize, f32)],
    threshold: f32,
    this: &[f64],
    not: &[f64],
    note: u8,
    reach: f64,
) -> Vec<(f64, u8)> {
    let score_of = |t: f64| {
        let f = (t * FPS).round().max(0.0) as usize;
        scores.binary_search_by_key(&f, |s| s.0).ok().map(|i| scores[i].1)
    };
    let mut out: Vec<(f64, u8)> = detected
        .iter()
        .filter(|(t, _)| score_of(*t).is_none_or(|s| s >= threshold))
        .copied()
        .collect();
    for &n in not {
        let near = out
            .iter()
            .enumerate()
            .filter(|(_, (t, _))| (t - n).abs() <= reach)
            .min_by(|a, b| (a.1 .0 - n).abs().total_cmp(&(b.1 .0 - n).abs()))
            .map(|(i, _)| i);
        if let Some(i) = near {
            out.remove(i);
        }
    }
    for &p in this {
        if !out.iter().any(|(t, _)| (t - p).abs() <= reach) && !not.iter().any(|n| (n - p).abs() < 1e-9) {
            out.push((p, note));
        }
    }
    out.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    out
}

#[cfg(test)]
mod tests;
