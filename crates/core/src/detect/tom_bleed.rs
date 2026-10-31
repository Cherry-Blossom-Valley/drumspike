//! The tom bleed rule CD (P4-14 stage 2): a session step after the per-track analysis that marks the hits of a tom
//! close mic which the other drums' tom close mics explain.
//!
//! Measurements and the choice of the rule are in `docs/migration/phase4-report.md`, sections "P4-14 stage 1" to
//! "P4-14 stage 2". The Python reference is `tools/tom_bleed_cd.py` (`rule_cd`).
//!
//! # Input and output
//!
//! [`tom_bleed`] takes the tom close mics of one session ([`TomMic`]: the drum it belongs to, the mono signal at the
//! session rate, the hits as sample positions). Sample, trigger and second mics of a drum are the caller's business:
//! two mics of one drum (the same [`TomMic::drum`]) never explain each other, and a sample or trigger track is no tom
//! close mic at all. It gives per mic the measures below, the transfer gains, the excess of every hit and the hits
//! the rule marks as bleed ([`TomBleed`]).
//!
//! # The rule
//!
//! L is the tom-band level (50 to 400 Hz, [`Role::band`]) of a mic in `[-0.5, +8)` ms at a hit (P2-6's
//! `pair_window_ms`, the filter run in over 50 ms before the window, zeros outside the signal).
//!
//! 1. Strong hits of a mic: the P2-6 rule (broadband peak in `[-0.5, +5)` ms within 6 dB of the mic's 90th
//!    percentile). The mic's level is the median L of its strong hits.
//! 2. Transfer gain `G(B->A)` for mics of different drums: P2-6's calibration ([`calibrate_gain`]) of `L_A - L_B` at
//!    B's strong hits: the 10th percentile over the hits where A has no own hit within 30 ms, when there are at
//!    least 5 of them, else over all; no gain with fewer than 5 values.
//! 3. Played (the stricter test of the P4-14 verifier rounds): a mic is played when its level is at most 6 dB under
//!    the median level of the other drums' mics AND fewer than half of its strong hits are at another drum's hit
//!    (a listed hit of another drum's mic within 30 ms whose L at that time is higher than this mic's own L). A mic
//!    that hears only bleed has its strong hits where another tom was struck, and that tom's mic is louder there.
//! 4. Excess of a hit of mic A: the lowest `L_A - L_B - G(B->A)` over the played mics B of other drums with a gain
//!    ([`NO_SOURCE_DB`] when there is none). The hit is bleed when its excess is under `threshold_db` (9.2 dB, the
//!    controller's choice: 1.5 dB under the fitted 10.7 dB).
//! 5. The rule is off ([`Off`]) when a mic is edited to digital silence (at a quarter or more of the other drums'
//!    strong hits its L is at or below -100 dB, P2-6's `silence_db`: the gains of a gated mic say nothing about the
//!    transfer), or when fewer than two drums have a played mic.
//!
//! 6. Shared pairs (fix round 3): a pair whose summed gains `G(A->B) + G(B->A)` lie above `shared_sum_db` (-26 dB)
//!    hears one drum about as loud on both mics. The levels keep their direction only when the weaker mic is at least
//!    `keep_gap_db` (8 dB, fix round 5) under the other drums without the pair: a bleed-only mic as recorded. Else (a
//!    hot preamp, or a quiet main tom) the pair counts as one drum and the rule runs again ([`merged_pairs`],
//!    [`TomBleed::merged`]).
//!
//! # Rule version 2: the kick and snare close mics as sources (P4-17 stage 2)
//!
//! [`tom_bleed_with`] adds the session's kick and snare close mics ([`SourceMic`]; the caller leaves out sample,
//! trigger and follower tracks). They are sources only: never targets, not in the played test, not in the shared-pair
//! merge. For a tom mic A and a source S: `G(S->A)` is the transfer gain at S's strong hits (A quiet within 30 ms, as
//! point 2), `G(A->S)` the reverse at A's strong hits. S is not used for A without `G(S->A)`, when A is digital
//! silence at a quarter or more of S's strong hits, or when `G(S->A) + G(A->S)` lies above `shared_sum_db` (a shared
//! pair). The source excess of a hit of A is `L_A - L_S - G(S->A)`, the lowest over the used sources of one kind
//! ([`TomBleed::source_excess`]). A hit is bleed when rule CD marks it, or its snare excess is under
//! `snare_threshold_db` (9.2 dB, as the tom sources), or its kick excess is under `kick_threshold_db` (4.0 dB: a tom
//! played with the kick is a common accent). Nothing is marked when rule CD is off. The Python reference is
//! `tools/tom_bleed_sk.py` (`sk_excess`, `rule_sk`).
//!
//! # Rule version 3: the tom pair rule, winner takes all (P4-21)
//!
//! [`tom_pairs`] runs after version 2 on the hits that are still in the output. The level of a hit is the peak of its
//! mic in `[t - 5, t + 30]` ms ([`pair_level_db`]) minus the median of that peak over the output hits of its own mic.
//! Of two output hits on mics of different drums within `winner_window_ms` (12 ms) the one whose relative level is
//! `winner_margin_db` (6 dB) or more under the other's is bleed. Hits are visited loudest first; a hit is marked when a kept
//! hit of another drum lies in the window and is that much louder; a marked hit protects no other. Two mics of one drum
//! never pair. A hit is marked only when its own relative level is also at most `winner_floor_db` ([`PAIR_FLOOR_DB`], -3 dB):
//! bleed is quiet on its own mic, a real unison stroke is not. The rule needs no bleed between the hits, so it runs when rule CD is off (tom tracks edited to digital
//! silence). The Python reference is `tools/tom_pairs.py` (`pair_marks`).
//!
//! # Rule version 4: the own-mic attack rise keep guard (#182)
//!
//! The pair rule lost real strokes: 42 of its 46 merges on the toms of the 3 real sessions were real strokes
//! (`docs/migration/reports/tom-pair-risk.md`). A marked hit whose own mic has an attack stays: when the rise of
//! the hit ([`stroke_rise_db`], the P4-18 rise test) is [`PAIR_KEEP_RISE_DB`] (10 dB) or more, the mic is not
//! marked. The measured guard keeps all 42 lost real strokes and unmarks no bleed merge. The guard applies per
//! mic from version 4 ([`PairMic::guarded`]): a recorded version 3 re-pick next to a version 4 target runs the
//! version 3 pair rule.
//!
//! # Rule version 5: the guarded shared kick source (#362)
//!
//! On ENST `drummer_3` the floor tom (`tom_3`) sits next to the kick: the kick and the floor tom hear each other
//! about as loud, so `G(kick->tom_3) + G(tom_3->kick)` lies above [`TomBleedParams::shared_sum_db`] and version 2
//! rejects the kick source as a shared pair - the kick bleed on the floor tom stays in the output (0.264 F1, 1682
//! kick false positives; `docs/migration/reports/release-0.9.2-enst.md`). Version 5 keeps a kick source whose gain
//! pair is shared, under a guarded rule: a hit is marked when its kick excess is under
//! [`TomBleedParams::kick_shared_threshold_db`] (8.0 dB, the bleed is louder than the unguarded 4.0 dB threshold
//! reaches: measured p50 4.8 dB, real strokes p5 24 dB) and the hit has no own 2 kHz attack on the tom mic (the
//! high-band part of [`stroke_rise_db`] under [`TomBleedParams::kick_shared_rise_db`], 15.0 dB: a real stroke's
//! stick click, which a kick bleed leaves on the tom mic about as often as not - the full-band rise does not
//! separate, the kick's own transient is an attack). Measured on the 68 `drummer_3` items: about 1140 kick false
//! positives marked, 1 real stroke lost. The marks are in [`TomBleed::shared_kick_bleed`] and, when rule CD is on,
//! also in [`TomBleed::bleed`]; when rule CD is off for fewer than two played tom close mics they stay in
//! [`TomBleed::shared_kick_bleed`] alone (the caller applies them there; never for a mic edited to digital
//! silence, where no level means anything).
//!
//! # Determinism
//!
//! Every value is computed in a fixed order in `f64`; window bounds use [`ms_to_samples`].

