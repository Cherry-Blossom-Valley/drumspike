//! Spike WAV export: one short blip per hit at full scale, mono at the source's native rate,
//! with the source's `bext` time reference so the file lands at the session position when a
//! DAW imports it.
//!
//! The blip is the hit itself: the sample at the hit index is +1.0 (full scale, 32767 in a
//! 16-bit file) and the following 1 ms decays linearly to zero, so the onset stays
//! sample-exact and the file is visible and audible on a track. Hits closer than the blip
//! keep the louder value per sample, so a later hit's full-scale sample is never covered.
//! Hits before the start or past the end of the file are skipped; a blip that runs past the
//! end is cut.

use std::path::Path;

use crate::detect::attribution::{Biquad, FilterKind};
use crate::io::{write_wav, Bext, WavFormat, WavSpec};
use crate::pyops::py_round;

/// Blip length in seconds (full-scale sample plus the decay).
pub const BLIP_S: f64 = 0.001;
/// Description written into the `bext` chunk of a spike file.
pub const BEXT_DESCRIPTION: &str = "Drum Spike spikes";
/// Description written into the `bext` chunk of an audio-spike file (#209).
pub const AUDIO_BEXT_DESCRIPTION: &str = "Drum Spike audio spikes";
/// The audio-spike snippet clamp in seconds: a snippet is at least [`AUDIO_MIN_S`], at most
/// [`AUDIO_MAX_S`] (#209).
pub const AUDIO_MIN_S: f64 = 0.002;
/// The upper clamp of the audio-spike snippet length (#378; was 0.030, too short for a
/// 40 Hz kick, whose 1.5 cycles are 37.5 ms).
pub const AUDIO_MAX_S: f64 = 0.060;
/// The snippet length when no period is found (silence, no crossing in the window) (#209).
pub const AUDIO_DEFAULT_S: f64 = 0.010;
/// The fade-out at the end of an audio-spike snippet, at most (#209).
pub const AUDIO_FADE_S: f64 = 0.001;
/// The window the period search looks at: the first part of the hit (#209).
pub const AUDIO_PERIOD_WINDOW_S: f64 = 0.030;
/// The lowpass corner that isolates the dominant low component of the hit for the snippet
/// length: the body tone, not the attack click (#378). Two cascaded second-order stages,
/// 24 dB per octave: a 3 kHz click is far out, a 200 Hz snare body passes with a group
/// delay of well under a millisecond.
pub const AUDIO_LOW_HZ: f64 = 800.0;

/// The blip shape at `sr`: `n = max(2, round(0.001 * sr))` samples, `1 - i / n`.
pub fn blip(sr: u32) -> Vec<f32> {
    let n = ((BLIP_S * sr as f64).round() as usize).max(2);
    (0..n).map(|i| (1.0 - i as f64 / n as f64) as f32).collect()
}

/// Sample index of a hit: `round(t * sr)` with ties to even (hit times are `k / sr`, so the
/// product is exact for refined hits); `None` outside `[0, n_samples)`.
pub fn hit_index(t: f64, sr: u32, n_samples: usize) -> Option<usize> {
    let i = py_round(t * sr as f64);
    if i < 0 || i as u64 >= n_samples as u64 {
        None
    } else {
        Some(i as usize)
    }
}

/// The mono spike signal: `n_samples` zeros with a [`blip`] at every hit.
pub fn spike_signal(sr: u32, n_samples: usize, times: &[f64]) -> Vec<f32> {
    let mut x = vec![0.0f32; n_samples];
    let b = blip(sr);
    for &t in times {
        if let Some(i) = hit_index(t, sr, n_samples) {
            for (k, &v) in b.iter().enumerate() {
                if i + k >= n_samples {
                    break;
                }
                x[i + k] = x[i + k].max(v);
            }
        }
    }
    x
}

/// The `bext` chunk of a spike file: time reference, origination date and time and
/// originator copied from the source record, the description and coding history of the
/// spike file itself.
pub fn spike_bext(source: &Bext, sr: u32, format: WavFormat) -> Bext {
    bext_with_description(source, sr, format, BEXT_DESCRIPTION)
}

/// The `bext` chunk of an audio-spike file: the source time reference, the audio-spike
/// description (#209).
pub fn audio_spike_bext(source: &Bext, sr: u32, format: WavFormat) -> Bext {
    bext_with_description(source, sr, format, AUDIO_BEXT_DESCRIPTION)
}

fn bext_with_description(source: &Bext, sr: u32, format: WavFormat, description: &str) -> Bext {
    Bext {
        description: description.to_string(),
        originator: source.originator.clone(),
        originator_reference: String::new(),
        origination_date: source.origination_date.clone(),
        origination_time: source.origination_time.clone(),
        time_reference: source.time_reference,
        version: 1,
        coding_history: format!("A=PCM,F={sr},W={},M=mono,T=Drum Spike", 8 * format.bytes_per_sample()),
    }
}

/// A zero crossing at index `j`: the sample is exactly zero, or the sign changes from
/// `j - 1` to `j` (#209).
fn is_crossing(x: &[f32], j: usize) -> bool {
    x[j] == 0.0 || x[j - 1] * x[j] < 0.0
}

