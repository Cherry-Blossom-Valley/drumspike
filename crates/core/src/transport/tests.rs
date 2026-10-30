use super::*;
use crate::io::{WavFormat, WavSpec, read_wav_mono, write_wav};
use std::path::Path;

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("drumspike-transport-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
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

fn spec(path: PathBuf, mode: MixMode, hits: Vec<u64>, pitch: f32) -> TrackSpec {
    TrackSpec {
        path,
        mode,
        hits,
        click_pitch: pitch,
    }
}

/// Small rings so a short session wraps them many times.
fn small_cfg(buffer: u32) -> TransportConfig {
    TransportConfig {
        buffer_frames: buffer,
        ring_frames: 1 << 15,
        prefill_frames: 1 << 13,
        clicks_on: false,
        ..TransportConfig::default()
    }
}

/// 30 short tracks at 48 kHz in mixed formats and both mix modes, noise-like content.
fn noise_session(dir: &Path) -> (Vec<TrackSpec>, Vec<[Vec<f32>; 2]>) {
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut rnd = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        ((seed >> 40) as f32 / (1u64 << 24) as f32) * 1.6 - 0.8
    };
    let kinds = [
        (1u16, WavFormat::Pcm16, MixMode::Average),
        (1, WavFormat::Pcm24, MixMode::Average),
        (1, WavFormat::Float32, MixMode::Average),
        (2, WavFormat::Pcm24, MixMode::Average),
        (2, WavFormat::Pcm16, MixMode::Ffmpeg),
        (1, WavFormat::Pcm32, MixMode::Ffmpeg),
    ];
    let mut specs = Vec::new();
    let mut decoded = Vec::new();
    for t in 0..30usize {
        let (ch, format, mode) = kinds[t % kinds.len()];
        let frames = 30_000 + 997 * t;
        let x: Vec<f32> = (0..frames * ch as usize).map(|_| rnd()).collect();
        let p = dir.join(format!("t{t:02}.wav"));
        write(&p, 48_000, ch, format, &x);
        decoded.push(sides(&p, mode));
        specs.push(spec(p, mode, vec![], 1000.0));
    }
    (specs, decoded)
}

/// Left and right of a track as the transport plays it (P3-16): the two channels of a
/// stereo file, else the mono fold on both sides.
fn sides(p: &Path, mode: MixMode) -> [Vec<f32>; 2] {
    let m = crate::io::MappedWav::open(p).unwrap();
    let i = &m.info;
    if i.channels != 2 {
        let x = read_wav_mono(p, mode).unwrap().0;
        return [x.clone(), x];
    }
    let data = &m.bytes()[i.data_offset..i.data_offset + i.n_frames * i.block_align];
    let mut lr = vec![0.0f32; 2 * i.n_frames];
    i.decode_channels_into(data, &mut lr);
    [
        lr.iter().step_by(2).copied().collect(),
        lr.iter().skip(1).step_by(2).copied().collect(),
    ]
}

/// The reference (interleaved stereo): per channel, the tracks with a nonzero gain in
/// order, `s += g * x` (the track's own side for a stereo file).
fn reference(decoded: &[[Vec<f32>; 2]], gains: &[f32], from: usize, n: usize) -> Vec<f32> {
    reference_lr(decoded, gains, gains, 1.0, from, n)
}

fn assert_stereo_bits(out: &[f32], want: &[f32], what: &str) {
    assert_eq!(out.len(), want.len(), "{what}: length");
    for (i, (o, w)) in out.iter().zip(want).enumerate() {
        let side = if i % 2 == 0 { "left" } else { "right" };
        assert_eq!(o.to_bits(), w.to_bits(), "{what}: {side} at frame {}", i / 2);
    }
}

#[test]
fn transport_offline_render_equals_the_reference_sum_bitwise() {
    let d = tmp("bitwise");
    let (specs, decoded) = noise_session(&d);
    let ones = vec![1.0f32; 30];
    for buffer in [256u32, 300, 4_096] {
        let mut o = Offline::new(&specs, 48_000, small_cfg(buffer)).unwrap();
        let n = o.session_frames() as usize + 1_000;
        assert_eq!(o.session_frames() as usize, 30_000 + 997 * 29);
        let out = o.render(n);
        assert_stereo_bits(&out, &reference(&decoded, &ones, 0, n), &format!("buffer {buffer}"));
        assert_eq!(o.stats().underruns, 0);
        assert_eq!(o.frame_pos(), n as u64);
    }
    // the master gain scales after the mix; the peak is measured before it
    let mut o = Offline::new(&specs, 48_000, small_cfg(256)).unwrap();
    o.set_master_gain(0.0);
    let out = o.render(10_000);
    assert!(out.iter().all(|v| *v == 0.0));
    let r = reference(&decoded, &ones, 0, 10_000);
    let peak = r.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert_eq!(o.stats().peak, peak);
    std::fs::remove_dir_all(&d).unwrap();
}

/// The P3-16 reference: per channel, the tracks with a nonzero channel gain in order,
/// `s += g * x`, then times the master volume.
fn reference_lr(decoded: &[[Vec<f32>; 2]], gl: &[f32], gr: &[f32], mvol: f32, from: usize, n: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(2 * n);
    for i in from..from + n {
        for (c, g) in [gl, gr].into_iter().enumerate() {
            let mut s = 0.0f32;
            for (x, g) in decoded.iter().zip(g) {
                if *g != 0.0 {
                    s += g * x[c].get(i).copied().unwrap_or(0.0);
                }
            }
            out.push(s * mvol);
        }
    }
    out
}

