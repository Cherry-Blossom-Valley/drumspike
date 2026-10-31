//! WAV writer: test data now, the spike WAV export in P1-5 (with the source's `bext`).

use std::path::Path;

use super::reader::{
    BEXT_DATE, BEXT_DESCRIPTION, BEXT_FIXED_LEN, BEXT_ORIGINATOR, BEXT_ORIGINATOR_REFERENCE, BEXT_TIME,
    BEXT_TIME_REFERENCE, BEXT_VERSION, Bext, SUBFORMAT_TAIL,
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
    /// Write a `WAVE_FORMAT_EXTENSIBLE` header (what DAWs write for more than two channels).
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

/// The fmt chunk body of a written file (the one [`encode_wav_raw`] and the block writer
/// build).
fn fmt_body(spec: &WavSpec) -> Vec<u8> {
    let bps = spec.format.bytes_per_sample();
    let block_align = bps * spec.channels;
    let mut fmt = Vec::with_capacity(40);
    fmt.extend_from_slice(&(if spec.extensible { 0xFFFEu16 } else { spec.format.tag() }).to_le_bytes());
    fmt.extend_from_slice(&spec.channels.to_le_bytes());
    fmt.extend_from_slice(&spec.sample_rate.to_le_bytes());
    fmt.extend_from_slice(&(spec.sample_rate * u32::from(block_align)).to_le_bytes());
    fmt.extend_from_slice(&block_align.to_le_bytes());
    fmt.extend_from_slice(&(8 * bps).to_le_bytes());
    if spec.extensible {
        fmt.extend_from_slice(&22u16.to_le_bytes());
        fmt.extend_from_slice(&(8 * bps).to_le_bytes());
        fmt.extend_from_slice(&spec.channel_mask.to_le_bytes());
        fmt.extend_from_slice(&spec.format.tag().to_le_bytes());
        fmt.extend_from_slice(&SUBFORMAT_TAIL);
    }
    fmt
}

/// Build a whole WAV file around already encoded sample bytes (`data` is the data chunk).
pub fn encode_wav_raw(spec: &WavSpec, data: &[u8], bext: Option<&Bext>) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 1024);
    out.extend_from_slice(b"RIFF\0\0\0\0WAVE");
    chunk(&mut out, b"fmt ", &fmt_body(spec));
    if let Some(b) = bext {
        chunk(&mut out, b"bext", &encode_bext(b));
    }
    chunk(&mut out, b"data", data);
    let riff = (out.len() - 8) as u32;
    out[4..8].copy_from_slice(&riff.to_le_bytes());
    out
}

/// The header of a `n_frames`-long WAV file, written before any sample (the sizes follow
/// from the frame count): the same bytes [`encode_wav_raw`] builds around the same samples,
/// an odd data chunk keeping its pad byte in the riff size.
fn wav_header(spec: &WavSpec, n_frames: usize, bext: Option<&Bext>) -> Vec<u8> {
    let data_len = n_frames * spec.format.bytes_per_sample() as usize * spec.channels as usize;
    let mut out = Vec::with_capacity(1024);
    out.extend_from_slice(b"RIFF\0\0\0\0WAVE");
    chunk(&mut out, b"fmt ", &fmt_body(spec));
    if let Some(b) = bext {
        chunk(&mut out, b"bext", &encode_bext(b));
    }
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data_len as u32).to_le_bytes());
    let riff = (out.len() - 8 + data_len + data_len % 2) as u32;
    out[4..8].copy_from_slice(&riff.to_le_bytes());
    out
}

/// The block size the streaming writer fills and encodes by default: 64 Ki frames (R19
/// step 2). `export_spikes` writes with it; the block writer takes the size per call.
pub const WRITE_BLOCK_FRAMES: usize = 64 * 1024;

