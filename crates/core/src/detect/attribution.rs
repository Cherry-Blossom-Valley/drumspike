//! Cross-mic attribution flags (P2-6): `bleed` and `resonance`, off by default.
//!
//! The flags only mark hits. Nothing is removed from a hit list: the app shows the flag and
//! the engineer deletes or keeps the hit. Measurements and the tuning history are in
//! `docs/archive/phase2-report.md`, section "P2-6 Attribution flags".
//!
//! # Input and output
//!
//! [`attribute`] takes the tracks of one session ([`TrackInput`]: id, [`Role`], instrument
//! name, the mono signal at the session rate, and the refined hits as sample positions with
//! the phase 1 `rise_db`). It gives one [`HitFlags`] per hit in the input order, plus the
//! transfer gains it calibrated ([`Transfer`]).
//!
//! # Bleed
//!
//! A hit on track A at sample `s_a` is bleed of track B when all of these are true:
//!
//! 1. B is a different instrument (two mics of one drum, for example two overheads, never
//!    flag each other) and A's role is in [`AttributionParams::bleed_roles`].
//! 2. B has an attack near `s_a`: [`crate::refine::refine_sr`] on B from `s_a` (the phase 1
//!    refinement with its symmetric +-7 ms search, [`RefineWindow::SYMMETRIC_7`]: `s_a` is an
//!    attack, not an early detector time) gives the attack `s_b` with `rise_db >= source_rise_db`,
//!    and one of B's own hits is within `source_match_ms` of `s_b`.
//! 3. Arrival order: B hears it first. `s_b - s_a` is in `[-window_ms, lag_tol_ms]`.
//!    Snare to tom and kick to tom bleed arrives 1 to 3 ms later on the tom mic, so the
//!    window is a few ms wide; `lag_tol_ms` = 0 allows a tie.
//! 4. A's level is explained by B: the excess `ex_ab = L_A(A) - L_B(A) - G(B->A)` is below
//!    `margin_db`. `L_X(Y)` is the peak level of track X in the frequency band of Y's role
//!    ([`Role::band`]), measured at `s_b`: X = A over `pair_window_ms` (the bleed arrives
//!    later on A), X = B over `own_window_ms`. `G(B->A)` is the transfer gain from B to A
//!    in A's band, calibrated on the session ([`Transfer`]).
//! 5. B's level is not explained by A: the reverse excess `ex_ba` (the same measurement
//!    with A as the source, in B's band, at `s_a`) is at least `dominance_db` above
//!    `ex_ab`. This is the "higher level at the source" test in calibrated form: two mics
//!    that hear each other's drums equally (a snare mic and an overhead aligned to it) give
//!    `ex_ab` close to `ex_ba`, and the hit stays unflagged.
//!
//! Of all sources that pass, the one with the lowest `ex_ab` is reported. Confidence:
//! `0.5 + 0.5 * clamp((margin_db - ex_ab) / 12 dB, 0, 1)`.
//!
//! Transfer gain `G(B->A)` (band of A): over B's strong hits (broadband peak in
//! `own_window_ms` within `calib_strong_db` of B's 90th percentile), the value
//! `L_A(A) - L_B(A)` at each hit as in rule 4. Only hits where A has no own hit within
//! `calib_quiet_ms` are used when there are at least `calib_min_hits` of them (else all
//! strong hits); `G` is their `calib_percentile` percentile (numpy's linear rule). A low
//! percentile because any second source can only add level on A. A value where either
//! level is at or below `silence_db` is skipped (a gated mic is digital silence between its
//! own hits and says nothing about the transfer); the same holds for `ex_ab` and `ex_ba`,
//! so a source that is silent in the target's window is no source. Fewer than
//! `calib_min_hits` values: no gain, B is no source for A.
//!
//! Band levels: a cascade of RBJ biquads (each stage twice, 24 dB per octave) in `f64`,
//! direct form II transposed, zero initial state, run over `preroll_ms` before the window
//! and the window itself; the level is `20 log10(max |y| + 1e-9)` inside the window. The
//! same recursion as `scipy.signal.sosfilt`, so the Python reference
//! (`tools/parity/check_attribution.py`) gives the same values.
//!
//! # Resonance
//!
//! A hit on a track whose role is in [`AttributionParams::resonance_roles`] is resonance
//! when its `rise_db` (the phase 1 envelope rise over 0.5 ms) is more than
//! `resonance_db` below the median `rise_db` of the track's hits (at least
//! `calib_min_hits` hits with a rise). A sympathetic tom ring or a snare buzz has no
//! stick attack. Confidence: `0.5 + 0.5 * clamp((median - resonance_db - rise) / 12 dB, 0,
//! 1)`.
//!
//! # Determinism
//!
//! Every value is computed in a fixed order in `f64`; window bounds use
//! `floor(ms * sr / 1000 + 0.5)` samples. The result does not depend on the thread count.

