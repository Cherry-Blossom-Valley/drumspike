//! Output devices: the system's default output or a named output device through cpal, and a
//! null device that renders at real-time pace into nothing (tests, CI, the e2e bridge).
//!
//! Sample-rate policy (controller decision 2026-10-11): the stream opens at the device's
//! current rate, with its current channel count and sample format. The app never changes
//! the device rate; a track at another rate is resampled for playback by the reader.
//!
//! P3-16: [`Output::Default`] builds the stream on cpal's default device, which `CoreAudio`
//! plays through its `DefaultOutput` unit: that unit moves to a new system default by itself
//! (cpal reports `DeviceChanged`), but keeps the stream format (rate, channels) of the device
//! it was opened on. [`Output::Device`] pins the stream to one device by name (HAL output
//! unit); when that device goes away, cpal reports `DeviceNotAvailable` and the stream stops.
//! The app's watcher (`crates/app/src/transport.rs`) reopens in both cases.
//!
//! P3-16, no-audio finding: on arm64-mac, building a stream on "Microsoft Teams Audio" (the Teams
//! HAL driver while Teams does not run) or on "Pro Tools Aggregate I/O" never returns:
//! `AudioUnitSetProperty(CurrentDevice)` waits in `CreateIOProcID` for an answer of
//! `coreaudiod` that does not come, and the dispose of such a unit waits the same way in
//! `DestroyIOProcID`. So the stream is built and dropped on a worker thread with a time limit
//! ([`OPEN_LIMIT`], [`CLOSE_LIMIT`]). A device whose worker is still blocked is refused at once
//! ([`TransportError::NoAnswer`]) until the worker returns, so retries do not pile up threads.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use super::clock::{StampMap, host_now_ns};
use super::mixer::Engine;
use super::{Shared, TransportError};

/// Where the transport plays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Output {
    /// The system's default output device (cpal); follows a change of the default.
    Default,
    /// The output device of this name (as [`output_devices`] lists it), pinned (P3-16).
    Device {
        /// The device name.
        name: String,
    },
    /// No device: a thread renders blocks at real-time pace and drops them. `sample_rate`
    /// `None` takes the first track's rate.
    Null {
        /// The rate the null device runs at.
        sample_rate: Option<u32>,
    },
}

impl Output {
    /// `DRUMSPIKE_AUDIO=null` selects the null device, anything else the default device.
    pub fn from_env() -> Output {
        Output::from_value(std::env::var("DRUMSPIKE_AUDIO").ok().as_deref())
    }

    /// The choice for one value of `DRUMSPIKE_AUDIO` (see [`Output::from_env`]).
    pub fn from_value(v: Option<&str>) -> Output {
        match v {
            Some(s) if s.eq_ignore_ascii_case("null") => Output::Null { sample_rate: None },
            _ => Output::Default,
        }
    }
}

/// The output for a saved choice (`None`: the system default) when the devices `available`
/// are connected: the named device when it is in the list, else the system default and the
/// text that says so (P3-16: a missing saved device falls back with a message).
pub fn resolve_choice(choice: Option<&str>, available: &[String]) -> (Output, Option<String>) {
    match choice {
        None => (Output::Default, None),
        Some(name) if available.iter().any(|a| a == name) => (Output::Device { name: name.to_string() }, None),
        Some(name) => (
            Output::Default,
            Some(format!(
                "The output device \"{name}\" is not connected. Drum Spike plays on the system default."
            )),
        ),
    }
}

/// How long the build and start of a device stream may take before the device counts as not
/// answering.
pub const OPEN_LIMIT: Duration = Duration::from_secs(5);
/// How long the stop and dispose of a device stream may take; after it the stream is left to
/// its worker thread.
pub const CLOSE_LIMIT: Duration = Duration::from_secs(2);

/// The devices whose worker thread is blocked in `CoreAudio` now.
static BLOCKED: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

