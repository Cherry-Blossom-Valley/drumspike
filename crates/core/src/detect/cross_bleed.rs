//! #281 A6: the cross-track bleed rule for sparse tracks (kick and snare bleed on a tom close mic).
//!
//! On ENST-Drums the tom mics output three to eight times more hits than there are strokes: kick and snare
//! bleed (model-vs-dsp report, part B). A rarely played tom has bleed as its loudest
//! events, so the loud-hit template of the own detector learns the bleed and the app's tom bleed rule
//! ([`super::tom_bleed`], transfer gains on tom-band levels) does not catch this material. This rule uses
//! two measurements the transfer-gain rule does not have: the **time of arrival** and the **level across
//! mics at the same instant**.
//!
//! # Input and output
//!
//! [`cross_bleed`] takes every close mic of the session ([`CrossMic`]: the kind, the mono signal at the
//! session rate, and per confirmed hit the **first-motion time** in seconds — the refine back-track of
//! P4-12 stage 2 finds it sample-exactly on every track, so the event time carries it for free). It gives
//! per tom mic, per hit, the [`CrossMark`]: whether the hit is bleed, and the evidence (the lead over and
//! the level ratio against the loudest coinciding hit of another close mic). The tom mics are the
//! targets; every other close mic of the session (kick, snare, hi-hat, and the other tom mics) is a source: a
//! hit is judged only on the mic it does not belong to.
//!
//! # The rule (the owner's research note §4.3, in the measured form of #285)
//!
//! A real hit is loudest **and earliest** on its own mic; bleed arrives later (0.5 to 3 ms for 20 cm to
//! 1 m of mic distance) and weaker. A hit of a tom mic X that coincides within `coincide_ms` with a
//! confirmed hit of a source mic S is **bleed** when
//!
//! 1. X's first motion starts later than S's by more than the pair's lead bound (`min_lead_ms`, or a
//!    quarter of the median lead of the pair's clear cases, whichever is larger — a real hit that leads
//!    S stays even when quieter), and
//! 2. X's band level at the hit lies below S's level at S's hit (each mic in its own [`Role::band`]):
//!    the bleed is quieter than the source hit in the source's own band.
//!
//! The learned median ratio of the pair's clear cases is **not** part of the verdict (it stays in
//! [`PairLearned`] and per [`CrossMark`] as stored evidence): on ENST-Drums the detector fires only on
//! the loud tail of the bleed responses, which the clear cases sample as well, so the detected bleed's
//! ratio spans the whole clear-case range and no median-based margin separates it from a real hit
//! (#285, the measured clear-case and at-hit ratio distributions). The lead and the source-louder test
//! separate them: a real hit is earliest and loudest on its own mic.
//!
//! A pair's clear cases are S's hits where X lists no own hit within `coincide_ms` and X's response is
//! not digital silence: X's first motion of the response (the bleed arrival on X, [`first_motion`]) and
//! the band level ratio `L_X - L_S` there. The medians of these samples are the pair's typical bleed
//! lead and bleed ratio, learned from the session itself; a pair with fewer than `min_clear` clear
//! cases falls back to the fixed `min_lead_ms` lead.
//!
//! The level window at a hit is the tom-band window of the tom bleed rule (`window_ms`, the filter run
//! in over `preroll_ms`), per mic in its own [`Role::band`].
//!
//! # The aligned session (#296)
//!
//! A session whose tracks were time-aligned (Auto-Align 2 or the like) has no bleed arrival delay:
//! the lead of a source's hits on X sits at 0, so the lead test cannot fire and the bleed stays in
//! the output. Per pair, the rule measures the |lead| of X's hits that coincide with a hit of S
//! (nearest hit within `coincide_ms`): with at least `align_min_coins` coincidences and a median
//! |lead| under `align_med_abs_lead_ms` the pair is **aligned** ([`PairLearned::aligned`]) and the
//! verdict for the pair is the level test alone (test 2, no lead condition). A pair with a real
//! lead keeps the two-part verdict, so unaligned sessions are unchanged. The per-pair result
//! falls under each of [`ALIGN_COUNT_MARGINS_DB`] — the ticket's margin sweep) is in
//! [`PairLearned`]; the app reports it per pair (`cross_bleed.pairs` in the job record and the
//! detector record). The alignment is a property of the PAIR, not of one hit list: the app's
//! re-detection pass hands the pass-1 pairs to [`cross_bleed_prior`], so the verdict does not
//! re-measure the alignment on the re-detected candidate times (whose |lead| medians move).
//!
//! # The aligned session, inherited alignment (#309, the session test of #313)
//!
//! A source that rarely coincides with X — the kick on a tom that plays little over the kick, the
//! other tom mics — gives a pair too few coincidences to measure its own alignment
//! (`align_min_coins` not reached), and the few coins it has are the bleed events themselves,
//! whose residual lead (2 to 5 ms on the measured session) looks like a real lead. Alignment is a
//! property of the SESSION: the tool is applied to every track, so when a pair of the target is
//! measured aligned, EVERY pair of that target judges by the level test alone
//! ([`PairLearned::align_inherited`] records the inheritance; the measured `align_coins` and
//! `align_med_abs_lead_ms` stay as measured) — sparse or not. The lead of the other pairs
//! carries no information on such a session: on the measured session the kick bleed on a tom mic
//! rises slowly, so the detector places the verdict time 2 to 5 ms AFTER the kick hit even on an
//! aligned session, and that placement lag is not acoustics. A session without a measured-aligned
//! pair stays on the two-part verdict, so unaligned sessions are unchanged. The inheritance
//! travels through [`cross_bleed_prior`] (the pass-1 `align_inherited` is carried with the
//! alignment).
//!
//! # The per-source session test (#313)
//!
//! The #309 inheritance fired from ANY measured-aligned pair of the target, and on a close-miked
//! unaligned session exactly ONE pair of ONE tom reads aligned (the hi-hat or the snare on the
//! nearest tom mic, median |lead| 0.05 to 0.39 ms over 26 to 93 coincidences) while the tom's
//! other pairs — whose own medians read 0.8 to 7.7 ms — inherited it and judged by the level
//! test alone: the ENST part-B run changed on 53 of 210 items against the build before #296. A
//! time-alignment tool shifts whole tracks, so a source's bleed lag reads near zero on EVERY tom
//! target it coincides with; a small lag on one target alone is a close mic. A source is aligned
//! on the session only when it measures aligned on every tom target that has at least
//! `align_min_coins` coincidences with it, and on at least two such targets
//! ([`PairLearned::source_aligned_targets`] of `source_targets`); a session with one tom mic
//! cannot establish alignment. The target-level inheritance fires only from a session-aligned
//! source's measured-aligned pair (on the owner's aligned sessions the snare measures aligned on
//! every tom, so every pair of every tom judges by the level test alone, as before). A pair that
//! measured aligned on its own but whose source fails the session test keeps the two-part
//! verdict. The session test is decided ONCE per session (the app's pre-pass over all tom
//! targets, [`measure_pair_alignment`], hands the verdict to every per-target run as a
//! [`SessionAlign`]): the app calls the rule ONE tom at a time, so a count over a per-target
//! call's tom mics can never reach the two-target bar.
//!
//! # The pass-1 inherit rule and the session test (#313)
//!
//! The pass-1 inherit rule of #296 (a re-detected candidate placed on a pass-1 bleed-marked time
//! inherits the mark) fired on EVERY pass-1 mark — including the two-part verdict's — and so
//! altered unaligned items against the build before #296 (38 tom-mic outputs lost 1 to 4 hits,
//! one item flipped 145 marked events into kept hits). Only a mark made by an ALIGNED pair (the
//! level test, [`CrossMark::aligned_mark`]) feeds the inherit rule: on a session with no aligned
//! pair the re-detection judges every candidate on its own, exactly as before #296.
//!
//! # The aligned session, the pair's own bleed ceiling (#309)
//!
//! The level test alone misjudges on the kick pairs: it compares X's tom-band level with S's
//! level in S's own band, and the kick-in mic is loud in its band even on a small kick — a
//! medium tom stroke with a kick under it reads -3 to -5 dB and is called bleed, while the
//! pair's real bleed reads -15 to -53 dB. The principle: **bleed of S on X is never louder than
//! the bleed this pair produces.** The pair's clear cases (S hits without an X hit) sample that
//! bleed; their ratio distribution is recorded per pair (`ratio_med_db` = [`PairLearned::ratio_db`],
//! [`PairLearned::ratio_p95_db`], [`PairLearned::ratio_max_db`]), and on an ALIGNED pair a
//! coinciding hit is bleed only when its ratio is under `margin_db` AND not above the pair's
//! clear-case maximum. Unaligned pairs keep the two-part verdict (the lead still separates
//! there), and a pair with fewer than `min_clear` clear cases keeps the level test alone.
//!
//! # Template validity
//!
//! The rule does not fix the detector's template on its own: a tom track whose loudest candidates are
//! bleed learns the bleed as its loud-hit template. The caller (the app's session step) excludes the
//! candidates that coincide with a louder hit on another track from the template and re-gates the track
//! ([`super::onset::retemplate_candidates`]); the rule gives it the evidence per candidate.
//!
//! # Determinism
//!
//! Every value is computed in a fixed order in `f64`; window bounds use [`ms_to_samples`].