use crate::refine::{RefineWindow, refine_sr};

/// The track role as the app names it (`engine.ROLE_CLASSES`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    /// Kick drum close mic.
    Kick,
    /// Snare close mic.
    Snare,
    /// Tom close mic.
    Tom,
    /// Hi-hat mic.
    Hat,
    /// Overhead or cymbal mic.
    Cymbal,
    /// Any other track (no band limit).
    Other,
}

/// The kind of one filter stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterKind {
    /// Second-order lowpass (RBJ cookbook, Q = 1/sqrt(2)).
    LowPass,
    /// Second-order highpass (RBJ cookbook, Q = 1/sqrt(2)).
    HighPass,
}

/// One stage of a band: the kind and the corner frequency in Hz. Each stage runs twice.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BandStage {
    /// Lowpass or highpass.
    pub kind: FilterKind,
    /// Corner frequency in Hz.
    pub freq_hz: f64,
}

const fn lp(freq_hz: f64) -> BandStage {
    BandStage {
        kind: FilterKind::LowPass,
        freq_hz,
    }
}

const fn hp(freq_hz: f64) -> BandStage {
    BandStage {
        kind: FilterKind::HighPass,
        freq_hz,
    }
}

const BAND_KICK: [BandStage; 1] = [lp(150.0)];
const BAND_TOM: [BandStage; 2] = [hp(50.0), lp(400.0)];
const BAND_SNARE: [BandStage; 2] = [hp(150.0), lp(2500.0)];
const BAND_HIGH: [BandStage; 1] = [hp(5000.0)];

impl Role {
    /// The role for an app role name (`kick`, `snare`, `tom`, `hat`, `cymbal`, `other`).
    #[must_use]
    pub fn from_name(name: &str) -> Option<Role> {
        match name {
            "kick" => Some(Role::Kick),
            "snare" => Some(Role::Snare),
            "tom" => Some(Role::Tom),
            "hat" => Some(Role::Hat),
            "cymbal" => Some(Role::Cymbal),
            "other" => Some(Role::Other),
            _ => None,
        }
    }

    /// The app role name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Role::Kick => "kick",
            Role::Snare => "snare",
            Role::Tom => "tom",
            Role::Hat => "hat",
            Role::Cymbal => "cymbal",
            Role::Other => "other",
        }
    }

    /// The frequency band in which a track of this role is judged: kick below 150 Hz, tom
    /// 50 to 400 Hz, snare 150 to 2,500 Hz, hat and cymbal above 5 kHz, other broadband.
    #[must_use]
    pub fn band(self) -> &'static [BandStage] {
        match self {
            Role::Kick => &BAND_KICK,
            Role::Tom => &BAND_TOM,
            Role::Snare => &BAND_SNARE,
            Role::Hat | Role::Cymbal => &BAND_HIGH,
            Role::Other => &[],
        }
    }
}

