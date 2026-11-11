//! Onset refinement: the port of `engine._refine_sr` (the v3.16 hybrid method).
//!
//! The method, operation by operation as the Python engine runs it:
//!
//! 1. Cut `[t0 + lo - 20 ms, t0 + hi + 20 ms)`: the search window [`RefineWindow`]
//!    (default `-2..+12` ms, P2-9) plus 20 ms margin on each side.
//! 2. Hilbert envelope of the cut ([`hilbert_env`](reference::hilbert_env): FFT analytic signal, no approximation),
//!    smoothed with a 0.2 ms moving average (`np.convolve(..., mode="same")`), in dB.
//! 3. Rise over 0.5 ms; the anchor `k` is the steepest rise in `[t0 + lo, t0 + hi)`, the
//!    search window clipped at the midpoints towards the neighbouring coarse hits.
//! 4. Walk back from `k` to where the envelope leaves the pre-attack level + 3 dB; that is
//!    the anchor sample. `rise_db` = envelope at `k` minus the pre-attack level.
//!    A window where the envelope never rises (steepest rise <= 0 dB, a decaying tail)
//!    takes the last sample of the search range as the anchor instead (P2-9).
//! 5. The 25 % rule in `[anchor - 1.5 ms, anchor + 10 ms)` on `|x|` in float32; a crossing
//!    at the window start (the previous hit still ringing) resolves to the anchor, or to
//!    `t0` when the window has no rise.
//!
//! Every index is computed with Python `int()` truncation ([`crate::pyops`]) and every
//! early return gives `t0` back unchanged, like the engine. The only place where the two
//! implementations can differ is the floating-point order inside the FFT (rustfft versus
//! numpy's pocketfft) and the smoothing sum; `tests/test_golden.py` (the refine cases)
//! checks that on the corpus, edge, window and refinement-benchmark scenarios.
//!
//! One module per mechanism: `reference` (version 1, above), `reach` (version 2), `walk` (the first-motion back-track of
//! versions 3 to 5), `edge` (the steep edge of versions 6 and 7) and `track` (the [`RefineRule`] record, its dispatch
//! and the entry points for a track). The items callers outside the crate use keep their path `refine::<name>`;
//! the tuning constants and stage helpers are `pub(crate)` and reached through their mechanism module (R8b).

mod edge;
mod reach;
mod reference;
mod track;
mod walk;

pub use edge::{PlacementDiag, placement_diag};
pub use reach::{ReachStep, TAIL_S};
pub use reference::{RefineWindow, refine_sr};
pub(crate) use track::refine_track_rule_peak;
pub use track::{
    Hit, RefineRule, recompute_velocities, refine_first_motion, refine_rule, refine_rule_peak, refine_track,
    refine_track_rule,
};
#[cfg(test)]
pub(crate) use walk::first_motion_mid;
pub use walk::{BackStep, FM_PEAK_DB, LeadTrace, first_motion, first_motion_trace};

/// A refined hit: the onset (seconds), `rise_db`, what the reach rule did and what the back-track did.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Refined {
    /// Onset in seconds.
    pub t: f64,
    /// `rise_db` of the refinement that gave `t`.
    pub rise: Option<f64>,
    /// The step of [`refine_reach`](reach::refine_reach) ([`ReachStep::Kept`] for the reference rule).
    pub step: ReachStep,
    /// The step of the back-track ([`BackStep::Off`] for versions 1 and 2).
    pub back: BackStep,
}

/// The largest `|x|` in `[from, to)`, clipped to the signal; 0 for an empty range.
fn max_abs(x: &[f32], from: i64, to: i64) -> f32 {
    let f = from.max(0) as usize;
    let t = (to.max(0) as usize).min(x.len());
    if t <= f {
        return 0.0;
    }
    x[f..t].iter().fold(0.0f32, |m, v| m.max(v.abs()))
}

#[cfg(test)]
mod golden_tests;
#[cfg(test)]
mod tests;