use super::attribution::{
    AttributionError, AttributionParams, Biquad, FilterKind, Role, Transfer, band_peak_db, band_sections,
    calibrate_gain, ms_to_samples, percentile,
};
use super::flam::zero_phase;

/// The version of the rule an analysis records (`detector.bleed_rule`); 0 means no rule, 1 rule CD (tom sources only),
/// 2 rule CD with the kick and snare close mics as sources ([`tom_bleed_with`]), 3 version 2 plus the tom pair rule
/// ([`tom_pairs`]), 4 version 3 plus the pair rule's own-mic attack rise keep guard (#182), 5 version 4 plus the
/// guarded shared kick source (#362).
pub const RULE_VERSION: u32 = 5;

/// Version 3: the level floor of the pair rule (dB against the mic's median): only a hit this far under its own mic's
/// median can be marked. Chosen on the seven yardstick songs (`tools/tom_pairs.py`).
pub const PAIR_FLOOR_DB: f64 = -3.0;

/// Version 4: a hit the pair rule would mark stays a hit when its own-mic attack rise ([`stroke_rise_db`]) is this
/// many dB or more (the P4-18 rise test, `tools/firm_fixes.py` `RISE_MIN_DB`). Measured in #177 on the toms of the
/// 3 real sessions: it keeps all 42 lost real strokes and unmarks no bleed merge (a smaller threshold unmarks bleed).
pub const PAIR_KEEP_RISE_DB: f64 = 10.0;

/// Version 4: the high-pass of the attack rise (Hz, `tools/tom_fixes.py` `HP_HZ`): a tom's ring is low, its stick
/// click is not; in a roll the ring of the earlier hit hides the full-band rise, the click over the faster decaying
/// highs does not.
pub const PAIR_HP_HZ: f64 = 2000.0;

/// Version 5: the kick excess under which a hit is marked by the guarded shared kick source (dB, #362). Higher than
/// the unguarded [`TomBleedParams::kick_threshold_db`] (4.0): on ENST `drummer_3` the kick bleed on the floor tom lies
/// at p50 4.8, p95 9.7 dB, the real strokes at p5 24 dB, so 8.0 catches most of the bleed; the rise guard keeps the
/// real strokes (1 of 371 lost at this value, 15 without it).
pub const KICK_SHARED_THRESHOLD_DB: f64 = 8.0;

/// Version 5: a hit the guarded shared kick source would mark stays a hit when its own 2 kHz attack rise (the
/// high-band part of [`stroke_rise_db`]) is this many dB or more (dB, #362). On ENST `drummer_3` the kick-only bleed
/// sits at p50 13.1, the real strokes (with or without the kick) at p50 30 or more; 15.0 keeps all but 1 of the
/// real strokes while the unguarded rule loses 15.
pub const KICK_SHARED_RISE_DB: f64 = 15.0;

/// The excess of a hit without a played source mic (never bleed).
pub const NO_SOURCE_DB: f64 = 200.0;

