//! Memory-mapped WAV/BWF reader: RIFF chunk walker and sample decoding.
//!
//! Supported: RIFF/WAVE with PCM 16/24/32 (tag 1) and IEEE float 32 (tag 3), plain or
//! WAVE_FORMAT_EXTENSIBLE headers, any channel count, `bext` and `iXML` chunks. RF64 and
//! BW64 (64-bit sizes) are recognised and rejected with a clear error; support comes later.

use std::fmt;
use std::fs::File;
use std::path::Path;

use memmap2::Mmap;
use serde::Serialize;

use super::mix::{MixMode, Plan};

/// Errors of the reader: the OS (open, map) or the file's structure.
#[derive(Debug)]
pub enum WavError {
    /// An I/O error from `open` or the mapping.
    Io(std::io::Error),
    /// The bytes are not a WAV file this reader supports; the string says why.
    Format(String),
}

impl fmt::Display for WavError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WavError::Io(e) => write!(f, "io: {e}"),
            WavError::Format(s) => write!(f, "wav format: {s}"),
        }
    }
}

impl std::error::Error for WavError {}

impl From<std::io::Error> for WavError {
    fn from(e: std::io::Error) -> Self {
        WavError::Io(e)
    }
}

/// The sample encoding of the data chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SampleFormat {
    /// Signed integer PCM, `bytes_per_sample` wide (2, 3 or 4).
    Pcm,
    /// IEEE float, 32-bit.
    Float,
}

/// The Broadcast Wave `bext` chunk (EBU Tech 3285): the fields the application uses.
/// Strings are read up to the first NUL of their fixed-width field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct Bext {
    /// Free text, 256 bytes.
    pub description: String,
    /// Producer, 32 bytes.
    pub originator: String,
    /// Producer's reference, 32 bytes.
    pub originator_reference: String,
    /// `yyyy-mm-dd`, 10 bytes.
    pub origination_date: String,
    /// `hh:mm:ss`, 8 bytes.
    pub origination_time: String,
    /// First sample's position since midnight in samples (TimeReferenceLow | High << 32).
    pub time_reference: u64,
    /// BWF version.
    pub version: u16,
    /// Coding history text after the fixed 602 bytes, empty when absent.
    pub coding_history: String,
}

/// Byte offset of each bext field.
pub(crate) const BEXT_DESCRIPTION: usize = 0;
pub(crate) const BEXT_ORIGINATOR: usize = 256;
pub(crate) const BEXT_ORIGINATOR_REFERENCE: usize = 288;
pub(crate) const BEXT_DATE: usize = 320;
pub(crate) const BEXT_TIME: usize = 330;
pub(crate) const BEXT_TIME_REFERENCE: usize = 338;
pub(crate) const BEXT_VERSION: usize = 346;
/// Size of the fixed part; the coding history follows it.
pub(crate) const BEXT_FIXED_LEN: usize = 602;

