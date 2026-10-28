//! Hit clicks: the WebAudio click of `static/app.js`, rendered at the exact sample.
//!
//! `app.js` plays one click per hit: a square oscillator at the role pitch through a gain
//! that starts at 0.0001, rises exponentially to the click volume in 2 ms, falls
//! exponentially to 0.0001 at 30 ms, and the oscillator stops at 40 ms. The click goes
//! through the track's gain, so mute and solo also silence the clicks of a track.
//!
//! Here the click is a template built before playback (never in the audio callback): a
//! band-limited square (odd harmonics below Nyquist, peak normalised to 1, as WebAudio's
//! normalised periodic wave) in cosine phase, so the first sample of the click is the top
//! of the square and the click starts at offset 0 at its hit sample. The value is close to
//! the WebAudio click, not equal to it sample for sample.
//!
//! P3-20b: at half speed the output frame of session time `t` is `t * 2 * device_rate`, so a
//! hit's click goes to `round(hit * 2 * device_rate / track_rate)` (the same mapping at twice the
//! device rate) and the clicks come at half the rate. The click itself is the same template at
//! the device rate: its pitch and length do not change.

use std::f64::consts::PI;

use super::Speed;

/// The click pitch per role in Hz (`ROLE_DEFAULT_CLICK` in `static/app.js`).
pub const ROLE_DEFAULT_CLICK: [(&str, f32); 6] = [
    ("kick", 1000.0),
    ("snare", 2500.0),
    ("tom", 700.0),
    ("hat", 4000.0),
    ("cymbal", 3000.0),
    ("other", 1500.0),
];

/// The default click volume of `app.js` (`SETTINGS().click.volume`).
pub const DEFAULT_CLICK_VOLUME: f32 = 0.22;

/// Envelope floor of the WebAudio ramps (`0.0001`).
const FLOOR: f64 = 0.0001;
/// End of the attack ramp, seconds.
const ATTACK_S: f64 = 0.002;
/// End of the decay ramp, seconds.
const DECAY_S: f64 = 0.030;
/// The oscillator stops here, seconds.
const STOP_S: f64 = 0.040;

/// The default click pitch of a role (`ROLE_DEFAULT_CLICK[role] || 1500`).
pub fn role_pitch(role: &str) -> f32 {
    ROLE_DEFAULT_CLICK
        .iter()
        .find(|(r, _)| *r == role)
        .map(|(_, f)| *f)
        .unwrap_or(1500.0)
}

/// The click envelope at `t` seconds for `volume` (the two exponential ramps of `app.js`,
/// then the floor until the stop).
pub fn envelope(t: f64, volume: f32) -> f64 {
    let v = volume as f64;
    if v <= 0.0 || !(0.0..STOP_S).contains(&t) {
        return 0.0;
    }
    if t < ATTACK_S {
        FLOOR * (v / FLOOR).powf(t / ATTACK_S)
    } else if t < DECAY_S {
        v * (FLOOR / v).powf((t - ATTACK_S) / (DECAY_S - ATTACK_S))
    } else {
        FLOOR
    }
}

/// The click template at `sample_rate`: `round(0.040 * sample_rate)` samples, a
/// band-limited square at `pitch_hz` in cosine phase times [`envelope`]. Empty when the
/// volume is 0, the pitch is not positive or the pitch is at or above Nyquist.
pub fn click_template(pitch_hz: f32, volume: f32, sample_rate: u32) -> Vec<f32> {
    let sr = sample_rate as f64;
    let f = pitch_hz as f64;
    if volume <= 0.0 || f <= 0.0 || f >= sr / 2.0 {
        return Vec::new();
    }
    let n = (STOP_S * sr).round() as usize;
    // square = 4/pi * sum over odd k of (-1)^((k-1)/2) cos(2 pi k f t) / k, k f < sr / 2
    let harmonics: Vec<(f64, f64)> = (0..)
        .map(|i| 2 * i + 1)
        .take_while(|k| (*k as f64) * f < sr / 2.0)
        .map(|k| {
            let sign = if (k / 2) % 2 == 0 { 1.0 } else { -1.0 };
            (k as f64, sign * 4.0 / (PI * k as f64))
        })
        .collect();
    let osc = |t: f64| -> f64 { harmonics.iter().map(|(k, a)| a * (2.0 * PI * k * f * t).cos()).sum() };
    // normalise the peak of the continuous wave to 1 (WebAudio normalises its periodic
    // waves): the maximum over one period on a fine grid
    let peak = (0..4096)
        .map(|i| osc(i as f64 / (4096.0 * f)).abs())
        .fold(0.0f64, f64::max);
    let norm = if peak > 0.0 { 1.0 / peak } else { 0.0 };
    (0..n)
        .map(|i| {
            let t = i as f64 / sr;
            (osc(t) * norm * envelope(t, volume)) as f32
        })
        .collect()
}