/// The waveform period at `start` in samples, from the zero crossings of the first
/// [`AUDIO_PERIOD_WINDOW_S`] of the hit (#209): twice the median gap of the crossing
/// indices in the window. A crossing is a sign change; a sample that quantized to exact
/// zero between the two signs (a digital-silence run at a crossing) still counts, at the
/// first nonzero sample after it. `None` with fewer than two crossings (silence, a
/// constant, the window too short).
pub fn period_samples(x: &[f32], start: usize, sr: u32) -> Option<f64> {
    // R14: `start` past the signal is a documented `None`, not an underflow panic.
    if start >= x.len() {
        return None;
    }
    let window = ((AUDIO_PERIOD_WINDOW_S * sr as f64).round() as usize).min(x.len() - start);
    if window < 4 || start + window > x.len() {
        return None;
    }
    let end = start + window;
    let mut gaps: Vec<usize> = Vec::new();
    let mut prev: Option<usize> = None; // the last crossing
    let mut last_sign = 0i8; // the sign of the last nonzero sample seen
    for (j, &v) in x.iter().enumerate().take(end).skip(start) {
        if v == 0.0 {
            continue; // a zero run: the crossing sits at the next nonzero sample
        }
        let s = if v > 0.0 { 1 } else { -1 };
        if last_sign != 0 && s != last_sign {
            if let Some(p) = prev {
                gaps.push(j - p);
            }
            prev = Some(j);
        }
        last_sign = s;
    }
    let n = gaps.len();
    if n == 0 {
        return None;
    }
    gaps.sort_unstable();
    let median = if n % 2 == 1 {
        gaps[n / 2] as f64
    } else {
        (gaps[n / 2 - 1] + gaps[n / 2]) as f64 / 2.0
    };
    Some(2.0 * median)
}

/// The dominant low component of the waveform: `x` through two cascaded RBJ lowpass
/// stages at [`AUDIO_LOW_HZ`] (24 dB per octave) forward and then backward, computed in
/// `f64`, back to `f32` (#378). The attack click and the broadband noise stay out; the
/// body tone of a kick, snare or tom drives the crossings. The backward pass makes the
/// smoothing zero-phase: the smoothed crossings sit on the real ones, free of the causal
/// group delay that miscounts the lobes of a tone that runs through the hit (#378
/// round 3).
fn low_component(x: &[f32], sr: u32) -> Vec<f32> {
    let mut rev = lowpass_once(x, sr);
    rev.reverse();
    let mut out = lowpass_once(&rev, sr);
    out.reverse();
    out
}

/// One forward pass of the two cascaded RBJ lowpass stages over all of `x`.
fn lowpass_once(x: &[f32], sr: u32) -> Vec<f32> {
    let stage = Biquad::new(FilterKind::LowPass, AUDIO_LOW_HZ, sr);
    let stages = [stage, stage];
    let mut z = [[0.0f64; 2]; 2];
    let mut out = Vec::with_capacity(x.len());
    for &v in x {
        let mut y = v as f64;
        for (s, zs) in stages.iter().zip(z.iter_mut()) {
            let o = s.b[0] * y + zs[0];
            zs[0] = s.b[1] * y - s.a[0] * o + zs[1];
            zs[1] = s.b[2] * y - s.a[1] * o;
            y = o;
        }
        out.push(y as f32);
    }
    out
}

/// The sign changes of `x[start..scan_to]` (zeros skipped: a crossing sits at the next
/// nonzero sample, as in [`period_samples`]) (#378).
fn sign_changes(x: &[f32], start: usize, scan_to: usize) -> Vec<usize> {
    let mut out = Vec::new();
    let mut last_sign = 0i8;
    for (j, &v) in x.iter().enumerate().take(scan_to).skip(start) {
        if v == 0.0 {
            continue;
        }
        let s = if v > 0.0 { 1 } else { -1 };
        if last_sign != 0 && s != last_sign {
            out.push(j);
        }
        last_sign = s;
    }
    out
}

/// The end index of the audio-spike snippet at the hit `start` and the flag whether the
/// dominant low component drove it (`false`: the default length). The length covers about
/// 1.5 cycles of the LOW tone of the hit, not of the attack click (#378): the lowpass of
/// [`low_component`] isolates the body, and the snippet ends at the real crossing that
/// ends counted lobe 3 of that smoothed signal after the onset - three half-cycles
/// anchored at the onset, which is free of the lowpass group delay and tracks a kick
/// whose pitch drops. Lobes count with the significance rule of
/// [`third_significant_lobe_end`] (#378 round 3). The end is snapped to the nearest raw
/// crossing within +-1 ms. With fewer than three crossings of the smoothed signal
/// (silence, a constant, no low tone) the default [`AUDIO_DEFAULT_S`]. The length clamps
/// to [`AUDIO_MIN_S`]..[`AUDIO_MAX_S`] and to the end of the file; the overlap cut to
/// the next hit happens in [`audio_spike_signal`]. `None` when `start` holds no snippet
/// (outside the file).
fn snippet_end(x: &[f32], smooth: &[f32], start: usize, sr: u32) -> (Option<usize>, bool) {
    if start >= x.len() || start + 1 >= x.len() {
        return (None, false);
    }
    let min_len = ((AUDIO_MIN_S * sr as f64).round() as usize).max(2);
    let max_len = ((AUDIO_MAX_S * sr as f64).round() as usize).max(min_len);
    let scan_to = x.len().min(start.saturating_add(max_len) + 1);
    let mut found = false;
    let end = if smooth.len() > start {
        let crossings = sign_changes(smooth, start + 1, scan_to);
        // the 3rd counted lobe of the smoothed signal drives the end (#378 round 3);
        // with fewer than three crossings there is no tone and the default applies
        let target = if crossings.len() >= 3 {
            third_significant_lobe_end(smooth, start, scan_to, &crossings)
        } else {
            None
        };
        if let Some(target) = target {
            found = true;
            // snap to the nearest raw crossing within +-1 ms (#378 round 2)
            let tol = ((0.001 * sr as f64).round() as usize).max(2);
            let mut best = target;
            let mut best_d = tol + 1;
            for j in start + 1..scan_to {
                if is_crossing(x, j) {
                    let d = j.abs_diff(target);
                    if d < best_d {
                        best_d = d;
                        best = j;
                    }
                }
            }
            if best_d <= tol {
                best
            } else {
                target
            }
        } else {
            // no dominant low component: the 10 ms default (the fade-out covers the cut)
            start + ((AUDIO_DEFAULT_S * sr as f64).round() as usize)
        }
    } else {
        start + ((AUDIO_DEFAULT_S * sr as f64).round() as usize)
    };
    let end = end.max(start + min_len).min(start + max_len).min(x.len());
    if end <= start {
        (None, found)
    } else {
        (Some(end), found)
    }
}