/// Normalised biquad coefficients (`a0` = 1), in the row order of a scipy `sos` section.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Biquad {
    /// Feed-forward coefficients.
    pub b: [f64; 3],
    /// Feedback coefficients `a1`, `a2`.
    pub a: [f64; 2],
}

impl Biquad {
    /// RBJ cookbook lowpass or highpass at `freq_hz`, Q = 1/sqrt(2), for rate `sr`.
    #[must_use]
    pub fn new(kind: FilterKind, freq_hz: f64, sr: u32) -> Biquad {
        let w = 2.0 * std::f64::consts::PI * freq_hz / f64::from(sr);
        let (cw, sw) = (w.cos(), w.sin());
        let alpha = sw / (2.0 * std::f64::consts::FRAC_1_SQRT_2);
        let b = match kind {
            FilterKind::LowPass => [(1.0 - cw) / 2.0, 1.0 - cw, (1.0 - cw) / 2.0],
            FilterKind::HighPass => [(1.0 + cw) / 2.0, -(1.0 + cw), (1.0 + cw) / 2.0],
        };
        let a0 = 1.0 + alpha;
        Biquad {
            b: [b[0] / a0, b[1] / a0, b[2] / a0],
            a: [-2.0 * cw / a0, (1.0 - alpha) / a0],
        }
    }
}

/// The sections of a band at rate `sr`: every stage twice, in order.
#[must_use]
pub fn band_sections(band: &[BandStage], sr: u32) -> Vec<Biquad> {
    band.iter()
        .flat_map(|s| {
            let q = Biquad::new(s.kind, s.freq_hz, sr);
            [q, q]
        })
        .collect()
}

/// Filter `x[from..to]` through `sections` (direct form II transposed, zero state at
/// `from`, the recursion of `scipy.signal.sosfilt`) and return
/// `20 log10(max |y| + 1e-9)` over the samples `y[w0..to]`. `from <= w0 <= to <= x.len()`;
/// an empty window gives -180 dB.
#[must_use]
pub fn band_peak_db(x: &[f32], sections: &[Biquad], from: usize, w0: usize, to: usize) -> f64 {
    if w0 >= to {
        return -180.0;
    }
    let mut z = vec![[0.0f64; 2]; sections.len()];
    let mut peak = 0.0f64;
    for (i, &v) in x[from..to].iter().enumerate() {
        let mut y = f64::from(v);
        for (s, zs) in sections.iter().zip(z.iter_mut()) {
            let out = s.b[0] * y + zs[0];
            zs[0] = s.b[1] * y - s.a[0] * out + zs[1];
            zs[1] = s.b[2] * y - s.a[1] * out;
            y = out;
        }
        if from + i >= w0 {
            peak = peak.max(y.abs());
        }
    }
    20.0 * (peak + 1e-9).log10()
}

/// The `q`-th percentile (0 to 100) of `v` with numpy's default linear interpolation;
/// `None` for an empty slice. NaN values are not expected.
pub fn percentile(v: &[f64], q: f64) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    let pos = q / 100.0 * (s.len() - 1) as f64;
    let lo = pos.floor() as usize;
    let hi = (lo + 1).min(s.len() - 1);
    let frac = pos - lo as f64;
    Some(s[lo] + (s[hi] - s[lo]) * frac)
}

/// Milliseconds to samples: `floor(ms * sr / 1000 + 0.5)` (the same rule in the Python
/// reference; Python's `round` rounds half to even and would differ).
#[must_use]
pub fn ms_to_samples(ms: f64, sr: u32) -> i64 {
    (ms * f64::from(sr) / 1000.0 + 0.5).floor() as i64
}