use crate::dsp::{band_level_db, band_sections, median, ms_to_samples};
use crate::refine::first_motion;
use crate::role::Role;
use crate::rules::ResearchOverrides;

/// The kind of a close mic of the rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XKind {
    /// A tom close mic (a target).
    Tom,
    /// A kick close mic (a source).
    Kick,
    /// A snare close mic (a source).
    Snare,
    /// A hi-hat close mic (a source; #285: the ENST toms sit next to the hi-hat).
    Hat,
    /// An overhead or cymbal mic (a source; #286 A1: hat and cymbal bleed on the snare close
    /// mic is louder on the oh than on the snare mic, a ghost note is not).
    Overhead,
}

impl XKind {
    /// The band the level is measured in ([`Role::band`]).
    fn role(self) -> Role {
        match self {
            XKind::Tom => Role::Tom,
            XKind::Kick => Role::Kick,
            XKind::Snare => Role::Snare,
            XKind::Hat => Role::Hat,
            // the oh's own band is not used for the verdict: the #286 A1 measurement shows
            // the oh test separates only when both mics are measured in one COMMON band —
            // the full-band peak. `cross_bleed` measures an Overhead source full-band;
            // this band stays for the pair's own evidence.
            XKind::Overhead => Role::Cymbal,
        }
    }

    /// An Overhead source is measured in the common band (the #286 A1 measurement, see
    /// [`XKind::Overhead`]): the FULL-BAND peak on both mics, the same measure on each.
    /// The overhead is a mix of everything; only a common band compares like with like.
    /// This deviates from A6's own-band rule (each mic in its own band) — measured: the
    /// own-band form does not separate (no margin cuts more than 16 of 142 false hits);
    /// the full-band form cuts 123/142 and denies 0/109 ghosts. Every other kind in its
    /// own band.
    fn in_target_band(self) -> bool {
        self == XKind::Overhead
    }
}

/// One close mic of the session.
#[derive(Debug, Clone)]
pub struct CrossMic<'a> {
    /// Tom (target), kick or snare (source).
    pub kind: XKind,
    /// Mono signal at the session rate.
    pub signal: &'a [f32],
    /// Per confirmed hit, the first-motion time in seconds (the refine back-track of the event), in any
    /// order.
    pub motion: Vec<f64>,
}

/// Parameters ([`Default`] gives the measured values; see the module documentation).
#[derive(Debug, Clone, PartialEq)]
pub struct CrossBleedParams {
    /// A hit of X and a hit of S this close (ms) coincide.
    pub coincide_ms: f64,
    /// A hit of X whose motion starts more than this later than the coinciding hit of S is bleed (the
    /// fixed lead bound; a learned pair raises it to the median lead of its clear cases).
    pub min_lead_ms: f64,
    /// The level window at a hit (ms), as the tom bleed rule's.
    pub window_ms: (f64, f64),
    /// Filter run-in before a level window (ms).
    pub preroll_ms: f64,
    /// Clear cases a pair needs before its medians are learned.
    pub min_clear: usize,
    /// A clear-case lead above this (ms) is not a bleed arrival (a real hit of X the coincidence test
    /// missed): the sample is left out of the pair's lead median. The ratio sample stays.
    pub clear_lead_cap_ms: f64,
    /// A response of X under this level (dBFS) at a clear case is digital silence: no sample at all.
    pub silence_db: f64,
    /// The level test fires when the ratio falls below this (dB, `L_X - L_S`): 0.0 is the measured
    /// form (the source strictly louder), a positive value is the #285 gated variant's margin (the
    /// env `DRUMSPIKE_A6_MARGIN_DB` overrides the default).
    pub margin_db: f64,
    /// The overhead source's level test (#286 A1): a hit is bleed when the oh is louder than the
    /// target by more than this (dB, both mics in the target's band). The env
    /// `DRUMSPIKE_A1_OH_MARGIN_DB` overrides the default (2.0: the a1-wake286 measurement cuts
    /// 123 of 142 model-absent false snare hits and denies none of the 109 ghost notes).
    pub overhead_margin_db: f64,
    /// #296: a pair is aligned (Auto-Align 2 and the like) when at least this many hits of X
    /// coincide with hits of S.
    pub align_min_coins: usize,
    /// #296: and the median |lead| of the coinciding hits is under this (ms). Acoustic bleed
    /// arrives 0.5 to 3 ms after the source; the ticket's 0.15 ms example left the measured
    /// session's toms 3 and 4 (signed-lead medians +0.33/+0.23 ms) undetected, so the default
    /// keeps a 0.1 ms gap to the 0.5 ms bleed floor.
    pub align_med_abs_lead_ms: f64,
}

impl Default for CrossBleedParams {
    /// [`CrossBleedParams::with_overrides`] with the research overrides of the process
    /// environment (R6 Parallel Change: stays while callers are left).
    fn default() -> Self {
        CrossBleedParams::with_overrides(&ResearchOverrides::from_env())
    }
}