fn field_str(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

impl Bext {
    /// Parse a bext chunk body. `None` when the chunk is shorter than the origination
    /// time (338 bytes), the minimum `project.bext_origination` accepts. A chunk of 338 to
    /// 345 bytes has no time reference and reads 0; a chunk without the version field
    /// reads version 0.
    pub fn parse(c: &[u8]) -> Option<Bext> {
        if c.len() < BEXT_TIME_REFERENCE {
            return None;
        }
        let version = if c.len() >= BEXT_VERSION + 2 {
            u16_at(c, BEXT_VERSION)
        } else {
            0
        };
        let coding_history = if c.len() > BEXT_FIXED_LEN {
            field_str(&c[BEXT_FIXED_LEN..])
        } else {
            String::new()
        };
        Some(Bext {
            description: field_str(&c[BEXT_DESCRIPTION..BEXT_ORIGINATOR]),
            originator: field_str(&c[BEXT_ORIGINATOR..BEXT_ORIGINATOR_REFERENCE]),
            originator_reference: field_str(&c[BEXT_ORIGINATOR_REFERENCE..BEXT_DATE]),
            origination_date: field_str(&c[BEXT_DATE..BEXT_TIME]),
            origination_time: field_str(&c[BEXT_TIME..BEXT_TIME_REFERENCE]),
            time_reference: if c.len() >= BEXT_TIME_REFERENCE + 8 {
                u64_at(c, BEXT_TIME_REFERENCE)
            } else {
                0
            },
            version,
            coding_history,
        })
    }

    /// `"<date>T<time>Z"` as `project.bext_origination` writes it into a source record;
    /// `None` when the date is empty.
    pub fn origination(&self) -> Option<String> {
        if self.origination_date.is_empty() {
            None
        } else {
            Some(format!("{}T{}Z", self.origination_date, self.origination_time))
        }
    }
}

/// Everything the chunk walker learned about the file. `data_offset`/`data_len`
/// point at the raw sample bytes inside the mapped file.
#[derive(Debug, Clone, Serialize)]
pub struct WavInfo {
    /// The format tag (1 = PCM, 3 = IEEE float); for an extensible header the sub-format.
    pub format_tag: u16,
    /// True when the header is WAVE_FORMAT_EXTENSIBLE.
    pub extensible: bool,
    /// The extensible header's channel mask (speaker positions); 0 when absent or unset.
    pub channel_mask: u32,
    /// Sample encoding.
    pub sample_format: SampleFormat,
    /// Channel count (1 or more).
    pub channels: u16,
    /// Native sample rate in Hz.
    pub sample_rate: u32,
    /// Bits per sample as declared in the header.
    pub bits_per_sample: u16,
    /// Container bytes per sample (2, 3 or 4).
    pub bytes_per_sample: usize,
    /// Bytes per frame (all channels).
    pub block_align: usize,
    /// Number of frames in the data chunk.
    pub n_frames: usize,
    /// Byte offset of the first sample in the file.
    pub data_offset: usize,
    /// Length of the data chunk in bytes (clamped to the file).
    pub data_len: usize,
    /// The `bext` chunk, if present and long enough.
    pub bext: Option<Bext>,
    /// The `iXML` chunk as text (trailing NULs removed), if present.
    pub ixml: Option<String>,
}

pub(crate) const SUBFORMAT_TAIL: [u8; 14] = [
    0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71,
];

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}
fn u64_at(b: &[u8], o: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[o..o + 8]);
    u64::from_le_bytes(a)
}

struct Fmt {
    tag: u16,
    extensible: bool,
    channel_mask: u32,
    sample_format: SampleFormat,
    channels: u16,
    sample_rate: u32,
    bits: u16,
    block_align: usize,
}

fn parse_fmt(c: &[u8]) -> Result<Fmt, WavError> {
    let err = |s: &str| WavError::Format(s.to_string());
    if c.len() < 16 {
        return Err(err("fmt chunk too short"));
    }
    let mut tag = u16_at(c, 0);
    let channels = u16_at(c, 2);
    let sample_rate = u32_at(c, 4);
    let block_align = u16_at(c, 12) as usize;
    let bits = u16_at(c, 14);
    let mut extensible = false;
    let mut channel_mask = 0;
    if tag == 0xFFFE {
        extensible = true;
        if c.len() < 40 {
            return Err(err("WAVE_FORMAT_EXTENSIBLE fmt chunk too short"));
        }
        // cbSize(2) validBits(2) channelMask(4) SubFormat GUID(16) at 24..40. validBits is
        // not used: the container width (wBitsPerSample) decides the decode, and PCM samples
        // are left-justified, so 20 bits in 3 bytes or 24 bits in 4 bytes decode correctly
        // (ffmpeg 9.0.2 reads 24-in-32 as pcm_f24le instead; see the report's deviations).
        channel_mask = u32_at(c, 20);
        let guid = &c[24..40];
        if guid[2..16] != SUBFORMAT_TAIL {
            return Err(err("unknown SubFormat GUID"));
        }
        tag = u16_at(guid, 0);
    }
    let sample_format = match tag {
        1 => SampleFormat::Pcm,
        3 => SampleFormat::Float,
        t => return Err(WavError::Format(format!("unsupported format tag {t}"))),
    };
    if channels == 0 {
        return Err(err("zero channels"));
    }
    Ok(Fmt {
        tag,
        extensible,
        channel_mask,
        sample_format,
        channels,
        sample_rate,
        bits,
        block_align,
    })
}

