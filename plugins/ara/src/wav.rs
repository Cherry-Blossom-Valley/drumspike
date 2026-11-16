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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn a_failed_writer_leaves_no_destination() {
        let dir = tmp_dir("gone");
        let dest = dir.join("gone.wav");
        let w = WavWriter::create(&dest, 48_000, 2, 2).unwrap();
        drop(w); // no commit: the temporary stays, the destination does not exist
        assert!(!dest.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