fn blocked() -> std::sync::MutexGuard<'static, BTreeSet<String>> {
    BLOCKED.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The error of a device that does not answer.
fn no_answer(name: &str) -> TransportError {
    TransportError::NoAnswer(format!(
        "the output device \"{name}\" does not answer (CoreAudio). Choose another output in Drum Spike > Settings or in the macOS Sound settings."
    ))
}

/// Whether a worker thread of device `name` is still blocked in `CoreAudio`.
pub fn is_blocked(name: &str) -> bool {
    blocked().contains(name)
}

/// Run `f` for device `name` on a worker thread and wait up to `limit`. While a worker of the
/// same device is still blocked, refuse at once (`wait_busy` false) or run without waiting for
/// it (`wait_busy` true: for a close, which must happen somewhere). On a timeout the worker
/// keeps the device marked as blocked until `f` returns; its late result is dropped there.
fn guarded<T: Send + 'static>(
    name: &str,
    limit: Duration,
    wait_busy: bool,
    f: impl FnOnce() -> T + Send + 'static,
) -> Result<T, TransportError> {
    let busy = !blocked().insert(name.to_string());
    if busy && !wait_busy {
        return Err(no_answer(name));
    }
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let key = name.to_string();
    let spawned = std::thread::Builder::new()
        .name("drumspike-device-io".into())
        .spawn(move || {
            let r = f();
            if !busy {
                blocked().remove(&key);
            }
            // a late result (the caller gave up) is dropped here, on this thread
            let _ = tx.send(r);
        });
    if let Err(e) = spawned {
        if !busy {
            blocked().remove(name);
        }
        return Err(TransportError::Device(format!("device thread: {e}")));
    }
    if busy {
        return Err(no_answer(name));
    }
    rx.recv_timeout(limit).map_err(|_| no_answer(name))
}

/// The name of the device an output opens now (`None`: the system default), or the error.
pub fn target_name(device: Option<&str>) -> Result<String, TransportError> {
    match device {
        Some(n) => Ok(n.to_string()),
        None => find_device(None).map(|d| device_name(&d)),
    }
}

/// What the device side reports at open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    /// Device name (`"null"` for the null device).
    pub name: String,
    /// The rate the stream runs at (the device's current rate).
    pub sample_rate: u32,
    /// Output channels of the stream.
    pub channels: u16,
    /// The requested callback size in frames, `None` when the device default is used.
    pub buffer_frames: Option<u32>,
}

/// The name cpal gives a device (the `CoreAudio` device name).
fn device_name(dev: &cpal::Device) -> String {
    dev.description()
        .map_or_else(|_| "output".into(), |d| d.name().to_string())
}

/// The device of `name` (`None`: the system default), or the error that names what is missing.
fn find_device(name: Option<&str>) -> Result<cpal::Device, TransportError> {
    let host = cpal::default_host();
    match name {
        None => host
            .default_output_device()
            .ok_or_else(|| TransportError::Device("no default output device".into())),
        Some(n) => {
            // The ALSA host reports the default under a pseudo-device name ("Default Audio
            // Device") that its enumeration never lists; the default answers to its own name.
            if let Some(def) = host.default_output_device()
                && device_name(&def) == n
            {
                return Ok(def);
            }
            host.output_devices()
                .map_err(|e| TransportError::Device(e.to_string()))?
                .find(|d| device_name(d) == n)
                .ok_or_else(|| TransportError::Device(format!("the output device \"{n}\" is not connected")))
        }
    }
}

/// Name, current rate and channels of a device, without opening a stream.
fn info_of(dev: &cpal::Device) -> Result<DeviceInfo, TransportError> {
    let cfg = dev
        .default_output_config()
        .map_err(|e| TransportError::Device(e.to_string()))?;
    Ok(DeviceInfo {
        name: device_name(dev),
        sample_rate: cfg.sample_rate(),
        channels: cfg.channels(),
        buffer_frames: None,
    })
}

