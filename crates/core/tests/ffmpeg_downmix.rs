//! The downmix rules of `io::mix` against ffmpeg's mono decode, bit for bit.
//!
//! For every sample format (PCM 16/24/32, float) and channel count (1, 2, 3, 4, 6, 8) a synthetic file
//! with independent pseudo-random channels is decoded by the crate and compared with what ffmpeg gave for
//! the same file as the retired Python engine called it (`-ac 1` for `MixMode::Ffmpeg`, `-af pan=mono|...`
//! for `MixMode::Average`, output `f32le`). P3-12: ffmpeg's outputs are frozen in
//! `tests/golden/frozen/ffmpeg_downmix.json` and `ffmpeg_masks.json` (frame count and SHA-256 of the
//! `f32le` bytes), recorded on arm64-mac with the fusing ffmpeg 9.0.2 at the commit named there. The reader
//! uses `f32::mul_add` and equals that build on every host, so the comparison is exact on `x86_64` too
//! (before P3-12 the x86 run skipped it: its ffmpeg does not fuse, finding X1). No ffmpeg is needed.

use std::path::PathBuf;

use serde_json::Value;
use sha2::{Digest, Sha256};

use drumspike_core::io::{
    MixMode, WavFormat, WavSpec, encode_wav_raw, ffmpeg_default_mask, ffmpeg_stereo_mask_ok, read_wav_mono,
};

struct Lcg(u64);
impl Lcg {
    fn next_u32(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 32) as u32
    }
}

