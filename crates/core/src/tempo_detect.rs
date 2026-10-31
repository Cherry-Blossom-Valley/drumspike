//! P4-8 stage B: suggest a tempo map from the kick and snare hits.
//!
//! The detection reads hit times and velocities only (no audio). It suggests the start tempo,
//! the time of bar 1 beat 1, the meter (4/4 unless the evidence for 3/4 is clear) and tempo steps
//! where a new tempo holds for at least [`DetectParams::min_bars`] bars. It never changes a map:
//! the caller shows the suggestion and stores it with `tempo_map_set`.
//!
//! The steps of the algorithm:
//! 0. Preparation: hits after the tracks' end are dropped, two mics of one drum merged, and a
//!    hit with no other hit within 6 s is left out (it carries no period and would lever the fit).
//! 1. Global beat period: the histogram of all inter-onset intervals up to 6 s (the
//!    autocorrelation of the hit train), read by a comb (the mean of the histogram at 1 to K
//!    times the period). The octave (half, double, 3/2 ...) is chosen by the comb, a wide
//!    prior around 120 BPM and the snare backbeat (snare to snare mostly 2 beats, not 1 or 3,
//!    and no snare period shorter than 2 beats: that rules out 3 and 5 beats as "the beat").
//!    #472: without a snare (a kick-only session) the octave referee reads the phase shares: a
//!    candidate with more kick weight a quarter beat off its grid than on it is the half
//!    tempo, and its double takes the vote.
//!    #130: with a snare, the backbeat verdict is corroborated by the shares before it
//!    decides: a pick that carries the off-beat 8ths of a finer grid while its double holds
//!    the weight on the beat with a nearly as strong comb is the half tempo of a half-time
//!    backbeat, and its double takes the vote (`DRUMSPIKE_TEMPO_RULE=1` pins the shipped
//!    referee).
//! 2. Local period: the same comb in windows of 4 bars (at least 8 s), hop 1 bar, limited to
//!    0.69 to 1.45 times the global period (no octave jumps). A dynamic programme cuts the local
//!    periods into constant pieces of at least `min_bars` windows (L1 cost, a penalty per piece).
//! 3. Beat tracking: a phase-locked loop walks beat by beat with the period of its piece; near a
//!    piece boundary it takes whichever of the two periods the hits confirm. The phase and the
//!    support weigh each hit by its role and its velocity (a ghost note counts little).
//! 4. Meter and beat 1: the pattern per beat (kick minus snare on the beat, kicks and snares
//!    between the beats); its correlation at 2, 3 and 4 beats decides the meter. Beat 1 in 4/4:
//!    the snare backbeat picks the pair of beats (1 and 3 against 2 and 4), then kicks on the
//!    beat or an eighth before it, crash hits (named cymbal tracks), fill endings and the first
//!    kick pick beat 1 of the pair.
//! 5. Exact map: one least-squares fit of the on-beat hits to a continuous beat grid with one
//!    period per piece (a small group of beats far from the rest is left out: it would lever the
//!    period) and the steps on bar lines (searched within 2 bars); pieces that differ
//!    by less than `min_step_pct` or are shorter than `min_bars` bars are merged and the fit runs
//!    again. #472: the suggestion is always a whole number of BPM (the map and every step;
//!    the nearest whole BPM of the piece, the grid follows); the fitted values stay as
//!    diagnostics.
//!    P4-39e: steps 3 to 5 run again with each piece after the first left out; a piece stays only
//!    when the grid with it holds more kick and snare hits on the beat. A sparse section (one kick
//!    pair per bar) lets the local comb read a period that divides the bar in 3; the tracker took it
//!    for a few beats, and a piece the fit drops keeps the beat numbers it gave (2 beats short).

use crate::tempo::{TempoMap, TempoStep};

/// Default: a new tempo must hold this many bars to become a step.
pub const DEFAULT_MIN_BARS: u32 = 4;
/// Default: smallest tempo step in percent.
pub const DEFAULT_MIN_STEP_PCT: f64 = 2.0;
/// Default tempo range in BPM (P4-8 fix round 2: 240 became 300, so a 250 BPM punk beat is
/// found as itself and not through a relative).
pub const DEFAULT_BPM_RANGE: (f64, f64) = (40.0, 300.0);
/// Fewest kick and snare hits the detection needs.
pub const MIN_HITS: usize = 8;
/// Two hits of the same role closer than this are one hit (two mics of one drum).
pub const DEDUPE_S: f64 = 0.025;
/// A hit within this share of a beat of a beat is on the beat.
pub const ON_BEAT: f64 = 0.125;
/// The velocity of a hit without one (the MIDI export's default).
pub const DEFAULT_VELOCITY: f64 = 100.0;
/// Longest span of hits the detection reads (6 hours): a stray time far out is refused, not
/// tracked beat by beat.
pub const MAX_SPAN_S: f64 = 6.0 * 3600.0;
/// A hit with no other hit within this many seconds is left out (isolated).
pub const ISOLATED_S: f64 = 6.0;

/// #317: the tempo ratios the step guard scrutinizes: a piece at 4/3, 3/2 or 2 times the
/// tempo before it (or an inverse) can be a sparse passage that the local comb read on the
/// wrong grid, not a tempo change.
const GUARD_RATIOS: [f64; 6] = [4.0 / 3.0, 1.5, 2.0, 3.0 / 4.0, 2.0 / 3.0, 0.5];
/// How close to a simple ratio a step must sit to be scrutinized (2 %, like `min_step_pct`).
const GUARD_RATIO_TOL: f64 = 0.02;

const LAG_MAX_S: f64 = 6.0;
const LAG_MIN_S: f64 = 0.06;
const LAG_BIN_S: f64 = 0.001;
const LAG_SIGMA_BINS: f64 = 6.0;
const COMB_STEP: f64 = 1.001;
const LOCAL_RANGE: f64 = 1.45;
/// A candidate period needs this share of the best comb score.
const COMB_GATE: f64 = 0.6;
const WINDOW_BEATS: f64 = 16.0;
/// Shortest local window in seconds (a fast song has short bars; fewer hits make a noisy comb).
const WINDOW_MIN_S: f64 = 8.0;
const HOP_BEATS: f64 = 4.0;
/// A chosen tempo this close (relative) to an end of the range is flagged.
const RANGE_EDGE: f64 = 0.03;
/// A gap of more than this many beats without on-beat hits splits the beats into groups.
const FIT_ISOLATED_BEATS: i64 = 16;
/// A group of fewer beats with hits than this is left out of the fit when a larger one exists.
const FIT_MIN_GROUP_BEATS: usize = 4;
/// Snare and tom hits at or above this velocity count for a fill.
const FILL_VELOCITY: f64 = 60.0;
/// #130: the smallest quarter-beat weight share at which a candidate's own grid shows the
/// off-beat 8ths of a finer grid (the half-time backbeat's pick carries a real 4x grid).
const HALF_TIME_MIN_QUART: f64 = 0.2;
/// #130: the smallest comb of the double tempo (relative to the best candidate) at which
/// the double counts as the corroborating beat of a half-time backbeat.
const HALF_TIME_DBL_COMB: f64 = 0.8;

/// The role of a hit; the weight of kick and snare is 1, of other drums 0.5.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// A kick drum.
    Kick,
    /// A snare drum.
    Snare,
    /// A tom: half weight; its runs mark fills (a fill ends on beat 1).
    Tom,
    /// A cymbal or overhead track: half weight; its hits on a beat vote for beat 1 (the crash).
    Cymbal,
    /// Any other drum (hi-hat, percussion): half weight, not used for the backbeat and beat 1.
    Other,
}

impl Role {
    /// The role of a document role name (`kick`, `snare`, `tom`, `cymbal`; anything else is
    /// `Other`).
    #[must_use]
    pub fn from_name(name: &str) -> Role {
        match name {
            "kick" => Role::Kick,
            "snare" => Role::Snare,
            "tom" => Role::Tom,
            "cymbal" => Role::Cymbal,
            _ => Role::Other,
        }
    }

    fn weight(self) -> f64 {
        match self {
            Role::Kick | Role::Snare => 1.0,
            Role::Tom | Role::Cymbal | Role::Other => 0.5,
        }
    }

    /// Kick or snare: the hits the detection needs and the fit quality counts.
    fn is_ks(self) -> bool {
        matches!(self, Role::Kick | Role::Snare)
    }
}

/// One hit: time in seconds, role and MIDI velocity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    /// Time in seconds.
    pub t: f64,
    /// The drum.
    pub role: Role,
    /// MIDI velocity 1 to 127 ([`DEFAULT_VELOCITY`] when unknown): weighs the hit in the phase
    /// and the beat tracking, so ghost notes between the beats do not pull the beat.
    pub v: f64,
}

impl Hit {
    /// A hit with the default velocity.
    #[must_use]
    pub fn new(t: f64, role: Role) -> Hit {
        Hit {
            t,
            role,
            v: DEFAULT_VELOCITY,
        }
    }

    /// The weight in the phase and the tracking: the role's weight times (velocity / 127)
    /// squared (the hit's energy: a ghost note at 35 counts 0.08, an accent at 110 0.75).
    fn weight(&self) -> f64 {
        let v = if self.v.is_finite() { self.v } else { DEFAULT_VELOCITY };
        self.role.weight() * (v.clamp(1.0, 127.0) / 127.0).powi(2)
    }
}

/// The parameters of a detection.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DetectParams {
    /// A new tempo must hold this many bars (at least 1).
    pub min_bars: u32,
    /// Smallest tempo step in percent of the tempo before it.
    pub min_step_pct: f64,
    /// Lowest and highest tempo in BPM.
    pub bpm_range: (f64, f64),
    /// The length of the tracks in seconds: hits after it are dropped (counted in
    /// [`Detection::late`]). `None`: no limit but [`MAX_SPAN_S`].
    pub duration_s: Option<f64>,
}

impl Default for DetectParams {
    fn default() -> Self {
        DetectParams {
            min_bars: DEFAULT_MIN_BARS,
            min_step_pct: DEFAULT_MIN_STEP_PCT,
            bpm_range: DEFAULT_BPM_RANGE,
            duration_s: None,
        }
    }
}

