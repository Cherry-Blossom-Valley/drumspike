//! The transport in the app (P3-5b): play, pause, seek, mute, solo and clicks on the native
//! transport of `drumspike-core` (`drumspike_core::transport`, P3-5a).
//!
//! | Command | Effect |
//! |---|---|
//! | `transport_play {sid, position?, events?, speed?, progress}` | open the transport for the project when needed, seek, play, start the playhead messages; P3-20b: `speed` 0.5 plays at half speed (one octave down), 1 (the default) at normal speed |
//! | `transport_play` window (P4-40d) | `start`, `end`, `loop`, `tracks`: an audition. Plays from `start` to `end` (a loop until stop when `loop`) with exactly `tracks` audible, whatever mute and solo say; for this play only (see "Audition" below) |
//! | `transport_stop {close?}` | pause; answers the paused position |
//! | `transport_seek {position}` | move the playhead (playing or paused) |
//! | `transport_mix {mute, solo}` | mute and solo by track name |
//! | `transport_clicks {enabled?, volume?, pitches?}` | metronome clicks: on, volume, pitch per role |
//! | `transport_status {}` | the state, the counters and the clock (tests, self-test) |
//! | `track_set_mix {sid, name, volume_db?, pan?}` | P3-16: a track's volume (dB) and pan, kept in the project (marks it dirty) and applied to the open transport |
//! | `transport_master {volume_db}` | P3-16: the master volume (dB), kept in the app settings |
//! | `outputs_list {}` | P3-16: the output devices, the saved choice, the device in use and the fallback message |
//! | `output_select {device}` | P3-16: choose an output device by name (`null`: the system default), kept in the app settings |
//!
//! # Audition (P4-40d)
//!
//! `transport_play` with `start`, `end`, `loop` or `tracks` ([`PlayWindow`]) is an audition. The tracks of `tracks` are
//! the only audible ones for this play (the override sits in `mix_lists`; `mute` and `solo` of the host are not
//! changed), it plays from `start` and pauses at `end`, or with `loop` plays the window again until `transport_stop` or
//! another play. A loop wraps in the engine (#166) and a windowed audition ends in the engine
//! (#201): the play's seek arms the region and the window end, the reader and the callback wrap
//! there and stop at the end, so the end is exact — every frame up to it is delivered, also when
//! the host starves the reader (#200). A thread (`spawn_audition`) watches the transport every
//! millisecond and gives mute and solo back once the audition is over. The audition sends no playhead
//! messages, so the page's playhead does not move. It ends at the end of the window, at a stop, at a plain play and when
//! the project closes; each of these gives mute and solo back (`finish_audition`, `restore_mix`). The surface stays
//! `Window`: the command line and the MCP server have no transport. `shot` (P4-40e) previews a track's layer one-shot
//! once at the start of the window (one extra voice in the plan; the close mic sounds as in any audition).
//!
//! # Layers (P4-7a)
//!
//! A track with a one-shot layer (`crate::layer`) that is on gets layer voices: the one-shot at the
//! device rate (and at twice it for half speed), a voice per uncut hit of the document, started at
//! the frame where the one-shot's onset lands on the hit (the click path generalised, see
//! `drumspike_core::transport::layer`). The plan holds the voices' start times; the transport gets
//! them as one bank at the open, at a play whose layers changed, and when a layer setting changes
//! while the project's transport is open (`track_layer_set`). `transport_status` counts the voices the
//! callback started (`layer_voices`) and gives the output frame of the last one (`layer_last_start`).
//! A transport without layers sends no bank and the callback does no layer work.
//!
//! # Playhead messages
//!
//! `progress` gets a message when the state changes (epoch, `playing`, underruns, reopens),
//! else one every 250 ms (4 Hz, P3-15a) while the transport plays, and a last one with
//! `end: true`. Each message is a `webview.eval` in the app (Tauri sends channel messages
//! under 8 KiB that way). Measured in P3-15a: going from 30 Hz to 4 Hz lowers the `WebKit`
//! allocator scavenger only from about 3.8 % to 3.3 % of a core; the 60 Hz drawing of the page
//! keeps it busy (`docs/archive/phase3-report.md`, section P3-15a). The message format:
//! `{sid, epoch, t_ns, frame, rate, pos, playing, speed, underruns, reopens, end}`.
//! `pos` (seconds) is the heard position at `t_ns` (the monotonic clock of the process, as
//! `transport_status` reports it): the callback's playback timestamp, extrapolated. The UI
//! keeps the last message and extrapolates `pos + (now - t) * speed` for each frame it draws
//! (P3-20b: `speed` is 1.0, or 0.5 at half speed: the session seconds per second).
//! `playing` is false while the transport pauses or fills its rings after a seek; the UI must
//! not run the playhead then. `epoch` grows with every seek, play and stop; the UI drops a
//! message whose `epoch` is older than the answer of its last seek. P3-15f: a message carries
//! the epoch of a seek only when the callback has taken that seek (the seek tag in the same
//! clock snapshot as the position); before that it carries an older epoch.
//!
//! # Meter messages (P3-16)
//!
//! While the transport plays, `progress` also gets one message for all tracks about every
//! 40 ms ([`METER_PERIOD`], 25 Hz): `{sid, meters: [l0, r0, l1, r1, ...], master: [l, r], epoch,
//! t_ns, frame, rate, pos, playing, speed}`, the linear peaks since the last message (1.0 is 0
//! dBFS), rounded to 4 decimals, in the order of the `names` that `transport_play` answers
//! (`channels` gives 2 for a track that plays a stereo file as stereo, else 1). The playhead
//! fields (#471) are read in the same look as the peaks; the 4 Hz playhead message stays. A mono track's two values are its level after the volume and
//! before the pan; a stereo track's are its left and right after volume and balance. Both are
//! before mute and solo; the master meter is after the master volume and before the master
//! gain of `DRUMSPIKE_MASTER_GAIN` (so a silent test run still shows its meters). The meters
//! are monitor values only: no export reads volume, pan or the master volume.
//!
//! # Output device (P3-16)
//!
//! The app settings (`<home>/settings.json`, keys `audio_output` and `master_db`) keep the
//! output choice. `null` (the default) plays on the system default output and follows a change
//! of it: the watcher sees the new device and reopens (P3-5b). A named device is pinned; when it
//! is not connected (or does not answer), the transport plays on the system default and
//! `outputs_list` and `transport_status` carry the message (`note`); when the device comes back,
//! the watcher moves to it again.
//!
//! # Half speed (P3-20b)
//!
//! `transport_play` with `speed: 0.5` plays from the position at half speed, one octave down (the
//! core resamples to twice the device rate, see `drumspike_core::transport`). An open transport
//! changes its speed in the running stream: the change is the seek of the play (the rings drain
//! and refill at the new rate). No transport open: it opens at that speed. A play without
//! `speed` is normal speed. Both designs of the card were measured on the 30-track session
//! (`docs/archive/phase3-report.md`, section P3-20b): the change in the running stream is the faster, a
//! close and reopen at the new speed pays the device open. A seek keeps the speed; a reopen
//! after a device change keeps it too.
//!
//! # Sample rates
//!
//! The device rate is the device's (P3-5a policy). A command speaks seconds; the transport
//! counts device frames, so `frame / rate` is the position.
//!
//! # Silence for tests
//!
//! `DRUMSPIKE_MASTER_GAIN` sets the master gain (after the mix) of every transport the app
//! opens, also after a reopen. Without it the gain is 1.0, the user's. `DRUMSPIKE_AUDIO=null`
//! selects the null device.
//!
//! # Device changes
//!
//! A watcher thread asks the system every 500 ms for the default output. When its name, rate
//! or channel count differs from the open stream, or the stream reported an error, the
//! transport reopens on the new device at the same position and in the same state (playing
//! or paused, mute, solo, clicks, master gain). If the reopen fails, the watcher retries at
//! the next tick, and `transport_status` says `lost`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak};
use std::thread::sleep;
use std::time::{Duration, Instant};