/// Parameters. [`Default`] gives the values of stage 1c and the verifier rounds.
#[derive(Debug, Clone, PartialEq)]
pub struct TomBleedParams {
    /// A hit is bleed when its excess is under this (dB).
    pub threshold_db: f64,
    /// Played: the mic's level is at most this many dB under the median level of the other drums' mics.
    pub played_gap_db: f64,
    /// Played: the share of the strong hits at a louder hit of another drum is under this.
    pub louder_share: f64,
    /// Edited to digital silence: the share of the other drums' strong hits where the mic is silent is at least this.
    pub silent_share: f64,
    /// Levels at or below this (dBFS) are digital silence.
    pub silence_db: f64,
    /// The level window at a hit (ms).
    pub window_ms: (f64, f64),
    /// The broadband window of the strong-hit test (ms).
    pub strong_window_ms: (f64, f64),
    /// Strong hits: within this many dB of the mic's 90th percentile peak.
    pub strong_db: f64,
    /// Filter run-in before a level window (ms).
    pub preroll_ms: f64,
    /// A mic is quiet at a time when it has no own hit within this many ms; also the reach of "at another drum's hit".
    pub quiet_ms: f64,
    /// Percentile of the transfer values.
    pub percentile: f64,
    /// Minimum number of values for a gain or a silence share.
    pub min_hits: usize,
    /// A pair of mics whose summed transfer gains `G(A->B) + G(B->A)` lie above this (dB) hears one drum about as
    /// loud on both mics (a "shared" pair; played pairs -29.0 to -61.2 dB, pairs with a bleed-only mic -7.1 to -22.5
    /// dB on the eight sessions).
    pub shared_sum_db: f64,
    /// Fix round 5: a shared pair keeps the direction of its levels only when its weaker mic is at least this many dB
    /// under the other drums (without the pair, one level per drum); else it counts as one drum. Real bleed-only mics
    /// are 8.4 to 13.3 dB under, a quiet main tom 6.2 dB.
    pub keep_gap_db: f64,
    /// Version 2: a hit is bleed when its snare excess is under this (dB).
    pub snare_threshold_db: f64,
    /// Version 2: a hit is bleed when its kick excess is under this (dB); lower than the others, as a tom played with
    /// the kick is a common accent (the three hits P4-17 stage 1 lost lie at a kick excess of 4.5 to 4.8 dB).
    pub kick_threshold_db: f64,
    /// Version 3: two output hits of different drums this close (ms) are a pair.
    pub winner_window_ms: f64,
    /// Version 3: of a pair the hit whose relative level is at least this many dB under the other's is bleed.
    pub winner_margin_db: f64,
    /// Version 3: a hit is marked only when its own relative level is at most this (dB, against the median of its mic's
    /// output hits): bleed is quiet on its own mic, a real unison stroke is not (P4-21 fix round).
    pub winner_floor_db: f64,
    /// Version 4: a hit the pair rule would mark stays a hit when its own-mic attack rise is this many dB or more.
    pub pair_keep_rise_db: f64,
    /// Version 5: the kick excess under which a hit is marked by the guarded shared kick source (dB).
    pub kick_shared_threshold_db: f64,
    /// Version 5: a hit the guarded shared kick source would mark stays a hit when its own 2 kHz attack rise is this
    /// many dB or more (dB).
    pub kick_shared_rise_db: f64,
}

impl Default for TomBleedParams {
    fn default() -> Self {
        let p = AttributionParams::default();
        TomBleedParams {
            threshold_db: 9.2,
            played_gap_db: 6.0,
            louder_share: 0.5,
            silent_share: 0.25,
            silence_db: p.silence_db,
            window_ms: p.pair_window_ms,
            strong_window_ms: p.own_window_ms,
            strong_db: p.calib_strong_db,
            preroll_ms: p.preroll_ms,
            quiet_ms: p.calib_quiet_ms,
            percentile: p.calib_percentile,
            min_hits: p.calib_min_hits,
            shared_sum_db: -26.0,
            keep_gap_db: 8.0,
            snare_threshold_db: 9.2,
            kick_threshold_db: 4.0,
            winner_window_ms: 12.0,
            winner_margin_db: 6.0,
            winner_floor_db: PAIR_FLOOR_DB,
            pair_keep_rise_db: PAIR_KEEP_RISE_DB,
            kick_shared_threshold_db: KICK_SHARED_THRESHOLD_DB,
            kick_shared_rise_db: KICK_SHARED_RISE_DB,
        }
    }
}

/// One tom close mic of the session.
#[derive(Debug, Clone)]
pub struct TomMic<'a> {
    /// The drum the mic belongs to (`tom1`, or the track name when the name tells no drum). Mics of one drum never
    /// explain each other.
    pub drum: String,
    /// Mono signal at the session rate.
    pub signal: &'a [f32],
    /// The hits as sample positions at the session rate, in any order.
    pub hits: Vec<i64>,
}

/// The kind of a source mic of version 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    /// A snare close mic.
    Snare,
    /// A kick close mic.
    Kick,
}

impl SourceKind {
    /// The index into [`TomBleed::source_excess`].
    #[must_use]
    pub fn index(self) -> usize {
        match self {
            SourceKind::Snare => 0,
            SourceKind::Kick => 1,
        }
    }
}

/// A kick or snare close mic of the session (version 2): a source only.
#[derive(Debug, Clone)]
pub struct SourceMic<'a> {
    /// Snare or kick.
    pub kind: SourceKind,
    /// Mono signal at the session rate.
    pub signal: &'a [f32],
    /// The hits as sample positions at the session rate, in any order.
    pub hits: Vec<i64>,
}

/// Why a source is not used for a tom mic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unused {
    /// No gain `G(S->A)` (fewer than `min_hits` strong hits of the source).
    NoGain,
    /// The tom mic is digital silence at a quarter or more of the source's strong hits.
    Silent,
    /// `G(S->A) + G(A->S)` lies above `shared_sum_db`.
    Shared,
}