impl CrossBleedParams {
    /// The measured values; `research` overrides the A6 lead and margin, the A1 overhead margin
    /// and the #296 alignment bounds (`None` keeps the measured value).
    #[must_use]
    pub fn with_overrides(research: &ResearchOverrides) -> Self {
        CrossBleedParams {
            coincide_ms: 10.0,
            min_lead_ms: research.a6_min_lead.unwrap_or(0.3),
            window_ms: (-0.5, 8.0),
            preroll_ms: 50.0,
            min_clear: 5,
            clear_lead_cap_ms: 3.0,
            silence_db: -100.0,
            margin_db: research.a6_margin_db.unwrap_or(0.0),
            overhead_margin_db: research.a1_oh_margin_db.unwrap_or(2.0),
            align_min_coins: research.a6_align_min_coins.unwrap_or(20.0) as usize,
            align_med_abs_lead_ms: research.a6_align_med_abs_lead.unwrap_or(0.4),
        }
    }
}

/// The verdict of the rule for one hit of a tom mic.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CrossMark {
    /// The hit is bleed and is dropped.
    pub bleed: bool,
    /// The lead over the loudest coinciding hit of a kick or snare mic (ms; `motion_X - motion_S`).
    /// `None` without a coinciding hit.
    pub lead_ms: Option<f64>,
    /// The level ratio against it (dB, `L_X - L_S`). `None` without a coinciding hit.
    pub ratio_db: Option<f64>,
    /// #103 step 2 calibration: the lead against the coinciding HAT source hit specifically
    /// (the pair the hat verdict judges), `None` without a coinciding hat hit. The stored
    /// `lead_ms`/`ratio_db` are the loudest source's, which for a hat-cause FP is often the
    /// kick or the snare — the hat verdict's own pair was never recorded, so the -32/-10
    /// pair calibrated on the stored keys fired on the wrong distribution (433 drops where
    /// the sweep on the stored keys predicted 857).
    pub hat_lead_ms: Option<f64>,
    /// The level ratio against the coinciding hat hit (dB, `L_X - L_hat`). See
    /// [`CrossMark::hat_lead_ms`].
    pub hat_ratio_db: Option<f64>,
    /// #313: the hit was marked bleed by an ALIGNED pair (the level test alone). `false` when
    /// the marking pair kept the two-part verdict or the hit is not bleed. The app hands only
    /// these marks to the pass-1 inherit rule: on a session with no aligned pair the
    /// re-detection keeps the pre-#296 behaviour (a two-part mark does not survive the
    /// re-detection there — the pass-2 verdict judges the candidate on its own, as before #296).
    pub aligned_mark: bool,
}

/// The level margins (dB) of the aligned-pair count sweep (#296): per aligned pair,
/// [`PairLearned::aligned_counts`] holds the number of X's hits whose ratio falls under each.
pub const ALIGN_COUNT_MARGINS_DB: [f64; 4] = [-3.0, 0.0, 3.0, 6.0];

/// The learned bounds of one pair (a tom mic index, a source index): the median lead and ratio of the
/// clear cases (`None` without at least `min_clear` of them).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PairLearned {
    /// The tom mic (input index).
    pub tom: usize,
    /// The source (input index).
    pub source: usize,
    /// The median bleed lead of the clear cases (ms).
    pub lead_ms: Option<f64>,
    /// The MEDIAN bleed level ratio of the clear cases (dB) — `ratio_med_db` of the #309 record.
    pub ratio_db: Option<f64>,
    /// #309: the 95th percentile of the clear cases' bleed level ratios (dB).
    pub ratio_p95_db: Option<f64>,
    /// #309: the MAXIMUM of the clear cases' bleed level ratios (dB) — the loudest bleed the
    /// pair itself produces. On an ALIGNED pair a coinciding hit above this is never bleed:
    /// bleed of S on X is never louder than the bleed this pair produces.
    pub ratio_max_db: Option<f64>,
    /// #296: the pair's coinciding hits are centred on 0 — an aligned session (Auto-Align 2 and
    /// the like removed the bleed's time lead): the verdict for the pair is the level test alone.
    pub aligned: bool,
    /// #296: the number of coinciding hits the alignment was measured over.
    pub align_coins: usize,
    /// #296: the median |lead| of the coinciding hits (ms); `None` without coincidences.
    pub align_med_abs_lead_ms: Option<f64>,
    /// #296: per aligned pair, the number of X's hits with a coinciding hit of S whose ratio
    /// falls under each of [`ALIGN_COUNT_MARGINS_DB`] (the ticket's margin sweep). All zero when
    /// the pair is not aligned.
    pub aligned_counts: [usize; 4],
    /// #309: the alignment was inherited from the session (any other pair of the target is
    /// measured aligned — the tool is applied to every track) rather than the pair's own
    /// measurement. The pair's own `align_coins` and `align_med_abs_lead_ms` stay as measured.
    /// `false` when the alignment is the pair's own measurement or the pair is not aligned.
    pub align_inherited: bool,
    /// #313: the source's per-session test — the number of tom targets on which the pair
    /// measured aligned (`source_aligned_targets`) out of the tom targets with at least
    /// `align_min_coins` coincidences (`source_targets`). A source is aligned on the session
    /// only when it measures aligned on EVERY target with enough coins, and on at least two
    /// such targets; a session with one tom mic cannot establish alignment.
    pub source_aligned_targets: usize,
    /// #313: the number of tom targets with at least `align_min_coins` coincidences with the
    /// source (see [`PairLearned::source_aligned_targets`]).
    pub source_targets: usize,
}

/// The level of every motion time of `mic` in its own band.
fn levels(sig: &[f32], kind: XKind, times: &[f64], sr: u32, w: (i64, i64), pre: i64) -> Vec<f64> {
    let sec = band_sections(kind.role().band(), sr);
    times
        .iter()
        .map(|&t| band_level_db(sig, &sec, (t * f64::from(sr)).round() as i64, w, pre))
        .collect()
}

/// #355: the canonical position of every hit of `mic` — the hit's refined position walked back
/// to the first motion of the mic's OWN signal, the same bare [`first_motion`] walk the app
/// gives every source motion (no bounds). The rule must not read a hit's position from the
/// refined list alone: the refined position of a hit can move by a fraction of a millisecond
/// between placement versions (version 8 to version 9 moved a snare hit 0.19 ms earlier and a
/// cross-track verdict then flipped, so a real tom hit vanished), while the hit's own attack
/// in the signal — its anchor — stays. The walk-back from any refined position near the anchor
/// crosses the same gap and lands on the same foot, so the derived time — and with it the
/// verdict — is the same for all of them. A refined position that IS the signal's first motion
/// is returned unchanged (the walk is idempotent there).
fn canon_motion(mic: &CrossMic, sr: u32) -> Vec<f64> {
    mic.motion
        .iter()
        .map(|&t| {
            let s = (t * f64::from(sr)).round() as i64;
            let (foot, _) = first_motion(mic.signal, s, sr, t, None, None);
            foot as f64 / f64::from(sr)
        })
        .collect()
}

/// [`canon_motion`] over every mic of the session.
fn canon_all(mics: &[CrossMic], sr: u32) -> Vec<Vec<f64>> {
    mics.iter().map(|m| canon_motion(m, sr)).collect()
}

/// The band level (dBFS) of a hit of `kind` at time `t` (s) in `x` at rate `sr`, with the rule's
/// level window (for the caller's coincidence tests, e.g. the template exclusion).
#[must_use]
pub fn hit_level(x: &[f32], sr: u32, kind: XKind, t: f64, p: &CrossBleedParams) -> f64 {
    let sec = band_sections(kind.role().band(), sr);
    let w = (ms_to_samples(p.window_ms.0, sr), ms_to_samples(p.window_ms.1, sr));
    let pre = ms_to_samples(p.preroll_ms, sr);
    band_level_db(x, &sec, (t * f64::from(sr)).round() as i64, w, pre)
}

