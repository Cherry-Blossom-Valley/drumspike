//! P4-7c: the sampler calibration. The engineer's sampler (Slate Trigger, Superior Drummer 3,
//! Drumforge Saviour) plays its sound a little after the MIDI note. The app writes a calibration
//! file with one note on a bar line, the engineer bounces the sampler's kit piece for that note,
//! and the app measures the delay of the bounce's transient and its polarity against the close mic.
//!
//! * [`calibration_time`], [`calibration_midi`]: one note (velocity 100) on the start of bar 2 of the
//!   tempo map (2.000 s with the default map), as the MIDI export writes any note.
//! * [`measure_offset`]: the bounce's onset (the refinement rule of the one-shots,
//!   [`crate::layer::one_shot_onset`]) minus the note-on time, in samples of the bounce (fractional
//!   when the note-on time is not a whole sample).
//! * [`median_shape`] and [`bounce_polarity`]: the polarity of the bounce against the close mic: the
//!   cross-correlation of the bounce's attack with the median shape of the mic's hits.
//! * [`compensate`]: the MIDI time of a note: the aligned attack minus the offset, rounded to the
//!   sample. The sub-sample part of an alignment exists only in the render.

use crate::layer::{AlignParams, attack_template, one_shot_onset, xcorr_hit};
use crate::midi::{MidiError, MidiEvent, midi_bytes_map};
use crate::tempo::{TempoMap, TickMap};

/// The velocity of the calibration note.
pub const CALIBRATION_VELOCITY: i64 = 100;
/// The most hits that go into the median shape (spread over the track).
pub const SHAPE_HITS: usize = 64;

/// The tick and the time (seconds) of the calibration note: the start of bar 2 (bar 1 of the map
/// holds the start meter): 2.000 s with the default map. `ppq` is the file's resolution.
///
/// # Errors
///
/// The message of [`TickMap::new`] when `ppq` or the tempo map is invalid, or a message that the
/// map's meter does not fit a whole number of ticks per bar at this `ppq`.
pub fn calibration_time(map: &TempoMap, ppq: u16) -> Result<(i64, f64), String> {
    let tm = TickMap::new(ppq, map)?;
    let bar = tm.bar_ticks(map.meter.0, map.meter.1).ok_or_else(|| {
        format!(
            "a bar of {}/{} does not fit {ppq} ticks per quarter",
            map.meter.0, map.meter.1
        )
    })?;
    let tick = tm.bar1_tick(map) + bar;
    Ok((tick, tm.time(tick)))
}

/// The calibration MIDI file: one note `note` (velocity 100) at [`calibration_time`], written
/// through the map like every export (`legacy_tempo` is used for the default map only). Returns
/// the bytes and the note's time in seconds.
///
/// # Errors
///
/// [`MidiError::Map`] when [`calibration_time`] fails; otherwise the errors of
/// [`midi_bytes_map`] (for example [`MidiError::Note`] when `note` is outside 0..=127).
pub fn calibration_midi(note: i64, map: &TempoMap, ppq: u16, legacy_tempo: f64) -> Result<(Vec<u8>, f64), MidiError> {
    let (_, t) = calibration_time(map, ppq).map_err(MidiError::Map)?;
    let ev = [MidiEvent {
        t,
        v: CALIBRATION_VELOCITY,
    }];
    Ok((midi_bytes_map(&ev, note, ppq, map, legacy_tempo)?, t))
}

/// What the bounce says about the sampler's delay.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Offset {
    /// The bounce's onset in its samples.
    pub onset: usize,
    /// The bounce's rate.
    pub sr: u32,
    /// The onset minus the note-on time, in samples of the bounce.
    pub offset_samples: f64,
}

/// The delay of the bounce's transient: its onset (the one-shots' rule) minus `note_on_s`. `Err`
/// for a silent bounce.
///
/// # Errors
///
/// An error message when `sr` is 0 or every sample of the bounce is exactly zero ("the bounce is
/// silent").
pub fn measure_offset(bounce: &[f32], sr: u32, note_on_s: f64) -> Result<Offset, String> {
    if sr == 0 || bounce.iter().all(|v| *v == 0.0) {
        return Err("the bounce is silent".to_string());
    }
    let onset = one_shot_onset(bounce, sr);
    Ok(Offset {
        onset,
        sr,
        offset_samples: onset as f64 - note_on_s * f64::from(sr),
    })
}

/// The median shape of a mic's hits: per sample the median over (at most [`SHAPE_HITS`]) hits of the
/// window from `pre + max_lag` samples before the hit to `len + max_lag` after it, each window
/// scaled to a root-mean-square of 1 (silent windows are left out). `None` without a usable hit.
/// The hit sits at index `pre + max_lag` of the result.
pub fn median_shape(mic: &[f32], hits: &[i64], pre: usize, len: usize, max_lag: usize) -> Option<Vec<f32>> {
    let total = pre + len + 2 * max_lag;
    let start = pre + max_lag;
    let step = (hits.len() / SHAPE_HITS).max(1);
    let mut wins: Vec<Vec<f32>> = Vec::new();
    for &h in hits.iter().step_by(step) {
        let w: Vec<f32> = (0..total as i64)
            .map(|k| {
                let i = h - start as i64 + k;
                if i >= 0 && (i as usize) < mic.len() {
                    mic[i as usize]
                } else {
                    0.0
                }
            })
            .collect();
        let rms = (w.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / total as f64).sqrt();
        if rms > 1e-9 {
            wins.push(w.iter().map(|v| (f64::from(*v) / rms) as f32).collect());
        }
    }
    if wins.is_empty() {
        return None;
    }
    Some(
        (0..total)
            .map(|k| {
                let mut c: Vec<f32> = wins.iter().map(|w| w[k]).collect();
                c.sort_by(f32::total_cmp);
                c[c.len() / 2]
            })
            .collect(),
    )
}

/// The polarity of a bounce against the close mic: +1 (normal) or -1 (inverted) and the height of
/// the better correlation (0 to 1). The bounce (mono, at the mic's rate) has its onset at `onset`;
/// its attack ([`AlignParams`]) is correlated with the median shape of the mic's hits `hits`
/// (sample numbers), within the search range of `p`. `None` when no hit has a usable window or the
/// bounce's attack is silent.
pub fn bounce_polarity(
    bounce: &[f32],
    onset: usize,
    mic: &[f32],
    hits: &[i64],
    sr: u32,
    p: &AlignParams,
) -> Option<(i8, f64)> {
    let (a, pre) = attack_template(bounce, onset, sr, p);
    let max_lag = (p.max_lag_s * f64::from(sr)).round() as usize;
    let len = a.len();
    let shape = median_shape(mic, hits, pre, len, max_lag)?;
    let x = xcorr_hit(&shape, (pre + max_lag) as i64, &a, pre, max_lag)?;
    let sign = x.sign();
    Some((sign, x.for_sign(sign).1))
}

/// The MIDI time of a note: the aligned attack `t * sr + lag_samples` minus `offset_samples`, rounded
/// to the sample, in seconds. A note that would fall before the first sample goes to 0 s (the second
/// value is true). `sr` is the rate of the track the samples are of.
pub fn compensate(t: f64, lag_samples: f64, offset_samples: f64, sr: u32) -> (f64, bool) {
    let s = (t * f64::from(sr) + lag_samples - offset_samples).round();
    if s < 0.0 {
        (0.0, true)
    } else {
        (s / f64::from(sr), false)
    }
}

#[cfg(test)]
mod tests;
