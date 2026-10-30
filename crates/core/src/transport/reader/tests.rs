use super::*;
use crate::io::{WavFormat, WavSpec, write_wav};

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("drumspike-transport-reader-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write(path: &Path, sr: u32, ch: u16, format: WavFormat, x: &[f32]) {
    let spec = WavSpec {
        sample_rate: sr,
        channels: ch,
        format,
        extensible: false,
        channel_mask: 0,
    };
    write_wav(path, &spec, x, None).unwrap();
}

/// Every channel of a file, interleaved, as the playback of a stereo file reads it (P3-16).
fn channel_decode(p: &Path) -> Vec<f32> {
    let m = MappedWav::open(p).unwrap();
    let i = &m.info;
    let data = &m.bytes()[i.data_offset..i.data_offset + i.n_frames * i.block_align];
    let mut out = vec![0.0f32; i.n_frames * i.channels as usize];
    assert_eq!(i.decode_channels_into(data, &mut out), i.n_frames);
    out
}

#[test]
fn fill_equals_the_full_decode_then_zeros_in_both_residencies() {
    let d = tmp("fill");
    // P3-16: a stereo file keeps left and right (interleaved); a 3-channel file plays the mono fold
    for (ch, mode) in [(2u16, MixMode::Ffmpeg), (3, MixMode::Average)] {
        let p = d.join(format!("a{ch}.wav"));
        let x: Vec<f32> = (0..ch as usize * 70_001)
            .map(|i| ((i * 31) % 997) as f32 / 997.0 - 0.5)
            .collect();
        write(&p, 48_000, ch, WavFormat::Pcm24, &x);
        let (full, c) = if ch == 2 {
            (channel_decode(&p), 2)
        } else {
            (crate::io::read_wav_mono(&p, mode).unwrap().0, 1)
        };
        for res in [Residency::Madvise, Residency::Pread] {
            let mut s = TrackSource::open(&p, mode, 48_000, res).unwrap();
            assert!(!s.resampled());
            assert_eq!(s.channels(), c);
            assert_eq!(s.track_rate(), 48_000);
            assert_eq!(s.device_frames(), 70_001);
            let mut got = vec![0.0f32; 80_000 * c];
            for k in got.chunks_mut(9_999 * c) {
                assert_eq!(s.fill(k), 0);
            }
            assert_eq!(full.len(), 70_001 * c);
            for (a, e) in got.iter().zip(&full) {
                assert_eq!(a.to_bits(), e.to_bits());
            }
            assert!(got[70_001 * c..].iter().all(|v| *v == 0.0));
            s.seek(12_345);
            let mut again = vec![0.0f32; 100 * c];
            s.fill(&mut again);
            assert_eq!(&again[..], &full[12_345 * c..12_445 * c]);
        }
    }
    // the left and right of the stereo file are its two channels, each as a mono file's sample
    let p = d.join("a2.wav");
    let lr = channel_decode(&p);
    let x: Vec<f32> = (0..2 * 70_001).map(|i| ((i * 31) % 997) as f32 / 997.0 - 0.5).collect();
    assert!(
        lr.iter().zip(&x).all(|(a, b)| (a - b).abs() < 1e-6),
        "24-bit quantisation only"
    );
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn a_stereo_file_resamples_both_channels_alike() {
    let d = tmp("rs2");
    let p = d.join("s.wav");
    // the same signal on both sides, one side inverted: the resampled sides stay exact opposites
    let x: Vec<f32> = (0..96_000)
        .flat_map(|i| {
            let v = (i as f32 * 0.01).sin() * 0.5;
            [v, -v]
        })
        .collect();
    write(&p, 96_000, 2, WavFormat::Float32, &x);
    let mut s = TrackSource::open(&p, MixMode::Average, 48_000, Residency::Pread).unwrap();
    assert!(s.resampled() && s.channels() == 2);
    let mut a = vec![0.0f32; 2 * 20_000];
    s.fill(&mut a);
    assert!(a.chunks(2).all(|f| f[0] == -f[1]));
    assert!(a.chunks(2).any(|f| f[0].abs() > 0.4));
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn resampled_fill_does_not_depend_on_the_block_sizes_or_the_previous_position() {
    let d = tmp("rs");
    let p = d.join("b.wav");
    let x: Vec<f32> = (0..96_000).map(|i| (i as f32 * 0.01).sin() * 0.5).collect();
    write(&p, 96_000, 1, WavFormat::Float32, &x);
    let mut s = TrackSource::open(&p, MixMode::Average, 48_000, Residency::Madvise).unwrap();
    assert!(s.resampled());
    assert_eq!(s.device_frames(), 48_000);
    let mut a = vec![0.0f32; 20_000];
    let ns = s.fill(&mut a);
    assert!(ns > 0);
    let mut b = vec![0.0f32; 20_000];
    s.seek(0);
    for c in b.chunks_mut(777) {
        s.fill(c);
    }
    assert_eq!(a, b);
    s.seek(30_000);
    let mut c1 = vec![0.0f32; 1_000];
    s.fill(&mut c1);
    s.seek(5);
    s.seek(30_000);
    let mut c2 = vec![0.0f32; 1_000];
    s.fill(&mut c2);
    assert_eq!(c1, c2);
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn pread_reads_blocks_of_at_least_256_kib_and_decodes_the_same_bits() {
    let d = tmp("block");
    let p = d.join("c.wav");
    // 24-bit mono, 400,000 frames = 1.2 MB: about 5 blocks of 256 KiB
    let x: Vec<f32> = (0..400_000u64)
        .map(|i| ((i * 7919) % 65_521) as f32 / 65_521.0 - 0.5)
        .collect();
    write(&p, 48_000, 1, WavFormat::Pcm24, &x);
    let (full, _) = crate::io::read_wav_mono(&p, MixMode::Average).unwrap();
    let mut s = TrackSource::open(&p, MixMode::Average, 48_000, Residency::Pread).unwrap();
    let mut got = vec![0.0f32; 410_000];
    for c in got.chunks_mut(1_024) {
        s.fill(c);
    }
    for (a, e) in got.iter().zip(&full) {
        assert_eq!(a.to_bits(), e.to_bits());
    }
    assert!(got[400_000..].iter().all(|v| *v == 0.0));
    let blocks = (400_000 * 3usize).div_ceil(READ_BLOCK) as u64;
    // a request that crosses the end of the buffer reads again from its own start
    assert!(s.reads() <= blocks + 2, "{} reads for {blocks} blocks", s.reads());
    // a request larger than the block reads its own size in one read
    s.seek(0);
    let r0 = s.reads();
    let mut big = vec![0.0f32; 131_072];
    s.fill(&mut big);
    assert_eq!(s.reads() - r0, 1);
    assert_eq!(&big[..], &full[..131_072]);
    // a seek back inside the buffer needs no read
    s.seek(1_000);
    let mut back = vec![0.0f32; 500];
    s.fill(&mut back);
    assert_eq!(s.reads() - r0, 1);
    assert_eq!(&back[..], &full[1_000..1_500]);
    let m = TrackSource::open(&p, MixMode::Average, 48_000, Residency::Madvise).unwrap();
    assert_eq!(m.reads(), 0);
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn a_file_shorter_than_its_header_gives_zeros_only_where_the_bytes_are_missing() {
    let d = tmp("short");
    let p = d.join("s.wav");
    let x: Vec<f32> = (0..200_000)
        .map(|i| ((i * 13) % 1_001) as f32 / 1_001.0 - 0.5)
        .collect();
    write(&p, 48_000, 1, WavFormat::Pcm16, &x);
    let (full, _) = crate::io::read_wav_mono(&p, MixMode::Average).unwrap();
    let mut s = TrackSource::open(&p, MixMode::Average, 48_000, Residency::Pread).unwrap();
    // cut the file after the source opened it: 150,000 whole frames stay
    let data_offset = s.info.data_offset as u64;
    std::fs::OpenOptions::new()
        .write(true)
        .open(&p)
        .unwrap()
        .set_len(data_offset + 150_000 * 2)
        .unwrap();
    let mut got = vec![0.0f32; 200_000];
    for c in got.chunks_mut(1_000) {
        s.fill(c);
    }
    // the blocks before the cut decode as before; a 1,000-frame request past the cut is zeros
    for (a, e) in got[..150_000].iter().zip(&full[..150_000]) {
        assert_eq!(a.to_bits(), e.to_bits());
    }
    assert!(got[150_000..].iter().all(|v| *v == 0.0));
    std::fs::remove_dir_all(&d).unwrap();
}

/// P3-20b: a speed change switches the source to another output rate; the resampler of each
/// rate is built once and kept; the device rate gives the plain decode again, bit for bit.
#[test]
fn set_out_rate_switches_the_resampler_and_keeps_the_old_one() {
    let d = tmp("outrate");
    let p = d.join("o.wav");
    let x: Vec<f32> = (0..48_000).map(|i| (i as f32 * 0.013).sin() * 0.5).collect();
    write(&p, 48_000, 1, WavFormat::Float32, &x);
    let (full, _) = crate::io::read_wav_mono(&p, MixMode::Average).unwrap();
    let mut s = TrackSource::open(&p, MixMode::Average, 48_000, Residency::Pread).unwrap();
    assert_eq!((s.out_rate(), s.resampled()), (48_000, false));
    s.set_out_rate(96_000).unwrap();
    s.seek(2_000);
    assert_eq!((s.out_rate(), s.resampled(), s.device_frames()), (96_000, true, 48_000));
    let mut up = vec![0.0f32; 4_000];
    assert!(s.fill(&mut up) > 0);
    // output frame 2k of 96 kHz is input frame k: the even frames are the samples (within the filter's error)
    for k in (0..4_000).step_by(2) {
        assert!(
            (up[k] - full[1_000 + k / 2]).abs() < 1e-4,
            "frame {k}: {} {}",
            up[k],
            full[1_000 + k / 2]
        );
    }
    s.set_out_rate(48_000).unwrap();
    s.seek(1_000);
    assert_eq!((s.spare.len(), s.resampled()), (1, false));
    let mut same = vec![0.0f32; 3_000];
    assert_eq!(s.fill(&mut same), 0);
    assert!(same.iter().zip(&full[1_000..]).all(|(a, b)| a.to_bits() == b.to_bits()));
    // back to 96 kHz: the kept resampler, the same frames as before
    s.set_out_rate(96_000).unwrap();
    assert_eq!(
        s.spare.len(),
        1,
        "the 48 kHz entry; the 96 kHz resampler came out of the list"
    );
    s.seek(2_000);
    let mut again = vec![0.0f32; 4_000];
    s.fill(&mut again);
    assert_eq!(up, again);
    // a rate without a resampler gives silence and the error, until the next change
    assert!(s.set_out_rate(0).is_err());
    let mut z = vec![1.0f32; 100];
    s.fill(&mut z);
    assert!(z.iter().all(|v| *v == 0.0));
    s.set_out_rate(48_000).unwrap();
    s.seek(0);
    s.fill(&mut z);
    assert_eq!(&z[..], &full[..100]);
    let e = TrackSource::open_at(&p, MixMode::Average, 48_000, 0, Residency::Pread);
    assert!(matches!(e, Err(TransportError::Track(_))));
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn the_reader_thread_can_ask_for_qos_user_interactive() {
    let ok = std::thread::spawn(set_reader_qos).join().unwrap();
    assert_eq!(ok, cfg!(target_os = "macos"));
}

#[test]
fn open_reports_missing_files() {
    let e = TrackSource::open(
        Path::new("/nonexistent/x.wav"),
        MixMode::Average,
        48_000,
        Residency::Pread,
    );
    assert!(matches!(e, Err(TransportError::Track(_))));
}

#[test]
fn gcd_works() {
    assert_eq!(gcd(44_100, 48_000), 300);
    assert_eq!(gcd(96_000, 48_000), 48_000);
}