impl DetectParams {
    /// Check the fields; the message names the fault.
    ///
    /// # Errors
    ///
    /// A message when `min_bars` is outside 1 to 64, `min_step_pct` is not finite or outside 0.1
    /// to 50, `bpm_range` is not finite, outside 20 to 400 BPM or narrower than an octave, or
    /// `duration_s` is set but not a positive finite number.
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=64).contains(&self.min_bars) {
            return Err(format!("min_bars must be 1 to 64, not {}", self.min_bars));
        }
        if !(self.min_step_pct.is_finite() && (0.1..=50.0).contains(&self.min_step_pct)) {
            return Err(format!("min_step_pct must be 0.1 to 50, not {}", self.min_step_pct));
        }
        let (lo, hi) = self.bpm_range;
        if !(lo.is_finite() && hi.is_finite() && lo >= 20.0 && hi <= 400.0 && hi >= lo * 2.0) {
            return Err(format!(
                "bpm_range must lie in 20 to 400 BPM and span an octave, not {lo} to {hi}"
            ));
        }
        if let Some(d) = self.duration_s
            && !(d.is_finite() && d > 0.0)
        {
            return Err(format!("duration_s must be a positive number, not {d}"));
        }
        Ok(())
    }
}

/// Another tempo the hits would allow, with its score relative to the chosen one (1 = as good).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Alternative {
    /// Tempo in BPM (a whole number).
    pub bpm: f64,
    /// Its score (comb x prior x backbeat) relative to the chosen tempo's score; for a gated
    /// candidate the score it would have without the gate.
    pub score: f64,
    /// Its comb score relative to the best comb peak (1 = the best).
    pub comb: f64,
    /// True when its comb peak is under the gate (60 % of the best): it could not be chosen.
    pub gated: bool,
}

/// Bars of the probe run that looks for excursions shorter than `min_bars`.
const PROBE_BARS: u32 = 2;

/// A suggested tempo step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StepSuggestion {
    /// Time of the step in seconds (a bar line of the fitted grid).
    pub t: f64,
    /// The new tempo.
    pub bpm: f64,
    /// Bar number of the step (bar 1 is the map's bar 1).
    pub bar: i64,
    /// Bars the new tempo holds (to the next step or the last hit).
    pub bars: i64,
    /// 0 to 1: how sure the step is (size against its standard error, and the share of hits
    /// on the beat in the new piece).
    pub confidence: f64,
}

/// A tempo excursion shorter than `min_bars` that the map has no step for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Excursion {
    /// Time of its first bar line in seconds.
    pub t: f64,
    /// Bars it lasts.
    pub bars: i64,
    /// Its tempo.
    pub bpm: f64,
    /// The tempo of the suggested map at `t`.
    pub map_bpm: f64,
}

/// The result of a detection.
#[derive(Debug, Clone, PartialEq)]
pub struct Detection {
    /// The suggested map (no meter changes). #472: its tempos are whole numbers (the nearest
    /// whole BPM of each fitted piece, the grid follows); the fit's own values stay in
    /// [`Detection::bpm`] and [`Detection::bpm_error_bar`] as diagnostics.
    pub map: TempoMap,
    /// Start tempo of the fit before rounding.
    pub bpm: f64,
    /// The error bar of [`Detection::bpm`] in BPM: the larger of 2 standard errors of the fit and
    /// the tempo change that moves the grid by the hits' RMS jitter at the ends of the piece.
    /// A diagnostic: the suggestion is the nearest whole BPM either way (#472).
    pub bpm_error_bar: f64,
    /// 0 to 1: share of hits on the beat and margin to the next octave candidate.
    pub confidence: f64,
    /// Other tempos with their relative score, best first.
    pub alternatives: Vec<Alternative>,
    /// The chosen tempo lies within 3 % of an end of the search range: the true beat may lie
    /// outside it (twice or half as fast).
    pub range_edge: bool,
    /// Bar 1 beat 1 in seconds.
    pub bar1_s: f64,
    /// 0 to 1: the product of [`Detection::bar1_backbeat`] and [`Detection::bar1_downbeat`].
    pub bar1_confidence: f64,
    /// 0 to 1: how clearly the snare picks the pair of beats (2 and 4 against 1 and 3).
    pub bar1_backbeat: f64,
    /// 0 to 1: how clearly the kicks (on the beat or an eighth before), crash hits, fill endings
    /// and the first kick pick beat 1 within its pair (1 against 3).
    pub bar1_downbeat: f64,
    /// The other candidate for bar 1, later than bar 1: two beats later in 4/4, the runner-up
    /// beat in 3/4.
    pub bar1_alternative_s: f64,
    /// Suggested meter `(num, den)`: 4/4 or 3/4.
    pub meter: (u8, u8),
    /// 0 to 1: how clear the meter evidence is.
    pub meter_confidence: f64,
    /// Correlation of the beat pattern (kick minus snare on the beat, kicks and snares between
    /// the beats) at 2 beats.
    pub meter_r2: f64,
    /// The same at 3 beats.
    pub meter_r3: f64,
    /// The same at 4 beats.
    pub meter_r4: f64,
    /// `Some("two-beat")` when the pattern repeats every 2 beats as well as every 4: the bar may
    /// be 2/4, or 6/8 counted in dotted quarters (compound feel); the meter confidence is 0.
    pub meter_feel: Option<&'static str>,
    /// The steps.
    pub steps: Vec<StepSuggestion>,
    /// Share of the kick and snare weight within 1/8 beat of the fitted grid.
    pub on_beat: f64,
    /// RMS distance of those hits from the grid, in ms.
    pub jitter_ms: f64,
    /// Hits used (after merging hits of one drum within 25 ms).
    pub hits: usize,
    /// Hits after [`DetectParams::duration_s`], dropped.
    pub late: usize,
    /// Hits with no other hit within [`ISOLATED_S`], left out.
    pub isolated: usize,
    /// Beats tracked from the first hit to the last.
    pub beats: usize,
    /// Tempo excursions shorter than `min_bars` bars that a run with 2 bars finds and the map
    /// does not hold: they pull the fitted tempo off (empty when `min_bars` is 2 or less).
    pub excursions: Vec<Excursion>,
}

// ---------------------------------------------------------------- preparation

/// The hits [`prepare`] keeps, and the counts of what it left out.
#[derive(Debug, Clone, PartialEq)]
pub struct Prepared {
    /// Sorted, merged hits.
    pub hits: Vec<Hit>,
    /// Hits after the end (`until`).
    pub late: usize,
    /// Hits with no other hit within [`ISOLATED_S`].
    pub isolated: usize,
}

/// Sort, drop non-finite and negative times and times after `until`, merge hits of one role
/// within [`DEDUPE_S`] (the first time, the louder velocity), and leave out isolated hits.
#[must_use]
pub fn prepare(hits: &[Hit], until: Option<f64>) -> Prepared {
    let until = until.unwrap_or(f64::INFINITY);
    let mut late = 0;
    let mut out: Vec<Hit> = Vec::with_capacity(hits.len());
    for role in [Role::Kick, Role::Snare, Role::Tom, Role::Cymbal, Role::Other] {
        let mut hs: Vec<Hit> = hits
            .iter()
            .filter(|h| h.role == role && h.t.is_finite() && h.t >= 0.0)
            .copied()
            .collect();
        hs.sort_by(|a, b| a.t.total_cmp(&b.t));
        let mut last = f64::NEG_INFINITY;
        for h in hs {
            if h.t > until {
                late += 1;
            } else if h.t - last >= DEDUPE_S {
                out.push(h);
                last = h.t;
            } else if let Some(prev) = out.last_mut() {
                prev.v = prev.v.max(h.v);
            }
        }
    }
    out.sort_by(|a, b| a.t.total_cmp(&b.t));
    let keep: Vec<bool> = (0..out.len())
        .map(|i| {
            let before = i.checked_sub(1).is_some_and(|j| out[i].t - out[j].t <= ISOLATED_S);
            let after = out.get(i + 1).is_some_and(|h| h.t - out[i].t <= ISOLATED_S);
            before || after
        })
        .collect();
    let isolated = keep.iter().filter(|k| !**k).count();
    let hits = out.into_iter().zip(keep).filter(|x| x.1).map(|x| x.0).collect();
    Prepared { hits, late, isolated }
}

// ---------------------------------------------------------------- lag histogram and comb

/// The smoothed histogram of the intervals between hits (the autocorrelation of the hit train).
struct LagHist {
    bins: Vec<f64>,
    mean: f64,
}

impl LagHist {
    fn new(hits: &[Hit], keep: impl Fn(&Hit) -> bool, max_lag: f64) -> LagHist {
        let n = (max_lag / LAG_BIN_S).ceil() as usize + 2;
        let mut raw = vec![0.0; n];
        let sel: Vec<&Hit> = hits.iter().filter(|h| keep(h)).collect();
        for (i, a) in sel.iter().enumerate() {
            for b in &sel[i + 1..] {
                let d = b.t - a.t;
                if d > max_lag {
                    break;
                }
                let x = d / LAG_BIN_S;
                let k = x.floor() as usize;
                let f = x - k as f64;
                let w = a.weight() * b.weight();
                if k + 1 < n {
                    raw[k] += w * (1.0 - f);
                    raw[k + 1] += w * f;
                }
            }
        }
        let half = (3.0 * LAG_SIGMA_BINS).ceil() as isize;
        let kern: Vec<f64> = (-half..=half)
            .map(|i| (-0.5 * (i as f64 / LAG_SIGMA_BINS).powi(2)).exp())
            .collect();
        let ks: f64 = kern.iter().sum();
        let mut bins = vec![0.0; n];
        for (i, &v) in raw.iter().enumerate() {
            if v == 0.0 {
                continue;
            }
            for (j, &k) in kern.iter().enumerate() {
                let p = i as isize + j as isize - half;
                if p >= 0 && (p as usize) < n {
                    bins[p as usize] += v * k / ks;
                }
            }
        }
        let lo = (LAG_MIN_S / LAG_BIN_S) as usize;
        let hi = ((max_lag / LAG_BIN_S) as usize).min(n);
        let mean = if hi > lo {
            bins[lo..hi].iter().sum::<f64>() / (hi - lo) as f64
        } else {
            0.0
        };
        LagHist { bins, mean }
    }

    fn max_lag(&self) -> f64 {
        (self.bins.len() - 2) as f64 * LAG_BIN_S
    }

    /// The histogram at `lag` in units of its mean (0 for an empty histogram).
    fn at(&self, lag: f64) -> f64 {
        if self.mean <= 0.0 || lag.is_nan() || lag < 0.0 {
            return 0.0;
        }
        let x = lag / LAG_BIN_S;
        let k = x.floor() as usize;
        if k + 1 >= self.bins.len() {
            return 0.0;
        }
        let f = x - k as f64;
        (self.bins[k] * (1.0 - f) + self.bins[k + 1] * f) / self.mean
    }

