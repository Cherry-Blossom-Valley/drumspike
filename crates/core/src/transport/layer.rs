//! P4-7a: the one-shot layer voices of a track in the transport (the click path generalised).
//!
//! A track with a layer has a [`LayerVoices`]: the one-shot at the device rate (and at twice the
//! device rate for half speed, so it plays one octave down like the tracks) and the start frame and
//! gain of every voice. A voice starts at its frame, sample-exact, and plays the whole one-shot; a
//! hit that comes before the voice before it ends starts a new voice and the two add (overlap, no
//! cut). The voices go through the track's mute and solo and its volume and pan. The control side
//! builds the voices (never the callback) and hands them over in a [`LayerBank`] like the click banks.

use std::sync::Arc;

use super::Speed;

/// The layer voices of every track (`None`: no layer on that track).
pub type LayerBank = Vec<Option<LayerVoices>>;

/// One track's layer: the one-shot and the voices.
#[derive(Debug, Clone, PartialEq)]
pub struct LayerVoices {
    /// Channels of the one-shot (1 or 2).
    pub channels: usize,
    /// The one-shot at the device rate, interleaved.
    pub tpl: Arc<Vec<f32>>,
    /// The one-shot at twice the device rate (half speed), interleaved.
    pub tpl_half: Arc<Vec<f32>>,
    /// The output frame of each voice's first sample at normal speed and its gain, sorted by frame.
    pub starts: Vec<(i64, f32)>,
    /// The same at half speed (frames at twice the device rate).
    pub starts_half: Vec<(i64, f32)>,
}

impl LayerVoices {
    /// Voices from the start times in seconds (the time of the one-shot's first sample, which is the
    /// hit time minus the one-shot's onset) and their gains. `tpl` and `tpl_half` are the one-shot at
    /// `device_rate` and at twice it, interleaved with `channels` (1 or 2).
    #[must_use]
    pub fn new(
        channels: usize,
        tpl: Arc<Vec<f32>>,
        tpl_half: Arc<Vec<f32>>,
        starts_s: &[(f64, f32)],
        device_rate: u32,
    ) -> LayerVoices {
        let at = |rate: f64| {
            let mut v: Vec<(i64, f32)> = starts_s
                .iter()
                .filter(|(t, g)| t.is_finite() && *g != 0.0)
                .map(|(t, g)| ((t * rate).round() as i64, *g))
                .collect();
            v.sort_by_key(|s| s.0);
            v
        };
        let r = f64::from(device_rate);
        LayerVoices {
            channels: channels.clamp(1, 2),
            tpl,
            tpl_half,
            starts: at(r),
            starts_half: at(2.0 * r),
        }
    }