/// The FULL-BAND peak level (dBFS) of a hit at time `t` (s) in `x` at rate `sr`, with the rule's
/// level window — the #286 A1 common-band measure: the overhead is a mix of everything; only
/// a common band compares like with like (no filter sections, the same measure on both mics).
#[must_use]
pub(crate) fn hit_level_full_band(x: &[f32], sr: u32, t: f64, p: &CrossBleedParams) -> f64 {
    let w = (ms_to_samples(p.window_ms.0, sr), ms_to_samples(p.window_ms.1, sr));
    let pre = ms_to_samples(p.preroll_ms, sr);
    band_level_db(x, &[], (t * f64::from(sr)).round() as i64, w, pre)
}

/// The inherit band of the re-detection: a pass-2 candidate within this of a pass-1
/// bleed-marked hit inherits the mark (#296).
pub(crate) const PASS1_INHERIT_MS: f64 = 1.0;

/// Whether `t` sits within [`PASS1_INHERIT_MS`] of a hit that pass 1 marked as bleed (#296).
/// Applied by the caller to the FINAL placement of a pass-2 candidate whose own verdict found
/// no coinciding source (`lead_ms` None): pass 1 removed the bleed hits from the track, so pass
/// 2 re-detects them and — with no coinciding source at its walked time — would keep them; the
/// pass-1 verdict closes that loop without touching any candidate time. A candidate that pass 2
/// judged WITH a coinciding source keeps pass 2's verdict.
#[must_use]
pub fn inherits_pass1_bleed(t: f64, pass1_bleed: &[f64]) -> bool {
    let band = PASS1_INHERIT_MS / 1000.0 + 1e-9;
    pass1_bleed.iter().any(|&b| (b - t).abs() <= band)
}

/// Run the rule over the close mics of one session (see the module documentation). All signals are at
/// rate `sr`. Returns per tom mic (input order), per hit (input order) the [`CrossMark`], and the
/// learned bounds per pair (tom order, then source order).
#[must_use]
pub fn cross_bleed(mics: &[CrossMic], sr: u32, p: &CrossBleedParams) -> (Vec<Vec<CrossMark>>, Vec<PairLearned>) {
    cross_bleed_with(mics, &CrossBleedContext::new(sr, p))
}

/// [`cross_bleed`] with the per-source session verdict handed in (#313): `session` holds, per
/// mic index, the session-wide per-source counts (see [`SessionAlign`]) — the app's per-target
/// runs each see ONE tom, so the count over the slice's tom mics cannot establish a session
/// verdict; the caller decides it once per session over all tom targets and hands it in. When
/// `None`, the in-call count over the slice's tom mics decides (the core's own full-session
/// runs).
#[must_use]
pub fn cross_bleed_session(
    mics: &[CrossMic],
    sr: u32,
    p: &CrossBleedParams,
    session: Option<&SessionAlign>,
) -> (Vec<Vec<CrossMark>>, Vec<PairLearned>) {
    cross_bleed_with(
        mics,
        &CrossBleedContext {
            session,
            ..CrossBleedContext::new(sr, p)
        },
    )
}

/// The per-pair alignment measurement of #296: the |lead| of X's hits that coincide with a hit of
/// S, nearest hit first. An aligned session's leads sit at 0 (the tool removed the bleed's arrival
/// delay); a bleed lead of 0.5 to 3 ms cannot. The hits' positions are the canonical ones
/// ([`canon_motion`], #355): the measurement must not flip on a sub-millisecond move of a
/// refined position either.
fn measure_pair(x_t: &[f64], s_t: &[f64], coincide: f64, p: &CrossBleedParams) -> PairAlignment {
    let mut s_sorted: Vec<f64> = s_t.to_vec();
    s_sorted.sort_by(f64::total_cmp);
    let mut coins: Vec<f64> = Vec::new();
    for &tx in x_t {
        let k = s_sorted.partition_point(|&ts| ts < tx);
        let cands = [s_sorted.get(k.saturating_sub(1)).copied(), s_sorted.get(k).copied()];
        if let Some(ts) = cands
            .into_iter()
            .flatten()
            .filter(|&ts| (ts - tx).abs() <= coincide + 1e-9)
            .min_by(|u, v| (u - tx).abs().total_cmp(&(v - tx).abs()))
        {
            coins.push((tx - ts).abs() * 1000.0);
        }
    }
    let med_abs_lead = median(&mut coins);
    let aligned = coins.len() >= p.align_min_coins && med_abs_lead.is_some_and(|m| m < p.align_med_abs_lead_ms);
    PairAlignment {
        aligned,
        coins: coins.len(),
        med_abs_lead_ms: med_abs_lead,
    }
}

/// The alignment measurement of one (tom, source) pair ([`measure_pair`], #296).
#[derive(Debug, Clone, Copy, PartialEq)]
struct PairAlignment {
    /// The pair's coinciding hits are centred on 0 (enough coins, median |lead| under the bound).
    aligned: bool,
    /// The number of X's hits that coincide with a hit of S.
    coins: usize,
    /// The median |lead| of the coinciding hits (ms); `None` without coincidences.
    med_abs_lead_ms: Option<f64>,
}

/// One measured pair alignment: (tom mic index, source mic index, aligned, coins, median |lead| ms).
pub(crate) type MeasuredAlign = (usize, usize, bool, usize, Option<f64>);

/// One measured row of a tom: the source mic index and the pair's alignment. A tom's rows are in
/// source order.
#[derive(Debug, Clone, Copy, PartialEq)]
struct MeasuredRow {
    /// The source (input index).
    source: usize,
    /// The pair's own alignment measurement.
    align: PairAlignment,
}

/// Measure every (tom, source) pair's alignment over the full mic set — the pre-pass the app
/// runs ONCE per session (#313): the per-source session test needs every pair's alignment
/// measured before any verdict, and the app's per-target runs each see ONE tom. The tom mics
/// are the targets; every non-Overhead mic is a source (the target itself is skipped). The
/// per-pair rule is [`measure_pair`] (#296) and the thresholds are the params'.
#[must_use]
pub fn measure_pair_alignment(mics: &[CrossMic], _sr: u32, p: &CrossBleedParams) -> Vec<MeasuredAlign> {
    let coincide = p.coincide_ms / 1000.0;
    let canon = canon_all(mics, _sr);
    measure_pairs(mics, &canon, coincide, p)
}

/// [`measure_pair_alignment`] over canon positions computed by the caller (the rule's own run
/// has them already).
fn measure_pairs(mics: &[CrossMic], canon: &[Vec<f64>], coincide: f64, p: &CrossBleedParams) -> Vec<MeasuredAlign> {
    let toms: Vec<usize> = mics
        .iter()
        .enumerate()
        .filter(|(_, m)| m.kind == XKind::Tom)
        .map(|(i, _)| i)
        .collect();
    let srcs: Vec<usize> = (0..mics.len()).filter(|&b| mics[b].kind != XKind::Overhead).collect();
    let mut out = Vec::new();
    for &a in &toms {
        for &b in &srcs {
            if b == a {
                continue;
            }
            let m = measure_pair(&canon[a], &canon[b], coincide, p);
            out.push((a, b, m.aligned, m.coins, m.med_abs_lead_ms));
        }
    }
    out
}