/// One source for one tom mic (version 2).
#[derive(Debug, Clone, PartialEq)]
pub struct SourceUse {
    /// The tom mic (input index).
    pub tom: usize,
    /// The source (index into the sources).
    pub source: usize,
    /// `G(S->A)` (dB).
    pub gain_db: Option<f64>,
    /// `G(A->S)` (dB).
    pub back_db: Option<f64>,
    /// The share of the source's strong hits where the tom mic is digital silence; `None` with fewer than
    /// `min_hits` of them.
    pub silent_share: Option<f64>,
    /// `None` when the source is used.
    pub unused: Option<Unused>,
}

/// Why the rule is off for a session.
#[derive(Debug, Clone, PartialEq)]
pub enum Off {
    /// Fewer than two drums have a played mic; the played mics (input indices).
    FewerThanTwoPlayed {
        /// Indices of the played mics.
        played: Vec<usize>,
    },
    /// These mics (input indices) are edited to digital silence.
    DigitalSilence {
        /// Indices of the silent mics.
        mics: Vec<usize>,
    },
}

/// The measures of one mic.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MicStats {
    /// Number of strong hits.
    pub strong: usize,
    /// The median tom-band level of the strong hits (dBFS); `None` without strong hits.
    pub level_db: Option<f64>,
    /// The level part of the played test.
    pub level_ok: bool,
    /// The share of the strong hits at a louder hit of another drum; `None` without strong hits.
    pub louder_share: Option<f64>,
    /// Both parts of the played test.
    pub played: bool,
    /// The share of the other drums' strong hits where this mic is digital silence; `None` with fewer than
    /// `min_hits` of them.
    pub silent_share: Option<f64>,
    /// Edited to digital silence.
    pub silent: bool,
}

/// The result of [`tom_bleed`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TomBleed {
    /// `None` when the rule is on.
    pub off: Option<Off>,
    /// Per mic (input order).
    pub mics: Vec<MicStats>,
    /// Every transfer gain that was calibrated (mics of different drums).
    pub transfers: Vec<Transfer>,
    /// Per mic, per hit (input order): the excess over the played mics of the other drums.
    pub excess: Vec<Vec<f64>>,
    /// Per mic, per hit: marked as bleed (all false when the rule is off).
    pub bleed: Vec<Vec<bool>>,
    /// Shared pairs (input indices) counted as one drum: the weaker mic passed the level test (a hot preamp on a mic
    /// that may hear only the other drum), so neither mic explains the other.
    pub merged: Vec<(usize, usize)>,
    /// Version 2: each pair of a tom mic and a source, in tom then source order (empty without sources).
    pub sources: Vec<SourceUse>,
    /// Version 2: per mic, per hit, the lowest excess over the used snare and kick sources (`[snare, kick]`,
    /// [`NO_SOURCE_DB`] without one; empty without sources).
    pub source_excess: Vec<Vec<[f64; 2]>>,
    /// Version 5: per mic, per hit, marked by the guarded shared kick source (a kick source whose gain pair is
    /// shared, see the module documentation). Valid whatever `off` says, except for a mic that is digitally silent
    /// (all false there): when rule CD is off for fewer than two played tom close mics, the caller applies these
    /// marks itself; when rule CD is on they are already in `bleed`.
    pub shared_kick_bleed: Vec<Vec<bool>>,
    /// Per mic, per hit: the rules that mark it, by name (`CD`, `v2-snare`, `v2-kick`, `v5-kick`; empty when none).
    /// The reporting key of the eval (#96): with rule CD on, a hit is in `bleed` exactly when this is not empty; with
    /// the rule off for fewer than two played mics only `v5-kick` can be listed (the marks of `shared_kick_bleed`).
    pub by: Vec<Vec<Vec<&'static str>>>,
}

impl TomBleed {
    /// The number of marked hits.
    #[must_use]
    pub fn n_bleed(&self) -> usize {
        self.bleed.iter().flatten().filter(|&&b| b).count()
    }
}

/// The tom-band level of `x` in `[c + w.0, c + w.1)` with the filter run in over `pre` samples before the window;
/// samples outside the signal are zeros (the Python reference pads the same way).
fn level_db(x: &[f32], sec: &[Biquad], c: i64, w: (i64, i64), pre: i64) -> f64 {
    let start = c + w.0 - pre;
    let len = (pre + w.1 - w.0).max(0) as usize;
    if start >= 0 && start as usize + len <= x.len() {
        let s = start as usize;
        return band_peak_db(x, sec, s, s + pre as usize, s + len);
    }
    let seg: Vec<f32> = (0..len)
        .map(|i| {
            let k = start + i as i64;
            if k >= 0 && (k as usize) < x.len() {
                x[k as usize]
            } else {
                0.0
            }
        })
        .collect();
    band_peak_db(&seg, sec, 0, pre as usize, len)
}

/// The broadband peak of `x` in `[c + w.0, c + w.1)`, clipped to the signal (-180 dB when empty).
fn broad_db(x: &[f32], c: i64, w: (i64, i64)) -> f64 {
    let n = x.len() as i64;
    let a = (c + w.0).clamp(0, n);
    let b = (c + w.1).clamp(0, n);
    band_peak_db(x, &[], a as usize, a as usize, b as usize)
}

/// True when `sorted` has a value within `tol` of `v`.
fn near(sorted: &[i64], v: i64, tol: i64) -> bool {
    let k = sorted.partition_point(|&s| s < v - tol);
    k < sorted.len() && sorted[k] <= v + tol
}

