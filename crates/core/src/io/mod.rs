//! Audio file I/O: the memory-mapped WAV/BWF reader, the downmix and a WAV writer.
//!
//! The reader never writes to a source file (invariant I3): files are opened read-only
//! and mapped read-only. The decoded signal is mono `f32` at the file's native rate, folded
//! with an explicit [`MixMode`] so the stereo decision of the Python engine stays visible.

mod atomic;
mod mix;
mod reader;
mod writer;

use std::path::Path;

use rayon::prelude::*;

pub use atomic::{write_bytes_atomic, TempFile};
pub use mix::{ffmpeg_default_mask, ffmpeg_mono_coefs, ffmpeg_stereo_mask_ok, MixMode, Plan};
pub use reader::{read_wav_bytes, read_wav_mono, Bext, MappedWav, SampleFormat, WavError, WavInfo};
pub use writer::{
    encode_bext, encode_samples, encode_wav, encode_wav_raw, write_atomic, write_wav, write_wav_blockwise, WavFormat,
    WavSpec, WRITE_BLOCK_FRAMES,
};

/// Decode several files in parallel (rayon), one result per path in the same order.
pub fn decode_many<P: AsRef<Path> + Sync>(paths: &[P], mode: MixMode) -> Vec<Result<(Vec<f32>, WavInfo), WavError>> {
    paths.par_iter().map(|p| read_wav_mono(p.as_ref(), mode)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_many_keeps_the_order_and_reports_each_error() {
        let dir = std::env::temp_dir().join(format!("drumspike-core-many-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let spec = WavSpec {
            sample_rate: 48_000,
            channels: 1,
            format: WavFormat::Float32,
            extensible: false,
            channel_mask: 0,
        };
        let mut paths = Vec::new();
        for k in 0..6 {
            let p = dir.join(format!("t{k}.wav"));
            write_wav(&p, &spec, &[k as f32 * 0.1; 100], None).unwrap();
            paths.push(p);
        }
        paths.push(dir.join("missing.wav"));
        let r = decode_many(&paths, MixMode::Average);
        assert_eq!(r.len(), 7);
        for (k, item) in r.iter().take(6).enumerate() {
            let (x, info) = item.as_ref().unwrap();
            assert_eq!(info.n_frames, 100);
            assert_eq!(x[0], k as f32 * 0.1);
        }
        assert!(matches!(r[6], Err(WavError::Io(_))));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