fn f32_sha256(x: &[f32]) -> String {
    let mut h = Sha256::new();
    for v in x {
        h.update(v.to_le_bytes());
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// The `cases` of a frozen ffmpeg file.
fn frozen(name: &str) -> serde_json::Map<String, Value> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/golden/frozen")
        .join(format!("{name}.json"));
    let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    v["cases"].as_object().cloned().expect("cases")
}

/// Random full-scale sample bytes, with the extremes planted in the first frames.
fn random_data(fmt: WavFormat, channels: usize, frames: usize, rng: &mut Lcg) -> Vec<u8> {
    let mut d = Vec::with_capacity(frames * channels * fmt.bytes_per_sample() as usize);
    for k in 0..frames * channels {
        let r = rng.next_u32();
        match fmt {
            WavFormat::Pcm16 => {
                let v = match k {
                    0 | 1 => i16::MAX,
                    2 | 3 => i16::MIN,
                    _ => r as i16,
                };
                d.extend_from_slice(&v.to_le_bytes());
            }
            WavFormat::Pcm24 => {
                let v = match k {
                    0 | 1 => 0x7F_FFFF,
                    2 | 3 => -0x80_0000,
                    _ => (r as i32) >> 8,
                };
                d.extend_from_slice(&v.to_le_bytes()[..3]);
            }
            WavFormat::Pcm32 => {
                let v = match k {
                    0 | 1 => i32::MAX,
                    2 | 3 => i32::MIN,
                    _ => r as i32,
                };
                d.extend_from_slice(&v.to_le_bytes());
            }
            WavFormat::Float32 => {
                // [-1.2, 1.2): above full scale on purpose, ffmpeg must not clip
                let v = match k {
                    0 | 1 => 1.0f32,
                    2 | 3 => -1.0,
                    _ => (r as f32 / u32::MAX as f32) * 2.4 - 1.2,
                };
                d.extend_from_slice(&v.to_le_bytes());
            }
        }
    }
    d
}

#[test]
fn downmix_matches_the_frozen_ffmpeg_decode_bit_for_bit() {
    let want = frozen("ffmpeg_downmix");
    let dir = std::env::temp_dir().join(format!("drumspike-core-ffmpeg-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut rng = Lcg(0x5eed);
    let mut failures = Vec::new();
    let mut n = 0;
    for fmt in [WavFormat::Pcm16, WavFormat::Pcm24, WavFormat::Pcm32, WavFormat::Float32] {
        for channels in [1usize, 2, 3, 4, 6, 8] {
            let frames = 6000;
            let data = random_data(fmt, channels, frames, &mut rng);
            let spec = WavSpec {
                sample_rate: 48_000,
                channels: channels as u16,
                format: fmt,
                extensible: channels > 2,
                channel_mask: ffmpeg_default_mask(channels),
            };
            let path = dir.join(format!("{fmt:?}_{channels}ch.wav"));
            std::fs::write(&path, encode_wav_raw(&spec, &data, None)).unwrap();
            for mode in [MixMode::Ffmpeg, MixMode::Average] {
                let key = format!("{fmt:?} {channels}ch {mode:?}");
                let w = &want[&key];
                let (got, info) = read_wav_mono(&path, mode).unwrap();
                assert_eq!(info.n_frames, frames);
                assert_eq!(w["frames"], frames, "{key}: ffmpeg frame count");
                if f32_sha256(&got) != w["sha256"].as_str().unwrap() {
                    failures.push(key.clone());
                }
                if mode == MixMode::Ffmpeg && channels == 2 {
                    let peak = got.iter().fold(0.0f32, |m, v| m.max(v.abs()));
                    assert!(
                        peak > 1.0,
                        "{key}: -ac 1 on correlated full-scale stereo exceeds 1.0 (no clipping)"
                    );
                }
                n += 1;
            }
        }
    }
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(n, want.len(), "every frozen case is compared");
    assert!(
        failures.is_empty(),
        "the downmix differs from ffmpeg's frozen decode:\n{}",
        failures.join("\n")
    );
}

/// Stereo `WAVE_FORMAT_EXTENSIBLE` files with every mask of two speaker bits, every single bit and the odd
/// cases: where the Rust `Ffmpeg` decode returns a signal it is ffmpeg's bit for bit; where ffmpeg folded
/// differently or refused the file, the Rust decode is an error (finding F2 of phase 1 stage 3; the app then
/// says the file is not supported).
#[test]
fn stereo_channel_masks_match_the_frozen_ffmpeg_decode_or_refuse() {
    let want = frozen("ffmpeg_masks");
    let dir = std::env::temp_dir().join(format!("drumspike-core-masks-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut masks: Vec<u32> = vec![0, 0x7, 0x103, 0x8000_0000, 0x8000_0003, 0x8000_000C];
    for a in 0..18 {
        masks.push(1 << a);
        for b in a + 1..18 {
            masks.push((1 << a) | (1 << b));
        }
    }
    let mut rng = Lcg(0x3a5c);
    let (mut same, mut refused, mut other_fold, mut n) = (0, 0, 0, 0);
    let mut failures = Vec::new();
    for fmt in [WavFormat::Pcm16, WavFormat::Pcm24, WavFormat::Float32] {
        // the full scan on 24-bit; the cases the review measured on 16-bit and float
        let fmt_masks: Vec<u32> = if fmt == WavFormat::Pcm24 {
            masks.clone()
        } else {
            vec![0, 0x3, 0x4, 0x5, 0x0C, 0x600, 0x103, 0x8000_0000]
        };
        for &mask in &fmt_masks {
            let frames = 2000;
            let data = random_data(fmt, 2, frames, &mut rng);
            let spec = WavSpec {
                sample_rate: 48_000,
                channels: 2,
                format: fmt,
                extensible: true,
                channel_mask: mask,
            };
            let path = dir.join(format!("{fmt:?}_{mask:x}.wav"));
            std::fs::write(&path, encode_wav_raw(&spec, &data, None)).unwrap();
            let w = &want[&format!("{fmt:?} {mask:#x}")];
            let got = read_wav_mono(&path, MixMode::Ffmpeg).ok().map(|(x, _)| x);
            n += 1;
            match (w.is_object(), &got) {
                (true, Some(g)) if f32_sha256(g) == w["sha256"].as_str().unwrap() => same += 1,
                (true, None) if !ffmpeg_stereo_mask_ok(mask) => {
                    let plain = read_wav_mono(&path, MixMode::Average).map(|(x, _)| x.len());
                    assert_eq!(
                        plain.ok(),
                        w["frames"].as_u64().map(|f| f as usize),
                        "{fmt:?} {mask:#x}: Average still decodes"
                    );
                    other_fold += 1;
                }
                (false, None) => refused += 1,
                _ => failures.push(format!(
                    "{fmt:?} mask {mask:#x}: ffmpeg {}, rust {}",
                    if w.is_object() { "decoded" } else { "refused" },
                    if got.is_some() {
                        "decodes (differs or should fall back)"
                    } else {
                        "refuses"
                    }
                )),
            }
        }
    }
    std::fs::remove_dir_all(&dir).unwrap();
    eprintln!(
        "stereo masks: {same} bit-identical, {other_fold} folded differently by ffmpeg (rust refuses), {refused} refused by both"
    );
    assert_eq!(n, want.len(), "every frozen case is compared");
    assert!(failures.is_empty(), "stereo masks:\n{}", failures.join("\n"));
}
