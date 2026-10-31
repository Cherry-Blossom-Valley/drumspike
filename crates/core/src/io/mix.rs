//! How a multichannel file becomes mono.
//!
//! Both modes reproduce what the Python engine gets from ffmpeg 9.0.2 bit for bit, so a
//! project analysed by the Python server and the same project analysed by the Rust core
//! see the same mono signal. The rules were measured on synthetic files (every PCM width,
//! float, 1 to 8 channels) and are re-verified by `tests/ffmpeg_downmix.rs` whenever
//! ffmpeg is installed. ffmpeg on Apple Silicon fuses multiply and add (NEON `fmla`);
//! `f32::mul_add` is a true fused multiply-add on every target, so the Rust result does
//! not depend on the host.
//!
//! `Ffmpeg` (`ffmpeg -ac 1`, the engine's `mix_mode = "ffmpeg"`): every sample is converted
//! to f32 first (`s16 / 32768`, `s24 / 2^23`, `s32 / 2^31`, float as is), then folded with
//! ffmpeg's default mono matrix for the channel count: stereo `fma(L, c, R * c)` with
//! `c = (1/sqrt 2) as f32`; 2.1 the same with the LFE dropped; 4 to 8 channels an fma chain
//! `v = fma(x_i, coef_i, v)` from 0 over the channels with a nonzero coefficient
//! (front 0.7071, centre 1, LFE 0, back and side 0.5). Nothing clips: correlated full-scale
//! stereo comes out at 1.414.
//!
//! `Average` (`-af pan=mono|c0=g*c0+...` with `g = 1/n` printed with 6 decimals, the
//! engine's `mix_mode = "average"`): the mean of the channels in ffmpeg's arithmetic for
//! the input format. 16-bit input is folded in the integer domain with Q15 coefficients,
//! `((sum s_i * q) + 16384) >> 15` with `q = round(g * 32768)`; 24- and 32-bit input (both
//! int32 inside ffmpeg, 24-bit shifted up by 8) are folded as an f32 chain on the integer
//! values and rounded back to an integer with ties to even; float input is folded as an
//! f32 chain. Two channels use the fused `fma(a, g, b * g)`, more use the chain from 0.
//! The result is never more than half an LSB of the input format away from the exact mean.
//! Mono files are copied unchanged in both modes (`-ac 1`).

/// How a multichannel file becomes mono.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MixMode {
    /// What `ffmpeg -ac 1` does: for stereo `fma(L, c, R * c)` with `c = (1/sqrt 2) as f32`,
    /// +3 dB over the average and above full scale for correlated channels. The mode of
    /// sessions analysed before `MixMode` existed; `engine.load_f32`'s default.
    Ffmpeg,
    /// The mean of the channels, computed as ffmpeg's `pan` filter computes it for the
    /// input format (see the module docs). The default for new projects.
    Average,
}

const C: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// ffmpeg's default mono downmix coefficients for `ch` channels, in channel order.
/// Verified against ffmpeg 9.0.2 for 1 to 8 channels (default layouts: mono, stereo,
/// 2.1, 4.0, 5.0, 5.1, 6.1, 7.1).
pub fn ffmpeg_mono_coefs(ch: usize) -> Option<&'static [f32]> {
    const T: [&[f32]; 8] = [
        &[1.0],
        &[C, C],
        &[C, C, 0.0],
        &[C, C, 1.0, 0.5],
        &[C, C, 1.0, 0.5, 0.5],
        &[C, C, 1.0, 0.0, 0.5, 0.5],
        &[C, C, 1.0, 0.0, 0.5, 0.5, 0.5],
        &[C, C, 1.0, 0.0, 0.5, 0.5, 0.5, 0.5],
    ];
    T.get(ch.wrapping_sub(1)).copied()
}

/// The `WAVE_FORMAT_EXTENSIBLE` channel mask of ffmpeg's default layout for `ch` channels
/// (0 for 1 and 2 channels; for stereo see [`ffmpeg_stereo_mask_ok`]).
pub fn ffmpeg_default_mask(ch: usize) -> u32 {
    match ch {
        3 => 0x00B,
        4 => 0x107,
        5 => 0x037,
        6 => 0x03F,
        7 => 0x70F,
        8 => 0x63F,
        _ => 0,
    }
}

/// The speaker bits of a `WAVE_FORMAT_EXTENSIBLE` channel mask (`SPEAKER_FRONT_LEFT` to
/// `SPEAKER_TOP_BACK_RIGHT`); bit 31 (`SPEAKER_ALL`) and the reserved bits are not positions.
const SPEAKER_BITS: u32 = 0x3_FFFF;