impl WavInfo {
    /// Walk the RIFF chunks of `bytes` (a whole WAV file in memory or mmap).
    ///
    /// # Errors
    ///
    /// [`WavError::Format`] when the bytes are RF64/BW64 or not RIFF/WAVE, the `fmt ` or
    /// `data` chunk is missing or the `fmt ` chunk is malformed, the sample format and width
    /// are unsupported, or `nBlockAlign` disagrees with the channels and width.
    pub fn parse(bytes: &[u8]) -> Result<WavInfo, WavError> {
        let err = |s: &str| WavError::Format(s.to_string());
        if bytes.len() >= 4 && (&bytes[0..4] == b"RF64" || &bytes[0..4] == b"BW64") {
            return Err(err(
                "RF64/BW64 (64-bit WAV) is not supported yet; convert the file to RIFF/WAVE",
            ));
        }
        if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
            return Err(err("not a RIFF/WAVE file"));
        }
        let mut pos = 12usize;
        let mut fmt_: Option<Fmt> = None;
        let mut data: Option<(usize, usize)> = None;
        let mut bext = None;
        let mut ixml = None;

        while pos + 8 <= bytes.len() {
            let id = &bytes[pos..pos + 4];
            let size = u32_at(bytes, pos + 4) as usize;
            let body = pos + 8;
            // Clamp: a chunk cannot reach past the end of the file. Streaming writers leave
            // 0xFFFFFFFF in the data chunk, so the clamp reads it to the end of the file.
            let avail = bytes.len().saturating_sub(body);
            let size_c = size.min(avail);
            let chunk = &bytes[body..body + size_c];
            match id {
                b"fmt " => fmt_ = Some(parse_fmt(chunk)?),
                // A data chunk of declared size 0 (left by a crashed or unfinalised recorder)
                // runs to the end of the file, as in ffmpeg's WAV demuxer, which also stops
                // the chunk walk there (review round 3, finding M1).
                b"data" if size == 0 => {
                    data = Some((body, avail));
                    break;
                }
                b"data" => data = Some((body, size_c)),
                b"bext" => bext = Bext::parse(chunk),
                b"iXML" => ixml = Some(String::from_utf8_lossy(chunk).trim_end_matches('\0').to_string()),
                _ => {}
            }
            // chunks are word-aligned (pad byte after odd sizes)
            pos = body + size_c + (size_c & 1);
        }