    /// Comb score of period `p`: the mean of the histogram at 1 to K times `p` (K as many as fit,
    /// at most `max_teeth`), minus 1 (0 is the level of a train without a period).
    fn comb(&self, p: f64, max_teeth: usize) -> f64 {
        let k = ((self.max_lag() / p).floor() as usize).min(max_teeth);
        if k == 0 {
            return f64::NEG_INFINITY;
        }
        (1..=k).map(|i| self.at(i as f64 * p)).sum::<f64>() / k as f64 - 1.0
    }
}

/// The period with the best comb score in `[lo, hi]` seconds (scan in steps of 0.1 %, then
/// refine by 0.01 %); `None` when no score is above 0.
fn best_period(h: &LagHist, lo: f64, hi: f64, max_teeth: usize) -> Option<(f64, f64)> {
    let mut best: Option<(f64, f64)> = None;
    let mut p = lo;
    while p <= hi {
        let c = h.comb(p, max_teeth);
        if c > best.map_or(0.0, |b| b.1) {
            best = Some((p, c));
        }
        p *= COMB_STEP;
    }
    best.map(|(p0, c0)| refine_period(h, p0, c0, max_teeth))
}

fn refine_period(h: &LagHist, p0: f64, c0: f64, max_teeth: usize) -> (f64, f64) {
    let mut best = (p0, c0);
    for i in -20..=20 {
        let p = p0 * (1.0 + f64::from(i) * 0.0001);
        let c = h.comb(p, max_teeth);
        if c > best.1 {
            best = (p, c);
        }
    }
    best
}

/// Wide prior on the tempo: a Gaussian in octaves around 120 BPM, one octave wide.
fn prior(bpm: f64) -> f64 {
    (-0.5 * (bpm / 120.0).log2().powi(2)).exp()
}

/// Half-time snare (#180): the busier beat of the snare's pair holds at least this many
/// times the snares of its partner, and at least [`HALF_TIME_MIN_SNARES`] of them.
const HALF_TIME_RATIO: f64 = 2.0;
const HALF_TIME_MIN_SNARES: f64 = 8.0;

/// #130 tempo-referee version (env `DRUMSPIKE_TEMPO_RULE`; unset counts as 3, the #180
/// product default). Version 1 pins the shipped referee: with a snare, the backbeat alone
/// picks the octave and the shares stay out of it. Version 2 corroborates the backbeat
/// verdict with the sub-beat shares (see [`half_time_backbeat`]). Version 3 adds the
/// half-time bar 1 of #180 (see `bars`). The detection is read
/// only: a project analysed before keeps its tempo until it is detected again.
#[must_use]
pub fn tempo_rule() -> u32 {
    parse_rule(std::env::var("DRUMSPIKE_TEMPO_RULE").ok().as_deref())
}

/// The pin value behind [`tempo_rule`]: `1` and `0` pin the shipped referee, `2` the #130
/// one, an unset or unparseable value is rule 3. A pure function so its test does not write
/// the process env, which the parallel detection tests share.
fn parse_rule(v: Option<&str>) -> u32 {
    match v {
        Some(v) => v.parse().unwrap_or(3),
        None => 3,
    }
}

/// Backbeat evidence of beat period `p`: snare to snare at 2 beats against 1 and 3 beats, and
/// against a snare period shorter than 2 beats (2p/3 or 2p/5: then `p` is 3 or 5 true beats,
/// whose doubles are even numbers of beats too). -0.5 to 1; 0 without enough snare hits.
fn backbeat(snare: &LagHist, p: f64) -> f64 {
    if snare.mean <= 0.0 {
        return 0.0;
    }
    let even = snare.at(2.0 * p);
    let sub = snare.at(2.0 * p / 3.0).max(snare.at(2.0 * p / 5.0));
    let odd = (0.5 * (snare.at(p) + snare.at(3.0 * p))).max(sub);
    ((even - odd) / (even + odd + 1e-9)).clamp(-0.5, 1.0)
}

/// The on-beat and the quarter-beat share of the kick and snare weight on the best-phase grid
/// of period `p` (windows of 1/8 and 1/16 of `p`, the phase searched as `phase` does).
fn octave_shares(hits: &[Hit], p: f64) -> (f64, f64) {
    let phi = phase(
        &hits.iter().filter(|h| h.role.is_ks()).copied().collect::<Vec<_>>(),
        p,
        hits[0].t,
        hits[hits.len() - 1].t,
    );
    let (mut on, mut quart, mut all) = (0.0, 0.0, 0.0);
    for h in hits.iter().filter(|h| h.role.is_ks()) {
        let x = (h.t - phi).rem_euclid(p);
        let d = x.min(p - x);
        all += h.weight();
        if d < p / 8.0 {
            on += h.weight();
        }
        let dq = (x - 0.25 * p).abs().min((x - 0.75 * p).abs());
        if dq < p / 16.0 {
            quart += h.weight();
        }
    }
    if all <= 0.0 {
        return (0.0, 0.0);
    }
    (on / all, quart / all)
}

/// #130: is the candidate at period `p` the half tempo of a half-time backbeat? The backbeat
/// referee cannot tell "snare on 2 and 4 at `p`" from "snare every 4 beats at `2p`" - the
/// snare-to-snare histogram is the same - so the verdict of a positive backbeat is
/// corroborated by the sub-beat shares before it decides the octave. The artifact pick
/// carries a real finer grid (a quarter-beat share of at least [`HALF_TIME_MIN_QUART`]:
/// the off-beat 8ths of the true tempo) while its double holds the weight on the beat
/// (more than `p` does) with a comb close to the best candidate's (at least
/// [`HALF_TIME_DBL_COMB`]) and almost nothing a quarter beat off. A straight backbeat
/// (no quarter-beat weight: kick on 1 and 3), a ghost-note funk groove (a strong kick a
/// quarter beat off, but a weak double) and a beat whose double is out of range or under
/// the comb gate stay at `p`. The Session A session (map 134, kick 8ths, snare every 4
/// beats): the 67 pick carries quarter 0.284 against 0.037 on the 134 grid and 134's comb
/// is 0.971 of the best - flagged; the ballad at 67 with sixteenth kicks carries quarter
/// 0.125 and stays.
fn half_time_backbeat(hits: &[Hit], snare: &LagHist, rows: &[(f64, f64, f64)], p: f64) -> bool {
    if backbeat(snare, p) <= 0.0 {
        return false;
    }
    let Some(&(_, dcomb, _)) = rows.iter().find(|&&(q, _, _)| (q / (p / 2.0)).ln().abs() < 0.03) else {
        return false;
    };
    if dcomb < HALF_TIME_DBL_COMB {
        return false;
    }
    let (on, quart) = octave_shares(hits, p);
    if quart < HALF_TIME_MIN_QUART {
        return false;
    }
    let (don, dquart) = octave_shares(hits, p / 2.0);
    don > on && dquart < quart
}

struct Global {
    period: f64,
    /// The pick before the #472 octave referee doubled it (`None` when it did not fire).
    prev_period: Option<f64>,
    alternatives: Vec<Alternative>,
    margin: f64,
    range_edge: bool,
}

