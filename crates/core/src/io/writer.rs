//! WAV writer: test data now, the spike WAV export in P1-5 (with the source's `bext`).

use std::path::Path;

use super::reader::{
    Bext, BEXT_DATE, BEXT_DESCRIPTION, BEXT_FIXED_LEN, BEXT_ORIGINATOR, BEXT_ORIGINATOR_REFERENCE, BEXT_TIME,
    BEXT_TIME_REFERENCE, BEXT_VERSION, SUBFORMAT_TAIL,
};

/// Sample encoding of a written file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WavFormat {
    /// 16-bit PCM (format tag 1).
    Pcm16,
    /// 24-bit PCM (format tag 1).
    Pcm24,
    /// 32-bit PCM (format tag 1).
    Pcm32,
    /// IEEE float 32 (format tag 3).
    Float32,
}

impl WavFormat {
    /// Container bytes per sample.
    pub fn bytes_per_sample(self) -> u16 {
        match self {
            WavFormat::Pcm16 => 2,
            WavFormat::Pcm24 => 3,
            WavFormat::Pcm32 | WavFormat::Float32 => 4,
        }
    }
    fn tag(self) -> u16 {
        match self {
            WavFormat::Float32 => 3,
            _ => 1,
        }
    }
}

/// Header parameters of a written file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WavSpec {
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Channel count.
    pub channels: u16,
    /// Sample encoding.
    pub format: WavFormat,
    /// Write a WAVE_FORMAT_EXTENSIBLE header (what DAWs write for more than two channels).
    pub extensible: bool,
    /// Channel mask for the extensible header (0 = unspecified).
    pub channel_mask: u32,
}

fn put_field(c: &mut [u8], at: usize, width: usize, s: &str) {
    let b = s.as_bytes();
    let n = b.len().min(width);
    c[at..at + n].copy_from_slice(&b[..n]);
}

/// Encode a `bext` chunk body (fixed 602 bytes plus the coding history, NUL-padded to an
/// even length).
pub fn encode_bext(b: &Bext) -> Vec<u8> {
    let mut c = vec![0u8; BEXT_FIXED_LEN];
    put_field(&mut c, BEXT_DESCRIPTION, 256, &b.description);
    put_field(&mut c, BEXT_ORIGINATOR, 32, &b.originator);
    put_field(&mut c, BEXT_ORIGINATOR_REFERENCE, 32, &b.originator_reference);
    put_field(&mut c, BEXT_DATE, 10, &b.origination_date);
    put_field(&mut c, BEXT_TIME, 8, &b.origination_time);
    c[BEXT_TIME_REFERENCE..BEXT_TIME_REFERENCE + 8].copy_from_slice(&b.time_reference.to_le_bytes());
    c[BEXT_VERSION..BEXT_VERSION + 2].copy_from_slice(&b.version.to_le_bytes());
    if !b.coding_history.is_empty() {
        c.extend_from_slice(b.coding_history.as_bytes());
        c.push(0);
        if c.len() % 2 == 1 {
            c.push(0);
        }
    }
    c
}

fn chunk(out: &mut Vec<u8>, id: &[u8; 4], body: &[u8]) {
    out.extend_from_slice(id);
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(body);
    if body.len() % 2 == 1 {
        out.push(0);
    }
}

/// Build a whole WAV file around already encoded sample bytes (`data` is the data chunk).
pub fn encode_wav_raw(spec: &WavSpec, data: &[u8], bext: Option<&Bext>) -> Vec<u8> {
    let bps = spec.format.bytes_per_sample();
    let block_align = bps * spec.channels;
    let mut fmt = Vec::with_capacity(40);
    fmt.extend_from_slice(&(if spec.extensible { 0xFFFEu16 } else { spec.format.tag() }).to_le_bytes());
    fmt.extend_from_slice(&spec.channels.to_le_bytes());
    fmt.extend_from_slice(&spec.sample_rate.to_le_bytes());
    fmt.extend_from_slice(&(spec.sample_rate * block_align as u32).to_le_bytes());
    fmt.extend_from_slice(&block_align.to_le_bytes());
    fmt.extend_from_slice(&(8 * bps).to_le_bytes());
    if spec.extensible {
        fmt.extend_from_slice(&22u16.to_le_bytes());
        fmt.extend_from_slice(&(8 * bps).to_le_bytes());
        fmt.extend_from_slice(&spec.channel_mask.to_le_bytes());
        fmt.extend_from_slice(&spec.format.tag().to_le_bytes());
        fmt.extend_from_slice(&SUBFORMAT_TAIL);
    }
    let mut out = Vec::with_capacity(data.len() + 1024);
    out.extend_from_slice(b"RIFF\0\0\0\0WAVE");
    chunk(&mut out, b"fmt ", &fmt);
    if let Some(b) = bext {
        chunk(&mut out, b"bext", &encode_bext(b));
    }
    chunk(&mut out, b"data", data);
    let riff = (out.len() - 8) as u32;
    out[4..8].copy_from_slice(&riff.to_le_bytes());
    out
}

