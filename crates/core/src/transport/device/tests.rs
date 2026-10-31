use super::*;

#[test]
fn default_output_info_answers_or_names_the_missing_device() {
    // a host without an output (CI) gives the error; a host with one gives a real rate
    match default_output_info() {
        Ok(i) => {
            assert!(i.sample_rate >= 8_000 && i.channels >= 1, "{i:?}");
            assert!(!i.name.is_empty());
            assert_eq!(i.buffer_frames, None, "no stream is opened");
        }
        Err(e) => assert!(e.to_string().contains("device") || e.to_string().len() > 3, "{e}"),
    }
}

#[test]
fn output_devices_lists_the_default_and_output_info_finds_each_by_name() {
    // a host without an output (CI) gives an empty list or an error; with one, every listed
    // device answers by its name, and the default is in the list
    let Ok(list) = output_devices() else { return };
    eprintln!("output devices: {list:?}");
    for d in &list {
        assert!(d.sample_rate >= 8_000 && d.channels >= 1, "{d:?}");
        let again = output_info(Some(&d.name)).unwrap();
        assert_eq!(again.name, d.name);
    }
    if let Ok(def) = default_output_info() {
        eprintln!("default output: {def:?}");
        assert!(list.iter().any(|d| d.name == def.name), "{def:?} not in {list:?}");
    }
    let e = output_info(Some("no such device 7f3a")).unwrap_err().to_string();
    assert!(e.contains("not connected") && e.contains("no such device 7f3a"), "{e}");
}

#[test]
fn guarded_gives_up_after_the_limit_and_refuses_the_device_until_its_worker_returns() {
    let name = "test device that blocks 5c1e";
    assert_eq!(guarded(name, Duration::from_secs(1), false, || 7).unwrap(), 7);
    assert!(!is_blocked(name));
    let t0 = Instant::now();
    let e = guarded(name, Duration::from_millis(50), false, || {
        std::thread::sleep(Duration::from_millis(400));
    })
    .unwrap_err();
    assert!(
        t0.elapsed() < Duration::from_millis(300),
        "the caller does not wait for the worker"
    );
    assert!(matches!(e, TransportError::NoAnswer(_)), "{e}");
    assert!(
        e.to_string().contains("does not answer") && e.to_string().contains(name),
        "{e}"
    );
    assert!(is_blocked(name));
    // refused at once while the worker blocks; a close still runs (on its own thread)
    let t1 = Instant::now();
    assert!(matches!(
        guarded(name, Duration::from_secs(5), false, || 1),
        Err(TransportError::NoAnswer(_))
    ));
    assert!(t1.elapsed() < Duration::from_millis(100));
    let (tx, rx) = std::sync::mpsc::channel();
    let _ = guarded(name, CLOSE_LIMIT, true, move || tx.send(()).unwrap());
    rx.recv_timeout(Duration::from_secs(1)).expect("the close ran");
    // the worker returns: the device is free again
    std::thread::sleep(Duration::from_millis(500));
    assert!(!is_blocked(name));
    assert_eq!(guarded(name, Duration::from_secs(1), false, || 8).unwrap(), 8);
}

#[test]
fn resolve_choice_falls_back_to_the_system_default_with_a_message() {
    let avail = vec!["MacBook Pro Speakers".to_string(), "AirPods".to_string()];
    assert_eq!(resolve_choice(None, &avail), (Output::Default, None));
    assert_eq!(
        resolve_choice(Some("AirPods"), &avail),
        (Output::Device { name: "AirPods".into() }, None)
    );
    let (o, msg) = resolve_choice(Some("Studio Monitor"), &avail);
    assert_eq!(o, Output::Default);
    let msg = msg.unwrap();
    assert!(
        msg.contains("\"Studio Monitor\" is not connected") && msg.contains("system default"),
        "{msg}"
    );
    // the names must match exactly (no prefix or case match)
    assert_eq!(resolve_choice(Some("airpods"), &avail).0, Output::Default);
    assert_eq!(resolve_choice(Some("AirPods"), &[]).0, Output::Default);
}

#[test]
fn classify_error_separates_fatal_errors_from_glitches_and_notices() {
    use cpal::ErrorKind as K;
    assert_eq!(classify_error(K::Xrun), ErrorClass::Xrun);
    for k in [K::DeviceChanged, K::RealtimeDenied, K::DeviceBusy] {
        assert_eq!(classify_error(k), ErrorClass::Notice, "{k:?}");
    }
    for k in [
        K::DeviceNotAvailable,
        K::StreamInvalidated,
        K::BackendError,
        K::Other,
        K::HostUnavailable,
    ] {
        assert_eq!(classify_error(k), ErrorClass::Fatal, "{k:?}");
    }
}