/// The per-source session verdict, decided ONCE per session over all tom targets (#313) and
/// handed in by the app (whose per-target runs each see ONE tom): per mic index of the
/// [`CrossMic`] slice, the number of tom targets with at least `align_min_coins` coincidences
/// with the source (`source_targets`) and how many of those measured aligned
/// (`source_aligned_targets`). A source is aligned on the session when it measures aligned on
/// EVERY qualifying target and on at least two of them; a session with one tom mic cannot
/// establish alignment. Without a handed-in verdict (the core's own full-session runs) the
/// count over the slice's tom mics decides.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SessionAlign {
    /// Per mic index: the tom targets the source measured aligned on (enough coins).
    pub source_aligned_targets: Vec<usize>,
    /// Per mic index: the tom targets with at least `align_min_coins` coincidences.
    pub source_targets: Vec<usize>,
}

impl SessionAlign {
    /// Whether the source at `source` is aligned on the session (see the type's documentation).
    #[must_use]
    pub fn is_aligned(&self, source: usize) -> bool {
        let t = self.source_targets.get(source).copied().unwrap_or(0);
        t >= 2 && self.source_aligned_targets.get(source).copied().unwrap_or(0) == t
    }
}

/// [`cross_bleed`] with the alignment handed in (#296): `prior` holds the pairs of an earlier run
/// over the SAME mics (the same tom and source indices) — the alignment is a property of the
/// PAIR, not of one hit list, so the re-detection pass (whose candidate times sit on the
/// detector's grid before the walk-back) reuses the alignment decided on the stored hits instead
/// of re-measuring it on a hit list that the alignment itself shaped. When a prior pair exists,
/// its `aligned`, `align_coins` and `align_med_abs_lead_ms` replace the measured ones (the
/// recorded evidence stays the pass-1 decision) and its `align_inherited` is carried (#309);
/// everything else is measured as usual.
#[must_use]
pub fn cross_bleed_prior(
    mics: &[CrossMic],
    sr: u32,
    p: &CrossBleedParams,
    prior: Option<&[PairLearned]>,
) -> (Vec<Vec<CrossMark>>, Vec<PairLearned>) {
    cross_bleed_with(
        mics,
        &CrossBleedContext {
            prior,
            ..CrossBleedContext::new(sr, p)
        },
    )
}

/// [`cross_bleed_prior`] with the per-source session verdict handed in (see
/// [`cross_bleed_session`] and [`SessionAlign`]).
#[must_use]
pub fn cross_bleed_prior_session(
    mics: &[CrossMic],
    sr: u32,
    p: &CrossBleedParams,
    prior: Option<&[PairLearned]>,
    session: Option<&SessionAlign>,
) -> (Vec<Vec<CrossMark>>, Vec<PairLearned>) {
    cross_bleed_with(
        mics,
        &CrossBleedContext {
            sr,
            params: p,
            prior,
            session,
        },
    )
}

/// The inputs of one run of the rule besides the mics: the session rate, the parameters, and the
/// two optional hand-ins of the app's two-pass, per-target runs.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CrossBleedContext<'a> {
    /// The rate of every signal of the run.
    pub sr: u32,
    /// The rule's parameters.
    pub params: &'a CrossBleedParams,
    /// The pairs of an earlier run over the SAME mics (#296, see [`cross_bleed_prior`]); `None`
    /// measures every pair.
    pub prior: Option<&'a [PairLearned]>,
    /// The per-source session verdict (#313, see [`cross_bleed_session`]); `None` counts over the
    /// slice's tom mics.
    pub session: Option<&'a SessionAlign>,
}

impl<'a> CrossBleedContext<'a> {
    /// A context without a prior and without a handed-in session verdict (the core's own
    /// full-session run, [`cross_bleed`]).
    #[must_use]
    pub fn new(sr: u32, params: &'a CrossBleedParams) -> Self {
        CrossBleedContext {
            sr,
            params,
            prior: None,
            session: None,
        }
    }
}

/// Run the rule over the close mics of one session with every input in one [`CrossBleedContext`]
/// (see the module documentation). [`cross_bleed`], [`cross_bleed_session`],
/// [`cross_bleed_prior`] and [`cross_bleed_prior_session`] are thin wrappers of it.
///
/// The phases: the levels of every hit (`Run::new`), the alignment of every (tom, source)
/// pair (`measured_rows`), the per-source session test (`SourceVerdict::count`), the
/// target-level inheritance (`target_inherits`), then per tom target the learned pairs
/// (`learn_pairs`) and the verdict per hit (`judge_hits`).
#[must_use]
pub(crate) fn cross_bleed_with(mics: &[CrossMic], ctx: &CrossBleedContext) -> (Vec<Vec<CrossMark>>, Vec<PairLearned>) {
    let run = Run::new(mics, ctx);
    let toms: Vec<usize> = mics
        .iter()
        .enumerate()
        .filter(|(_, m)| m.kind == XKind::Tom)
        .map(|(i, _)| i)
        .collect();
    // the sources: every close mic (kick, snare and the other tom mics — a tom mic bleeds on the
    // other tom close mics as well); the target itself is skipped in the per-target loop.
    // An Overhead source is NOT a tom-pair source: the tom's own-band level against the oh's
    // full-band peak is not like with like (the oh rings with every cymbal), and the #286 A1
    // measurement of the no-lead common-band form is a SNARE-target test — it lives in the
    // soft gate's `soft_overhead_peaks` (crates/app ghost_gate session step), not here. A
    // measured intermediate form (the oh in the tom pairs) dropped about two thirds of the
    // tom hits (Session A t1 66 -> 19). The set itself is measured by
    // [`measure_pair_alignment`] below (the same filter, per pair).
    // #313: the per-source session test needs every (tom, source) pair's alignment measured
    // before any verdict. A time-alignment tool shifts whole tracks, so a source's bleed lag
    // reads near zero on EVERY tom target it coincides with; a small lag on one target alone
    // is a close mic. A source is aligned on the session only when it measures aligned on
    // every tom target that has at least `align_min_coins` coincidences with it, and on at
    // least two such targets; a session with one tom mic cannot establish alignment.
    let measured = measured_rows(mics, &run.cm, run.p);
    let verdict = SourceVerdict::count(&measured, ctx.session, run.p, mics.len());
    let target_inherits = target_inherits(&toms, &measured, &verdict, mics.len());
    let mut learned = Vec::new();
    // per tom (pushed in tom order, read at the mic index `a` as before the phase split: the two
    // agree when the tom mics lead the slice; otherwise `counts[a]` is out of range)
    let mut counts: Vec<Vec<[usize; 4]>> = Vec::new();
    let mut marks: Vec<Vec<CrossMark>> = mics.iter().map(|m| vec![NO_MARK; m.motion.len()]).collect();
    for (ai, &a) in toms.iter().enumerate() {
        // `measured` is read at the tom order `ai` (it is filled at the mic index), as before
        let pair = learn_pairs(&run, &verdict, a, &measured[ai], target_inherits[a]);
        counts.push(vec![[0; 4]; pair.len()]);
        judge_hits(&run, a, &pair, &mut marks[a], &mut counts[a]);
        let learn_start = learned.len();
        learned.extend(pair.iter().map(|(_, l)| *l));
        for (pl, c) in learned[learn_start..].iter_mut().zip(&counts[a]) {
            pl.aligned_counts = *c;
        }
    }
    (marks, learned)
}

