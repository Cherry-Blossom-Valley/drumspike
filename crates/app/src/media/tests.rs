use super::*;
use drumspike_core::io::{encode_wav, WavFormat, WavSpec};

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ds_app_media_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write(path: &Path, sr: u32, n: usize) {
    let x: Vec<f32> = (0..n).map(|i| ((i as f32) * 0.05).sin() * 0.5).collect();
    let spec = WavSpec {
        sample_rate: sr,
        channels: 1,
        format: WavFormat::Float32,
        extensible: false,
        channel_mask: 0,
    };
    std::fs::write(path, encode_wav(&spec, &x, None)).unwrap();
}

#[test]
fn mix_mode_reads_the_document_word() {
    assert_eq!(mix_mode("average"), MixMode::Average);
    assert_eq!(mix_mode("ffmpeg"), MixMode::Ffmpeg);
    assert_eq!(mix_mode("anything"), MixMode::Ffmpeg);
}

#[test]
fn decode_reads_rate_and_samples_and_names_the_file_on_error() {
    let d = tmp("decode");
    write(&d.join("a.wav"), 48_000, 1000);
    let x = decode(&d.join("a.wav"), MixMode::Ffmpeg).unwrap();
    assert_eq!((x.sr, x.samples.len()), (48_000, 1000));
    std::fs::write(d.join("bad.wav"), b"not a wav file at all").unwrap();
    let e = decode(&d.join("bad.wav"), MixMode::Ffmpeg).unwrap_err();
    assert_eq!(e.code, 400);
    assert!(e.message.contains("bad.wav"), "{e}");
    let e = decode(&d.join("none.wav"), MixMode::Ffmpeg).unwrap_err();
    assert_eq!(e.code, 500);
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn decode_cache_returns_the_same_decode_and_keeps_three() {
    let d = tmp("cache");
    let m = Media::new();
    for k in 0..4 {
        write(&d.join(format!("{k}.wav")), 44_100, 500 + k);
    }
    let a1 = m.decode_cached(&d.join("0.wav"), MixMode::Ffmpeg).unwrap();
    let a2 = m.decode_cached(&d.join("0.wav"), MixMode::Ffmpeg).unwrap();
    assert!(Arc::ptr_eq(&a1, &a2));
    for k in 1..4 {
        m.decode_cached(&d.join(format!("{k}.wav")), MixMode::Ffmpeg).unwrap();
    }
    assert_eq!(lock(&m.decoded).len(), DECODE_CACHE_LEN);
    let a3 = m.decode_cached(&d.join("0.wav"), MixMode::Ffmpeg).unwrap();
    assert!(!Arc::ptr_eq(&a1, &a3), "the oldest entry was dropped");
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn eight_threads_on_one_file_decode_it_once_and_share_the_result() {
    let d = tmp("cache_flight");
    let m = Media::new();
    let p = d.join("a.wav");
    write(&p, 48_000, 20_000);
    let got: Mutex<Vec<Arc<Decoded>>> = Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..8 {
            let (m, p, got) = (&m, &p, &got);
            s.spawn(move || {
                let x = m.decode_cached(p, MixMode::Ffmpeg).unwrap();
                assert_eq!(x.samples.len(), 20_000);
                lock(got).push(x);
            });
        }
    });
    let all = lock(&got);
    assert_eq!(all.len(), 8);
    for x in all.iter() {
        assert!(Arc::ptr_eq(&all[0], x), "all eight requests share one decode");
    }
    assert_eq!(m.decode_runs(), 1, "the decoder ran exactly once");
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn three_decodes_over_half_the_budget_leave_the_held_bytes_within_it() {
    let d = tmp("cache_budget");
    // each decode is 6_000 samples = 24_000 bytes, over half of the 40_000 budget
    let m = Media::with_budget(40_000);
    for k in 0..3 {
        write(&d.join(format!("{k}.wav")), 44_100, 6_000);
        m.decode_cached(&d.join(format!("{k}.wav")), MixMode::Ffmpeg).unwrap();
        let held: usize = lock(&m.decoded).iter().map(|(_, x)| x.samples.len() * 4).sum();
        assert!(held <= 40_000, "held {held} bytes after {k} decodes");
    }
    assert_eq!(m.decode_runs(), 3, "every file is its own decode");
    // an entry larger than the budget is returned but not kept
    write(&d.join("big.wav"), 44_100, 20_000);
    let big = m.decode_cached(&d.join("big.wav"), MixMode::Ffmpeg).unwrap();
    assert_eq!(big.samples.len(), 20_000);
    let held: usize = lock(&m.decoded).iter().map(|(_, x)| x.samples.len() * 4).sum();
    assert!(held <= 40_000, "the oversized decode was not kept ({held})");
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn a_failing_decode_leaves_no_inflight_slot() {
    let d = tmp("cache_err");
    let m = Media::new();
    std::fs::write(d.join("bad.wav"), b"not a wav file at all").unwrap();
    assert!(m.decode_cached(&d.join("bad.wav"), MixMode::Ffmpeg).is_err());
    assert!(lock(&m.inflight).is_empty(), "the failed decode leaves no slot behind");
    // a request for the same key afterwards decodes again (no stale empty slot)
    write(&d.join("bad.wav"), 44_100, 100);
    let x = m.decode_cached(&d.join("bad.wav"), MixMode::Ffmpeg).unwrap();
    assert_eq!(x.samples.len(), 100);
    std::fs::remove_dir_all(&d).unwrap();
}

fn want(full: &[f32], sr: u32, s: usize, e: usize) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(s as u32).to_le_bytes());
    out.extend_from_slice(&sr.to_le_bytes());
    for v in &full[s..e] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

#[test]
fn window_bytes_equal_the_full_decode_and_clamp_like_the_command() {
    let d = tmp("window");
    let m = Media::new();
    let p = d.join("a.wav");
    write(&p, 44_100, 10_000);
    let full = decode(&p, MixMode::Average).unwrap().samples;
    let w = |a, b, max| m.window_bytes(&p, MixMode::Average, a, b, max).unwrap();
    assert_eq!(w(0, 100, 262_144), want(&full, 44_100, 0, 100));
    assert_eq!(w(4_950, 5_050, 262_144), want(&full, 44_100, 4_950, 5_050));
    assert_eq!(w(9_900, 10_000, 262_144), want(&full, 44_100, 9_900, 10_000));
    assert_eq!(
        w(9_900, 10_500, 262_144),
        want(&full, 44_100, 9_900, 10_000),
        "across the end"
    );
    assert_eq!(w(-50, 10, 262_144), want(&full, 44_100, 0, 10), "before the start");
    assert_eq!(
        w(20_000, 30_000, 262_144),
        want(&full, 44_100, 10_000, 10_000),
        "past the end"
    );
    assert_eq!(w(500, 100, 262_144), want(&full, 44_100, 500, 500), "reversed");
    assert_eq!(w(100, 10_000, 1_000), want(&full, 44_100, 100, 1_100), "the cap");
    // a planted wrong sample is seen
    let mut bad = want(&full, 44_100, 0, 100);
    bad[8 + 4 * 50] ^= 1;
    assert_ne!(w(0, 100, 262_144), bad);
    assert_eq!(m.window_opens(), 1, "one open for every window of the file");
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn window_bytes_open_a_file_once_for_parallel_requests_and_again_when_it_changed() {
    let d = tmp("window_par");
    let m = Media::new();
    let p = d.join("a.wav");
    write(&p, 48_000, 50_000);
    let full = decode(&p, MixMode::Ffmpeg).unwrap().samples;
    std::thread::scope(|s| {
        for k in 0..12usize {
            let (m, p, full) = (&m, &p, &full);
            s.spawn(move || {
                let a = k * 3_000;
                let b = m.window_bytes(p, MixMode::Ffmpeg, a as i64, (a + 5_000) as i64, 262_144);
                assert_eq!(b.unwrap(), want(full, 48_000, a, a + 5_000));
            });
        }
    });
    assert_eq!(m.window_opens(), 1, "parallel first requests open the file once");
    // another mode is another source
    m.window_bytes(&p, MixMode::Average, 0, 10, 262_144).unwrap();
    assert_eq!(m.window_opens(), 2);
    // the file changes (another length): it is opened again and the new samples come back
    write(&p, 48_000, 20_000);
    let full2 = decode(&p, MixMode::Ffmpeg).unwrap().samples;
    let b = m.window_bytes(&p, MixMode::Ffmpeg, 19_000, 21_000, 262_144).unwrap();
    assert_eq!(b, want(&full2, 48_000, 19_000, 20_000));
    assert_eq!(m.window_opens(), 3);
    assert_eq!(lock(&m.windows).len(), 2, "the old state of the file is dropped");
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn window_bytes_name_the_file_on_error_like_decode() {
    let d = tmp("window_err");
    let m = Media::new();
    std::fs::write(d.join("bad.wav"), b"not a wav file at all").unwrap();
    let e = m
        .window_bytes(&d.join("bad.wav"), MixMode::Ffmpeg, 0, 10, 100)
        .unwrap_err();
    assert_eq!(e, decode(&d.join("bad.wav"), MixMode::Ffmpeg).unwrap_err());
    let e = m
        .window_bytes(&d.join("none.wav"), MixMode::Ffmpeg, 0, 10, 100)
        .unwrap_err();
    assert_eq!(e.code, 500);
    assert!(e.message.contains("none.wav"), "{e}");
    assert_eq!(m.window_opens(), 0, "errors are not kept");
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn the_window_cache_keeps_the_last_sources() {
    let d = tmp("window_len");
    let m = Media::new();
    for k in 0..WINDOW_SOURCES_LEN + 3 {
        let p = d.join(format!("{k}.wav"));
        write(&p, 48_000, 100);
        m.window_bytes(&p, MixMode::Ffmpeg, 0, 10, 100).unwrap();
    }
    assert_eq!(lock(&m.windows).len(), WINDOW_SOURCES_LEN);
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn on_peak_workers_runs_every_index_once_and_in_parallel() {
    use std::sync::Condvar;
    use std::time::{Duration, Instant};
    let seen = Mutex::new(vec![0usize; 40]);
    let active = Mutex::new((0usize, 0usize)); // (now, most)
    let cv = Condvar::new();
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    let want_par = cores.clamp(1, PEAK_WORKERS);
    on_peak_workers(40, |i| {
        lock(&seen)[i] += 1;
        let mut a = lock(&active);
        a.0 += 1;
        a.1 = a.1.max(a.0);
        cv.notify_all();
        // wait (at most 2 s) until as many workers run as the host allows
        let end = Instant::now() + Duration::from_secs(2);
        while a.1 < want_par && Instant::now() < end {
            a = cv.wait_timeout(a, Duration::from_millis(20)).unwrap().0;
        }
        a.0 -= 1;
    });
    assert!(lock(&seen).iter().all(|&n| n == 1));
    let most = lock(&active).1;
    assert_eq!(most, want_par, "{most} workers at the same time, {cores} cores");
    assert!(most <= PEAK_WORKERS);
    on_peak_workers(0, |_| panic!("no work"));
}