// The size hook of the block-writer test (#48): per thread, the length in samples of the
// largest slice `encode_samples` has encoded since it was armed, `None` while disarmed. A
// doc comment cannot sit on a `thread_local!` invocation, so this is a plain comment.
#[cfg(test)]
thread_local! {
    static ENCODE_HOOK: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// Arm the size hook: record every slice [`encode_samples`] encodes on this thread.
#[cfg(test)]
pub(crate) fn encode_hook_arm() {
    ENCODE_HOOK.with(|h| h.set(Some(0)));
}

/// The largest slice length the size hook has seen since it was armed.
#[cfg(test)]
pub(crate) fn encode_hook_max() -> Option<usize> {
    ENCODE_HOOK.with(std::cell::Cell::get)
}

/// Note a slice length on the size hook (a no-op while it is disarmed).
fn encode_hook_note(len: usize) {
    #[cfg(test)]
    ENCODE_HOOK.with(|h| {
        if let Some(m) = h.get() {
            h.set(Some(m.max(len)));
        }
    });
    let _ = len;
}

/// Quantise interleaved float samples to `format`: clamp to [-1, 1], scale to the integer
/// full scale, round to nearest.
pub fn encode_samples(format: WavFormat, interleaved: &[f32]) -> Vec<u8> {
    encode_hook_note(interleaved.len());
    let mut buf = Vec::with_capacity(interleaved.len() * format.bytes_per_sample() as usize);
    for &v in interleaved {
        let c = f64::from(v.clamp(-1.0, 1.0));
        match format {
            WavFormat::Float32 => buf.extend_from_slice(&v.to_le_bytes()),
            WavFormat::Pcm16 => buf.extend_from_slice(&((c * 32767.0).round() as i16).to_le_bytes()),
            WavFormat::Pcm24 => buf.extend_from_slice(&((c * 8_388_607.0).round() as i32).to_le_bytes()[..3]),
            WavFormat::Pcm32 => buf.extend_from_slice(&((c * 2_147_483_647.0).round() as i32).to_le_bytes()),
        }
    }
    buf
}

/// Build a whole WAV file from interleaved float samples.
pub fn encode_wav(spec: &WavSpec, interleaved: &[f32], bext: Option<&Bext>) -> Vec<u8> {
    encode_wav_raw(spec, &encode_samples(spec.format, interleaved), bext)
}

/// Write a WAV file from interleaved float samples.
///
/// # Errors
///
/// An [`std::io::Error`] from the atomic write (see [`write_atomic`]).
pub fn write_wav(path: &Path, spec: &WavSpec, interleaved: &[f32], bext: Option<&Bext>) -> std::io::Result<()> {
    write_atomic(path, &encode_wav(spec, interleaved, bext))
}

/// Write a `n_frames`-long WAV file in blocks (R19 step 2): the header first (the sizes
/// follow from the frame count), then the samples block by block — `fill(start, buf)` fills
/// `buf` with the interleaved samples of the frames `start..start + buf.len() / channels`,
/// and the writer allocates exactly one `frames_per_block` block and reuses it, so no buffer
/// of the writer holds more than one block. The file goes through the same atomic temporary
/// plus rename as [`write_atomic`] (no partial file on error), and the bytes on disk are the
/// ones [`encode_wav_raw`] builds for the same samples. A `fill` or write error removes the
/// temporary file and leaves the destination exactly as it was.
///
/// # Errors
///
/// An [`std::io::Error`] when the temporary file cannot be created, written or renamed, or
/// the error `fill` returns for a block.
pub fn write_wav_blockwise<F>(
    path: &Path,
    spec: &WavSpec,
    n_frames: usize,
    bext: Option<&Bext>,
    frames_per_block: usize,
    mut fill: F,
) -> std::io::Result<()>
where
    F: FnMut(usize, &mut [f32]) -> std::io::Result<()>,
{
    let frames_per_block = frames_per_block.max(1);
    let channels = spec.channels as usize;
    let mut t = super::atomic::TempFile::create(path)?;
    t.write_all(&wav_header(spec, n_frames, bext))?;
    let mut block = vec![0.0f32; frames_per_block.saturating_mul(channels)];
    let mut start = 0;
    while start < n_frames {
        let frames = frames_per_block.min(n_frames - start);
        let buf = &mut block[..frames * channels];
        fill(start, buf)?;
        t.write_all(&encode_samples(spec.format, buf))?;
        start += frames;
    }
    let data_len = n_frames * spec.format.bytes_per_sample() as usize * channels;
    if data_len % 2 == 1 {
        t.write_all(&[0])?; // the pad byte of an odd data chunk (the byte `chunk` appends)
    }
    t.commit()
}

/// Write `bytes` to `path` atomically: a unique temporary file in the same folder (R10,
/// opened with `create_new`, so two writers never share one inode), synced to disk, then a
/// rename, so a reader never sees a half-written file (P4-7c). The temporary file is removed
/// when the write fails (the guard of the shared helper, `io::atomic`).
///
/// # Errors
///
/// An [`std::io::Error`] when the path has no file name, the temporary file cannot be
/// created, written or synced, or the rename fails.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    super::atomic::write_bytes_atomic(path, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::{MixMode, SampleFormat, WavInfo, read_wav_mono};

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
        let x = [0.5f32, -1.0, 2.0, 0.123_456, -0.000_123];
        for (fmt, lsb) in [
            (WavFormat::Pcm16, 1.0 / 32767.0),
            (WavFormat::Pcm24, 1.0 / 8_388_607.0),
            (WavFormat::Pcm32, 1.0 / 8_388_608.0),
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

    #[test]
    fn spike_writer_encodes_never_more_than_one_block_at_once() {
        // R19 step 2, the size hook: the largest slice the writer encodes for a 3.5-block
        // spike file. Red on the full-buffer writer: it encodes the whole signal in one call.
        let dir = std::env::temp_dir().join(format!("drumspike-core-writer-hook-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sr = 48_000u32;
        let n = WRITE_BLOCK_FRAMES * 7 / 2;
        let times = [
            0.0,
            1.0,
            (WRITE_BLOCK_FRAMES - 2) as f64 / f64::from(sr),
            (WRITE_BLOCK_FRAMES + 3) as f64 / f64::from(sr),
            (n - 1) as f64 / f64::from(sr),
        ];
        let src = Bext::default();
        let out = dir.join("hook.wav");
        encode_hook_arm();
        crate::spike::export_spikes(&out, sr, n, &times, Some(&src), WavFormat::Pcm16).unwrap();
        let max = encode_hook_max().expect("the writer encoded at least one slice");
        assert!(
            max <= WRITE_BLOCK_FRAMES,
            "the writer encoded {max} samples in one slice, more than one block ({WRITE_BLOCK_FRAMES})"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The pre-#48 export path: the whole signal and the whole encoded file in memory.
    fn old_export_spikes(
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
        let bext = source_bext.map(|b| crate::spike::spike_bext(b, sr, format));
        write_wav(
            path,
            &spec,
            &crate::spike::spike_signal(sr, n_samples, times),
            bext.as_ref(),
        )
    }

    #[test]
    fn spike_export_in_blocks_is_byte_identical_to_the_full_buffer_path() {
        let dir = std::env::temp_dir().join(format!("drumspike-core-writer-ident-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sr = 48_000u32;
        let block = WRITE_BLOCK_FRAMES;
        // the gate lengths: 0, 1, one block - 1, one block, one block + 1, 3.5 blocks
        let lengths = [0usize, 1, block - 1, block, block + 1, block * 7 / 2];
        // hits at the start, across the first block border and at the end of the longest file
        let times = [
            0.0,
            1.0,
            (block - 2) as f64 / f64::from(sr),
            (block + 3) as f64 / f64::from(sr),
            (lengths[5] - 1) as f64 / f64::from(sr),
        ];
        let src = Bext {
            time_reference: 123_456,
            origination_date: "2026-10-28".into(),
            ..Default::default()
        };
        for (k, &n) in lengths.iter().enumerate() {
            for format in [WavFormat::Pcm16, WavFormat::Pcm24, WavFormat::Float32] {
                // every second length carries the source bext (the spike file gets its own)
                let bext = if k % 2 == 0 { Some(&src) } else { None };
                let old = dir.join(format!("old-{k}-{format:?}.wav"));
                let new = dir.join(format!("new-{k}-{format:?}.wav"));
                old_export_spikes(&old, sr, n, &times, bext, format).unwrap();
                crate::spike::export_spikes(&new, sr, n, &times, bext, format).unwrap();
                assert_eq!(
                    std::fs::read(&new).unwrap(),
                    std::fs::read(&old).unwrap(),
                    "n={n} {format:?} bext={}",
                    bext.is_some()
                );
            }
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Write `samples` through the block writer and read the file back.
    fn blockwise_bytes(spec: &WavSpec, samples: &[f32], frames_per_block: usize, bext: Option<&Bext>) -> Vec<u8> {
        let dir = std::env::temp_dir().join(format!("drumspike-core-writer-bw-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("bw.wav");
        let n = samples.len() / spec.channels as usize;
        write_wav_blockwise(&p, spec, n, bext, frames_per_block, |start, buf| {
            let at = start * spec.channels as usize;
            buf.copy_from_slice(&samples[at..at + buf.len()]);
            Ok(())
        })
        .unwrap();
        let bytes = std::fs::read(&p).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        bytes
    }

    #[test]
    fn write_wav_blockwise_matches_encode_wav_raw_for_small_blocks() {
        let x: Vec<f32> = (0..30).map(|i| ((i % 17) as f32 * 0.125) - 1.0).collect();
        // mono Pcm24, 9 frames in blocks of 4: an odd data chunk keeps its pad byte
        let m9 = &x[..9];
        assert_eq!(
            blockwise_bytes(&spec(WavFormat::Pcm24, 1), m9, 4, None),
            encode_wav_raw(&spec(WavFormat::Pcm24, 1), &encode_samples(WavFormat::Pcm24, m9), None)
        );
        // one frame in one oversized block
        let m1 = &x[..1];
        assert_eq!(
            blockwise_bytes(&spec(WavFormat::Pcm16, 1), m1, 100, None),
            encode_wav_raw(&spec(WavFormat::Pcm16, 1), &encode_samples(WavFormat::Pcm16, m1), None)
        );
        // stereo in blocks of 2 frames, with a bext chunk in the header
        let st = &x[..10];
        let bext = Bext {
            time_reference: 42,
            ..Default::default()
        };
        assert_eq!(
            blockwise_bytes(&spec(WavFormat::Float32, 2), st, 2, Some(&bext)),
            encode_wav_raw(
                &spec(WavFormat::Float32, 2),
                &encode_samples(WavFormat::Float32, st),
                Some(&bext)
            )
        );
        // extensible 3 channels, Pcm24, 7 frames in blocks of 3: an odd data chunk again
        let c3 = &x[..21];
        let mut s3 = spec(WavFormat::Pcm24, 3);
        s3.channel_mask = 0x7;
        assert_eq!(
            blockwise_bytes(&s3, c3, 3, None),
            encode_wav_raw(&s3, &encode_samples(WavFormat::Pcm24, c3), None)
        );
        // an empty file is the bare header
        assert_eq!(
            blockwise_bytes(&spec(WavFormat::Pcm16, 1), &[], 8, None),
            encode_wav_raw(&spec(WavFormat::Pcm16, 1), &[], None)
        );
    }

    #[test]
    fn the_block_writer_hands_out_one_block_at_a_time_in_order() {
        let dir = std::env::temp_dir().join(format!("drumspike-core-writer-order-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("order.wav");
        let n = WRITE_BLOCK_FRAMES * 2 + 5;
        let mut calls: Vec<(usize, usize)> = Vec::new();
        write_wav_blockwise(
            &p,
            &spec(WavFormat::Pcm16, 1),
            n,
            None,
            WRITE_BLOCK_FRAMES,
            |start, buf| {
                calls.push((start, buf.len()));
                buf.fill(0.25);
                Ok(())
            },
        )
        .unwrap();
        let mut want = Vec::new();
        let mut start = 0;
        while start < n {
            let frames = WRITE_BLOCK_FRAMES.min(n - start);
            want.push((start, frames));
            start += frames;
        }
        assert_eq!(calls, want, "one block per call, ascending, the last one cut");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_failing_fill_leaves_no_destination_and_no_temporary() {
        let dir = std::env::temp_dir().join(format!("drumspike-core-writer-fail-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("x.wav");
        std::fs::write(&dest, b"keep").unwrap();
        let e = write_wav_blockwise(&dest, &spec(WavFormat::Pcm16, 1), 100, None, 10, |_, _| {
            Err(std::io::Error::other("fill failed"))
        })
        .unwrap_err();
        assert_eq!(e.to_string(), "fill failed");
        assert_eq!(std::fs::read(&dest).unwrap(), b"keep", "the destination is untouched");
        // a destination that did not exist stays absent
        let gone = dir.join("gone.wav");
        assert!(
            write_wav_blockwise(&gone, &spec(WavFormat::Pcm16, 1), 10, None, 5, |_, _| {
                Err(std::io::Error::other("no"))
            })
            .is_err()
        );
        assert!(!gone.exists());
        let left: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(left.is_empty(), "temporary files left behind: {left:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_block_writer_replaces_the_destination_atomically() {
        let dir = std::env::temp_dir().join(format!("drumspike-core-writer-repl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("r.wav");
        write_wav_blockwise(&p, &spec(WavFormat::Pcm16, 1), 4, None, 2, |_, buf| {
            buf.fill(0.5);
            Ok(())
        })
        .unwrap();
        write_wav_blockwise(&p, &spec(WavFormat::Pcm16, 1), 2, None, 2, |_, buf| {
            buf.fill(-0.5);
            Ok(())
        })
        .unwrap();
        let want = encode_wav_raw(
            &spec(WavFormat::Pcm16, 1),
            &encode_samples(WavFormat::Pcm16, &[-0.5, -0.5]),
            None,
        );
        assert_eq!(std::fs::read(&p).unwrap(), want, "the second write replaced the first");
        let left: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(left.is_empty(), "temporary files left behind: {left:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