/// The sample index that ends counted lobe 3 of the smoothed low component in
/// `smooth[start..scan_to]`, `None` without one (#378 round 3). Lobes are the segments
/// between the onset and the successive `crossings` (lobe 0 starts at the onset); the
/// peak and the sign of a lobe come from the sample of the largest `|smooth|` in it.
/// `P` is the largest `|peak|` of the first three lobes; a lobe is significant when its
/// peak is at least 25 % of `P`. The walk: a significant lobe with the opposite sign of
/// the last counted one starts a new counted lobe; a significant lobe with the same sign
/// (after an insignificant dip) extends the current counted lobe; an insignificant lobe
/// never extends anything. The end is the end crossing of the last significant segment
/// of counted lobe 3 (trailing insignificant lobes are not included). With fewer than
/// three counted lobes in the scan range the end of the last significant segment, with
/// no significant lobe at all `None`.
fn third_significant_lobe_end(smooth: &[f32], start: usize, scan_to: usize, crossings: &[usize]) -> Option<usize> {
    // peak and sign per lobe: lobe j spans from its start boundary to crossings[j]
    let mut peaks: Vec<f32> = Vec::with_capacity(crossings.len() + 1);
    let mut signs: Vec<i8> = Vec::with_capacity(crossings.len() + 1);
    let mut lo = start;
    for &c in crossings {
        let (p, s) = peak_and_sign(&smooth[lo..c]);
        peaks.push(p);
        signs.push(s);
        lo = c;
    }
    let (p, s) = peak_and_sign(&smooth[lo..scan_to]);
    peaks.push(p);
    signs.push(s);
    // P: the largest |peak| of the first three lobes
    let big = peaks.iter().take(3).fold(0.0f32, |m, &p| m.max(p));
    if big <= 0.0 {
        return None;
    }
    let threshold = 0.25 * big;
    let mut count = 0usize;
    let mut last_sign = 0i8;
    let mut end = None;
    let mut last_sig_end = None;
    for (j, &p) in peaks.iter().enumerate() {
        if p < threshold {
            continue; // an insignificant lobe never extends anything
        }
        let boundary = crossings.get(j).copied().unwrap_or(scan_to);
        last_sig_end = Some(boundary);
        if count > 0 && signs[j] == last_sign {
            // same sign after a dip: the lobe extends the current counted one
            if count == 3 {
                end = Some(boundary);
            }
        } else {
            count += 1;
            last_sign = signs[j];
            if count == 3 {
                end = Some(boundary);
            }
        }
    }
    if count >= 3 {
        end
    } else {
        last_sig_end
    }
}

/// The largest `|v|` of a lobe and the sign at its sample `(0.0, 0)` for an empty or
/// all-zero lobe (#378 round 3).
fn peak_and_sign(seg: &[f32]) -> (f32, i8) {
    let mut best = 0.0f32;
    let mut sign = 0i8;
    for &v in seg {
        let a = v.abs();
        if a > best {
            best = a;
            sign = if v > 0.0 { 1 } else { -1 };
        }
    }
    (best, sign)
}

/// The `(start, end)` sample pairs of the audio-spike snippets of `times`, ascending by
/// start, after the overlap cut to the next hit, with the flag whether the dominant low
/// component drove the length (`false`: the default length, #378). Pairs with
/// `end <= start + 1` (nothing to copy) are left out. The shared body of
/// [`audio_spike_signal`] and [`audio_snippet_lengths`].
fn snippet_ends(source: &[f32], sr: u32, times: &[f64]) -> Vec<(usize, usize, bool)> {
    let mut starts: Vec<usize> = times.iter().filter_map(|&t| hit_index(t, sr, source.len())).collect();
    starts.sort_unstable();
    starts.dedup();
    // the low component drives every snippet length: one lowpass over the whole source (#378)
    let smooth = if starts.is_empty() {
        Vec::new()
    } else {
        low_component(source, sr)
    };
    let mut out = Vec::new();
    for (k, &start) in starts.iter().enumerate() {
        let next = starts.get(k + 1).copied().unwrap_or(usize::MAX);
        let (Some(mut end), found) = snippet_end(source, &smooth, start, sr) else {
            continue;
        };
        end = end.min(next); // the next exported hit cuts: the overlap rule (#209)
        if end <= start + 1 {
            continue; // nothing to copy (degenerate: the next hit starts here)
        }
        out.push((start, end, found));
    }
    out
}

