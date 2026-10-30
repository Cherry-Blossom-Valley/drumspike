//! #363: the [`DetectorBackend`] seam. The DspBackend path is byte-identical to the old
//! `Detector::Dsp` call (the golden guarantee of the ticket: the default analysis output
//! does not change), and the `ExternalDetector` adapter runs the example plugin
//! (`tests/fixtures/plugins/json_hits.py`) and the error cases of the design section 5.

use std::path::{Path, PathBuf};

use drumspike_core::backend::{BackendError, DetectorBackend, DspBackend, ExternalDetector};
use drumspike_core::{AnalyzeParams, Detector, MixMode, analyze_track, read_wav_mono};

/// A 10 s synthetic kick train: the same signal the detector tests use.
fn signal() -> Vec<f32> {
    let sr = 48000u32;
    let mut x = vec![0.0f32; 10 * sr as usize];
    for n in 0..10usize {
        let start = n * sr as usize;
        for (i, s) in x[start..start + 4000].iter_mut().enumerate() {
            let env = (-(i as f32) / 1500.0).exp();
            *s = 0.8 * env * (2.0 * std::f32::consts::PI * 60.0 * i as f32 / sr as f32).sin();
        }
    }
    x
}

/// The DspBackend path equals the old `Detector::Dsp` call, on the fixture WAVs when they
/// are present (the three real sessions, read only) and on the synthetic signal always.
#[test]
fn the_dsp_backend_path_equals_the_old_call() {
    let params = AnalyzeParams::default();
    let x = signal();
    let events_new = analyze_track(&x, 48000, "kick", &params, Detector::Backend(&DspBackend)).unwrap();
    let events_old = analyze_track(&x, 48000, "kick", &params, Detector::Dsp).unwrap();
    assert_eq!(events_new.events.len(), events_old.events.len());
    for (a, b) in events_new.events.iter().zip(events_old.events.iter()) {
        assert_eq!(a.t, b.t, "byte-identical times");
        assert_eq!(a.rise, b.rise, "byte-identical rise");
        assert_eq!(a.v, b.v, "byte-identical velocities");
    }
    if let Some(dir) = samples_dir() {
        for (session, kick, snare) in [
            ("Session A", "k in_02.wav", "s top_02.wav"),
            ("Session B", "k in_01.wav", "s top_01.wav"),
            ("Session C", "k in_02.wav", "s top_02.wav"),
        ] {
            let audio = dir.join(session).join("Audio Files");
            for (path, role) in [(audio.join(kick), "kick"), (audio.join(snare), "snare")] {
                if !path.is_file() {
                    continue;
                }
                let (x, info) = read_wav_mono(&path, MixMode::Average).unwrap();
                let a = analyze_track(&x, info.sample_rate, role, &params, Detector::Backend(&DspBackend)).unwrap();
                let b = analyze_track(&x, info.sample_rate, role, &params, Detector::Dsp).unwrap();
                assert_eq!(a.events.len(), b.events.len(), "{session} {role}");
                for (e1, e2) in a.events.iter().zip(b.events.iter()) {
                    assert_eq!(e1.t, e2.t);
                    assert_eq!(e1.rise, e2.rise);
                    assert_eq!(e1.v, e2.v);
                }
            }
        }
    }
}

fn samples_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("HOME")?).join(".cache/drumspike/sessions");
    dir.is_dir().then_some(dir)
}

/// The fixture plugin folder: a copy of `tests/fixtures/plugins` in the scratch dir, with a
/// `times.json` written for the test.
struct PluginFixture {
    dir: PathBuf,
}

impl PluginFixture {
    fn new(name: &str, times: &str) -> Self {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/plugins");
        let dir = std::env::temp_dir().join(format!("drumspike-plugin-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = "json_hits.py";
        std::fs::copy(src.join(file), dir.join(file)).unwrap();
        std::fs::write(dir.join("times.json"), times).unwrap();
        // executable bit on the script (git keeps it; the copy may not)
        let script = dir.join("json_hits.py");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut p = std::fs::metadata(&script).unwrap().permissions();
            p.set_mode(0o755);
            std::fs::set_permissions(&script, p).unwrap();
        }
        PluginFixture { dir }
    }

    fn detector(&self, timeout_s: u64) -> ExternalDetector {
        ExternalDetector {
            id: "json-hits".into(),
            executable: self.dir.join("json_hits.py"),
            timeout_s,
            params: serde_json::json!({}),
        }
    }
}

impl Drop for PluginFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The adapter runs the example plugin and returns its hits through the trait.
#[test]
fn the_adapter_runs_the_example_plugin() {
    let plugin = PluginFixture::new("run", r#"{"kick": [0.5, 1.5, 2.5], "snare": [1.0]}"#);
    let x = vec![0.0f32; 48000 * 4];
    let (hits, cands) = plugin.detector(120).detect(&x, 48000, "kick").unwrap();
    let times: Vec<f64> = hits.iter().map(|h| h.time).collect();
    assert_eq!(times, vec![0.5, 1.5, 2.5]);
    assert!(cands.is_none());
    // the class of a kick hit maps to the internal kick note through the dispatch
    let x = signal();
    let params = AnalyzeParams::default();
    let events = analyze_track(&x, 48000, "kick", &params, Detector::Backend(&plugin.detector(120))).unwrap();
    assert_eq!(events.events.len(), 3);
    assert_eq!(events.events[0].t, 0.5);
}

/// Section 5, adapter rows: no response line, more than one line, non-zero exit with the
/// stderr tail, timeout, invalid JSON and a missing executable each fail the track with a
/// readable error.
#[test]
fn the_adapter_error_cases_of_section_5() {
    let plugin = PluginFixture::new("errors", "{}");
    let script = plugin.dir.join("json_hits.py");
    let x = vec![0.0f32; 4800];
    let run = |timeout_s: u64| -> BackendError { plugin.detector(timeout_s).detect(&x, 48000, "kick").unwrap_err() };

    // no response line
    std::fs::write(&script, "#!/bin/sh\nexit 0\n").unwrap();
    assert!(matches!(run(120), BackendError::Process(_)));

    // more than one response line
    std::fs::write(&script, "#!/bin/sh\necho '{}'\necho '{}'\n").unwrap();
    assert!(matches!(run(120), BackendError::Process(_)));

    // non-zero exit with a stderr tail
    std::fs::write(&script, "#!/bin/sh\necho boom >&2\nexit 3\n").unwrap();
    match run(120) {
        BackendError::Process(msg) => assert!(msg.contains("boom") && msg.contains("3"), "{msg}"),
        e => panic!("{e:?}"),
    }

    // timeout (the process is killed)
    std::fs::write(&script, "#!/bin/sh\nsleep 30\n").unwrap();
    assert!(matches!(run(1), BackendError::Process(_)));

    // invalid JSON
    std::fs::write(&script, "#!/bin/sh\necho 'not json'\n").unwrap();
    match run(120) {
        BackendError::Json(_) => {}
        e => panic!("expected Json, got {e:?}"),
    }

    // a missing executable is not runnable
    let missing = ExternalDetector {
        id: "gone".into(),
        executable: plugin.dir.join("no_such_plugin"),
        timeout_s: 120,
        params: serde_json::json!({}),
    };
    assert!(matches!(
        missing.detect(&x, 48000, "kick").unwrap_err(),
        BackendError::Process(_)
    ));
}
