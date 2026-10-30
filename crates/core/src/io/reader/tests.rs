use super::*;
use crate::io::writer::{WavFormat, WavSpec, encode_wav, encode_wav_raw};

#[test]
fn bext_parse_accepts_338_bytes_like_python() {
    let mut c = vec![0u8; 345];
    c[BEXT_DATE..BEXT_DATE + 10].copy_from_slice(b"2026-09-14");
    c[BEXT_TIME..BEXT_TIME + 8].copy_from_slice(b"10:31:02");
    c[BEXT_TIME_REFERENCE] = 7; // partial time reference field: ignored
    for len in [338, 340, 345] {
        let b = Bext::parse(&c[..len]).expect("338 bytes or more parse");
        assert_eq!(b.origination().as_deref(), Some("2026-09-14T10:31:02Z"));
        assert_eq!((b.time_reference, b.version), (0, 0));
    }
    assert!(Bext::parse(&c[..337]).is_none());
    assert!(Bext::parse(&[]).is_none());
    c.resize(346, 0);
    assert_eq!(Bext::parse(&c).unwrap().time_reference, 7);
}

fn spec(format: WavFormat, channels: u16, extensible: bool) -> WavSpec {
    WavSpec {
        sample_rate: 48_000,
        channels,
        format,
        extensible,
        channel_mask: 0,
    }
}

#[test]
fn decode_frames_into_equals_decode_mono_for_every_format_and_block() {
    // 1, 2, 3 and 10 channels in every sample format, both modes, decoded in blocks of
    // odd sizes from odd frame offsets: the block decode equals the full decode bitwise.
    let formats = [WavFormat::Pcm16, WavFormat::Pcm24, WavFormat::Pcm32, WavFormat::Float32];
    for format in formats {
        for ch in [1u16, 2, 3, 10] {
            let n = 257usize;
            let x: Vec<f32> = (0..n * ch as usize)
                .map(|i| ((i * 7919) % 2001) as f32 / 1000.0 - 1.0)
                .collect();
            let b = encode_wav(&spec(format, ch, false), &x, None);
            let info = WavInfo::parse(&b).unwrap();
            for mode in [MixMode::Average, MixMode::Ffmpeg] {
                let Ok(plan) = info.mix_plan(mode) else { continue };
                let full = info.decode_mono(&b, mode).unwrap();
                let mut got = vec![f32::NAN; n];
                let mut at = 3usize;
                got[..3].copy_from_slice(&full[..3]);
                for block in [1usize, 5, 64, 100, 1000].iter().cycle() {
                    if at >= n {
                        break;
                    }
                    let off = info.data_offset + at * info.block_align;
                    let k = info.decode_frames_into(
                        &plan,
                        &b[off..info.data_offset + info.data_len],
                        &mut got[at..(at + block).min(n)],
                    );
                    assert_eq!(k, (*block).min(n - at));
                    at += k;
                }
                for (a, e) in got.iter().zip(&full) {
                    assert_eq!(a.to_bits(), e.to_bits(), "{format:?} {ch} ch {mode:?}");
                }
                // a partial frame at the end and an empty output give 0 frames
                let off = info.data_offset + (n - 1) * info.block_align;
                assert_eq!(
                    info.decode_frames_into(&plan, &b[off..off + info.block_align - 1], &mut got[..4]),
                    0
                );
                assert_eq!(info.decode_frames_into(&plan, &b[info.data_offset..], &mut []), 0);
            }
        }
    }
}