        let f = fmt_.ok_or_else(|| err("no fmt chunk"))?;
        let (data_offset, data_len) = data.ok_or_else(|| err("no data chunk"))?;
        // The sample width comes from wBitsPerSample, rounded up to whole bytes, as ffmpeg's
        // WAV demuxer picks the codec (pcm_s16le for 9 to 16 bits, pcm_s24le for 17 to 24,
        // pcm_s32le for 25 to 32). A header whose nBlockAlign disagrees with that width is
        // refused: ffmpeg decodes the samples by the width and only frames packets by the
        // block align, so the binding hands such a file to ffmpeg (review finding M1).
        let bytes_per_sample = (f.bits as usize).div_ceil(8);
        match (f.sample_format, bytes_per_sample) {
            (SampleFormat::Pcm, 2..=4) | (SampleFormat::Float, 4) => {}
            (sf, b) => return Err(WavError::Format(format!("unsupported {sf:?} with {b} bytes/sample"))),
        }
        if f.block_align != 0 && f.block_align != bytes_per_sample * f.channels as usize {
            return Err(WavError::Format(format!(
                "block align {} does not match {} channels of {} bits",
                f.block_align, f.channels, f.bits
            )));
        }
        let frame = bytes_per_sample * f.channels as usize;
        Ok(WavInfo {
            format_tag: f.tag,
            extensible: f.extensible,
            channel_mask: f.channel_mask,
            sample_format: f.sample_format,
            channels: f.channels,
            sample_rate: f.sample_rate,
            bits_per_sample: f.bits,
            bytes_per_sample,
            block_align: frame,
            n_frames: data_len / frame,
            data_offset,
            data_len,
            bext,
            ixml,
        })
    }

    /// The `bext` TimeReference (first sample's position since midnight, in samples).
    pub fn time_reference(&self) -> Option<u64> {
        self.bext.as_ref().map(|b| b.time_reference)
    }

    /// The downmix plan for this file in `mode` (see [`Plan::new`] for the errors).
    ///
    /// # Errors
    ///
    /// [`WavError::Format`] with the message of [`Plan::new`] when the channel count or mask
    /// is not supported by `mode`.
    pub fn mix_plan(&self, mode: MixMode) -> Result<Plan, WavError> {
        let mask = if self.extensible { self.channel_mask } else { 0 };
        Plan::new(mode, self.channels as usize, mask).map_err(WavError::Format)
    }

    /// Decode the data chunk to mono f32 at native rate, folded with `mode`.
    ///
    /// # Errors
    ///
    /// [`WavError::Format`] when the downmix `mode` is refused for this file (see [`Plan::new`]).
    pub fn decode_mono(&self, bytes: &[u8], mode: MixMode) -> Result<Vec<f32>, WavError> {
        let plan = self.mix_plan(mode)?;
        let d = &bytes[self.data_offset..self.data_offset + self.n_frames * self.block_align];
        let ch = self.channels as usize;
        Ok(match (self.sample_format, self.bytes_per_sample) {
            (SampleFormat::Float, 4) => {
                decode_frames::<4, f32>(d, ch, |b| f32::from_le_bytes(*b), |f| plan.fold_f32(f))
            }
            (SampleFormat::Pcm, 2) => decode_frames::<2, i16>(d, ch, |b| i16::from_le_bytes(*b), |f| plan.fold_s16(f)),
            // ffmpeg decodes 24-bit into the top of an int32
            (SampleFormat::Pcm, 3) => decode_frames::<3, i32>(
                d,
                ch,
                |b| (((b[2] as i8 as i32) << 16) | ((b[1] as i32) << 8) | b[0] as i32) << 8,
                |f| plan.fold_s32(f),
            ),
            (SampleFormat::Pcm, 4) => decode_frames::<4, i32>(d, ch, |b| i32::from_le_bytes(*b), |f| plan.fold_s32(f)),
            _ => unreachable!("validated in parse"),
        })
    }

    /// P3-16: decode whole frames of `data` into `out` with every channel kept (interleaved,
    /// `channels` samples per frame), each sample converted exactly as the sample of a mono
    /// file is (the 1-channel fold of [`Plan`]). Returns the frames written. The playback of a
    /// stereo file uses it; detection and exports stay on the mono fold.
    ///
    /// # Panics
    ///
    /// The `expect("one channel")` on the 1-channel [`Plan`] cannot fail. The
    /// `unreachable!("validated in parse")` arm is reached only for a sample format and width
    /// that [`WavInfo::parse`] rejects, so it needs a hand-built `WavInfo`. The final division
    /// by the channel count panics for a `WavInfo` with 0 channels, which is likewise not
    /// produced by `parse`.
    pub fn decode_channels_into(&self, data: &[u8], out: &mut [f32]) -> usize {
        let ch = self.channels as usize;
        let one = Plan::new(MixMode::Average, 1, 0).expect("one channel");
        let n = match (self.sample_format, self.bytes_per_sample) {
            (SampleFormat::Float, 4) => {
                decode_samples_into::<4, f32>(data, out, |b| f32::from_le_bytes(*b), |x| one.fold_f32(&[x]))
            }
            (SampleFormat::Pcm, 2) => {
                decode_samples_into::<2, i16>(data, out, |b| i16::from_le_bytes(*b), |x| one.fold_s16(&[x]))
            }
            (SampleFormat::Pcm, 3) => decode_samples_into::<3, i32>(
                data,
                out,
                |b| (((b[2] as i8 as i32) << 16) | ((b[1] as i32) << 8) | b[0] as i32) << 8,
                |x| one.fold_s32(&[x]),
            ),
            (SampleFormat::Pcm, 4) => {
                decode_samples_into::<4, i32>(data, out, |b| i32::from_le_bytes(*b), |x| one.fold_s32(&[x]))
            }
            _ => unreachable!("validated in parse"),
        };
        n / ch
    }

    /// Decode whole frames from `data` (raw bytes of the data chunk that start at a frame
    /// boundary, for example a slice of the mapping or a positioned read) into `out`, folded
    /// with `plan` (from [`WavInfo::mix_plan`]); returns the number of frames written, at
    /// most `out.len()` and `data.len() / block_align`. The values equal the same frames of
    /// [`WavInfo::decode_mono`] bit for bit. The transport decodes in blocks with it.
    pub fn decode_frames_into(&self, plan: &Plan, data: &[u8], out: &mut [f32]) -> usize {
        let ch = self.channels as usize;
        debug_assert_eq!(plan.channels(), ch);
        match (self.sample_format, self.bytes_per_sample) {
            (SampleFormat::Float, 4) => {
                decode_frames_into::<4, f32>(data, ch, out, |b| f32::from_le_bytes(*b), |f| plan.fold_f32(f))
            }
            (SampleFormat::Pcm, 2) => {
                decode_frames_into::<2, i16>(data, ch, out, |b| i16::from_le_bytes(*b), |f| plan.fold_s16(f))
            }
            (SampleFormat::Pcm, 3) => decode_frames_into::<3, i32>(
                data,
                ch,
                out,
                |b| (((b[2] as i8 as i32) << 16) | ((b[1] as i32) << 8) | b[0] as i32) << 8,
                |f| plan.fold_s32(f),
            ),
            (SampleFormat::Pcm, 4) => {
                decode_frames_into::<4, i32>(data, ch, out, |b| i32::from_le_bytes(*b), |f| plan.fold_s32(f))
            }
            _ => unreachable!("validated in parse"),
        }
    }
}