/// The mark of a hit before the verdict: not bleed, no coinciding source.
const NO_MARK: CrossMark = CrossMark {
    bleed: false,
    lead_ms: None,
    ratio_db: None,
    hat_lead_ms: None,
    hat_ratio_db: None,
    aligned_mark: false,
};

/// One run's inputs and the levels of every hit, measured once before any pair is judged.
struct Run<'r> {
    mics: &'r [CrossMic<'r>],
    sr: u32,
    p: &'r CrossBleedParams,
    prior: Option<&'r [PairLearned]>,
    /// The level window (samples).
    w: (i64, i64),
    /// The filter run-in before a level window (samples).
    pre: i64,
    /// The coincidence band (s).
    coincide: f64,
    /// #355: the canonical position of every hit of every mic (per mic, per hit).
    cm: Vec<Vec<f64>>,
    /// Per mic, the level of every CANONICAL hit time in its own band.
    lv: Vec<Vec<f64>>,
    /// Per mic, the full-band levels of an Overhead source's canonical hit times; `None` for
    /// every other kind.
    oh_lv: Vec<Option<Vec<f64>>>,
}

impl<'r> Run<'r> {
    /// The canon phase ([`canon_motion`], #355) and the levels phase: every canonical hit of
    /// every mic in its own band, and the full-band level of every canonical hit of an
    /// Overhead source.
    fn new(mics: &'r [CrossMic<'r>], ctx: &CrossBleedContext<'r>) -> Self {
        let CrossBleedContext {
            sr,
            params: p,
            prior,
            session: _,
        } = *ctx;
        let w = (ms_to_samples(p.window_ms.0, sr), ms_to_samples(p.window_ms.1, sr));
        let pre = ms_to_samples(p.preroll_ms, sr);
        let coincide = p.coincide_ms / 1000.0;
        let cm = canon_all(mics, sr);
        let lv: Vec<Vec<f64>> = mics
            .iter()
            .enumerate()
            .map(|(i, m)| levels(m.signal, m.kind, &cm[i], sr, w, pre))
            .collect();
        // the full-band peak levels of the Overhead sources (the #286 A1 common-band test),
        // per target kind; the targets are tom close mics, so one recomputation covers them.
        // The overhead is a mix of everything; only a common band compares like with like —
        // the full-band peak, the same measure on both mics (no filter sections).
        let oh_lv: Vec<Option<Vec<f64>>> = mics
            .iter()
            .enumerate()
            .map(|(i, m)| {
                if m.kind.in_target_band() {
                    // the full-band peak (no filter sections) — the #286 A1 common-band measure
                    Some(
                        cm[i]
                            .iter()
                            .map(|&t| band_level_db(m.signal, &[], (t * f64::from(sr)).round() as i64, w, pre))
                            .collect(),
                    )
                } else {
                    None
                }
            })
            .collect();
        Run {
            mics,
            sr,
            p,
            prior,
            w,
            pre,
            coincide,
            cm,
            lv,
            oh_lv,
        }
    }
}

/// The alignment phase: per mic index, the measured rows of the tom at that index (empty for a
/// mic that is not a tom), from [`measure_pair_alignment`].
fn measured_rows(mics: &[CrossMic], canon: &[Vec<f64>], p: &CrossBleedParams) -> Vec<Vec<MeasuredRow>> {
    let mut measured: Vec<Vec<MeasuredRow>> = vec![Vec::new(); mics.len()]; // per tom: the rows, source order
    for (a, b, aligned, coins_n, med_abs_lead) in measure_pairs(mics, canon, p.coincide_ms / 1000.0, p) {
        measured[a].push(MeasuredRow {
            source: b,
            align: PairAlignment {
                aligned,
                coins: coins_n,
                med_abs_lead_ms: med_abs_lead,
            },
        });
    }
    measured
}

/// The per-source session test of one run (#313): the handed-in [`SessionAlign`] when the caller
/// has one, else the count over the slice's tom mics.
struct SourceVerdict<'r> {
    session: Option<&'r SessionAlign>,
    /// Per source: the tom targets with at least `align_min_coins` coincidences (in-call count;
    /// all zero when a session verdict is handed in).
    source_targets: Vec<usize>,
    /// Per source: how many of those the pair measured aligned (in-call count, as above).
    source_aligned_targets: Vec<usize>,
}

impl<'r> SourceVerdict<'r> {
    /// The session phase: per source, the tom targets with at least `align_min_coins`
    /// coincidences, and how many of those the pair measured aligned. With a handed-in session
    /// verdict (the app's per-target runs, one tom per call) the SESSION-WIDE counts win (#313):
    /// the in-call count over the slice's single tom could never reach the two-target bar.
    fn count(measured: &[Vec<MeasuredRow>], session: Option<&'r SessionAlign>, p: &CrossBleedParams, n: usize) -> Self {
        let mut source_targets = vec![0usize; n];
        let mut source_aligned_targets = vec![0usize; n];
        if session.is_none() {
            for rows in measured {
                for row in rows {
                    if row.align.coins >= p.align_min_coins {
                        source_targets[row.source] += 1;
                        if row.align.aligned {
                            source_aligned_targets[row.source] += 1;
                        }
                    }
                }
            }
        }
        SourceVerdict {
            session,
            source_targets,
            source_aligned_targets,
        }
    }

    /// A source is aligned on the session only when it measured aligned on EVERY qualifying
    /// target, and on at least two of them.
    fn is_aligned(&self, b: usize) -> bool {
        match self.session {
            Some(s) => s.is_aligned(b),
            None => self.source_targets[b] >= 2 && self.source_aligned_targets[b] == self.source_targets[b],
        }
    }

    /// The recorded `source_aligned_targets` of source `b`: the session-wide count when the
    /// caller handed the verdict in (#313), else the in-call count.
    fn aligned_targets_of(&self, b: usize) -> usize {
        self.session.map_or(self.source_aligned_targets[b], |s| {
            s.source_aligned_targets.get(b).copied().unwrap_or(0)
        })
    }

    /// The recorded `source_targets` of source `b` (see [`SourceVerdict::aligned_targets_of`]).
    fn targets_of(&self, b: usize) -> usize {
        self.session.map_or(self.source_targets[b], |s| {
            s.source_targets.get(b).copied().unwrap_or(0)
        })
    }
}

/// The inheritance phase, per mic index. #309, bound to the session test by #313: the
/// target-level inheritance fires only from a SESSION-aligned source's measured-aligned pair —
/// when one exists on the target, the tool was applied to the whole session, so EVERY pair of
/// the target judges by the level test alone (the lead of the other pairs carries no information
/// on such a session). On a session where the only measured-aligned pair's source fails the
/// session test (one close-miked target reading a small lag — ENST) no pair inherits.
fn target_inherits(toms: &[usize], measured: &[Vec<MeasuredRow>], verdict: &SourceVerdict, n: usize) -> Vec<bool> {
    let mut target_inherits = vec![false; n];
    for (ai, &a) in toms.iter().enumerate() {
        target_inherits[a] = measured[ai]
            .iter()
            .any(|row| row.align.aligned && verdict.is_aligned(row.source));
    }
    target_inherits
}

