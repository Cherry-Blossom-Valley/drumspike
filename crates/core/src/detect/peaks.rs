//! Peak picking on the detector activations (P2-4), for both reference chains.
//!
//! Input: the `(frames, 5)` `f32` activations of the detector in
//! row-major order. Output: the onsets of all five classes, sorted by frame and class, with
//! `time = frame / 100`. Each class is picked on its own column with its threshold
//! ([`THRESHOLDS`]: BD 0.22, SD 0.24, TT 0.32, HH 0.22, CY+RD 0.30) and the reference timing
//! (`pre_avg` 0.1 s, `post_avg` 0.01 s, `pre_max` 0.02 s, `post_max` 0.01 s, combine 0.02 s at 100
//! fps: 10, 1, 2, 1 and 2 frames). [`Chain`] selects the rules ([`Chain::Mix`] uses the
//! madmom rules; `docs/archive/phase2-report.md`, section "P2-1c R3 vs CLI gap"):
//!
//! | Rule | [`Chain::Cli`] (the CLI reference post-processing) | [`Chain::Madmom`] (madmom 27f032e `NotePeakPickingProcessor`) |
//! |---|---|---|
//! | moving average | mean over `[t - 10, t + 1]`, edge padding; `np.convolve` with the `f32` kernel `ones(12) / 12` | `uniform_filter` over `[t - 10, t + 1]`, zero padding |
//! | peak | `proc = max(0, act - avg)`; peak where `proc` is the maximum of `[t - 2, t + 1]` (edge padding) and `proc >= threshold` | `det = act * (act >= avg + threshold)`; peak where `det != 0` and `det` is the maximum of `[t - 2, t + 1]` (zero padding) |
//! | combine (2 frames) | chain: a peak within 2 frames of the previous peak joins its group; the group keeps the frame with the largest `proc`, the first on a tie | `combine_events(.., 0.02, 'left')`: keep the first; the next peak is compared with the kept one (`t - t_kept <= 0.02 + 1e-12` s) |
//! | frame 0 | no rule | no onset of the class when its only peaks are at frame 0 (`if not onsets.any()`) |
//!
//! # Arithmetic
//!
//! Both references compare in `f32` (numpy 1.26 casts the Python threshold to `f32`), so
//! the thresholds are rounded to `f32` before every comparison. The moving averages copy
//! the reference arithmetic bit for bit (checked on all 5.9 million values of the 56
//! reference files of each chain, `docs/archive/phase2-report.md`, section P2-4):
//!
//! * CLI: `np.convolve` calls the BLAS `sdot` (`OpenBLAS`) for each output: the 12 products
//!   `x * f32(1/12)` are rounded to `f32` and summed in `f64` in window order, and the sum is
//!   rounded to `f32`.
//! * madmom: scipy's `NI_UniformFilter1D` keeps a running `f64` sum over the zero-padded
//!   line (`tmp += new - old`, the difference first) and stores `tmp / 12` as `f32`.

use super::Chain;
use super::N_CLASSES;

/// Per-class thresholds of the reference detector (BD, SD, TT, HH, CY+RD).
pub const THRESHOLDS: [f64; N_CLASSES] = [0.22, 0.24, 0.32, 0.22, 0.30];
/// Frames per second of the activations.
pub const FPS: f64 = 100.0;
/// Moving average: frames before `t` (`round(0.1 * 100)`).
pub const PRE_AVG: usize = 10;
/// Moving average: frames after `t` (`round(0.01 * 100)`).
pub const POST_AVG: usize = 1;
/// Local maximum: frames before `t` (`round(0.02 * 100)`).
pub const PRE_MAX: usize = 2;
/// Local maximum: frames after `t` (`round(0.01 * 100)`).
pub const POST_MAX: usize = 1;
/// Combine window in seconds (2 frames).
pub const COMBINE_S: f64 = 0.02;
/// Combine window in frames (`max(1, round(0.02 * 100))`, the CLI rule).
pub const COMBINE_FRAMES: usize = 2;

/// One detected onset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Onset {
    /// Frame index (100 frames per second).
    pub frame: usize,
    /// Class index 0..5: BD, SD, TT, HH, CY+RD (MIDI 35, 38, 47, 42, 49).
    pub class: usize,
    /// `frame / 100` in seconds.
    pub time: f64,
    /// The activation of the class at the frame.
    pub activation: f32,
}

