//! The transport commands (P3-5b) through `drumspike_app::Service` on the null device: a
//! thread renders 256-frame blocks at real-time pace, so the clock, the playhead messages,
//! mute, solo, seek and the device change run as on a real device, without sound.
//! The test names start with the command name; `command_tests.rs` checks that.

mod common;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::*;
use drumspike_app::transport::PLAYHEAD_POLL;
use drumspike_app::{DeviceKey, Progress, Service, TRANSPORT_COMMANDS};
use drumspike_core::transport::device::Output;
use serde_json::{Value, json};

fn null() -> Output {
    Output::Null { sample_rate: None }
}

/// A silent service with a project of the given tracks; returns (service, sid, root).
fn opened(tag: &str, names: &[&'static str]) -> (Service, String, std::path::PathBuf) {
    opened_dur(tag, names, 3.5)
}

/// [`opened`] with a clip of `dur` seconds ([`common::kick_spec_dur`]: the 0.5 s onset
/// grid continues to the end, so a longer clip has no silent tail).
fn opened_dur(tag: &str, names: &[&'static str], dur: f64) -> (Service, String, std::path::PathBuf) {
    let root = tmp(tag);
    let specs: Vec<Spec> = names.iter().map(|n| kick_spec_dur(n, "kick", 36, dur)).collect();
    let dir = project(&root, "Song", &specs);
    let svc = service_dsp();
    svc.set_output_select(null());
    svc.set_master_gain(0.0);
    let sid = svc.project_open(dir.to_str().unwrap()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    (svc, sid, root)
}

type Log = Arc<Mutex<Vec<Value>>>;

/// The playhead messages only (P3-16: the meter messages share the channel; see
/// [`collector_all`]).
fn collector() -> (Progress, Log) {
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let l = log.clone();
    let p: Progress = Arc::new(move |v: Value| {
        if v.get("meters").is_none() {
            l.lock().unwrap().push(v);
        }
    });
    (p, log)
}

/// Every message of the channel, the meter messages too.
fn collector_all() -> (Progress, Log) {
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let l = log.clone();
    let p: Progress = Arc::new(move |v: Value| l.lock().unwrap().push(v));
    (p, log)
}

fn wait_for(what: &str, secs: f64, mut f: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs_f64(secs);
    while !f() {
        assert!(Instant::now() < end, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn ms(n: u64) {
    std::thread::sleep(Duration::from_millis(n));
}

/// P4-40d-t: the throttle of this test process, not of the product. The median of 15 sleeps
/// of the sender's own poll ([`PLAYHEAD_POLL`]): about 20 ms at normal priority, more at a
/// lowered class or under a timer coalescing floor (about 160 ms measured on this host in
/// every class). The bounds of the timing tests scale with it, so a halved sender fails at
/// every scheduling class; sleeping the sender's own poll keeps the calibration and the
/// sender's achievable rate from drifting apart (review 6227692703).
fn look_secs() -> f64 {
    let mut samples: Vec<f64> = (0..15)
        .map(|_| {
            let t0 = Instant::now();
            std::thread::sleep(PLAYHEAD_POLL);
            t0.elapsed().as_secs_f64()
        })
        .collect();
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    samples[7]
}

#[test]
fn transport_play_starts_the_clock_and_sends_the_playhead_on_changes_and_at_4_hz() {
    let (svc, sid, root) = opened("tp_play", &["Kick_In", "Snare_Top"]);
    let (p, log) = collector();
    assert!(
        !drumspike_app::transport::watcher_active(&svc),
        "no thread before the first play"
    );
    let r = svc.transport_play(&sid, Some(0.5), None, Some(p)).unwrap();
    assert!(
        drumspike_app::transport::watcher_active(&svc),
        "the device watcher runs once a transport is open"
    );
    assert_eq!(r["ready"], true, "{r}");
    assert_eq!(r["rate"], 48_000);
    assert_eq!(r["tracks"], 2);
    assert_eq!(r["master"], 0.0, "the test service is silent");
    assert!((r["duration"].as_f64().unwrap() - 3.5).abs() < 0.01);
    assert!(r["start_ms"].as_f64().unwrap() < 5_000.0);
    ms(1_000);
    let st = svc.transport_status();
    assert_eq!(st["playing"], true);
    let pos = st["pos"].as_f64().unwrap();
    assert!((1.3..1.9).contains(&pos), "0.5 s + about 1 s of playing, got {pos}");
    assert_eq!(st["underruns"], 0);
    assert_eq!(st["master"], 0.0);
    assert!(st["peak"].as_f64().unwrap() > 0.0, "the mix ran with the master at 0");
    let msgs = log.lock().unwrap().clone();
    // P3-15a: 4 Hz while nothing changes, not 30 Hz — P4-40d-t: the rate from the playing
    // messages' own t_ns, so a throttled test thread does not stretch or shrink the window. The
    // lower bound still fails when the playhead thread stops or halves its rate (4 Hz / 2 = 2).
    let playing: Vec<u64> = msgs
        .iter()
        .filter(|m| m["playing"] == true)
        .map(|m| m["t_ns"].as_u64().unwrap())
        .collect();
    assert!(playing.len() >= 3, "only {} playing messages", playing.len());
    let secs = (playing.last().unwrap() - playing.first().unwrap()) as f64 / 1e9;
    let hz = (playing.len() - 1) as f64 / secs;
    assert!(
        (2.5..=6.0).contains(&hz),
        "{hz:.2} Hz from {} playing messages over {secs:.2} s",
        playing.len()
    );
    // every message is consistent: pos = frame / rate, t_ns grows
    let mut last_t = 0u64;
    for m in &msgs {
        let (f, rt) = (m["frame"].as_u64().unwrap(), m["rate"].as_u64().unwrap());
        assert!((m["pos"].as_f64().unwrap() - f as f64 / rt as f64).abs() < 1e-9);
        let t = m["t_ns"].as_u64().unwrap();
        assert!(t >= last_t);
        last_t = t;
        assert_eq!(m["sid"], json!(sid));
        assert_eq!(m["end"], false);
    }
    let first_playing = msgs.iter().find(|m| m["playing"] == true).unwrap();
    assert!(first_playing["pos"].as_f64().unwrap() >= 0.49);
    // a seek is a change: its message goes out within one poll of the seek, not after the 250 ms
    // period — P4-40d-t: at an unthrottled look the message is measured from the seek call to
    // its arrival in the log (bound 150 ms), the test-side measure that passed CI for months;
    // the gap to the last message before it is period-bound (the playhead sends at 4 Hz) and
    // cannot show promptness. At a throttled look (>= 60 ms) the test-side measure is dominated
    // by the throttled sleeps, so the gap from the messages' own t_ns is used with the 700 ms
    // bound (one period plus a throttled poll, rounded up); it only catches a stalled or
    // detached sender. The fresh position itself is checked by the seek stress test.
    let look = look_secs();
    let n0 = log.lock().unwrap().len();
    let t0 = Instant::now();
    let e = svc.transport_seek(2.0).unwrap()["epoch"].as_u64().unwrap();
    wait_for("the message of the seek", 2.0, || {
        log.lock().unwrap()[n0..].iter().any(|m| m["epoch"].as_u64() == Some(e))
    });
    let seen = t0.elapsed();
    let msgs = log.lock().unwrap().clone();
    let seek_t = msgs[n0..].iter().find(|m| m["epoch"].as_u64() == Some(e)).unwrap()["t_ns"]
        .as_u64()
        .unwrap();
    let prev_t = msgs[..n0].last().map_or(seek_t, |m| m["t_ns"].as_u64().unwrap());
    let gap_ms = seek_t.saturating_sub(prev_t) / 1_000_000;
    if look < 0.060 {
        assert!(
            seen.as_millis() < 150,
            "the seek message took {} ms to reach the log (bound 150 ms, look {:.0} ms)",
            seen.as_millis(),
            look * 1000.0
        );
    } else {
        assert!(
            gap_ms < 700,
            "the seek message {gap_ms} ms after the last message before it (bound 700 ms, look {:.0} ms)",
            look * 1000.0
        );
    }
    let _ = svc.transport_stop(true);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn transport_stop_pauses_at_a_position_that_holds_and_ends_the_messages() {
    let (svc, sid, root) = opened("tp_stop", &["Kick_In"]);
    assert_eq!(svc.transport_stop(false)["open"], false, "nothing to stop");
    let (p, log) = collector();
    svc.transport_play(&sid, Some(0.0), None, Some(p)).unwrap();
    ms(400);
    let r = svc.transport_stop(false);
    assert_eq!(r["playing"], false);
    assert_eq!(r["open"], true);
    let pos = r["pos"].as_f64().unwrap();
    assert!((0.2..0.9).contains(&pos), "{pos}");
    ms(300);
    let st = svc.transport_status();
    assert_eq!(st["playing"], false);
    assert_eq!(st["pos"].as_f64().unwrap(), pos, "the paused position holds");
    wait_for("the end message", 2.0, || {
        log.lock().unwrap().last().is_some_and(|m| m["end"] == true)
    });
    let n = log.lock().unwrap().len();
    ms(150);
    assert_eq!(log.lock().unwrap().len(), n, "no message after the end");
    // play again continues from the paused position (no position given)
    svc.transport_play(&sid, None, None, None).unwrap();
    ms(300);
    assert!(svc.transport_status()["pos"].as_f64().unwrap() > pos + 0.1);
    // close releases the transport
    let c = svc.transport_stop(true);
    assert_eq!(c["open"], false);
    assert_eq!(svc.transport_status()["open"], false);
    std::fs::remove_dir_all(root).unwrap();
}

/// P3-15c: the seek waits for the callback without the host lock, so the playhead thread runs during
/// the wait. No message may carry the epoch of a seek with a position from before that seek: the page
/// trusts every message whose epoch is not older than its last seek answer. (During the wait of the
/// next seek a message of that epoch may already show the next target, with `playing` false.)
#[test]
fn transport_seek_sends_no_message_with_the_new_epoch_and_the_old_position() {
    let (svc, sid, root) = opened("tp_seek_epoch", &["Kick_In", "Snare_Top"]);
    let (p, log) = collector();
    svc.transport_play(&sid, Some(0.2), None, Some(p)).unwrap();
    let targets = [0.3, 1.1, 1.9, 2.7];
    let mut answers: Vec<(u64, f64)> = Vec::new();
    for k in 0..24 {
        let target = targets[k % 4];
        let r = svc.transport_seek(target).unwrap();
        answers.push((r["epoch"].as_u64().unwrap(), target));
        ms(30 + (k as u64 % 3) * 7);
    }
    let _ = svc.transport_stop(true);
    let msgs = log.lock().unwrap().clone();
    let near = |pos: f64, t: f64| (t - 0.01..t + 0.3).contains(&pos);
    let mut checked = 0;
    for (i, (epoch, target)) in answers.iter().enumerate() {
        let next = answers.get(i + 1).copied();
        for m in msgs.iter().filter(|m| {
            let e = m["epoch"].as_u64().unwrap();
            e >= *epoch && e < next.map_or(u64::MAX, |a| a.0) && m["end"] == false
        }) {
            let pos = m["pos"].as_f64().unwrap();
            let ok = near(pos, *target) || next.is_some_and(|(_, t)| near(pos, t) && m["playing"] == false);
            assert!(
                ok,
                "seek {i} to {target} (epoch {epoch}): a message of that epoch shows {pos}: {m}"
            );
            checked += 1;
        }
    }
    assert!(checked >= 12, "{checked} messages checked");
    std::fs::remove_dir_all(root).unwrap();
}

/// One entry of the stress log, in the order the page would get it.
enum SeekEv {
    /// `transport_seek` was called (the page drops every message until the answer).
    Ask,
    /// The answer of a seek: its epoch and target.
    Answer(u64, f64),
    /// A playhead message.
    Msg(Value),
}

fn env_n(name: &str, default: usize) -> usize {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

/// P3-15f: the stress version of the test above. Many seeks; `DRUMSPIKE_SEEK_STRESS_LOAD` busy threads
/// (default 0) make the CPU load that opened the window on x86 (CI run 38162126566), and
/// `DRUMSPIKE_SEEK_STRESS_SEEKS` sets the number of seeks (default 160). Two checks over every message:
/// by epoch, a message whose epoch is at least that of seek i and older than seek i+1 shows the target
/// of seek i (no allowance for the next target any more); as the page sees it (`appOnPlayhead`: no
/// message between a seek and its answer, none older than the answer), an accepted message shows the
/// target of the last answered seek. The test prints the seek time p50 and p95.
#[test]
fn transport_seek_stress_every_message_of_a_seek_epoch_shows_that_seek() {
    let seeks = env_n("DRUMSPIKE_SEEK_STRESS_SEEKS", 160);
    let load = env_n("DRUMSPIKE_SEEK_STRESS_LOAD", 0);
    let (svc, sid, root) = opened("tp_seek_stress", &["Kick_In", "Snare_Top"]);
    let log: Arc<Mutex<Vec<SeekEv>>> = Arc::new(Mutex::new(Vec::new()));
    let l = log.clone();
    // The page routes meter messages (`meters`, no epoch) away before it reads the playhead.
    let p: Progress = Arc::new(move |v: Value| {
        if v.get("meters").is_none() {
            l.lock().unwrap().push(SeekEv::Msg(v));
        }
    });
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let busy: Vec<_> = (0..load)
        .map(|_| {
            let stop = stop.clone();
            std::thread::spawn(move || {
                let mut x = 0u64;
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    x = std::hint::black_box(x.wrapping_mul(6364136223846793005).wrapping_add(1));
                }
            })
        })
        .collect();
    let play = svc.transport_play(&sid, Some(0.2), None, Some(p)).unwrap();
    log.lock()
        .unwrap()
        .push(SeekEv::Answer(play["epoch"].as_u64().unwrap(), 0.2));
    let targets = [0.3, 1.1, 1.9, 2.7];
    let mut took = Vec::with_capacity(seeks);
    for k in 0..seeks {
        let target = targets[k % 4];
        log.lock().unwrap().push(SeekEv::Ask);
        let t0 = Instant::now();
        let r = svc.transport_seek(target).unwrap();
        took.push(t0.elapsed().as_secs_f64() * 1e3);
        log.lock()
            .unwrap()
            .push(SeekEv::Answer(r["epoch"].as_u64().unwrap(), target));
        // gaps of 5 to 41 ms: some seeks come before the refill ends, some after
        ms(5 + (k as u64 * 7) % 37);
    }
    let _ = svc.transport_stop(true);
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    for b in busy {
        b.join().unwrap();
    }
    took.sort_by(f64::total_cmp);
    let q = |f: f64| took[((took.len() - 1) as f64 * f).round() as usize];
    eprintln!(
        "seek stress: {seeks} seeks, load threads {load}: seek p50 {:.2} ms, p95 {:.2} ms, max {:.2} ms",
        q(0.5),
        q(0.95),
        q(1.0)
    );
    // positions move on while playing: up to 0.5 s after the target counts as that target
    let near = |pos: f64, t: f64| (t - 0.01..t + 0.5).contains(&pos);
    let log = std::mem::take(&mut *log.lock().unwrap());
    let answers: Vec<(u64, f64)> = log
        .iter()
        .filter_map(|e| match e {
            SeekEv::Answer(a, t) => Some((*a, *t)),
            _ => None,
        })
        .collect();
    let msgs: Vec<&Value> = log
        .iter()
        .filter_map(|e| match e {
            SeekEv::Msg(m) if m["end"] == false => Some(m),
            _ => None,
        })
        .collect();
    // by epoch
    let (mut checked, mut old_pos, mut next_playing, mut next_paused) = (0, 0, 0, 0);
    let mut first: Option<String> = None;
    for (i, (epoch, target)) in answers.iter().enumerate() {
        let next = answers.get(i + 1).copied();
        for m in msgs.iter().filter(|m| {
            let e = m["epoch"].as_u64().unwrap();
            e >= *epoch && e < next.map_or(u64::MAX, |a| a.0)
        }) {
            checked += 1;
            let pos = m["pos"].as_f64().unwrap();
            if near(pos, *target) {
                continue;
            }
            match next {
                Some((_, t)) if near(pos, t) && m["playing"] == true => next_playing += 1,
                Some((_, t)) if near(pos, t) => next_paused += 1,
                _ => old_pos += 1,
            }
            first.get_or_insert_with(|| format!("answer {i} to {target} (epoch {epoch}): {m}"));
        }
    }
    // as the page sees it
    let (mut seeking, mut min_epoch, mut last) = (false, 0u64, 0.2f64);
    let (mut accepted, mut jumps) = (0, 0);
    for e in &log {
        match e {
            SeekEv::Ask => seeking = true,
            SeekEv::Answer(a, t) => {
                seeking = false;
                min_epoch = *a;
                last = *t;
            }
            SeekEv::Msg(m) => {
                if seeking || m["end"] == true || m["epoch"].as_u64().unwrap() < min_epoch {
                    continue;
                }
                accepted += 1;
                let pos = m["pos"].as_f64().unwrap();
                if !near(pos, last) {
                    jumps += 1;
                    first.get_or_insert_with(|| format!("the page accepts {m} after the answer to {last}"));
                }
            }
        }
    }
    eprintln!(
        "seek stress: {checked} messages by epoch: {old_pos} with an old position, {next_playing} with the next \
         target while playing, {next_paused} with the next target while paused; {accepted} accepted by the page, \
         {jumps} away from the target"
    );
    assert!(
        old_pos + next_playing + next_paused + jumps == 0,
        "a message does not show the seek of its epoch; first: {}",
        first.unwrap_or_default()
    );
    assert!(checked >= seeks / 2, "{checked} messages checked");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn transport_seek_moves_the_playhead_while_playing_and_while_paused() {
    let (svc, sid, root) = opened("tp_seek", &["Kick_In", "Snare_Top"]);
    assert_eq!(svc.transport_seek(1.0).unwrap_err().code, 409, "not open yet");
    svc.transport_play(&sid, Some(0.0), None, None).unwrap();
    ms(200);
    let s = svc.transport_seek(2.5).unwrap();
    assert_eq!(s["frame"], 120_000);
    assert_eq!(s["pos"], 2.5);
    let e1 = s["epoch"].as_u64().unwrap();
    wait_for("playing after the seek", 3.0, || {
        let st = svc.transport_status();
        st["playing"] == true && st["pos"].as_f64().unwrap() >= 2.5
    });
    let pos = svc.transport_status()["pos"].as_f64().unwrap();
    assert!((2.5..2.9).contains(&pos), "{pos}");
    let s2 = svc.transport_seek(0.25).unwrap();
    assert!(s2["epoch"].as_u64().unwrap() > e1, "every seek has a new epoch");
    // paused seek: the position moves, the transport stays paused
    let _ = svc.transport_stop(false);
    let s3 = svc.transport_seek(1.0).unwrap();
    assert_eq!(s3["frame"], 48_000);
    wait_for("the paused seek", 2.0, || {
        svc.transport_status()["pos"].as_f64().unwrap() == 1.0
    });
    ms(150);
    let st = svc.transport_status();
    assert_eq!(st["playing"], false);
    assert_eq!(st["pos"], 1.0);
    // bad arguments
    assert_eq!(svc.transport_seek(-1.0).unwrap_err().code, 400);
    assert_eq!(svc.transport_seek(f64::NAN).unwrap_err().code, 400);
    let _ = svc.transport_stop(true);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn transport_mix_mutes_and_solos_by_name_and_reports_the_latency() {
    let (svc, sid, root) = opened("tp_mix", &["Kick_In", "Snare_Top", "Tom_1"]);
    // before the transport opens the state is kept and used at the open
    let r = svc.transport_mix(&json!({"Snare_Top": true}), &Value::Null).unwrap();
    assert_eq!(r["applied"], false);
    svc.transport_play(&sid, Some(0.0), None, None).unwrap();
    assert_eq!(
        svc.transport_status()["audible"],
        json!(["Kick_In", "Tom_1"]),
        "the early mute applies at open"
    );
    ms(100);
    let r = svc
        .transport_mix(&json!({"Snare_Top": true, "Kick_In": true}), &json!({}))
        .unwrap();
    assert_eq!(r["applied"], true);
    assert_eq!(r["tracks_audible"], 1);
    // #239: the bound is the engine's own block count, read from the status answer.
    // mix_latency_blocks counts the mixed blocks between the gain set and the first block
    // that carries the new gains: 1-2 when healthy (the set lands in or just after a
    // block, the next block applies), and it does not move while the callback is starved
    // - no block runs, no count moves. The 20x20 measurement (results/239) shows one wall
    // clock apply reaching 33-38 ms under the host's CI runner plus a parallel `cargo
    // test --workspace` while the engine still applies within 1-2 blocks of 5.33 ms, so
    // the wall clock measures the scheduler, not the engine. A regression that applies
    // the gains N blocks late on EVERY apply reads N on every sample (the planted demo,
    // results/239cb-regression), so all 9 applies must stay within 2 blocks. The
    // wall-clock median of 9 stays as an info print; a stall past the command's own 60 ms
    // wait answers Null.
    let read_blocks = || {
        svc.transport_status()["mix_latency_blocks"]
            .as_u64()
            .unwrap_or_else(|| panic!("mix_latency_blocks missing from the status answer"))
    };
    let b0 = read_blocks();
    assert!(b0 <= 2, "mute heard within one block or two: apply 1 took {b0} blocks");
    let mut lats = vec![r["latency_ms"].as_f64().unwrap_or(60.0)];
    let mut blocks = vec![b0];
    for k in 0..8 {
        let mute = if k % 2 == 0 {
            json!({"Snare_Top": false})
        } else {
            json!({"Snare_Top": true})
        };
        let r = svc.transport_mix(&mute, &Value::Null).unwrap();
        assert_eq!(r["applied"], true, "{r}");
        lats.push(r["latency_ms"].as_f64().unwrap_or(60.0));
        let b = read_blocks();
        assert!(
            b <= 2,
            "mute heard within one block or two: apply {} took {} blocks",
            k + 1,
            b
        );
        blocks.push(b);
    }
    lats.sort_by(f64::total_cmp);
    let lat = lats[4];
    // #239: one line per run, the 20x20 measurement reads it
    eprintln!("MUTE_LATENCY_MS={lat:.3} SAMPLES={lats:?} BLOCKS={blocks:?}");
    // solo wins over mute
    let r = svc
        .transport_mix(&json!({"Kick_In": true}), &json!({"Kick_In": true, "Tom_1": true}))
        .unwrap();
    assert_eq!(r["tracks_audible"], 2);
    assert_eq!(svc.transport_status()["audible"], json!(["Kick_In", "Tom_1"]));
    // a name that is not a track is ignored; a wrong type is a 400
    svc.transport_mix(&json!({"Nope": true}), &Value::Null).unwrap();
    assert_eq!(
        svc.transport_status()["audible"],
        json!(["Kick_In", "Snare_Top", "Tom_1"])
    );
    assert_eq!(svc.transport_mix(&json!([1]), &Value::Null).unwrap_err().code, 400);
    let _ = svc.transport_stop(true);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn transport_clicks_apply_to_the_open_transport_and_to_the_next_open() {
    let (svc, sid, root) = opened("tp_clicks", &["Kick_In", "Snare_Top"]);
    let r = svc
        .transport_clicks(Some(false), Some(0.5), Some(&json!({"kick": 880.0})))
        .unwrap();
    assert_eq!(r["applied"], false);
    assert_eq!(r["enabled"], false);
    assert_eq!(r["volume"], 0.5);
    svc.transport_play(&sid, Some(0.0), None, None).unwrap();
    assert_eq!(
        svc.transport_status()["clicks"],
        json!({"enabled": false, "volume": 0.5})
    );
    let r = svc
        .transport_clicks(Some(true), None, Some(&json!({"snare": 3000.0, "kick": 900.0})))
        .unwrap();
    assert_eq!(r["applied"], true);
    assert_eq!(r["enabled"], true);
    assert_eq!(r["volume"], 0.5, "a missing field stays");
    // wrong arguments
    for (v, p) in [
        (Some(2.0), None),
        (Some(f64::NAN), None),
        (None, Some(json!({"kick": -5}))),
        (None, Some(json!([1]))),
        (None, Some(json!({"kick": "x"}))),
    ] {
        assert_eq!(
            svc.transport_clicks(None, v, p.as_ref()).unwrap_err().code,
            400,
            "{v:?} {p:?}"
        );
    }
    // many quick changes do not fail (the update queue drains once per block)
    for i in 0..20 {
        svc.transport_clicks(None, Some(0.1 + 0.01 * i as f64), None).unwrap();
    }
    let _ = svc.transport_stop(true);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn transport_play_uses_the_events_of_the_call_for_the_clicks() {
    let (svc, sid, root) = opened("tp_events", &["Kick_In", "Snare_Top"]);
    // nothing analysed yet: the project holds no events
    svc.transport_play(&sid, Some(0.0), None, None).unwrap();
    assert_eq!(svc.transport_status()["hits"], 0);
    let _ = svc.transport_stop(false);
    // events as numbers and as objects; the same sample twice counts once
    let ev = json!({"Kick_In": [0.5, 1.0, 1.0, 2.0], "Snare_Top": [{"t": 0.75, "v": 90}, {"t": 1.5}]});
    svc.transport_play(&sid, Some(0.0), Some(&ev), None).unwrap();
    assert_eq!(svc.transport_status()["hits"], 5);
    let _ = svc.transport_stop(false);
    // the project's own events count when the call has none
    svc.events_set(&sid, "Kick_In", &json!([{"t": 0.25, "v": 100}, {"t": 0.5, "v": 90}]))
        .unwrap();
    svc.transport_play(&sid, Some(0.0), None, None).unwrap();
    assert_eq!(svc.transport_status()["hits"], 2);
    // a bad position is a 400
    assert_eq!(svc.transport_play(&sid, Some(-1.0), None, None).unwrap_err().code, 400);
    assert_eq!(svc.transport_play("nope", None, None, None).unwrap_err().code, 404);
    let _ = svc.transport_stop(true);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn transport_status_of_a_closed_transport_has_the_host_fields_only() {
    let svc = service_dsp();
    svc.set_output_select(null());
    let st = svc.transport_status();
    assert_eq!(st["open"], false);
    assert_eq!(st["reopens"], 0);
    assert_eq!(st["lost"], Value::Null);
    assert!(st.get("frame").is_none());
    assert_eq!(st["clicks"]["enabled"], true);
    assert_eq!(st["clicks"]["volume"], 0.22f32 as f64);
}

#[test]
fn transport_play_skips_offline_tracks_and_refuses_a_project_without_audio() {
    let (svc, sid, root) = opened("tp_off", &["Kick_In", "Snare_Top"]);
    std::fs::remove_file(root.join("daw/Snare_Top.wav")).unwrap();
    let r = svc.transport_play(&sid, Some(0.0), None, None).unwrap();
    assert_eq!(r["tracks"], 1, "the offline track is left out");
    let _ = svc.transport_stop(true);
    std::fs::remove_file(root.join("daw/Kick_In.wav")).unwrap();
    let e = svc.transport_play(&sid, None, None, None).unwrap_err();
    assert_eq!(e.code, 409);
    assert!(e.message.contains("no track with an audio file"), "{e}");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn transport_play_opens_again_when_the_tracks_change() {
    let (svc, sid, root) = opened("tp_reopen_tracks", &["Kick_In", "Snare_Top"]);
    svc.transport_play(&sid, Some(0.0), None, None).unwrap();
    assert_eq!(svc.transport_status()["tracks"], 2);
    let _ = svc.transport_stop(false);
    std::fs::remove_file(root.join("daw/Snare_Top.wav")).unwrap();
    let r = svc.transport_play(&sid, Some(0.0), None, None).unwrap();
    assert_eq!(r["tracks"], 1, "a changed track list opens the transport again");
    svc.transport_close_project(&sid);
    assert_eq!(svc.transport_status()["open"], false);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn transport_play_ends_when_its_project_closes_and_not_for_another_project() {
    let (svc, sid, root) = opened("tp_close", &["Kick_In"]);
    let other_root = tmp("tp_close_other");
    let other_dir = project(&other_root, "Other", &[kick_spec("Kick_In", "kick", 36)]);
    let other = svc.project_open(other_dir.to_str().unwrap()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    svc.transport_play(&sid, Some(0.0), None, None).unwrap();
    svc.project_close(&other, true).unwrap();
    assert_eq!(
        svc.transport_status()["open"],
        true,
        "closing another project keeps the transport"
    );
    svc.project_close(&sid, true).unwrap();
    assert_eq!(
        svc.transport_status()["open"],
        false,
        "project_close releases its transport"
    );
    std::fs::remove_dir_all(root).unwrap();
    std::fs::remove_dir_all(other_root).unwrap();
}

#[test]
fn transport_play_follows_a_change_of_the_output_device() {
    let (svc, sid, root) = opened("tp_device", &["Kick_In", "Snare_Top"]);
    svc.transport_mix(&json!({"Snare_Top": true}), &Value::Null).unwrap();
    let key = |rate| DeviceKey {
        name: "null".into(),
        rate,
        channels: 2,
    };
    svc.set_probe_device(Some(key(48_000)), Some(Duration::from_millis(20)));
    let (p, log) = collector();
    let r = svc.transport_play(&sid, Some(0.5), None, Some(p)).unwrap();
    assert_eq!(r["rate"], 48_000);
    ms(300);
    assert_eq!(svc.transport_status()["reopens"], 0, "the same device does not reopen");
    let before = svc.transport_status()["pos"].as_f64().unwrap();
    // the user switches to a device at 44.1 kHz
    svc.set_probe_device(Some(key(44_100)), None);
    wait_for("the reopen", 5.0, || svc.transport_status()["reopens"] == 1);
    wait_for("playing again", 5.0, || svc.transport_status()["playing"] == true);
    let st = svc.transport_status();
    assert_eq!(st["rate"], 44_100);
    assert_eq!(st["device"]["rate"], 44_100);
    assert_eq!(st["requested"], true, "it was playing, it plays");
    assert_eq!(st["audible"], json!(["Kick_In"]), "mute survives");
    assert_eq!(st["master"], 0.0, "the master gain survives");
    let pos = st["pos"].as_f64().unwrap();
    assert!(
        pos >= before - 0.05 && pos < before + 2.0,
        "continues near {before}: {pos}"
    );
    ms(500);
    let after = svc.transport_status()["pos"].as_f64().unwrap();
    assert!(after > pos + 0.3, "plays on: {pos} to {after}");
    // the playhead messages go on, with a newer epoch and the new rate
    let last = log.lock().unwrap().last().unwrap().clone();
    assert_eq!(last["rate"], 44_100);
    assert_eq!(last["reopens"], 1);
    let _ = svc.transport_stop(true);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn transport_play_reports_a_lost_device_and_recovers() {
    let (svc, sid, root) = opened("tp_lost", &["Kick_In"]);
    let key = |rate| DeviceKey {
        name: "null".into(),
        rate,
        channels: 2,
    };
    svc.set_probe_device(Some(key(48_000)), Some(Duration::from_millis(20)));
    let (p, log) = collector();
    svc.transport_play(&sid, Some(0.0), None, Some(p)).unwrap();
    // the file goes away while the device changes: the reopen fails, the state says why
    let wav = root.join("daw/Kick_In.wav");
    let hidden = root.join("hidden.wav");
    std::fs::rename(&wav, &hidden).unwrap();
    svc.set_probe_device(Some(key(44_100)), None);
    wait_for("the lost state", 5.0, || svc.transport_status()["lost"].is_string());
    let st = svc.transport_status();
    assert_eq!(st["open"], false);
    // #204: a retry rewrites the `lost` text after one watcher period (20 ms here), so the
    // transient "output device changed: ..." is gone too fast to poll under load. The lasting
    // why is `last_reopen`: the failed reopen was for the device change 48000 -> 44100.
    let why = st["last_reopen"].as_str().unwrap_or_default();
    assert!(
        why.contains("48000") && why.contains("44100"),
        "the lost state says why it reopened: {st}"
    );
    wait_for("a message with the error", 2.0, || {
        log.lock()
            .unwrap()
            .iter()
            .any(|m| m["error"].is_string() && m["end"] == false)
    });
    // the file comes back: the next tick opens again at the same position, playing
    std::fs::rename(&hidden, &wav).unwrap();
    wait_for("the recovery", 5.0, || svc.transport_status()["open"] == true);
    let st = svc.transport_status();
    assert_eq!(st["lost"], Value::Null);
    assert_eq!(st["rate"], 44_100);
    assert_eq!(st["requested"], true);
    let _ = svc.transport_stop(true);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn transport_commands_list_matches_the_commands_of_the_service() {
    assert_eq!(TRANSPORT_COMMANDS.len(), 10);
    let svc = service_dsp();
    // every name is a method that the bridge can call; the unknown name is a 404
    let b = drumspike_app::bridge::Bridge::new(svc, repo_root().join("static"));
    for c in TRANSPORT_COMMANDS {
        let r = b.invoke(c, &json!({}));
        if let Err(e) = r {
            assert_ne!(e.code, 404, "{c} is not wired in the bridge");
        }
    }
    assert_eq!(b.invoke("transport_nope", &json!({})).unwrap_err().code, 404);
}

#[test]
fn transport_master_gain_zero_keeps_the_output_silent_and_the_mix_running() {
    let (svc, sid, root) = opened("tp_silent", &["Kick_In", "Snare_Top"]);
    let ev = json!({"Kick_In": [0.1, 0.3], "Snare_Top": [0.2]});
    svc.transport_play(&sid, Some(0.0), Some(&ev), None).unwrap();
    ms(800);
    let st = svc.transport_status();
    assert_eq!(st["master"], 0.0);
    assert!(
        st["peak"].as_f64().unwrap() > 0.1,
        "pre-gain peak shows that the mix and the clicks ran"
    );
    assert_eq!(st["underruns"], 0);
    let _ = svc.transport_stop(true);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn transport_play_runs_past_the_end_and_plays_again_from_the_start() {
    let (svc, sid, root) = opened("tp_end", &["Kick_In", "Snare_Top"]);
    let dur = 3.5;
    svc.transport_play(&sid, Some(dur - 0.3), None, None).unwrap();
    ms(900); // past the end of every file
    let st = svc.transport_status();
    assert_eq!(
        st["playing"], true,
        "the transport keeps its clock after the files end: {st}"
    );
    assert_eq!(st["underruns"], 0, "the end of a file is silence, not an underrun");
    let _ = svc.transport_stop(false);
    let (p, log) = collector();
    let r = svc.transport_play(&sid, Some(0.0), None, Some(p)).unwrap();
    assert_eq!(r["ready"], true, "{r}");
    ms(300);
    let pos = svc.transport_status()["pos"].as_f64().unwrap();
    assert!((0.1..0.7).contains(&pos), "{pos}");
    // no message shows the old position (the clock of a transport that was stopped at the end)
    let msgs = log.lock().unwrap().clone();
    assert!(!msgs.is_empty());
    assert!(msgs.iter().all(|m| m["pos"].as_f64().unwrap() < 1.0), "{msgs:?}");
    // the same for a seek while playing
    svc.transport_seek(3.0).unwrap();
    let msgs_before = log.lock().unwrap().len();
    ms(250);
    let after: Vec<f64> = log.lock().unwrap()[msgs_before..]
        .iter()
        .map(|m| m["pos"].as_f64().unwrap())
        .collect();
    assert!(
        !after.is_empty() && after.iter().all(|p| (2.99..3.6).contains(p)),
        "{after:?}"
    );
    let _ = svc.transport_stop(true);
    std::fs::remove_dir_all(root).unwrap();
}

/// Device run (arm64-mac, `-- --ignored device`): the default output at its own rate, master gain 0.0
/// (silent), a few seconds; prints the status. Changes no system setting.
#[test]
#[ignore = "needs the audio device"]
fn transport_device_plays_silently_on_the_default_output() {
    let root = tmp("tp_device_real");
    let dir = project(
        &root,
        "Song",
        &[kick_spec("Kick_In", "kick", 36), kick_spec("Snare_Top", "snare", 38)],
    );
    let svc = service_dsp();
    svc.set_output_select(Output::Default);
    svc.set_master_gain(0.0);
    let sid = svc.project_open(dir.to_str().unwrap()).unwrap()["sid"]
        .as_str()
        .unwrap()
        .to_string();
    let r = svc.transport_play(&sid, Some(0.0), None, None).unwrap();
    println!("PLAY {r}");
    ms(3_000);
    let st = svc.transport_status();
    println!("STATUS {st}");
    assert_eq!(st["master"], 0.0);
    assert_eq!(st["reopens"], 0, "{st}");
    assert!(st["callbacks"].as_u64().unwrap() > 100, "{st}");
    assert_eq!(st["underruns"], 0);
    let _ = svc.transport_stop(true);
    std::fs::remove_dir_all(root).unwrap();
}

// ---------- P3-16: volume, pan, master volume, meters, output device ----------

fn devices(names: &[&str]) -> Vec<DeviceKey> {
    names
        .iter()
        .map(|n| DeviceKey {
            name: n.to_string(),
            rate: 48_000,
            channels: 2,
        })
        .collect()
}

/// The meter messages of a progress log.
fn meter_msgs(log: &Log) -> Vec<Value> {
    log.lock()
        .unwrap()
        .iter()
        .filter(|m| m.get("meters").is_some())
        .cloned()
        .collect()
}

#[test]
fn track_set_mix_keeps_volume_and_pan_in_the_project_and_applies_them_live() {
    let (svc, sid, root) = opened("tp_setmix", &["Kick_In", "Snare_Top"]);
    // before any play: the project keeps it, nothing is applied
    let r = svc.track_set_mix(&sid, "Snare_Top", Some(-6.0), Some(0.5)).unwrap();
    assert_eq!(r, json!({"volume_db": -6.0, "pan": 0.5, "applied": false}));
    let r = svc.transport_play(&sid, Some(0.0), None, None).unwrap();
    assert_eq!(r["names"], json!(["Kick_In", "Snare_Top"]));
    let st = svc.transport_status();
    assert_eq!(
        st["vols"],
        json!([[0.0, 0.0], [-6.0, 0.5]]),
        "read from the project at the open"
    );
    // live: applied at once; a missing value stays; values are clamped
    let r = svc.track_set_mix(&sid, "Kick_In", Some(50.0), None).unwrap();
    assert_eq!(r, json!({"volume_db": 12.0, "pan": 0.0, "applied": true}));
    let r = svc.track_set_mix(&sid, "Kick_In", None, Some(-3.0)).unwrap();
    assert_eq!((r["volume_db"].clone(), r["pan"].clone()), (json!(12.0), json!(-1.0)));
    let r = svc.track_set_mix(&sid, "Kick_In", Some(-500.0), None).unwrap();
    assert_eq!(r["volume_db"], -120.0, "the bottom is off");
    assert_eq!(svc.transport_status()["vols"][0], json!([-120.0, -1.0]));
    // the document holds the fields and is dirty; a save and a new open read them back
    let t = svc.tracks_get(&sid).unwrap();
    assert_eq!(t["dirty"], true);
    assert_eq!(t["tracks"]["Kick_In"]["volume_db"], -120.0);
    assert_eq!(t["tracks"]["Snare_Top"]["pan"], 0.5);
    let _ = svc.transport_stop(true);
    // errors
    assert_eq!(
        svc.track_set_mix(&sid, "Nope", Some(0.0), None).unwrap_err().code,
        400,
        "unknown track"
    );
    assert_eq!(
        svc.track_set_mix("nope", "Kick_In", Some(0.0), None).unwrap_err().code,
        404
    );
    assert_eq!(
        svc.track_set_mix(&sid, "Kick_In", Some(f64::NAN), None)
            .unwrap_err()
            .code,
        400
    );
    // reset to 0 dB and the centre
    let r = svc.track_set_mix(&sid, "Kick_In", Some(0.0), Some(0.0)).unwrap();
    assert_eq!((r["volume_db"].clone(), r["pan"].clone()), (json!(0.0), json!(0.0)));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn transport_play_sends_the_meters_at_about_25_hz_while_playing() {
    // #239: a 9 s clip. The window below runs on the wall clock while the playhead keeps
    // the device time: a throttled test thread pushes the mix change later without slowing
    // the playhead, and with the 3.5 s clip (last audible sample ~3.3 s) the whole
    // post-change window then sits in the clip's silence - every level reads 0 and the
    // mono-level check fails without a regression (2026-10-23 sanity run on this host,
    // results/239v2-sanity-meters-1.txt, finished in 9.76 s against ~7.7 s nominal). The
    // 9 s clip keeps hits on the 0.5 s grid up to ~8.8 s, so the window holds hits even
    // when the change lands seconds late; a real regression still zeroes every level.
    let (svc, sid, root) = opened_dur("tp_meters", &["Kick_In", "Snare_Top"], 9.0);
    let (p, log) = collector_all();
    svc.transport_play(&sid, Some(0.0), None, Some(p)).unwrap();
    // the rate is measured from the first meter message on, so the start-up of the audio
    // thread on a loaded runner does not count (CI run 38358505253: 18 in 1.01 s from play)
    wait_for("the first meter message", 5.0, || !meter_msgs(&log).is_empty());
    // P4-40d-t + #239: the ceiling for the sender is not a point sample of a bare sleep.
    // Under a bursty cargo load all three 15-sleep medians landed at 52-64 ms while the
    // sender's sustained rate over the window was 7-8 Hz (the 20x20 on this host, loaded
    // meters #8: a 9.5 Hz bound against 7.16 Hz achieved). The sender's loop is sleep +
    // host lock + log push, so the probe below runs that shape over the window: it sleeps
    // PLAYHEAD_POLL, takes the log lock and drops it again (no push, the log stays clean),
    // and counts its wakes. Its sustained rate over the same span is the environment's
    // ceiling for the sender; the 0.6 bound on it still fails a halved sender (0.5 < 0.6)
    // at every scheduling class.
    let probe_stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let probe_wakes = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (plog, pstop, pwakes) = (log.clone(), probe_stop.clone(), probe_wakes.clone());
    let probe_start = Instant::now();
    std::thread::Builder::new()
        .name("meters-probe".into())
        .spawn(move || {
            while !pstop.load(std::sync::atomic::Ordering::Relaxed) {
                std::thread::sleep(PLAYHEAD_POLL);
                // the sender's own push path: sleep, then the log lock (no push here)
                let _g = plog.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                pwakes.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        })
        .unwrap();
    // P4-40d-t: the play is long enough that the meter window below holds about 3 s: at a low
    // look rate only ~5 messages land in a 1.6 s window, and one message of quantization is
    // then the size of the halving margin, so the bound would flip on the count.
    ms(2_400);
    let m = meter_msgs(&log);
    // the null device with master gain 0 still gives meter values: both tracks and the master
    let peak = |i: usize| m.iter().map(|x| x["meters"][i].as_f64().unwrap()).fold(0.0, f64::max);
    assert!(peak(0) > 0.01 && peak(2) > 0.01, "{:?}", m.last());
    let mpeak = m.iter().map(|x| x["master"][0].as_f64().unwrap()).fold(0.0, f64::max);
    assert!(mpeak > 0.01, "master meter before the master gain: {:?}", m.last());
    assert_eq!(m[0]["sid"], sid);
    assert_eq!(m[0]["meters"].as_array().unwrap().len(), 4, "left and right per track");
    // #471: each meter message carries the playhead fields of its look (the page's clock)
    for x in &m {
        assert!(x["pos"].is_f64() && x["t_ns"].is_u64() && x["epoch"].is_u64(), "{x:?}");
        assert!(
            x["frame"].is_u64() && x["rate"].is_u64() && x["speed"].is_f64(),
            "{x:?}"
        );
    }
    // a track at -inf meters 0, the other goes on; pan hard left leaves its right side at 0
    svc.track_set_mix(&sid, "Kick_In", Some(-120.0), None).unwrap();
    svc.track_set_mix(&sid, "Snare_Top", None, Some(-1.0)).unwrap();
    ms(150);
    let n0 = meter_msgs(&log).len();
    ms(500);
    let all = log.lock().unwrap().clone();
    // stop the probe and rate its own sustained wake cadence over the same span (the
    // window's data is complete in `all`)
    probe_stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let probe_hz = probe_wakes.load(std::sync::atomic::Ordering::Relaxed) as f64 / probe_start.elapsed().as_secs_f64();
    // the stale check below is about the block thread's lag at the mix change, the end of
    // the window: it keeps the end measure.
    let look_end = look_secs();
    let late: Vec<Value> = all
        .iter()
        .filter(|x| x.get("meters").is_some())
        .skip(n0)
        .cloned()
        .collect();
    assert!(!late.is_empty());
    // the mix change applies live. At a throttled look the null device's block thread can lag
    // one send behind the change, so ONE stale snapshot right after it is allowed while
    // look >= 60 ms; strict at an unthrottled look (review 6227692703).
    let stale: Vec<usize> = late
        .iter()
        .enumerate()
        .filter(|(_, x)| !(x["meters"][0] == 0.0 && x["meters"][1] == 0.0))
        .map(|(i, _)| i)
        .collect();
    let allowed = if look_end >= 0.060 { 1 } else { 0 };
    assert!(
        stale.len() <= allowed && stale.iter().all(|i| *i < allowed),
        "{} stale meter messages after the mix change (allowed {allowed}, look {:.0} ms): {late:?}",
        stale.len(),
        look_end * 1000.0
    );
    // a mono track meters its level after the volume and before the pan: both values are equal
    assert!(
        late.iter().all(|x| x["meters"][2] == x["meters"][3]),
        "a mono track: one level"
    );
    assert!(late.iter().any(|x| x["meters"][2].as_f64().unwrap() > 0.01));
    // P4-40d-t: the meter rate over the playhead's own clock. Meters and playhead messages
    // share one log in send order; the counted meters are bracketed by two playhead messages
    // and the window is their t_ns span, so the sleeps of a throttled test thread do not
    // stretch or shrink the window. At the lowered scheduling class of the runner every look
    // of the sender thread can take ~150 ms (at the utility class: 7 meter messages in 1.14 s
    // of test time), so the meters fall with the look rate; the lower bound still fails when
    // the meter thread stops or halves its sends. The window spans the whole play so far
    // (about 3 s, see above) to keep the estimate steady at a low look rate; it ends before
    // the pause.
    let t_ns = |x: &Value| x["t_ns"].as_u64().unwrap();
    let first_meter = all
        .iter()
        .position(|x| x.get("meters").is_some())
        .expect("no meter message");
    let (p0, t0) = all[first_meter..]
        .iter()
        .enumerate()
        .find(|(_, x)| x.get("meters").is_none())
        .map(|(i, x)| (first_meter + i, t_ns(x)))
        .expect("no playhead message after the first meter");
    let last_playhead = all.iter().rposition(|x| x.get("meters").is_none()).unwrap();
    let count = all[p0 + 1..last_playhead]
        .iter()
        .filter(|x| x.get("meters").is_some())
        .count();
    let secs = (t_ns(&all[last_playhead]) - t0) as f64 / 1e9;
    assert!(
        secs >= 1.5,
        "only {secs:.2} s of the playhead's own clock in the window"
    );
    let rate = count as f64 / secs;
    // Calibrated by the environment's ceiling for the sender's loop shape (the probe), not
    // by the product's own messages: a self-normalizing bound hides a halving. Normal: the
    // probe sustains far above 25 Hz, expected caps at 25, the low bound is 15 Hz and a
    // halving (12.5 Hz) fails; throttled (probe ~7 Hz under a gate load): the low bound is
    // ~4.2 Hz and a halving (3.5 Hz) still fails.
    let expected = 25.0_f64.min(probe_hz);
    // #239: one line per run, the 20x20 measurement reads it
    eprintln!(
        "METERS_RATE_HZ={rate:.2} COUNT={count} SECS={secs:.3} PROBE_HZ={probe_hz:.2} LOW_BOUND_HZ={:.1}",
        0.6 * expected
    );
    assert!(
        rate >= 0.6 * expected && rate <= 32.0,
        "{count} meter messages over {secs:.2} s of the playhead's own clock ({rate:.1} Hz); probe {:.2} Hz, low bound {:.1} Hz",
        probe_hz,
        0.6 * expected
    );
    // no meter messages after the pause
    let _ = svc.transport_stop(false);
    ms(100);
    let n1 = meter_msgs(&log).len();
    ms(300);
    assert_eq!(meter_msgs(&log).len(), n1, "no meters while paused");
    let _ = svc.transport_stop(true);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn transport_master_sets_the_master_volume_and_keeps_it_in_the_app_settings() {
    let (svc, sid, root) = opened("tp_master", &["Kick_In"]);
    let home = root.join("home");
    svc.set_home(&home);
    assert_eq!(svc.transport_status()["master_db"], 0.0);
    let r = svc.transport_master(-6.0).unwrap();
    assert_eq!(r, json!({"volume_db": -6.0, "applied": false}));
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(home.join("settings.json")).unwrap()).unwrap();
    assert_eq!(saved["master_db"], -6.0);
    assert_eq!(saved["audio_output"], Value::Null);
    // live: the master meter falls to 0 at -inf while the track meter goes on
    let (p, log) = collector_all();
    let r = svc.transport_play(&sid, Some(0.0), None, Some(p)).unwrap();
    assert_eq!(r["master_db"], -6.0);
    assert_eq!(
        svc.transport_master(-500.0).unwrap(),
        json!({"volume_db": -120.0, "applied": true})
    );
    ms(150);
    let n0 = meter_msgs(&log).len();
    ms(400);
    let late: Vec<Value> = meter_msgs(&log)[n0..].to_vec();
    assert!(late.iter().all(|x| x["master"] == json!([0.0, 0.0])), "{late:?}");
    assert!(late.iter().any(|x| x["meters"][0].as_f64().unwrap() > 0.01));
    assert_eq!(svc.transport_master(f64::NAN).unwrap_err().code, 400);
    svc.transport_master(3.0).unwrap();
    let _ = svc.transport_stop(true);
    // a new service with the same home reads it back
    let svc2 = service_dsp();
    svc2.set_home(&home);
    assert_eq!(svc2.transport_status()["master_db"], 3.0);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn outputs_list_lists_the_devices_the_choice_and_the_current_device() {
    let (svc, sid, root) = opened("tp_devices", &["Kick_In"]);
    let d = svc.outputs_list();
    assert_eq!(d["devices"], json!([]));
    assert_eq!(d["null_output"], true);
    assert_eq!(d["choice"], Value::Null);
    assert_eq!(d["current"], Value::Null);
    svc.set_device_list(Some(devices(&["Speakers", "AirPods"])));
    let d = svc.outputs_list();
    assert_eq!(
        d["devices"][0],
        json!({"name": "Speakers", "rate": 48_000, "channels": 2, "default": true})
    );
    assert_eq!(d["devices"][1]["default"], false);
    svc.transport_play(&sid, Some(0.0), None, None).unwrap();
    let d = svc.outputs_list();
    assert_eq!(d["current"]["name"], "null");
    assert!(d["current"]["rate"].as_u64().unwrap() > 0);
    let _ = svc.transport_stop(true);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn output_select_keeps_the_choice_and_falls_back_to_the_system_default_with_a_message() {
    let (svc, sid, root) = opened("tp_output", &["Kick_In"]);
    let home = root.join("home");
    svc.set_home(&home);
    svc.set_device_list(Some(devices(&["Speakers", "AirPods"])));
    let d = svc.output_select(Some("AirPods")).unwrap();
    assert_eq!(d["choice"], "AirPods");
    assert_eq!(d["note"], Value::Null);
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(home.join("settings.json")).unwrap()).unwrap();
    assert_eq!(saved["audio_output"], "AirPods");
    // the saved device is gone: the system default plays, and the message says so
    svc.set_device_list(Some(devices(&["Speakers"])));
    svc.transport_play(&sid, Some(0.0), None, None).unwrap();
    let st = svc.transport_status();
    assert_eq!(st["choice"], "AirPods");
    let note = st["note"].as_str().unwrap();
    assert!(
        note.contains("\"AirPods\" is not connected") && note.contains("system default"),
        "{note}"
    );
    assert_eq!(st["requested"], true, "it plays");
    // a choice while playing reopens at the same position and state
    let reopens = st["reopens"].as_u64().unwrap();
    let d = svc.output_select(None).unwrap();
    assert_eq!(d["choice"], Value::Null);
    assert_eq!(d["note"], Value::Null);
    let st = svc.transport_status();
    assert_eq!(st["reopens"].as_u64().unwrap(), reopens + 1);
    assert_eq!(st["requested"], true);
    let _ = svc.transport_stop(true);
    assert_eq!(svc.output_select(Some(" ")).unwrap_err().code, 400);
    // a new service reads the choice and gives the message before any play
    svc.output_select(Some("Studio")).unwrap();
    let svc2 = service_dsp();
    svc2.set_output_select(null());
    svc2.set_home(&home);
    svc2.set_device_list(Some(devices(&["Speakers"])));
    assert_eq!(svc2.outputs_list()["choice"], "Studio");
    assert!(
        svc2.output_select(Some("Studio")).unwrap()["note"]
            .as_str()
            .unwrap()
            .contains("Studio")
    );
    std::fs::remove_dir_all(root).unwrap();
}

// ---------- P3-20b: half speed ----------

/// The session seconds per second of the transport over `ms` milliseconds (status `pos` and `t_ns`).
fn rate_over(svc: &Service, ms_: u64) -> f64 {
    let a = svc.transport_status();
    ms(ms_);
    let b = svc.transport_status();
    let dp = b["pos"].as_f64().unwrap() - a["pos"].as_f64().unwrap();
    let dt = (b["t_ns"].as_u64().unwrap() - a["t_ns"].as_u64().unwrap()) as f64 / 1e9;
    dp / dt
}

#[test]
fn transport_play_at_half_speed_runs_the_playhead_at_half_the_rate_and_back() {
    let (svc, sid, root) = opened("tp_half", &["Kick_In", "Snare_Top"]);
    let (p, log) = collector();
    // the first play opens the transport at half speed
    let r = svc
        .transport_play_at(&sid, Some(0.5), None, Some(p), Some(0.5))
        .unwrap();
    assert_eq!(
        (r["ready"].clone(), r["speed"].clone()),
        (json!(true), json!(0.5)),
        "{r}"
    );
    assert!(r["heard_ms"].as_f64().unwrap() >= 0.0, "{r}");
    assert!(
        (r["duration"].as_f64().unwrap() - 3.5).abs() < 0.01,
        "the session length is the same"
    );
    let st = svc.transport_status();
    assert_eq!((st["speed"].clone(), st["playing"].clone()), (json!(0.5), json!(true)));
    let half = rate_over(&svc, 600);
    assert!((half - 0.5).abs() < 0.06, "half speed: {half} s per s");
    // the playhead messages carry the speed; the page extrapolates with it
    let msgs = log.lock().unwrap().clone();
    assert!(
        msgs.iter()
            .filter(|m| m["playing"] == true)
            .all(|m| m["speed"] == json!(0.5)),
        "{msgs:?}"
    );
    // a seek keeps the speed
    svc.transport_seek(1.0).unwrap();
    wait_for("playing after the seek", 5.0, || {
        svc.transport_status()["playing"] == true
    });
    assert_eq!(svc.transport_status()["speed"], 0.5);
    // a stop holds the position; a play without speed is normal speed in the same transport
    let stop = svc.transport_stop(false);
    let at = stop["pos"].as_f64().unwrap();
    let (p2, log2) = collector();
    let r = svc.transport_play(&sid, None, None, Some(p2)).unwrap();
    assert_eq!(
        (r["ready"].clone(), r["speed"].clone()),
        (json!(true), json!(1.0)),
        "{r}"
    );
    assert_eq!(svc.transport_status()["reopens"], 0);
    let normal = rate_over(&svc, 600);
    assert!((normal - 1.0).abs() < 0.08, "normal speed: {normal}");
    let first = log2
        .lock()
        .unwrap()
        .iter()
        .find(|m| m["playing"] == true)
        .cloned()
        .unwrap();
    assert_eq!(first["speed"], 1.0);
    let p0 = first["pos"].as_f64().unwrap();
    assert!(
        p0 >= at - 0.01 && p0 < at + 0.3,
        "normal speed from where half speed stopped: {at} -> {p0}"
    );
    // and to half speed again from the running transport, without a position: from where it is
    let r = svc.transport_play_at(&sid, None, None, None, Some(0.5)).unwrap();
    assert_eq!(r["speed"], 0.5);
    assert_eq!(svc.transport_status()["speed"], 0.5);
    // a wrong speed is a 400
    let e = svc
        .transport_play_at(&sid, Some(0.0), None, None, Some(2.0))
        .unwrap_err();
    assert_eq!(e.code, 400, "{}", e.message);
    let _ = svc.transport_stop(true);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn transport_play_at_half_speed_keeps_the_speed_after_a_device_change() {
    let (svc, sid, root) = opened("tp_half_device", &["Kick_In"]);
    let key = |rate| DeviceKey {
        name: "null".into(),
        rate,
        channels: 2,
    };
    svc.set_probe_device(Some(key(48_000)), Some(Duration::from_millis(20)));
    svc.transport_play_at(&sid, Some(0.2), None, None, Some(0.5)).unwrap();
    svc.set_probe_device(Some(key(44_100)), None);
    wait_for("the reopen", 5.0, || svc.transport_status()["reopens"] == 1);
    wait_for("playing again", 5.0, || svc.transport_status()["playing"] == true);
    let st = svc.transport_status();
    assert_eq!(
        (st["rate"].clone(), st["speed"].clone()),
        (json!(44_100), json!(0.5)),
        "{st}"
    );
    let half = rate_over(&svc, 600);
    assert!((half - 0.5).abs() < 0.06, "half speed on the new device: {half}");
    let _ = svc.transport_stop(true);
    std::fs::remove_dir_all(root).unwrap();
}

/// P3-20b measurement (arm64-mac, `-- --ignored latency --nocapture`): the start latency of half speed
/// in the two designs of the card on the 30-track session of `make_session.py`
/// (`DRUMSPIKE_TRACKS_BENCH=<folder>`). Silent: the master gain is 0. The default output, or the null
/// device with `DRUMSPIKE_LATENCY_OUTPUT=null`. Each round starts from an open transport, paused after a
/// normal-speed play: (a) the speed change in the running stream (`transport_play_at` with speed 0.5),
/// (b) a close and an open at half speed (`transport_stop(close)`, then the same call). The numbers are
/// the call to the first playing block (`start_ms`) and to the moment its first sample is heard
/// (`heard_ms`, the block's playback time); for (b) the close is added.
#[test]
#[ignore = "latency: needs DRUMSPIKE_TRACKS_BENCH=<make_session.py folder> and the audio device"]
fn transport_play_half_speed_start_latency_of_both_designs() {
    let Some(folder) = std::env::var_os("DRUMSPIKE_TRACKS_BENCH").map(std::path::PathBuf::from) else {
        panic!("DRUMSPIKE_TRACKS_BENCH is not set");
    };
    let rounds = env_n("DRUMSPIKE_LATENCY_ROUNDS", 5);
    let root = tmp("tp_half_latency");
    let svc = service_dsp();
    svc.set_home(root.join("home"));
    svc.set_projects_dir(root.join("projects"));
    let null_out = std::env::var("DRUMSPIKE_LATENCY_OUTPUT").is_ok_and(|v| v == "null");
    svc.set_output_select(if null_out { null() } else { Output::Default });
    svc.set_master_gain(0.0);
    let r = svc.project_new(Some("Latency"), None, None).unwrap();
    let sid = r["sid"].as_str().unwrap().to_string();
    let add = svc
        .tracks_add(&sid, &[folder.display().to_string()], &[], &[], false, None)
        .unwrap();
    assert_eq!(add["added"].as_array().unwrap().len(), 30, "{add}");
    let st0 = svc.transport_play(&sid, Some(10.0), None, None).unwrap();
    println!(
        "LATENCY device {} rate {} tracks {} open_ms {:.1} start_ms {:.1}",
        st0["device"]["name"],
        st0["rate"],
        st0["tracks"],
        st0["open_ms"].as_f64().unwrap(),
        st0["start_ms"].as_f64().unwrap()
    );
    assert_eq!(st0["master"], 0.0, "silent");
    ms(500);
    let _ = svc.transport_stop(false);
    let (mut field, mut reopen) = (Vec::new(), Vec::new());
    for k in 0..rounds {
        let pos = 20.0 + 37.0 * k as f64;
        for design in ["field", "reopen"] {
            // the same start state for both: open, paused after a normal-speed play
            svc.transport_play(&sid, Some(pos - 5.0), None, None).unwrap();
            ms(300);
            let _ = svc.transport_stop(false);
            ms(200);
            let t = Instant::now();
            if design == "reopen" {
                let _ = svc.transport_stop(true);
            }
            let close_ms = t.elapsed().as_secs_f64() * 1e3;
            let r = svc.transport_play_at(&sid, Some(pos), None, None, Some(0.5)).unwrap();
            let total = t.elapsed().as_secs_f64() * 1e3;
            assert_eq!(
                (r["ready"].clone(), r["speed"].clone()),
                (json!(true), json!(0.5)),
                "{r}"
            );
            let start = r["start_ms"].as_f64().unwrap() + close_ms;
            let heard = r["heard_ms"].as_f64().map(|h| h + close_ms);
            let s = svc.transport_status();
            println!(
                "LATENCY round {k} {design}: start {start:.1} ms, heard {heard:?} ms, call {total:.1} ms, close {close_ms:.1} ms, underruns {}",
                s["underruns"]
            );
            if design == "field" {
                field.push((start, heard.unwrap_or(start)));
            } else {
                reopen.push((start, heard.unwrap_or(start)));
            }
            ms(300);
            let _ = svc.transport_stop(false);
        }
    }
    let med = |v: &Vec<(f64, f64)>, i: usize| {
        let mut x: Vec<f64> = v.iter().map(|p| if i == 0 { p.0 } else { p.1 }).collect();
        x.sort_by(f64::total_cmp);
        (x[x.len() / 2], x[x.len() - 1])
    };
    println!(
        "LATENCY field: start median {:.1} max {:.1} ms, heard median {:.1} max {:.1} ms",
        med(&field, 0).0,
        med(&field, 0).1,
        med(&field, 1).0,
        med(&field, 1).1
    );
    println!(
        "LATENCY reopen: start median {:.1} max {:.1} ms, heard median {:.1} max {:.1} ms",
        med(&reopen, 0).0,
        med(&reopen, 0).1,
        med(&reopen, 1).0,
        med(&reopen, 1).1
    );
    let _ = svc.transport_stop(true);
    std::fs::remove_dir_all(root).unwrap();
}

// ---- P4-40d: the audition (a play with start, end, loop and tracks) ----

/// The paused position after a window play ends, in seconds, once `playing` is false.
fn wait_stopped(svc: &Service, secs: f64) -> f64 {
    wait_for("the audition to end", secs, || {
        svc.transport_status()["requested"] == false
    });
    ms(60);
    svc.transport_status()["pos"].as_f64().unwrap()
}

fn window(start: f64, end: Option<f64>, looped: bool, tracks: &[&str]) -> Option<drumspike_app::transport::PlayWindow> {
    Some(drumspike_app::transport::PlayWindow {
        start: Some(start),
        end,
        looped,
        tracks: tracks.iter().map(std::string::ToString::to_string).collect(),
        shot: None,
    })
}

#[test]
fn transport_play_window_plays_one_track_to_its_end_and_leaves_mute_and_solo_alone() {
    let (svc, sid, root) = opened("tp_aud", &["Kick_In", "Snare_Top", "Tom_1"]);
    // the page's state: Snare muted, Tom soloed
    svc.transport_mix(&json!({"Snare_Top": true}), &json!({"Tom_1": true}))
        .unwrap();
    let r = svc
        .transport_play_window(
            &sid,
            None,
            None,
            None,
            None,
            window(0.5, Some(0.78), false, &["Kick_In"]),
        )
        .unwrap();
    assert_eq!(
        r["audition"],
        json!({"tracks": ["Kick_In"], "start": 0.5, "end": 0.78, "loop": false})
    );
    let st = svc.transport_status();
    assert_eq!(
        st["audible"],
        json!(["Kick_In"]),
        "the audition track alone, whatever solo says"
    );
    assert_eq!(st["audition"]["tracks"], json!(["Kick_In"]));
    let pos = wait_stopped(&svc, 3.0);
    // it stopped at the end of the window, not at the end of the file. The watcher is exact to
    // one block and one poll, and at a lowered class the render thread lags the watcher's
    // extrapolation by its own sleep: 1408 frames (29 ms) measured at the utility class
    // (P4-40d-t), so the last rendered block may begin up to 0.1 s of session below the end.
    assert!((0.78 - 0.1..0.9).contains(&pos), "stopped at {pos}");
    let st = svc.transport_status();
    assert_eq!(st["audition"], Value::Null);
    assert_eq!(st["audible"], json!(["Tom_1"]), "mute and solo are the page's again");
    // a play of the page (no window) uses the page's mix as well
    svc.transport_play(&sid, Some(0.0), None, None).unwrap();
    assert_eq!(svc.transport_status()["audible"], json!(["Tom_1"]));
    let _ = svc.transport_stop(true);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn transport_play_window_previews_the_layer_one_shot_once() {
    // P4-40e (the Layer view's Play): `shot` adds one voice at the window start — the one-shot once
    // through the track's fader — and solos the track. The hits sit at 0.25 s and later, outside
    // the 0.2 s window, so the only voice that can start is the preview. Planted fault: the same
    // window without `shot` starts no voice at all.
    use drumspike_core::io::{WavFormat, WavSpec, write_wav as write_core};
    let (svc, sid, root) = opened("tp_shot", &["Kick_In", "Snare_Top"]);
    let f = root.join("shots").join("shot.wav");
    let sr = 48_000u32;
    let n = (0.2 * sr as f64) as usize;
    let mut x = vec![0.0f32; n];
    x[480..].fill(0.8); // the attack at 480: the measured onset
    std::fs::create_dir_all(f.parent().unwrap()).unwrap();
    let spec = WavSpec {
        sample_rate: sr,
        channels: 1,
        format: WavFormat::Float32,
        extensible: false,
        channel_mask: 0,
    };
    write_core(&f, &spec, &x, None).unwrap();
    svc.track_layer_set(&sid, "Kick_In", &json!({"path": f.to_str().unwrap()}))
        .unwrap();

    // the counter exists while the transport is open; it starts at 0 on this play's fresh open
    let count = || {
        svc.transport_status()
            .get("layer_voices")
            .and_then(Value::as_u64)
            .unwrap_or(0)
    };
    let win = |shot: Option<&str>| {
        Some(drumspike_app::transport::PlayWindow {
            start: Some(0.0),
            // the same 0.2 s window; with a shot the service derives it from the one-shot
            end: shot.is_none().then_some(0.2),
            looped: false,
            tracks: vec![],
            shot: shot.map(str::to_string),
        })
    };
    let r = svc
        .transport_play_window(&sid, None, None, None, None, win(Some("Kick_In")))
        .unwrap();
    assert_eq!(r["audition"]["shot"], json!("Kick_In"));
    assert_eq!(
        r["audition"]["tracks"],
        json!(["Kick_In"]),
        "the shot track alone is audible"
    );
    assert!(
        (r["audition"]["end"].as_f64().unwrap() - 0.2).abs() < 1e-9,
        "the window is the one-shot's length"
    );
    assert_eq!(svc.transport_status()["audible"], json!(["Kick_In"]));
    wait_stopped(&svc, 3.0);
    assert_eq!(count(), 1, "the preview voice, and no hit voice in the window");

    // planted fault: the same window without the shot starts no voice (no hit before 0.2 s)
    let r = svc
        .transport_play_window(&sid, None, None, None, None, win(None))
        .unwrap();
    assert!(
        r["audition"].get("shot").is_none(),
        "the plain audition answer has no shot"
    );
    wait_stopped(&svc, 3.0);
    assert_eq!(count(), 1, "no voice without the shot");
    // a track without a layer is a 409, before the transport opens
    assert_eq!(
        svc.transport_play_window(&sid, None, None, None, None, win(Some("Snare_Top")))
            .unwrap_err()
            .code,
        409
    );
    let _ = svc.transport_stop(true);
    std::fs::remove_dir_all(root).unwrap();
}

/// #166: a looped audition wraps in the engine: the playhead jumps back at the region end
/// while no seek is taken per pass (the seek tag stays at the play's) and the transport keeps
/// playing. A stop gives the mix back.
#[test]
fn transport_play_window_loop_wraps_in_the_engine_without_a_seek_per_pass() {
    let (svc, sid, root) = opened("tp_loop_engine", &["Kick_In", "Snare_Top"]);
    svc.transport_play_window(
        &sid,
        None,
        None,
        None,
        None,
        window(0.5, Some(0.65), true, &["Kick_In"]), // a 150 ms window: ~6 passes in 0.9 s
    )
    .unwrap();
    let mut jumps = 0;
    let mut last = 1.0f64;
    let mut seek_tags = Vec::new();
    let end = Instant::now() + Duration::from_millis(900);
    while Instant::now() < end {
        let st = svc.transport_status();
        let pos = st["pos"].as_f64().unwrap();
        assert!((0.49..0.75).contains(&pos), "the loop left its window: {pos}");
        if pos < last - 0.05 {
            jumps += 1;
        }
        last = pos;
        assert_eq!(st["requested"], true, "still looping");
        seek_tags.push(st["seek_tag"].as_u64().unwrap());
        ms(4);
    }
    assert!(jumps >= 2, "{jumps} wraps in 0.9 s");
    let first = seek_tags[0];
    assert!(
        seek_tags.iter().all(|t| *t == first),
        "no seek per pass: the seek tag stayed at {first}, saw {seek_tags:?}"
    );
    let _ = svc.transport_stop(false);
    let st = svc.transport_status();
    assert_eq!(st["audition"], Value::Null);
    assert_eq!(st["audible"], json!(["Kick_In", "Snare_Top"]));
    let _ = svc.transport_stop(true);
    std::fs::remove_dir_all(root).unwrap();
}

/// #166: a loop that a past audition armed cannot wrap a plain play without a position (that
/// play sends no seek): the service ends the loop with one seek to where the transport is.
#[test]
fn a_plain_play_without_a_position_ends_an_armed_loop() {
    let (svc, sid, root) = opened("tp_loop_clear", &["Kick_In", "Snare_Top"]);
    svc.transport_play_window(
        &sid,
        None,
        None,
        None,
        None,
        window(0.5, Some(0.65), true, &["Kick_In"]),
    )
    .unwrap();
    ms(60);
    assert_eq!(svc.transport_status()["requested"], true);
    // the plain play replaces the looped audition and sends no position: no seek of its own
    svc.transport_play(&sid, None, None, None).unwrap();
    ms(400);
    let st = svc.transport_status();
    assert_eq!(st["requested"], true, "still playing");
    let pos = st["pos"].as_f64().unwrap();
    assert!(pos > 0.65, "the play ran past the old region end (no wrap): {pos}");
    assert_eq!(st["audition"], Value::Null);
    let _ = svc.transport_stop(true);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn transport_play_window_with_loop_repeats_until_stop_and_a_stop_gives_the_mix_back() {
    let (svc, sid, root) = opened("tp_aud_loop", &["Kick_In", "Snare_Top"]);
    svc.transport_play_window(
        &sid,
        None,
        None,
        None,
        None,
        window(0.5, Some(0.7), true, &["Snare_Top"]),
    )
    .unwrap();
    // 1.2 s is six windows: the playhead never leaves the window by much and the transport still plays
    let mut jumps = 0;
    let mut last = 0.0;
    let end = Instant::now() + Duration::from_millis(1200);
    while Instant::now() < end {
        let st = svc.transport_status();
        let pos = st["pos"].as_f64().unwrap();
        assert!((0.49..0.9).contains(&pos), "the loop left its window: {pos}");
        if pos < last - 0.05 {
            jumps += 1;
        }
        last = pos;
        assert_eq!(st["requested"], true, "still looping");
        ms(5);
    }
    assert!(jumps >= 2, "{jumps} jumps back to the start in 1.2 s");
    let _ = svc.transport_stop(false);
    let st = svc.transport_status();
    assert_eq!(st["audition"], Value::Null);
    assert_eq!(st["audible"], json!(["Kick_In", "Snare_Top"]));
    // the loop thread is gone: the position does not move after the stop
    let p0 = svc.transport_status()["pos"].as_f64().unwrap();
    ms(300);
    assert_eq!(svc.transport_status()["pos"].as_f64().unwrap(), p0);
    let _ = svc.transport_stop(true);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn transport_play_window_without_progress_sends_nothing_and_a_new_play_replaces_the_audition() {
    let (svc, sid, root) = opened("tp_aud_prog", &["Kick_In", "Snare_Top"]);
    let (p, log) = collector_all();
    svc.transport_play(&sid, Some(0.0), None, Some(p.clone())).unwrap();
    ms(150);
    // even a channel that is given gets nothing from an audition
    svc.transport_play_window(
        &sid,
        None,
        None,
        Some(p),
        None,
        window(0.5, Some(0.6), false, &["Kick_In"]),
    )
    .unwrap();
    ms(100);
    let n = log.lock().unwrap().len();
    ms(400);
    assert_eq!(
        log.lock().unwrap().len(),
        n,
        "the page's channel gets nothing during an audition"
    );
    // an audition with a long window, replaced by a plain play: the audition is gone and the end does not pause the play
    svc.transport_play_window(
        &sid,
        None,
        None,
        None,
        None,
        window(0.5, Some(1.0), false, &["Kick_In"]),
    )
    .unwrap();
    svc.transport_play(&sid, Some(0.0), None, None).unwrap();
    ms(700);
    let st = svc.transport_status();
    assert_eq!(st["audition"], Value::Null);
    assert_eq!(
        st["requested"], true,
        "the old audition thread did not pause the new play"
    );
    assert_eq!(st["audible"], json!(["Kick_In", "Snare_Top"]));
    // a second audition replaces the first: the thread of the first window does not pause the second
    svc.transport_play_window(
        &sid,
        None,
        None,
        None,
        None,
        window(0.5, Some(0.6), false, &["Kick_In"]),
    )
    .unwrap();
    svc.transport_play_window(
        &sid,
        None,
        None,
        None,
        None,
        window(0.5, Some(1.2), false, &["Snare_Top"]),
    )
    .unwrap();
    ms(350);
    let st = svc.transport_status();
    assert_eq!(
        st["requested"], true,
        "the first audition's end did not stop the second"
    );
    assert_eq!(st["audible"], json!(["Snare_Top"]));
    let _ = svc.transport_stop(true);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn transport_play_window_at_half_speed_ends_at_the_window_end_in_session_time() {
    let (svc, sid, root) = opened("tp_aud_half", &["Kick_In", "Snare_Top"]);
    let t = Instant::now();
    svc.transport_play_window(
        &sid,
        None,
        None,
        None,
        Some(0.5),
        window(0.5, Some(0.7), false, &["Kick_In"]),
    )
    .unwrap();
    let pos = wait_stopped(&svc, 3.0);
    let took = t.elapsed().as_secs_f64();
    // the last rendered block may begin below the end: the watcher is exact to one block and
    // one poll, and at a lowered class the render thread lags by its own sleep — 640 frames
    // (13 ms of session, about 27 ms of wall at half speed) measured at the utility class
    // (P4-40d-t). Allowed: up to 0.05 s of session (0.1 s of wall) below the end.
    assert!((0.7 - 0.05..0.8).contains(&pos), "{pos}");
    assert!(took > 0.35, "0.2 s of session at half speed takes 0.4 s, took {took}");
    let _ = svc.transport_stop(true);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn transport_play_window_refuses_a_wrong_window() {
    let (svc, sid, root) = opened("tp_aud_bad", &["Kick_In", "Snare_Top"]);
    let play = |w| {
        svc.transport_play_window(&sid, None, None, None, None, w)
            .unwrap_err()
            .code
    };
    assert_eq!(
        play(window(0.5, Some(0.5), false, &["Kick_In"])),
        400,
        "end after start"
    );
    assert_eq!(play(window(0.5, None, true, &["Kick_In"])), 400, "loop needs an end");
    assert_eq!(play(window(0.5, Some(0.8), false, &["Nope"])), 400, "unknown track");
    assert_eq!(play(window(-1.0, Some(0.8), false, &["Kick_In"])), 400);
    // the arguments of the command: nothing given is no window; a wrong list is a 400
    use drumspike_app::transport::PlayWindow;
    assert_eq!(PlayWindow::from_args(None, None, None, None, None).unwrap(), None);
    assert_eq!(
        PlayWindow::from_args(None, None, None, Some(&Value::Null), None).unwrap(),
        None
    );
    assert_eq!(
        PlayWindow::from_args(None, None, None, Some(&json!([])), None)
            .unwrap_err()
            .code,
        400
    );
    assert_eq!(
        PlayWindow::from_args(None, None, None, Some(&json!([1])), None)
            .unwrap_err()
            .code,
        400
    );
    // P4-40e: a shot solos its track when no tracks are given; the window holds it
    let w = PlayWindow::from_args(None, None, None, None, Some(&json!("Kick In")))
        .unwrap()
        .unwrap();
    assert_eq!(
        (w.tracks.clone(), w.shot.clone()),
        (vec!["Kick In".to_string()], Some("Kick In".to_string()))
    );
    let w = PlayWindow::from_args(None, None, None, Some(&json!([])), Some(&json!(5))).unwrap_err();
    assert_eq!(w.code, 400);
    let w = PlayWindow::from_args(Some(1.0), Some(2.0), Some(true), Some(&json!(["a"])), None)
        .unwrap()
        .unwrap();
    assert_eq!(
        (w.start, w.end, w.looped, w.tracks),
        (Some(1.0), Some(2.0), true, vec!["a".to_string()])
    );
    std::fs::remove_dir_all(root).unwrap();
}