/// The strong hits of a mic (indices into its hits): the P2-6 rule.
#[must_use]
pub fn strong_hits(x: &[f32], hits: &[i64], w: (i64, i64), strong_db: f64) -> Vec<usize> {
    let pk: Vec<f64> = hits.iter().map(|&c| broad_db(x, c, w)).collect();
    match percentile(&pk, 90.0) {
        Some(p90) => (0..hits.len()).filter(|&i| pk[i] >= p90 - strong_db).collect(),
        None => Vec::new(),
    }
}

/// Version 2: rule CD on the tom close mics ([`tom_bleed`]) with the kick and snare close mics `sources` (see the
/// module documentation). Without sources it is [`tom_bleed`].
pub fn tom_bleed_with(
    mics: &[TomMic],
    sources: &[SourceMic],
    sr: u32,
    p: &TomBleedParams,
) -> Result<TomBleed, AttributionError> {
    let mut r = tom_bleed(mics, sr, p)?;
    if sources.is_empty() {
        return Ok(r);
    }
    let sec = band_sections(Role::Tom.band(), sr);
    let win = (ms_to_samples(p.window_ms.0, sr), ms_to_samples(p.window_ms.1, sr));
    let sw = (
        ms_to_samples(p.strong_window_ms.0, sr),
        ms_to_samples(p.strong_window_ms.1, sr),
    );
    let pre = ms_to_samples(p.preroll_ms, sr);
    let quiet = ms_to_samples(p.quiet_ms, sr);
    let sorted = |h: &[i64]| {
        let mut v = h.to_vec();
        v.sort_unstable();
        v
    };
    let levels = |x: &[f32], at: &[i64]| -> Vec<f64> { at.iter().map(|&c| level_db(x, &sec, c, win, pre)).collect() };
    let src_strong: Vec<Vec<i64>> = sources
        .iter()
        .map(|s| {
            strong_hits(s.signal, &s.hits, sw, p.strong_db)
                .into_iter()
                .map(|j| s.hits[j])
                .collect()
        })
        .collect();
    let src_sorted: Vec<Vec<i64>> = sources.iter().map(|s| sorted(&s.hits)).collect();
    let mut uses = Vec::new();
    let mut excess = Vec::with_capacity(mics.len());
    let mut shared_kick = Vec::with_capacity(mics.len());
    let mut hps: Vec<Option<Vec<f64>>> = vec![None; mics.len()];
    for (a, tom) in mics.iter().enumerate() {
        let tom_sorted = sorted(&tom.hits);
        let tom_strong: Vec<i64> = strong_hits(tom.signal, &tom.hits, sw, p.strong_db)
            .into_iter()
            .map(|j| tom.hits[j])
            .collect();
        let own = levels(tom.signal, &tom.hits);
        let mut ex = vec![[NO_SOURCE_DB; 2]; tom.hits.len()];
        let mut shared = vec![false; tom.hits.len()];
        for (si, src) in sources.iter().enumerate() {
            // G(S->A) at the source's strong hits, A quiet where it lists no hit within `quiet_ms`
            let st = &src_strong[si];
            let la = levels(tom.signal, st);
            let ls = levels(src.signal, st);
            let all: Vec<f64> = la.iter().zip(&ls).map(|(a, s)| a - s).collect();
            let qv: Vec<f64> = st
                .iter()
                .zip(&all)
                .filter(|(c, _)| !near(&tom_sorted, **c, quiet))
                .map(|(_, &v)| v)
                .collect();
            let gain = calibrate_gain(&all, &qv, p.percentile, p.min_hits).map(|g| g.0);
            let silent_share = (la.len() >= p.min_hits.max(1))
                .then(|| la.iter().filter(|&&v| v <= p.silence_db).count() as f64 / la.len() as f64);
            // G(A->S) at the tom mic's strong hits
            let la2 = levels(tom.signal, &tom_strong);
            let ls2 = levels(src.signal, &tom_strong);
            let all2: Vec<f64> = ls2.iter().zip(&la2).map(|(s, a)| s - a).collect();
            let qv2: Vec<f64> = tom_strong
                .iter()
                .zip(&all2)
                .filter(|(c, _)| !near(&src_sorted[si], **c, quiet))
                .map(|(_, &v)| v)
                .collect();
            let back = calibrate_gain(&all2, &qv2, p.percentile, p.min_hits).map(|g| g.0);
            let unused = match (gain, back) {
                (None, _) => Some(Unused::NoGain),
                _ if silent_share.is_some_and(|s| s >= p.silent_share) => Some(Unused::Silent),
                (Some(g), Some(b)) if g + b > p.shared_sum_db => Some(Unused::Shared),
                _ => None,
            };
            if let (None, Some(g)) = (unused, gain) {
                let k = src.kind.index();
                for (j, v) in levels(src.signal, &tom.hits).into_iter().enumerate() {
                    ex[j][k] = ex[j][k].min(own[j] - v - g);
                }
            }
            // version 5: a kick source in a shared pair still marks, under the guarded rule (#362)
            if unused == Some(Unused::Shared) && src.kind == SourceKind::Kick {
                let Some(g) = gain else {
                    unreachable!("shared needs a gain")
                };
                let hp =
                    hps[a].get_or_insert_with(|| zero_phase(tom.signal, sr, &[(FilterKind::HighPass, PAIR_HP_HZ)]));
                for (j, v) in levels(src.signal, &tom.hits).into_iter().enumerate() {
                    if own[j] - v - g < p.kick_shared_threshold_db
                        && high_rise_db(hp, sr, tom.hits[j]) < p.kick_shared_rise_db
                    {
                        shared[j] = true;
                    }
                }
            }
            uses.push(SourceUse {
                tom: a,
                source: si,
                gain_db: gain,
                back_db: back,
                silent_share,
                unused,
            });
        }
        // a digitally silent mic has no levels worth anything: no marks there (the CD rule is off for the session)
        if r.mics[a].silent {
            shared.iter_mut().for_each(|m| *m = false);
        }
        if r.off.is_none() {
            for (j, (b, e)) in r.bleed[a].iter_mut().zip(&ex).enumerate() {
                let (snare, kick) = (e[0] < p.snare_threshold_db, e[1] < p.kick_threshold_db);
                *b |= snare || kick;
                if snare {
                    r.by[a][j].push("v2-snare");
                }
                if kick {
                    r.by[a][j].push("v2-kick");
                }
            }
            for (b, &m) in r.bleed[a].iter_mut().zip(&shared) {
                *b |= m;
            }
        }
        for (j, &m) in shared.iter().enumerate() {
            if m {
                r.by[a][j].push("v5-kick");
            }
        }
        excess.push(ex);
        shared_kick.push(shared);
    }
    r.shared_kick_bleed = shared_kick;
    r.sources = uses;
    r.source_excess = excess;
    Ok(r)
}