/// The calibration of a transfer gain `G(B->A)` from its values (P2-6; P4-14 reuses it): `all` holds the value at
/// every strong hit of B, `quiet` the values at the strong hits where A has no own hit. The `pct` percentile of
/// `quiet` when it has at least `min_hits` values, else of `all`; `None` when `all` has fewer than `min_hits` (at
/// least 1). Returns the gain, the number of values used and whether only quiet values were used.
#[must_use]
pub fn calibrate_gain(all: &[f64], quiet: &[f64], pct: f64, min_hits: usize) -> Option<(f64, usize, bool)> {
    let min = min_hits.max(1);
    if all.len() < min {
        return None;
    }
    let quiet_only = quiet.len() >= min;
    let vals = if quiet_only { quiet } else { all };
    percentile(vals, pct).map(|g| (g, vals.len(), quiet_only))
}

/// One refined hit of a track.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HitInput {
    /// The refined attack as a sample index at the session rate.
    pub sample: i64,
    /// The phase 1 `rise_db` of the refinement, when it gave one.
    pub rise: Option<f64>,
}

/// One track of the session.
#[derive(Debug, Clone)]
pub struct TrackInput<'a> {
    /// Track id (reported back unchanged by the caller; not used here).
    pub id: String,
    /// The track role.
    pub role: Role,
    /// The drum this mic belongs to. Tracks with the same instrument never flag each other
    /// (for example `oh` for both overheads, `snare` for top and bottom).
    pub instrument: String,
    /// Mono signal at the session rate.
    pub signal: &'a [f32],
    /// The track's hits.
    pub hits: Vec<HitInput>,
}

/// Parameters. [`Default`] gives the tuned values with both flags off.
#[derive(Debug, Clone, PartialEq)]
pub struct AttributionParams {
    /// Compute the `bleed` flag (default false: off).
    pub bleed_enabled: bool,
    /// Compute the `resonance` flag (default false: off).
    pub resonance_enabled: bool,
    /// Roles whose hits can get `bleed`.
    pub bleed_roles: Vec<Role>,
    /// Roles whose hits can get `resonance`.
    pub resonance_roles: Vec<Role>,
    /// The source attack may be up to this many ms before the hit.
    pub window_ms: f64,
    /// The source attack may be up to this many ms after the hit.
    pub lag_tol_ms: f64,
    /// Minimum `rise_db` of the source attack.
    pub source_rise_db: f64,
    /// A listed hit of the source must be this close to its attack (ms).
    pub source_match_ms: f64,
    /// `ex_ab` must be below this (dB).
    pub margin_db: f64,
    /// `ex_ba - ex_ab` must be at least this (dB).
    pub dominance_db: f64,
    /// Level window of the source, relative to its attack (ms).
    pub own_window_ms: (f64, f64),
    /// Level window of the target, relative to the source attack (ms).
    pub pair_window_ms: (f64, f64),
    /// Filter run-in before a level window (ms).
    pub preroll_ms: f64,
    /// Band levels at or below this (dBFS) count as silence: no transfer value there.
    pub silence_db: f64,
    /// Strong source hits: within this many dB of the source's 90th percentile peak.
    pub calib_strong_db: f64,
    /// The target is quiet when it has no own hit within this many ms.
    pub calib_quiet_ms: f64,
    /// Percentile of the transfer values.
    pub calib_percentile: f64,
    /// Minimum number of hits for a transfer gain or a rise median.
    pub calib_min_hits: usize,
    /// `resonance` when `rise_db` is more than this many dB below the track median.
    pub resonance_db: f64,
}

impl Default for AttributionParams {
    fn default() -> Self {
        AttributionParams {
            bleed_enabled: false,
            resonance_enabled: false,
            bleed_roles: vec![Role::Kick, Role::Tom],
            resonance_roles: vec![Role::Tom],
            window_ms: 5.0,
            lag_tol_ms: 0.0,
            source_rise_db: 10.0,
            source_match_ms: 2.0,
            margin_db: 6.0,
            dominance_db: 12.0,
            own_window_ms: (-0.5, 5.0),
            pair_window_ms: (-0.5, 8.0),
            preroll_ms: 50.0,
            silence_db: -100.0,
            calib_strong_db: 6.0,
            calib_quiet_ms: 30.0,
            calib_percentile: 10.0,
            calib_min_hits: 5,
            resonance_db: 10.0,
        }
    }
}