/// The learning phase of tom target `a`: per measured row (source order) the source index and
/// the pair's learned bounds. `aligned_counts` stays zero here; [`judge_hits`] counts it.
fn learn_pairs(
    run: &Run,
    verdict: &SourceVerdict,
    a: usize,
    rows: &[MeasuredRow],
    target_inherits: bool,
) -> Vec<(usize, PairLearned)> {
    // #309: X's canonical hit times sorted, for the clear-case test below (#355: the canon
    // positions, not the refined list): a clear case is an S hit WITHOUT an X hit within
    // `coincide_ms` — a coinciding hit of X is the tom's own motion, and its ratio must not
    // enter the pair's bleed distribution (a real stroke's ratio would raise the ceiling and
    // hide the bleed).
    let mut x_sorted: Vec<f64> = run.cm[a].clone();
    x_sorted.sort_by(f64::total_cmp);
    let x_hit_near = |ts: f64| {
        let k = x_sorted.partition_point(|&tx| tx < ts);
        [x_sorted.get(k.saturating_sub(1)).copied(), x_sorted.get(k).copied()]
            .into_iter()
            .flatten()
            .any(|tx| (tx - ts).abs() <= run.coincide + 1e-9)
    };
    rows.iter()
        .map(|row| {
            (
                row.source,
                learn_pair(run, verdict, a, row, target_inherits, &x_hit_near),
            )
        })
        .collect()
}

/// The learned bounds of one pair: the alignment (measured, inherited or the prior's), the
/// clear cases' lead and ratio medians and the ratio ceiling.
fn learn_pair(
    run: &Run,
    verdict: &SourceVerdict,
    a: usize,
    row: &MeasuredRow,
    target_inherits: bool,
    x_hit_near: &impl Fn(f64) -> bool,
) -> PairLearned {
    let p = run.p;
    let prior = run.prior;
    let b = row.source;
    let measured_aligned = row.align.aligned;
    let coins_n = row.align.coins;
    let med_abs_lead = row.align.med_abs_lead_ms;
    // #313: the pair judges by the level test alone when the source is aligned on the
    // session, or when a session-aligned source's pair measured aligned on this target
    // (the #309 inheritance). A pair that measured aligned on its own but whose source
    // fails the session test keeps the two-part verdict.
    let aligned = verdict.is_aligned(b) || target_inherits;
    let align_inherited = aligned && !measured_aligned;
    // the pass-1 ratio fields, when the caller handed pairs in (see above)
    let prior_ratios = prior
        .and_then(|ps| ps.iter().find(|l| l.tom == a && l.source == b))
        .map(|prev| (prev.ratio_db, prev.ratio_p95_db, prev.ratio_max_db));
    // #296: the pass-1 decision wins when the caller hands the pairs in (the alignment is
    // a property of the pair, not of one hit list — see [`cross_bleed_prior`]). The
    // pass-1 per-pair decision already carries the session test and its inheritance
    // (#313), so it replaces the measurement verbatim.
    let (aligned, coins_n, med_abs_lead, align_inherited, src_aligned_targets, src_targets) = prior
        .and_then(|ps| ps.iter().find(|l| l.tom == a && l.source == b))
        .map_or(
            (
                aligned,
                coins_n,
                med_abs_lead,
                align_inherited,
                // #313: the session-wide counts when the caller handed the verdict in
                verdict.aligned_targets_of(b),
                verdict.targets_of(b),
            ),
            |prev| {
                (
                    prev.aligned,
                    prev.align_coins,
                    prev.align_med_abs_lead_ms,
                    prev.align_inherited,
                    prev.source_aligned_targets,
                    prev.source_targets,
                )
            },
        );
    let ClearCases {
        mut ratios,
        ratios_clear,
        mut leads,
    } = clear_cases(run, a, b, x_hit_near);
    let lead_learned = if leads.len() >= p.min_clear {
        median(&mut leads)
    } else {
        None
    };
    let mut ratio_learned = if ratios.len() >= p.min_clear {
        median(&mut ratios)
    } else {
        None
    };
    // #309: the pair's clear-case ratio distribution — median, p95 and max. The max is
    // the loudest bleed the pair itself produces; on an aligned pair a coinciding hit
    // louder than it is the tom's own motion, never bleed (bleed of S on X is never
    // louder than the bleed this pair produces). A ceiling needs at least
    // `align_min_coins` clear cases (the alignment's own bar, no new number): on a pair
    // where the detector fires on nearly every source hit the clear cases are a handful
    // and the max is one stray value — the level test alone then.
    let mut sorted = ratios_clear.clone();
    sorted.sort_by(f64::total_cmp);
    let mut p95 = if sorted.len() >= p.align_min_coins {
        let idx = ((sorted.len() - 1) as f64 * 0.95).ceil() as usize;
        Some(sorted[idx])
    } else {
        None
    };
    let mut max = if sorted.len() >= p.align_min_coins {
        sorted.last().copied()
    } else {
        None
    };
    if let Some((m, p95_p, max_p)) = prior_ratios {
        ratio_learned = m;
        p95 = p95_p;
        max = max_p;
    }
    PairLearned {
        tom: a,
        source: b,
        lead_ms: lead_learned,
        ratio_db: ratio_learned,
        ratio_p95_db: p95,
        ratio_max_db: max,
        aligned,
        align_coins: coins_n,
        align_med_abs_lead_ms: med_abs_lead,
        aligned_counts: [0; 4],
        align_inherited,
        source_aligned_targets: src_aligned_targets,
        source_targets: src_targets,
    }
}

/// The samples of one pair over every hit of its source: the level ratios (every source hit,
/// the OLD set), the ratios of the clear cases only, and the bleed leads.
struct ClearCases {
    /// `L_X - L_S` per source hit above the silence floor.
    ratios: Vec<f64>,
    /// The same ratio, only for the clear cases (no hit of X within the coincidence band).
    ratios_clear: Vec<f64>,
    /// The bleed arrival lead (ms) per source hit, inside `(0, clear_lead_cap_ms]`.
    leads: Vec<f64>,
}