/// How often each rule of the pickers decided something (for tests and the parity
/// report). All counts are per peak or per group, summed over the classes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Branches {
    /// Peaks before the combine step.
    pub peaks: usize,
    /// Peaks within [`PRE_AVG`] frames of the start (the moving average reads the padding).
    pub pad_start: usize,
    /// Peaks at the last frame (the average and the maximum read the padding).
    pub pad_end: usize,
    /// CLI: a peak within [`COMBINE_FRAMES`] of the previous peak joined its group.
    pub cli_joined: usize,
    /// CLI: a group whose span is longer than [`COMBINE_FRAMES`] (the chain rule).
    pub cli_chain: usize,
    /// CLI: a group with two peaks of the same largest `proc` (the first one wins).
    pub cli_tie: usize,
    /// CLI: a peak at frame 0 (kept: the CLI has no frame-0 rule).
    pub cli_frame0: usize,
    /// madmom: a peak merged into the kept one by `combine_events`.
    pub madmom_merged: usize,
    /// madmom: a peak within [`COMBINE_FRAMES`] of the previous peak that is kept because
    /// the kept one is further away (the 'left' rule, where the CLI chain would merge).
    pub madmom_left_anchor: usize,
    /// madmom: a peak equal to its predecessor (a plateau; both pass the maximum test).
    pub madmom_plateau: usize,
    /// madmom: a class whose only peaks are at frame 0, dropped.
    pub madmom_frame0_dropped: usize,
    /// madmom: a peak at frame 0 kept because the class has other peaks.
    pub madmom_frame0_kept: usize,
}

