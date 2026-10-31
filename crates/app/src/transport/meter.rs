//! The meter messages (P3-16): the 25 Hz [`METER_PERIOD`], the message shape
//! ([`meter_message`]) and the due-time rule (`next_meter_due`) that keeps the rate under
//! late looks. See the module docs of `super` for the whole transport.

use std::time::{Duration, Instant};

use drumspike_core::transport::MeterSnapshot;
use serde_json::{Value, json};

/// Time between two meter messages while the transport plays (P3-16: 25 Hz). Each message is
/// due one period after the due time of the one before (not after its send time), so a late
/// look of the playhead thread (timer slack on a loaded Intel runner: CI run 38376400763 had
/// 17 Hz) does not lower the rate while the thread looks at least once per period.
pub const METER_PERIOD: Duration = Duration::from_millis(40);

/// Round a meter value to 4 decimals for the message.
fn meter_value(v: f32) -> f64 {
    (f64::from(v) * 1e4).round() / 1e4
}

/// The meter message of a [`MeterSnapshot`] (see the module docs).
pub(crate) fn meter_message(sid: &str, m: &MeterSnapshot) -> Value {
    let flat: Vec<f64> = m.tracks.iter().flatten().map(|v| meter_value(*v)).collect();
    json!({"sid": sid, "meters": flat, "master": [meter_value(m.master[0]), meter_value(m.master[1])]})
}

/// When the meter message after one sent at `now` is due: a period after the due time of the one
/// sent (so late looks keep the rate), or a period after `now` for the first one and after a
/// delay of more than a period (no burst to catch up).
pub(crate) fn next_meter_due(due: Option<Instant>, now: Instant) -> Instant {
    match due {
        Some(d) if now.saturating_duration_since(d) < METER_PERIOD => d + METER_PERIOD,
        _ => now + METER_PERIOD,
    }
}