fn global_period(hits: &[Hit], params: &DetectParams) -> Result<Global, String> {
    let all = LagHist::new(hits, |_| true, LAG_MAX_S);
    let n_snare = hits.iter().filter(|h| h.role == Role::Snare).count();
    let snare = LagHist::new(hits, |h| h.role == Role::Snare && n_snare >= MIN_HITS, LAG_MAX_S);
    let (lo, hi) = (60.0 / params.bpm_range.1, 60.0 / params.bpm_range.0);
    // Local maxima of the comb, best first, at least 3 % apart.
    let mut peaks: Vec<(f64, f64)> = Vec::new();
    let mut prev = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    let mut p = lo;
    let mut cur = all.comb(p, 64);
    while p <= hi {
        let np = p * COMB_STEP;
        let next = all.comb(np, 64);
        if cur > 0.0 && cur >= prev.1 && cur > next {
            peaks.push(refine_period(&all, p, cur, 64));
        }
        prev = (p, cur);
        p = np;
        cur = next;
    }
    peaks.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut cands: Vec<(f64, f64)> = Vec::new();
    for pk in peaks {
        if cands.iter().all(|c| (c.0 / pk.0).ln().abs() > 0.03) && cands.len() < 6 {
            cands.push(pk);
        }
    }
    let Some(&(top, _)) = cands.first() else {
        return Err("the hits show no regular beat".into());
    };
    for r in [2.0, 0.5, 3.0, 1.0 / 3.0, 1.5, 2.0 / 3.0] {
        let q = top * r;
        if q < lo || q > hi || cands.iter().any(|c| (c.0 / q).ln().abs() < 0.03) {
            continue;
        }
        if let Some(b) = best_period(&all, q / 1.02, q * 1.02, 64) {
            cands.push(b);
        }
    }
    let cmax = cands.iter().map(|c| c.1).fold(0.0, f64::max);
    // (period, comb relative to the best, score without the gate)
    let rows: Vec<(f64, f64, f64)> = cands
        .iter()
        .map(|&(p, c)| {
            let rel = if cmax > 0.0 { c / cmax } else { 0.0 };
            (p, rel, rel * prior(60.0 / p).sqrt() * (1.0 + backbeat(&snare, p)))
        })
        .collect();
    // #472: the octave referee for sessions without a snare. A kick-only session played at
    // twice `p` leaves about half its kicks a quarter beat off the grid of `p` (they are its
    // off-beat 8ths), more than sit on it; a session at `p` leaves almost none there. The
    // Other Side (kick only, 92 BPM played): on the 46 BPM pick 43.5 % of the kick weight sits
    // a quarter beat off, 37.1 % on it; on 92 BPM the shares are 23.6 % and 43.5 %. A candidate
    // with more weight at the quarter beat than on the beat is the half tempo: it loses the
    // octave vote (it stays in the alternatives). With a snare the backbeat picks the octave
    // and the referee stays out of it: a tempo step leaves its section's hits off the beat
    // grid of the rest of the song, which the shares cannot tell from a half tempo (the
    // 120/140/120 trains read 46 % off the 120 grid).
    let half_tempo: Vec<bool> = if n_snare >= MIN_HITS {
        // #130: with a snare the backbeat picks the octave; since rule 2 its verdict is
        // corroborated by the sub-beat shares before it decides (the half-time backbeat),
        // unless DRUMSPIKE_TEMPO_RULE pins the shipped referee.
        if tempo_rule() < 2 {
            vec![false; rows.len()]
        } else {
            rows.iter()
                .map(|(p, _, _)| half_time_backbeat(hits, &snare, &rows, *p))
                .collect()
        }
    } else {
        rows.iter()
            .map(|(p, _, _)| {
                let (on, quart) = octave_shares(hits, *p);
                quart > on
            })
            .collect()
    };
    // #472: the octave referee for sessions without a snare. A kick-only session played at
    // twice `p` leaves about half its kicks a quarter beat off the grid of `p` (they are its
    // off-beat 8ths), more than sit on it; a session at `p` leaves almost none there. The
    // Other Side (kick only, 92 BPM played): on the 46 BPM pick 43.5 % of the kick weight sits
    // a quarter beat off, 37.1 % on it; on 92 BPM the shares are 23.6 % and 43.5 %. When the
    // picked period looks like a half tempo, its octave double takes the vote even when the
    // comb gate held it back (the half tempo's comb is the stronger only because it counts the
    // off-beat 8ths as teeth too). With a snare the backbeat picks the octave and the shares
    // agree with it; a session at a genuinely slow tempo leaves almost no weight at the
    // quarter beat and keeps its pick.
    let half_at = |q: f64| -> Option<bool> {
        rows.iter()
            .zip(&half_tempo)
            .find(|(r, _)| (r.0 / q).ln().abs() < 1e-9)
            .map(|(_, h)| *h)
    };
    // A weak comb peak is not a candidate, however good its backbeat.
    let open = |r: &(f64, f64, f64)| r.1 >= COMB_GATE;
    let (mut period, _, mut score) = rows
        .iter()
        .filter(|r| open(r))
        .copied()
        .fold(None::<(f64, f64, f64)>, |best, r| match best {
            Some(b) if b.2 >= r.2 => Some(b),
            _ => Some(r),
        })
        .ok_or("the hits show no regular beat")?;
    let mut prev_period = None;
    if half_at(period).unwrap_or(false) {
        let doubled = period / 2.0;
        if let Some(&(q, _, s)) = rows.iter().find(|&&(p, _, _)| (p / doubled).ln().abs() < 0.03) {
            prev_period = Some(period);
            period = q;
            score = s;
        }
    }
    let is_half = |q: f64| half_at(q).unwrap_or(false);
    let mut others: Vec<(f64, f64, f64)> = rows.iter().copied().filter(|r| r.0 != period).collect();
    others.sort_by(|a, b| b.2.total_cmp(&a.2));
    let alternatives: Vec<Alternative> = others
        .iter()
        .take(4)
        .map(|r| Alternative {
            bpm: round_to(60.0 / r.0, 1.0),
            score: round_to(if score > 0.0 { r.2 / score } else { 0.0 }, 0.001),
            comb: round_to(r.1, 0.001),
            gated: !open(r),
        })
        .collect();
    // The margin counts only the octave relatives (2, 1/2, 3, 1/3, 3/2, 2/3 times the tempo)
    // that pass the gate: a second tempo of the song is no doubt about the first.
    let related = |q: f64| {
        [2.0, 0.5, 3.0, 1.0 / 3.0, 1.5, 2.0 / 3.0]
            .iter()
            .any(|r| (q / (period * r)).ln().abs() < 0.03)
    };
    let rival = others
        .iter()
        .filter(|r| open(r) && related(r.0) && !is_half(r.0))
        .map(|r| r.2)
        .fold(0.0, f64::max);
    let margin = if score > 0.0 { 1.0 - rival / score } else { 0.0 };
    let bpm = 60.0 / period;
    let (blo, bhi) = params.bpm_range;
    let range_edge = bpm * (1.0 + RANGE_EDGE) > bhi || bpm < blo * (1.0 + RANGE_EDGE);
    Ok(Global {
        period,
        prev_period,
        alternatives,
        margin,
        range_edge,
    })
}

// ---------------------------------------------------------------- local periods and pieces

/// A piece of constant tempo: the time span of its windows and its period.
#[derive(Debug, Clone, Copy)]
struct Piece {
    start: f64,
    period: f64,
}

fn local_periods(hits: &[Hit], p0: f64) -> Vec<(f64, f64)> {
    let len = (WINDOW_BEATS * p0).max(WINDOW_MIN_S);
    let hop = HOP_BEATS * p0;
    let (first, last) = (hits[0].t, hits[hits.len() - 1].t);
    let max_lag = 6.0 * p0 * LOCAL_RANGE;
    let mut out = Vec::new();
    let mut s = first;
    loop {
        let w: Vec<Hit> = hits.iter().filter(|h| h.t >= s && h.t < s + len).copied().collect();
        if w.iter().filter(|h| h.role.is_ks()).count() >= 6 {
            let h = LagHist::new(&w, |_| true, max_lag);
            if let Some((p, _)) = best_period(&h, p0 / LOCAL_RANGE, p0 * LOCAL_RANGE, 64) {
                out.push((s + 0.5 * len, p));
            }
        }
        if s + len >= last {
            break;
        }
        s += hop;
    }
    out
}

fn median(v: &mut [f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f64::total_cmp);
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        0.5 * (v[n / 2 - 1] + v[n / 2])
    }
}

/// Cut the local periods (log) into constant pieces of at least `min_len` values with a
/// dynamic programme (L1 cost around the median, `penalty` per piece). Returns index ranges.
fn segment(u: &[f64], min_len: usize, penalty: f64) -> Vec<(usize, usize)> {
    let n = u.len();
    if n < 2 * min_len {
        return vec![(0, n)];
    }
    let mut cost = vec![vec![f64::INFINITY; n + 1]; n + 1];
    for (i, row) in cost.iter_mut().enumerate().take(n) {
        let mut v: Vec<f64> = Vec::new();
        for (j, &x) in u.iter().enumerate().skip(i) {
            v.push(x);
            if j + 1 - i >= min_len {
                let m = median(&mut v.clone());
                row[j + 1] = v.iter().map(|y| (y - m).abs()).sum();
            }
        }
    }
    let mut f = vec![f64::INFINITY; n + 1];
    let mut from = vec![0usize; n + 1];
    f[0] = 0.0;
    for j in min_len..=n {
        for i in 0..=j - min_len {
            if f[i].is_finite() && cost[i][j].is_finite() {
                let c = f[i] + cost[i][j] + penalty;
                if c < f[j] {
                    f[j] = c;
                    from[j] = i;
                }
            }
        }
    }
    if !f[n].is_finite() {
        return vec![(0, n)];
    }
    let mut out = Vec::new();
    let mut j = n;
    while j > 0 {
        out.push((from[j], j));
        j = from[j];
    }
    out.reverse();
    out
}

fn pieces(hits: &[Hit], p0: f64, params: &DetectParams) -> Vec<Piece> {
    let local = local_periods(hits, p0);
    let first = hits[0].t;
    if local.is_empty() {
        return vec![Piece {
            start: first,
            period: p0,
        }];
    }
    let u: Vec<f64> = local.iter().map(|x| x.1.ln()).collect();
    let step = (1.0 + params.min_step_pct / 100.0).ln();
    let min_len = params.min_bars as usize;
    let mut ranges = segment(&u, min_len, 0.5 * min_len as f64 * step);
    // Merge neighbours closer than the smallest step.
    loop {
        let meds: Vec<f64> = ranges.iter().map(|&(a, b)| median(&mut u[a..b].to_vec())).collect();
        let Some(k) = (1..ranges.len()).find(|&k| (meds[k] - meds[k - 1]).abs() < step) else {
            break;
        };
        ranges[k - 1].1 = ranges[k].1;
        ranges.remove(k);
    }
    ranges
        .iter()
        .enumerate()
        .map(|(k, &(a, b))| {
            // A boundary lies between the centres of the last window before it and the first after it.
            let start = if k == 0 {
                first
            } else {
                0.5 * (local[a - 1].0 + local[a].0)
            };
            Piece {
                start,
                period: median(&mut u[a..b].to_vec()).exp(),
            }
        })
        .collect()
}

// ---------------------------------------------------------------- beat tracking

/// Weighted support of hits around `pred` (within 0.2 of period `q`), and their mean offset.
/// Each hit counts with its weight (role and velocity).
fn support(hits: &[Hit], pred: f64, q: f64) -> (f64, f64) {
    let lo = hits.partition_point(|h| h.t < pred - 0.2 * q);
    let mut m = 0.0;
    let mut e = 0.0;
    for h in &hits[lo..] {
        if h.t > pred + 0.2 * q {
            break;
        }
        let d = h.t - pred;
        let g = h.weight() * (-0.5 * (d / (0.06 * q)).powi(2)).exp();
        m += g;
        e += g * d;
    }
    (m, if m > 0.0 { e / m } else { 0.0 })
}

/// The phase (0 to p) that puts most hit weight (role and velocity) on the beat, from the hits
/// in `[t0, t1)`.
fn phase(hits: &[Hit], p: f64, t0: f64, t1: f64) -> f64 {
    phase_scored(hits, p, t0, t1).0
}

/// The kernel weight of the hits in `[t0, t1)` on the phase `phi` of a beat of period `p`
/// (role and velocity weighted, sigma a 20th of the beat).
fn phase_kernel(hits: &[Hit], p: f64, t0: f64, t1: f64, phi: f64) -> f64 {
    hits.iter()
        .filter(|h| h.t >= t0 && h.t < t1)
        .map(|h| {
            let x = (h.t - phi).rem_euclid(p);
            let d = x.min(p - x);
            h.weight() * (-0.5 * (d / (0.05 * p)).powi(2)).exp()
        })
        .sum()
}

/// `phase` with the kernel weight of the winner (the support the phase has from the hits).
fn phase_scored(hits: &[Hit], p: f64, t0: f64, t1: f64) -> (f64, f64) {
    let mut best = (0.0, f64::NEG_INFINITY);
    for k in 0..200 {
        let phi = f64::from(k) / 200.0 * p;
        let s = phase_kernel(hits, p, t0, t1, phi);
        if s > best.1 {
            best = (phi, s);
        }
    }
    best
}

/// Support of a beat grid with period `q` for the `k` beats after `b`.
fn support_ahead(hits: &[Hit], b: f64, q: f64, k: usize) -> f64 {
    (1..=k).map(|i| support(hits, b + i as f64 * q, q).0).sum()
}