/// The mono audio-spike signal (#209): `n_samples` zeros with a copy of the `source` audio
/// from every hit for about 1.5 periods of the LOW tone at the hit (the length clamped to
/// [`AUDIO_MIN_S`]..[`AUDIO_MAX_S`], [`AUDIO_DEFAULT_S`] without a dominant low component,
/// the next hit cuts, a fade-out of at most [`AUDIO_FADE_S`] to the end, #378). Every other
/// sample is exact zero. Returns the signal and the number of hits that got the default
/// length (no dominant low component found, #378).
pub fn audio_spike_signal(source: &[f32], sr: u32, times: &[f64]) -> (Vec<f32>, usize) {
    let mut out = vec![0.0f32; source.len()];
    let mut no_low = 0;
    for (start, end, found) in snippet_ends(source, sr, times) {
        if !found {
            no_low += 1;
        }
        let fade = (((AUDIO_FADE_S * sr as f64).round() as usize).max(1)).min(end - start);
        let fade_from = end - fade;
        out[start..end].copy_from_slice(&source[start..end]);
        for (j, i) in (fade_from..end).enumerate() {
            out[i] = source[i] * (fade - j) as f32 / fade as f32;
        }
    }
    (out, no_low)
}

/// The length in seconds of the audio-spike snippet of every distinct hit of `times`
/// (ascending hit index; hits outside the file and degenerate zero-length snippets have no
/// entry), for the measurement tools (#378). The same lengths the real export
/// ([`audio_spike_signal`]) writes.
pub fn audio_snippet_lengths(source: &[f32], sr: u32, times: &[f64]) -> Vec<f64> {
    snippet_ends(source, sr, times)
        .into_iter()
        .map(|(start, end, _)| (end - start) as f64 / sr as f64)
        .collect()
}

/// Write the audio-spike file for `times` (seconds) with the snippets copied from `source`
/// ([`audio_spike_signal`]): mono at `sr`, `source.len()` long, encoded as `format`, with
/// [`audio_spike_bext`] when the source has a `bext` chunk. Returns the number of hits that
/// got the default length because no dominant low component was found (#378).
pub fn export_audio_spikes(
    path: &Path,
    sr: u32,
    times: &[f64],
    source: &[f32],
    source_bext: Option<&Bext>,
    format: WavFormat,
) -> std::io::Result<usize> {
    let (signal, no_low) = audio_spike_signal(source, sr, times);
    let spec = WavSpec {
        sample_rate: sr,
        channels: 1,
        format,
        extensible: false,
        channel_mask: 0,
    };
    let bext = source_bext.map(|b| audio_spike_bext(b, sr, format));
    write_wav(path, &spec, &signal, bext.as_ref())?;
    Ok(no_low)
}