/// Decode whole frames of `N`-byte samples into `out` channel by channel (interleaved, no
/// fold), at most `out.len()` samples; returns the count of samples. `conv` converts one sample.
fn decode_samples_into<const N: usize, T: Copy>(
    d: &[u8],
    out: &mut [f32],
    read: impl Fn(&[u8; N]) -> T,
    conv: impl Fn(T) -> f32,
) -> usize {
    let samples = d.as_chunks::<N>().0;
    let n = samples.len().min(out.len());
    for (o, b) in out[..n].iter_mut().zip(samples) {
        *o = conv(read(b));
    }
    n
}

/// Decode whole frames of `ch` samples of `N` bytes each into `out`, at most `out.len()`
/// frames; returns the count. The same per-frame fold as [`decode_frames`].
fn decode_frames_into<const N: usize, T: Copy>(
    d: &[u8],
    ch: usize,
    out: &mut [f32],
    read: impl Fn(&[u8; N]) -> T,
    fold: impl Fn(&[T]) -> f32,
) -> usize {
    let samples = d.as_chunks::<N>().0;
    let n = (samples.len() / ch).min(out.len());
    if n == 0 {
        return 0;
    }
    if ch == 1 {
        for (o, b) in out[..n].iter_mut().zip(samples) {
            *o = fold(&[read(b)]);
        }
    } else {
        let mut frame = [read(&samples[0]); 8];
        let mut wide = Vec::new();
        let buf: &mut [T] = if ch <= 8 {
            &mut frame[..ch]
        } else {
            wide.resize(ch, read(&samples[0]));
            &mut wide
        };
        for (o, f) in out[..n].iter_mut().zip(samples.chunks_exact(ch)) {
            for (s, b) in buf.iter_mut().zip(f) {
                *s = read(b);
            }
            *o = fold(buf);
        }
    }
    n
}

/// Decode interleaved frames of `ch` samples of `N` bytes each: `read` converts one sample,
/// `fold` one frame.
fn decode_frames<const N: usize, T: Copy>(
    d: &[u8],
    ch: usize,
    read: impl Fn(&[u8; N]) -> T,
    fold: impl Fn(&[T]) -> f32,
) -> Vec<f32> {
    let samples = d.as_chunks::<N>().0;
    let mut out = Vec::with_capacity(samples.len() / ch);
    if samples.is_empty() {
        return out;
    }
    if ch == 1 {
        out.extend(samples.iter().map(|b| fold(&[read(b)])));
    } else {
        let mut frame = vec![read(&samples[0]); ch];
        for f in samples.chunks_exact(ch) {
            for (o, b) in frame.iter_mut().zip(f) {
                *o = read(b);
            }
            out.push(fold(&frame));
        }
    }
    out
}