/// Track the beats: times of beat 0, 1, ... from the first hit to the last, and for each piece
/// boundary the index of the first beat of the new piece (`usize::MAX` when the hits never took
/// the new period: that boundary is dropped).
fn track(hits: &[Hit], pieces: &[Piece], phi_hint: Option<f64>) -> (Vec<f64>, Vec<usize>) {
    let p1 = pieces[0].period;
    let first = hits[0].t;
    let last = hits[hits.len() - 1].t;
    // #472: when the octave referee doubled the tempo, the half tempo's grid carries the
    // phase: its lines are a subset of the double's (its beats are the double's even beats).
    // The double's own phase search would rather take the off-beat 8ths, which hold more
    // kicks than the beats in a kick-only session.
    let phi = phi_hint.map_or_else(|| phase(hits, p1, first, first + 16.0 * p1), |h| h.rem_euclid(p1));
    let mut b = phi + p1 * ((first - 0.25 * p1 - phi) / p1).ceil();
    let mut beats = vec![b];
    let mut switches = Vec::new();
    let mut next = 1usize;
    let mut base = p1;
    let mut p = p1;
    while b <= last + 0.5 * p {
        if let Some(np) = pieces.get(next) {
            let zone = 4.0 * HOP_BEATS * base.max(np.period);
            if (b - np.start).abs() < zone {
                let m_old = support_ahead(hits, b, p, 4);
                let m_new = support_ahead(hits, b, np.period, 4);
                if m_new > m_old * 1.05 || (m_new >= m_old && b + np.period >= np.start) {
                    switches.push(beats.len());
                    base = np.period;
                    p = np.period;
                    next += 1;
                }
            } else if b > np.start {
                switches.push(usize::MAX);
                next += 1;
            }
        }
        let pred = b + p;
        let (m, err) = support(hits, pred, p);
        if m > 0.0 {
            b = pred + 0.5 * err;
            p = (p + 0.05 * err).clamp(base * 0.97, base * 1.03);
        } else {
            b = pred;
        }
        beats.push(b);
    }
    (beats, switches)
}

// ---------------------------------------------------------------- fit

/// A hit on a beat: beat index, time, role.
#[derive(Debug, Clone, Copy)]
struct OnBeat {
    n: i64,
    t: f64,
    role: Role,
}

fn assign(hits: &[Hit], beats: &[f64]) -> Vec<OnBeat> {
    let mut out = Vec::new();
    for h in hits {
        let i = beats.partition_point(|&b| b < h.t);
        let mut best: Option<(usize, f64)> = None;
        for j in [i.wrapping_sub(1), i] {
            if let Some(&b) = beats.get(j) {
                let d = (h.t - b).abs();
                if best.is_none_or(|x| d < x.1) {
                    best = Some((j, d));
                }
            }
        }
        if let Some((j, d)) = best {
            let q = if j + 1 < beats.len() {
                beats[j + 1] - beats[j]
            } else if j > 0 {
                beats[j] - beats[j - 1]
            } else {
                continue;
            };
            if d < ON_BEAT * q {
                out.push(OnBeat {
                    n: j as i64,
                    t: h.t,
                    role: h.role,
                });
            }
        }
    }
    out
}

/// Leave out the on-beat hits of a small group of beats far from the rest: the beats with hits
/// split into groups at gaps of more than [`FIT_ISOLATED_BEATS`]; a group of fewer than
/// [`FIT_MIN_GROUP_BEATS`] beats is left out when a larger group exists. One or two stray hits
/// far from the song would lever the fitted period (P4-8 fix round 2, L6).
fn drop_isolated(on: &[OnBeat]) -> Vec<OnBeat> {
    let mut ns: Vec<i64> = on.iter().map(|o| o.n).collect();
    ns.sort_unstable();
    ns.dedup();
    // (first beat, last beat, beats with hits)
    let mut groups: Vec<(i64, i64, usize)> = Vec::new();
    for n in ns {
        match groups.last_mut() {
            Some(g) if n - g.1 <= FIT_ISOLATED_BEATS => {
                g.1 = n;
                g.2 += 1;
            }
            _ => groups.push((n, n, 1)),
        }
    }
    groups.retain(|g| g.2 >= FIT_MIN_GROUP_BEATS);
    if groups.is_empty() {
        return on.to_vec();
    }
    on.iter()
        .filter(|o| groups.iter().any(|g| o.n >= g.0 && o.n <= g.1))
        .copied()
        .collect()
}

/// The fitted grid: `t(n) = t0 + sum of the periods of the pieces up to beat n`.
#[derive(Debug, Clone)]
struct Grid {
    t0: f64,
    /// Beat index where each piece after the first starts.
    breaks: Vec<i64>,
    periods: Vec<f64>,
    /// Standard error of each period.
    se: Vec<f64>,
}

impl Grid {
    fn row(&self, n: i64, breaks: &[i64]) -> Vec<f64> {
        let m = breaks.len() + 1;
        let mut r = vec![0.0; m + 1];
        r[0] = 1.0;
        r[1] = n.min(breaks.first().copied().unwrap_or(i64::MAX)) as f64;
        for j in 1..m {
            let hi = breaks.get(j).copied().unwrap_or(i64::MAX);
            r[j + 1] = (n.min(hi) - breaks[j - 1]).max(0) as f64;
        }
        r
    }

    fn time(&self, n: i64) -> f64 {
        let r = self.row(n, &self.breaks);
        self.t0 + r[1..].iter().zip(&self.periods).map(|(x, p)| x * p).sum::<f64>()
    }
}

/// Solve the normal equations of the rows (least squares); `None` when singular.
fn least_squares(rows: &[Vec<f64>], ys: &[f64]) -> Option<(Vec<f64>, Vec<f64>, f64)> {
    let k = rows.first()?.len();
    let mut a = vec![vec![0.0; k]; k];
    let mut b = vec![0.0; k];
    for (r, &y) in rows.iter().zip(ys) {
        for i in 0..k {
            b[i] += r[i] * y;
            for j in 0..k {
                a[i][j] += r[i] * r[j];
            }
        }
    }
    let inv = invert(&a)?;
    let x: Vec<f64> = (0..k).map(|i| (0..k).map(|j| inv[i][j] * b[j]).sum()).collect();
    let sse: f64 = rows
        .iter()
        .zip(ys)
        .map(|(r, &y)| {
            let f: f64 = r.iter().zip(&x).map(|(a, b)| a * b).sum();
            (y - f).powi(2)
        })
        .sum();
    let dof = rows.len().saturating_sub(k).max(1) as f64;
    let s2 = sse / dof;
    let se = (0..k).map(|i| (inv[i][i] * s2).max(0.0).sqrt()).collect();
    Some((x, se, sse))
}

fn invert(a: &[Vec<f64>]) -> Option<Vec<Vec<f64>>> {
    let k = a.len();
    let mut m: Vec<Vec<f64>> = a
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let mut x = r.clone();
            x.extend((0..k).map(|j| if i == j { 1.0 } else { 0.0 }));
            x
        })
        .collect();
    for c in 0..k {
        let piv = (c..k).max_by(|&i, &j| m[i][c].abs().total_cmp(&m[j][c].abs()))?;
        if m[piv][c].abs() < 1e-12 {
            return None;
        }
        m.swap(c, piv);
        let d = m[c][c];
        for x in &mut m[c] {
            *x /= d;
        }
        let pivot = m[c].clone();
        for (r, row) in m.iter_mut().enumerate() {
            let f = row[c];
            if r != c && f != 0.0 {
                for (x, p) in row.iter_mut().zip(&pivot) {
                    *x -= f * p;
                }
            }
        }
    }
    Some(m.into_iter().map(|r| r[k..].to_vec()).collect())
}

/// The least-squares fit: two passes, the second with the hits within 3 x the RMS residual (at
/// least 2 ms) of the first grid. Returns the grid and the hits of the last pass.
fn fit(on: &[OnBeat], breaks: &[i64]) -> Option<(Grid, Vec<OnBeat>)> {
    let shell = Grid {
        t0: 0.0,
        breaks: breaks.to_vec(),
        periods: vec![],
        se: vec![],
    };
    let mut keep: Vec<OnBeat> = on.to_vec();
    let mut out = None;
    for _ in 0..2 {
        // Every piece needs two beats with hits.
        for j in 0..=breaks.len() {
            let lo = if j == 0 { i64::MIN } else { breaks[j - 1] };
            let hi = if j < breaks.len() { breaks[j] } else { i64::MAX };
            let mut ns: Vec<i64> = keep.iter().filter(|o| o.n >= lo && o.n < hi).map(|o| o.n).collect();
            ns.dedup();
            if ns.len() < 2 {
                return None;
            }
        }
        let rows: Vec<Vec<f64>> = keep.iter().map(|o| shell.row(o.n, breaks)).collect();
        let ys: Vec<f64> = keep.iter().map(|o| o.t).collect();
        let (x, se, sse) = least_squares(&rows, &ys)?;
        let g = Grid {
            t0: x[0],
            breaks: breaks.to_vec(),
            periods: x[1..].to_vec(),
            se: se[1..].to_vec(),
        };
        let lim = (3.0 * (sse / keep.len() as f64).sqrt()).max(0.002);
        let next: Vec<OnBeat> = on
            .iter()
            .filter(|o| (o.t - g.time(o.n)).abs() <= lim)
            .copied()
            .collect();
        out = Some((g, keep));
        keep = next;
    }
    out
}

/// SSE of the fit with fixed hits (comparison of break positions).
fn sse_of(on: &[OnBeat], breaks: &[i64]) -> f64 {
    let shell = Grid {
        t0: 0.0,
        breaks: breaks.to_vec(),
        periods: vec![],
        se: vec![],
    };
    let rows: Vec<Vec<f64>> = on.iter().map(|o| shell.row(o.n, breaks)).collect();
    let ys: Vec<f64> = on.iter().map(|o| o.t).collect();
    least_squares(&rows, &ys).map_or(f64::INFINITY, |r| r.2)
}

