//! A7 (#353): the kit-template NMFD detection function, an alternative to [`superflux`].
//!
//! The templates come only from the user's own audio: 5 to 10 confirmed hits per drum of the
//! session (in the evaluation: the first annotated hits). Never ship a template. Opt-in only:
//! nothing in the default analysis reads this module, so the default detection output stays
//! byte-identical.
//!
//! 1. [`template_from_hits`]: the mean attack-frame spectrum of the confirmed hits, normalised.
//! 2. [`nmfd_activations`]: non-negative matrix factorisation with the templates fixed as `W`
//!    (Dittmar & Gaertner 2014, NMFD with one-frame drums): the magnitude spectrogram `V` is
//!    modelled as `W H` with `H >= 0` by multiplicative updates (Euclidean cost), so every other
//!    drum's template absorbs its own energy and the drum's activation row is its detection
//!    function. The spectrogram is the compressed log of the band magnitudes
//!    (`log10(1 + x)`, [`crate::detect::features`]); [`decompress`] inverts it.

use rayon::prelude::*;

/// A kit template: the mean attack spectrum over the 84 bands, normalised to its maximum.
#[derive(Debug, Clone, PartialEq)]
pub struct Template {
    /// The 84 band values.
    pub bands: Vec<f32>,
}

/// Frame rate of the spectrogram: 100 frames per second (`HOP` 441 at 44.1 kHz).
pub const FRAMES_PER_S: f64 = 100.0;

/// Invert the `log10(1 + x)` compression of the spectrogram: the band magnitudes.
pub fn decompress(spec: &[f32]) -> Vec<f32> {
    spec.iter().map(|&s| 10f32.powf(s) - 1.0).collect()
}