/// A memory-mapped WAV file. Dropping it unmaps the file.
pub struct MappedWav {
    /// Parsed header.
    pub info: WavInfo,
    mmap: Mmap,
}

impl MappedWav {
    /// Open `path` read-only, map it and parse the header. The file is never written.
    ///
    /// # Errors
    ///
    /// [`WavError::Io`] when the file cannot be opened or mapped; [`WavError::Format`] when
    /// the header is not a supported WAV (see [`WavInfo::parse`]).
    pub fn open(path: &Path) -> Result<MappedWav, WavError> {
        let f = File::open(path)?;
        // SAFETY: the mapping is read-only and the file is opened read-only, so the only way
        // the mapped bytes change is another process truncating or rewriting the source
        // file while we read it. The reader treats sources as immutable (I3) and the worst
        // case of an outside change is a wrong decode or a SIGBUS, never memory corruption
        // of our own data.
        let mmap = unsafe { Mmap::map(&f)? };
        let info = WavInfo::parse(&mmap)?;
        Ok(MappedWav { info, mmap })
    }
    /// The whole file as bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.mmap
    }
    /// Decode the data chunk to mono f32 (see [`WavInfo::decode_mono`]).
    ///
    /// # Errors
    ///
    /// [`WavError::Format`] when the downmix `mode` is refused for this file (see [`Plan::new`]).
    pub fn decode_mono(&self, mode: MixMode) -> Result<Vec<f32>, WavError> {
        self.info.decode_mono(&self.mmap, mode)
    }
    /// Tell the OS that the mapped bytes `from..to` are not needed now, so their pages leave
    /// the resident set (`madvise(MADV_DONTNEED)`; a no-op on other systems). The transport
    /// calls it for the pages behind its read position. A later read of the range maps the
    /// file pages again with the same content. The range is clamped to the mapping and
    /// widened down to a page boundary by the OS layer. On macOS the advice does not lower
    /// the resident set of a file mapping (measured in P3-5a); the transport therefore reads
    /// by positioned reads by default.
    pub fn release(&self, from: usize, to: usize) {
        let to = to.min(self.mmap.len());
        if from >= to {
            return;
        }
        #[cfg(unix)]
        {
            // SAFETY: the mapping is a read-only shared file mapping. MADV_DONTNEED only drops
            // the resident pages; the next access maps the file pages again with the same
            // bytes (the file is treated as immutable, I3), so no slice handed out by
            // `bytes()` can observe a change. The advice is a hint; an error is ignored.
            let _ = unsafe {
                self.mmap
                    .unchecked_advise_range(memmap2::UncheckedAdvice::DontNeed, from, to - from)
            };
        }
    }
}

/// Convenience: mmap, parse, decode.
///
/// # Errors
///
/// [`WavError::Io`] when the file cannot be opened or mapped; [`WavError::Format`] when the
/// header is unsupported or the downmix `mode` is refused for this file (see [`Plan::new`]).
pub fn read_wav_mono(path: &Path, mode: MixMode) -> Result<(Vec<f32>, WavInfo), WavError> {
    let m = MappedWav::open(path)?;
    let x = m.decode_mono(mode)?;
    Ok((x, m.info))
}

/// Read the whole file into memory (no mapping) and parse the header. The Python binding
/// uses this instead of [`MappedWav`]: a source file that another program truncates while
/// it is read then gives a short read or a format error, not a SIGBUS that ends the server
/// process. For a file in the page cache the copy costs about as much as the mapping.
///
/// # Errors
///
/// [`WavError::Io`] when the file cannot be read; [`WavError::Format`] when the bytes are not
/// a WAV file this reader supports (see [`WavInfo::parse`]).
pub fn read_wav_bytes(path: &Path) -> Result<(Vec<u8>, WavInfo), WavError> {
    let bytes = std::fs::read(path)?;
    let info = WavInfo::parse(&bytes)?;
    Ok((bytes, info))
}

#[cfg(test)]
mod tests;