/// Quantise interleaved float samples to `format`: clamp to [-1, 1], scale to the integer
/// full scale, round to nearest.
pub fn encode_samples(format: WavFormat, interleaved: &[f32]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(interleaved.len() * format.bytes_per_sample() as usize);
    for &v in interleaved {
        let c = v.clamp(-1.0, 1.0) as f64;
        match format {
            WavFormat::Float32 => buf.extend_from_slice(&v.to_le_bytes()),
            WavFormat::Pcm16 => buf.extend_from_slice(&((c * 32767.0).round() as i16).to_le_bytes()),
            WavFormat::Pcm24 => buf.extend_from_slice(&((c * 8388607.0).round() as i32).to_le_bytes()[..3]),
            WavFormat::Pcm32 => buf.extend_from_slice(&((c * 2147483647.0).round() as i32).to_le_bytes()),
        }
    }
    buf
}

/// Build a whole WAV file from interleaved float samples.
pub fn encode_wav(spec: &WavSpec, interleaved: &[f32], bext: Option<&Bext>) -> Vec<u8> {
    encode_wav_raw(spec, &encode_samples(spec.format, interleaved), bext)
}

/// Write a WAV file from interleaved float samples.
pub fn write_wav(path: &Path, spec: &WavSpec, interleaved: &[f32], bext: Option<&Bext>) -> std::io::Result<()> {
    write_atomic(path, &encode_wav(spec, interleaved, bext))
}