/// Map a hit sample at the track rate to the device sample:
/// `round(hit * device_rate / track_rate)`, ties to even, in exact integer arithmetic.
pub fn map_hit(hit: u64, track_rate: u32, device_rate: u32) -> u64 {
    if track_rate == device_rate {
        return hit;
    }
    let num = hit as u128 * device_rate as u128;
    let den = track_rate as u128;
    let q = num / den;
    let r = num % den;
    let up = match (2 * r).cmp(&den) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => q % 2 == 1,
    };
    (q + up as u128) as u64
}

/// The clicks of one track at the device rate: sorted onset frames and the template.
#[derive(Debug, Clone, Default)]
pub struct TrackClicks {
    /// Click onset frames at the device rate, sorted ascending.
    pub onsets: Vec<u64>,
    /// P3-20b: the onset output frames at half speed (the hits mapped to twice the device rate), sorted.
    pub onsets_half: Vec<u64>,
    /// The click samples (empty: no clicks for this track).
    pub template: Vec<f32>,
}

/// The sorted output frames of `hits` at `out_rate`.
fn mapped(hits: &[u64], track_rate: u32, out_rate: u32) -> Vec<u64> {
    let mut v: Vec<u64> = hits.iter().map(|&h| map_hit(h, track_rate, out_rate)).collect();
    v.sort_unstable();
    v
}

impl TrackClicks {
    /// Build from hit samples at `track_rate` (any order; mapped with [`map_hit`], sorted,
    /// duplicates kept) and the click sound. The onsets of both speeds are built here, on the
    /// control side, so a speed change needs no new bank.
    pub fn new(hits: &[u64], track_rate: u32, device_rate: u32, pitch_hz: f32, volume: f32) -> TrackClicks {
        TrackClicks {
            onsets: mapped(hits, track_rate, device_rate),
            onsets_half: mapped(hits, track_rate, Speed::Half.render_rate(device_rate)),
            template: click_template(pitch_hz, volume, device_rate),
        }
    }

    /// Add `gain` times the clicks that overlap the block `frame0 .. frame0 + out.len()` to
    /// `out` (mono). No allocation; used by the audio callback.
    pub fn add_block(&self, out: &mut [f32], frame0: u64, gain: f32) {
        self.add_block_at(out, frame0, gain, Speed::Normal)
    }