#[cfg(test)]
use drumspike_core::layer::OneShot;
use drumspike_core::transport::clicks::{DEFAULT_CLICK_VOLUME, role_pitch};
use drumspike_core::transport::clock::{Playhead, host_now_ns};
use drumspike_core::transport::device::{
    DeviceInfo, Output, default_output_info, is_blocked, output_devices, output_info, resolve_choice,
};
use drumspike_core::transport::mixer::{MAX_DB, MIN_DB};
use drumspike_core::transport::{CallbackCounter, ClickSettings, Speed, Transport, TransportConfig, TransportError};
#[cfg(test)]
use drumspike_core::transport::{MeterSnapshot, TrackSpec};
use serde_json::{Map, Value, json};

use crate::error::AppError;
#[cfg(test)]
use crate::media::mix_mode;
use crate::registry::lock;
use crate::{Progress, Service};

mod audition;
mod meter;
mod plan;
mod playhead;

pub use audition::PlayWindow;
pub(crate) use audition::{Audition, spawn_audition};
pub use meter::METER_PERIOD;
pub(crate) use meter::{meter_message, next_meter_due};
#[cfg(test)]
pub(crate) use plan::LAYER_PLAN_CALLS;
#[cfg(test)]
pub(crate) use plan::{LayerPlan, hits_from_times, layer_bank, times_of};
pub(crate) use plan::{LayerTpls, Plan, push_layers};
pub(crate) use playhead::spawn_sender;
pub use playhead::{PLAYHEAD_PERIOD, PLAYHEAD_POLL};

/// Every command of the transport. (`COMMANDS` in `commands.rs` lists the slice; the
/// route map test of this card reads this list.)
pub const TRANSPORT_COMMANDS: [&str; 10] = [
    "transport_play",
    "transport_stop",
    "transport_seek",
    "transport_mix",
    "transport_clicks",
    "transport_status",
    "track_set_mix",
    "transport_master",
    "outputs_list",
    "output_select",
];

/// Time between two looks at the default output device.
pub const WATCH_PERIOD: Duration = Duration::from_millis(500);
/// How long `transport_play` waits for the first playing block.
const START_WAIT: Duration = Duration::from_secs(10);

/// What identifies an output device: a change of any field reopens the stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceKey {
    /// The device name.
    pub name: String,
    /// The device's current rate.
    pub rate: u32,
    /// Output channels.
    pub channels: u16,
}

impl DeviceKey {
    fn json(&self) -> Value {
        json!({"name": self.name, "rate": self.rate, "channels": self.channels})
    }

    fn of(d: &DeviceInfo) -> DeviceKey {
        DeviceKey {
            name: d.name.clone(),
            rate: d.sample_rate,
            channels: d.channels,
        }
    }
}

/// A volume in dB from a command or the project, clamped to the fader range ([`MIN_DB`] is off).
fn clamp_db(db: f64) -> f64 {
    db.clamp(f64::from(MIN_DB), f64::from(MAX_DB))
}

/// A pan from a command or the project, clamped to -1 to +1.
fn clamp_pan(pan: f64) -> f64 {
    pan.clamp(-1.0, 1.0)
}

/// The metronome settings the page owns (`SETTINGS().click` in `app.js`).
#[derive(Debug, Clone)]
struct ClickState {
    enabled: bool,
    volume: f32,
    /// Hz per role; a role without an entry uses `role_pitch`.
    pitches: HashMap<String, f32>,
}

impl ClickState {
    fn pitch(&self, role: &str) -> f32 {
        self.pitches.get(role).copied().unwrap_or_else(|| role_pitch(role))
    }
}

pub(crate) struct Open {
    plan: Plan,
    transport: Transport,
    key: DeviceKey,
    errors_at_open: u64,
    open_ms: f64,
    /// P3-15f: the epoch of the last seek requested on this transport, also its seek tag (0:
    /// none). See [`heard_epoch`].
    seek_epoch: u64,
}

/// A reopen that failed: tried again at the next watcher tick.
struct Lost {
    plan: Plan,
    position_s: f64,
    playing: bool,
    /// P3-20b: the speed to play at again.
    speed: Speed,
    error: String,
}

pub(crate) struct HostState {
    open: Option<Open>,
    /// P4-40d: the running audition (its solo override and window).
    audition: Option<Audition>,
    /// Grows with every play and stop: the audition thread of an older play ends itself.
    audition_gen: u64,
    lost: Option<Lost>,
    mute: HashMap<String, bool>,
    solo: HashMap<String, bool>,
    clicks: ClickState,
    master: f32,
    output: Output,
    /// Test hook: the device the watcher sees for a null output (`None`: the stream's own).
    probe_override: Option<DeviceKey>,
    watch_period: Duration,
    /// Grows with every seek, play and stop.
    epoch: u64,
    /// The playhead thread of the last `transport_play` (an older one ends itself).
    sender_gen: u64,
    reopens: u64,
    watcher_started: bool,
    /// The span between `transport_play` and the first playing block, in ms.
    last_start_ms: Option<f64>,
    /// Why the last reopen happened (old device, new device, stream errors).
    last_reopen: Option<String>,
    /// When the last reopen ran: a stream error right after a reopen does not start another one.
    last_reopen_at: Option<Instant>,
    /// P3-16: the saved output choice (`None`: the system default).
    choice: Option<String>,
    /// P3-16: why the transport does not play on the choice (missing or silent device).
    note: Option<String>,
    /// P3-16: the user's master volume in dB.
    master_db: f64,
    /// P3-16: the app settings were read (once the home folder is known).
    settings_loaded: bool,
    /// P3-16 test hook: the device list of a null output (`None`: no devices).
    device_list: Option<Vec<DeviceKey>>,
    /// P4-7a: the one-shots at a device rate and at twice it, by (file, rate, frames).
    layer_tpls: LayerTpls,
}

/// The transport of the app: one per [`Service`].
pub struct TransportHost {
    st: Arc<Mutex<HostState>>,
}

fn master_from_env() -> f32 {
    std::env::var("DRUMSPIKE_MASTER_GAIN")
        .ok()
        .and_then(|v| v.trim().parse::<f32>().ok())
        .filter(|g| g.is_finite() && *g >= 0.0)
        .unwrap_or(1.0)
}

impl Default for TransportHost {
    fn default() -> Self {
        TransportHost::new()
    }
}

impl TransportHost {
    /// A host with no transport. The output comes from `DRUMSPIKE_AUDIO`, the master gain
    /// from `DRUMSPIKE_MASTER_GAIN`.
    pub fn new() -> TransportHost {
        TransportHost {
            st: Arc::new(Mutex::new(HostState {
                open: None,
                audition: None,
                audition_gen: 0,
                lost: None,
                mute: HashMap::new(),
                solo: HashMap::new(),
                clicks: ClickState {
                    enabled: true,
                    volume: DEFAULT_CLICK_VOLUME,
                    pitches: HashMap::new(),
                },
                master: master_from_env(),
                output: Output::from_env(),
                probe_override: None,
                watch_period: WATCH_PERIOD,
                epoch: 0,
                sender_gen: 0,
                reopens: 0,
                watcher_started: false,
                last_start_ms: None,
                last_reopen: None,
                last_reopen_at: None,
                choice: None,
                note: None,
                master_db: 0.0,
                settings_loaded: false,
                device_list: None,
                layer_tpls: HashMap::new(),
            })),
        }
    }
}

fn dev_err(e: TransportError) -> AppError {
    match e {
        TransportError::Arg(m) => AppError::bad(m),
        other => AppError::internal(other.to_string()),
    }
}

