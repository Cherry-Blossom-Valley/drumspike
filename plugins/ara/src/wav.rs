//! The streaming f32 WAV writer of the plugin: the samples the host delivers go to the
//! cache folder block by block, so no reader holds the whole source in memory. The frame
//! count is known up front (the ARA source properties), so the header carries the final
//! sizes and the file goes through a temporary plus rename (a reader never sees half a
//! file). The service decodes the file with its own reader (`drumspike_core::io`), which
//! reads float 32 bit for bit.
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

/// One open WAV file: write `write_block` per block, then `commit`.
pub struct WavWriter {
    out: BufWriter<std::fs::File>,
    tmp: PathBuf,
    dest: PathBuf,
}

fn fmt_body(channels: u16, sample_rate: u32) -> Vec<u8> {
    // IEEE float 32 (format tag 3), canonical fmt chunk, no extensible header (the
    // service's reader takes the plain one and the fixtures are mono or stereo)
    let bytes_per_sample = 4u16;
    let block_align = bytes_per_sample * channels;
    let mut fmt = Vec::with_capacity(16);
    fmt.extend_from_slice(&3u16.to_le_bytes());
    fmt.extend_from_slice(&channels.to_le_bytes());
    fmt.extend_from_slice(&sample_rate.to_le_bytes());
    fmt.extend_from_slice(&(sample_rate * u32::from(block_align)).to_le_bytes());
    fmt.extend_from_slice(&block_align.to_le_bytes());
    fmt.extend_from_slice(&(8 * bytes_per_sample).to_le_bytes());
    fmt
}

fn header(channels: u16, sample_rate: u32, n_frames: usize) -> Vec<u8> {
    let data_len = n_frames * 4 * channels as usize;
    let mut out = Vec::with_capacity(64);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((4 + 8 + 16 + 8 + data_len) as u32).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&fmt_body(channels, sample_rate));
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data_len as u32).to_le_bytes());
    out
}

/// The size of a finished file of `n_frames` frames: the 44 bytes of the header and the
/// samples. The plugin checks a cached file against it before the service reads it.
#[must_use]
pub fn file_len(n_frames: usize, channels: u16) -> u64 {
    44 + (n_frames as u64) * 4 * u64::from(channels)
}

impl WavWriter {
    /// Open the temporary file and write the header for `n_frames` frames.
    ///
    /// # Errors
    ///
    /// An [`std::io::Error`] when the cache folder cannot be created or the temporary
    /// file cannot be written.
    pub fn create(dest: &Path, sample_rate: u32, channels: u16, n_frames: usize) -> std::io::Result<WavWriter> {
        if let Some(dir) = dest.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut tmp = dest.as_os_str().to_os_string();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        let mut out = BufWriter::new(std::fs::File::create(&tmp)?);
        out.write_all(&header(channels, sample_rate, n_frames))?;
        Ok(WavWriter {
            out,
            tmp,
            dest: dest.to_path_buf(),
        })
    }