#[test]
fn output_from_value() {
    assert_eq!(Output::from_value(Some("null")), Output::Null { sample_rate: None });
    assert_eq!(Output::from_value(Some("NULL")), Output::Null { sample_rate: None });
    assert_eq!(Output::from_value(Some("coreaudio")), Output::Default);
    assert_eq!(Output::from_value(None), Output::Default);
    // from_env reads the variable; it is not set in the test environment unless the
    // runner sets it, and either value is a valid choice
    let _ = Output::from_env();
}

#[test]
fn cpal_stream_is_send() {
    fn send<T: Send>() {}
    send::<Running>();
}

/// P3-16 no-audio finding (arm64-mac): `DRUMSPIKE_HANG_DEVICE=<a device whose stream build
/// blocks in CoreAudio>` ("Microsoft Teams Audio" while Teams does not run). The open
/// fails with [`TransportError::NoAnswer`] after [`OPEN_LIMIT`] instead of blocking for
/// ever, and a second open is refused at once while the first worker still blocks.
#[test]
#[ignore = "needs a device that does not answer (DRUMSPIKE_HANG_DEVICE)"]
fn a_device_that_does_not_answer_fails_within_the_open_limit() {
    use super::super::{TrackSpec, Transport, TransportConfig};
    use crate::io::{MixMode, WavFormat, WavSpec, write_wav};
    let name = std::env::var("DRUMSPIKE_HANG_DEVICE").expect("set DRUMSPIKE_HANG_DEVICE");
    let dir = std::env::temp_dir().join(format!("drumspike-hang-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("t.wav");
    let spec = WavSpec {
        sample_rate: 48_000,
        channels: 1,
        format: WavFormat::Float32,
        extensible: false,
        channel_mask: 0,
    };
    write_wav(&p, &spec, &vec![0.0; 4_800], None).unwrap();
    let tracks = vec![TrackSpec {
        path: p,
        mode: MixMode::Average,
        hits: vec![],
        click_pitch: 1000.0,
    }];
    let out = Output::Device { name: name.clone() };
    let t0 = Instant::now();
    let e = Transport::open(&tracks, out.clone(), TransportConfig::default())
        .err()
        .expect("no stream");
    let took = t0.elapsed();
    eprintln!("first open: {e} after {:.2} s", took.as_secs_f64());
    assert!(matches!(e, TransportError::NoAnswer(_)), "{e}");
    assert!(
        took >= OPEN_LIMIT && took < OPEN_LIMIT + Duration::from_secs(2),
        "{took:?}"
    );
    let t1 = Instant::now();
    let e2 = Transport::open(&tracks, out, TransportConfig::default())
        .err()
        .expect("refused");
    eprintln!("second open: {e2} after {:.3} s", t1.elapsed().as_secs_f64());
    assert!(matches!(e2, TransportError::NoAnswer(_)));
    assert!(t1.elapsed() < Duration::from_millis(500));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Process CPU seconds (user + system).
fn cpu_seconds() -> f64 {
    #[cfg(target_os = "macos")]
    {
        // SAFETY: getrusage fills a zeroed struct for the calling process.
        let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
        unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut ru) };
        let tv = |t: libc::timeval| t.tv_sec as f64 + t.tv_usec as f64 / 1e6;
        tv(ru.ru_utime) + tv(ru.ru_stime)
    }
    #[cfg(not(target_os = "macos"))]
    {
        f64::NAN
    }
}

/// Current resident set size in MB (`ps`).
fn rss_mb() -> f64 {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output();
    out.ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<f64>().ok())
        .map_or(f64::NAN, |kb| kb / 1024.0)
}

