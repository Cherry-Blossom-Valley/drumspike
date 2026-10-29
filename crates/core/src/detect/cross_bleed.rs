//! #281 A6: the cross-track bleed rule for sparse tracks (kick and snare bleed on a tom close mic).
//!
//! On ENST-Drums the tom mics output three to eight times more hits than there are strokes: kick and snare
//! bleed (`docs/migration/reports/model-vs-dsp.md`, part B). A rarely played tom has bleed as its loudest
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

use super::attribution::{band_peak_db, band_sections, ms_to_samples, Biquad, Role};
use crate::refine::first_motion;

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
}

fn env_f64(name: &str) -> Option<f64> {
    std::env::var(name).ok().and_then(|v| v.parse().ok())
}

impl Default for CrossBleedParams {
    fn default() -> Self {
        CrossBleedParams {
            coincide_ms: 10.0,
            min_lead_ms: env_f64("DRUMSPIKE_A6_MIN_LEAD").unwrap_or(0.3),
            window_ms: (-0.5, 8.0),
            preroll_ms: 50.0,
            min_clear: 5,
            clear_lead_cap_ms: 3.0,
            silence_db: -100.0,
            margin_db: env_f64("DRUMSPIKE_A6_MARGIN_DB").unwrap_or(0.0),
            overhead_margin_db: env_f64("DRUMSPIKE_A1_OH_MARGIN_DB").unwrap_or(2.0),
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
}

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
    /// The median bleed level ratio of the clear cases (dB).
    pub ratio_db: Option<f64>,
}

/// The band level (dBFS) of `x` in `[c + w.0, c + w.1)` with the filter run in over `pre` samples
/// before the window; samples outside the signal are zeros (as the tom bleed rule measures).
fn level_at(x: &[f32], sec: &[Biquad], c: i64, w: (i64, i64), pre: i64) -> f64 {
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

/// The level of every motion time of `mic` in its own band.
fn levels(mic: &CrossMic, sr: u32, w: (i64, i64), pre: i64) -> Vec<f64> {
    let sec = band_sections(mic.kind.role().band(), sr);
    mic.motion
        .iter()
        .map(|&t| level_at(mic.signal, &sec, (t * f64::from(sr)).round() as i64, w, pre))
        .collect()
}

/// The band level (dBFS) of a hit of `kind` at time `t` (s) in `x` at rate `sr`, with the rule's
/// level window (for the caller's coincidence tests, e.g. the template exclusion).
pub fn hit_level(x: &[f32], sr: u32, kind: XKind, t: f64, p: &CrossBleedParams) -> f64 {
    let sec = band_sections(kind.role().band(), sr);
    let w = (ms_to_samples(p.window_ms.0, sr), ms_to_samples(p.window_ms.1, sr));
    let pre = ms_to_samples(p.preroll_ms, sr);
    level_at(x, &sec, (t * f64::from(sr)).round() as i64, w, pre)
}

/// The FULL-BAND peak level (dBFS) of a hit at time `t` (s) in `x` at rate `sr`, with the rule's
/// level window — the #286 A1 common-band measure: the overhead is a mix of everything; only
/// a common band compares like with like (no filter sections, the same measure on both mics).
pub fn hit_level_full_band(x: &[f32], sr: u32, t: f64, p: &CrossBleedParams) -> f64 {
    let w = (ms_to_samples(p.window_ms.0, sr), ms_to_samples(p.window_ms.1, sr));
    let pre = ms_to_samples(p.preroll_ms, sr);
    level_at(x, &[], (t * f64::from(sr)).round() as i64, w, pre)
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

/// Run the rule over the close mics of one session (see the module documentation). All signals are at
/// rate `sr`. Returns per tom mic (input order), per hit (input order) the [`CrossMark`], and the
/// learned bounds per pair (tom order, then source order).
pub fn cross_bleed(mics: &[CrossMic], sr: u32, p: &CrossBleedParams) -> (Vec<Vec<CrossMark>>, Vec<PairLearned>) {
    let w = (ms_to_samples(p.window_ms.0, sr), ms_to_samples(p.window_ms.1, sr));
    let pre = ms_to_samples(p.preroll_ms, sr);
    let coincide = p.coincide_ms / 1000.0;
    let lv: Vec<Vec<f64>> = mics.iter().map(|m| levels(m, sr, w, pre)).collect();
    // the full-band peak levels of the Overhead sources (the #286 A1 common-band test),
    // per target kind; the targets are tom close mics, so one recomputation covers them.
    // The overhead is a mix of everything; only a common band compares like with like —
    // the full-band peak, the same measure on both mics (no filter sections).
    let oh_lv: Vec<Option<Vec<f64>>> = mics
        .iter()
        .map(|m| {
            if m.kind.in_target_band() {
                Some(
                    m.motion
                        .iter()
                        .map(|&t| level_at(m.signal, &[], (t * f64::from(sr)).round() as i64, w, pre))
                        .collect(),
                )
            } else {
                None
            }
        })
        .collect();
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
    // tom hits (Session A t1 66 -> 19).
    let srcs: Vec<usize> = (0..mics.len()).filter(|&b| mics[b].kind != XKind::Overhead).collect();
    let mut learned = Vec::new();
    let mut marks: Vec<Vec<CrossMark>> = mics
        .iter()
        .map(|m| {
            vec![
                CrossMark {
                    bleed: false,
                    lead_ms: None,
                    ratio_db: None,
                    hat_lead_ms: None,
                    hat_ratio_db: None,
                };
                m.motion.len()
            ]
        })
        .collect();
    for &a in &toms {
        let x = &mics[a];
        let mut pair: Vec<(usize, PairLearned)> = Vec::new();
        for &b in &srcs {
            if b == a {
                continue;
            }
            let s = &mics[b];
            let src_t = oh_lv[b].as_ref(); // the oh's levels in the target's band
            let lv_s = |k: usize| src_t.map_or(lv[b][k], |v| v[k]);
            let mut ratios: Vec<f64> = Vec::new();
            let mut leads: Vec<f64> = Vec::new();
            for (k, &ts) in s.motion.iter().enumerate() {
                let lx = level_at(
                    x.signal,
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
                // the bleed arrival on X: the peak of X's response in [ts, ts + clear_lead_cap_ms],
                // then the first motion of it (the walk-back); a bleed arrives later, so the search
                // goes forward from the source hit
                let s0 = (ts * f64::from(sr)).round() as i64;
                let cap = ms_to_samples(p.clear_lead_cap_ms, sr);
                let a0 = s0.clamp(0, x.signal.len() as i64 - 1) as usize;
                let a1 = (s0 + cap).clamp(0, x.signal.len() as i64) as usize;
                let peak = (a0..a1)
                    .map(|i| (x.signal[i].abs(), i))
                    .max_by(|u, v| u.0.total_cmp(&v.0))
                    .map_or(s0, |(_, i)| i as i64);
                let (foot, _) = first_motion(x.signal, peak, sr, peak as f64 / f64::from(sr), None, None);
                let lead = (foot as f64 / f64::from(sr) - ts) * 1000.0;
                #[cfg(test)]
                eprintln!("DBG clear case ts={ts} lx={lx} lead={lead} foot={foot} peak={peak}");
                if lead > 0.0 && lead <= p.clear_lead_cap_ms {
                    leads.push(lead);
                }
            }
            let lead_learned = if leads.len() >= p.min_clear {
                median(&mut leads)
            } else {
                None
            };
            let ratio_learned = if ratios.len() >= p.min_clear {
                median(&mut ratios)
            } else {
                None
            };
            pair.push((
                b,
                PairLearned {
                    tom: a,
                    source: b,
                    lead_ms: lead_learned,
                    ratio_db: ratio_learned,
                },
            ));
        }
        learned.extend(pair.iter().map(|(_, l)| *l));
        // the verdict per hit of X: the loudest coinciding source hit decides the evidence (the lead and
        // the ratio are stored either way); any firing source marks the hit
        for (j, &tx) in x.motion.iter().enumerate() {
            let mut best_level = f64::NEG_INFINITY;
            let mut bleed = false;
            for &(b, pl) in &pair {
                let s = &mics[b];
                let lvs = &lv[b];
                let Some(k) = (0..s.motion.len())
                    .filter(|&k| s.motion[k] - tx <= coincide + 1e-9 && tx - s.motion[k] <= coincide + 1e-9)
                    .max_by(|&u, &v| lvs[u].total_cmp(&lvs[v]))
                else {
                    continue;
                };
                let l = (tx - s.motion[k]) * 1000.0;
                // an Overhead source's ratio is measured full-band (the #286 A1 common-band
                // test); every other kind in its own band
                let s_level = oh_lv[b].as_ref().map_or(lvs[k], |v| v[k]);
                let r = lv[a][j] - s_level;
                if lvs[k] > best_level {
                    best_level = lvs[k];
                    marks[a][j].lead_ms = Some(l);
                    marks[a][j].ratio_db = Some(r);
                }
                if s.kind.in_target_band() {
                    // the #286 A1 overhead test: both mics in the target's band, the oh louder
                    // than the target by more than the margin is bleed. No lead condition: the
                    // a1-wake286 measurement shows the lead does not separate hat/cymbal bleed
                    // from a ghost note (both classes sit at about 0 ms).
                    if r < -p.overhead_margin_db {
                        bleed = true;
                    }
                } else if s.kind == XKind::Hat {
                    // #103 step 2: record the hat pair's own evidence on every hit with a
                    // coinciding hat hit, firing or not (see [`CrossMark::hat_lead_ms`]). No hat
                    // verdict fires here: the #103 measured verdict (docs/review/detector-plan.md,
                    // package 8) is that the hat-pair distributions do not separate - the best
                    // threshold pair (level -20 dB, lead -12 ms) removes 398 of 2927 hat-cause
                    // scored FPs (left-share 76.4 % -> 62.8 %, limit 50 %) and costs 18 real
                    // strokes, and every tighter pair trades recall for little; the sweep is
                    // recorded in the ticket. The learned pair medians stay as stored evidence.
                    marks[a][j].hat_lead_ms = Some(l);
                    marks[a][j].hat_ratio_db = Some(r);
                } else {
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
                    let lead_min = pl.lead_ms.map_or(p.min_lead_ms, |l| (l * 0.25).max(p.min_lead_ms));
                    if r < p.margin_db && l > lead_min {
                        bleed = true;
                    }
                }
            }
            marks[a][j].bleed = bleed;
        }
    }
    (marks, learned)
}

#[cfg(test)]
mod tests;