impl AttributionParams {
    /// The defaults with both flags on.
    #[must_use]
    pub fn enabled() -> Self {
        AttributionParams {
            bleed_enabled: true,
            resonance_enabled: true,
            ..Default::default()
        }
    }
}

/// The `bleed` flag of one hit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BleedFlag {
    /// Index of the source track in the input.
    pub source: usize,
    /// Source attack minus hit, in samples (negative: the source heard it first).
    pub lag_samples: i64,
    /// `ex_ab` in dB.
    pub excess_db: f64,
    /// `ex_ba - ex_ab` in dB.
    pub dominance_db: f64,
    /// 0.5 to 1.
    pub confidence: f64,
}

/// The `resonance` flag of one hit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResonanceFlag {
    /// The hit's `rise_db`.
    pub rise_db: f64,
    /// The median `rise_db` of the track.
    pub median_db: f64,
    /// 0.5 to 1.
    pub confidence: f64,
}

/// The flag set of one hit.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct HitFlags {
    /// Set when the hit is bleed of another mic.
    pub bleed: Option<BleedFlag>,
    /// Set when the hit has no attack of its own.
    pub resonance: Option<ResonanceFlag>,
}

impl HitFlags {
    /// True when any flag is set.
    #[must_use]
    pub fn is_flagged(&self) -> bool {
        self.bleed.is_some() || self.resonance.is_some()
    }

    /// The highest confidence of the set flags, 0 when none is set.
    #[must_use]
    pub fn confidence(&self) -> f64 {
        let b = self.bleed.map_or(0.0, |f| f.confidence);
        let r = self.resonance.map_or(0.0, |f| f.confidence);
        b.max(r)
    }

    /// The names of the set flags (`bleed`, `resonance`).
    #[must_use]
    pub fn names(&self) -> Vec<&'static str> {
        let mut v = Vec::new();
        if self.bleed.is_some() {
            v.push("bleed");
        }
        if self.resonance.is_some() {
            v.push("resonance");
        }
        v
    }
}

/// A calibrated transfer gain from one track to another (in the target's band).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transfer {
    /// Source track index.
    pub source: usize,
    /// Target track index.
    pub target: usize,
    /// `G(source -> target)` in dB.
    pub gain_db: f64,
    /// Number of values the percentile used.
    pub hits: usize,
    /// True when only hits with a quiet target were used.
    pub quiet_only: bool,
}

/// The result of [`attribute`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Attribution {
    /// Per track, per hit (input order).
    pub flags: Vec<Vec<HitFlags>>,
    /// Every transfer gain that was calibrated (empty when `bleed` is off).
    pub transfers: Vec<Transfer>,
}

/// Errors of [`attribute`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttributionError {
    /// The sample rate is 0.
    ZeroRate,
}

impl std::fmt::Display for AttributionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AttributionError::ZeroRate => write!(f, "sample rate is 0"),
        }
    }
}

impl std::error::Error for AttributionError {}

/// Per-session working state: sections per role, window lengths in samples.
struct Ctx<'a> {
    tracks: &'a [TrackInput<'a>],
    sections: Vec<Vec<Biquad>>,
    own: (i64, i64),
    pair: (i64, i64),
    preroll: i64,
    silence: f64,
}