/// The device's current rate and channel count, without opening a stream.
pub fn default_output_rate() -> Result<(u32, u16), TransportError> {
    output_info(None).map(|i| (i.sample_rate, i.channels))
}

/// The default output device as the system reports it now: name, current rate, channels. The
/// app polls it to see a change of the output (P3-5b); it opens no stream.
pub fn default_output_info() -> Result<DeviceInfo, TransportError> {
    output_info(None)
}

/// [`default_output_info`] for the device of `name` (`None`: the system default).
pub fn output_info(name: Option<&str>) -> Result<DeviceInfo, TransportError> {
    info_of(&find_device(name)?)
}

/// Every output device the system lists now, with its current rate and channels (P3-16:
/// the Settings list). A device that does not answer its configuration is left out.
pub fn output_devices() -> Result<Vec<DeviceInfo>, TransportError> {
    let host = cpal::default_host();
    let devs = host
        .output_devices()
        .map_err(|e| TransportError::Device(e.to_string()))?;
    let mut list: Vec<DeviceInfo> = devs.filter_map(|d| info_of(&d).ok()).collect();
    // The ALSA host's default is a pseudo-device ("Default Audio Device") that the
    // enumeration never lists; add it, so the default is in the list and selectable by name.
    // Where the default is a listed device (CoreAudio, PipeWire-as-default) this adds nothing.
    if let Some(def) = host.default_output_device()
        && let Ok(info) = info_of(&def)
        && !list.iter().any(|d| d.name == info.name)
    {
        list.push(info);
    }
    Ok(list)
}

/// How the transport treats an error of the device stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    /// The stream cannot go on (device gone, stream invalid, backend error): the app reopens.
    Fatal,
    /// A buffer under- or overrun: a glitch, the stream goes on.
    Xrun,
    /// The stream goes on (route changed and followed, device busy, real-time scheduling refused).
    Notice,
}

/// Sort a cpal error kind into [`ErrorClass`].
pub fn classify_error(kind: cpal::ErrorKind) -> ErrorClass {
    use cpal::ErrorKind as K;
    match kind {
        K::Xrun => ErrorClass::Xrun,
        K::DeviceChanged | K::RealtimeDenied | K::DeviceBusy => ErrorClass::Notice,
        _ => ErrorClass::Fatal,
    }
}

/// A running output; dropping it stops the stream or the null thread.
pub enum Running {
    /// A cpal stream and its device name; the drop runs on a worker thread with
    /// [`CLOSE_LIMIT`].
    Cpal(Option<cpal::Stream>, String),
    /// The null device thread.
    Null {
        /// Set to stop the thread.
        stop: Arc<AtomicBool>,
        /// The thread.
        thread: Option<JoinHandle<()>>,
    },
}

impl Drop for Running {
    fn drop(&mut self) {
        if let Running::Cpal(stream, name) = self
            && let Some(st) = stream.take()
        {
            let _ = guarded(name, CLOSE_LIMIT, true, move || drop(st));
        }
        if let Running::Null { stop, thread } = self {
            stop.store(true, Ordering::Release);
            if let Some(t) = thread.take() {
                let _ = t.join();
            }
        }
    }
}

/// Start the output device `device` (`None`: the system default) at its current rate with
/// `engine` in the callback. The build and start run on a worker thread with [`OPEN_LIMIT`]
/// (see the module docs).
pub(crate) fn start_cpal(
    engine: Engine,
    shared: Arc<Shared>,
    buffer_frames: u32,
    device: Option<&str>,
) -> Result<(Running, DeviceInfo), TransportError> {
    let key = target_name(device)?;
    let device = device.map(str::to_string);
    let (stream, info) = guarded(&key, OPEN_LIMIT, false, move || {
        build_cpal(engine, shared, buffer_frames, device.as_deref())
    })??;
    let name = info.name.clone();
    Ok((Running::Cpal(Some(stream), name), info))
}