/// Run the rule on the tom close mics of one session (see the module documentation). All signals are at rate `sr`.
pub fn tom_bleed(mics: &[TomMic], sr: u32, p: &TomBleedParams) -> Result<TomBleed, AttributionError> {
    if sr == 0 {
        return Err(AttributionError::ZeroRate);
    }
    let n = mics.len();
    // one group per drum name
    let mut group: Vec<usize> = (0..n)
        .map(|m| (0..=m).find(|&o| mics[o].drum == mics[m].drum).unwrap_or(m))
        .collect();
    let first = run_pass(mics, sr, p, &group);
    let merged = merged_pairs(&first, &group, p);
    if merged.is_empty() {
        return Ok(first);
    }
    // fix round 3: a shared pair whose weaker mic passes the level test hears one drum: one group, then again
    for &(a, b) in &merged {
        let (from, to) = (group[b], group[a]);
        for g in group.iter_mut().filter(|g| **g == from) {
            *g = to;
        }
    }
    let mut second = run_pass(mics, sr, p, &group);
    second.merged = merged;
    Ok(second)
}

/// The shared pairs of a first pass (`a < b`, both gains present, their sum above `shared_sum_db`) whose weaker mic
/// (the lower level) is less than `keep_gap_db` under the other drums without the pair (one level per drum): those
/// count as one drum. A shared pair whose weaker mic is further under keeps the direction of the levels.
#[must_use]
pub fn merged_pairs(r: &TomBleed, group: &[usize], p: &TomBleedParams) -> Vec<(usize, usize)> {
    let gain = |s: usize, t: usize| {
        r.transfers
            .iter()
            .find(|g| g.source == s && g.target == t)
            .map(|g| g.gain_db)
    };
    let n = r.mics.len();
    let mut out = Vec::new();
    for a in 0..n {
        for b in a + 1..n {
            let (Some(ab), Some(ba)) = (gain(a, b), gain(b, a)) else {
                continue;
            };
            if ab + ba <= p.shared_sum_db {
                continue;
            }
            let (Some(la), Some(lb)) = (r.mics[a].level_db, r.mics[b].level_db) else {
                continue;
            };
            let lw = la.min(lb);
            // the level test of the weaker mic without its partner: a hot partner must not lift the median
            let others = group_levels(&r.mics, group, &[group[a], group[b]]);
            if percentile(&others, 50.0).is_none_or(|med| lw > med - p.keep_gap_db) {
                out.push((a, b));
            }
        }
    }
    out
}

/// The level of each drum group not in `skip`: the lowest level of its mics with strong hits.
fn group_levels(stats: &[MicStats], group: &[usize], skip: &[usize]) -> Vec<f64> {
    let mut by: Vec<(usize, f64)> = Vec::new();
    for (m, st) in stats.iter().enumerate() {
        let Some(l) = st.level_db else { continue };
        if skip.contains(&group[m]) {
            continue;
        }
        match by.iter_mut().find(|(g, _)| *g == group[m]) {
            Some(e) => e.1 = e.1.min(l),
            None => by.push((group[m], l)),
        }
    }
    by.into_iter().map(|(_, l)| l).collect()
}