impl Ctx<'_> {
    /// Band level of track `x` in the band of track `band_of`, window `[c + w.0, c + w.1)`.
    fn level(&self, x: usize, band_of: usize, c: i64, w: (i64, i64)) -> f64 {
        let sig = self.tracks[x].signal;
        let n = sig.len() as i64;
        let w0 = (c + w.0).clamp(0, n);
        let to = (c + w.1).clamp(0, n);
        let from = (w0 - self.preroll).max(0);
        band_peak_db(sig, &self.sections[band_of], from as usize, w0 as usize, to as usize)
    }

    /// `L_target(target band) - L_source(target band)` at the source attack `c`; `None`
    /// when either level is at or below `silence_db` (a gated or muted mic carries no
    /// information about the transfer).
    fn transfer_value(&self, source: usize, target: usize, c: i64) -> Option<f64> {
        let lt = self.level(target, target, c, self.pair);
        let ls = self.level(source, target, c, self.own);
        (lt > self.silence && ls > self.silence).then_some(lt - ls)
    }
}

fn broadband_peak_db(x: &[f32], c: i64, w: (i64, i64)) -> f64 {
    let n = x.len() as i64;
    let w0 = (c + w.0).clamp(0, n);
    let to = (c + w.1).clamp(0, n);
    band_peak_db(x, &[], w0 as usize, w0 as usize, to as usize)
}

/// True when `sorted` has a value within `tol` of `v`.
fn has_near(sorted: &[i64], v: i64, tol: i64) -> bool {
    let k = sorted.partition_point(|&s| s < v - tol);
    k < sorted.len() && sorted[k] <= v + tol
}

/// Compute the flags of every hit of one session (see the module documentation). All
/// signals are at rate `sr`. With both flags off every [`HitFlags`] is empty.
///
/// # Errors
///
/// [`AttributionError::ZeroRate`] when `sr` is 0.
pub fn attribute(tracks: &[TrackInput], sr: u32, p: &AttributionParams) -> Result<Attribution, AttributionError> {
    if sr == 0 {
        return Err(AttributionError::ZeroRate);
    }
    let mut out = Attribution {
        flags: tracks.iter().map(|t| vec![HitFlags::default(); t.hits.len()]).collect(),
        transfers: Vec::new(),
    };
    if p.resonance_enabled {
        resonance(tracks, p, &mut out);
    }
    if p.bleed_enabled {
        bleed(tracks, sr, p, &mut out);
    }
    Ok(out)
}

fn resonance(tracks: &[TrackInput], p: &AttributionParams, out: &mut Attribution) {
    for (ti, t) in tracks.iter().enumerate() {
        if !p.resonance_roles.contains(&t.role) {
            continue;
        }
        let rises: Vec<f64> = t.hits.iter().filter_map(|h| h.rise).collect();
        if rises.len() < p.calib_min_hits.max(1) {
            continue;
        }
        let med = percentile(&rises, 50.0).expect("non-empty");
        for (hi, h) in t.hits.iter().enumerate() {
            let Some(r) = h.rise else { continue };
            let below = med - p.resonance_db - r;
            if below > 0.0 {
                out.flags[ti][hi].resonance = Some(ResonanceFlag {
                    rise_db: r,
                    median_db: med,
                    confidence: 0.5 + 0.5 * (below / 12.0).clamp(0.0, 1.0),
                });
            }
        }
    }
}