#[test]
fn release_keeps_the_mapped_content() {
    let dir = std::env::temp_dir().join(format!("drumspike-core-release-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("r.wav");
    let x: Vec<f32> = (0..200_000).map(|i| (i % 1000) as f32 / 1000.0).collect();
    crate::io::write_wav(&p, &spec(WavFormat::Float32, 1, false), &x, None).unwrap();
    let m = MappedWav::open(&p).unwrap();
    let before = m.decode_mono(MixMode::Average).unwrap();
    m.release(0, m.bytes().len());
    m.release(10, 5); // empty range: no-op
    m.release(4096, usize::MAX); // clamped to the mapping
    assert_eq!(m.decode_mono(MixMode::Average).unwrap(), before);
    drop(m);
    std::fs::remove_dir_all(&dir).unwrap();
}

fn with_junk(mut file: Vec<u8>) -> Vec<u8> {
    // insert an odd-sized junk chunk before the data chunk to exercise the pad byte
    let at = file.windows(4).position(|w| w == b"data").unwrap();
    let junk = [b"junk".as_slice(), &3u32.to_le_bytes(), &[1, 2, 3, 0]].concat();
    file.splice(at..at, junk);
    let riff = (file.len() - 8) as u32;
    file[4..8].copy_from_slice(&riff.to_le_bytes());
    file
}

#[test]
fn pcm16_stereo_both_modes_and_bext() {
    let mut d = Vec::new();
    for (l, r) in [(16384i16, -16384i16), (32767, 32767), (-32768, 0), (1, 0)] {
        d.extend_from_slice(&l.to_le_bytes());
        d.extend_from_slice(&r.to_le_bytes());
    }
    let bext = Bext {
        description: "parity test".into(),
        originator: "drumspike".into(),
        originator_reference: "ref".into(),
        origination_date: "2026-09-14".into(),
        origination_time: "10:31:02".into(),
        time_reference: 0x0102_0304_0506_0708,
        version: 1,
        coding_history: "A=PCM,F=48000,W=16".into(),
    };
    let b = with_junk(encode_wav_raw(&spec(WavFormat::Pcm16, 2, false), &d, Some(&bext)));
    let info = WavInfo::parse(&b).unwrap();
    assert_eq!(info.bext.as_ref(), Some(&bext));
    assert_eq!(info.time_reference(), Some(0x0102_0304_0506_0708));
    assert_eq!(
        info.bext.as_ref().unwrap().origination().as_deref(),
        Some("2026-09-14T10:31:02Z")
    );
    assert_eq!(info.n_frames, 4);
    assert_eq!(info.ixml, None);
    let x = info.decode_mono(&b, MixMode::Average).unwrap();
    // ffmpeg pan on s16: (L + R + 1) >> 1
    assert_eq!(x, vec![0.0, 32767.0 / 32768.0, -0.5, 1.0 / 32768.0]);
    let y = info.decode_mono(&b, MixMode::Ffmpeg).unwrap();
    let c = std::f32::consts::FRAC_1_SQRT_2;
    assert_eq!(y[0], 0.0);
    assert_eq!(y[1], (32767.0f32 / 32768.0).mul_add(c, (32767.0f32 / 32768.0) * c));
    assert!(
        y[1] > 1.0,
        "ffmpeg mix of correlated full-scale stereo exceeds full scale"
    );
}

#[test]
fn pcm24_extensible_with_ixml() {
    let d = [0x00, 0x00, 0x80, 0xFF, 0xFF, 0x7F, 0x01, 0x00, 0x00]; // -2^23, 2^23-1, 1
    let mut b = encode_wav_raw(&spec(WavFormat::Pcm24, 1, true), &d, None);
    let xml = b"<BWFXML><IXML_VERSION>1.5</IXML_VERSION></BWFXML>\0";
    let at = b.windows(4).position(|w| w == b"data").unwrap();
    let chunk = [b"iXML".as_slice(), &(xml.len() as u32).to_le_bytes(), xml].concat();
    b.splice(at..at, chunk);
    let info = WavInfo::parse(&b).unwrap();
    assert!(info.extensible && info.bytes_per_sample == 3 && info.format_tag == 1);
    assert_eq!(
        info.ixml.as_deref(),
        Some("<BWFXML><IXML_VERSION>1.5</IXML_VERSION></BWFXML>")
    );
    let x = info.decode_mono(&b, MixMode::Average).unwrap();
    assert_eq!(x, vec![-1.0, 8388607.0 / 8388608.0, 1.0 / 8388608.0]);
}

#[test]
fn float32_extensible() {
    let mut d = Vec::new();
    for v in [0.25f32, -1.0, 1e-7] {
        d.extend_from_slice(&v.to_le_bytes());
    }
    let b = encode_wav_raw(&spec(WavFormat::Float32, 1, true), &d, None);
    let info = WavInfo::parse(&b).unwrap();
    assert_eq!(info.sample_format, SampleFormat::Float);
    assert_eq!(info.decode_mono(&b, MixMode::Ffmpeg).unwrap(), vec![0.25, -1.0, 1e-7]);
}

#[test]
fn pcm32_mono_scales_by_2_pow_31() {
    let mut d = Vec::new();
    for v in [i32::MIN, i32::MAX, 1 << 8] {
        d.extend_from_slice(&v.to_le_bytes());
    }
    let b = encode_wav_raw(&spec(WavFormat::Pcm32, 1, false), &d, None);
    let info = WavInfo::parse(&b).unwrap();
    assert_eq!(info.bytes_per_sample, 4);
    let x = info.decode_mono(&b, MixMode::Ffmpeg).unwrap();
    assert_eq!(x, vec![-1.0, i32::MAX as f32 / 2147483648.0, 1.0 / 8388608.0]);
}

#[test]
fn valid_bits_below_the_container_decode_as_the_container() {
    // Extensible PCM, 24 valid bits in a 32-bit container (wValidBitsPerSample = 24,
    // 4 bytes per sample): the samples are left-justified, so the 32-bit decode is the
    // 24-bit value. ffmpeg 9.0.2 probes this layout as pcm_f24le and decodes garbage
    // (NaN, 1e31), so the Python path differs here on purpose (docs/archive/phase1-report.md,
    // Deviations); this test pins the Rust behaviour.
    let v24 = [-0x80_0000i32, 0x7F_FFFF, 1, -1, 0x12_3456];
    let mut d = Vec::new();
    for v in v24 {
        d.extend_from_slice(&(v << 8).to_le_bytes());
    }
    for (channels, mask) in [(1u16, 0x4u32), (2, 0x3)] {
        let mut s = spec(WavFormat::Pcm32, channels, true);
        s.channel_mask = mask;
        let mut b = encode_wav_raw(&s, &d[..d.len() / 4 / channels as usize * 4 * channels as usize], None);
        let fmt_body = b.windows(4).position(|w| w == b"fmt ").unwrap() + 8;
        assert_eq!(u16_at(&b, fmt_body + 18), 32, "wValidBitsPerSample of the writer");
        b[fmt_body + 18..fmt_body + 20].copy_from_slice(&24u16.to_le_bytes());
        let info = WavInfo::parse(&b).unwrap();
        assert_eq!((info.bits_per_sample, info.bytes_per_sample), (32, 4));
        let x = info.decode_mono(&b, MixMode::Ffmpeg).unwrap();
        if channels == 1 {
            let want: Vec<f32> = v24.iter().map(|v| *v as f32 / 8_388_608.0).collect();
            assert_eq!(x, want);
        } else {
            let c = std::f32::consts::FRAC_1_SQRT_2;
            let f = |v: i32| v as f32 / 8_388_608.0;
            assert_eq!(
                x,
                vec![f(v24[0]).mul_add(c, f(v24[1]) * c), f(v24[2]).mul_add(c, f(v24[3]) * c)]
            );
        }
    }
}

#[test]
fn sample_width_comes_from_bits_and_a_disagreeing_block_align_is_refused() {
    // (format, channels, extensible, wBitsPerSample, nBlockAlign): ffmpeg decodes these
    // by wBitsPerSample, so the reader refuses them and the binding hands them to ffmpeg
    // (review finding M1). Plain 24-bit in a 4-byte block is refused as well.
    let refused = [
        (WavFormat::Pcm16, 1u16, false, 16u16, 4u16),
        (WavFormat::Pcm16, 1, false, 16, 3),
        (WavFormat::Pcm16, 1, true, 16, 4),
        (WavFormat::Pcm16, 2, true, 16, 6),
        (WavFormat::Pcm24, 1, false, 24, 2),
        (WavFormat::Pcm32, 1, false, 32, 3),
        (WavFormat::Pcm32, 1, false, 24, 4),
        (WavFormat::Pcm32, 2, false, 24, 8),
        (WavFormat::Float32, 1, false, 32, 8),
    ];
    for (format, channels, extensible, bits, ba) in refused {
        let n = 2 * channels as usize * format.bytes_per_sample() as usize;
        let mut b = encode_wav_raw(&spec(format, channels, extensible), &vec![0u8; n], None);
        let f = b.windows(4).position(|w| w == b"fmt ").unwrap() + 8;
        b[f + 12..f + 14].copy_from_slice(&ba.to_le_bytes());
        b[f + 14..f + 16].copy_from_slice(&bits.to_le_bytes());
        let e = WavInfo::parse(&b).unwrap_err();
        assert!(
            matches!(&e, WavError::Format(m) if m.starts_with(&format!("block align {ba} does not match"))),
            "{format:?} ch {channels} bits {bits} ba {ba}: {e}"
        );
    }
    // the width is ceil(bits / 8): 12 bits in 2 bytes and 20 bits in 3 bytes decode as
    // the container, and a zero block align takes the width from the bits
    for (format, bits, ba, width) in [
        (WavFormat::Pcm16, 12u16, 2u16, 2usize),
        (WavFormat::Pcm24, 20, 3, 3),
        (WavFormat::Pcm24, 24, 0, 3),
    ] {
        let d = vec![0x10u8; 3 * format.bytes_per_sample() as usize];
        let mut b = encode_wav_raw(&spec(format, 1, false), &d, None);
        let f = b.windows(4).position(|w| w == b"fmt ").unwrap() + 8;
        b[f + 12..f + 14].copy_from_slice(&ba.to_le_bytes());
        b[f + 14..f + 16].copy_from_slice(&bits.to_le_bytes());
        let info = WavInfo::parse(&b).unwrap();
        assert_eq!(
            (info.bytes_per_sample, info.block_align, info.n_frames),
            (width, width, 3)
        );
        let ok = encode_wav_raw(&spec(format, 1, false), &d, None);
        assert_eq!(
            info.decode_mono(&b, MixMode::Ffmpeg).unwrap(),
            WavInfo::parse(&ok).unwrap().decode_mono(&ok, MixMode::Ffmpeg).unwrap()
        );
    }
}

#[test]
fn a_data_chunk_of_size_zero_runs_to_the_end_of_the_file() {
    // review round 3, finding M1: ffmpeg reads a data chunk of size 0 to the end of the
    // file, with the RIFF size 0 or correct, and so does the reader
    let mut d = Vec::new();
    for v in [1000i16, -1000, 0, 5] {
        d.extend_from_slice(&v.to_le_bytes());
    }
    let ok = encode_wav_raw(&spec(WavFormat::Pcm16, 1, false), &d, None);
    let want = WavInfo::parse(&ok).unwrap().decode_mono(&ok, MixMode::Ffmpeg).unwrap();
    let at = ok.windows(4).position(|w| w == b"data").unwrap() + 4;
    for riff_zero in [false, true] {
        let mut b = ok.clone();
        b[at..at + 4].copy_from_slice(&0u32.to_le_bytes());
        if riff_zero {
            b[4..8].copy_from_slice(&0u32.to_le_bytes());
        }
        let info = WavInfo::parse(&b).unwrap();
        assert_eq!((info.data_offset, info.data_len, info.n_frames), (at + 4, 8, 4));
        assert_eq!(info.decode_mono(&b, MixMode::Ffmpeg).unwrap(), want);
        // bytes after the samples are read as samples too (ffmpeg does not look for
        // chunks after a data chunk of size 0); an odd tail is not a whole frame
        b.extend_from_slice(&[7, 0, 9]);
        let info = WavInfo::parse(&b).unwrap();
        assert_eq!((info.data_len, info.n_frames), (11, 5));
        assert_eq!(info.decode_mono(&b, MixMode::Ffmpeg).unwrap()[4], 7.0 / 32768.0);
    }
    // a data chunk of size 0 with nothing after it is an empty signal
    let mut empty = encode_wav_raw(&spec(WavFormat::Pcm16, 1, false), &[], None);
    let n = empty.len();
    empty[n - 4..].copy_from_slice(&0u32.to_le_bytes());
    assert_eq!(WavInfo::parse(&empty).unwrap().n_frames, 0);
}

#[test]
fn mix_plan_uses_the_mask_only_of_an_extensible_header() {
    let frame = [0.1f32, 0.2, 0.3, 0.4];
    let mut d = Vec::new();
    for v in frame {
        d.extend_from_slice(&v.to_le_bytes());
    }
    let mut s = spec(WavFormat::Float32, 4, true);
    s.channel_mask = 0x33; // quad: ffmpeg has no -ac 1 rule for it
    let b = encode_wav_raw(&s, &d, None);
    let mut info = WavInfo::parse(&b).unwrap();
    assert!(matches!(info.mix_plan(MixMode::Ffmpeg), Err(WavError::Format(m)) if m.contains("channel mask")));
    let avg = info.mix_plan(MixMode::Average).unwrap();
    assert_eq!(
        avg.fold_f32(&frame),
        Plan::new(MixMode::Average, 4, 0x33).unwrap().fold_f32(&frame)
    );
    // a plain header has no mask: the default layout of the channel count
    info.extensible = false;
    let plain = info.mix_plan(MixMode::Ffmpeg).unwrap();
    assert_eq!(
        plain.fold_f32(&frame),
        Plan::new(MixMode::Ffmpeg, 4, 0).unwrap().fold_f32(&frame)
    );
    info.channels = 0;
    assert!(matches!(info.mix_plan(MixMode::Average), Err(WavError::Format(m)) if m == "zero channels"));
}

#[test]
fn four_channels_default_layout_and_quad_mask() {
    let mut d = Vec::new();
    for v in [0.1f32, 0.2, 0.3, 0.4] {
        d.extend_from_slice(&v.to_le_bytes());
    }
    let mut s = spec(WavFormat::Float32, 4, true);
    s.channel_mask = 0x107;
    let b = encode_wav_raw(&s, &d, None);
    let info = WavInfo::parse(&b).unwrap();
    assert_eq!(info.channel_mask, 0x107);
    let c = std::f32::consts::FRAC_1_SQRT_2;
    let want = 0.4f32.mul_add(0.5, 0.3f32.mul_add(1.0, 0.2f32.mul_add(c, 0.1f32.mul_add(c, 0.0))));
    assert_eq!(info.decode_mono(&b, MixMode::Ffmpeg).unwrap(), vec![want]);
    let avg = 0.4f32.mul_add(
        0.25,
        0.3f32.mul_add(0.25, 0.2f32.mul_add(0.25, 0.1f32.mul_add(0.25, 0.0))),
    );
    assert_eq!(info.decode_mono(&b, MixMode::Average).unwrap(), vec![avg]);
    s.channel_mask = 0x33; // quad: FL FR BL BR
    let b = encode_wav_raw(&s, &d, None);
    let info = WavInfo::parse(&b).unwrap();
    assert!(matches!(info.decode_mono(&b, MixMode::Ffmpeg), Err(WavError::Format(m)) if m.contains("channel mask")));
    assert!(info.decode_mono(&b, MixMode::Average).is_ok());
}

#[test]
fn rejects_rf64_non_wav_short_bext_and_missing_chunks() {
    let mut rf64 = encode_wav_raw(&spec(WavFormat::Pcm16, 1, false), &[0, 0], None);
    rf64[0..4].copy_from_slice(b"RF64");
    assert!(matches!(WavInfo::parse(&rf64), Err(WavError::Format(m)) if m.starts_with("RF64/BW64")));
    rf64[0..4].copy_from_slice(b"BW64");
    assert!(matches!(WavInfo::parse(&rf64), Err(WavError::Format(m)) if m.starts_with("RF64/BW64")));
    assert!(matches!(WavInfo::parse(b"RIFF\0\0\0\0AIFF"), Err(WavError::Format(_))));
    assert!(matches!(WavInfo::parse(b"RIF"), Err(WavError::Format(_))));
    let no_data = encode_wav_raw(&spec(WavFormat::Pcm16, 1, false), &[], None);
    let cut = &no_data[..no_data.len() - 8];
    assert!(matches!(WavInfo::parse(cut), Err(WavError::Format(s)) if s.contains("no data chunk")));
    // a bext shorter than the time reference is ignored, like project.bext_origination does
    let mut short = encode_wav_raw(&spec(WavFormat::Pcm16, 1, false), &[0, 0], None);
    let at = short.windows(4).position(|w| w == b"data").unwrap();
    short.splice(
        at..at,
        [b"bext".as_slice(), &4u32.to_le_bytes(), &[0, 0, 0, 0]].concat(),
    );
    assert_eq!(WavInfo::parse(&short).unwrap().bext, None);
    // unsupported encodings
    let mut mulaw = encode_wav_raw(&spec(WavFormat::Pcm16, 1, false), &[0, 0], None);
    let fmt_at = mulaw.windows(4).position(|w| w == b"fmt ").unwrap() + 8;
    mulaw[fmt_at..fmt_at + 2].copy_from_slice(&7u16.to_le_bytes());
    assert!(matches!(WavInfo::parse(&mulaw), Err(WavError::Format(m)) if m.contains("format tag 7")));
    let mut eight = encode_wav_raw(&spec(WavFormat::Pcm16, 1, false), &[0, 0], None);
    eight[fmt_at + 12..fmt_at + 14].copy_from_slice(&1u16.to_le_bytes());
    eight[fmt_at + 14..fmt_at + 16].copy_from_slice(&8u16.to_le_bytes());
    assert!(matches!(WavInfo::parse(&eight), Err(WavError::Format(m)) if m.contains("1 bytes/sample")));
}

#[test]
fn open_and_read_a_file() {
    let dir = std::env::temp_dir().join(format!("drumspike-core-reader-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("t.wav");
    let mut d = Vec::new();
    for v in [1000i16, -1000, 0] {
        d.extend_from_slice(&v.to_le_bytes());
    }
    std::fs::write(&p, encode_wav_raw(&spec(WavFormat::Pcm16, 1, false), &d, None)).unwrap();
    let (x, info) = read_wav_mono(&p, MixMode::Ffmpeg).unwrap();
    assert_eq!(info.n_frames, 3);
    assert_eq!(x, vec![1000.0 / 32768.0, -1000.0 / 32768.0, 0.0]);
    let m = MappedWav::open(&p).unwrap();
    assert_eq!(m.bytes().len(), std::fs::metadata(&p).unwrap().len() as usize);
    assert_eq!(m.decode_mono(MixMode::Average).unwrap(), x);
    assert!(matches!(
        MappedWav::open(&dir.join("missing.wav")),
        Err(WavError::Io(_))
    ));
    let e = read_wav_mono(&dir.join("missing.wav"), MixMode::Ffmpeg).unwrap_err();
    assert!(e.to_string().starts_with("io:"));
    let (b, info2) = read_wav_bytes(&p).unwrap();
    assert_eq!(b, std::fs::read(&p).unwrap());
    assert_eq!((info2.n_frames, info2.data_offset), (info.n_frames, info.data_offset));
    assert_eq!(info2.decode_mono(&b, MixMode::Ffmpeg).unwrap(), x);
    assert!(matches!(read_wav_bytes(&dir.join("missing.wav")), Err(WavError::Io(_))));
    std::fs::write(dir.join("bad.wav"), b"RIFF\0\0\0\0WAVE").unwrap();
    assert!(matches!(read_wav_bytes(&dir.join("bad.wav")), Err(WavError::Format(m)) if m == "no fmt chunk"));
    std::fs::remove_dir_all(&dir).unwrap();
}