/// Write `bytes` to `path` atomically: a unique temporary file in the same folder (R10,
/// opened with `create_new`, so two writers never share one inode), synced to disk, then a
/// rename, so a reader never sees a half-written file (P4-7c). The temporary file is removed
/// when the write fails (the guard of the shared helper, `io::atomic`).
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    super::atomic::write_bytes_atomic(path, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::{read_wav_mono, MixMode, SampleFormat, WavInfo};

    fn tmp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("drumspike-core-writer-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn concurrent_write_atomic_never_shares_a_temporary_and_keeps_payloads_whole() {
        let dir = std::env::temp_dir().join(format!("drumspike-core-writer-par-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("raced.bin");
        // different lengths, so a mixed inode cannot pass as one payload
        let payloads: Vec<Vec<u8>> = (0..8).map(|i| vec![b'A' + i as u8; 2_000_000 + i * 4099]).collect();
        let errors: Vec<String> = std::thread::scope(|s| {
            let handles: Vec<_> = payloads
                .iter()
                .map(|p| s.spawn(|| write_atomic(&dest, p).map_err(|e| e.to_string())))
                .collect();
            handles.into_iter().filter_map(|h| h.join().unwrap().err()).collect()
        });
        assert!(errors.is_empty(), "threads failed writing one destination: {errors:?}");
        let final_bytes = std::fs::read(&dest).unwrap();
        assert!(
            payloads.contains(&final_bytes),
            "the destination holds a mixed or partial payload ({} bytes)",
            final_bytes.len()
        );
        let left: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(left.is_empty(), "temporary files left behind: {left:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn write_atomic_leaves_no_temporary_file_and_replaces_the_target() {
        let p = tmp("atomic.bin");
        write_atomic(&p, b"one").unwrap();
        write_atomic(&p, b"two!").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"two!");
        let left: Vec<_> = std::fs::read_dir(p.parent().unwrap())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(left.is_empty());
        // a failed write (no such folder) is an error and leaves nothing
        assert!(write_atomic(&p.join("no").join("x"), b"x").is_err());
    }

    fn spec(format: WavFormat, channels: u16) -> WavSpec {
        WavSpec {
            sample_rate: 48_000,
            channels,
            format,
            extensible: channels > 2,
            channel_mask: 0,
        }
    }

    #[test]
    fn encode_samples_clamps_scales_and_rounds() {
        let x = [0.0f32, 1.0, -1.0, 2.0, -2.0, 0.5, -0.25, 0.75 / 32767.0];
        let mut want16 = Vec::new();
        for v in [0i16, 32767, -32767, 32767, -32767, 16384, -8192, 1] {
            want16.extend_from_slice(&v.to_le_bytes());
        }
        assert_eq!(encode_samples(WavFormat::Pcm16, &x), want16);
        let b24 = encode_samples(WavFormat::Pcm24, &x[..3]);
        assert_eq!(b24, [0, 0, 0, 0xFF, 0xFF, 0x7F, 0x01, 0x00, 0x80]);
        let b32 = encode_samples(WavFormat::Pcm32, &[1.0, -1.0, 0.5]);
        let w32: Vec<u8> = [i32::MAX, -i32::MAX, 1_073_741_824]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        assert_eq!(b32, w32);
        // float samples are written as they are, without the clamp
        let f = encode_samples(WavFormat::Float32, &[2.0, -0.125]);
        assert_eq!(f, [2.0f32.to_le_bytes(), (-0.125f32).to_le_bytes()].concat());
        assert!(encode_samples(WavFormat::Pcm24, &[]).is_empty());
    }

    #[test]
    fn f32_round_trips_with_bext() {
        let p = tmp("f32.wav");
        let bext = Bext {
            time_reference: 48_000 * 3600,
            origination_date: "2026-10-10".into(),
            ..Default::default()
        };
        write_wav(
            &p,
            &WavSpec {
                sample_rate: 96_000,
                ..spec(WavFormat::Float32, 1)
            },
            &[0.5, -0.25, 1e-3],
            Some(&bext),
        )
        .unwrap();
        let (x, info) = read_wav_mono(&p, MixMode::Ffmpeg).unwrap();
        assert_eq!(info.sample_format, SampleFormat::Float);
        assert_eq!(info.sample_rate, 96_000);
        assert_eq!(x, vec![0.5, -0.25, 1e-3]);
        assert_eq!(info.bext, Some(bext));
    }

    #[test]
    fn every_format_round_trips_within_one_lsb() {
        let x = [0.5f32, -1.0, 2.0, 0.123456, -0.000123];
        for (fmt, lsb) in [
            (WavFormat::Pcm16, 1.0 / 32767.0),
            (WavFormat::Pcm24, 1.0 / 8388607.0),
            (WavFormat::Pcm32, 1.0 / 8388608.0),
            (WavFormat::Float32, 0.0),
        ] {
            let b = encode_wav(&spec(fmt, 1), &x, None);
            let info = WavInfo::parse(&b).unwrap();
            assert_eq!(info.bytes_per_sample as u16, fmt.bytes_per_sample());
            let y = info.decode_mono(&b, MixMode::Ffmpeg).unwrap();
            for (a, b) in x.iter().zip(&y) {
                // PCM clamps to full scale, float is written as is (ffmpeg's f32 does not clip either)
                let want = if fmt == WavFormat::Float32 {
                    *a
                } else {
                    a.clamp(-1.0, 1.0)
                };
                assert!((want - b).abs() <= lsb, "{fmt:?}: {a} -> {b}");
            }
        }
    }

    #[test]
    fn extensible_header_carries_the_channel_mask() {
        let mut s = spec(WavFormat::Pcm24, 6);
        s.channel_mask = 0x3F;
        let b = encode_wav(&s, &[0.0; 12], None);
        let info = WavInfo::parse(&b).unwrap();
        assert!(info.extensible);
        assert_eq!(info.channel_mask, 0x3F);
        assert_eq!(info.channels, 6);
        assert_eq!(info.n_frames, 2);
    }

    #[test]
    fn bext_fields_are_truncated_to_their_width_and_the_chunk_stays_even() {
        let long = "x".repeat(300);
        let b = Bext {
            description: long.clone(),
            coding_history: "abc".into(),
            ..Default::default()
        };
        let c = encode_bext(&b);
        assert_eq!(c.len() % 2, 0);
        let back = Bext::parse(&c).unwrap();
        assert_eq!(back.description, "x".repeat(256));
        assert_eq!(back.coding_history, "abc");
        let empty = encode_bext(&Bext::default());
        assert_eq!(empty.len(), 602);
    }
}