impl Branches {
    /// The counters as `(name, value)` pairs, in declaration order.
    pub fn pairs(&self) -> [(&'static str, usize); 12] {
        [
            ("peaks", self.peaks),
            ("pad_start", self.pad_start),
            ("pad_end", self.pad_end),
            ("cli_joined", self.cli_joined),
            ("cli_chain", self.cli_chain),
            ("cli_tie", self.cli_tie),
            ("cli_frame0", self.cli_frame0),
            ("madmom_merged", self.madmom_merged),
            ("madmom_left_anchor", self.madmom_left_anchor),
            ("madmom_plateau", self.madmom_plateau),
            ("madmom_frame0_dropped", self.madmom_frame0_dropped),
            ("madmom_frame0_kept", self.madmom_frame0_kept),
        ]
    }

    /// Add the counts of `other`.
    pub fn add(&mut self, other: &Branches) {
        self.peaks += other.peaks;
        self.pad_start += other.pad_start;
        self.pad_end += other.pad_end;
        self.cli_joined += other.cli_joined;
        self.cli_chain += other.cli_chain;
        self.cli_tie += other.cli_tie;
        self.cli_frame0 += other.cli_frame0;
        self.madmom_merged += other.madmom_merged;
        self.madmom_left_anchor += other.madmom_left_anchor;
        self.madmom_plateau += other.madmom_plateau;
        self.madmom_frame0_dropped += other.madmom_frame0_dropped;
        self.madmom_frame0_kept += other.madmom_frame0_kept;
    }
}

/// The chain's moving average of one activation column over `[t - 10, t + 1]` (module
/// docs: edge padding and the `sdot` sum for [`Chain::Cli`], zero padding and the scipy
/// running sum for [`Chain::Madmom`]).
pub fn moving_average(x: &[f32], chain: Chain) -> Vec<f32> {
    let n = x.len();
    if n == 0 {
        return Vec::new();
    }
    let len = PRE_AVG + 1 + POST_AVG;
    match chain {
        Chain::Cli => {
            let k = 1.0f32 / len as f32;
            let at = |i: isize| x[i.clamp(0, n as isize - 1) as usize];
            (0..n as isize)
                .map(|t| {
                    let mut d = 0f64;
                    for j in t - PRE_AVG as isize..=t + POST_AVG as isize {
                        d += f64::from(at(j) * k);
                    }
                    d as f32
                })
                .collect()
        }
        Chain::Madmom | Chain::Mix => {
            // The line buffer: PRE_AVG zeros, the column, POST_AVG zeros (as f64).
            let line: Vec<f64> = std::iter::repeat_n(0.0, PRE_AVG)
                .chain(x.iter().map(|&v| f64::from(v)))
                .chain(std::iter::repeat_n(0.0, POST_AVG))
                .collect();
            let mut out = Vec::with_capacity(n);
            let mut tmp = 0f64;
            for v in &line[..len] {
                tmp += v;
            }
            out.push((tmp / len as f64) as f32);
            for t in 1..n {
                tmp += line[t + len - 1] - line[t - 1];
                out.push((tmp / len as f64) as f32);
            }
            out
        }
    }
}

/// The CLI picker on one column: the frames of the onsets.
fn pick_cli(x: &[f32], threshold: f32, b: &mut Branches) -> Vec<usize> {
    let n = x.len();
    let avg = moving_average(x, Chain::Cli);
    let proc: Vec<f32> = x.iter().zip(&avg).map(|(&a, &m)| (a - m).max(0.0)).collect();
    let at = |i: isize| proc[i.clamp(0, n as isize - 1) as usize];
    let peaks: Vec<usize> = (0..n)
        .filter(|&t| {
            let ti = t as isize;
            let wmax = (ti - PRE_MAX as isize..=ti + POST_MAX as isize)
                .map(at)
                .fold(f32::NEG_INFINITY, f32::max);
            proc[t] >= wmax && proc[t] >= threshold
        })
        .collect();
    count_pads(&peaks, n, b);
    b.cli_frame0 += peaks.iter().filter(|&&p| p == 0).count();
    let mut kept = Vec::new();
    let mut group: Vec<usize> = Vec::new();
    let close = |group: &mut Vec<usize>, kept: &mut Vec<usize>, b: &mut Branches| {
        if group.is_empty() {
            return;
        }
        // Python max(group, key=proc): the first of the largest.
        let mut best = group[0];
        for &g in &group[1..] {
            if proc[g] > proc[best] {
                best = g;
            }
        }
        if group.iter().filter(|&&g| proc[g] == proc[best]).count() > 1 {
            b.cli_tie += 1;
        }
        if group[group.len() - 1] - group[0] > COMBINE_FRAMES {
            b.cli_chain += 1;
        }
        kept.push(best);
        group.clear();
    };
    for p in peaks {
        if group.last().is_some_and(|&l| p - l <= COMBINE_FRAMES) {
            b.cli_joined += 1;
            group.push(p);
        } else {
            close(&mut group, &mut kept, b);
            group.push(p);
        }
    }
    close(&mut group, &mut kept, b);
    kept
}

/// The madmom picker on one column: the frames of the onsets.
fn pick_madmom(x: &[f32], threshold: f32, b: &mut Branches) -> Vec<usize> {
    let n = x.len();
    let avg = moving_average(x, Chain::Madmom);
    let det: Vec<f32> = x
        .iter()
        .zip(&avg)
        .map(|(&a, &m)| if a >= m + threshold { a } else { 0.0 })
        .collect();
    let at = |i: isize| {
        if i < 0 || i >= n as isize { 0.0 } else { det[i as usize] }
    };
    let peaks: Vec<usize> = (0..n)
        .filter(|&t| {
            let ti = t as isize;
            let wmax = (ti - PRE_MAX as isize..=ti + POST_MAX as isize)
                .map(at)
                .fold(0f32, f32::max);
            det[t] != 0.0 && det[t] == wmax
        })
        .collect();
    count_pads(&peaks, n, b);
    b.madmom_plateau += peaks.windows(2).filter(|w| w[1] == w[0] + 1).count();
    // `if not onsets.any()`: nothing when there is no peak or every peak is at frame 0.
    if peaks.iter().all(|&p| p == 0) {
        b.madmom_frame0_dropped += usize::from(!peaks.is_empty());
        return Vec::new();
    }
    b.madmom_frame0_kept += usize::from(peaks[0] == 0);
    // combine_events(onsets / fps, 0.02, 'left') in seconds, as madmom computes it.
    let delta = COMBINE_S + 1e-12;
    let time = |f: usize| f as f64 / FPS;
    let mut kept = vec![peaks[0]];
    let mut left = time(peaks[0]);
    for w in peaks.windows(2) {
        let right = time(w[1]);
        if right - left <= delta {
            b.madmom_merged += 1;
        } else {
            if w[1] - w[0] <= COMBINE_FRAMES {
                b.madmom_left_anchor += 1;
            }
            kept.push(w[1]);
            left = right;
        }
    }
    kept
}

fn count_pads(peaks: &[usize], n: usize, b: &mut Branches) {
    b.peaks += peaks.len();
    b.pad_start += peaks.iter().filter(|&&p| p < PRE_AVG).count();
    b.pad_end += peaks.iter().filter(|&&p| p + 1 == n).count();
}

/// The onset frames of one activation column with the chain's picker and `threshold`
/// (compared in `f32`); `b` counts the rules that decided.
pub fn pick_column(x: &[f32], threshold: f64, chain: Chain, b: &mut Branches) -> Vec<usize> {
    let thr = threshold as f32;
    match chain {
        Chain::Cli => pick_cli(x, thr, b),
        Chain::Madmom | Chain::Mix => pick_madmom(x, thr, b),
    }
}

/// P4-3: the candidate onsets of one activation column for the detection histogram: the frames
/// that the chain's picker gives at `floor` (the lowest threshold the Sensitivity slider
/// reaches), each with its strength `x - avg` (the column minus its moving average, in `f32`),
/// the margin a threshold is compared with. A frame with strength `s` passes a threshold `th`
/// when `s >= th` (up to the `f32` rounding of `avg + th` in the madmom rule); its neighbours
/// and the combine rule can still take it out, so the hit count at `th` comes from
/// [`pick_column`], not from these strengths.
pub fn candidates(x: &[f32], floor: f64, chain: Chain) -> Vec<(usize, f32)> {
    let frames = pick_column(x, floor, chain, &mut Branches::default());
    if frames.is_empty() {
        return Vec::new();
    }
    let avg = moving_average(x, chain);
    frames.into_iter().map(|f| (f, x[f] - avg[f])).collect()
}

/// [`pick`] that also returns the branch counts.
///
/// # Panics
///
/// When `activations.len()` is not a multiple of [`N_CLASSES`].
pub fn pick_counted(activations: &[f32], chain: Chain) -> (Vec<Onset>, Branches) {
    pick_counted_with(activations, chain, &THRESHOLDS)
}

/// [`pick_counted`] with per-class `thresholds` in place of [`THRESHOLDS`].
fn pick_counted_with(activations: &[f32], chain: Chain, thresholds: &[f64; N_CLASSES]) -> (Vec<Onset>, Branches) {
    assert!(
        activations.len().is_multiple_of(N_CLASSES),
        "activations: {} values is not (frames, {N_CLASSES})",
        activations.len()
    );
    let mut b = Branches::default();
    let mut out = Vec::new();
    for (class, &thr) in thresholds.iter().enumerate() {
        let col: Vec<f32> = activations.iter().skip(class).step_by(N_CLASSES).copied().collect();
        for frame in pick_column(&col, thr, chain, &mut b) {
            out.push(Onset {
                frame,
                class,
                time: frame as f64 / FPS,
                activation: col[frame],
            });
        }
    }
    out.sort_by_key(|o| (o.frame, o.class));
    (out, b)
}

/// Onsets of the `(frames, 5)` row-major activations with the chain's picker, sorted by
/// frame and class.
///
/// # Panics
///
/// When `activations.len()` is not a multiple of [`N_CLASSES`].
pub fn pick(activations: &[f32], chain: Chain) -> Vec<Onset> {
    pick_counted(activations, chain).0
}

/// [`pick`] with per-class `thresholds` (BD, SD, TT, HH, CY+RD) in place of [`THRESHOLDS`]:
/// the app's sensitivity control scales them (`static/app.js`, the `thresholds` parameter
/// that the reference CLI takes as `--thresholds`).
///
/// # Panics
///
/// When `activations.len()` is not a multiple of [`N_CLASSES`].
pub fn pick_with(activations: &[f32], chain: Chain, thresholds: &[f64; N_CLASSES]) -> Vec<Onset> {
    pick_counted_with(activations, chain, thresholds).0
}

#[cfg(test)]
mod tests;