    /// Add the voices that sound in the block `frame0 .. frame0 + left.len()` (output frames) to
    /// `left` and `right`, the left channel with `gl` and the right with `gr` (a mono one-shot goes to
    /// both). Returns the voices that start in the block and the frame of the last of them. No
    /// allocation; the audio callback calls it.
    pub fn add_block(
        &self,
        left: &mut [f32],
        right: &mut [f32],
        frame0: u64,
        gl: f32,
        gr: f32,
        speed: Speed,
    ) -> (u64, Option<i64>) {
        let (tpl, starts) = match speed {
            Speed::Normal => (&self.tpl, &self.starts),
            Speed::Half => (&self.tpl_half, &self.starts_half),
        };
        let k = self.channels;
        let len = (tpl.len() / k) as i64;
        if len == 0 || left.is_empty() {
            return (0, None);
        }
        let f0 = frame0 as i64;
        let end = f0 + left.len() as i64;
        // the voices whose span [s, s + len) meets the block
        let first = starts.partition_point(|s| s.0 + len <= f0);
        let (mut started, mut last) = (0u64, None);
        for &(s, g) in &starts[first..] {
            if s >= end {
                break;
            }
            if s >= f0 {
                started += 1;
                last = Some(s);
            }
            let a = s.max(f0);
            let b = (s + len).min(end);
            let (gl, gr) = (g * gl, g * gr);
            for f in a..b {
                let o = (f - f0) as usize;
                let i = (f - s) as usize * k;
                if k == 2 {
                    left[o] += gl * tpl[i];
                    right[o] += gr * tpl[i + 1];
                } else {
                    left[o] += gl * tpl[i];
                    right[o] += gr * tpl[i];
                }
            }
        }
        (started, last)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn voices(starts: &[(f64, f32)], ch: usize) -> LayerVoices {
        let tpl: Vec<f32> = (0..10 * ch).map(|i| (i + 1) as f32).collect();
        let half: Vec<f32> = (0..20 * ch).map(|i| -((i + 1) as f32)).collect();
        LayerVoices::new(ch, Arc::new(tpl), Arc::new(half), starts, 1_000)
    }

    #[test]
    fn new_maps_the_start_times_to_frames_at_both_speeds() {
        let v = voices(&[(0.0104, 1.0), (0.002, 0.5), (f64::NAN, 1.0), (0.5, 0.0)], 1);
        assert_eq!(
            v.starts,
            vec![(2, 0.5), (10, 1.0)],
            "sorted, rounded; NaN and silent dropped"
        );
        assert_eq!(v.starts_half, vec![(4, 0.5), (21, 1.0)]);
        assert_eq!(voices(&[], 3).channels, 2);
    }

    #[test]
    fn voices_start_sample_exact_overlap_and_cross_blocks() {
        let v = voices(&[(0.003, 1.0), (0.008, 0.5), (-0.004, 1.0)], 1);
        let n = 40usize;
        let (mut wl, mut wr) = (vec![0.0f32; n], vec![0.0f32; n]);
        let (started, last) = v.add_block(&mut wl, &mut wr, 0, 1.0, 2.0, Speed::Normal);
        assert_eq!(
            (started, last),
            (2, Some(8)),
            "the voice before the block is not counted"
        );
        // the voice that started at -4: its samples 4..10 at frames 0..6
        assert_eq!(wl[0], 5.0 + 0.0);
        assert_eq!(wl[3], 8.0 + 1.0);
        assert_eq!(wl[8], 6.0 + 0.5 * 1.0, "overlap adds, no cut");
        assert_eq!(wl[12], 10.0 + 0.5 * 5.0);
        assert_eq!(wl[13], 0.5 * 6.0);
        assert_eq!(wr[8], 2.0 * wl[8], "mono goes to both sides with their gains");
        assert!(wl[18..].iter().all(|x| *x == 0.0));
        // the same in blocks of 7
        let (mut bl, mut br) = (vec![0.0f32; n], vec![0.0f32; n]);
        let mut count = 0;
        for k in 0..n.div_ceil(7) {
            let (a, b) = (k * 7, ((k + 1) * 7).min(n));
            count += v
                .add_block(&mut bl[a..b], &mut br[a..b], a as u64, 1.0, 2.0, Speed::Normal)
                .0;
        }
        assert_eq!((bl, br, count), (wl, wr, 2));
    }

    #[test]
    fn stereo_and_half_speed() {
        let v = voices(&[(0.002, 1.0)], 2);
        let (mut l, mut r) = (vec![0.0f32; 30], vec![0.0f32; 30]);
        v.add_block(&mut l, &mut r, 0, 1.0, 1.0, Speed::Normal);
        assert_eq!((l[2], r[2], l[3], r[3]), (1.0, 2.0, 3.0, 4.0));
        let (mut l, mut r) = (vec![0.0f32; 30], vec![0.0f32; 30]);
        let (n, last) = v.add_block(&mut l, &mut r, 0, 1.0, 1.0, Speed::Half);
        assert_eq!((n, last), (1, Some(4)));
        assert_eq!((l[4], r[4]), (-1.0, -2.0), "the half-speed one-shot at twice the frame");
        let (mut l, mut r) = (vec![0.0f32; 0], vec![0.0f32; 0]);
        assert_eq!(v.add_block(&mut l, &mut r, 0, 1.0, 1.0, Speed::Normal), (0, None));
    }
}