/// The key of the stream that `t` runs on.
fn key_of(t: &Transport) -> DeviceKey {
    let d = t.device();
    DeviceKey {
        name: d.name.clone(),
        rate: d.sample_rate,
        channels: d.channels,
    }
}

/// What the watcher must ask: the output, the test override and the output choice (read
/// under the host lock).
struct ProbeInput {
    output: Output,
    override_key: Option<DeviceKey>,
    choice: Option<String>,
}

/// The [`ProbeInput`] of the host now.
fn probe_input(st: &HostState) -> ProbeInput {
    ProbeInput {
        output: st.output.clone(),
        override_key: st.probe_override.clone(),
        choice: st.choice.clone(),
    }
}

/// What the watcher sees now: the test override for a null output; for the real one the chosen
/// device when it is connected and answers, else the system's default output. `None`: nothing
/// to compare (no device answers). P3-15c: the `CoreAudio` query runs without the host lock (the
/// caller passes what it read under the lock).
fn probe(input: ProbeInput) -> Option<DeviceKey> {
    if let Output::Null { .. } = input.output {
        input.override_key
    } else {
        let _span = crate::trace::stage("transport:device_query");
        let chosen = input
            .choice
            .filter(|n| !is_blocked(n))
            .and_then(|n| output_info(Some(&n)).ok());
        chosen.or_else(|| default_output_info().ok()).map(|d| DeviceKey::of(&d))
    }
}

/// The output for the next open and the fallback message: a null device follows the
/// override's rate (and resolves the choice against the test device list); the real output
/// resolves the choice against the connected devices ([`resolve_choice`]).
fn output_for(st: &HostState) -> (Output, Option<String>) {
    if let Output::Null { .. } = &st.output {
        let names: Vec<String> = st.device_list.iter().flatten().map(|k| k.name.clone()).collect();
        let note = st.choice.as_deref().and_then(|c| resolve_choice(Some(c), &names).1);
        let out = match &st.probe_override {
            Some(k) => Output::Null {
                sample_rate: Some(k.rate),
            },
            None => st.output.clone(),
        };
        (out, note)
    } else {
        let names: Vec<String> = output_devices()
            .map(|l| l.into_iter().map(|d| d.name).collect())
            .unwrap_or_default();
        resolve_choice(st.choice.as_deref(), &names)
    }
}

fn config_for(st: &HostState, speed: Speed) -> TransportConfig {
    TransportConfig {
        click_volume: st.clicks.volume,
        clicks_on: st.clicks.enabled,
        speed,
        ..TransportConfig::default()
    }
}

/// `audible()` of `app.js` for the names of a plan.
fn mix_lists(st: &HostState, names: &[String]) -> (Vec<bool>, Vec<bool>) {
    let get = |m: &HashMap<String, bool>, n: &String| m.get(n).copied().unwrap_or(false);
    let solo = match st.audition.as_ref().filter(|a| !a.win.tracks.is_empty()) {
        // P4-40d: an audition solos its tracks and no others; `st.solo` is not touched
        Some(a) => names.iter().map(|n| a.win.tracks.contains(n)).collect(),
        None => names.iter().map(|n| get(&st.solo, n)).collect(),
    };
    (names.iter().map(|n| get(&st.mute, n)).collect(), solo)
}

/// Push mute and solo (with a running audition's override) into the open transport.
fn restore_mix(st: &HostState) {
    if let Some(o) = &st.open {
        let (m, s) = mix_lists(st, &o.plan.names);
        let _ = o.transport.set_mix(&m, &s);
    }
}

/// Push mute, solo, master gain, volume and pan, the master volume and the click pitches into
/// an open transport.
fn apply_state(st: &HostState, open: &mut Open) -> Result<(), AppError> {
    let t = &mut open.transport;
    t.set_master_gain(st.master);
    t.set_master_volume_db(st.master_db as f32);
    for (i, (db, pan)) in open.plan.vols.iter().enumerate() {
        t.set_volume_pan(i, *db as f32, *pan as f32).map_err(dev_err)?;
    }
    let (mute, solo) = mix_lists(st, &open.plan.names);
    t.set_mix(&mute, &solo).map_err(dev_err)?;
    let pitches: Vec<f32> = open.plan.roles.iter().map(|r| st.clicks.pitch(r)).collect();
    t.set_clicks(&ClickSettings {
        enabled: st.clicks.enabled,
        volume: st.clicks.volume,
        pitches: Some(pitches),
    })
    .map_err(dev_err)
}

/// Open the transport for `plan` on the output the host chose, paused at frame 0, at `speed`. A
/// chosen device that is missing or does not answer gives way to the system default; `st.note`
/// says so.
fn start(st: &mut HostState, plan: Plan, speed: Speed) -> Result<Open, AppError> {
    let t0 = Instant::now();
    let mut specs = plan.specs.clone();
    for (s, r) in specs.iter_mut().zip(&plan.roles) {
        s.click_pitch = st.clicks.pitch(r);
    }
    let (out, mut note) = output_for(st);
    let opened = match Transport::open(&specs, out.clone(), config_for(st, speed)) {
        Err(TransportError::NoAnswer(_)) if matches!(out, Output::Device { .. }) => {
            if let Output::Device { name } = &out {
                note = Some(format!(
                    "The output device \"{name}\" does not answer. Drum Spike plays on the system default."
                ));
            }
            Transport::open(&specs, Output::Default, config_for(st, speed))
        }
        r => r,
    };
    st.note = note;
    let transport = opened.map_err(dev_err)?;
    let key = key_of(&transport);
    let errors_at_open = transport.stats().stream_errors;
    let mut open = Open {
        plan,
        transport,
        key,
        errors_at_open,
        open_ms: t0.elapsed().as_secs_f64() * 1e3,
        seek_epoch: 0,
    };
    apply_state(st, &mut open)?;
    if open.plan.layers.iter().any(Option::is_some) {
        push_layers(&mut st.layer_tpls, &mut open)?;
    }
    Ok(open)
}

/// Wait until the callback has taken the seek tagged `epoch`: the clock then shows the target,
/// not the old position. P3-15c: the caller does not hold the host lock while it waits (up to
/// 100 ms). P3-15f: the wait is not needed for the messages any more ([`heard_epoch`]); it keeps
/// the answer after the move, as before.
fn wait_seek_taken(counter: &CallbackCounter, epoch: u64) {
    let _span = crate::trace::stage("transport:seek_wait");
    let end = Instant::now() + Duration::from_millis(100);
    while counter.seek_taken() < epoch && Instant::now() < end {
        sleep(Duration::from_micros(500));
    }
}

/// Seek `o` to `frame`, tagged with `epoch` (the epoch that the seek's answer carries).
fn seek_tagged(o: &mut Open, frame: u64, epoch: u64) {
    o.seek_epoch = epoch;
    o.transport.seek_tagged(frame, epoch);
}

/// #166, #201: [`seek_tagged`] that arms a loop region and a window end (each `Some`/`None`, in
/// session frames): the engine wraps a looped audition's window there with no gap and no seek,
/// and ends a windowed audition itself at its end, every frame delivered (#200).
fn seek_tagged_window(o: &mut Open, frame: u64, epoch: u64, loop_frames: Option<(u64, u64)>, end_frames: Option<u64>) {
    o.seek_epoch = epoch;
    o.transport.seek_tagged_window(frame, epoch, loop_frames, end_frames);
}

/// P3-15f: the epoch of a message or status that shows the playhead `ph` of `o`. The callback
/// publishes the tag of the last seek it took in the same snapshot as the position. When it has
/// taken the last requested seek, the epoch is the host's. Else the position is from before
/// that seek, and the epoch is the tag of the seek it shows: older than the answer of the
/// pending seek, so the page drops the message. Thus no epoch of a seek ever comes with a
/// position from before that seek, however late the waiting thread runs.
fn heard_epoch(st: &HostState, o: &Open, ph: &Playhead) -> u64 {
    if ph.seek_tag >= o.seek_epoch {
        st.epoch
    } else {
        ph.seek_tag
    }
}