/// Measure every hit of source `b` on tom target `a` (see [`ClearCases`]).
fn clear_cases(run: &Run, a: usize, b: usize, x_hit_near: &impl Fn(f64) -> bool) -> ClearCases {
    let (sr, p, w, pre) = (run.sr, run.p, run.w, run.pre);
    let x = &run.mics[a].signal;
    let src_t = run.oh_lv[b].as_ref(); // the oh's levels in the target's band
    let lv_s = |k: usize| src_t.map_or(run.lv[b][k], |v| v[k]);
    let mut ratios: Vec<f64> = Vec::new();
    let mut ratios_clear: Vec<f64> = Vec::new();
    let mut leads: Vec<f64> = Vec::new();
    // the hits of the source at their CANONICAL positions (#355): the ratios and the arrival
    // search must not move with a sub-millisecond shift of the refined list.
    for (k, &ts) in run.cm[b].iter().enumerate() {
        // #309 defect 1 (coordinator gate 2026-11-03): `lead_ms` and `ratio_db` are the
        // OLD set — every source hit, as main — because the unaligned two-part verdict's
        // `lead_min` moves with the lead median otherwise (Session B changed). The
        // clear-case exclusion feeds the CEILING only (`ratio_p95_db`, `ratio_max_db`):
        // a coinciding hit of X is the tom's own motion, and its ratio must not raise
        // the pair's bleed ceiling and hide the bleed behind it.
        let clear = !x_hit_near(ts);
        let lx = band_level_db(
            x,
            &band_sections(XKind::Tom.role().band(), sr),
            (ts * f64::from(sr)).round() as i64,
            w,
            pre,
        );
        if lx <= p.silence_db {
            continue;
        }
        let r = lx - lv_s(k);
        ratios.push(r);
        if clear {
            ratios_clear.push(r);
        }
        // the bleed arrival on X: the peak of X's response in [ts, ts + clear_lead_cap_ms],
        // then the first motion of it (the walk-back); a bleed arrives later, so the search
        // goes forward from the source hit
        let s0 = (ts * f64::from(sr)).round() as i64;
        let cap = ms_to_samples(p.clear_lead_cap_ms, sr);
        let a0 = s0.clamp(0, x.len() as i64 - 1) as usize;
        let a1 = (s0 + cap).clamp(0, x.len() as i64) as usize;
        let peak = (a0..a1)
            .map(|i| (x[i].abs(), i))
            .max_by(|u, v| u.0.total_cmp(&v.0))
            .map_or(s0, |(_, i)| i as i64);
        let (foot, _) = first_motion(x, peak, sr, peak as f64 / f64::from(sr), None, None);
        let lead = (foot as f64 / f64::from(sr) - ts) * 1000.0;
        #[cfg(test)]
        eprintln!("DBG clear case ts={ts} lx={lx} lead={lead} foot={foot} peak={peak}");
        if lead > 0.0 && lead <= p.clear_lead_cap_ms {
            leads.push(lead);
        }
    }
    ClearCases {
        ratios,
        ratios_clear,
        leads,
    }
}

/// The verdict phase of tom target `a`: per hit of X, the loudest coinciding source hit decides
/// the evidence (the lead and the ratio are stored either way); any firing source marks the hit.
/// `marks_a` are X's marks; `counts_a` the aligned-pair margin counts per pair (pair order).
fn judge_hits(
    run: &Run,
    a: usize,
    pair: &[(usize, PairLearned)],
    marks_a: &mut [CrossMark],
    counts_a: &mut [[usize; 4]],
) {
    let (p, coincide) = (run.p, run.coincide);
    let (mics, lv, oh_lv) = (run.mics, &run.lv, &run.oh_lv);
    for (j, &tx) in run.cm[a].iter().enumerate() {
        let mut best_level = f64::NEG_INFINITY;
        let mut bleed = false;
        let mut by_aligned = false;
        for (pi, &(b, pl)) in pair.iter().enumerate() {
            let lvs = &lv[b];
            let Some(k) = (0..run.cm[b].len())
                .filter(|&k| run.cm[b][k] - tx <= coincide + 1e-9 && tx - run.cm[b][k] <= coincide + 1e-9)
                .max_by(|&u, &v| lvs[u].total_cmp(&lvs[v]))
            else {
                continue;
            };
            let l = (tx - run.cm[b][k]) * 1000.0;
            // an Overhead source's ratio is measured full-band (the #286 A1 common-band
            // test); every other kind in its own band
            let s_level = oh_lv[b].as_ref().map_or(lvs[k], |v| v[k]);
            let r = lv[a][j] - s_level;
            if lvs[k] > best_level {
                best_level = lvs[k];
                marks_a[j].lead_ms = Some(l);
                marks_a[j].ratio_db = Some(r);
            }
            if mics[b].kind.in_target_band() {
                // the #286 A1 overhead test: both mics in the target's band, the oh louder
                // than the target by more than the margin is bleed. No lead condition: the
                // a1-wake286 measurement shows the lead does not separate hat/cymbal bleed
                // from a ghost note (both classes sit at about 0 ms).
                if r < -p.overhead_margin_db {
                    bleed = true;
                }
            } else {
                // #103 step 2: record the hat pair's own evidence on every hit with a
                // coinciding hat hit, firing or not (see [`CrossMark::hat_lead_ms`]). The
                // #103 measured verdict is that no
                // NEW hat-pair threshold separates (the best pair, level -20 dB and lead
                // -12 ms, removes 398 of 2927 hat-cause scored FPs — left-share 76.4 % ->
                // 62.8 %, limit 50 % — and costs 18 real strokes; every tighter pair trades
                // recall for little; the sweep is recorded in the ticket). The GENERIC
                // verdict below still fires on hat pairs exactly as on kick and snare
                // pairs — disabling it (an intermediate commit) kept 521 hat-bleed tom
                // events that main drops and regressed the tom F1 rows (the 103-tomdiff
                // measurement). The learned pair medians stay as stored evidence.
                if mics[b].kind == XKind::Hat {
                    marks_a[j].hat_lead_ms = Some(l);
                    marks_a[j].hat_ratio_db = Some(r);
                }
                // the verdict (the measured form, #285): the hit is bleed when it arrives later
                // than the source hit by more than the pair's lead bound AND the level ratio
                // falls below the margin (`margin_db`: 0.0 = the source strictly louder; the
                // #285 gated variant sets a positive margin via `DRUMSPIKE_A6_MARGIN_DB`). The
                // learned median ratio plus a slack cannot do this job: the detected bleed's
                // ratio spans the whole clear-case range (the detector only fires on the loud
                // tail of the bleed responses, which the clear cases sample as well), so no
                // median-based margin separates bleed from a real hit. The lead and the
                // source-louder test do: a real hit is loudest and earliest on its own mic.
                // The learned ratio stays as stored evidence (the review queue B9).
                //
                // #296: an ALIGNED pair judges by level alone — the alignment removed the
                // bleed's arrival delay, so the lead test cannot fire; the ratio sweep of the
                // ticket is counted per margin.
                //
                // #309: and the level alone is not enough — the kick-in mic is loud in its
                // band even on a small kick, so a medium tom stroke with a kick under it
                // reads -3 to -5 dB while the pair's real bleed sits far lower. Bleed of S
                // on X is never louder than the bleed this pair produces: the clear cases
                // (S hits without an X hit) give the pair's bleed ratio distribution, and a
                // coinciding hit whose ratio is ABOVE the pair's clear-case maximum
                // (`ratio_max_db`) is the tom's own motion, never bleed. A pair with fewer
                // than `align_min_coins` clear cases keeps the level test alone. Unaligned pairs
                // keep the two-part verdict unchanged.
                if pl.aligned {
                    if r < p.margin_db && pl.ratio_max_db.is_none_or(|mx| r <= mx) {
                        bleed = true;
                        by_aligned = true;
                    }
                    for (mi, &m) in ALIGN_COUNT_MARGINS_DB.iter().enumerate() {
                        if r < m {
                            counts_a[pi][mi] += 1;
                        }
                    }
                } else {
                    let lead_min = pl.lead_ms.map_or(p.min_lead_ms, |l| (l * 0.25).max(p.min_lead_ms));
                    if r < p.margin_db && l > lead_min {
                        bleed = true;
                    }
                }
            }
        }
        marks_a[j].bleed = bleed;
        // #313: only an aligned pair's mark feeds the pass-1 inherit rule (the app's
        // `bleed_pass1`); a two-part mark does not survive the re-detection — the pass-2
        // verdict judges the candidate on its own, as before #296.
        marks_a[j].aligned_mark = bleed && by_aligned;
    }
}

#[cfg(test)]
mod tests;