fn bleed(tracks: &[TrackInput], sr: u32, p: &AttributionParams, out: &mut Attribution) {
    let n = tracks.len();
    let ctx = Ctx {
        tracks,
        sections: tracks.iter().map(|t| band_sections(t.role.band(), sr)).collect(),
        own: (
            ms_to_samples(p.own_window_ms.0, sr),
            ms_to_samples(p.own_window_ms.1, sr),
        ),
        pair: (
            ms_to_samples(p.pair_window_ms.0, sr),
            ms_to_samples(p.pair_window_ms.1, sr),
        ),
        preroll: ms_to_samples(p.preroll_ms, sr),
        silence: p.silence_db,
    };
    let sorted: Vec<Vec<i64>> = tracks
        .iter()
        .map(|t| {
            let mut v: Vec<i64> = t.hits.iter().map(|h| h.sample).collect();
            v.sort_unstable();
            v
        })
        .collect();
    // strong hits per track: broadband peak within calib_strong_db of the 90th percentile
    let strong: Vec<Vec<i64>> = tracks
        .iter()
        .map(|t| {
            let pk: Vec<f64> = t
                .hits
                .iter()
                .map(|h| broadband_peak_db(t.signal, h.sample, ctx.own))
                .collect();
            match percentile(&pk, 90.0) {
                Some(p90) => t
                    .hits
                    .iter()
                    .zip(&pk)
                    .filter(|(_, v)| **v >= p90 - p.calib_strong_db)
                    .map(|(h, _)| h.sample)
                    .collect(),
                None => Vec::new(),
            }
        })
        .collect();
    let quiet = ms_to_samples(p.calib_quiet_ms, sr);
    let mut gain: Vec<Vec<Option<f64>>> = vec![vec![None; n]; n];
    for src in 0..n {
        if strong[src].len() < p.calib_min_hits.max(1) {
            continue;
        }
        for tgt in 0..n {
            if tgt == src || tracks[tgt].instrument == tracks[src].instrument {
                continue;
            }
            let mut all = Vec::with_capacity(strong[src].len());
            let mut silent = Vec::new();
            for &c in &strong[src] {
                let Some(v) = ctx.transfer_value(src, tgt, c) else {
                    continue;
                };
                all.push(v);
                if !has_near(&sorted[tgt], c, quiet) {
                    silent.push(v);
                }
            }
            let Some((g, hits, quiet_only)) = calibrate_gain(&all, &silent, p.calib_percentile, p.calib_min_hits)
            else {
                continue;
            };
            gain[src][tgt] = Some(g);
            out.transfers.push(Transfer {
                source: src,
                target: tgt,
                gain_db: g,
                hits,
                quiet_only,
            });
        }
    }
    let w = ms_to_samples(p.window_ms, sr);
    let tol = ms_to_samples(p.lag_tol_ms, sr);
    let mtol = ms_to_samples(p.source_match_ms, sr);
    for a in 0..n {
        if !p.bleed_roles.contains(&tracks[a].role) {
            continue;
        }
        for (hi, h) in tracks[a].hits.iter().enumerate() {
            let sa = h.sample;
            let mut best: Option<BleedFlag> = None;
            for b in 0..n {
                let (Some(g_ba), Some(g_ab)) = (gain[b][a], gain[a][b]) else {
                    continue;
                };
                let (tb, rise_b) = refine_sr(
                    tracks[b].signal,
                    sa as f64 / f64::from(sr),
                    sr,
                    &[],
                    RefineWindow::SYMMETRIC_7,
                );
                let Some(rise_b) = rise_b else { continue };
                let sb = (tb * f64::from(sr)).round() as i64;
                let lag = sb - sa;
                if lag < -w || lag > tol || rise_b < p.source_rise_db || !has_near(&sorted[b], sb, mtol) {
                    continue;
                }
                let Some(v_ab) = ctx.transfer_value(b, a, sb) else {
                    continue;
                };
                let ex_ab = v_ab - g_ba;
                if ex_ab >= p.margin_db {
                    continue;
                }
                let Some(v_ba) = ctx.transfer_value(a, b, sa) else {
                    continue;
                };
                let ex_ba = v_ba - g_ab;
                if ex_ba - ex_ab < p.dominance_db {
                    continue;
                }
                if best.is_none_or(|f| ex_ab < f.excess_db) {
                    best = Some(BleedFlag {
                        source: b,
                        lag_samples: lag,
                        excess_db: ex_ab,
                        dominance_db: ex_ba - ex_ab,
                        confidence: 0.5 + 0.5 * ((p.margin_db - ex_ab) / 12.0).clamp(0.0, 1.0),
                    });
                }
            }
            out.flags[a][hi].bleed = best;
        }
    }
}

#[cfg(test)]
mod tests;