/// True when `ffmpeg -ac 1` folds a 2-channel file with this mask as plain stereo, the
/// rule [`MixMode::Ffmpeg`] implements. Measured on ffmpeg 9.0.2: a mask with exactly two
/// speaker bits names the layout, and every such layout other than FL+FR (0x3) either
/// selects another matrix (0x0C, FC+LFE: ffmpeg keeps FC alone) or makes ffmpeg fail
/// ("Input channel layout ... is not supported": 0x5, 0x6, 0x9, 0xA, 0x30, 0x600). A mask
/// with fewer or more than two speaker bits (0, a single bit, 0x7, 0x103, 0x80000000) does
/// not match the channel count, ffmpeg falls back to the default stereo layout, and the
/// result is plain stereo.
pub fn ffmpeg_stereo_mask_ok(channel_mask: u32) -> bool {
    (channel_mask & SPEAKER_BITS).count_ones() != 2 || channel_mask & SPEAKER_BITS == 0x3
}

/// The `pan` gain the engine prints for `ch` channels (`f"{1/ch:.6f}"`), parsed the way
/// ffmpeg parses it: to f64, then narrowed to f32 for the float path.
fn pan_gain(ch: usize) -> f64 {
    format!("{:.6}", 1.0 / ch as f64).parse::<f64>().expect("decimal gain")
}

/// A prepared downmix for one file: the mode, the channel count and the coefficients.
#[derive(Debug, Clone)]
pub struct Plan {
    mode: MixMode,
    ch: usize,
    /// `Ffmpeg`: the channels with a nonzero coefficient, `(index, coefficient)`.
    taps: Vec<(usize, f32)>,
    /// `Average`: the float gain.
    g: f32,
    /// `Average`: the Q15 gain for 16-bit input.
    q: i32,
}

