//! The playhead messages (P3-15a): the 4 Hz [`PLAYHEAD_PERIOD`] and the [`PLAYHEAD_POLL`]
//! look, the message and its change key, and the sender thread (`spawn_sender`) that also
//! sends the meter messages. See the module docs of `super` for the whole transport.

use std::sync::{Arc, Mutex, Weak};
use std::thread::sleep;
use std::time::{Duration, Instant};

use drumspike_core::transport::clock::host_now_ns;
use serde_json::{Map, Value, json};

use super::{HostState, Open, heard_epoch, meter_message, next_meter_due};
use crate::Progress;
use crate::registry::lock;

/// Longest time between two playhead messages while nothing changes (4 Hz, P3-15a).
pub const PLAYHEAD_PERIOD: Duration = Duration::from_millis(250);
/// How often the playhead thread looks at the state: a change is sent within this time.
pub const PLAYHEAD_POLL: Duration = Duration::from_millis(20);

/// The playhead fields of an open transport at the host now (epoch, `t_ns`, frame, rate, pos,
/// playing, speed). The playhead message and the meter message both carry them (#471).
fn playhead_fields(st: &HostState, o: &Open) -> Map<String, Value> {
    let ph = o.transport.playhead();
    let now = host_now_ns();
    let frame = ph.frame_at(now);
    json!({
        "epoch": heard_epoch(st, o, &ph),
        "t_ns": now,
        "frame": frame,
        "rate": ph.sample_rate,
        "pos": frame as f64 / ph.sample_rate as f64,
        "playing": ph.playing,
        "speed": ph.rate(),
    })
    .as_object()
    .cloned()
    .unwrap_or_default()
}

/// The playhead message of an open transport (see the module docs).
fn playhead_message(st: &HostState, o: &Open, end: bool) -> Value {
    let mut msg = playhead_fields(st, o);
    msg.insert("sid".into(), json!(o.plan.sid));
    msg.insert("underruns".into(), json!(o.transport.stats().underruns));
    msg.insert("reopens".into(), json!(st.reopens));
    msg.insert("end".into(), json!(end));
    Value::Object(msg)
}

/// What makes the playhead thread send at once: (epoch, playing as heard, underruns, reopens,
/// lost, slowdown as heard).
type PlayheadKey = (u64, bool, u64, u64, bool, u32);

/// The [`PlayheadKey`] of the host now.
fn playhead_key(st: &HostState) -> PlayheadKey {
    let (epoch, playing, underruns, slowdown) = st.open.as_ref().map_or((st.epoch, false, 0, 1), |o| {
        let ph = o.transport.playhead();
        (
            heard_epoch(st, o, &ph),
            ph.playing,
            o.transport.stats().underruns,
            ph.slowdown,
        )
    });
    (epoch, playing, underruns, st.reopens, st.lost.is_some(), slowdown)
}

/// The thread that sends the playhead until the transport pauses, closes or a newer play
/// starts: on a change of [`playhead_key`] (looked at every [`PLAYHEAD_POLL`]) and at least
/// every [`PLAYHEAD_PERIOD`]. It sends a last message with `end: true`. P3-16: while the
/// transport plays it also sends the meter message every [`METER_PERIOD`].
pub(crate) fn spawn_sender(host: &Arc<Mutex<HostState>>, gen_n: u64, sid: String, progress: Progress) {
    let weak: Weak<Mutex<HostState>> = Arc::downgrade(host);
    let _ = std::thread::Builder::new().name("drumspike-playhead".into()).spawn(move || {
        let mut sent: Option<(Instant, PlayheadKey)> = None;
        let mut metered: Option<Instant> = None;
        loop {
            let Some(h) = weak.upgrade() else { return };
            let (msg, meters) = {
                let st = lock(&h);
                if st.sender_gen != gen_n {
                    return;
                }
                let meters = match &st.open {
                    // `metered`: when the next meter message is due ([`METER_PERIOD`])
                    Some(o) if o.transport.is_playing() && metered.is_none_or(|due| Instant::now() >= due) => {
                        metered = Some(next_meter_due(metered, Instant::now()));
                        // #471: the meter message carries the playhead fields too (one eval per period)
                        let mut msg = meter_message(&o.plan.sid, &o.transport.take_meters());
                        if let Some(obj) = msg.as_object_mut() {
                            obj.extend(playhead_fields(&st, o));
                        }
                        Some(msg)
                    }
                    _ => None,
                };
                let stopping = !st.open.as_ref().is_some_and(|o| o.transport.is_playing()) && st.lost.is_none();
                let key = playhead_key(&st);
                let due = match sent {
                    None => true,
                    Some((at, k)) => k != key || at.elapsed() >= PLAYHEAD_PERIOD || stopping,
                };
                let msg = if due {
                    sent = Some((Instant::now(), key));
                    Some(match &st.open {
                        Some(o) if o.transport.is_playing() => (playhead_message(&st, o, false), false),
                        Some(o) => (playhead_message(&st, o, true), true),
                        None => {
                            let (error, pos) = st
                                .lost
                                .as_ref()
                                .map_or((Value::Null, 0.0), |l| (json!(l.error), l.position_s));
                            let msg = json!({"sid": sid, "epoch": st.epoch, "t_ns": host_now_ns(), "frame": 0, "rate": 0,
                                "pos": pos, "playing": false, "speed": 1.0, "underruns": 0, "reopens": st.reopens,
                                "end": error.is_null(), "error": error});
                            (msg, error.is_null())
                        }
                    })
                } else {
                    None
                };
                (msg, meters)
            };
            drop(h);
            if let Some(m) = meters {
                progress(m);
            }
            if let Some((msg, last)) = msg {
                progress(msg);
                if last {
                    return;
                }
            }
            sleep(PLAYHEAD_POLL);
        }
    });
}