    /// Write one block, frame-major interleaved (the caller interleaves the per-channel
    /// blocks of `readAudioSamples`).
    ///
    /// # Errors
    ///
    /// An [`std::io::Error`] when the write fails.
    pub fn write_interleaved_f32(&mut self, interleaved: &[f32]) -> std::io::Result<()> {
        let mut bytes = Vec::with_capacity(interleaved.len() * 4);
        for v in interleaved {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        self.out.write_all(&bytes)
    }

    /// Flush and rename the temporary file into place.
    ///
    /// # Errors
    ///
    /// An [`std::io::Error`] when the flush or the rename fails.
    pub fn commit(mut self) -> std::io::Result<()> {
        self.out.flush()?;
        std::fs::rename(&self.tmp, &self.dest)
    }
}

/// The first `seconds` of a WAV the plugin wrote, as mono samples (M1b10, the role hint): the header
/// is read, then only the frames needed, never the whole file. The mix to mono is the one of the
/// service (`MixMode::Ffmpeg`, as the scan of the app uses it). Answers the samples, the rate and
/// the bytes read from the file (header included); `None` when the file is not a WAV of this plugin.
#[must_use]
pub fn read_head_mono(path: &Path, seconds: usize) -> Option<(Vec<f32>, u32, usize)> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut head = [0u8; 44];
    f.read_exact(&mut head).ok()?;
    if &head[0..4] != b"RIFF" || &head[8..12] != b"WAVE" || &head[36..40] != b"data" {
        return None;
    }
    let channels = u16::from_le_bytes([head[22], head[23]]);
    let rate = u32::from_le_bytes([head[24], head[25], head[26], head[27]]);
    let data_len = u32::from_le_bytes([head[40], head[41], head[42], head[43]]) as usize;
    if channels == 0 || rate == 0 {
        return None;
    }
    let frame = 4 * channels as usize;
    // what the header claims is capped by what the file holds: a header that lies reserves nothing, and a
    // file shorter than its header decodes what is there (as the whole-file reader does)
    let on_disk = f.metadata().ok()?.len().saturating_sub(44) as usize;
    let frames = (data_len.min(on_disk) / frame).min(rate as usize * seconds);
    let mut data = vec![0u8; frames * frame];
    f.read_exact(&mut data).ok()?;
    let mut buf = header(channels, rate, frames);
    buf.extend_from_slice(&data);
    let info = drumspike_core::WavInfo::parse(&buf).ok()?;
    let mono = info.decode_mono(&buf, drumspike_core::MixMode::Ffmpeg).ok()?;
    Some((mono, rate, 44 + data.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_head_of_a_long_file_is_read_alone() {
        let dir = std::env::temp_dir().join(format!("drumspike-ara-wav-{}-head", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("long.wav");
        // 120 s of stereo at 8 kHz: 7.7 MB on disk; the first 30 s are 1.9 MB
        let rate = 8_000usize;
        let n = rate * 120;
        let mut w = WavWriter::create(&dest, rate as u32, 2, n).unwrap();
        let block: Vec<f32> = (0..rate * 2).map(|i| if i % 2 == 0 { 0.5 } else { -0.5 }).collect();
        for _ in 0..120 {
            w.write_interleaved_f32(&block).unwrap();
        }
        w.commit().unwrap();
        let (mono, sr, bytes) = read_head_mono(&dest, 30).unwrap();
        assert_eq!(sr, rate as u32);
        assert_eq!(mono.len(), rate * 30, "30 s of mono");
        assert_eq!(
            bytes,
            44 + rate * 30 * 8,
            "the header and 30 s of stereo frames, not the whole file"
        );
        assert!(bytes * 3 < file_len(n, 2) as usize, "a quarter of the file");
        assert!(read_head_mono(&dir.join("missing.wav"), 30).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn tmp_dir(label: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("drumspike-ara-wav-{}-{label}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn written_file_round_trips_through_the_service_reader() {
        let dir = tmp_dir("round");
        let dest = dir.join("round.wav");
        let mut w = WavWriter::create(&dest, 48_000, 1, 4).unwrap();
        w.write_interleaved_f32(&[0.5, -0.25, 1e-3, 0.0]).unwrap();
        w.commit().unwrap();
        let (mono, info) = drumspike_core::read_wav_mono(&dest, drumspike_core::MixMode::Ffmpeg).unwrap();
        assert_eq!(info.sample_rate, 48_000);
        assert_eq!(mono, vec![0.5, -0.25, 1e-3, 0.0]);
        // the size the plugin expects of a finished file is the size on disk
        assert_eq!(std::fs::metadata(&dest).unwrap().len(), file_len(4, 1));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_header_that_claims_more_than_the_file_holds_reads_what_is_there() {
        let dir = std::env::temp_dir().join(format!("drumspike-ara-wav-{}-lie", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("lie.wav");
        // the header says 4 billion frames of 8 channels, the file holds 1000 mono-ish frames
        let mut bytes = header(1, 8_000, 1_000_000_000);
        for _ in 0..1000 {
            bytes.extend_from_slice(&0.25f32.to_le_bytes());
        }
        std::fs::write(&dest, &bytes).unwrap();
        let (mono, _, read) = read_head_mono(&dest, 30).unwrap();
        assert_eq!(mono.len(), 1000);
        assert_eq!(read, 44 + 4000);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_header_only_file_and_an_empty_file_read_no_samples() {
        let dir = std::env::temp_dir().join(format!("drumspike-ara-wav-{}-empty", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let head_only = dir.join("head.wav");
        std::fs::write(&head_only, header(1, 8_000, 0)).unwrap();
        let (mono, sr, read) = read_head_mono(&head_only, 30).unwrap();
        assert_eq!((mono.len(), sr, read), (0, 8_000, 44));
        let empty = dir.join("empty.wav");
        std::fs::write(&empty, b"").unwrap();
        assert!(read_head_mono(&empty, 30).is_none(), "an empty file is no WAV");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_failed_writer_leaves_no_destination() {
        let dir = tmp_dir("gone");
        let dest = dir.join("gone.wav");
        let w = WavWriter::create(&dest, 48_000, 2, 2).unwrap();
        drop(w); // no commit: the temporary stays, the destination does not exist
        assert!(!dest.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