    /// P3-20b: [`TrackClicks::add_block`] at `speed`; `frame0` counts output frames
    /// ([`TrackClicks::onsets_half`] at half speed).
    pub fn add_block_at(&self, out: &mut [f32], frame0: u64, gain: f32, speed: Speed) {
        let len = self.template.len() as u64;
        if len == 0 || out.is_empty() {
            return;
        }
        let onsets = match speed {
            Speed::Normal => &self.onsets,
            Speed::Half => &self.onsets_half,
        };
        let end = frame0 + out.len() as u64;
        let lo = frame0.saturating_sub(len - 1);
        let first = onsets.partition_point(|&c| c < lo);
        for &c in &onsets[first..] {
            if c >= end {
                break;
            }
            // overlap of [c, c + len) with [frame0, end)
            let a = c.max(frame0);
            let b = (c + len).min(end);
            let tpl = &self.template[(a - c) as usize..(b - c) as usize];
            let dst = &mut out[(a - frame0) as usize..(b - frame0) as usize];
            for (o, s) in dst.iter_mut().zip(tpl) {
                *o += gain * s;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_pitch_follows_app_js() {
        assert_eq!(role_pitch("kick"), 1000.0);
        assert_eq!(role_pitch("hat"), 4000.0);
        assert_eq!(role_pitch("cowbell"), 1500.0);
    }

    #[test]
    fn envelope_has_the_two_ramps_and_the_floor() {
        assert!((envelope(0.0, 0.22) - 0.0001).abs() < 1e-12);
        assert!((envelope(0.002, 0.22) - 0.22).abs() < 1e-6);
        assert!(
            (envelope(0.001, 0.22) - (0.0001f64 * 0.22).sqrt()).abs() < 1e-9,
            "geometric midpoint"
        );
        assert!((envelope(0.0299999, 0.22) - 0.0001).abs() < 1e-6);
        assert_eq!(envelope(0.035, 0.22), 0.0001);
        assert_eq!(envelope(0.040, 0.22), 0.0);
        assert_eq!(envelope(-0.001, 0.22), 0.0);
        assert_eq!(envelope(0.01, 0.0), 0.0);
    }

    #[test]
    fn click_template_starts_at_offset_0_and_is_band_limited() {
        for sr in [44_100u32, 48_000, 96_000] {
            for (_, f) in ROLE_DEFAULT_CLICK {
                let t = click_template(f, 0.22, sr);
                assert_eq!(t.len(), (0.04 * sr as f64).round() as usize);
                assert!(t[0] > 0.0, "the first sample is the top of the square");
                // the floor 0.0001 times the square at its plateau centre (1 / Gibbs peak)
                assert!(t[0] > 0.00005 && t[0] <= 0.0001, "starts at the floor: {}", t[0]);
                let peak = t.iter().fold(0.0f32, |m, x| m.max(x.abs()));
                assert!(peak <= 0.22 + 1e-6 && peak > 0.15, "{sr} {f}: peak {peak}");
            }
        }
        assert!(click_template(1000.0, 0.0, 48_000).is_empty());
        assert!(click_template(30_000.0, 0.22, 48_000).is_empty());
        assert!(click_template(-5.0, 0.22, 48_000).is_empty());
    }

    #[test]
    fn map_hit_rounds_to_the_device_sample() {
        assert_eq!(map_hit(12_345, 48_000, 48_000), 12_345);
        assert_eq!(map_hit(96_000, 96_000, 48_000), 48_000);
        assert_eq!(map_hit(3, 96_000, 48_000), 2, "1.5 ties to even");
        assert_eq!(map_hit(5, 96_000, 48_000), 2, "2.5 ties to even");
        assert_eq!(map_hit(7, 96_000, 48_000), 4, "3.5 ties to even");
        assert_eq!(map_hit(44_100, 44_100, 48_000), 48_000);
        assert_eq!(map_hit(1, 44_100, 48_000), 1, "1.088 rounds down");
        assert_eq!(map_hit(7, 44_100, 48_000), 8, "7.619 rounds up");
        assert_eq!(
            map_hit(300 * 44_100 + 147, 44_100, 48_000),
            300 * 48_000 + 160,
            "no overflow, exact grid"
        );
    }

    #[test]
    fn add_block_places_every_click_at_its_onset_across_block_edges() {
        let c = TrackClicks::new(&[10, 300, 301, 2_300], 48_000, 48_000, 1000.0, 0.22);
        let n = 4_000usize;
        let mut whole = vec![0.0f32; n];
        c.add_block(&mut whole, 0, 1.0);
        let mut blocks = vec![0.0f32; n];
        for (k, chunk) in blocks.chunks_mut(256).enumerate() {
            c.add_block(chunk, k as u64 * 256, 1.0);
        }
        assert_eq!(whole, blocks);
        assert!(whole[..10].iter().all(|x| *x == 0.0));
        assert_eq!(whole[10], c.template[0]);
        assert_eq!(whole[299], c.template[289]);
        assert_eq!(whole[300], c.template[290] + c.template[0]);
        assert_eq!(whole[2_300], c.template[0]);
        let mut muted = vec![0.0f32; n];
        c.add_block(&mut muted, 0, 0.0);
        assert!(muted.iter().all(|x| *x == 0.0));
        let empty = TrackClicks::default();
        empty.add_block(&mut muted, 0, 1.0);
        assert!(muted.iter().all(|x| *x == 0.0));
    }

    #[test]
    fn resampled_hits_map_to_device_onsets() {
        let c = TrackClicks::new(&[96_001, 3], 96_000, 48_000, 2500.0, 0.22);
        assert_eq!(c.onsets, vec![2, 48_000]);
        assert_eq!(
            c.onsets_half,
            vec![3, 96_001],
            "half speed: the 96 kHz hits are the output frames"
        );
        assert_eq!(c.template.len(), 1_920);
    }

    /// P3-20b: at half speed the onsets are twice as far apart and the click is the same template.
    #[test]
    fn half_speed_clicks_sit_at_twice_the_frame_with_the_same_template() {
        // 3,000 frames apart: no two clicks (1,920 frames) overlap at either speed
        let c = TrackClicks::new(&[10, 3_000, 9_000], 48_000, 48_000, 1000.0, 0.22);
        assert_eq!(c.onsets_half, vec![20, 6_000, 18_000]);
        let n = 20_000usize;
        let mut half = vec![0.0f32; n];
        for (k, chunk) in half.chunks_mut(256).enumerate() {
            c.add_block_at(chunk, k as u64 * 256, 1.0, Speed::Half);
        }
        assert!(half[..20].iter().all(|x| *x == 0.0));
        assert_eq!(half[20], c.template[0]);
        assert_eq!(half[6_000 + 100], c.template[100]);
        assert_eq!(half[18_000], c.template[0]);
        assert!(
            half[3_000..5_999].iter().all(|x| *x == 0.0),
            "nothing at the normal-speed onset"
        );
        let mut normal = vec![0.0f32; n];
        c.add_block_at(&mut normal, 0, 1.0, Speed::Normal);
        let mut plain = vec![0.0f32; n];
        c.add_block(&mut plain, 0, 1.0);
        assert_eq!(normal, plain, "Normal is add_block");
        assert_eq!(normal[10], c.template[0]);
    }
}