/// One pass of the rule with the drum groups `group` (mics of one group never explain each other).
fn run_pass(mics: &[TomMic], sr: u32, p: &TomBleedParams, group: &[usize]) -> TomBleed {
    let n = mics.len();
    let sec = band_sections(Role::Tom.band(), sr);
    let win = (ms_to_samples(p.window_ms.0, sr), ms_to_samples(p.window_ms.1, sr));
    let sw = (
        ms_to_samples(p.strong_window_ms.0, sr),
        ms_to_samples(p.strong_window_ms.1, sr),
    );
    let pre = ms_to_samples(p.preroll_ms, sr);
    let quiet = ms_to_samples(p.quiet_ms, sr);
    let other = |a: usize, b: usize| a != b && group[a] != group[b];
    let sorted: Vec<Vec<i64>> = mics
        .iter()
        .map(|m| {
            let mut v = m.hits.clone();
            v.sort_unstable();
            v
        })
        .collect();
    let strong: Vec<Vec<usize>> = mics
        .iter()
        .map(|m| strong_hits(m.signal, &m.hits, sw, p.strong_db))
        .collect();
    // lv[m][o][j]: the level of mic o at hit j of mic m (own mic and the other drums' mics only)
    let lv: Vec<Vec<Option<Vec<f64>>>> = (0..n)
        .map(|m| {
            (0..n)
                .map(|o| {
                    (o == m || other(m, o)).then(|| {
                        mics[m]
                            .hits
                            .iter()
                            .map(|&c| level_db(mics[o].signal, &sec, c, win, pre))
                            .collect()
                    })
                })
                .collect()
        })
        .collect();
    let at = |m: usize, o: usize| lv[m][o].as_deref().expect("measured pair");
    let mut stats: Vec<MicStats> = (0..n)
        .map(|m| {
            let own: Vec<f64> = strong[m].iter().map(|&j| at(m, m)[j]).collect();
            let louder = strong[m]
                .iter()
                .filter(|&&j| {
                    (0..n).any(|o| other(m, o) && near(&sorted[o], mics[m].hits[j], quiet) && at(m, o)[j] > at(m, m)[j])
                })
                .count();
            MicStats {
                strong: own.len(),
                level_db: percentile(&own, 50.0),
                louder_share: (!own.is_empty()).then(|| louder as f64 / own.len() as f64),
                ..Default::default()
            }
        })
        .collect();
    for (a, st) in stats.iter_mut().enumerate() {
        let vals: Vec<f64> = (0..n)
            .filter(|&b| other(a, b))
            .flat_map(|b| strong[b].iter().map(move |&j| (b, j)))
            .map(|(b, j)| at(b, a)[j])
            .collect();
        if vals.len() >= p.min_hits.max(1) {
            let share = vals.iter().filter(|&&v| v <= p.silence_db).count() as f64 / vals.len() as f64;
            st.silent_share = Some(share);
            st.silent = share >= p.silent_share;
        }
    }
    for m in 0..n {
        let Some(own) = stats[m].level_db else { continue };
        // one level per other drum: its quietest mic (a merged pair counts once, not with the hot mic's level)
        let others = group_levels(&stats, group, &[group[m]]);
        stats[m].level_ok = match percentile(&others, 50.0) {
            Some(med) => own >= med - p.played_gap_db,
            None => true,
        };
        stats[m].played = stats[m].level_ok && stats[m].louder_share.is_some_and(|s| s < p.louder_share);
    }
    let mut gain: Vec<Vec<Option<f64>>> = vec![vec![None; n]; n];
    let mut transfers = Vec::new();
    for b in 0..n {
        for a in (0..n).filter(|&a| other(a, b)) {
            let mut all = Vec::with_capacity(strong[b].len());
            let mut qv = Vec::new();
            for &j in &strong[b] {
                let v = at(b, a)[j] - at(b, b)[j];
                all.push(v);
                if !near(&sorted[a], mics[b].hits[j], quiet) {
                    qv.push(v);
                }
            }
            if let Some((g, hits, quiet_only)) = calibrate_gain(&all, &qv, p.percentile, p.min_hits) {
                gain[b][a] = Some(g);
                transfers.push(Transfer {
                    source: b,
                    target: a,
                    gain_db: g,
                    hits,
                    quiet_only,
                });
            }
        }
    }
    let silent: Vec<usize> = (0..n).filter(|&m| stats[m].silent).collect();
    let played: Vec<usize> = (0..n).filter(|&m| stats[m].played).collect();
    let mut drums: Vec<usize> = played.iter().map(|&m| group[m]).collect();
    drums.sort_unstable();
    drums.dedup();
    let off = if !silent.is_empty() {
        Some(Off::DigitalSilence { mics: silent })
    } else if drums.len() < 2 {
        Some(Off::FewerThanTwoPlayed { played })
    } else {
        None
    };
    let excess: Vec<Vec<f64>> = (0..n)
        .map(|m| {
            (0..mics[m].hits.len())
                .map(|j| {
                    (0..n)
                        .filter(|&o| other(m, o) && stats[o].played)
                        .filter_map(|o| gain[o][m].map(|g| at(m, m)[j] - at(m, o)[j] - g))
                        .fold(NO_SOURCE_DB, f64::min)
                })
                .collect()
        })
        .collect();
    let by: Vec<Vec<Vec<&'static str>>> = excess
        .iter()
        .map(|ex| {
            ex.iter()
                .map(|&e| {
                    if off.is_none() && e < p.threshold_db {
                        vec!["CD"]
                    } else {
                        Vec::new()
                    }
                })
                .collect()
        })
        .collect();
    let bleed = by.iter().map(|ex| ex.iter().map(|b| !b.is_empty()).collect()).collect();
    TomBleed {
        off,
        mics: stats,
        transfers,
        excess,
        bleed,
        merged: Vec::new(),
        sources: Vec::new(),
        source_excess: Vec::new(),
        shared_kick_bleed: Vec::new(),
        by,
    }
}

/// One tom close mic for the pair rule (version 3).
#[derive(Debug, Clone)]
pub struct PairMic<'a> {
    /// The drum of the mic: two mics of one drum never pair.
    pub drum: &'a str,
    /// The mono signal at the session rate.
    pub signal: &'a [f32],
    /// The hit times (s), one entry per hit.
    pub times: &'a [f64],
    /// Per hit: it is in the output (not marked by version 2, not cut). Only these hits count and can pair.
    pub output: &'a [bool],
    /// Per hit: the rule may mark it (an edited hit stays).
    pub target: &'a [bool],
    /// Version 4: this mic's hits keep the guard (a marked hit with an own-mic attack rise of [`PAIR_KEEP_RISE_DB`]
    /// or more stays a hit). `false` runs the version 3 pair rule on the mic's hits (a recorded version 3 re-pick
    /// next to a version 4 target).
    pub guarded: bool,
}

/// The peak level (dBFS) of `x` in `[t - before, t + after]` s; -180 for an empty window (the Python reference
/// `firm_fixes.peak_db`).
fn peak_window_db<T: Copy + Into<f64>>(x: &[T], sr: u32, t: f64, before: f64, after: f64) -> f64 {
    let sr = f64::from(sr);
    let i0 = (((t - before) * sr) as i64).max(0) as usize;
    let i1 = ((((t + after) * sr) as i64).max(0) as usize).min(x.len());
    if i1 <= i0 {
        return -180.0;
    }
    let peak = x[i0..i1].iter().fold(0.0f64, |m, &v| m.max(v.into().abs()));
    20.0 * peak.max(1e-9).log10()
}