/// Write the spike file for `times` (seconds): mono at `sr`, `n_samples` long (the source's
/// length), encoded as `format`, with [`spike_bext`] when the source has a `bext` chunk.
pub fn export_spikes(
    path: &Path,
    sr: u32,
    n_samples: usize,
    times: &[f64],
    source_bext: Option<&Bext>,
    format: WavFormat,
) -> std::io::Result<()> {
    let spec = WavSpec {
        sample_rate: sr,
        channels: 1,
        format,
        extensible: false,
        channel_mask: 0,
    };
    let bext = source_bext.map(|b| spike_bext(b, sr, format));
    write_wav(path, &spec, &spike_signal(sr, n_samples, times), bext.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::{read_wav_mono, MixMode};

    #[test]
    fn spike_bext_keeps_the_source_time_and_describes_the_spike_file() {
        let src = Bext {
            description: "Kick In, take 3".into(),
            originator: "Pro Tools".into(),
            originator_reference: "USID123".into(),
            origination_date: "2026-09-14".into(),
            origination_time: "10:31:02".into(),
            time_reference: 1_728_012_345,
            version: 2,
            coding_history: "A=PCM,F=48000,W=24".into(),
        };
        let b = spike_bext(&src, 96_000, WavFormat::Pcm24);
        assert_eq!(b.description, BEXT_DESCRIPTION);
        assert_eq!(b.originator, "Pro Tools");
        assert_eq!(b.originator_reference, "");
        assert_eq!(
            (b.origination_date.as_str(), b.origination_time.as_str()),
            ("2026-09-14", "10:31:02")
        );
        assert_eq!(b.time_reference, 1_728_012_345);
        assert_eq!(b.version, 1);
        assert_eq!(b.coding_history, "A=PCM,F=96000,W=24,M=mono,T=Drum Spike");
        for (f, w) in [(WavFormat::Pcm16, 16), (WavFormat::Pcm32, 32), (WavFormat::Float32, 32)] {
            assert!(spike_bext(&src, 48_000, f)
                .coding_history
                .starts_with(&format!("A=PCM,F=48000,W={w},")));
        }
        assert_eq!(spike_bext(&Bext::default(), 44_100, WavFormat::Pcm16).time_reference, 0);
    }

    #[test]
    fn blip_starts_at_full_scale_and_decays_to_zero() {
        let b = blip(48_000);
        assert_eq!(b.len(), 48);
        assert_eq!(b[0], 1.0);
        assert!(b[47] > 0.0 && b[47] < 0.05);
        assert!(b.windows(2).all(|w| w[1] < w[0]));
        assert_eq!(blip(96_000).len(), 96);
        assert_eq!(blip(100).len(), 2, "never shorter than two samples");
    }

    /// R14: a `start` past the signal is a documented `None`, not an underflow panic.
    #[test]
    fn period_samples_rejects_a_start_beyond_the_signal() {
        assert_eq!(period_samples(&[0.0], 2, 48_000), None);
        assert_eq!(period_samples(&[], 0, 48_000), None);
        let x: Vec<f32> = (0..4000).map(|i| if i % 2 == 0 { 1.0 } else { -1.0 }).collect();
        assert!(period_samples(&x, 4, 48_000).is_some(), "a valid start works");
    }

    #[test]
    fn hit_index_rounds_and_rejects_outside() {
        assert_eq!(hit_index(0.5, 48_000, 48_000), Some(24_000));
        assert_eq!(hit_index(24_001.0 / 48_000.0, 48_000, 48_000), Some(24_001));
        assert_eq!(hit_index(0.0, 48_000, 48_000), Some(0));
        assert_eq!(hit_index(-1e-9, 48_000, 48_000), Some(0), "rounds to 0");
        assert_eq!(hit_index(-0.001, 48_000, 48_000), None);
        assert_eq!(hit_index(1.0, 48_000, 48_000), None, "one past the end");
        assert_eq!(hit_index(0.5, 48_000, 0), None);
    }

    #[test]
    fn signal_has_a_blip_per_hit_and_keeps_the_louder_sample() {
        let x = spike_signal(48_000, 1_000, &[0.01, 0.0102, 5.0, 999.0 / 48_000.0]);
        assert_eq!(x[480], 1.0);
        // 0.0102 * 48000 = 489.6 rounds to 490; the first blip is still 0.8125 at 489
        assert_eq!(
            x[490], 1.0,
            "the second hit's full-scale sample is not covered by the first blip"
        );
        assert_eq!(x[489], 0.8125);
        assert!(x[479] == 0.0 && x[481] < 1.0 && x[481] > 0.9);
        assert_eq!(x[999], 1.0, "a blip at the last sample is cut after it");
        assert_eq!(x.iter().filter(|&&v| v == 1.0).count(), 3);
        assert!(spike_signal(48_000, 0, &[0.0]).is_empty());
    }

    #[test]
    fn audio_snippet_is_one_and_a_half_periods_to_a_zero_crossing() {
        let sr = 48_000u32;
        // 240 Hz starting at the hit: the period is 200 samples, 1.5 periods are 300 samples
        // (6.25 ms, inside the clamp); the hit (sample 24000 = 0.5 s) starts on a rising
        // zero crossing, so 1.5 periods land exactly on a falling crossing
        let source: Vec<f32> = (0..96_000)
            .map(|i| {
                let dt = i as f64 / 48_000.0 - 0.5;
                if dt < 0.0 {
                    0.0
                } else {
                    (std::f64::consts::TAU * 240.0 * dt).sin() as f32
                }
            })
            .collect();
        let (x, no_period) = audio_spike_signal(&source, sr, &[0.5]);
        assert_eq!(no_period, 0);
        let start = 24_000usize;
        assert_eq!(x[start], source[start], "the snippet starts at the hit sample");
        assert_eq!(x[start - 1], 0.0, "exact silence before the snippet");
        let end = (start + 1..).find(|&j| x[j] == 0.0).unwrap();
        let len = end - start;
        assert!((len as i64 - 300).abs() <= 1, "1.5 periods of 200 samples, got {len}");
        assert!(
            is_crossing(&source, end),
            "the snippet ends on a zero crossing of the source"
        );
        // the unfaded interior is a copy of the source
        let fade = 48usize;
        for j in start..end - fade {
            assert_eq!(x[j], source[j], "sample {j}");
        }
        // the fade-out runs to the end point, at most 1 ms
        for (k, j) in (end - fade..end).enumerate() {
            let want = source[j] * (fade - k) as f32 / fade as f32;
            assert!((x[j] - want).abs() < 1e-6, "fade at {j}");
        }
        assert!(x[end..].iter().all(|&v| v == 0.0), "exact silence after the snippet");
    }

    #[test]
    fn audio_snippet_without_a_period_gets_the_10_ms_default() {
        let sr = 48_000u32;
        let source = vec![0.25f32; 96_000]; // a constant: no crossing, no period
        let (x, no_period) = audio_spike_signal(&source, sr, &[0.5]);
        assert_eq!(no_period, 1);
        let start = 24_000usize;
        let end = (start + 1..).find(|&j| x[j] == 0.0).unwrap();
        assert_eq!(
            end - start,
            (AUDIO_DEFAULT_S * sr as f64).round() as usize,
            "the 10 ms default"
        );
        for &v in x.iter().take(end - 48).skip(start) {
            assert_eq!(v, 0.25);
        }
        assert!(x[end..].iter().all(|&v| v == 0.0));
        // silence in, silence out: no snippet changes the file's silence
        let quiet = vec![0.0f32; 48_000];
        let (y, n) = audio_spike_signal(&quiet, sr, &[0.5]);
        assert_eq!(n, 1);
        assert!(y.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn audio_snippets_clamp_to_2_ms_and_60_ms() {
        let sr = 48_000u32;
        // 6 kHz starting at the hit: 1.5 periods are 12 samples (0.25 ms) - clamped up to
        // the 2 ms minimum
        let source: Vec<f32> = (0..96_000)
            .map(|i| {
                let dt = i as f64 / 48_000.0 - 0.5;
                if dt < 0.0 {
                    0.0
                } else {
                    (std::f64::consts::TAU * 6_000.0 * dt).sin() as f32
                }
            })
            .collect();
        let (x, _) = audio_spike_signal(&source, sr, &[0.5]);
        let end = (24_001..).find(|&j| x[j] == 0.0).unwrap();
        // no low tone: the snippet sits at the 2 ms minimum (the lowpass group delay of a
        // 6 kHz residual may push the 3rd crossing a fraction of a millisecond past it)
        assert!(end - 24_000 >= (AUDIO_MIN_S * sr as f64).round() as usize);
        assert!(
            end - 24_000 <= (0.0023 * sr as f64).round() as usize,
            "at the 2 ms minimum"
        );
        // 40 Hz starting at the hit: 1.5 periods are 1800 samples (37.5 ms) - inside the
        // 60 ms clamp (#378), not capped at the old 30 ms maximum; the end is the 3rd
        // smoothed crossing, snapped to the raw crossing at the full period x 1.5
        let source: Vec<f32> = (0..96_000)
            .map(|i| {
                let dt = i as f64 / 48_000.0 - 0.5;
                if dt < 0.0 {
                    0.0
                } else {
                    (std::f64::consts::TAU * 40.0 * dt).sin() as f32
                }
            })
            .collect();
        let (x, _) = audio_spike_signal(&source, sr, &[0.5]);
        let end = (24_001..).find(|&j| x[j] == 0.0).unwrap();
        assert!(
            (end - 24_000).abs_diff(1800) <= 2,
            "1.5 periods of 1200 samples, got {}",
            end - 24_000
        );
    }

    /// #378: a synthetic kick (55 Hz decaying body + a 3 kHz click and noise in the first
    /// 3 ms) gets a snippet of 1.5 cycles of the LOW tone (27.3 ms +-2 ms), not of the
    /// attack click, ending on a zero crossing.
    #[test]
    fn kick_snippet_covers_the_low_tone_not_the_attack_click() {
        let sr = 48_000u32;
        let t0 = 0.5;
        let start = 24_000usize;
        let source: Vec<f32> = (0..96_000)
            .map(|i| {
                let dt = i as f64 / sr as f64 - t0;
                if dt < 0.0 {
                    return 0.0;
                }
                let mut v = (std::f64::consts::TAU * 55.0 * dt).sin() * (-3.0 * dt).exp();
                if dt < 0.003 {
                    v += (std::f64::consts::TAU * 3_000.0 * dt).sin() * (-3_000.0 * dt).exp() * 0.8;
                    let s = (i as f64 * 12.9898).sin() * 43758.5453;
                    v += (s - s.floor() - 0.5) * 0.3;
                }
                v as f32
            })
            .collect();
        let (x, _) = audio_spike_signal(&source, sr, &[t0]);
        let end = (start + 1..).find(|&j| x[j] == 0.0).unwrap();
        let len_ms = (end - start) as f64 / sr as f64 * 1000.0;
        assert!((len_ms - 27.3).abs() <= 2.0, "kick snippet {len_ms} ms, want 27.3 +-2");
        assert!(
            is_crossing(&source, end),
            "the snippet ends on a zero crossing of the source"
        );
    }

    /// #378 round 2: a small dip below zero inside the 3rd lobe (about 10-15 % of the
    /// peak) does not end the snippet: lobes count with the 25 % hysteresis, so the dip
    /// merges into its neighbours and the snippet ends at the end of the whole lobe.
    #[test]
    fn a_small_dip_in_the_third_lobe_does_not_end_the_snippet() {
        let sr = 48_000u32;
        let t0 = 0.5;
        let start = 24_000usize;
        let source: Vec<f32> = (0..96_000)
            .map(|i| {
                let dt = i as f64 / sr as f64 - t0;
                if dt < 0.0 {
                    return 0.0;
                }
                let mut v = (std::f64::consts::TAU * 55.0 * dt).sin() * (-3.0 * dt).exp();
                // a negative bump centered at 22.7 ms, inside the 3rd lobe (18.2-27.3 ms)
                let c = dt - 0.022_7;
                if c.abs() < 0.002 {
                    let w = (std::f64::consts::PI * c / 0.002).cos();
                    v -= 0.15 * w * w;
                }
                v as f32
            })
            .collect();
        let (x, _) = audio_spike_signal(&source, sr, &[t0]);
        let end = (start + 1..).find(|&j| x[j] == 0.0).unwrap();
        let len_ms = (end - start) as f64 / sr as f64 * 1000.0;
        // the whole 3rd lobe ends at 1.5 periods = 27.3 ms, +-1 ms
        assert!(
            (len_ms - 27.3).abs() <= 1.0,
            "kick snippet {len_ms} ms, want the whole lobe 27.3 +-1"
        );
    }

    /// #378 round 3: the lobe table of the loudest Session A kick (258.801 s) as a synthetic
    /// shape: lobes 0-5 with the measured peaks and boundaries, everything after 28 ms
    /// under 0.02. The small lobes 3-5 stay under the 25 % threshold of P (the largest of
    /// the first three peaks, 0.351), so the snippet ends at the end of lobe 2, 16.0 ms.
    #[test]
    fn the_session_a_kick_lobe_table_ends_at_the_third_big_lobe() {
        let sr = 48_000u32;
        let t0 = 0.5;
        let start = 24_000usize;
        let lobes: [(f64, f64, f64); 6] = [
            (0.000, 0.0040, 0.225),
            (0.0040, 0.0086, -0.351),
            (0.0086, 0.0160, 0.124),
            (0.0160, 0.0193, -0.055),
            (0.0193, 0.0231, 0.079),
            (0.0231, 0.0280, -0.080),
        ];
        let source: Vec<f32> = (0..96_000)
            .map(|i| {
                let dt = i as f64 / sr as f64 - t0;
                if dt < 0.0 {
                    return 0.0;
                }
                for &(b0, b1, amp) in lobes.iter() {
                    if dt < b1 {
                        if dt < b0 {
                            return 0.0;
                        }
                        let u = (dt - b0) / (b1 - b0);
                        return (amp * (std::f64::consts::PI * u).sin().powi(2)) as f32;
                    }
                }
                0.0
            })
            .collect();
        let (x, _) = audio_spike_signal(&source, sr, &[t0]);
        let end = (start + 1..).find(|&j| x[j] == 0.0).unwrap();
        let len_ms = (end - start) as f64 / sr as f64 * 1000.0;
        assert!(
            (len_ms - 16.0).abs() <= 1.0,
            "Session A kick snippet {len_ms} ms, want the end of lobe 2 at 16.0 +-1"
        );
    }

    /// #378: a 40 Hz kick gets its 1.5 cycles (37.5 ms +-2 ms), not the old 30 ms cap.
    #[test]
    fn kick_snippet_of_40_hz_is_not_capped_at_30_ms() {
        let sr = 48_000u32;
        let t0 = 0.5;
        let start = 24_000usize;
        let source: Vec<f32> = (0..96_000)
            .map(|i| {
                let dt = i as f64 / sr as f64 - t0;
                if dt < 0.0 {
                    0.0
                } else {
                    ((std::f64::consts::TAU * 40.0 * dt).sin() * (-2.0 * dt).exp()) as f32
                }
            })
            .collect();
        let (x, _) = audio_spike_signal(&source, sr, &[t0]);
        let end = (start + 1..).find(|&j| x[j] == 0.0).unwrap();
        let len_ms = (end - start) as f64 / sr as f64 * 1000.0;
        assert!(
            (len_ms - 37.5).abs() <= 2.0,
            "40 Hz kick snippet {len_ms} ms, want 37.5 +-2"
        );
    }

    /// #378: a synthetic snare (200 Hz body + broadband noise) gets 7.5 ms +-1 ms.
    #[test]
    fn snare_snippet_covers_the_200_hz_body() {
        let sr = 48_000u32;
        let t0 = 0.5;
        let start = 24_000usize;
        let source: Vec<f32> = (0..96_000)
            .map(|i| {
                let dt = i as f64 / sr as f64 - t0;
                if dt < 0.0 {
                    return 0.0;
                }
                let mut v = (std::f64::consts::TAU * 200.0 * dt).sin() * (-8.0 * dt).exp();
                let s = (i as f64 * 12.9898).sin() * 43758.5453;
                v += (s - s.floor() - 0.5) * 0.3;
                v as f32
            })
            .collect();
        let (x, _) = audio_spike_signal(&source, sr, &[t0]);
        let end = (start + 1..).find(|&j| x[j] == 0.0).unwrap();
        let len_ms = (end - start) as f64 / sr as f64 * 1000.0;
        assert!((len_ms - 7.5).abs() <= 1.0, "snare snippet {len_ms} ms, want 7.5 +-1");
    }

    /// #378 round 3: a tone that runs CONTINUOUSLY through the hit (no silence before,
    /// the shape of the app test the_audio_export_leaves_the_source_unchanged_and_writes_
    /// the_snippets) counts the same three lobes: the zero-phase smoothing puts the
    /// smoothed crossings on the real ones, free of the causal group delay.
    #[test]
    fn a_continuous_tone_counts_the_same_three_lobes() {
        let sr = 48_000u32;
        let source: Vec<f32> = (0..96_000)
            .map(|i| (std::f64::consts::TAU * 240.0 * i as f64 / sr as f64).sin() as f32)
            .collect();
        let lens = audio_snippet_lengths(&source, sr, &[0.5]);
        assert_eq!(lens.len(), 1);
        // 1.5 periods of 200 samples = 300, on a real crossing of the source
        assert!(
            (lens[0] * sr as f64 - 300.0).abs() <= 2.0,
            "continuous 240 Hz snippet {} samples, want 300",
            lens[0] * sr as f64
        );
    }

    /// #378: the lengths helper reports the same lengths the signal writes, per hit.
    #[test]
    fn snippet_lengths_match_the_written_snippets() {
        let sr = 48_000u32;
        let source: Vec<f32> = (0..96_000)
            .map(|i| {
                let dt = i as f64 / 48_000.0 - 0.5;
                if dt < 0.0 {
                    0.0
                } else {
                    (std::f64::consts::TAU * 240.0 * dt).sin() as f32
                }
            })
            .collect();
        // one inside, one outside the file, one duplicate (no second entry)
        let lens = audio_snippet_lengths(&source, sr, &[0.5, 2.0, 0.5]);
        assert_eq!(lens.len(), 1);
        assert!((lens[0] - 300.0 / 48_000.0).abs() < 2.0 / 48_000.0);
    }

    #[test]
    fn an_overlap_cuts_the_snippet_at_the_next_hit() {
        let sr = 48_000u32;
        // 240 Hz: 1.5 periods are 300 samples (6.25 ms); the second hit starts 3 ms after the
        // first (144 samples), so the first snippet is cut there
        let source: Vec<f32> = (0..96_000)
            .map(|i| (std::f64::consts::TAU * 240.0 * i as f64 / 48_000.0).sin() as f32)
            .collect();
        let (x, _) = audio_spike_signal(&source, sr, &[0.5, 0.503]);
        assert_eq!(x[24_000], source[24_000]);
        assert_eq!(
            x[24_143],
            source[24_143] * 1.0 / 48.0,
            "the last sample of the cut snippet is faded"
        );
        assert_eq!(x[24_144], source[24_144], "the second snippet starts at its hit sample");
        assert!(x[24_000..24_144].iter().enumerate().all(|(k, &v)| {
            let j = 24_000 + k;
            v == source[j] || (24_096..24_144).contains(&j) // the copy, then the 1 ms fade
        }));
    }

    #[test]
    fn exported_audio_spikes_read_back_with_the_audio_bext() {
        let dir = std::env::temp_dir().join(format!("drumspike-core-audio-spike-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sr = 48_000u32;
        let source: Vec<f32> = (0..96_000)
            .map(|i| (std::f64::consts::TAU * 240.0 * i as f64 / 48_000.0).sin() as f32)
            .collect();
        let src = Bext {
            time_reference: 123_456,
            ..Bext::default()
        };
        let p = dir.join("kick_audio_spikes.wav");
        let no_period = export_audio_spikes(&p, sr, &[0.5], &source, Some(&src), WavFormat::Pcm16).unwrap();
        assert_eq!(no_period, 0);
        let (x, info) = read_wav_mono(&p, MixMode::Ffmpeg).unwrap();
        assert_eq!(info.sample_rate, 48_000);
        assert_eq!(info.channels, 1);
        assert_eq!(info.n_frames, 96_000, "the file has the length of the source");
        // a 16-bit comparison: the copy is exact within the quantization (the source hit
        // sample itself is ~1e-14, quantized to exact zero)
        assert!(
            (x[24_000] - source[24_000]).abs() < 2.0 / 32_768.0,
            "the snippet is the source audio"
        );
        let b = info.bext.unwrap();
        assert_eq!(b.description, AUDIO_BEXT_DESCRIPTION);
        assert_eq!(b.time_reference, 123_456);
        assert!((x[24_050] - source[24_050]).abs() < 2.0 / 32_768.0);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn exported_file_reads_back_with_the_source_bext_time_reference() {
        let dir = std::env::temp_dir().join(format!("drumspike-core-spike-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = Bext {
            description: "kick in".into(),
            originator: "DAW".into(),
            originator_reference: "ABC123".into(),
            origination_date: "2026-10-10".into(),
            origination_time: "12:34:56".into(),
            time_reference: 48_000 * 3600 * 10 + 12_345,
            version: 2,
            coding_history: "A=PCM,F=48000,W=24,M=stereo".into(),
        };
        let p = dir.join("kick_spikes.wav");
        export_spikes(&p, 48_000, 96_000, &[0.5, 1.0, 1.5], Some(&src), WavFormat::Pcm16).unwrap();
        let (x, info) = read_wav_mono(&p, MixMode::Ffmpeg).unwrap();
        assert_eq!(info.sample_rate, 48_000);
        assert_eq!(info.channels, 1);
        assert_eq!(info.n_frames, 96_000);
        assert_eq!(x.len(), 96_000);
        // 16-bit full scale is 32767 / 32768
        assert_eq!(x[24_000], 32_767.0 / 32_768.0);
        assert_eq!(x[48_000], 32_767.0 / 32_768.0);
        assert_eq!(x[72_000], 32_767.0 / 32_768.0);
        assert_eq!(x[23_999], 0.0);
        let b = info.bext.unwrap();
        assert_eq!(b.time_reference, src.time_reference);
        assert_eq!(b.origination_date, "2026-10-10");
        assert_eq!(b.origination_time, "12:34:56");
        assert_eq!(b.originator, "DAW");
        assert_eq!(b.originator_reference, "");
        assert_eq!(b.description, BEXT_DESCRIPTION);
        assert_eq!(b.coding_history, "A=PCM,F=48000,W=16,M=mono,T=Drum Spike");
        // float output keeps 1.0 exactly; no bext when the source has none
        let q = dir.join("float_spikes.wav");
        export_spikes(&q, 96_000, 1_000, &[0.005], None, WavFormat::Float32).unwrap();
        let (y, info) = read_wav_mono(&q, MixMode::Ffmpeg).unwrap();
        assert_eq!(y[480], 1.0);
        assert_eq!(info.sample_rate, 96_000);
        assert!(info.bext.is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