/// The position of an open transport in seconds, extrapolated to now.
fn position_s(t: &Transport) -> f64 {
    let ph = t.playhead();
    ph.frame_at(host_now_ns()) as f64 / f64::from(ph.sample_rate)
}

/// Reopen on the device the watcher sees: same position, same state.
fn reopen(st: &mut HostState, why: &str) {
    let Some(old) = st.open.take() else { return };
    let (pos, playing, speed) = (
        position_s(&old.transport),
        old.transport.is_playing(),
        old.transport.speed(),
    );
    let plan = old.plan.clone();
    drop(old);
    resume(st, plan, pos, playing, speed, why);
}

/// Open `plan` and continue at `pos` seconds at `speed`. On failure the state becomes `lost`.
fn resume(st: &mut HostState, plan: Plan, pos: f64, playing: bool, speed: Speed, why: &str) {
    st.last_reopen_at = Some(Instant::now());
    match start(st, plan.clone(), speed) {
        Ok(mut open) => {
            // the new transport starts at frame 0: its messages carry the new epoch only
            // once the callback has taken this seek
            let frame = open.transport.frame_of(pos);
            // #166: a running looped audition keeps its region across the reopen; #201: a
            // running windowed audition keeps its end
            let loop_frames = st
                .audition
                .as_ref()
                .filter(|a| a.win.looped)
                .and_then(|a| a.win.end.map(|e| (a.start, e)))
                .map(|(s, e)| (open.transport.frame_of(s), open.transport.frame_of(e)));
            let end_frames = st
                .audition
                .as_ref()
                .filter(|a| !a.win.looped)
                .and_then(|a| a.win.end)
                .map(|e| open.transport.frame_of(e));
            seek_tagged_window(&mut open, frame, st.epoch + 1, loop_frames, end_frames);
            if playing {
                open.transport.play();
            }
            st.open = Some(open);
            st.lost = None;
            st.reopens += 1;
            st.epoch += 1;
        }
        Err(e) => {
            st.lost = Some(Lost {
                plan,
                position_s: pos,
                playing,
                speed,
                error: format!("{why}: {e}"),
            });
        }
    }
}

/// One look at the output device (the watcher's tick): `seen` is what [`probe`] answered,
/// without the lock, just before.
fn watch_tick(st: &mut HostState, seen: Option<DeviceKey>) {
    if let Some(l) = st.lost.take() {
        resume(st, l.plan, l.position_s, l.playing, l.speed, "retry");
        if st.lost.is_some() {
            // keep the first error text and try again at the next tick
            return;
        }
    }
    let Some(open) = st.open.as_ref() else { return };
    let errors = open.transport.stats().stream_errors;
    let changed = seen.as_ref().is_some_and(|k| *k != open.key);
    if changed {
        st.last_reopen = Some(format!(
            "device {:?} -> {:?}",
            st.open.as_ref().map(|o| o.key.clone()),
            seen
        ));
        reopen(st, "output device changed");
    } else if errors > open.errors_at_open && st.last_reopen_at.is_none_or(|t| t.elapsed() > Duration::from_secs(3)) {
        st.last_reopen = Some(format!("stream errors {} -> {errors}", open.errors_at_open));
        reopen(st, "stream error");
    }
}

fn spawn_watcher(host: &Arc<Mutex<HostState>>) {
    let weak: Weak<Mutex<HostState>> = Arc::downgrade(host);
    let _ = std::thread::Builder::new()
        .name("drumspike-audio-watch".into())
        .spawn(move || {
            loop {
                let period = match weak.upgrade() {
                    Some(h) => lock(&h).watch_period,
                    None => return,
                };
                sleep(period);
                let Some(h) = weak.upgrade() else { return };
                let input = {
                    let st = lock(&h);
                    // nothing to compare without an open transport: no device query
                    if st.open.is_none() && st.lost.is_none() {
                        continue;
                    }
                    probe_input(&st)
                };
                let seen = probe(input);
                watch_tick(&mut lock(&h), seen);
            }
        });
}

impl Service {
    fn host(&self) -> &TransportHost {
        &self.inner.transport
    }

    /// The app settings file (`<home>/settings.json`), when the home folder is known.
    fn settings_path(&self) -> Option<PathBuf> {
        self.home().map(|h| h.join("settings.json"))
    }