/// Move each break to the bar line (`n % bpb == down`) within 2 bars that fits best.
fn place_breaks(on: &[OnBeat], breaks: &mut [i64], down: i64, bpb: i64) {
    for b in breaks.iter_mut() {
        *b = down + ((*b - down) as f64 / bpb as f64).round() as i64 * bpb;
    }
    for _ in 0..2 {
        for k in 0..breaks.len() {
            let lo = if k == 0 { bpb } else { breaks[k - 1] + bpb };
            let hi = breaks.get(k + 1).map_or(i64::MAX, |&x| x - bpb);
            let mut best = (breaks[k], sse_of(on, breaks));
            for d in [-2 * bpb, -bpb, bpb, 2 * bpb] {
                let c = best.0 + d;
                if c < lo || c > hi {
                    continue;
                }
                let mut trial = breaks.to_vec();
                trial[k] = c;
                let s = sse_of(on, &trial);
                if s < best.1 {
                    best = (c, s);
                }
            }
            breaks[k] = best.0;
        }
    }
}

/// #472 rule (owner 2026-10-27): Detect Tempo always suggests whole-number tempos, never
/// fractions. The map and every step carry the nearest whole BPM and the grid follows it;
/// the error bar does not hold the suggestion back (the fit's own values stay in
/// [`Detection::bpm`] and [`Detection::bpm_error_bar`] as diagnostics).
///
/// The whole-number tempo of a fitted period `p` (seconds) when the fit supports it: the
/// nearest whole BPM lies within the error bar `err` (over `beats` beats) and keeps the grid
/// within 1/16 beat of the fitted grid over the piece. `None` when the fit does not reach the
/// integer: the map still suggests it, but the grid is not re-read onto it (the #465
/// intercept search would then pull bar 1 off the hits).
fn clean_bpm(p: f64, err: f64, beats: f64) -> Option<f64> {
    let bpm = 60.0 / p;
    [1.0]
        .into_iter()
        .map(|q| round_to(bpm, q))
        .find(|&c| (c - bpm).abs() <= err && (60.0 / c - p).abs() * beats.max(1.0) <= p / 16.0)
}

/// The error bar in BPM of the period of piece `j` over `beats` beats: the larger of 2 standard
/// errors of the fit and the tempo change that moves the grid by the RMS jitter of the piece's
/// hits at its ends (with the grid centred on the hits). The second holds when the timing
/// drifts slowly, which the standard error, made for independent errors, does not see.
fn error_bar(g: &Grid, j: usize, kept: &[OnBeat], (lo, hi): (i64, i64), beats: f64) -> f64 {
    let p = g.periods[j];
    let res: Vec<f64> = kept
        .iter()
        .filter(|o| o.n >= lo && o.n < hi)
        .map(|o| o.t - g.time(o.n))
        .collect();
    let rms = if res.is_empty() {
        0.0
    } else {
        (res.iter().map(|r| r * r).sum::<f64>() / res.len() as f64).sqrt()
    };
    (2.0 * 60.0 * g.se[j] / (p * p)).max(60.0 * 2.0 * rms / beats.max(1.0) / (p * p))
}

/// The grid with each period on its whole-number tempo ([`round_to`] at 1 BPM) and `t0` fitted
/// again to the kept hits with the periods fixed. Also returns each period's error bar in BPM
/// of the fit before the snap (a diagnostic).
fn snap(g: &Grid, kept: &[OnBeat], last_beat: i64) -> (Grid, Vec<f64>) {
    let mut bounds = vec![0i64];
    bounds.extend(&g.breaks);
    bounds.push(last_beat.max(*bounds.last().unwrap_or(&0)));
    let mut out = g.clone();
    let mut moved = Vec::new();
    let mut errs = Vec::new();
    for (j, p) in out.periods.iter_mut().enumerate() {
        // the first piece holds the hits before beat 0, the last those after the last beat
        let lo = if j == 0 { i64::MIN } else { bounds[j] };
        let hi = if j + 1 == g.periods.len() {
            i64::MAX
        } else {
            bounds[j + 1]
        };
        let beats = (bounds[j + 1] - bounds[j]) as f64;
        errs.push(error_bar(g, j, kept, (lo, hi), beats));
        moved.push(clean_bpm(*p, errs[j], beats).is_some());
        *p = 60.0 / round_to(60.0 / *p, 1.0);
    }
    if moved.iter().any(|m| *m) && !kept.is_empty() {
        let base = Grid { t0: 0.0, ..out.clone() };
        let mean = kept.iter().map(|o| o.t - base.time(o.n)).sum::<f64>() / kept.len() as f64;
        // #465: the mean intercept is levered when the beat is a half or a double of the hits'
        // own finer grid. Session D (kick only, 92 BPM played, 46 suggested): the on-beat
        // window of the half tempo (1/8 beat) is a full 16th of the true grid, so hits one
        // 16th off the beat enter the fit asymmetrically (117 against 35 here) and the mean
        // lands 47 ms between the modes - bar 1 came out 53 ms before a beat of the hits'
        // grid. So the intercept is not the mean but the offset within a quarter beat that
        // puts the most hit weight on the grid (the same Gaussian kernel `phase` uses, here
        // with sigma = 1/32 of the first piece's period). On a tight or jittered train the
        // modes coincide with the mean and the offset stays 0.
        let p1 = out.periods[0];
        let sigma = p1 / 32.0;
        let score = |off: f64| -> f64 {
            kept.iter()
                .map(|o| {
                    let d = (o.t - base.time(o.n) - mean - off).abs();
                    (-0.5 * (d / sigma).powi(2)).exp()
                })
                .sum()
        };
        let steps = 200.0;
        let mut best = (0.0f64, score(0.0));
        for k in -200..=200 {
            let off = f64::from(k) / steps * p1 / 4.0;
            let s = score(off);
            if s > best.1 {
                best = (off, s);
            }
        }
        out.t0 = mean + best.0;
    }
    (out, errs)
}

// ---------------------------------------------------------------- meter and beat 1

struct Bars {
    bpb: i64,
    down: i64,
    alt: i64,
    confidence: f64,
    backbeat: f64,
    downbeat: f64,
    meter_confidence: f64,
    r2: f64,
    r3: f64,
    r4: f64,
    feel: Option<&'static str>,
}

/// Correlation at `lag` of per-beat features (each with its mean removed), summed over them.
fn corr(fs: &[Vec<f64>], lag: usize) -> f64 {
    let (mut s, mut a, mut b) = (0.0, 0.0, 0.0);
    for x in fs {
        if x.len() <= lag {
            continue;
        }
        let m = x.iter().sum::<f64>() / x.len() as f64;
        for i in 0..x.len() - lag {
            let (u, w) = (x[i] - m, x[i + lag] - m);
            s += u * w;
            a += u * u;
            b += w * w;
        }
    }
    if a > 0.0 && b > 0.0 { s / (a * b).sqrt() } else { 0.0 }
}

/// A sign test of two counts: `tanh(z / 2)` with `z = (a - b) / sqrt(a + b + 1)`, -1 to 1 (0 for
/// a tie; 0.42 for 3 against 1, 0.84 for 9 against 1, 0.45 for 38 against 30; a margin of a few
/// counts in hundreds stays small). The half makes the bins of the reliability table match their
/// accuracy (phase4-report.md "P4-8 fix round 2").
fn sign_test(a: f64, b: f64) -> f64 {
    (0.5 * (a - b) / (a + b + 1.0).sqrt()).tanh()
}

fn bars(hits: &[Hit], on: &[OnBeat], beats: &[f64]) -> Bars {
    let nb = beats.len();
    let (mut k, mut s, mut cym) = (vec![0.0; nb], vec![0.0; nb], vec![0.0; nb]);
    for o in on {
        let i = o.n as usize;
        match o.role {
            Role::Kick => k[i] += 1.0,
            Role::Snare => s[i] += 1.0,
            Role::Cymbal => cym[i] += 1.0,
            _ => {}
        }
    }
    // Per beat: kicks and snares between the beats (more than 1/8 beat from both), kicks an
    // eighth before it (a pushed beat 1), loud snare and tom hits (fills).
    let (mut koff, mut soff) = (vec![0.0; nb], vec![0.0; nb]);
    let (mut push, mut busy) = (vec![0.0; nb], vec![0.0; nb]);
    for h in hits {
        let i = beats.partition_point(|&b| b <= h.t);
        if i == 0 || i >= nb {
            continue;
        }
        let f = (h.t - beats[i - 1]) / (beats[i] - beats[i - 1]);
        if matches!(h.role, Role::Snare | Role::Tom) && h.v >= FILL_VELOCITY {
            busy[i - 1] += 1.0;
        }
        if f > ON_BEAT && f < 1.0 - ON_BEAT {
            match h.role {
                Role::Kick => koff[i - 1] += 1.0,
                Role::Snare => soff[i - 1] += 1.0,
                _ => {}
            }
        }
        if h.role == Role::Kick && (0.4..=0.6).contains(&(1.0 - f)) {
            push[i] += 1.0;
        }
    }
    let x: Vec<f64> = k.iter().zip(&s).map(|(a, b)| a - b).collect();
    let fs = [x, koff, soff];
    let (r2, r3, r4) = (corr(&fs, 2), corr(&fs, 3), corr(&fs, 4));
    let three = r3 >= 0.4 && r3 - r4 >= 0.5 && r3 - r2 >= 0.2;
    let bpb: i64 = if three { 3 } else { 4 };
    // 4/4 needs the 4-beat pattern to beat the 3-beat one and the 2-beat one: when the pattern
    // repeats every 2 beats as well, 2/4 or 6/8 in dotted quarters fit the hits as well.
    let feel = (!three && r4 - r2 < 0.05).then_some("two-beat");
    let meter_confidence = if three {
        r3 - r4.max(r2)
    } else {
        (r4 - r3).min(4.0 * (r4 - r2)).min(r4)
    }
    .clamp(0.0, 1.0);
    // Fill endings: the beat after a run of busy beats (3 loud snare or tom hits or more, and
    // over twice the median of the song).
    let med = median(&mut busy.clone());
    let dense = |i: usize| busy[i] >= 3.0 && busy[i] >= 2.0 * med + 1.0;
    let fill_end: Vec<f64> = (0..nb)
        .map(|i| f64::from(u8::from(i > 0 && dense(i - 1) && !dense(i))))
        .collect();
    let first_kick = on.iter().find(|o| o.role == Role::Kick).map(|o| o.n);
    let class = |v: &[f64], c: i64| -> f64 {
        v.iter()
            .enumerate()
            .filter(|(i, _)| (*i as i64).rem_euclid(bpb) == c.rem_euclid(bpb))
            .map(|(_, x)| x)
            .sum()
    };
    // Votes for beat `c` as beat 1: kicks on it or an eighth before it, crash hits on it (2 each),
    // fill endings on it (2 each), the first kick of the song (1).
    let votes = |c: i64| -> f64 {
        let first = first_kick.is_some_and(|n| n.rem_euclid(bpb) == c.rem_euclid(bpb));
        class(&k, c) + class(&push, c) + 2.0 * class(&cym, c) + 2.0 * class(&fill_end, c) + f64::from(u8::from(first))
    };
    let (down, alt, backbeat, downbeat) = if bpb == 4 {
        // The snare picks the pair of beats: its parity is the backbeat, the other is 1 and 3.
        let pair = |v: &[f64], c: i64| class(v, c) + class(v, c + 2);
        let (s0, s1) = (pair(&s, 0), pair(&s, 1));
        let (par, backbeat) = if s0 != s1 {
            (i64::from(s0 > s1), sign_test(s0.max(s1), s0.min(s1)))
        } else {
            // no snare evidence: the pair with more kicks
            let (k0, k1) = (pair(&k, 0), pair(&k, 1));
            (i64::from(k1 > k0), sign_test(k0.max(k1), k0.min(k1)))
        };
        // #180: under a half-time snare (one beat of the pair holds the snare, its partner
        // under half as many) the snare is on beat 3, so beat 1 is two beats from it, in the
        // snare's own pair. The 2-and-4 reading above picks the other pair: a beat late.
        let sp = i64::from(s1 > s0);
        let (h0, h1) = (class(&s, sp), class(&s, sp + 2));
        let (hi, lo, hc) = if h0 >= h1 { (h0, h1, sp) } else { (h1, h0, sp + 2) };
        if tempo_rule() >= 3 && s0 != s1 && hi >= HALF_TIME_MIN_SNARES && hi >= HALF_TIME_RATIO * lo {
            (hc + 2, hc, backbeat, sign_test(hi, lo))
        } else {
            let (a, b) = (votes(par), votes(par + 2));
            let (down, alt) = if a >= b { (par, par + 2) } else { (par + 2, par) };
            (down, alt, backbeat, sign_test(a.max(b), a.min(b)))
        }
    } else {
        let scores: Vec<f64> = (0..bpb).map(|o| votes(o) - class(&s, o)).collect();
        let mut order: Vec<usize> = (0..scores.len()).collect();
        order.sort_by(|&a, &b| scores[b].total_cmp(&scores[a]));
        let (b1, b2) = (scores[order[0]], scores[order[1]]);
        let downbeat = (0.5 * (b1 - b2) / (b1.abs() + b2.abs() + 1.0).sqrt()).tanh();
        (order[0] as i64, order[1] as i64, 1.0, downbeat)
    };
    Bars {
        bpb,
        down,
        alt,
        confidence: (backbeat * downbeat).clamp(0.0, 1.0),
        backbeat: backbeat.clamp(0.0, 1.0),
        downbeat: downbeat.clamp(0.0, 1.0),
        meter_confidence,
        r2,
        r3,
        r4,
        feel,
    }
}