/// The attack frame of one hit: the frame in `[t, t + 20 ms]` with the highest band sum.
fn attack_frame(spec: &[f32], t: f64) -> Option<usize> {
    let frames = spec.len() / crate::detect::features::N_BANDS;
    let f0 = (t * FRAMES_PER_S).floor().max(0.0) as usize;
    let f1 = (f0 + 2).min(frames); // 20 ms of attack frames
    (f0..f1)
        .map(|i| {
            let s = &spec[i * crate::detect::features::N_BANDS..(i + 1) * crate::detect::features::N_BANDS];
            let sum: f32 = s.iter().sum();
            (i, sum)
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// The mean attack-frame spectrum of up to `max_hits` hit times, or `None` with fewer than
/// `min_hits` of them. The attack frame of a hit is the frame in `[t, t + 20 ms]` with the
/// highest band sum; the template is the mean of the hits' spectra, normalised to its maximum.
pub fn template_from_hits(spec: &[f32], hits: &[f64], min_hits: usize, max_hits: usize) -> Option<Template> {
    template_from_hits_multi(&[spec], &[hits], min_hits, max_hits)
}

/// The pooled variant of [`template_from_hits`] (A7c, #409): the mean attack-frame spectrum over
/// hit sets taken from several spectrograms - e.g. the same drum across the items of one kit, like
/// a full song. Up to `max_hits` hits in the order given (set by set, hit by hit), `None` with
/// fewer than `min_hits` of them or on a length mismatch between `specs` and `hit_sets`.
pub fn template_from_hits_multi(
    specs: &[&[f32]],
    hit_sets: &[&[f64]],
    min_hits: usize,
    max_hits: usize,
) -> Option<Template> {
    if specs.len() != hit_sets.len() {
        return None;
    }
    let mut acc = vec![0f32; crate::detect::features::N_BANDS];
    let mut n = 0usize;
    for (spec, hits) in specs.iter().zip(hit_sets) {
        for &t in hits.iter() {
            if n >= max_hits {
                break;
            }
            let best = attack_frame(spec, t)?;
            let s = &spec[best * crate::detect::features::N_BANDS..(best + 1) * crate::detect::features::N_BANDS];
            for (a, &v) in acc.iter_mut().zip(s) {
                *a += v;
            }
            n += 1;
        }
    }
    if n < min_hits {
        return None;
    }
    let max = acc.iter().cloned().fold(0f32, f32::max);
    if max <= 0.0 {
        return None;
    }
    for v in acc.iter_mut() {
        *v /= max;
    }
    Some(Template { bands: acc })
}

/// The activation of every template over the frames: `H` in `V ~= W H`, `W` fixed, by
/// multiplicative updates on the decompressed magnitudes (Euclidean cost). `iters` updates;
/// `H` starts uniform. Returns one `Vec<f32>` of `frames` activations per template, in order.
pub fn nmfd_activations(spec: &[f32], templates: &[Template], iters: usize) -> Vec<Vec<f32>> {
    let n_bands = crate::detect::features::N_BANDS;
    let frames = spec.len() / n_bands;
    let r = templates.len();
    if frames == 0 || r == 0 {
        return Vec::new();
    }
    let v = decompress(spec);
    // transpose to band-major: v[b * frames + i]
    let mut v_banded = vec![0f32; n_bands * frames];
    for i in 0..frames {
        for b in 0..n_bands {
            v_banded[b * frames + i] = v[i * n_bands + b];
        }
    }
    // W: bands x r (column per template)
    let mut w = vec![0f32; n_bands * r];
    for (j, t) in templates.iter().enumerate() {
        for (b, &val) in t.bands.iter().enumerate() {
            w[b * r + j] = val;
        }
    }
    // WtW: r x r
    let mut wtw = vec![0f32; r * r];
    for i in 0..r {
        for j in 0..r {
            let mut s = 0f32;
            for b in 0..n_bands {
                s += w[b * r + i] * w[b * r + j];
            }
            wtw[i * r + j] = s;
        }
    }
    let mut h = vec![1f32; r * frames];
    let mut next = vec![0f32; r * frames];
    for _ in 0..iters {
        next.par_chunks_mut(frames).enumerate().for_each(|(j, nj)| {
            for i in 0..frames {
                // numerator: (W^T V)[j, i]; denominator: (W^T W H)[j, i] from the old H
                let mut num = 0f32;
                for b in 0..n_bands {
                    num += w[b * r + j] * v_banded[b * frames + i];
                }
                let mut den = 0f32;
                for k in 0..r {
                    den += wtw[j * r + k] * h[k * frames + i];
                }
                nj[i] = if den > 1e-12 {
                    h[j * frames + i] * num / den
                } else {
                    h[j * frames + i]
                };
            }
        });
        h.copy_from_slice(&next);
    }
    (0..r).map(|j| h[j * frames..(j + 1) * frames].to_vec()).collect()
}

/// What a peak threshold is relative to (#407 A7b): the row maximum (the shipped rule) or the
/// median of the row's own local maxima (the row's own distribution, not its loudest frame).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreshRef {
    /// `thresh_rel` of the row's maximum (default, the #353 rule).
    Max,
    /// `thresh_rel` of the median of the row's local maxima.
    Median,
}

/// The kit template parameters of the peak picking ([`pick_hits`]).
#[derive(Debug, Clone, PartialEq)]
pub struct NmfdParams {
    /// The minimum gap between hits in seconds.
    pub min_gap_s: f64,
    /// A peak must reach this fraction of the reference level (`thresh_ref`).
    pub thresh_rel: f32,
    /// What the threshold is relative to (default [`ThreshRef::Max`]).
    pub thresh_ref: ThreshRef,
}

impl Default for NmfdParams {
    fn default() -> Self {
        NmfdParams {
            min_gap_s: 0.02,
            thresh_rel: 0.1,
            thresh_ref: ThreshRef::Max,
        }
    }
}

/// Pick the hits of one activation row: local maxima of `h` that are at least `p.thresh_rel` of
/// the reference level (`p.thresh_ref`: the row maximum, or the median of the row's own local
/// maxima), at least `p.min_gap_s` apart (the higher one wins). Times in seconds.
pub fn pick_hits(h: &[f32], p: &NmfdParams) -> Vec<f64> {
    if h.len() < 3 {
        return Vec::new();
    }
    let max = h.iter().cloned().fold(0f32, f32::max);
    if max <= 0.0 {
        return Vec::new();
    }
    let locals: Vec<usize> = (1..h.len() - 1)
        .filter(|&i| h[i] > h[i - 1] && h[i] >= h[i + 1])
        .collect();
    let reference = match p.thresh_ref {
        ThreshRef::Max => max,
        ThreshRef::Median => {
            if locals.is_empty() {
                return Vec::new();
            }
            let mut amps: Vec<f32> = locals.iter().map(|&i| h[i]).collect();
            amps.sort_by(f32::total_cmp);
            let mid = amps.len() / 2;
            let med = if amps.len() % 2 == 1 {
                amps[mid]
            } else {
                (amps[mid - 1] + amps[mid]) / 2.0
            };
            // explosion guard: a flat, noisy row has a tiny median and would pick everywhere;
            // the peak must also reach a tenth of the row maximum.
            med.max(0.1 * max)
        }
    };
    let mut peaks: Vec<usize> = locals
        .iter()
        .copied()
        .filter(|&i| h[i] >= p.thresh_rel * reference)
        .collect();
    // enforce the minimum gap, the higher peak wins
    let gap = (p.min_gap_s * FRAMES_PER_S).round() as usize;
    let mut keep: Vec<usize> = Vec::new();
    for &i in &peaks {
        if let Some(&j) = keep.last()
            && i - j < gap
        {
            if h[i] > h[j] {
                keep.pop();
            } else {
                continue;
            }
        }
        keep.push(i);
    }
    peaks = keep;
    peaks.into_iter().map(|i| i as f64 / FRAMES_PER_S).collect()
}

/// The hits of one drum: [`nmfd_activations`] and [`pick_hits`] on its row, against the other
/// templates so their energy is absorbed there. `mine` is the index in `templates`.
pub fn nmfd_hits(spec: &[f32], templates: &[Template], mine: usize, iters: usize, p: &NmfdParams) -> Vec<f64> {
    if mine >= templates.len() {
        return Vec::new();
    }
    let acts = nmfd_activations(spec, templates, iters);
    pick_hits(&acts[mine], p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::features::N_BANDS;

    fn spec_with(frames: usize, peaks: &[(usize, usize, f32)]) -> Vec<f32> {
        // one frame of a band bump, compressed like the pipeline does
        let mut s = vec![0f32; frames * N_BANDS];
        for &(f, b, amp) in peaks {
            s[f * N_BANDS + b] = (1.0 + amp).log10();
        }
        s
    }

    #[test]
    fn a_template_comes_from_five_hits_and_their_attack_frame() {
        // the band bump sits one frame after the hit: the attack frame search finds it
        let mut s = vec![0f32; 50 * N_BANDS];
        for &t in &[0.05, 0.15, 0.25, 0.35, 0.45] {
            let f = (t * FRAMES_PER_S) as usize + 1;
            s[f * N_BANDS + 10] = 2.0f32.log10();
            s[f * N_BANDS + 11] = 1.0f32.log10();
        }
        let t = template_from_hits(&s, &[0.05, 0.15, 0.25, 0.35, 0.45], 5, 10).expect("template");
        assert!(
            t.bands[10] > t.bands[11],
            "the louder band dominates: {:?}",
            &t.bands[10..12]
        );
        assert!((t.bands[10] - 1.0).abs() < 1e-6, "normalised to the maximum");
        assert!(template_from_hits(&s, &[0.05], 5, 10).is_none(), "too few hits");
    }

    #[test]
    fn the_activation_peaks_at_the_templates_frames_and_the_other_template_stays_quiet() {
        // two drums: A bumps band 10, B bumps band 70; each sounds alone at its own frames
        let a_t = Template {
            bands: {
                let mut b = vec![0f32; N_BANDS];
                b[10] = 1.0;
                b
            },
        };
        let b_t = Template {
            bands: {
                let mut b = vec![0f32; N_BANDS];
                b[70] = 1.0;
                b
            },
        };
        let s = spec_with(10, &[(2, 10, 1000.0), (7, 70, 1000.0)]);
        let h = nmfd_activations(&s, &[a_t.clone(), b_t.clone()], 50);
        assert_eq!(h.len(), 2);
        let (a, b) = (&h[0], &h[1]);
        let a_at = |i: usize| a[i];
        assert!(
            a_at(2) > 50.0 * a_at(5),
            "A active at frame 2: {} vs {}",
            a_at(2),
            a_at(5)
        );
        assert!(b[7] > 50.0 * b[2], "B active at frame 7: {} vs {}", b[7], b[2]);
        assert!(a_at(7) < 0.05 * a_at(2), "A quiet where only B sounds: {}", a_at(7));
        assert!(b[2] < 0.05 * b[7], "B quiet where only A sounds: {}", b[2]);
    }

    #[test]
    fn a_template_is_pooled_over_hit_sets_from_several_spectrograms() {
        // two kits' items: A bumps band 10 in both specs, B only in the second
        let mut s1 = vec![0f32; 50 * N_BANDS];
        let mut s2 = vec![0f32; 50 * N_BANDS];
        for &t in &[0.05, 0.15] {
            let f = (t * FRAMES_PER_S) as usize + 1;
            s1[f * N_BANDS + 10] = 2.0f32.log10();
            s2[f * N_BANDS + 10] = 3.0f32.log10();
        }
        s2[10 * N_BANDS + 70] = 2.0f32.log10();
        let t = template_from_hits_multi(&[&s1, &s2], &[&[0.05, 0.15], &[0.10]], 3, 10).expect("pooled template");
        assert!(t.bands[10] > 0.9, "the repeated band dominates");
        assert!(
            t.bands[70] > 0.0 && t.bands[70] < 0.9,
            "the single hit is diluted: {}",
            t.bands[70]
        );
        // max_hits caps the total over the sets, min_hits counts pooled hits
        let t = template_from_hits_multi(&[&s1, &s2], &[&[0.05, 0.15], &[0.09]], 3, 2);
        assert!(t.is_none(), "only two of the three hits fit under max_hits 2");
        assert!(template_from_hits_multi(&[&s1], &[&[0.05]], 2, 10).is_none());
        assert!(template_from_hits_multi(&[&s1], &[&[0.05]], 1, 10).is_some());
        assert!(
            template_from_hits_multi(&[&s1], &[], 1, 10).is_none(),
            "length mismatch"
        );
    }

    #[test]
    fn pick_hits_finds_local_maxima_and_enforces_the_gap() {
        let p = NmfdParams::default();
        let mut h = vec![0f32; 100];
        h[10] = 5.0;
        h[11] = 3.0; // one frame after 10: inside the gap, the lower one loses
        h[50] = 2.0;
        h[51] = 1.0; // not a local maximum
        assert_eq!(pick_hits(&h, &p), vec![0.10, 0.50]);
        // below the relative threshold: a small bump is not picked
        let mut h2 = h.clone();
        h2[90] = 0.01;
        assert_eq!(pick_hits(&h2, &p), vec![0.10, 0.50]);
        let h2 = vec![0f32; 10];
        assert!(pick_hits(&h2, &p).is_empty());
        assert!(pick_hits(&[1.0, 2.0], &p).is_empty(), "fewer than 3 frames");
    }

    #[test]
    fn the_median_reference_thresholds_against_the_rows_own_distribution() {
        // two loud peaks (5.0, 4.0) and one small bump (0.4); the median of the local maxima is
        // 4.0, so thresh_rel 0.5 x median keeps the two loud ones and drops 0.4; the Max
        // reference (0.1 x 5.0 = 0.5) would keep it too here, but 0.01 would not.
        let mut h = vec![0f32; 100];
        h[10] = 5.0;
        h[50] = 4.0;
        h[90] = 0.4;
        let mut p = NmfdParams {
            thresh_ref: ThreshRef::Median,
            thresh_rel: 0.5,
            ..Default::default()
        };
        assert_eq!(pick_hits(&h, &p), vec![0.10, 0.50]);
        p.thresh_rel = 0.1;
        assert_eq!(pick_hits(&h, &p), vec![0.10, 0.50, 0.90], "0.4 passes 0.1 x 4.0 median");
        // an all-flat row has no local maxima: nothing is picked
        let flat = vec![1.0f32; 10];
        assert!(pick_hits(&flat, &p).is_empty());
    }

    #[test]
    fn empty_inputs_give_no_activations() {
        assert!(
            nmfd_activations(
                &[],
                &[Template {
                    bands: vec![0.0; N_BANDS]
                }],
                10
            )
            .is_empty()
        );
        assert!(nmfd_activations(&spec_with(5, &[(1, 3, 10.0)]), &[], 10).is_empty());
    }
}