    /// Read the output choice and the master volume from the app settings, once.
    fn load_audio_settings(&self) {
        let path = self.settings_path();
        let mut st = lock(&self.host().st);
        if st.settings_loaded {
            return;
        }
        let Some(path) = path else { return };
        st.settings_loaded = true;
        let v: Option<Value> = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok());
        if let Some(v) = v {
            st.choice = v.get("audio_output").and_then(Value::as_str).map(str::to_string);
            if let Some(db) = v.get("master_db").and_then(Value::as_f64) {
                st.master_db = clamp_db(db);
            }
        }
    }

    /// Write the output choice and the master volume into the app settings (other keys stay).
    fn save_audio_settings(&self, choice: Option<&str>, master_db: f64) -> Result<(), AppError> {
        let Some(path) = self.settings_path() else {
            return Ok(());
        };
        let mut v: Value = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}));
        v["audio_output"] = json!(choice);
        v["master_db"] = json!(master_db);
        crate::lifecycle::write_json_atomic(&path, &v)
    }

    /// Test hook (P3-16): with a null output, the output devices that `outputs_list` lists
    /// and the choice resolves against (the first one is the system default).
    pub fn set_device_list(&self, list: Option<Vec<DeviceKey>>) {
        lock(&self.host().st).device_list = list;
    }

    /// Choose the output (tests: [`Output::Null`]). Closes an open transport so the next
    /// play opens on the new output.
    pub fn set_output_select(&self, output: Output) {
        let mut st = lock(&self.host().st);
        st.output = output;
        st.open = None;
        st.lost = None;
    }

    /// Set the master gain (after the mix) for the transports that open from now on and for
    /// the open one. The default is 1.0 (or `DRUMSPIKE_MASTER_GAIN`).
    pub fn set_master_gain(&self, gain: f32) {
        let mut st = lock(&self.host().st);
        st.master = gain;
        if let Some(o) = &st.open {
            o.transport.set_master_gain(gain);
        }
    }

    /// Test hook for the device watcher: with a null output, the device the watcher sees.
    /// A change of the key reopens the transport as a real change of the default output would.
    pub fn set_probe_device(&self, key: Option<DeviceKey>, watch_period: Option<Duration>) {
        let mut st = lock(&self.host().st);
        st.probe_override = key;
        if let Some(p) = watch_period {
            st.watch_period = p;
        }
    }

    /// `transport_play`: play from `position` (seconds; without it from where the transport
    /// is). Opens the transport when none is open for this project or when its tracks
    /// changed. `events` (`{track name: [t, ...]}`) are the hits the clicks follow; without
    /// them the project's events count. `progress` gets the playhead messages.
    ///
    /// Answers `{sid, device, rate, channels, frames, duration, position, epoch, ready,
    /// start_ms, open_ms, master, speed, heard_ms}` once the first playing block is rendered
    /// (`ready`), or after 10 s. `heard_ms` (P3-20b) is the time from the call to the moment the
    /// first sample of that block is heard (the block's playback time; `null` before it). It plays at normal speed ([`Service::transport_play_at`] takes a speed).
    ///
    /// # Errors
    /// The same as [`Service::transport_play_window`] without a window or a speed.
    pub fn transport_play(
        &self,
        sid: &str,
        position: Option<f64>,
        events: Option<&Value>,
        progress: Option<Progress>,
    ) -> Result<Value, AppError> {
        self.transport_play_at(sid, position, events, progress, None)
    }

    /// `transport_play` with the speed of the command (P3-20b): `speed` 0.5 is half speed, 1.0
    /// or `None` normal speed; another value is a 400. An open transport changes its speed in
    /// the running stream, as part of the play's seek (from `position`, else from where it is);
    /// otherwise the transport opens at that speed. `start_ms` is the time from the call to the
    /// first block at that speed.
    ///
    /// # Errors
    /// The same as [`Service::transport_play_window`] without a window.
    pub fn transport_play_at(
        &self,
        sid: &str,
        position: Option<f64>,
        events: Option<&Value>,
        progress: Option<Progress>,
        speed: Option<f64>,
    ) -> Result<Value, AppError> {
        self.transport_play_window(sid, position, events, progress, speed, None)
    }

    /// `transport_play` with an audition window (P4-40d, see [`PlayWindow`]). `win` `None` is a plain play (it also ends a
    /// running audition and gives mute and solo back). With a window the play starts at `start` (else `position`, else 0),
    /// the tracks of `win.tracks` are the only audible ones for this play (mute and solo of the host are not changed; an
    /// unknown name is a 400), it pauses at `end` or, with `looped`, starts again at `start` until a stop or another play. The
    /// page passes no `progress`: its playhead does not move. The answer has `audition` (the window, or `null`). With
    /// `shot` (P4-40e) the one-shot of that track plays once as a preview voice at the window start (the Layer view's Play);
    /// without an `end` the window ends at the start plus the one-shot's length, and without `tracks` the shot track is the
    /// only audible one. 409 for a track without a layer or its file.
    ///
    /// # Errors
    /// 400 when `position`, or the `start` or `end` of `win`, is negative or not finite, `end` is
    /// not after `start`, `loop` has no `end`, `speed` is not 1 or 0.5, or an audition track is not
    /// a playable track of the project; 409 for a `shot` track without a layer or its file; 404 when
    /// `sid` is not an open project; any error of building the plan or opening the audio device;
    /// 500 when the device is gone before the answer is built.
    ///
    /// # Panics
    /// Panics on `expect("open after start")` if the transport is not open right after it was
    /// opened or reused under the same lock, and on the `unwrap()` of the audition when a `shot`
    /// is set without one; neither can happen, as both are set just before.
    pub fn transport_play_window(
        &self,
        sid: &str,
        position: Option<f64>,
        events: Option<&Value>,
        progress: Option<Progress>,
        speed: Option<f64>,
        win: Option<PlayWindow>,
    ) -> Result<Value, AppError> {
        let mut position = position;
        let mut audition = None;
        // an audition sends no playhead messages: the page's playhead does not move (and an older sender ends)
        let progress = if win.is_some() { None } else { progress };
        if let Some(w) = &win {
            let ok = |v: Option<f64>| v.is_none_or(|x| x.is_finite() && x >= 0.0);
            if !ok(w.start) || !ok(w.end) {
                return Err(AppError::bad("start and end must be numbers of seconds, 0 or more"));
            }
            let start = w.start.or(position).unwrap_or(0.0);
            if w.end.is_some_and(|e| e <= start) {
                return Err(AppError::bad("end must be after start"));
            }
            if w.looped && w.end.is_none() {
                return Err(AppError::bad("loop needs an end"));
            }
            // P4-40e: a one-shot preview plays once through: the window ends at the start plus the
            // one-shot's length (409 for a track without a layer, before the transport opens), and
            // the shot track is the only audible one when the caller named none.
            let mut tracks = w.tracks.clone();
            if let Some(n) = &w.shot
                && !tracks.iter().any(|t| t == n)
            {
                tracks.push(n.clone());
            }
            let end = match (&w.shot, w.end) {
                (Some(n), None) => Some(start + self.layer_len_s(sid, n)?),
                (_, e) => e,
            };
            position = Some(start);
            audition = Some(Audition {
                win: PlayWindow {
                    tracks,
                    end,
                    ..w.clone()
                },
                start,
            });
        }
        if position.is_some_and(|p| !p.is_finite() || p < 0.0) {
            return Err(AppError::bad("position must be a number of seconds, 0 or more"));
        }
        let speed = match speed {
            None => Speed::Normal,
            Some(r) => Speed::from_rate(r).ok_or_else(|| AppError::bad("speed must be 1 (normal) or 0.5 (half)"))?,
        };
        self.load_audio_settings();
        let mut plan = self.transport_plan(sid, events)?;
        // P4-40e: the preview voice of the Layer view's Play: the one-shot once at the start of the
        // window, through the track's mute, solo, volume and pan (the close mic sounds as in any
        // audition). The plan is rebuilt here on every play, so the voice is in it and the
        // open-or-reuse branch below pushes the changed layer bank.
        if let Some(n) = audition.as_ref().and_then(|a| a.win.shot.as_ref()) {
            let at = audition.as_ref().unwrap().start;
            self.plan_with_preview(sid, n, &mut plan, at)?;
        }
        if let Some(a) = &audition
            && let Some(n) = a.win.tracks.iter().find(|n| !plan.names.contains(n))
        {
            return Err(AppError::bad(format!(
                "audition track \"{n}\" is not in the project's playable tracks"
            )));
        }
        let t0 = Instant::now();
        let t0_ns = host_now_ns();
        let (gen_n, rate, counter, epoch, sought, agen) = {
            let host = self.host();
            let mut st = lock(&host.st);
            if !st.watcher_started {
                st.watcher_started = true;
                spawn_watcher(&host.st);
            }
            // P4-40d: a play replaces the audition before the transport opens, so that the open sets the right mix
            st.audition = audition.clone();
            st.audition_gen += 1;
            let agen = st.audition_gen;
            let reusable = st.open.as_ref().is_some_and(|o| o.plan.same_tracks(&plan));
            if !reusable {
                st.open = None;
                st.lost = None;
                let open = start(&mut st, plan.clone(), speed)?;
                st.open = Some(open);
            } else if let Some(o) = st.open.as_mut() {
                if o.plan.hits() != plan.hits() {
                    o.transport.set_all_hits(plan.hits()).map_err(dev_err)?;
                }
                let layers_changed = o.plan.layers != plan.layers;
                for (i, (db, pan)) in plan.vols.iter().enumerate() {
                    o.transport
                        .set_volume_pan(i, *db as f32, *pan as f32)
                        .map_err(dev_err)?;
                }
                o.plan = plan.clone();
                if layers_changed {
                    let st = &mut *st;
                    if let Some(o) = st.open.as_mut() {
                        push_layers(&mut st.layer_tpls, o)?;
                    }
                }
            }
            restore_mix(&st);
            st.epoch += 1;
            st.sender_gen += 1;
            let epoch = st.epoch;
            let o = st.open.as_mut().expect("open after start");
            let counter = o.transport.callback_counter();
            let respeed = o.transport.speed() != speed;
            // #166: a looped audition arms its region with the play's seek (session frames); any
            // other seek clears it, so a plain play or a move never wraps
            let loop_frames = audition
                .as_ref()
                .filter(|a| a.win.looped)
                .and_then(|a| a.win.end.map(|e| (a.start, e)))
                .map(|(s, e)| (o.transport.frame_of(s), o.transport.frame_of(e)));
            // #201: a windowed audition that does not loop ends in the engine: the seek arms the
            // window end (session frames), the engine delivers every frame before it and pauses
            // there. A looped audition wraps instead, so it sends no end.
            let end_frames = audition
                .as_ref()
                .filter(|a| !a.win.looped)
                .and_then(|a| a.win.end)
                .map(|e| o.transport.frame_of(e));
            let mut sought = false;
            if respeed {
                // P3-20b: the speed changes in the running stream, as one seek (to the position of
                // the call, else to where the transport is)
                let frame = match position {
                    Some(p) => o.transport.frame_of(p),
                    None => o.transport.playhead().frame_at(host_now_ns()),
                };
                o.seek_epoch = epoch;
                o.transport
                    .set_speed_window(speed, frame, Some(epoch), loop_frames, end_frames);
                sought = true;
            } else if let Some(p) = position {
                let frame = o.transport.frame_of(p);
                seek_tagged_window(o, frame, epoch, loop_frames, end_frames);
                sought = true;
            } else if o.transport.loop_armed() || o.transport.end_armed() {
                // #166, #201: a play without a position sends no seek; end a loop or a window end
                // that a past audition armed, so it cannot wrap or re-end this play (one seek to
                // where the transport is)
                let frame = o.transport.playhead().frame_at(host_now_ns());
                seek_tagged(o, frame, epoch);
                sought = true;
            }
            o.transport.play();
            let rate = o.transport.sample_rate();
            (st.sender_gen, rate, counter, epoch, sought, agen)
        };
        if sought {
            wait_seek_taken(&counter, epoch);
        }
        // wait for the first playing block, without holding the lock
        let mut ready = false;
        let mut heard_ms = Value::Null;
        while t0.elapsed() < START_WAIT {
            {
                let st = lock(&self.host().st);
                match &st.open {
                    Some(o) => {
                        // P3-15f: playing at the position of this play (its seek taken); P3-20b: at its speed
                        let ph = o.transport.playhead();
                        if ph.playing && ph.seek_tag >= o.seek_epoch && ph.slowdown == speed.slowdown() {
                            ready = true;
                            // P3-20b: when the first sample of that block is heard (the callback's
                            // playback time; 0 offline)
                            if ph.host_time_ns > 0 {
                                heard_ms = json!(ph.host_time_ns.saturating_sub(t0_ns) as f64 / 1e6);
                            }
                            break;
                        }
                    }
                    None => break,
                }
            }
            sleep(Duration::from_millis(1));
        }
        let start_ms = t0.elapsed().as_secs_f64() * 1e3;
        if audition.as_ref().is_some_and(|a| a.win.end.is_some()) {
            spawn_audition(&self.host().st, agen);
        }
        // the first message already shows the position after the seek (and `playing` when it plays)
        if let Some(p) = progress {
            spawn_sender(&self.host().st, gen_n, sid.to_string(), p);
        }
        let mut st = lock(&self.host().st);
        st.last_start_ms = Some(start_ms);
        let o = st
            .open
            .as_ref()
            .ok_or_else(|| AppError::internal("the audio device went away while starting"))?;
        Ok(json!({
            "sid": sid,
            "device": o.key.json(),
            "rate": rate,
            "channels": o.key.channels,
            "frames": o.transport.session_frames(),
            "duration": o.transport.session_frames() as f64 / f64::from(rate),
            "position": position.unwrap_or_else(|| position_s(&o.transport)),
            "epoch": st.epoch,
            "ready": ready,
            "start_ms": start_ms,
            "open_ms": o.open_ms,
            "master": o.transport.master_gain(),
            "master_db": st.master_db,
            "tracks": o.plan.names.len(),
            "names": o.plan.names,
            "channels": o.transport.track_channels(),
            "note": st.note,
            "speed": o.transport.speed().rate(),
            "heard_ms": heard_ms,
            "audition": audition.as_ref().map_or(Value::Null, Audition::json),
        }))
    }

    /// `transport_stop`: pause; the position stays. With `close` the transport is released
    /// too (its files and threads). Answers `{open, playing: false, frame, rate, pos, epoch}`;
    /// `pos` is the position at which the callback stopped.
    pub fn transport_stop(&self, close: bool) -> Value {
        let host = self.host();
        {
            let mut st = lock(&host.st);
            st.epoch += 1;
            st.lost = None;
            if st.audition.take().is_some() {
                // P4-40d: the audition ends; mute and solo are the page's again
                st.audition_gen += 1;
                restore_mix(&st);
            }
            match &st.open {
                Some(o) => o.transport.pause(),
                None => {
                    return json!({"open": false, "playing": false, "frame": 0, "rate": 0, "pos": 0.0, "epoch": st.epoch});
                }
            }
        }
        // the callback publishes the stopped position within one block
        let end = Instant::now() + Duration::from_millis(100);
        let mut st;
        loop {
            st = lock(&host.st);
            let stopped = st.open.as_ref().is_none_or(|o| !o.transport.playhead().playing);
            if stopped || Instant::now() > end {
                break;
            }
            drop(st);
            sleep(Duration::from_millis(1));
        }
        let (frame, rate) = st.open.as_ref().map_or((0, 0), |o| {
            let ph = o.transport.playhead();
            (ph.frame, ph.sample_rate)
        });
        let epoch = st.epoch;
        if close {
            st.open = None;
        }
        json!({"open": !close, "playing": false, "frame": frame, "rate": rate,
            "pos": if rate > 0 { frame as f64 / f64::from(rate) } else { 0.0 }, "epoch": epoch})
    }

    /// `transport_seek`: move to `position` seconds, playing or paused. 409 without an open
    /// transport. Answers `{frame, rate, pos, epoch}`: the target frame and the epoch the
    /// messages after the seek carry.
    ///
    /// # Errors
    /// 400 when `position` is negative or not finite; 409 when no transport is open.
    pub fn transport_seek(&self, position: f64) -> Result<Value, AppError> {
        if !position.is_finite() || position < 0.0 {
            return Err(AppError::bad("position must be a number of seconds, 0 or more"));
        }
        // P3-15c: the wait for the callback runs without the host lock. P3-15f: the epoch grows at
        // the request and tags the seek; a message carries it only when the callback has taken
        // the seek ([`heard_epoch`]), however late this thread wakes up.
        let (rate, frame, counter, epoch) = {
            let mut st = lock(&self.host().st);
            st.epoch += 1;
            let epoch = st.epoch;
            let Some(o) = st.open.as_mut() else {
                return Err(AppError::conflict("the transport is not open: press play first"));
            };
            let rate = o.transport.sample_rate();
            let frame = (position * f64::from(rate)).round() as u64;
            let counter = o.transport.callback_counter();
            seek_tagged(o, frame, epoch);
            (rate, frame, counter, epoch)
        };
        wait_seek_taken(&counter, epoch);
        Ok(json!({"frame": frame, "rate": rate, "pos": frame as f64 / f64::from(rate), "epoch": epoch}))
    }

    /// `transport_mix`: mute and solo as `{track name: bool}` objects (a missing name is
    /// false). The state stays when no transport is open and applies at the next open.
    /// Answers `{applied, tracks_audible, latency_ms, t_ns}` (`t_ns`: when the change was made, on the clock of the playhead messages): while the transport plays, `latency_ms`
    /// is the time from this change to the moment the first block with the new gains is heard.
    ///
    /// # Errors
    /// 400 when `mute` or `solo` is neither null nor an object `{track: bool}`; 500 when the open
    /// transport refuses the new mix.
    pub fn transport_mix(&self, mute: &Value, solo: &Value) -> Result<Value, AppError> {
        let read = |v: &Value, what: &str| -> Result<HashMap<String, bool>, AppError> {
            match v {
                Value::Null => Ok(HashMap::new()),
                Value::Object(m) => Ok(m
                    .iter()
                    .filter_map(|(k, b)| b.as_bool().map(|b| (k.clone(), b)))
                    .collect()),
                _ => Err(AppError::bad(format!("`{what}` must be an object {{track: bool}}"))),
            }
        };
        let (mute, solo) = (read(mute, "mute")?, read(solo, "solo")?);
        let mut st = lock(&self.host().st);
        st.mute = mute;
        st.solo = solo;
        let set_t_ns = host_now_ns();
        let mut audible = None;
        let mut playing = false;
        if let Some(o) = &st.open {
            let (m, s) = mix_lists(&st, &o.plan.names);
            o.transport.set_mix(&m, &s).map_err(dev_err)?;
            let any_solo = s.iter().any(|x| *x);
            audible = Some(m.iter().zip(&s).filter(|(m, s)| **s || (!any_solo && !**m)).count());
            playing = o.transport.playhead().playing;
        }
        drop(st);
        // while the transport plays, wait for the callback that applies the change (one block,
        // 5 ms at 256 frames) and report how long it took until that block is heard
        let mut latency_ms = Value::Null;
        if playing {
            let end = Instant::now() + Duration::from_millis(60);
            while Instant::now() < end {
                let st = lock(&self.host().st);
                let Some(o) = &st.open else { break };
                if let Some(ns) = o.transport.mix_latency_ns() {
                    latency_ms = json!(ns as f64 / 1e6);
                    break;
                }
                drop(st);
                sleep(Duration::from_micros(500));
            }
        }
        Ok(json!({"applied": audible.is_some(), "tracks_audible": audible, "latency_ms": latency_ms, "t_ns": set_t_ns}))
    }

    /// `transport_clicks`: the metronome. `enabled`, `volume` (0 to 1) and `pitches`
    /// (`{role: Hz}`) are optional; a missing one stays. Applies to the open transport at
    /// once and to every later open. Answers `{enabled, volume, applied}`.
    ///
    /// # Errors
    /// 400 when `volume` is outside 0 to 1, `pitches` is not an object of `{role: Hz}` with Hz
    /// above 0; 500 when the open transport still refuses the settings after about 100 ms of
    /// retries.
    pub fn transport_clicks(
        &self,
        enabled: Option<bool>,
        volume: Option<f64>,
        pitches: Option<&Value>,
    ) -> Result<Value, AppError> {
        if volume.is_some_and(|v| !v.is_finite() || !(0.0..=1.0).contains(&v)) {
            return Err(AppError::bad("volume must be between 0 and 1"));
        }
        let mut new_pitches = None;
        match pitches {
            None | Some(Value::Null) => {}
            Some(Value::Object(m)) => {
                let mut p = HashMap::new();
                for (role, hz) in m {
                    let hz = hz
                        .as_f64()
                        .filter(|f| f.is_finite() && *f > 0.0)
                        .ok_or_else(|| AppError::bad(format!("pitch of {role} must be a number of Hz above 0")))?;
                    p.insert(role.clone(), hz as f32);
                }
                new_pitches = Some(p);
            }
            Some(_) => return Err(AppError::bad("`pitches` must be an object {role: Hz}")),
        }
        let mut st = lock(&self.host().st);
        if let Some(e) = enabled {
            st.clicks.enabled = e;
        }
        if let Some(v) = volume {
            st.clicks.volume = v as f32;
        }
        if let Some(p) = new_pitches {
            st.clicks.pitches = p;
        }
        let applied = st.open.is_some();
        if applied {
            let st = &mut *st;
            let clicks = st.clicks.clone();
            if let Some(o) = st.open.as_mut() {
                let pitches: Vec<f32> = o.plan.roles.iter().map(|r| clicks.pitch(r)).collect();
                let s = ClickSettings {
                    enabled: clicks.enabled,
                    volume: clicks.volume,
                    pitches: Some(pitches),
                };
                // the update queue holds 8 banks and drains once per callback: retry briefly
                let mut r = o.transport.set_clicks(&s);
                for _ in 0..50 {
                    if r.is_ok() {
                        break;
                    }
                    sleep(Duration::from_millis(2));
                    r = o.transport.set_clicks(&s);
                }
                r.map_err(dev_err)?;
            }
        }
        Ok(json!({"enabled": st.clicks.enabled, "volume": st.clicks.volume, "applied": applied}))
    }

    /// `transport_status`: `{open, sid, playing, speed, requested, frame, rate, pos, t_ns, epoch,
    /// underruns, underrun_frames, stream_errors, callbacks, peak, master, device, tracks,
    /// reopens, lost, mix_latency_ms, mix_latency_blocks, last_start_ms, clicks, hits, audible}`. `pos` is extrapolated to
    /// `t_ns`. Without an open transport only `open: false` and the host fields.
    pub fn transport_status(&self) -> Value {
        self.load_audio_settings();
        let st = lock(&self.host().st);
        let mut m = Map::new();
        m.insert("open".into(), json!(st.open.is_some()));
        m.insert("epoch".into(), json!(st.epoch));
        m.insert("t_ns".into(), json!(host_now_ns()));
        m.insert("master".into(), json!(st.master));
        m.insert("reopens".into(), json!(st.reopens));
        m.insert("lost".into(), st.lost.as_ref().map_or(Value::Null, |l| json!(l.error)));
        m.insert("last_start_ms".into(), json!(st.last_start_ms));
        m.insert("last_reopen".into(), json!(st.last_reopen));
        m.insert(
            "clicks".into(),
            json!({"enabled": st.clicks.enabled, "volume": st.clicks.volume}),
        );
        m.insert("master_db".into(), json!(st.master_db));
        m.insert("choice".into(), json!(st.choice));
        m.insert("note".into(), json!(st.note));
        // P4-40d: the running audition (its tracks, window and loop), else null
        m.insert(
            "audition".into(),
            st.audition.as_ref().map_or(Value::Null, Audition::json),
        );
        if let Some(o) = &st.open {
            let ph = o.transport.playhead();
            let now = host_now_ns();
            let frame = ph.frame_at(now);
            let s = o.transport.stats();
            // P3-15f: the epoch of this position (see the playhead messages)
            m.insert("epoch".into(), json!(heard_epoch(&st, o, &ph)));
            m.insert("t_ns".into(), json!(now));
            m.insert("sid".into(), json!(o.plan.sid));
            m.insert("playing".into(), json!(ph.playing));
            m.insert("speed".into(), json!(ph.rate()));
            m.insert("requested".into(), json!(o.transport.is_playing()));
            m.insert("frame".into(), json!(frame));
            m.insert("rate".into(), json!(ph.sample_rate));
            m.insert("pos".into(), json!(frame as f64 / f64::from(ph.sample_rate)));
            // #166: the tag of the last seek the callback took; with an engine-side loop it does
            // not grow per pass (the old watcher sought back on every wrap)
            m.insert("seek_tag".into(), json!(ph.seek_tag));
            m.insert("underruns".into(), json!(s.underruns));
            m.insert("underrun_frames".into(), json!(s.underrun_frames));
            m.insert("stream_errors".into(), json!(s.stream_errors));
            m.insert("stream_xruns".into(), json!(s.stream_xruns));
            m.insert("stream_warnings".into(), json!(s.stream_warnings));
            m.insert("callbacks".into(), json!(s.callbacks));
            m.insert("peak".into(), json!(s.peak));
            m.insert("master".into(), json!(o.transport.master_gain()));
            m.insert("device".into(), o.key.json());
            m.insert("tracks".into(), json!(o.plan.names.len()));
            m.insert("names".into(), json!(o.plan.names));
            m.insert("channels".into(), json!(o.transport.track_channels()));
            m.insert(
                "vols".into(),
                json!(o.plan.vols.iter().map(|(d, p)| json!([d, p])).collect::<Vec<_>>()),
            );
            m.insert(
                "hits".into(),
                json!(o.plan.specs.iter().map(|s| s.hits.len()).sum::<usize>()),
            );
            let (mute, solo) = mix_lists(&st, &o.plan.names);
            let any_solo = solo.iter().any(|x| *x);
            let audible: Vec<&String> = o
                .plan
                .names
                .iter()
                .enumerate()
                .filter(|(i, _)| solo[*i] || (!any_solo && !mute[*i]))
                .map(|(_, n)| n)
                .collect();
            m.insert("audible".into(), json!(audible));
            m.insert("open_ms".into(), json!(o.open_ms));
            // P4-7a: the layer voices
            m.insert("layer_voices".into(), json!(s.layer_voices));
            m.insert("layer_last_start".into(), json!(s.layer_last_start));
            m.insert(
                "layers".into(),
                json!(
                    o.plan
                        .names
                        .iter()
                        .zip(&o.plan.layers)
                        .filter(|(_, l)| l.is_some())
                        .map(|(n, l)| json!({"name": n, "voices": l.as_ref().map_or(0, |l| l.starts.len())}))
                        .collect::<Vec<_>>()
                ),
            );
            m.insert(
                "mix_latency_ms".into(),
                o.transport
                    .mix_latency_ns()
                    .map_or(Value::Null, |n| json!(n as f64 / 1e6)),
            );
            m.insert(
                "mix_latency_blocks".into(),
                o.transport.mix_latency_blocks().map_or(Value::Null, |b| json!(b)),
            );
        }
        Value::Object(m)
    }

    /// `track_set_mix` (P3-16): the playback volume (dB, [`MIN_DB`] and below is off, up to
    /// [`MAX_DB`]) and pan (-1 to +1) of one track; a missing value stays. Kept in the project
    /// (`volume_db`, `pan`; marks it dirty) and applied to the open transport of this project at
    /// the next callback. Answers `{volume_db, pan, applied}`.
    ///
    /// # Errors
    /// 400 when `volume_db` or `pan` is not a finite number; 404 when `sid` is not an open project
    /// or `name` is not a track; 409 when the project is open read-only; 500 when the open
    /// transport refuses the new volume or pan.
    pub fn track_set_mix(
        &self,
        sid: &str,
        name: &str,
        volume_db: Option<f64>,
        pan: Option<f64>,
    ) -> Result<Value, AppError> {
        if volume_db.is_some_and(|v| !v.is_finite()) || pan.is_some_and(|v| !v.is_finite()) {
            return Err(AppError::bad("volume_db and pan must be numbers"));
        }
        let (db, pn) = {
            let op = self.inner.registry.get_writable(sid)?;
            let mut g = lock(&op);
            let tr = g.track_mut(name)?;
            if let Some(v) = volume_db {
                tr.volume_db = Some(clamp_db(v));
            }
            if let Some(p) = pan {
                tr.pan = Some(clamp_pan(p));
            }
            let out = (tr.volume_db.map_or(0.0, clamp_db), tr.pan.map_or(0.0, clamp_pan));
            g.mark_dirty();
            out
        };
        let mut st = lock(&self.host().st);
        let mut applied = false;
        if let Some(o) = st.open.as_mut().filter(|o| o.plan.sid == sid)
            && let Some(i) = o.plan.names.iter().position(|n| n == name)
        {
            o.transport.set_volume_pan(i, db as f32, pn as f32).map_err(dev_err)?;
            o.plan.vols[i] = (db, pn);
            applied = true;
        }
        if let Some(l) = st.lost.as_mut().filter(|l| l.plan.sid == sid)
            && let Some(i) = l.plan.names.iter().position(|n| n == name)
        {
            l.plan.vols[i] = (db, pn);
        }
        Ok(json!({"volume_db": db, "pan": pn, "applied": applied}))
    }

    /// `transport_master` (P3-16): the master volume in dB (clamped to [`MIN_DB`] to
    /// [`MAX_DB`]), applied to the open transport at once and to every later open, kept in the
    /// app settings. It comes before the master gain of `DRUMSPIKE_MASTER_GAIN`. Answers
    /// `{volume_db, applied}`.
    ///
    /// # Errors
    /// 400 when `volume_db` is not a finite number; otherwise the error of writing the app
    /// settings file (500 when it cannot be written).
    pub fn transport_master(&self, volume_db: f64) -> Result<Value, AppError> {
        if !volume_db.is_finite() {
            return Err(AppError::bad("volume_db must be a number"));
        }
        self.load_audio_settings();
        let db = clamp_db(volume_db);
        let (choice, applied) = {
            let mut st = lock(&self.host().st);
            st.master_db = db;
            if let Some(o) = &st.open {
                o.transport.set_master_volume_db(db as f32);
            }
            (st.choice.clone(), st.open.is_some())
        };
        self.save_audio_settings(choice.as_deref(), db)?;
        Ok(json!({"volume_db": db, "applied": applied}))
    }

    /// `outputs_list` (P3-16): `{devices: [{name, rate, channels, default}], choice, current,
    /// note, null_output}`. `choice` is the saved device name (`null`: the system default),
    /// `current` the device the open transport plays on (`null` when none is open), `note` why
    /// it does not (or, with no open transport, will not) play on the choice. The `CoreAudio` query
    /// runs without the host lock.
    pub fn outputs_list(&self) -> Value {
        self.load_audio_settings();
        let (output, test_list) = {
            let st = lock(&self.host().st);
            (st.output.clone(), st.device_list.clone())
        };
        let null_output = matches!(output, Output::Null { .. });
        let (list, default_name): (Vec<DeviceKey>, Option<String>) = if null_output {
            let l = test_list.unwrap_or_default();
            let d = l.first().map(|k| k.name.clone());
            (l, d)
        } else {
            let l: Vec<DeviceKey> = output_devices()
                .map(|v| v.iter().map(DeviceKey::of).collect())
                .unwrap_or_default();
            (l, default_output_info().ok().map(|d| d.name))
        };
        let st = lock(&self.host().st);
        // with no open transport the message follows the list of now; an open one keeps the note of its open
        let note = if st.open.is_some() {
            st.note.clone()
        } else {
            let names: Vec<String> = list.iter().map(|k| k.name.clone()).collect();
            st.choice.as_deref().and_then(|c| resolve_choice(Some(c), &names).1)
        };
        let devices: Vec<Value> = list
            .iter()
            .map(|k| {
                let mut v = k.json();
                v["default"] = json!(Some(&k.name) == default_name.as_ref());
                v
            })
            .collect();
        json!({
            "devices": devices,
            "choice": st.choice,
            "current": st.open.as_ref().map(|o| o.key.json()),
            "note": note,
            "null_output": null_output,
            "master_db": st.master_db,
        })
    }

    /// `output_select` (P3-16): choose the output device by name; `None` is the system default.
    /// Kept in the app settings. An open transport reopens on it at the same position and
    /// state; a missing device gives the system default and the `note`. Answers like
    /// [`Service::outputs_list`].
    ///
    /// # Errors
    /// 400 when `device` is an empty or blank name; otherwise the error of writing the app
    /// settings file (500 when it cannot be written).
    pub fn output_select(&self, device: Option<&str>) -> Result<Value, AppError> {
        if device.is_some_and(|d| d.trim().is_empty()) {
            return Err(AppError::bad("device must be a device name or null"));
        }
        self.load_audio_settings();
        let master_db = {
            let mut st = lock(&self.host().st);
            st.choice = device.map(str::to_string);
            if st.open.is_some() {
                st.last_reopen = Some(format!("output choice {:?}", st.choice));
                reopen(&mut st, "output choice");
            } else {
                st.note = output_for(&st).1;
            }
            st.master_db
        };
        self.save_audio_settings(device, master_db)?;
        Ok(self.outputs_list())
    }

    /// Release the transport when it plays this project (the project closes or another
    /// one takes over). `project_close` (P3-4a) calls it.
    pub fn transport_close_project(&self, sid: &str) {
        let mut st = lock(&self.host().st);
        if st.open.as_ref().is_some_and(|o| o.plan.sid == sid) {
            st.sender_gen += 1;
            st.open = None;
        }
        if st.lost.as_ref().is_some_and(|l| l.plan.sid == sid) {
            st.lost = None;
        }
    }
}

/// Whether the watcher thread is active (tests).
pub fn watcher_active(svc: &Service) -> bool {
    lock(&svc.host().st).watcher_started
}

#[cfg(test)]
mod tests;