impl Plan {
    /// Build the plan for `ch` channels; `channel_mask` is the extensible header's mask (0
    /// when absent). `Ffmpeg` is defined for 1 to 8 channels with ffmpeg's default layout;
    /// a different mask on more than two channels, or a stereo mask that names a layout
    /// other than FL+FR ([`ffmpeg_stereo_mask_ok`]), would select another matrix in ffmpeg
    /// or make it fail, so it is an error instead of a silent mismatch (the binding then
    /// hands the file to ffmpeg). `Average` takes any channel count and mask.
    pub fn new(mode: MixMode, ch: usize, channel_mask: u32) -> Result<Plan, String> {
        if ch == 0 {
            return Err("zero channels".into());
        }
        let mut plan = Plan {
            mode,
            ch,
            taps: Vec::new(),
            g: 1.0,
            q: 32768,
        };
        match mode {
            MixMode::Ffmpeg => {
                let coefs = ffmpeg_mono_coefs(ch)
                    .ok_or_else(|| format!("ffmpeg downmix is verified for 1 to 8 channels, file has {ch}"))?;
                if ch > 2 && channel_mask != 0 && channel_mask != ffmpeg_default_mask(ch) {
                    return Err(format!(
                        "ffmpeg downmix for channel mask {channel_mask:#x} with {ch} channels is not verified (default is {:#x})",
                        ffmpeg_default_mask(ch)
                    ));
                }
                if ch == 2 && !ffmpeg_stereo_mask_ok(channel_mask) {
                    return Err(format!(
                        "ffmpeg downmix for channel mask {channel_mask:#x} with 2 channels is not verified (only FL+FR, 0x3)"
                    ));
                }
                plan.taps = coefs
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| **c != 0.0)
                    .map(|(i, c)| (i, *c))
                    .collect();
            }
            MixMode::Average => {
                let g = pan_gain(ch);
                plan.g = g as f32;
                plan.q = ((g * 32768.0) as f32).round_ties_even() as i32;
            }
        }
        Ok(plan)
    }

    /// The mode this plan folds with.
    pub fn mode(&self) -> MixMode {
        self.mode
    }

    /// Channel count of the input frames.
    pub fn channels(&self) -> usize {
        self.ch
    }

    /// `ffmpeg -ac 1` on a frame already converted to f32.
    #[inline]
    fn ffmpeg_fold(&self, frame: &[f32]) -> f32 {
        match self.taps.as_slice() {
            [(i, c)] => frame[*i] * c,
            [(i, ci), (j, cj)] => frame[*i].mul_add(*ci, frame[*j] * cj),
            taps => taps.iter().fold(0.0f32, |v, (i, c)| frame[*i].mul_add(*c, v)),
        }
    }

    /// `pan` on float samples: `fma(a, g, b * g)` for two channels, the fma chain otherwise.
    #[inline]
    fn average_fold_f32(&self, frame: &[f32]) -> f32 {
        if frame.len() == 2 {
            frame[0].mul_add(self.g, frame[1] * self.g)
        } else {
            frame.iter().fold(0.0f32, |v, x| x.mul_add(self.g, v))
        }
    }

    /// Fold one frame of float samples (IEEE float input).
    #[inline]
    pub fn fold_f32(&self, frame: &[f32]) -> f32 {
        debug_assert_eq!(frame.len(), self.ch);
        if self.ch == 1 {
            return frame[0];
        }
        match self.mode {
            MixMode::Ffmpeg => self.ffmpeg_fold(frame),
            MixMode::Average => self.average_fold_f32(frame),
        }
    }

    /// Fold one frame of 16-bit samples.
    #[inline]
    pub fn fold_s16(&self, frame: &[i16]) -> f32 {
        debug_assert_eq!(frame.len(), self.ch);
        const SCALE: f32 = 1.0 / 32768.0;
        if self.ch == 1 {
            return f32::from(frame[0]) * SCALE;
        }
        match self.mode {
            MixMode::Ffmpeg => {
                let mut f = [0.0f32; 8];
                for (o, s) in f.iter_mut().zip(frame) {
                    *o = f32::from(*s) * SCALE;
                }
                self.ffmpeg_fold(&f[..frame.len()])
            }
            MixMode::Average => {
                let sum: i64 = frame.iter().map(|&s| i64::from(s) * i64::from(self.q)).sum();
                // ffmpeg stores the Q15 result in an int16 (wraps like C on overflow)
                f32::from(((sum + 16384) >> 15) as i16) * SCALE
            }
        }
    }

    /// Fold one frame of 32-bit integer samples; 24-bit input is passed shifted up by 8,
    /// which is how ffmpeg holds it.
    #[inline]
    pub fn fold_s32(&self, frame: &[i32]) -> f32 {
        debug_assert_eq!(frame.len(), self.ch);
        const SCALE: f32 = 1.0 / 2_147_483_648.0;
        if self.ch == 1 {
            return frame[0] as f32 * SCALE;
        }
        match self.mode {
            MixMode::Ffmpeg => {
                let mut f = [0.0f32; 8];
                for (o, s) in f.iter_mut().zip(frame) {
                    *o = *s as f32 * SCALE;
                }
                self.ffmpeg_fold(&f[..frame.len()])
            }
            MixMode::Average => {
                let v = if frame.len() == 2 {
                    (frame[0] as f32).mul_add(self.g, frame[1] as f32 * self.g)
                } else {
                    frame.iter().fold(0.0f32, |v, &x| (x as f32).mul_add(self.g, v))
                };
                // lrintf: ties to even, back into the int32 sample
                (v.round_ties_even() as i32) as f32 * SCALE
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ffmpeg_stereo_sums_at_minus_3_db_and_may_exceed_full_scale() {
        let p = Plan::new(MixMode::Ffmpeg, 2, 0).unwrap();
        assert_eq!(p.fold_f32(&[1.0, 1.0]), 1.0f32.mul_add(C, C));
        assert!(p.fold_f32(&[1.0, 1.0]) > 1.0);
        assert_eq!(p.fold_f32(&[0.5, -0.5]), 0.0);
        assert_eq!(
            p.fold_s16(&[32767, 32767]),
            (32767.0f32 / 32768.0).mul_add(C, (32767.0f32 / 32768.0) * C)
        );
        assert_eq!(p.fold_s32(&[i32::MIN, 0]), (-1.0f32).mul_add(C, 0.0));
        assert_eq!(p.mode(), MixMode::Ffmpeg);
        assert_eq!(p.channels(), 2);
    }

    #[test]
    fn ffmpeg_layouts_drop_lfe_and_chain_the_rest() {
        let p3 = Plan::new(MixMode::Ffmpeg, 3, 0).unwrap();
        assert_eq!(
            p3.fold_f32(&[0.5, 0.25, 1.0]),
            0.5f32.mul_add(C, 0.25 * C),
            "2.1: LFE dropped"
        );
        let p6 = Plan::new(MixMode::Ffmpeg, 6, 0x3F).unwrap();
        let f = [0.1f32, 0.2, 0.3, 0.9, 0.4, 0.5];
        let want = [(0usize, C), (1, C), (2, 1.0), (4, 0.5), (5, 0.5)]
            .iter()
            .fold(0.0f32, |v, (i, c)| f[*i].mul_add(*c, v));
        assert_eq!(p6.fold_f32(&f), want);
        assert!(Plan::new(MixMode::Ffmpeg, 9, 0).is_err());
        assert!(
            Plan::new(MixMode::Ffmpeg, 4, 0x33).is_err(),
            "quad mask is not the default 4.0 layout"
        );
        for mask in [0x0C, 0x600, 0x5, 0x6, 0x9, 0xA, 0x30] {
            // two speaker bits other than FL+FR: ffmpeg keeps FC alone (0x0C) or fails
            assert!(Plan::new(MixMode::Ffmpeg, 2, mask).is_err(), "{mask:#x}");
            assert!(Plan::new(MixMode::Average, 2, mask).is_ok(), "pan ignores the layout");
        }
        for mask in [0, 0x3, 0x4, 0x7, 0x103, 0x8000_0000, 0x8000_0003] {
            // FL+FR, or a mask that does not match two channels: plain stereo
            assert!(Plan::new(MixMode::Ffmpeg, 2, mask).is_ok(), "{mask:#x}");
        }
        assert!(Plan::new(MixMode::Average, 4, 0x33).is_ok(), "pan ignores the layout");
        assert!(Plan::new(MixMode::Average, 0, 0).is_err());
        assert_eq!(ffmpeg_mono_coefs(0), None);
        assert_eq!(ffmpeg_default_mask(2), 0);
    }

    #[test]
    fn average_mono_is_identity_and_stereo_is_the_mean() {
        let m = Plan::new(MixMode::Average, 1, 0).unwrap();
        assert_eq!(m.fold_s16(&[-32768]), -1.0);
        assert_eq!(m.fold_f32(&[0.3]), 0.3);
        let p = Plan::new(MixMode::Average, 2, 0).unwrap();
        assert_eq!(p.fold_f32(&[0.25, 0.75]), 0.5);
        assert_eq!(p.fold_s32(&[1 << 30, 1 << 30]), 0.5);
    }

    #[test]
    fn average_16_bit_rounds_half_up_in_the_integer_domain() {
        // ffmpeg's pan on s16: (L + R + 1) >> 1, so 1 and 0 average to 1, not 0.5
        let p = Plan::new(MixMode::Average, 2, 0).unwrap();
        assert_eq!(p.fold_s16(&[1, 0]), 1.0 / 32768.0);
        assert_eq!(p.fold_s16(&[-1, 0]), 0.0);
        assert_eq!(p.fold_s16(&[32767, 32767]), 32767.0 / 32768.0);
        // 3 channels: q = round(0.333333 * 32768) = 10923
        let p3 = Plan::new(MixMode::Average, 3, 0).unwrap();
        assert_eq!(p3.q, 10923);
        let s = [1000i16, -2000, 3000];
        let want = ((1000i64 - 2000 + 3000) * 10923 + 16384) >> 15;
        assert_eq!(p3.fold_s16(&s), want as f32 / 32768.0);
    }

    #[test]
    fn average_24_bit_chains_in_float_and_rounds_ties_to_even() {
        let p3 = Plan::new(MixMode::Average, 3, 0).unwrap();
        assert_eq!(p3.g, 0.333_333_f64 as f32);
        let s: [i32; 3] = [1 << 8, 2 << 8, 4 << 8]; // 24-bit values 1, 2, 4 shifted
        let g = p3.g;
        let v = (s[2] as f32).mul_add(g, (s[1] as f32).mul_add(g, (s[0] as f32).mul_add(g, 0.0)));
        assert_eq!(p3.fold_s32(&s), (v.round_ties_even() as i32) as f32 / 2_147_483_648.0);
        // the exact mean is 2.333 LSB, ffmpeg lands on 2 LSB: half an LSB away at most
        assert!((p3.fold_s32(&s) - 7.0 / 3.0 / 8_388_608.0).abs() < 0.5 / 8_388_608.0);
    }

    #[test]
    fn pan_gain_prints_like_the_engine() {
        assert_eq!(pan_gain(2), 0.5);
        assert_eq!(pan_gain(3), 0.333_333);
        assert_eq!(pan_gain(6), 0.166_667);
        assert_eq!(pan_gain(7), 0.142_857);
    }
}