// ---------------------------------------------------------------- the #317 step guard

/// True when `a` is within [`GUARD_RATIO_TOL`] of a simple ratio of `b`
/// ([`GUARD_RATIOS`]).
fn is_simple_ratio(a: f64, b: f64) -> bool {
    let r = a / b;
    GUARD_RATIOS
        .iter()
        .any(|&q| (r / q).ln().abs() < GUARD_RATIO_TOL.ln_1p())
}

/// Weighted share of the kick and snare hits on the best-phase beat grid of `period` (the
/// on-beat window of [`ON_BEAT`], the phase searched as `phase` does). 0 without hits.
fn grid_share(hits: &[Hit], t0: f64, t1: f64, period: f64) -> f64 {
    let mut best = 0.0f64;
    for k in 0..200 {
        let phi = f64::from(k) / 200.0 * period;
        let (mut w_on, mut w_all) = (0.0, 0.0);
        for h in hits.iter().filter(|h| h.role.is_ks() && h.t >= t0 && h.t < t1) {
            let x = (h.t - phi).rem_euclid(period);
            let d = x.min(period - x);
            let w = h.weight();
            w_all += w;
            if d < ON_BEAT * period {
                w_on += w;
            }
        }
        if w_all > 0.0 {
            best = best.max(w_on / w_all);
        }
    }
    best
}

/// #317, hypothesis 2 ("same tempo, other level"): a piece whose tempo sits within 2 % of a
/// simple ratio (4/3, 3/2, 2 or an inverse) of the piece before it stays only when its own
/// grid explains the piece clearly better than the old grid does. A sparse or noisy passage
/// (Session A at 236 s: 13 kick and snare hits in 16 s, 4 of them on the song's grid) reads a
/// 4/3 period in the local comb that fits the hits no better than the song's tempo; the tie
/// goes to the old tempo, and the piece is dropped (the map keeps one tempo event). A real
/// step at such a ratio puts its hits on its own grid and off the old one, so it survives.
fn guard_simple_ratio_pieces(hits: &[Hit], pcs: &[Piece]) -> Vec<Piece> {
    if pcs.len() < 2 {
        return pcs.to_vec();
    }
    let end = hits.iter().map(|h| h.t).fold(f64::NEG_INFINITY, f64::max) + 1.0;
    let mut out: Vec<Piece> = Vec::with_capacity(pcs.len());
    for (j, p) in pcs.iter().enumerate() {
        let keep = match out.last() {
            Some(prev) if is_simple_ratio(p.period, prev.period) => {
                let next = pcs.get(j + 1).map_or(end, |n| n.start);
                let n_ks = hits
                    .iter()
                    .filter(|h| h.role.is_ks() && h.t >= p.start && h.t < next)
                    .count();
                // The guard decides only where there is something to compare (the local comb
                // needs 6 kick and snare hits in a window too); a thinner piece is left to
                // the fit's short-piece rule. Hypothesis 3 (#317): a share difference of a
                // few hits is noise. The on-beat window is a quarter beat wide, so a grid
                // without a fit reads about 0.25; the difference of the two shares must clear
                // two standard errors of that chance level at the piece's hit count (the
                // weights of `grid_share` are near uniform at the default velocity).
                n_ks < 6 || {
                    let old = grid_share(hits, p.start, next, prev.period);
                    let new = grid_share(hits, p.start, next, p.period);
                    new - old > 2.0 * (0.25f64 * 0.75 / n_ks as f64).sqrt()
                }
            }
            _ => true,
        };
        if keep {
            out.push(*p);
        }
    }
    out
}

// ---------------------------------------------------------------- the detection

fn round_to(x: f64, q: f64) -> f64 {
    (x / q).round() * q
}

/// Suggest a tempo map from the hits. Errors: fewer than [`MIN_HITS`] kick and snare hits, no
/// regular beat, hits spanning more than [`MAX_SPAN_S`], parameters out of range.
///
/// # Errors
///
/// A message when `params` fails [`DetectParams::validate`], there are fewer than [`MIN_HITS`]
/// kick and snare hits, the hits span more than [`MAX_SPAN_S`], or no regular beat is found.
pub fn detect(hits: &[Hit], params: &DetectParams) -> Result<Detection, String> {
    let mut d = detect_at(hits, params)?;
    if params.min_bars > PROBE_BARS {
        // The same detection with a short minimum: a piece it finds that is shorter than
        // `min_bars` and differs from the map is an excursion the map cannot hold.
        let probe = DetectParams {
            min_bars: PROBE_BARS,
            ..*params
        };
        if let Ok(p) = detect_at(hits, &probe) {
            let at = |t: f64| {
                let mut bpm = d.map.bpm;
                for s in d.map.steps.iter().take_while(|s| s.t <= t + 1e-6) {
                    bpm = s.bpm;
                }
                bpm
            };
            d.excursions = p
                .steps
                .iter()
                .filter(|s| s.bars < i64::from(params.min_bars))
                .filter(|s| (s.bpm / at(s.t)).ln().abs() >= (1.0 + params.min_step_pct / 100.0).ln())
                .map(|s| Excursion {
                    t: s.t,
                    bars: s.bars,
                    bpm: s.bpm,
                    map_bpm: at(s.t),
                })
                .collect();
        }
    }
    Ok(d)
}

/// The answer of [`refit_bar1`]: the new time of bar 1 and the hit weight that supports its phase.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bar1Refit {
    /// The new time of bar 1 in seconds: the supported beat phase, at the occurrence nearest to
    /// the old bar 1 (the bar keeps its number).
    pub bar1_s: f64,
    /// The kernel weight of the kick and snare hits on the phase (role and velocity weighted).
    pub support: f64,
}

/// The beat phase of `bpm` that the kick and snare hits support, as the new time of bar 1.
/// #468: a typed start tempo re-fits bar 1 to the hits, so the typed grid sits on them (a map
/// with steps keeps bar 1: the caller decides). The phase is the detector's kernel estimate
/// ([`phase`], `phase_kernel`) over the whole hit range, searched within a quarter beat of the
/// current bar 1 — the same local rule the detector's `snap` uses, so bar 1 keeps its number
/// and its musical position and only slides to the nearest supported phase (the global winner
/// could be a 16th or an eighth off: this session's 16th "and" carries more hits than the
/// beats). Bar 1 is never before 0 s. Errors: a tempo that is not positive and finite, fewer
/// than [`MIN_HITS`] kick and snare hits.
///
/// # Errors
///
/// A message when `bpm` is not a positive finite number, or `hits` holds fewer than
/// [`MIN_HITS`] kick and snare hits.
pub fn refit_bar1(hits: &[Hit], bpm: f64, current_bar1_s: f64) -> Result<Bar1Refit, String> {
    if !(bpm.is_finite() && bpm > 0.0) {
        return Err(format!("tempo {bpm} BPM: the refit needs a positive tempo"));
    }
    let ks: Vec<Hit> = hits.iter().filter(|h| h.role.is_ks()).copied().collect();
    if ks.len() < MIN_HITS {
        return Err(format!(
            "{} kick and snare hits: the refit needs at least {MIN_HITS} (analyse the kick and snare tracks first)",
            ks.len()
        ));
    }
    let p = 60.0 / bpm;
    let (t0, t1) = (ks[0].t, ks[ks.len() - 1].t);
    let from = current_bar1_s.rem_euclid(p) - 0.25 * p;
    let mut best = (from.rem_euclid(p), f64::NEG_INFINITY);
    for k in 0..=400 {
        let phi = (from + 0.5 * p * f64::from(k) / 400.0).rem_euclid(p);
        let s = phase_kernel(&ks, p, t0, t1 + p, phi);
        if s > best.1 {
            best = (phi, s);
        }
    }
    let (phi, support) = best;
    // Bar 1 before 0 s is not a valid map (`TempoMap::validate`): a downbeat on the first hit can
    // fit a hair early (-0.8 ms on a real session, owner hotfix 0.9.10), so the refit clamps to
    // 0 s like the detector does with its own bar 1.
    let bar1_s = (phi + ((current_bar1_s - phi) / p).round() * p).max(0.0);
    Ok(Bar1Refit { bar1_s, support })
}