#[test]
fn transport_volume_pan_and_master_volume_follow_the_mix_order_bitwise() {
    let d = tmp("volpan");
    let (specs, decoded) = noise_session(&d);
    let mut o = Offline::new(&specs, 48_000, small_cfg(256)).unwrap();
    // 0 dB at the centre is the old mix (set explicitly: the same bits as never set)
    for t in 0..30 {
        o.set_volume_pan(t, 0.0, 0.0).unwrap();
    }
    o.set_master_volume_db(0.0);
    let a = o.render(4_096);
    assert_stereo_bits(&a, &reference(&decoded, &[1.0; 30], 0, 4_096), "0 dB, centre");
    // volume, pan and the master volume
    let mut gl = vec![1.0f32; 30];
    let mut gr = vec![1.0f32; 30];
    for (t, db, pan) in [
        (3usize, -6.0f32, -0.5f32),
        (5, 3.0, 1.0),
        (7, -200.0, 0.0),
        (11, 0.0, -1.0),
        (12, 12.0, 0.25),
    ] {
        o.set_volume_pan(t, db, pan).unwrap();
        let (l, r) = mixer::channel_gains(db, pan);
        gl[t] = l;
        gr[t] = r;
    }
    assert_eq!((gl[7], gr[7]), (0.0, 0.0), "-200 dB is off");
    assert_eq!(gr[11], 0.0, "hard left");
    o.set_master_volume_db(-3.0);
    let mvol = mixer::db_to_gain(-3.0);
    let b = o.render(4_096);
    let want = reference_lr(&decoded, &gl, &gr, mvol, 4_096, 4_096);
    assert_eq!(b.len(), want.len());
    for (i, (x, w)) in b.iter().zip(&want).enumerate() {
        assert_eq!(x.to_bits(), w.to_bits(), "frame {} channel {}", i / 2, i % 2);
    }
    assert!(b.chunks(2).any(|f| f[0] != f[1]), "pan makes left and right differ");
    // mute multiplies in: track 3 muted drops out of both sides
    let mut mute = vec![false; 30];
    mute[3] = true;
    o.set_mix(&mute, &[]).unwrap();
    gl[3] = 0.0;
    gr[3] = 0.0;
    let c = o.render(1_024);
    let want = reference_lr(&decoded, &gl, &gr, mvol, 8_192, 1_024);
    assert!(
        c.iter().zip(&want).all(|(x, w)| x.to_bits() == w.to_bits()),
        "mute after volume"
    );
    assert!(o.set_volume_pan(30, 0.0, 0.0).is_err());
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn transport_mono_device_gets_the_mean_of_left_and_right() {
    let d = tmp("mono");
    let (specs, decoded) = noise_session(&d);
    let specs = &specs[..4];
    let mut o = Offline::new(specs, 48_000, small_cfg(256)).unwrap();
    o.settle();
    let mut mono = vec![0.0f32; 256];
    o.engine.render(&mut mono, 1, 0);
    // tracks 0 to 2 are mono files, track 3 a stereo file
    let mean = |r: Vec<f32>| -> Vec<f32> { r.chunks(2).map(|f| (f[0] + f[1]) * 0.5).collect() };
    let r = mean(reference(&decoded[..4], &[1.0; 4], 0, 256));
    assert!(
        mono.iter().zip(&r).all(|(a, b)| a.to_bits() == b.to_bits()),
        "centre: mono is the mean of the two sides"
    );
    o.set_volume_pan(0, 0.0, -1.0).unwrap();
    o.settle();
    o.engine.render(&mut mono, 1, 0);
    let (l, _) = mixer::pan_gains(-1.0);
    let want = mean(reference_lr(
        &decoded[..4],
        &[l, 1.0, 1.0, 1.0],
        &[0.0, 1.0, 1.0, 1.0],
        1.0,
        256,
        256,
    ));
    assert!(
        mono.iter().zip(&want).all(|(a, b)| a.to_bits() == b.to_bits()),
        "hard left on a mono device"
    );
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn transport_a_stereo_file_plays_left_and_right_and_its_pan_is_a_balance() {
    let d = tmp("stereo");
    // left: a ramp; right: a different signal; plus a mono track and a 3-channel track
    let n = 20_000usize;
    let lr: Vec<f32> = (0..n)
        .flat_map(|i| [(i % 100) as f32 / 200.0, -((i % 37) as f32) / 100.0])
        .collect();
    let ps = d.join("oh.wav");
    write(&ps, 48_000, 2, WavFormat::Pcm24, &lr);
    let pm = d.join("k.wav");
    write(
        &pm,
        48_000,
        1,
        WavFormat::Pcm24,
        &(0..n).map(|i| ((i * 7) % 51) as f32 / 300.0).collect::<Vec<_>>(),
    );
    let p3 = d.join("s3.wav");
    write(
        &p3,
        48_000,
        3,
        WavFormat::Pcm16,
        &(0..3 * n).map(|i| ((i * 13) % 41) as f32 / 400.0).collect::<Vec<_>>(),
    );
    let specs = vec![
        spec(ps.clone(), MixMode::Ffmpeg, vec![], 1000.0),
        spec(pm.clone(), MixMode::Average, vec![], 1000.0),
        spec(p3.clone(), MixMode::Average, vec![], 1000.0),
    ];
    let decoded = vec![
        sides(&ps, MixMode::Ffmpeg),
        sides(&pm, MixMode::Average),
        sides(&p3, MixMode::Average),
    ];
    assert_ne!(decoded[0][0], decoded[0][1], "the stereo file has two different sides");
    assert_eq!(decoded[2][0], decoded[2][1], "a 3-channel file plays its mono fold");
    let mut o = Offline::new(&specs, 48_000, small_cfg(256)).unwrap();
    // at the centre the stereo file gives different output channels, bit for bit its sides
    let out = o.render(4_096);
    assert!(
        out.chunks(2).filter(|f| f[0] != f[1]).count() > 4_000,
        "left and right differ"
    );
    assert_stereo_bits(&out, &reference(&decoded, &[1.0; 3], 0, 4_096), "centre");
    // only the stereo track: left is its left channel, right its right channel
    o.set_mix(&[false, true, true], &[]).unwrap();
    let out = o.render(1_024);
    for (k, f) in out.chunks(2).enumerate() {
        assert_eq!(f[0].to_bits(), decoded[0][0][4_096 + k].to_bits());
        assert_eq!(f[1].to_bits(), decoded[0][1][4_096 + k].to_bits());
    }
    // balance: hard left keeps only the left side (constant power: +3 dB there), and the
    // meter shows both sides after the balance
    o.take_meters();
    o.set_volume_pan(0, 0.0, -1.0).unwrap();
    let out = o.render(1_024);
    let (bl, br) = mixer::pan_gains(-1.0);
    assert_eq!(br, 0.0);
    for (k, f) in out.chunks(2).enumerate() {
        assert_eq!(f[0].to_bits(), (bl * decoded[0][0][5_120 + k]).to_bits());
        assert_eq!(f[1], 0.0);
    }
    let m = o.take_meters();
    let pk = |c: usize| decoded[0][c][5_120..6_144].iter().fold(0.0f32, |a, v| a.max(v.abs()));
    assert_eq!(m.tracks[0], [pk(0) * bl, 0.0]);
    assert!(m.tracks[0][0] > 0.0);
    assert_eq!(o.track_channels(), vec![2, 1, 1]);
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn transport_meters_give_the_known_peak_of_a_known_signal() {
    let d = tmp("meters");
    // track 0: 0.5 at frame 1000 and -0.8 at frame 3000, else 0; track 1: a constant 0.25
    let mut a = vec![0.0f32; 48_000];
    a[1_000] = 0.5;
    a[3_000] = -0.8;
    let pa = d.join("a.wav");
    write(&pa, 48_000, 1, WavFormat::Float32, &a);
    let pb = d.join("b.wav");
    write(&pb, 48_000, 1, WavFormat::Float32, &vec![0.25; 48_000]);
    let specs = vec![
        spec(pa, MixMode::Average, vec![], 1000.0),
        spec(pb, MixMode::Average, vec![], 1000.0),
    ];
    let mut o = Offline::new(&specs, 48_000, small_cfg(256)).unwrap();
    assert_eq!(
        o.take_meters(),
        MeterSnapshot {
            tracks: vec![[0.0; 2]; 2],
            master: [0.0; 2]
        }
    );
    o.set_master_gain(0.0);
    o.render(2_048);
    let m = o.take_meters();
    assert_eq!(m.tracks[0], [0.5, 0.5]);
    assert_eq!(m.tracks[1], [0.25, 0.25]);
    assert_eq!(
        m.master,
        [0.5 + 0.25, 0.5 + 0.25],
        "the master meter is before the master gain"
    );
    assert_eq!(o.take_meters().master, [0.0, 0.0], "a take resets the peaks");
    // post-fader: -6 dB and hard left on track 0, track 1 muted (its meter still shows it)
    o.set_volume_pan(0, -6.0, -1.0).unwrap();
    o.set_mix(&[false, true], &[]).unwrap();
    o.set_master_volume_db(6.0);
    o.render(2_048);
    let m = o.take_meters();
    let (l, _) = mixer::channel_gains(-6.0, -1.0);
    let g = mixer::db_to_gain(-6.0);
    assert_eq!(
        m.tracks[0],
        [0.8 * g, 0.8 * g],
        "a mono track: post-fader, before the pan"
    );
    assert_eq!(m.tracks[1], [0.25, 0.25], "a muted track still meters");
    let mv = mixer::db_to_gain(6.0);
    assert_eq!(m.master, [(l * -0.8f32).abs() * mv, 0.0], "post master volume");
    // paused: no new peaks
    o.set_playing(false);
    o.render(1_024);
    assert_eq!(o.take_meters().master, [0.0, 0.0]);
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn transport_mute_and_solo_apply_at_the_next_block() {
    let d = tmp("mix");
    let (specs, decoded) = noise_session(&d);
    let mut o = Offline::new(&specs, 48_000, small_cfg(256)).unwrap();
    let a = o.render(5_120);
    assert_stereo_bits(&a, &reference(&decoded, &[1.0; 30], 0, 5_120), "all");
    let mut mute = vec![false; 30];
    mute[2] = true;
    mute[29] = true;
    o.set_mix(&mute, &[]).unwrap();
    let g = mixer::audible_gains(30, &mute, &[]);
    let b = o.render(5_120);
    assert_stereo_bits(&b, &reference(&decoded, &g, 5_120, 5_120), "muted 2 and 29");
    let mut solo = vec![false; 30];
    solo[4] = true;
    solo[7] = true;
    o.set_mix(&mute, &solo).unwrap();
    let g = mixer::audible_gains(30, &mute, &solo);
    assert_eq!(g.iter().filter(|x| **x == 1.0).count(), 2);
    let c = o.render(5_120);
    assert_stereo_bits(&c, &reference(&decoded, &g, 10_240, 5_120), "solo 4 and 7");
    o.set_track_gain(4, 0.5).unwrap();
    let mut g2 = g.clone();
    g2[4] = 0.5;
    let e = o.render(1_024);
    assert_stereo_bits(&e, &reference(&decoded, &g2, 15_360, 1_024), "gain 0.5");
    assert!(o.set_track_gain(30, 1.0).is_err());
    assert!(o.set_mix(&[false; 31], &[]).is_err());
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn transport_seek_and_pause_render_the_target_frames() {
    let d = tmp("seek");
    let (specs, decoded) = noise_session(&d);
    let ones = [1.0f32; 30];
    let mut o = Offline::new(&specs, 48_000, small_cfg(256)).unwrap();
    o.render(3_000);
    for target in [20_000u64, 100, 40_000, 0, 58_000] {
        o.seek(target);
        let out = o.render(4_000);
        assert_stereo_bits(
            &out,
            &reference(&decoded, &ones, target as usize, 4_000),
            &format!("seek {target}"),
        );
        assert_eq!(o.frame_pos(), target + 4_000);
        let p = o.playhead();
        assert!(p.playing && p.frame >= target && p.frame < target + 4_000);
    }
    // two seeks before the next block: the last one wins
    o.seek(1_000);
    o.seek(2_000);
    let out = o.render(512);
    assert_stereo_bits(&out, &reference(&decoded, &ones, 2_000, 512), "double seek");
    // paused: silence, the position stays; a seek while paused is honoured on play
    o.set_playing(false);
    let at = o.frame_pos();
    let out = o.render(1_000);
    assert!(out.iter().all(|v| *v == 0.0));
    assert_eq!(o.frame_pos(), at);
    assert!(!o.playhead().playing);
    o.seek(5_000);
    o.render(256);
    o.set_playing(true);
    let out = o.render(2_000);
    assert_stereo_bits(&out, &reference(&decoded, &ones, 5_000, 2_000), "seek while paused");
    std::fs::remove_dir_all(&d).unwrap();
}

/// P3-15f: the callback publishes the tag of the seek it took in the same snapshot as the
/// target; a plain seek keeps the tag.
#[test]
fn transport_seek_tagged_publishes_the_tag_with_the_target() {
    let d = tmp("seektag");
    let (specs, _) = noise_session(&d);
    let mut o = Offline::new(&specs, 48_000, small_cfg(256)).unwrap();
    o.render(1_000);
    assert_eq!(o.playhead().seek_tag, 0, "no tagged seek yet");
    o.seek_tagged(20_000, 7);
    o.render(256);
    let p = o.playhead();
    assert_eq!((p.frame, p.seek_tag), (20_000, 7));
    o.seek(30_000);
    o.render(256);
    let p = o.playhead();
    assert_eq!((p.frame, p.seek_tag), (30_000, 7), "a plain seek keeps the tag");
    // two tagged seeks before the next block: the last one wins, with its own tag
    o.seek_tagged(1_000, 8);
    o.seek_tagged(2_000, 9);
    o.render(256);
    let p = o.playhead();
    assert_eq!((p.frame, p.seek_tag), (2_000, 9));
    std::fs::remove_dir_all(&d).unwrap();
}

/// P3-15f: a seek request is never read torn (the target of one request with the tag of
/// another), while a writer sends requests as fast as it can.
#[test]
fn seek_requests_are_read_whole_under_a_writer() {
    let sh = Arc::new(Shared::new(1, false, Speed::Normal));
    let w = {
        let sh = sh.clone();
        std::thread::spawn(move || {
            for k in 1..=200_000u64 {
                let speed = if k % 2 == 0 { Speed::Half } else { Speed::Normal };
                sh.request_seek(10 * k, Some(k), Some(speed), None, None);
            }
        })
    };
    let (mut whole, mut busy) = (0u64, 0u64);
    while !w.is_finished() || whole < 1000 {
        match sh.seek_request() {
            Some((target, tag, speed, loop_range, end_at)) => {
                assert_eq!(target, 10 * tag, "torn request");
                // P3-20b: the speed belongs to the same request
                if tag > 0 {
                    assert_eq!(speed.slowdown() as u64, 1 + (tag + 1) % 2, "torn speed");
                }
                assert!(loop_range.is_none(), "these requests carry no loop");
                assert!(end_at.is_none(), "these requests carry no window end");
                whole += 1;
            }
            None => busy += 1,
        }
    }
    w.join().unwrap();
    assert_eq!(sh.seek_request(), Some((2_000_000, 200_000, Speed::Half, None, None)));
    eprintln!("seek requests: {whole} whole reads, {busy} reads during a write");
}

#[test]
fn transport_counts_underruns_when_the_rings_run_dry() {
    let d = tmp("under");
    let (specs, _) = noise_session(&d);
    let mut o = Offline::new(&specs, 48_000, small_cfg(256)).unwrap();
    o.render(256);
    assert_eq!(o.stats().underruns, 0);
    let out = o.render_starved(1 << 16);
    let s = o.stats();
    assert!(s.underruns > 0, "{s:?}");
    assert!(s.underrun_frames > 0);
    assert!(
        out[out.len() - 2..].iter().all(|v| *v == 0.0),
        "the dry part is silence"
    );
    assert!(s.callbacks > 0 && s.frames_max >= 256 && s.frames_min > 0);
    std::fs::remove_dir_all(&d).unwrap();
}

/// #430: a small session of two tracks at the device rate, then one 96 kHz track on a
/// 48 kHz device (the resampled case). `rate` is the sample rate of the second file.
/// Long enough that a starved render of 1 << 16 frames leaves content on both sides.
fn two_track_session(dir: &Path, rate: u32) -> Vec<TrackSpec> {
    let frames = 120_000usize;
    let x: Vec<f32> = (0..frames).map(|i| ((i * 7 + 3) % 1000) as f32 / 500.0 - 1.0).collect();
    write(&dir.join("a.wav"), 48_000, 1, WavFormat::Float32, &x);
    let y: Vec<f32> = (0..frames * rate as usize / 48_000)
        .map(|i| ((i * 11 + 5) % 997) as f32 / 498.0 - 1.0)
        .collect();
    write(&dir.join("b.wav"), rate, 1, WavFormat::Float32, &y);
    vec![
        spec(dir.join("a.wav"), MixMode::Average, vec![], 1000.0),
        spec(dir.join("b.wav"), MixMode::Average, vec![], 1000.0),
    ]
}

#[test]
fn transport_after_an_underrun_every_track_sits_at_the_engine_position_again() {
    let d = tmp("starve-sync");
    let specs = two_track_session(&d, 48_000);
    let decoded: Vec<[Vec<f32>; 2]> = specs.iter().map(|s| sides(&s.path, s.mode)).collect();
    let ones = vec![1.0f32; 2];
    let mut o = Offline::new(&specs, 48_000, small_cfg(256)).unwrap();
    o.render(2_000);
    let starved = o.render_starved(1 << 16);
    let s = o.stats();
    assert!(
        s.underruns > 0 && s.underrun_frames > 0,
        "the starvation was real: {s:?}"
    );
    assert_eq!(o.frame_pos(), 67_536);
    let got = o.render(2_000);
    // bitwise: the recovery output is the reference sum at the engine position, not the
    // source delayed by the starved frames (the old underrun slip)
    assert_stereo_bits(&got, &reference(&decoded, &ones, 67_536, 2_000), "48 kHz recovery");
    let _ = starved;
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn transport_after_an_underrun_a_resampled_track_sits_at_the_engine_position_again() {
    let d = tmp("starve-sync-96");
    let specs = two_track_session(&d, 96_000);
    // the reference: the same session played through without a starvation, at the engine
    // position of the recovery (the resampled track has no plain reference sum)
    let mut twin = Offline::new(&specs, 48_000, small_cfg(256)).unwrap();
    let full = twin.render(80_000);
    let want = full[2 * 67_536..2 * 69_536].to_vec();
    let mut o = Offline::new(&specs, 48_000, small_cfg(256)).unwrap();
    o.render(2_000);
    o.render_starved(1 << 16);
    let s = o.stats();
    assert!(
        s.underruns > 0 && s.underrun_frames > 0,
        "the starvation was real: {s:?}"
    );
    assert_eq!(o.frame_pos(), 67_536);
    let got = o.render(2_000);
    assert_stereo_bits(&got, &want, "96 kHz recovery");
    std::fs::remove_dir_all(&d).unwrap();
}

/// Silent tracks with hits; the clicks are the only signal.
fn click_session(dir: &Path, sr: u32, spacing: u64) -> Vec<TrackSpec> {
    let roles = ["kick", "snare", "tom", "hat", "cymbal", "other"];
    let frames = (2.6 * sr as f64) as usize;
    (0..30u64)
        .map(|t| {
            let p = dir.join(format!("c{t:02}.wav"));
            write(&p, sr, 1, WavFormat::Pcm24, &vec![0.0; frames]);
            let hits = vec![1_000 + t * spacing, 1_000 + (t + 30) * spacing + 7];
            spec(p, MixMode::Average, hits, clicks::role_pitch(roles[t as usize % 6]))
        })
        .collect()
}

/// Onsets of the clicks: the first nonzero sample after at least `gap` zeros.
fn onsets(out: &[f32], gap: usize) -> Vec<u64> {
    let mut v = Vec::new();
    let mut zeros = gap;
    for (i, f) in out.as_chunks::<2>().0.iter().enumerate() {
        if f[0] != 0.0 {
            if zeros >= gap {
                v.push(i as u64);
            }
            zeros = 0;
        } else {
            zeros += 1;
        }
    }
    v
}

#[test]
fn transport_every_click_starts_at_offset_0_at_its_hit_sample() {
    for sr in [48_000u32, 96_000] {
        let d = tmp(&format!("clicks{sr}"));
        let spacing = (0.042 * sr as f64) as u64; // longer than a click (40 ms)
        let specs = click_session(&d, sr, spacing);
        let cfg = TransportConfig {
            clicks_on: true,
            ..small_cfg(256)
        };
        let mut o = Offline::new(&specs, sr, cfg).unwrap();
        let n = o.session_frames() as usize;
        let out = o.render(n);
        let mut want: Vec<u64> = specs.iter().flat_map(|s| s.hits.clone()).collect();
        want.sort();
        assert_eq!(onsets(&out, 50), want, "{sr} Hz");
        // each click equals its template at offset 0, sample for sample
        for s in &specs {
            let tpl = clicks::click_template(s.click_pitch, clicks::DEFAULT_CLICK_VOLUME, sr);
            for &h in &s.hits {
                let h = h as usize;
                assert_eq!(out[2 * (h - 1)], 0.0);
                for (k, v) in tpl.iter().enumerate() {
                    assert_eq!(out[2 * (h + k)].to_bits(), v.to_bits(), "{sr} Hz hit {h} offset {k}");
                }
            }
        }
        std::fs::remove_dir_all(&d).unwrap();
    }
}

#[test]
fn transport_click_settings_and_hits_update_and_mute_clicks() {
    let d = tmp("clickset");
    let specs = click_session(&d, 48_000, 2_016);
    let cfg = TransportConfig {
        clicks_on: true,
        ..small_cfg(256)
    };
    let mut o = Offline::new(&specs, 48_000, cfg).unwrap();
    let mut pitches: Vec<f32> = specs.iter().map(|s| s.click_pitch).collect();
    pitches[0] = 800.0;
    o.set_clicks(&ClickSettings {
        enabled: true,
        volume: 0.5,
        pitches: Some(pitches.clone()),
    })
    .unwrap();
    let out = o.render(4_000);
    let tpl = clicks::click_template(800.0, 0.5, 48_000);
    assert_eq!(out[2 * 1_000], tpl[0]);
    assert_eq!(out[2 * 1_100], tpl[100]);
    // muting track 1 silences its click (hit at 3016)
    o.seek(0);
    let mut mute = vec![false; 30];
    mute[1] = true;
    o.set_mix(&mute, &[]).unwrap();
    let out = o.render(4_000);
    assert!(out[2 * 3_016..2 * 4_000].iter().all(|v| *v == 0.0));
    assert_ne!(out[2 * 1_000], 0.0);
    // clicks off
    o.set_mix(&[], &[]).unwrap();
    o.set_clicks(&ClickSettings {
        enabled: false,
        volume: 0.5,
        pitches: None,
    })
    .unwrap();
    o.seek(0);
    assert!(o.render(4_000).iter().all(|v| *v == 0.0));
    // new hits for track 0
    o.set_clicks(&ClickSettings {
        enabled: true,
        volume: 0.5,
        pitches: None,
    })
    .unwrap();
    o.set_hits(0, vec![500]).unwrap();
    o.seek(0);
    let out = o.render(2_000);
    assert_eq!(onsets(&out, 50)[0], 500);
    assert!(o.set_hits(30, vec![]).is_err());
    assert!(
        o.set_clicks(&ClickSettings {
            enabled: true,
            volume: 0.5,
            pitches: Some(vec![1.0])
        })
        .is_err()
    );
    std::fs::remove_dir_all(&d).unwrap();
}

/// A band-limited test signal: a 10 ms raised-cosine fade-in times three sines.
fn tone(t: f64) -> f64 {
    let fade = if t < 0.01 {
        0.5 - 0.5 * (std::f64::consts::PI * t / 0.01).cos()
    } else {
        1.0
    };
    let s: f64 = [(100.0, 0.2), (1_000.0, 0.2), (7_000.0, 0.2)]
        .iter()
        .map(|(f, a)| a * (2.0 * std::f64::consts::PI * f * t).sin())
        .sum();
    if t < 0.0 { 0.0 } else { fade * s }
}

/// Max abs error of the left channel against `tone` summed `k` times, frames `from..`,
/// leaving out the last `tail` frames of the track (its hard end rings).
fn max_err(out: &[f32], from: u64, sr: u32, k: f64, end: u64) -> f64 {
    out.as_chunks::<2>()
        .0
        .iter()
        .enumerate()
        .map(|(i, f)| (from + i as u64, f[0]))
        .filter(|(j, _)| *j < end)
        .map(|(j, v)| (v as f64 - k * tone(j as f64 / sr as f64)).abs())
        .fold(0.0, f64::max)
}

/// Resampling tolerance: 1e-5 absolute per track (-100 dB) on a signal of peak 0.6
/// (100 Hz, 1 kHz and 7 kHz); measured 5e-7 to 8.4e-7 on arm64-mac.
const RS_TOL: f64 = 1e-5;

#[test]
fn transport_resampled_render_matches_the_signal_within_the_tolerance() {
    let d = tmp("resample");
    let mut specs = Vec::new();
    for (k, sr) in [96_000u32, 44_100].iter().enumerate() {
        let x: Vec<f32> = (0..*sr as usize).map(|i| tone(i as f64 / *sr as f64) as f32).collect();
        let p = d.join(format!("r{k}.wav"));
        write(&p, *sr, 1, WavFormat::Float32, &x);
        specs.push(spec(p, MixMode::Average, vec![], 1000.0));
    }
    let end = 48_000 - 960; // leave out the last 20 ms
    for (name, which, k) in [
        ("96k", vec![0usize], 1.0),
        ("44.1k", vec![1], 1.0),
        ("both", vec![0, 1], 2.0),
    ] {
        let s: Vec<TrackSpec> = which.iter().map(|i| specs[*i].clone()).collect();
        let mut o = Offline::new(&s, 48_000, small_cfg(256)).unwrap();
        assert_eq!(o.session_frames(), 48_000);
        let out = o.render(48_000);
        let e = max_err(&out, 0, 48_000, k, end);
        eprintln!("resampled {name} to 48 kHz: max abs error {e:.3e}");
        assert!(e < RS_TOL * k, "{name}: {e}");
        o.seek(24_000);
        let out = o.render(4_000);
        let e2 = max_err(&out, 24_000, 48_000, k, end);
        eprintln!("resampled {name} after a seek: max abs error {e2:.3e}");
        assert!(e2 < RS_TOL * k, "{name} after seek: {e2}");
        assert!(o.stats().resample_ns > 0);
    }
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn transport_tracks_at_the_device_rate_stay_bitwise_in_a_resampled_session() {
    let d = tmp("mixedrate");
    let a: Vec<f32> = (0..48_000).map(|i| ((i * 13) % 1001) as f32 / 1001.0 - 0.5).collect();
    let pa = d.join("a.wav");
    write(&pa, 48_000, 1, WavFormat::Pcm24, &a);
    let pb = d.join("b.wav");
    write(&pb, 96_000, 1, WavFormat::Float32, &vec![0.25; 96_000]);
    let specs = vec![
        spec(pa.clone(), MixMode::Average, vec![], 1000.0),
        spec(pb, MixMode::Average, vec![], 1000.0),
    ];
    let mut o = Offline::new(&specs, 48_000, small_cfg(256)).unwrap();
    o.set_mix(&[false, true], &[]).unwrap();
    let out = o.render(48_000);
    let (da, _) = read_wav_mono(&pa, MixMode::Average).unwrap();
    assert_stereo_bits(&out, &reference(&[[da.clone(), da]], &[1.0], 0, 48_000), "48 kHz track");
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn transport_resampled_clicks_sit_at_the_mapped_device_samples() {
    for sr in [96_000u32, 44_100] {
        let d = tmp(&format!("rsclicks{sr}"));
        let spacing = (0.042 * sr as f64) as u64;
        let specs = click_session(&d, sr, spacing);
        let cfg = TransportConfig {
            clicks_on: true,
            ..small_cfg(256)
        };
        let mut o = Offline::new(&specs, 48_000, cfg).unwrap();
        let n = o.session_frames() as usize;
        let out = o.render(n);
        let mut want: Vec<u64> = specs
            .iter()
            .flat_map(|s| s.hits.iter().map(|h| clicks::map_hit(*h, sr, 48_000)))
            .collect();
        want.sort();
        assert_eq!(onsets(&out, 50), want, "{sr} Hz tracks on a 48 kHz device");
        let s0 = &specs[0];
        let tpl = clicks::click_template(s0.click_pitch, clicks::DEFAULT_CLICK_VOLUME, 48_000);
        let h = clicks::map_hit(s0.hits[0], sr, 48_000) as usize;
        assert_eq!(out[2 * h], tpl[0]);
        std::fs::remove_dir_all(&d).unwrap();
    }
}

#[test]
fn transport_null_device_plays_in_real_time() {
    let d = tmp("null");
    let (specs, _) = noise_session(&d);
    assert!(matches!(
        Transport::open(&[], Output::Null { sample_rate: None }, TransportConfig::default()),
        Err(TransportError::Arg(_))
    ));
    let missing = vec![spec(d.join("missing.wav"), MixMode::Average, vec![], 1000.0)];
    let e = Transport::open(
        &missing,
        Output::Null {
            sample_rate: Some(48_000),
        },
        TransportConfig::default(),
    );
    assert!(matches!(e, Err(TransportError::Track(_))));
    assert!(e.err().unwrap().to_string().contains("missing.wav"));
    let bad_cfg = TransportConfig {
        ring_frames: 1_000,
        ..TransportConfig::default()
    };
    assert!(matches!(
        Offline::new(&specs, 48_000, bad_cfg),
        Err(TransportError::Arg(_))
    ));

    let mut t = Transport::open(&specs, Output::Null { sample_rate: None }, TransportConfig::default()).unwrap();
    assert_eq!(t.sample_rate(), 48_000);
    assert_eq!(t.device().name, "null");
    assert_eq!(t.device().buffer_frames, Some(256));
    assert_eq!(t.session_frames(), 30_000 + 997 * 29);
    assert!(!t.is_playing());
    t.set_master_gain(0.0);
    t.play();
    assert!(t.is_playing());
    // Wait for the first callbacks: the reader primes the rings first, and a loaded
    // x86 host (CI runner) can take longer than 300 ms for that.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut p = t.playhead();
    while !(p.playing && p.frame > 0) && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(20));
        p = t.playhead();
    }
    assert!(p.playing && p.frame > 0, "{p:?}");
    assert!(p.frame_at(clock::host_now_ns() + 1_000_000_000) > p.frame);
    t.set_mix(&[true], &[]).unwrap();
    t.set_track_gain(3, 0.5).unwrap();
    t.set_clicks(&ClickSettings {
        enabled: true,
        volume: 0.3,
        pitches: None,
    })
    .unwrap();
    t.set_hits(0, vec![10]).unwrap();
    t.seek_seconds(0.25);
    std::thread::sleep(std::time::Duration::from_millis(200));
    let p2 = t.playhead();
    assert!(p2.frame >= 12_000, "{p2:?}");
    // P3-16: the meters run on the null device under master gain 0
    t.set_volume_pan(1, -6.0, 0.5).unwrap();
    t.set_master_volume_db(-1.0);
    std::thread::sleep(std::time::Duration::from_millis(30));
    let m = t.take_meters();
    assert_eq!(m.tracks.len(), 30);
    assert!(m.master[0] > 0.0 && m.master[1] > 0.0, "{m:?}");
    assert!(m.tracks[0][0] > 0.0, "a muted track meters: {m:?}");
    t.seek(0);
    t.pause();
    std::thread::sleep(std::time::Duration::from_millis(50));
    let s = t.stats();
    assert_eq!(s.underruns, 0);
    assert!(s.callbacks > 10 && s.frames_min == 256 && s.frames_max == 256, "{s:?}");
    assert!(s.peak > 0.0, "the mix ran under master gain 0");
    assert_eq!(s.stream_errors, 0);
    // P3-15c: the counter reads the same count without the transport and stops with it
    let counter = t.callback_counter();
    assert!(counter.get() >= s.callbacks);
    drop(t);
    let n = counter.get();
    std::thread::sleep(std::time::Duration::from_millis(30));
    assert_eq!(counter.get(), n, "no callback after the drop");
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn transport_error_messages() {
    assert_eq!(TransportError::Arg("x".into()).to_string(), "x");
    assert!(TransportError::Track("y".into()).to_string().starts_with("track"));
    assert!(
        TransportError::Device("z".into())
            .to_string()
            .starts_with("audio device")
    );
    assert!(
        TransportError::NoAnswer("w".into())
            .to_string()
            .starts_with("audio device: w")
    );
}

#[test]
fn transport_mix_latency_is_known_after_the_callback_applied_the_change() {
    let d = tmp("mixlat");
    let (specs, _) = noise_session(&d);
    let mut o = Offline::new(&specs, 48_000, small_cfg(256)).unwrap();
    assert_eq!(o.mix_latency_ns(), None, "no change yet");
    o.render(512);
    assert_eq!(o.mix_latency_ns(), None, "still no change");
    o.set_mix(&[true], &[]).unwrap();
    assert_eq!(o.mix_latency_ns(), None, "set, not yet applied by a callback");
    o.render(256);
    assert_eq!(
        o.mix_latency_ns(),
        Some(0),
        "applied by the next block (offline host time is 0)"
    );
    // a pause holds the latest value; a second change is None until a live block applies it
    o.set_playing(false);
    o.set_mix(&[], &[]).unwrap();
    o.render_starved(256);
    assert_eq!(o.mix_latency_ns(), None, "a paused callback does not read the gains");
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn transport_set_all_hits_builds_one_bank_and_checks_the_length() {
    let d = tmp("allhits");
    let (specs, _) = noise_session(&d);
    let mut cfg = small_cfg(256);
    cfg.clicks_on = true;
    let hits: Vec<Vec<u64>> = (0..4).map(|t| vec![1_000 + 37 * t as u64, 9_000 + t as u64]).collect();
    let specs = &specs[..4];
    // one transport at a time: a default macOS shell allows 256 open files for the whole test process
    let ra = {
        let mut a = Offline::new(specs, 48_000, cfg).unwrap();
        a.set_all_hits(hits.clone()).unwrap();
        assert!(matches!(a.set_all_hits(vec![vec![]; 3]), Err(TransportError::Arg(_))));
        a.render(12_000)
    };
    let mut b = Offline::new(specs, 48_000, cfg).unwrap();
    for (t, h) in hits.iter().enumerate() {
        b.set_hits(t, h.clone()).unwrap();
        b.engine.poll();
    }
    let rb = b.render(12_000);
    assert!(
        ra.iter().zip(&rb).all(|(x, y)| x.to_bits() == y.to_bits()),
        "same clicks as 4 set_hits calls"
    );
    std::fs::remove_dir_all(&d).unwrap();
}

/// FNV-1a (64 bit) over the bits of the samples.
fn fnv1a(x: &[f32]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for v in x {
        for b in v.to_bits().to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    h
}

/// P3-20b: five tracks of every kind the transport plays (48 kHz mono and stereo, a 3-channel
/// file in `Ffmpeg` mode, a 96 kHz and a 44.1 kHz track that are resampled), hits on each.
fn identity_session(dir: &Path) -> Vec<TrackSpec> {
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    let mut rnd = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        ((seed >> 40) as f32 / (1u64 << 24) as f32) * 1.2 - 0.6
    };
    let mut specs = Vec::new();
    for (k, (sr, ch, format, mode)) in [
        (48_000u32, 1u16, WavFormat::Pcm24, MixMode::Average),
        (48_000, 2, WavFormat::Pcm16, MixMode::Average),
        (48_000, 3, WavFormat::Pcm16, MixMode::Ffmpeg),
        (96_000, 1, WavFormat::Float32, MixMode::Average),
        (44_100, 1, WavFormat::Pcm24, MixMode::Average),
    ]
    .into_iter()
    .enumerate()
    {
        let frames = (1.5 * sr as f64) as usize;
        let x: Vec<f32> = (0..frames * ch as usize).map(|_| rnd()).collect();
        let p = dir.join(format!("id{k}.wav"));
        write(&p, sr, ch, format, &x);
        let step = sr as u64 / 10 + 17 * k as u64;
        let hits = (0..14u64).map(|i| 300 + 11 * k as u64 + i * step).collect();
        specs.push(spec(
            p,
            mode,
            hits,
            clicks::role_pitch(["kick", "snare", "tom", "hat", "other"][k]),
        ));
    }
    specs
}

/// P3-20b: the normal-speed render of [`identity_session`] (clicks on, a mute, a seek, a
/// volume and pan) on a 48 kHz device.
fn identity_render(specs: &[TrackSpec]) -> Vec<f32> {
    let cfg = TransportConfig {
        clicks_on: true,
        ..small_cfg(256)
    };
    let mut o = Offline::new(specs, 48_000, cfg).unwrap();
    let mut out = o.render(20_000);
    o.set_mix(&[false, true], &[]).unwrap();
    out.extend(o.render(5_000));
    o.seek(30_000);
    o.set_mix(&[], &[]).unwrap();
    out.extend(o.render(10_000));
    o.set_volume_pan(0, -6.0, 0.3).unwrap();
    out.extend(o.render(5_000));
    out
}

/// P3-20b: normal speed is the output of the transport before half speed existed, sample
/// for sample. The hash was taken from the same render at `d302d59` (before P3-20b) on
/// arm64-mac (aarch64), and on linux-arm64 for Cherry-Blossom-Valley/drumspike#7. The
/// resampler's FFT and the click's `cos` and `powf` may round differently on another host,
/// so each host checks its own digest (other targets print their hash only).
#[test]
fn transport_normal_speed_render_equals_the_stored_output() {
    let d = tmp("identity");
    let specs = identity_session(&d);
    let out = identity_render(&specs);
    assert_eq!(out.len(), 2 * 40_000);
    let h = fnv1a(&out);
    eprintln!("normal-speed render: fnv1a {h:#018x}");
    #[cfg(all(target_arch = "aarch64", target_os = "macos"))]
    assert_eq!(h, STORED_NORMAL_RENDER_AARCH64, "normal speed changed");
    #[cfg(all(target_arch = "aarch64", target_os = "linux"))]
    assert_eq!(
        h, STORED_NORMAL_RENDER_LINUX_ARM64,
        "normal speed changed on linux-arm64"
    );
    std::fs::remove_dir_all(&d).unwrap();
}

/// P3-20b: a normal-speed render after a trip to half speed and back is the normal render
/// of the same frames, bit for bit (the resamplers of the normal rate are the ones kept).
#[test]
fn transport_normal_speed_after_half_speed_equals_a_fresh_render() {
    let d = tmp("roundtrip");
    let specs = identity_session(&d);
    let cfg = TransportConfig {
        clicks_on: true,
        ..small_cfg(256)
    };
    let mut fresh = Offline::new(&specs, 48_000, cfg).unwrap();
    fresh.seek(30_000);
    let want = fresh.render(12_000);
    drop(fresh);
    let mut o = Offline::new(&specs, 48_000, cfg).unwrap();
    o.render(5_000);
    o.set_speed(Speed::Half, 10_000, Some(4));
    assert_eq!(o.speed(), Speed::Half);
    let half = o.render(6_000);
    assert_eq!(o.playhead().slowdown, 2);
    assert!(half.iter().any(|v| *v != 0.0));
    o.set_speed(Speed::Normal, 30_000, None);
    let got = o.render(12_000);
    assert_eq!(o.playhead().slowdown, 1);
    assert_stereo_bits(&got, &want, "normal speed after half speed");
    std::fs::remove_dir_all(&d).unwrap();
}

/// Power of `x` at `freq` Hz (Goertzel), normalised by the length.
fn tone_power(x: &[f64], freq: f64, sr: f64) -> f64 {
    let w = 2.0 * std::f64::consts::PI * freq / sr;
    let c = 2.0 * w.cos();
    let (mut s1, mut s2) = (0.0f64, 0.0f64);
    for v in x {
        let s0 = v + c * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    (s1 * s1 + s2 * s2 - c * s1 * s2) / (x.len() as f64 * x.len() as f64)
}

/// The last frame of the left channel with `|x| > level` (0 when none).
fn last_above(out: &[f32], level: f32) -> usize {
    out.as_chunks::<2>()
        .0
        .iter()
        .rposition(|f| f[0].abs() > level)
        .unwrap_or(0)
}

/// P3-20b, done criterion: a 1 kHz sine at half speed has its energy at 500 Hz and lasts
/// twice as long. The 48 kHz track is resampled to 96 kHz and played at 48 kHz; with the
/// speed at open and with the speed changed while it plays.
#[test]
fn transport_half_speed_plays_a_1_khz_sine_at_500_hz_for_twice_as_long() {
    let d = tmp("halfsine");
    let sr = 48_000u32;
    let x: Vec<f32> = (0..sr as usize)
        .map(|i| (0.5 * (2.0 * std::f64::consts::PI * 1_000.0 * i as f64 / sr as f64).sin()) as f32)
        .collect();
    let p = d.join("sine.wav");
    write(&p, sr, 1, WavFormat::Float32, &x);
    let specs = vec![spec(p, MixMode::Average, vec![], 1000.0)];
    let mut normal = Offline::new(&specs, sr, small_cfg(256)).unwrap();
    let n_out = normal.render(120_000);
    let n_last = last_above(&n_out, 1e-3);
    assert_eq!(n_last, 47_999, "normal speed: 1 s");
    let cfg = TransportConfig {
        speed: Speed::Half,
        ..small_cfg(256)
    };
    let mut o = Offline::new(&specs, sr, cfg).unwrap();
    assert_eq!(o.speed(), Speed::Half);
    assert_eq!(o.session_frames(), 48_000, "the session length does not change");
    let out = o.render(120_000);
    let last = last_above(&out, 1e-3);
    eprintln!("half speed: last frame above -60 dBFS {last} (normal {n_last})");
    assert!((95_990..=96_010).contains(&last), "half speed lasts 2 s: {last}");
    assert_eq!(o.frame_pos(), 60_000, "the session moved by half the output frames");
    // the steady part: 1 s of output away from the edges
    let left: Vec<f64> = out.as_chunks::<2>().0[24_000..72_000]
        .iter()
        .map(|f| f[0] as f64)
        .collect();
    let (p500, p1000, p250) = (
        tone_power(&left, 500.0, 48_000.0),
        tone_power(&left, 1_000.0, 48_000.0),
        tone_power(&left, 250.0, 48_000.0),
    );
    eprintln!("half speed: power at 500 Hz {p500:.3e}, at 1 kHz {p1000:.3e}, at 250 Hz {p250:.3e}");
    assert!((p500 - 0.0625).abs() < 1e-3, "a 0.5 sine at 500 Hz: {p500}");
    assert!(p1000 < 1e-9 && p250 < 1e-9, "no energy left at 1 kHz: {p1000}, {p250}");
    let nl: Vec<f64> = n_out.as_chunks::<2>().0[0..48_000]
        .iter()
        .map(|f| f[0] as f64)
        .collect();
    assert!(tone_power(&nl, 1_000.0, 48_000.0) > 0.06, "normal speed is at 1 kHz");
    // the same after a change while it plays: from session frame 12,000 at half speed
    let mut o = Offline::new(&specs, sr, small_cfg(256)).unwrap();
    o.render(4_000);
    o.set_speed(Speed::Half, 12_000, None);
    let out = o.render(100_000);
    let last = last_above(&out, 1e-3);
    assert!(
        (71_990..=72_010).contains(&last),
        "36,000 frames left take 72,000: {last}"
    );
    let left: Vec<f64> = out.as_chunks::<2>().0[10_000..58_000]
        .iter()
        .map(|f| f[0] as f64)
        .collect();
    assert!((tone_power(&left, 500.0, 48_000.0) - 0.0625).abs() < 1e-3);
    std::fs::remove_dir_all(&d).unwrap();
}

/// P3-20b: a 96 kHz track at half speed on a 48 kHz device needs no resampler: the output
/// frames are the track's samples, bit for bit; the playhead moves by half a frame per frame.
#[test]
fn transport_half_speed_of_a_track_at_twice_the_device_rate_is_its_samples() {
    let d = tmp("half96");
    let x: Vec<f32> = (0..96_000).map(|i| ((i * 37) % 1_009) as f32 / 1_009.0 - 0.5).collect();
    let p = d.join("t96.wav");
    write(&p, 96_000, 1, WavFormat::Pcm24, &x);
    let (dec, _) = read_wav_mono(&p, MixMode::Average).unwrap();
    let specs = vec![spec(p, MixMode::Average, vec![], 1000.0)];
    let mut o = Offline::new(&specs, 48_000, small_cfg(256)).unwrap();
    o.render(1_000);
    o.set_speed(Speed::Half, 20_000, Some(9));
    let rs0 = o.stats().resample_ns;
    assert!(rs0 > 0, "normal speed resamples 96 to 48 kHz");
    let out = o.render(10_000);
    for (k, f) in out.chunks(2).enumerate() {
        assert_eq!(f[0].to_bits(), dec[40_000 + k].to_bits(), "frame {k}");
        assert_eq!(f[0].to_bits(), f[1].to_bits());
    }
    assert_eq!(o.stats().resample_ns, rs0, "no resampler at half speed");
    let ph = o.playhead();
    // published at the start of the last 256-frame block: output frame 39 * 256 after the target
    let want = 20_000 + 39 * 256 / 2;
    assert_eq!(
        (ph.frame, ph.slowdown, ph.seek_tag, ph.sample_rate),
        (want, 2, 9, 48_000),
        "{ph:?}"
    );
    // a seek at half speed stays at half speed
    o.seek(30_000);
    let out = o.render(1_000);
    assert_eq!(out[0].to_bits(), dec[60_000].to_bits());
    assert_eq!(o.speed(), Speed::Half);
    std::fs::remove_dir_all(&d).unwrap();
}

/// P3-20b, done criterion: at half speed the hit clicks fall at twice the session interval
/// (the click itself is the same template at the device rate), for a track at the device
/// rate and for a 96 kHz track.
#[test]
fn transport_half_speed_clicks_fall_at_twice_the_session_interval() {
    for sr in [48_000u32, 96_000] {
        let d = tmp(&format!("halfclicks{sr}"));
        let p = d.join("silent.wav");
        write(&p, sr, 1, WavFormat::Pcm24, &vec![0.0; 2 * sr as usize]);
        let step = sr as u64 / 10; // 0.1 s
        let hits: Vec<u64> = (0..15).map(|i| 1_000 + i * step).collect();
        let specs = vec![spec(p, MixMode::Average, hits.clone(), clicks::role_pitch("snare"))];
        let cfg = TransportConfig {
            clicks_on: true,
            ..small_cfg(256)
        };
        let mut o = Offline::new(&specs, 48_000, cfg).unwrap();
        let normal = o.render(96_000);
        let n_on = onsets(&normal, 50);
        o.set_speed(Speed::Half, 0, None);
        let half = o.render(192_000);
        let h_on = onsets(&half, 50);
        let want: Vec<u64> = hits.iter().map(|h| clicks::map_hit(*h, sr, 96_000)).collect();
        assert_eq!(h_on, want, "{sr} Hz: the hits mapped to twice the device rate");
        assert_eq!(n_on.len(), 15);
        for (k, w) in h_on.windows(2).enumerate() {
            let session = n_on[k + 1] - n_on[k];
            assert_eq!(w[1] - w[0], 2 * session, "{sr} Hz: interval {k}");
        }
        assert_eq!(h_on[1] - h_on[0], 9_600, "0.1 s of the session is 0.2 s of output");
        let tpl = clicks::click_template(clicks::role_pitch("snare"), clicks::DEFAULT_CLICK_VOLUME, 48_000);
        for &c in &h_on {
            for (k, v) in tpl.iter().enumerate() {
                assert_eq!(
                    half[2 * (c as usize + k)].to_bits(),
                    v.to_bits(),
                    "{sr} Hz click at {c} offset {k}"
                );
            }
        }
        std::fs::remove_dir_all(&d).unwrap();
    }
}

/// P3-20b: on the null device the playhead runs at half the rate after `set_speed`, and back.
#[test]
fn transport_set_speed_on_the_null_device_halves_the_playhead_rate() {
    let d = tmp("nullhalf");
    let x: Vec<f32> = (0..240_000).map(|i| ((i * 7) % 101) as f32 / 404.0).collect();
    let p = d.join("n.wav");
    write(&p, 48_000, 1, WavFormat::Pcm16, &x);
    let specs = vec![spec(p, MixMode::Average, vec![], 1000.0)];
    let t = Transport::open(&specs, Output::Null { sample_rate: None }, TransportConfig::default()).unwrap();
    t.set_master_gain(0.0);
    assert_eq!(t.speed(), Speed::Normal);
    t.set_speed(Speed::Half, 24_000, Some(3));
    assert_eq!(t.speed(), Speed::Half);
    t.play();
    let wait = |pred: &dyn Fn(&Playhead) -> bool| {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut ph = t.playhead();
        while !pred(&ph) && std::time::Instant::now() < end {
            std::thread::sleep(std::time::Duration::from_millis(5));
            ph = t.playhead();
        }
        ph
    };
    let a = wait(&|p| p.playing && p.seek_tag == 3);
    assert!(a.playing && a.slowdown == 2 && a.frame >= 24_000, "{a:?}");
    std::thread::sleep(std::time::Duration::from_millis(400));
    let b = t.playhead();
    let rate = (b.frame - a.frame) as f64 / ((b.host_time_ns - a.host_time_ns) as f64 / 1e9);
    eprintln!("null device, half speed: {rate:.0} session frames per second");
    assert!((rate - 24_000.0).abs() < 1_500.0, "half speed: {rate}");
    t.set_speed(Speed::Normal, b.frame, Some(4));
    let c = wait(&|p| p.playing && p.seek_tag == 4);
    std::thread::sleep(std::time::Duration::from_millis(400));
    let e = t.playhead();
    let rate = (e.frame - c.frame) as f64 / ((e.host_time_ns - c.host_time_ns) as f64 / 1e9);
    assert!((rate - 48_000.0).abs() < 3_000.0, "normal speed again: {rate}");
    assert_eq!(e.slowdown, 1);
    assert_eq!(t.stats().underruns, 0);
    drop(t);
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn speed_names_slowdowns_and_render_rates() {
    assert_eq!(Speed::default(), Speed::Normal);
    assert_eq!((Speed::Normal.slowdown(), Speed::Half.slowdown()), (1, 2));
    assert_eq!(Speed::from_slowdown(2), Speed::Half);
    assert_eq!(Speed::from_slowdown(1), Speed::Normal);
    assert_eq!(Speed::from_slowdown(7), Speed::Normal);
    assert_eq!(Speed::Half.render_rate(48_000), 96_000);
    assert_eq!(Speed::Normal.render_rate(44_100), 44_100);
    assert_eq!((Speed::Normal.rate(), Speed::Half.rate()), (1.0, 0.5));
    for v in [Speed::Normal, Speed::Half] {
        assert_eq!(Speed::from_rate(v.rate()), Some(v));
    }
    assert_eq!(Speed::from_rate(2.0), None);
    assert_eq!(Speed::from_rate(f64::NAN), None);
}

/// The hash of [`identity_render`] at `d302d59` on aarch64 (see the test above).
#[cfg(all(target_arch = "aarch64", target_os = "macos"))]
const STORED_NORMAL_RENDER_AARCH64: u64 = 0x4aaa_12ad_dea6_a76d;

/// The hash of [`identity_render`] on Linux arm64 (Cherry-Blossom-Valley/drumspike#7): stable over
/// 3 runs on linux-arm64, and different from the arm64-mac value (the resampler's FFT and the
/// click's `cos` and `powf` round differently there), so the digest is per host.
#[cfg(all(target_arch = "aarch64", target_os = "linux"))]
const STORED_NORMAL_RENDER_LINUX_ARM64: u64 = 0xb9c1_dfd1_c55b_de64;

/// P3-20b proof on the generated 30-track session (`DRUMSPIKE_TRACKS_BENCH=<make_session.py folder>`:
/// 96 kHz files, so the 48 kHz render resamples every track; the hits of `session.json` click): the
/// normal-speed offline render of 20 s from the start and 10 s from 150 s, hashed. The same test at
/// `d302d59` (before P3-20b) gives the same hash (`docs/archive/phase3-report.md`, section P3-20b).
#[test]
#[ignore = "proof: needs DRUMSPIKE_TRACKS_BENCH=<make_session.py folder>"]
fn transport_normal_speed_render_of_the_generated_session_hash() {
    fn fnv(h: &mut u64, x: &[f32]) {
        for v in x {
            for b in v.to_bits().to_le_bytes() {
                *h ^= b as u64;
                *h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
    }
    let dir = PathBuf::from(std::env::var_os("DRUMSPIKE_TRACKS_BENCH").expect("DRUMSPIKE_TRACKS_BENCH"));
    let meta: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("session.json")).unwrap()).unwrap();
    let roles = ["kick", "snare", "tom", "hat", "cymbal", "other"];
    let specs: Vec<TrackSpec> = meta["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(i, t)| TrackSpec {
            path: dir.join(t["file"].as_str().unwrap()),
            mode: MixMode::Average,
            hits: t["hit_samples"]
                .as_array()
                .unwrap()
                .iter()
                .map(|h| h.as_u64().unwrap())
                .collect(),
            click_pitch: clicks::role_pitch(roles[i % 6]),
        })
        .collect();
    assert_eq!(specs.len(), 30);
    let mut o = Offline::new(&specs, 48_000, TransportConfig::default()).unwrap();
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    let a = o.render(20 * 48_000);
    fnv(&mut h, &a);
    o.seek(150 * 48_000);
    let b = o.render(10 * 48_000);
    fnv(&mut h, &b);
    let peak = a.iter().chain(&b).fold(0.0f32, |m, v| m.max(v.abs()));
    println!(
        "SESSION normal-speed render: {} frames, peak {peak}, fnv1a {h:#018x}",
        (a.len() + b.len()) / 2
    );
}

/// P4-7a: a silent track with a layer: each voice starts at its frame (sample-exact, offset 0 of
/// the one-shot), overlapping voices add, mute silences the layer, the counter counts the starts,
/// and at half speed the half-speed one-shot starts at twice the frame.
#[test]
fn transport_layer_voices_start_sample_exact_at_both_speeds() {
    use super::layer::LayerVoices;
    let d = tmp("layer");
    let p = d.join("silent.wav");
    write(&p, 48_000, 1, WavFormat::Float32, &vec![0.0f32; 48_000]);
    let specs = vec![spec(p, MixMode::Average, vec![], 1000.0)];
    let shot: Vec<f32> = (0..2_000).map(|i| 1.0 - i as f32 / 2_000.0).collect();
    let half: Vec<f32> = (0..4_000).map(|i| 0.5 - i as f32 / 8_000.0).collect();
    // voices at 0.1 s, 0.12 s (overlaps the first) and 0.5 s (gain -0.5: an inverted layer)
    let starts = [(0.1, 1.0f32), (0.12, 1.0), (0.5, -0.5)];
    let lv = LayerVoices::new(1, Arc::new(shot.clone()), Arc::new(half.clone()), &starts, 48_000);
    let mut o = Offline::new(&specs, 48_000, small_cfg(256)).unwrap();
    assert_eq!(o.stats().layer_voices, 0);
    assert_eq!(o.stats().layer_last_start, None);
    assert!(o.set_layers(vec![None, None]).is_err(), "one entry per track");
    o.set_layers(vec![Some(lv.clone())]).unwrap();
    let out = o.render(30_000);
    let l = |f: usize| out[2 * f];
    assert_eq!(l(4_799), 0.0);
    assert_eq!(l(4_800), shot[0], "the first sample of the one-shot on the start frame");
    assert_eq!(l(5_000), shot[200]);
    assert_eq!(l(5_760), shot[960] + shot[0], "overlap: both voices sound");
    assert_eq!(l(24_000), -0.5 * shot[0]);
    assert_eq!(out[2 * 24_000 + 1], l(24_000), "mono layer on both sides");
    let s = o.stats();
    assert_eq!((s.layer_voices, s.layer_last_start), (3, Some(24_000)));
    // muted track: no layer, no new voice counted
    o.seek(0);
    o.set_mix(&[true], &[]).unwrap();
    let muted = o.render(30_000);
    assert!(muted.iter().all(|v| *v == 0.0));
    assert_eq!(o.stats().layer_voices, 3);
    // half speed: the half-speed one-shot at 2 * 48,000 * t output frames
    o.set_mix(&[false], &[]).unwrap();
    o.set_speed(Speed::Half, 0, None);
    let h = o.render(60_000);
    assert_eq!(h[2 * 9_599], 0.0);
    assert_eq!(h[2 * 9_600], half[0]);
    assert_eq!(h[2 * 48_000], -0.5 * half[0]);
    assert_eq!(o.stats().layer_last_start, Some(48_000));
    // no layer: the output is the output without layers
    o.set_layers(vec![None]).unwrap();
    o.set_speed(Speed::Normal, 0, None);
    assert!(o.render(30_000).iter().all(|v| *v == 0.0));
    std::fs::remove_dir_all(&d).unwrap();
}

/// P4-7a: the cost of layer voices on the generated 30-track session (`DRUMSPIKE_TRACKS_BENCH=<make_session.py
/// folder>`): the offline render of 30 s with no layer and with a layer (a 0.5 s one-shot, mono) on 5 tracks at
/// every hit, and the layer mixing alone (`LayerVoices::add_block` in 256-frame blocks) as a share of real time.
/// Run in release: `cargo test --release -p drumspike-core --features audio -- --ignored transport_layer_cost`.
#[test]
#[ignore = "measure: needs DRUMSPIKE_TRACKS_BENCH=<make_session.py folder>"]
fn transport_layer_cost_on_the_generated_session() {
    use super::layer::LayerVoices;
    use std::time::Instant;
    let dir = PathBuf::from(std::env::var_os("DRUMSPIKE_TRACKS_BENCH").expect("DRUMSPIKE_TRACKS_BENCH"));
    let meta: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("session.json")).unwrap()).unwrap();
    let specs: Vec<TrackSpec> = meta["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| TrackSpec {
            path: dir.join(t["file"].as_str().unwrap()),
            mode: MixMode::Average,
            hits: t["hit_samples"]
                .as_array()
                .unwrap()
                .iter()
                .map(|h| h.as_u64().unwrap())
                .collect(),
            click_pitch: 1000.0,
        })
        .collect();
    let src_sr = meta["sr"].as_f64().unwrap();
    let rate = 48_000u32;
    let shot: Vec<f32> = (0..24_000)
        .map(|i| (1.0 - i as f32 / 24_000.0) * ((i as f32) * 0.05).sin())
        .collect();
    let half: Vec<f32> = (0..48_000)
        .map(|i| (1.0 - i as f32 / 48_000.0) * ((i as f32) * 0.025).sin())
        .collect();
    let (tpl, tpl_half) = (Arc::new(shot), Arc::new(half));
    let bank: Vec<Option<LayerVoices>> = specs
        .iter()
        .enumerate()
        .map(|(k, s)| {
            (k < 5).then(|| {
                let starts: Vec<(f64, f32)> = s.hits.iter().map(|h| (*h as f64 / src_sr - 0.001, 0.8)).collect();
                LayerVoices::new(1, tpl.clone(), tpl_half.clone(), &starts, rate)
            })
        })
        .collect();
    let secs = 30usize;
    let time = |layers: bool| {
        let mut o = Offline::new(&specs, rate, TransportConfig::default()).unwrap();
        if layers {
            o.set_layers(bank.clone()).unwrap();
        }
        let t0 = Instant::now();
        let out = o.render(secs * rate as usize);
        (t0.elapsed().as_secs_f64(), o.stats().layer_voices, out.len())
    };
    let (plain, v0, _) = time(false);
    let (with, v1, _) = time(true);
    // the layer mixing alone over the whole 300 s, in callback-sized blocks
    let frames = (meta["seconds"].as_f64().unwrap() * rate as f64) as u64;
    let (mut l, mut r) = (vec![0.0f32; 256], vec![0.0f32; 256]);
    let t0 = Instant::now();
    let mut started = 0;
    let mut f = 0u64;
    while f < frames {
        l.fill(0.0);
        r.fill(0.0);
        for lv in bank.iter().flatten() {
            started += lv.add_block(&mut l, &mut r, f, 1.0, 1.0, Speed::Normal).0;
        }
        f += 256;
    }
    let mix = t0.elapsed().as_secs_f64();
    println!(
        "LAYER COST: 30 tracks, {secs} s offline render: {plain:.3} s without layers ({v0} voices), {with:.3} s with 5 layers \
         ({v1} voices); layer mixing alone over {:.0} s: {mix:.3} s for {started} voices = {:.3} % of real time",
        frames as f64 / rate as f64,
        100.0 * mix / (frames as f64 / rate as f64)
    );
    assert_eq!(v0, 0);
    assert!(v1 > 0);
}

// ---- #166: the audition loop wraps in the engine (no seek, no gap) ----

/// A 0.5 s synthetic click track at `sr`: sharp decaying clicks at the given frames, plus two
/// outside the loop region (at 300 and `tail`), so a wrap into them would be heard in the compare.
fn click_track(dir: &Path, name: &str, sr: u32, at: &[usize]) -> TrackSpec {
    let n = (0.5 * sr as f64) as usize;
    let mut x = vec![0.0f32; n];
    for &f in at {
        let decay = 3.0f64; // ~1/(ln 3) per frame: a click rings for a few dozen frames
        for k in 0..300.min(n - f) {
            x[f + k] += (0.9f64 * decay.powi(-(k as i32))) as f32 * if k % 2 == 0 { 1.0 } else { -1.0 };
        }
    }
    let p = dir.join(name);
    write(&p, sr, 1, WavFormat::Float32, &x);
    spec(p, MixMode::Average, vec![], 1000.0)
}

/// The reference for one loop pass: a fresh transport that seeks to `start` and renders `n` frames.
fn pass_render(specs: &[TrackSpec], cfg: TransportConfig, start: u64, n: usize, hits: Vec<Vec<u64>>) -> Vec<f32> {
    let mut o = Offline::new(specs, 48_000, cfg).unwrap();
    if !hits.is_empty() {
        o.set_all_hits(hits).unwrap();
    }
    o.seek_tagged(start, 1);
    o.render(n)
}

/// #166 (the gate): 3 passes of a loop over a 100 ms region of a synthetic click track render
/// sample-identical to the region repeated 3 times: no silence, no click at the wrap, no
/// dropped or doubled sample at the 2 wrap points. Run at buffer sizes below, at and above the
/// region, so the render and the reader split at the wrap point.
#[test]
fn transport_loop_renders_the_region_repeated_bit_for_bit() {
    let d = tmp("loop");
    // 48 kHz: the region is frames [4800, 9600) = 100 ms; clicks inside it, two outside
    let specs = vec![click_track(&d, "clicks.wav", 48_000, &[300, 4800, 6111, 8333, 20_000])];
    let (start, end) = (4_800u64, 9_600u64);
    let n = (end - start) as usize;
    for buffer in [256u32, 300, 4_096] {
        let cfg = small_cfg(buffer);
        let reference = pass_render(&specs, cfg, start, n, vec![]);
        assert!(
            reference.iter().any(|v| v.abs() > 0.5),
            "buffer {buffer}: the region has its clicks (the compare is not against silence)"
        );
        let mut o = Offline::new(&specs, 48_000, cfg).unwrap();
        o.seek_tagged_with_loop(start, 1, Some((start, end)));
        let out = o.render(3 * n);
        let want: Vec<f32> = reference.iter().chain(&reference).chain(&reference).copied().collect();
        assert_stereo_bits(&out, &want, "buffer {buffer}: 3 passes");
    }
    std::fs::remove_dir_all(&d).unwrap();
}

/// #166: the loop keeps its sample identity when the track is resampled (44.1 kHz on a 48 kHz
/// device): the wrap re-primes the resampler exactly like a seek, per pass.
#[test]
fn transport_loop_of_a_resampled_track_renders_the_region_repeated() {
    let d = tmp("loop-rs");
    let specs = vec![click_track(
        &d,
        "clicks44.wav",
        44_100,
        &[300, 4800, 6111, 8333, 20_000],
    )];
    let (start, end) = (4_800u64, 9_600u64);
    let n = (end - start) as usize;
    let cfg = small_cfg(256);
    let reference = pass_render(&specs, cfg, start, n, vec![]);
    let mut o = Offline::new(&specs, 48_000, cfg).unwrap();
    o.seek_tagged_with_loop(start, 1, Some((start, end)));
    let out = o.render(3 * n);
    let want: Vec<f32> = reference.iter().chain(&reference).chain(&reference).copied().collect();
    assert_stereo_bits(&out, &want, "resampled: 3 passes");
    std::fs::remove_dir_all(&d).unwrap();
}

/// #166: with clicks on, the metronome of the hits inside the region fires again on every pass
/// at the same place (the engine wraps its counter; a block never spans the wrap point).
#[test]
fn transport_loop_places_the_clicks_again_on_every_pass() {
    let d = tmp("loop-clicks");
    let specs = vec![click_track(&d, "clicks.wav", 48_000, &[4_800, 6_300, 7_800])];
    // hits at track frames inside the region (the last still ends 30 ms before the region end)
    let hits = vec![vec![4_800u64, 6_300, 7_800]];
    let (start, end) = (4_800u64, 9_600u64);
    let n = (end - start) as usize;
    for buffer in [256u32, 4_096] {
        let cfg = TransportConfig {
            clicks_on: true,
            ..small_cfg(buffer)
        };
        let reference = pass_render(&specs, cfg, start, n, hits.clone());
        assert!(reference.iter().any(|v| v.abs() > 0.5));
        let mut o = Offline::new(&specs, 48_000, cfg).unwrap();
        o.set_all_hits(hits.clone()).unwrap();
        o.seek_tagged_with_loop(start, 1, Some((start, end)));
        let out = o.render(3 * n);
        let want: Vec<f32> = reference.iter().chain(&reference).chain(&reference).copied().collect();
        assert_stereo_bits(&out, &want, "buffer {buffer}: 3 passes with clicks");
    }
    std::fs::remove_dir_all(&d).unwrap();
}

/// #166: the loop ends at the next seek without a region: the render after a plain seek is the
/// track's frames past the region (with the click at 20 000, outside it), not a wrapped pass.
#[test]
fn transport_loop_ends_at_the_next_plain_seek() {
    let d = tmp("loop-clear");
    let specs = vec![click_track(
        &d,
        "clicks.wav",
        48_000,
        &[300, 4_800, 6_111, 8_333, 20_000],
    )];
    let (start, end) = (4_800u64, 9_600u64);
    let cfg = small_cfg(256);
    let mut o = Offline::new(&specs, 48_000, cfg).unwrap();
    o.seek_tagged_with_loop(start, 1, Some((start, end)));
    let wrapped = o.render(3 * (end - start) as usize);
    // the wrapped render really wrapped: its third pass is the region again, not new frames
    let reference = pass_render(&specs, cfg, start, (end - start) as usize, vec![]);
    assert_eq!(
        &wrapped[4 * (end - start) as usize..],
        &reference[..],
        "the 3rd pass is the region"
    );
    // a plain seek (no region): the playhead leaves the loop and plays the track's frames
    o.seek(12_000);
    let after = o.render(4_800);
    let plain = pass_render(&specs, cfg, 12_000, 4_800, vec![]);
    assert_stereo_bits(&after, &plain, "the frames after the plain seek");
    std::fs::remove_dir_all(&d).unwrap();
}

/// #201 (the gate): a starved reader must not shorten an audition. The render path is starved
/// the way the loaded CI host of #200 starved it: the render thread gets about a quarter of
/// the periods (the clock — the watcher's `host_now_ns` — runs at three times the render
/// pace), and the reader is serviced only after the rings ran dry (a forced late fill). The
/// audition ends in the engine: every frame of the window reaches the output (the tail is the
/// audition's audio, not the cut of #200's 1408 frames = 29.3 ms) and the reported position
/// equals the window end. On the current code (the app paused on the playhead extrapolated at
/// the clock rate) this run ended 384 frames short — the reproduction is in the ticket's PR.
#[test]
fn transport_audition_starved_delivers_every_frame_up_to_the_window_end() {
    let d = tmp("t201end");
    let sr = 48_000u32;
    // 4 s of tone: every frame of the window carries signal
    let x: Vec<f32> = (0..sr * 4).map(|i| tone(i as f64 / sr as f64) as f32).collect();
    write(&d.join("a.wav"), sr, 1, WavFormat::Float32, &x);
    let specs = vec![spec(d.join("a.wav"), MixMode::Average, vec![], 1000.0)];
    // a small ring, so the starved reader runs it dry early in the window
    let cfg = TransportConfig {
        ring_frames: 1 << 14,
        prefill_frames: 1 << 11,
        ..small_cfg(256)
    };
    let mut o = Offline::new(&specs, sr, cfg).unwrap();
    let (start, end) = (sr as u64, 2 * sr as u64); // a 1 s window inside the tone
    o.seek_tagged_window(start, 1, None, Some(end));
    o.settle();
    let block = 256usize;
    let period_ns = (block as f64 * 1e9 / sr as f64).round() as u64;
    let mut clock = 1_000_000_000u64;
    let mut i = 0u64;
    let mut underrun_seen = false;
    let mut delivered: Vec<(u64, u64, Vec<f32>)> = Vec::new(); // (from, to, samples)
    loop {
        if !underrun_seen && o.stats().underruns > 0 {
            underrun_seen = true; // from here the reader catches up (the forced late fill)
        }
        if underrun_seen {
            o.reader.step();
        }
        // no end decision here: the engine ends the audition itself at the delivered end
        if i % 8 != 7 {
            let before = o.engine.frame_pos();
            let mut buf = vec![0.0f32; 2 * block];
            o.engine.render(&mut buf, 2, clock);
            let after = o.engine.frame_pos();
            if after > before {
                delivered.push((before, after, buf));
            }
        }
        clock += 3 * period_ns;
        i += 1;
        if !o.ctl.shared.playing.load(Ordering::Acquire) {
            break; // the engine paused the audition: the window is out
        }
        assert!(i < 100_000, "the audition never ended");
    }
    assert!(underrun_seen, "the starvation was real (the rings ran dry)");
    // every frame of the window reached the output: no gap, no hole in the delivery
    let got = o.engine.frame_pos();
    assert_eq!(
        got,
        end,
        "#201: the audition ended {} frames short of the window end {end}",
        end.saturating_sub(got)
    );
    assert_eq!(
        delivered.first().map(|r| r.0),
        Some(start),
        "the delivery starts at the window start"
    );
    let mut upto = start;
    for (from, to, _) in &delivered {
        assert_eq!(*from, upto, "a hole in the delivery at frame {upto}");
        upto = *to;
    }
    assert_eq!(upto, end, "the delivery ends at the window end");
    // the tail is the audition's own audio: #430 dropped the frames the starved reader
    // lost (the old underrun slip), so the output carries the source at the engine
    // position — the tail is the audition's audio, not the cut of #200, not silence
    let missing = o.stats().underrun_frames as i64;
    assert!(missing > 0, "the starvation lost frames");
    let tail_len = 2048usize;
    let want_from = end - tail_len as u64;
    let mut tail: Vec<f32> = Vec::new();
    for (from, to, samples) in &delivered {
        let a = want_from.max(*from);
        let b = end.min(*to);
        if a < b {
            let s = ((a - from) * 2) as usize;
            let e = ((b - from) * 2) as usize;
            tail.extend_from_slice(&samples[s..e]);
        }
    }
    assert_eq!(tail.len(), tail_len * 2, "the tail was delivered whole");
    for (k, v) in tail.iter().enumerate() {
        let f = want_from + (k / 2) as u64;
        assert_eq!(*v, x[f as usize], "the tail at frame {} is not the audition's audio", f);
    }
    // one more render publishes the paused end (the engine's next callback in the app)
    let mut buf = vec![0.0f32; 2 * block];
    o.engine.render(&mut buf, 2, clock);
    // the reported position is the window end, and the audition is over
    let ph = o.ctl.clock.read();
    assert_eq!((ph.frame, ph.playing), (end, false), "the playhead at the end");
    std::fs::remove_dir_all(&d).unwrap();
}