/// The body of [`start_cpal`] on its worker thread.
fn build_cpal(
    mut engine: Engine,
    shared: Arc<Shared>,
    buffer_frames: u32,
    device: Option<&str>,
) -> Result<(cpal::Stream, DeviceInfo), TransportError> {
    let dev = find_device(device)?;
    let name = device_name(&dev);
    let cfg = dev
        .default_output_config()
        .map_err(|e| TransportError::Device(e.to_string()))?;
    if cfg.sample_format() != cpal::SampleFormat::F32 {
        return Err(TransportError::Device(format!(
            "{name}: sample format {:?} is not supported (f32 only)",
            cfg.sample_format()
        )));
    }
    let buffer = match cfg.buffer_size() {
        cpal::SupportedBufferSize::Range { min, max } if (*min..=*max).contains(&buffer_frames) => {
            cpal::BufferSize::Fixed(buffer_frames)
        }
        _ => cpal::BufferSize::Default,
    };
    let channels = cfg.channels();
    let config = cpal::StreamConfig {
        channels,
        sample_rate: cfg.sample_rate(),
        buffer_size: buffer,
    };
    let ch = channels as usize;
    let errs = shared.clone();
    // #471: the block plays at the device's playback instant, mapped onto the host clock (not
    // at the host clock read here, which carries the callback thread's wake-up jitter).
    let mut stamps = StampMap::new();
    let stream = dev
        .build_output_stream(
            config,
            move |data: &mut [f32], info: &cpal::OutputCallbackInfo| {
                let ts = info.timestamp();
                let host = host_now_ns();
                let play = stamps.host_play_ns(ts.callback.as_nanos() as i128, ts.playback.as_nanos() as i128, host);
                engine.render(data, ch, play);
            },
            move |e: cpal::Error| {
                let counter = match classify_error(e.kind()) {
                    ErrorClass::Fatal => &errs.stats.stream_errors,
                    ErrorClass::Xrun => &errs.stats.stream_xruns,
                    ErrorClass::Notice => &errs.stats.stream_warnings,
                };
                counter.fetch_add(1, Ordering::Relaxed);
            },
            None,
        )
        .map_err(|e| TransportError::Device(format!("{name}: {e}")))?;
    stream
        .play()
        .map_err(|e| TransportError::Device(format!("{name}: {e}")))?;
    let info = DeviceInfo {
        name,
        sample_rate: cfg.sample_rate(),
        channels,
        buffer_frames: matches!(buffer, cpal::BufferSize::Fixed(_)).then_some(buffer_frames),
    };
    Ok((stream, info))
}

/// Start the null device: blocks of `buffer_frames` at real-time pace at `sample_rate`.
pub(crate) fn start_null(
    mut engine: Engine,
    sample_rate: u32,
    buffer_frames: u32,
) -> Result<(Running, DeviceInfo), TransportError> {
    let stop = Arc::new(AtomicBool::new(false));
    let s2 = stop.clone();
    let n = buffer_frames.max(1) as usize;
    let thread = std::thread::Builder::new()
        .name("drumspike-null-audio".into())
        .spawn(move || {
            let mut buf = vec![0.0f32; 2 * n];
            let period = Duration::from_secs_f64(n as f64 / f64::from(sample_rate));
            let start = Instant::now();
            let mut k: u32 = 0;
            while !s2.load(Ordering::Acquire) {
                engine.render(&mut buf, 2, host_now_ns());
                k += 1;
                let next = start + period * k;
                let now = Instant::now();
                if next > now {
                    std::thread::sleep(next - now);
                }
            }
        })
        .map_err(|e| TransportError::Device(format!("null device thread: {e}")))?;
    let info = DeviceInfo {
        name: "null".into(),
        sample_rate,
        channels: 2,
        buffer_frames: Some(buffer_frames),
    };
    Ok((
        Running::Null {
            stop,
            thread: Some(thread),
        },
        info,
    ))
}

#[cfg(test)]
mod tests;