/// Share of the kick and snare hits within 1/8 beat of `grid`, and their RMS distance from it in ms.
fn quality(hits: &[Hit], beats: &[f64], grid: &Grid) -> (f64, f64) {
    let mut w_all: f64 = 0.0;
    let mut w_on: f64 = 0.0;
    let mut sq = 0.0;
    let mut cnt = 0.0;
    for h in hits.iter().filter(|h| h.role.is_ks()) {
        w_all += 1.0;
        let i = beats.partition_point(|&b| b < h.t) as i64;
        let near = [i - 1, i]
            .into_iter()
            .map(|n| (n, h.t - grid.time(n)))
            .min_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
            .unwrap_or((0, f64::INFINITY));
        let q = grid.time(near.0 + 1) - grid.time(near.0);
        if near.1.abs() < ON_BEAT * q {
            w_on += 1.0;
            sq += near.1 * near.1;
            cnt += 1.0;
        }
    }
    let on_beat = if w_all > 0.0 { w_on / w_all } else { 0.0 };
    let jitter_ms = if cnt > 0.0 { (sq / cnt).sqrt() * 1000.0 } else { 0.0 };
    (on_beat, jitter_ms)
}

/// Steps 3 to 5 for one set of pieces: the tracked beats, the meter and beat 1, the on-beat hits,
/// the fitted grid with the hits of its last pass, and the share of kick and snare hits on that grid.
struct TrackFit {
    beats: Vec<f64>,
    bars: Bars,
    on: Vec<OnBeat>,
    fitted: Grid,
    kept: Vec<OnBeat>,
    on_beat: f64,
}

fn track_and_fit(
    hits: &[Hit],
    pcs: &[Piece],
    params: &DetectParams,
    phi_hint: Option<f64>,
) -> Result<TrackFit, String> {
    let (beats, switches) = track(hits, pcs, phi_hint);
    let on_all = assign(hits, &beats);
    let bars = bars(hits, &on_all, &beats);
    let on = drop_isolated(&on_all);
    let mut breaks: Vec<i64> = switches
        .iter()
        .filter(|&&s| s != usize::MAX)
        .map(|&s| s as i64)
        .collect();
    place_breaks(&on, &mut breaks, bars.down, bars.bpb);
    breaks.dedup();
    let step = (1.0 + params.min_step_pct / 100.0).ln();
    let min_beats = i64::from(params.min_bars) * bars.bpb;
    let last_beat = beats.len() as i64 - 1;
    let (fitted, kept) = loop {
        let Some((g, kept)) = fit(&on, &breaks) else {
            if breaks.pop().is_none() {
                return Err("the hits show no regular beat".into());
            }
            continue;
        };
        // Lengths of the pieces in beats, and the tempo ratio to the piece before.
        let mut drop: Option<usize> = None;
        let mut bounds = vec![0i64];
        bounds.extend(&breaks);
        bounds.push(last_beat.max(*bounds.last().unwrap_or(&0)));
        for j in 1..g.periods.len() {
            let small = (g.periods[j] / g.periods[j - 1]).ln().abs() < step;
            let short = bounds[j + 1] - bounds[j] < min_beats || bounds[j] - bounds[j - 1] < min_beats;
            if small || short {
                drop = Some(j - 1);
                break;
            }
        }
        match drop {
            Some(k) => {
                breaks.remove(k);
            }
            None => break (g, kept),
        }
    };
    let (on_beat, _) = quality(hits, &beats, &fitted);
    Ok(TrackFit {
        beats,
        bars,
        on,
        fitted,
        kept,
        on_beat,
    })
}

fn detect_at(hits: &[Hit], params: &DetectParams) -> Result<Detection, String> {
    params.validate()?;
    let prep = prepare(hits, params.duration_s);
    let hits = prep.hits;
    let ks = hits.iter().filter(|h| h.role.is_ks()).count();
    if ks < MIN_HITS {
        return Err(format!(
            "{ks} kick and snare hits: the detection needs at least {MIN_HITS} (analyse the kick and snare tracks first)"
        ));
    }
    let span = hits[hits.len() - 1].t - hits[0].t;
    if span > MAX_SPAN_S {
        return Err(format!(
            "the hits span {span:.0} s, more than {MAX_SPAN_S:.0} s: check for hits far after the song"
        ));
    }
    let global = global_period(&hits, params)?;
    let mut pcs = guard_simple_ratio_pieces(&hits, &pieces(&hits, global.period, params));
    // #472: when the referee doubled the tempo, the half tempo's grid carries the phase.
    let phi_hint = global
        .prev_period
        .map(|hp| phase(&hits, hp, hits[0].t, hits[0].t + 16.0 * hp));
    let mut run = track_and_fit(&hits, &pcs, params, phi_hint)?;
    // A new tempo must earn its piece (P4-39e): each piece after the first is tried away (the
    // tracker keeps the tempo before it) and stays only when the grid with it holds more kick and
    // snare hits on the beat. In a sparse section (one kick pair per bar) a period that divides the
    // bar in 3 fits the hits as well as the beat; the tracker took it for 6 beats, the fit dropped
    // that short piece but kept its beat numbers, 2 beats short for every later hit, and the grid
    // bent (117.48 BPM instead of 118 on Session E).
    let mut tried: Vec<f64> = Vec::new();
    while let Some(j) = (1..pcs.len()).find(|&j| !tried.contains(&pcs[j].start)) {
        tried.push(pcs[j].start);
        let mut fewer = pcs.clone();
        fewer.remove(j);
        if let Ok(r) = track_and_fit(&hits, &fewer, params, phi_hint)
            && r.on_beat >= run.on_beat
        {
            pcs = fewer;
            run = r;
        }
    }
    let TrackFit {
        beats,
        bars,
        on,
        fitted,
        kept,
        ..
    } = run;
    let last_beat = beats.len() as i64 - 1;
    // #472: the map and every step carry whole-number tempos; the fitted values stay in
    // `Detection::bpm` as a diagnostic.
    let (grid, errs) = snap(&fitted, &kept, last_beat);
    let tempo = |j: usize| round_to(60.0 / grid.periods[j], 1.0);
    // Bar 1: the first downbeat of the grid at or after 0 s (an early hit within a quarter
    // beat of 0 s counts as 0 s).
    let p1 = grid.periods[0];
    let n_min = ((-0.25 * p1 - grid.t0) / p1).ceil() as i64;
    let bar1_n = n_min + (bars.down - n_min).rem_euclid(bars.bpb);
    let bar1_s = grid.time(bar1_n).max(0.0);
    let bar1_alternative_s = grid.time(bar1_n + (bars.alt - bars.down).rem_euclid(bars.bpb));
    let (on_beat, jitter_ms) = quality(&hits, &beats, &grid);
    let share = (on_beat / 0.5).min(1.0);
    let confidence = round_to(share * (2.0 * global.margin).clamp(0.0, 1.0), 0.001);
    let mut bounds = vec![0i64];
    bounds.extend(&grid.breaks);
    bounds.push(last_beat.max(*bounds.last().unwrap_or(&0)));
    let steps: Vec<StepSuggestion> = (1..grid.periods.len())
        .map(|j| {
            let z = (fitted.periods[j] - fitted.periods[j - 1]).abs()
                / (grid.se[j].powi(2) + grid.se[j - 1].powi(2)).sqrt().max(1e-9);
            let (a, b) = (bounds[j], bounds[j + 1]);
            let tot = hits
                .iter()
                .filter(|h| h.role.is_ks() && h.t >= grid.time(a) && h.t < grid.time(b))
                .count() as f64;
            let onp = on.iter().filter(|o| o.role.is_ks() && o.n >= a && o.n < b).count() as f64;
            let sh = if tot > 0.0 { (onp / tot / 0.5).min(1.0) } else { 0.0 };
            StepSuggestion {
                t: round_to(grid.time(a), 1e-5),
                bpm: tempo(j),
                bar: (a - bar1_n).div_euclid(bars.bpb) + 1,
                bars: (b - a) / bars.bpb,
                confidence: round_to((1.0 - (-z / 8.0).exp()) * sh, 0.001),
            }
        })
        .collect();
    // #472: a step whose whole-number tempo equals the tempo in force is no step.
    let mut in_force = tempo(0);
    let steps: Vec<StepSuggestion> = steps
        .into_iter()
        .filter(|s| {
            if (s.bpm - in_force).abs() < 0.5 {
                false
            } else {
                in_force = s.bpm;
                true
            }
        })
        .collect();
    let fp1 = fitted.periods[0];
    let map = TempoMap {
        bpm: tempo(0),
        steps: steps.iter().map(|s| TempoStep { t: s.t, bpm: s.bpm }).collect(),
        meter: (bars.bpb as u8, 4),
        changes: Vec::new(),
        bar1_s: round_to(bar1_s, 1e-5),
    };
    map.validate()?;
    Ok(Detection {
        map,
        bpm: 60.0 / fp1,
        bpm_error_bar: errs[0],
        confidence,
        alternatives: global.alternatives,
        range_edge: global.range_edge,
        bar1_s,
        bar1_confidence: round_to(bars.confidence, 0.001),
        bar1_backbeat: round_to(bars.backbeat, 0.001),
        bar1_downbeat: round_to(bars.downbeat, 0.001),
        bar1_alternative_s,
        meter: (bars.bpb as u8, 4),
        meter_confidence: round_to(bars.meter_confidence, 0.001),
        meter_r2: round_to(bars.r2, 0.001),
        meter_r3: round_to(bars.r3, 0.001),
        meter_r4: round_to(bars.r4, 0.001),
        meter_feel: bars.feel,
        steps,
        on_beat: round_to(on_beat, 0.001),
        jitter_ms: round_to(jitter_ms, 0.01),
        hits: hits.len(),
        late: prep.late,
        isolated: prep.isolated,
        beats: beats.len(),
        excursions: Vec::new(),
    })
}

#[cfg(test)]
mod tests;