/// P3-5a device test (arm64-mac): `DRUMSPIKE_SESSION=<folder of make_session.py>`, optional
/// `DRUMSPIKE_DEVICE_SECS` (60), `DRUMSPIKE_RESIDENCY` (`pread`, the default, or `madvise`)
/// and P3-16 `DRUMSPIKE_DEVICE_NAME` (a named output device instead of the system default).
/// Plays the session on the default output at the device's current rate with a
/// 256-frame callback and master gain 0.0 after the mix (silent; the full mix and the
/// clicks still run). Checks: 0 underruns, the device rate unchanged, the mix ran.
/// Prints CPU, RSS growth and the resampler's share for the report.
#[test]
#[ignore = "needs the audio device and a generated session (DRUMSPIKE_SESSION)"]
fn device_plays_the_session_silently() {
    use super::super::{ClickSettings, TrackSpec, Transport, TransportConfig, clicks};
    use crate::io::MixMode;
    let dir = std::path::PathBuf::from(
        std::env::var("DRUMSPIKE_SESSION").expect("set DRUMSPIKE_SESSION to a make_session.py folder"),
    );
    let secs: f64 = std::env::var("DRUMSPIKE_DEVICE_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(60.0);
    let residency = match std::env::var("DRUMSPIKE_RESIDENCY").as_deref() {
        Ok("madvise") => super::super::reader::Residency::Madvise,
        _ => super::super::reader::Residency::Pread,
    };
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("session.json")).unwrap()).unwrap();
    let roles = ["kick", "snare", "tom", "hat", "cymbal", "other"];
    let tracks: Vec<TrackSpec> = manifest["tracks"]
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
            click_pitch: clicks::role_pitch(roles[i % roles.len()]),
        })
        .collect();
    let track_rate = manifest["sr"].as_u64().unwrap();
    let device_name = std::env::var("DRUMSPIKE_DEVICE_NAME").ok().filter(|n| !n.is_empty());
    let output = match &device_name {
        Some(name) => Output::Device { name: name.clone() },
        None => Output::Default,
    };
    let rate_before = output_info(device_name.as_deref()).unwrap().sample_rate;

    let rss_open = rss_mb();
    let cfg = TransportConfig {
        residency,
        ..TransportConfig::default()
    };
    let mut t = Transport::open(&tracks, output, cfg).unwrap();
    t.set_master_gain(0.0);
    t.set_clicks(&ClickSettings {
        enabled: true,
        volume: clicks::DEFAULT_CLICK_VOLUME,
        pitches: None,
    })
    .unwrap();
    t.play();
    let t_wait = Instant::now();
    while !t.playhead().playing {
        assert!(t_wait.elapsed() < Duration::from_secs(10), "no playback after 10 s");
        std::thread::sleep(Duration::from_millis(5));
    }
    let start_wait_ms = t_wait.elapsed().as_secs_f64() * 1e3;
    let s0 = t.stats();
    let rss0 = rss_mb();
    let cpu0 = cpu_seconds();
    let w0 = Instant::now();
    let mut rss_max = rss0;
    let mut drift_max = 0i64;
    while w0.elapsed().as_secs_f64() < secs {
        std::thread::sleep(Duration::from_millis(500));
        rss_max = rss_max.max(rss_mb());
        // the playhead extrapolated to now against the frames the callback rendered
        let p = t.playhead();
        let now = super::super::clock::host_now_ns();
        let rendered = t.stats().rendered_frames as i64;
        drift_max = drift_max.max((p.frame_at(now) as i64 - rendered).abs());
    }
    let wall = w0.elapsed().as_secs_f64();
    let meters = t.take_meters();
    let cpu = (cpu_seconds() - cpu0) / wall * 100.0;
    let s1 = t.stats();
    let p = t.playhead();
    let info = t.device().clone();
    t.pause();
    drop(t);
    let rate_after = output_info(device_name.as_deref()).unwrap().sample_rate;
    let resample_pct = (s1.resample_ns - s0.resample_ns) as f64 / 1e9 / wall * 100.0;
    let reader_pct = (s1.reader_ns - s0.reader_ns) as f64 / 1e9 / wall * 100.0;
    println!(
        "DEVICE {} | {} Hz device, {} Hz tracks, {} tracks, {:?} | buffer {:?}, frames per callback {}..{} | \
         {:.1} s, {} callbacks, rendered {} frames ({:.2} s), playhead {} | underruns {} ({} frames), stream errors {} | \
         process CPU {:.2} % | reader {:.2} % of one core, resampler {:.2} % | RSS at open {:.0} MB, at play {:.0} MB, max {:.0} MB, growth {:.0} MB | \
         pre-gain peak {:.3} | start {:.0} ms | playhead vs rendered max {} frames | device rate before {} after {} | \
         channels {} | master meter {:?} | stream xruns {} warnings {}",
        info.name,
        info.sample_rate,
        track_rate,
        tracks.len(),
        residency,
        info.buffer_frames,
        s1.frames_min,
        s1.frames_max,
        wall,
        s1.callbacks - s0.callbacks,
        s1.rendered_frames - s0.rendered_frames,
        (s1.rendered_frames - s0.rendered_frames) as f64 / f64::from(info.sample_rate),
        p.frame,
        s1.underruns,
        s1.underrun_frames,
        s1.stream_errors,
        cpu,
        reader_pct,
        resample_pct,
        rss_open,
        rss0,
        rss_max,
        rss_max - rss0,
        s1.peak,
        start_wait_ms,
        drift_max,
        rate_before,
        rate_after,
        info.channels,
        meters.master,
        s1.stream_xruns,
        s1.stream_warnings
    );
    assert!(meters.master[0] > 0.0, "the master meter runs before the master gain");
    assert_eq!(rate_before, rate_after, "the device rate must not change");
    assert_eq!(info.sample_rate, rate_before);
    assert_eq!(s1.underruns, 0, "underruns");
    assert!(s1.peak > 0.0, "the mix ran");
    assert!((s1.rendered_frames as f64) >= 0.95 * secs * f64::from(info.sample_rate));
}