/// The level (dBFS) of a hit for the pair rule: the peak of `x` in `[t - 5 ms, t + 30 ms]` (`firm_fixes.peak_db`);
/// -180 for an empty window.
#[must_use]
pub fn pair_level_db(x: &[f32], sr: u32, t: f64) -> f64 {
    peak_window_db(x, sr, t, 0.005, 0.030)
}

/// The own-mic attack rise of a hit (dB): the peak in `[t - 2, t + 10]` ms over the peak in `[t - 30, t - 10]` ms,
/// the larger of the full band `x` and the 2 kHz high-pass `hp` ([`PAIR_HP_HZ`], [`zero_phase`]). The P4-18 rise test
/// (`tools/tom_fixes.py` `stroke_rise`, `tools/firm_fixes.py` `rise_db`): a stroke is 10 dB or more over what came
/// before; the decay of an earlier hit or noise is about 0. In a roll the ring of the earlier hit hides the
/// full-band rise, the stick click over the faster decaying highs does not.
#[must_use]
pub fn stroke_rise_db(x: &[f32], hp: &[f64], sr: u32, t: f64) -> f64 {
    // the near window is [t - 2, t + 10] ms, the far window [t - 30, t - 10] ms (before the hit)
    let near = (0.002, 0.010);
    let far = (0.030, -0.010);
    let full = peak_window_db(x, sr, t, near.0, near.1) - peak_window_db(x, sr, t, far.0, far.1);
    let high = peak_window_db(hp, sr, t, near.0, near.1) - peak_window_db(hp, sr, t, far.0, far.1);
    full.max(high)
}

/// The high-band part of [`stroke_rise_db`] on its own (dB): the [`PAIR_HP_HZ`] high-pass peak in `[t - 2, t + 10]` ms
/// over `[t - 30, t - 10]` ms. Version 5's guard reads this, not the full-band rise: a kick's own transient is an
/// attack on the tom mic too, only the stick click of a real stroke lives in the high band alone (#362).
#[must_use]
pub fn high_rise_db(hp: &[f64], sr: u32, at: i64) -> f64 {
    let t = at as f64 / f64::from(sr);
    peak_window_db(hp, sr, t, 0.002, 0.010) - peak_window_db(hp, sr, t, 0.030, -0.010)
}

/// The median of `v` (the mean of the two middle values for an even count); `None` for an empty slice.
fn median(v: &mut [f64]) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    let n = v.len();
    Some(if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    })
}

/// Version 3, the tom pair rule: per mic the hits it marks (see the module documentation). `mics` must have equal-length
/// `times`, `output` and `target` per mic. Version 4: a guarded mic's hits keep the own-mic attack rise guard.
#[must_use]
pub fn tom_pairs(mics: &[PairMic], sr: u32, p: &TomBleedParams) -> Vec<Vec<bool>> {
    let levels: Vec<Vec<f64>> = mics
        .iter()
        .map(|m| m.times.iter().map(|&t| pair_level_db(m.signal, sr, t)).collect())
        .collect();
    let rises: Vec<Option<Vec<f64>>> = mics
        .iter()
        .map(|m| {
            if !m.guarded {
                return None;
            }
            let hp = zero_phase(m.signal, sr, &[(FilterKind::HighPass, PAIR_HP_HZ)]);
            Some(m.times.iter().map(|&t| stroke_rise_db(m.signal, &hp, sr, t)).collect())
        })
        .collect();
    pair_marks(mics, &levels, &rises, p)
}

/// [`tom_pairs`] with the levels given (dB per hit) and per mic the own-mic attack rises (`None`: the mic's hits
/// keep no guard, the version 3 pair rule): the part that decides, tested on its own.
#[must_use]
pub fn pair_marks(
    mics: &[PairMic],
    levels: &[Vec<f64>],
    rises: &[Option<Vec<f64>>],
    p: &TomBleedParams,
) -> Vec<Vec<bool>> {
    let window = p.winner_window_ms / 1000.0 + 1e-9;
    // (relative level, time, mic, hit, may be marked)
    let mut ev: Vec<(f64, f64, usize, usize, bool)> = Vec::new();
    for (a, m) in mics.iter().enumerate() {
        let mut out: Vec<f64> = (0..m.times.len())
            .filter(|&j| m.output[j])
            .map(|j| levels[a][j])
            .collect();
        let Some(med) = median(&mut out) else { continue };
        for j in (0..m.times.len()).filter(|&j| m.output[j]) {
            ev.push((levels[a][j] - med, m.times[j], a, j, m.target[j]));
        }
    }
    ev.sort_by(|x, y| y.0.total_cmp(&x.0));
    let mut kept: Vec<(f64, f64, usize)> = Vec::new();
    let mut marks: Vec<Vec<bool>> = mics.iter().map(|m| vec![false; m.times.len()]).collect();
    for (rel, t, a, j, target) in ev {
        let beaten = kept.iter().any(|&(kr, kt, ka)| {
            mics[ka].drum != mics[a].drum && (kt - t).abs() <= window && kr - rel >= p.winner_margin_db
        });
        // version 4: a hit with an own-mic attack stays (bleed has no stick attack on its own mic)
        let keep = rises[a].as_ref().is_some_and(|r| r[j] >= p.pair_keep_rise_db);
        if target && rel <= p.winner_floor_db && !keep && beaten {
            marks[a][j] = true;
        } else {
            kept.push((rel, t, a));
        }
    }
    marks
}

#[cfg(test)]
mod tests;
