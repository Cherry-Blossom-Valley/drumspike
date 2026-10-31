//! Sample-exact native playback of a whole session (feature `audio`, P3-5a).
//!
//! The design follows the spike `cpal-transport` (`docs/migration/spikes.md`): one reader thread fills one lock-free ring
//! (rtrb) per track; the audio callback pops one block per track, mixes to stereo, adds the
//! hit clicks at their exact samples and applies the master gain.
//!
//! * [`reader`]: decode the WAV (header from [`crate::io::MappedWav`], blocks by positioned
//!   reads, so no page stays resident behind the playhead) at the native rate and format,
//!   fold with the track's [`MixMode`], resample to the device rate when the rates differ
//!   (rubato).
//! * [`mixer`]: the callback's [`mixer::Engine`]; mute and solo are per-track gains
//!   (atomics), applied without a restart; P3-16: volume and pan per track, the master
//!   volume, and the peak meters (atomics, see [`MeterSnapshot`]).
//! * [`clicks`]: the click of `static/app.js` (role pitch, volume, 2 ms attack, 30 ms decay)
//!   at the device sample of each hit.
//! * [`clock`]: the playhead from the callback's playback timestamps.
//! * [`device`]: the default or a named output device (cpal) at its current rate, and a null device.
//!
//! Sample-rate policy (controller decision 2026-10-11): the app never changes the device
//! rate. The stream opens at the device's current rate. A track at that rate plays bit for
//! bit (no resampling); a track at another rate is resampled for playback, and its clicks
//! go to the device sample `round(hit * device_rate / track_rate)`.
//!
//! [`Offline`] drives the same reader and engine synchronously, without a device and
//! without threads; the tests use it to compare renders bit for bit.
//!
//! Loop (audition, #166): a seek can carry a loop region (`Transport::seek_tagged_with_loop`,
//! `set_speed_loop`) in session frames. The reader wraps its read position at the region end
//! (a plain `TrackSource::seek` back to the start: the resampler is re-primed exactly like at
//! a seek, the rings never drain, so there is no gap), and the engine wraps its playhead
//! counter there, so clicks, layer voices and the clock follow the looped content. Every seek
//! without a region ends a loop. A block is never rendered across the wrap point.
//!
//! Half speed (P3-20b, Pro Tools Shift+Space): the session plays at half speed and one octave
//! down, without a time stretch. The reader resamples every track to twice the device rate
//! (the render rate of [`Speed::render_rate`]) and the device plays those frames at its own
//! rate. The engine counts output frames; the session position is the output frame divided
//! by the slowdown ([`Speed::slowdown`]), so the clock advances by half a frame per output
//! frame. The clicks sit at the hits mapped to the render rate ([`clicks::TrackClicks::onsets_half`]).
//! A speed change is a seek with a new speed ([`Transport::set_speed`]): the reader stops, the
//! callback drains the rings, the reader switches each source to the new render rate (a
//! resampler per rate, built once and kept) and refills from the target. Every public frame
//! (seek targets, the playhead, [`Transport::session_frames`]) is a session frame at the
//! device rate at both speeds. At normal speed nothing of this runs: the output is the
//! output before P3-20b, bit for bit (test `transport_normal_speed_render_equals_the_stored_output`).

pub mod clicks;
pub mod clock;
pub mod device;
pub mod layer;
pub mod mixer;
pub mod reader;

use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use rtrb::{Consumer, Producer, RingBuffer};

use crate::io::MixMode;
use clicks::TrackClicks;
use clock::{Clock, Playhead};
use device::{DeviceInfo, Output, Running};
use layer::LayerBank;
use mixer::{ClickBank, Engine};
use reader::{Reader, Residency, TrackSource};

/// Errors of the transport.
#[derive(Debug)]
pub enum TransportError {
    /// A track cannot be opened or decoded (file, format, resampler).
    Track(String),
    /// The output device cannot be opened.
    Device(String),
    /// A bad argument (no tracks, a list of the wrong length, a bad track index).
    Arg(String),
    /// P3-16: the output device does not answer (`CoreAudio` blocks); see [`device::OPEN_LIMIT`].
    NoAnswer(String),
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TransportError::Track(s) => write!(f, "track: {s}"),
            TransportError::Device(s) => write!(f, "audio device: {s}"),
            TransportError::Arg(s) => write!(f, "{s}"),
            TransportError::NoAnswer(s) => write!(f, "audio device: {s}"),
        }
    }
}

impl std::error::Error for TransportError {}

/// The playback speed (P3-20b).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Speed {
    /// The session at its own speed.
    #[default]
    Normal,
    /// Half speed, one octave down (Pro Tools "Half-speed playback"; no time stretch).
    Half,
}

impl Speed {
    /// Output frames per session frame: 1 at normal speed, 2 at half speed.
    pub fn slowdown(self) -> u32 {
        match self {
            Speed::Normal => 1,
            Speed::Half => 2,
        }
    }

    /// The speed of a slowdown: 2 is half speed, any other value normal speed.
    pub fn from_slowdown(n: u32) -> Speed {
        if n == 2 { Speed::Half } else { Speed::Normal }
    }

    /// The rate the reader resamples the tracks to: `device_rate` times the slowdown.
    pub fn render_rate(self, device_rate: u32) -> u32 {
        device_rate * self.slowdown()
    }

    /// Session seconds per second of playback: 1.0 or 0.5 (the value of the app's commands and
    /// messages).
    pub fn rate(self) -> f64 {
        1.0 / f64::from(self.slowdown())
    }

    /// The speed of a playback rate of [`Speed::rate`] (1.0 or 0.5); `None` for another value.
    pub fn from_rate(rate: f64) -> Option<Speed> {
        if rate == 1.0 {
            Some(Speed::Normal)
        } else if rate == 0.5 {
            Some(Speed::Half)
        } else {
            None
        }
    }
}

/// One track of the session.
#[derive(Debug, Clone)]
pub struct TrackSpec {
    /// The source file.
    pub path: PathBuf,
    /// How a multichannel file becomes mono (the track's `MixMode`).
    pub mode: MixMode,
    /// Hit samples at the track's own rate (`spike::hit_index` of the events).
    pub hits: Vec<u64>,
    /// Click pitch in Hz ([`clicks::role_pitch`] of the track's role by default).
    pub click_pitch: f32,
}

/// Transport settings.
#[derive(Debug, Clone, Copy)]
pub struct TransportConfig {
    /// Requested callback size in frames (256).
    pub buffer_frames: u32,
    /// Ring size per track in frames.
    pub ring_frames: usize,
    /// Frames every ring must hold before playback starts or resumes after a seek.
    pub prefill_frames: usize,
    /// How the reader keeps the source bytes.
    pub residency: Residency,
    /// Click volume (0.22 in `app.js`).
    pub click_volume: f32,
    /// Clicks on at open.
    pub clicks_on: bool,
    /// P3-20b: the speed at open (the reader builds the sources for its render rate).
    pub speed: Speed,
}

impl Default for TransportConfig {
    fn default() -> Self {
        TransportConfig {
            buffer_frames: 256,
            ring_frames: 1 << 17,
            prefill_frames: 1 << 15,
            residency: Residency::Pread,
            click_volume: clicks::DEFAULT_CLICK_VOLUME,
            clicks_on: true,
            speed: Speed::Normal,
        }
    }
}

/// Counters of the running transport.
#[derive(Debug, Default)]
pub struct Stats {
    pub(crate) callbacks: AtomicU64,
    pub(crate) rendered_frames: AtomicU64,
    pub(crate) frames_min: AtomicU64,
    pub(crate) frames_max: AtomicU64,
    pub(crate) underruns: AtomicU64,
    pub(crate) underrun_frames: AtomicU64,
    pub(crate) peak_bits: AtomicU32,
    pub(crate) reader_ns: AtomicU64,
    pub(crate) resample_ns: AtomicU64,
    pub(crate) stream_errors: AtomicU64,
    pub(crate) stream_xruns: AtomicU64,
    pub(crate) stream_warnings: AtomicU64,
    /// P4-7a: layer voices started while playing (audible tracks only).
    pub(crate) layer_voices: AtomicU64,
    /// P4-7a: the output frame of the last voice start (bits of an `i64`; `i64::MIN`: none yet).
    pub(crate) layer_last: AtomicU64,
}

/// A copy of the counters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StatsSnapshot {
    /// Callbacks (blocks) rendered, playing or not.
    pub callbacks: u64,
    /// Frames rendered while playing.
    pub rendered_frames: u64,
    /// Smallest and largest callback in frames.
    pub frames_min: u64,
    /// Largest callback in frames.
    pub frames_max: u64,
    /// Callbacks in which at least one ring ran short while playing.
    pub underruns: u64,
    /// Frames replaced by silence in those callbacks (summed over tracks).
    pub underrun_frames: u64,
    /// Peak of the mix before the master gain (proves the mix ran with master 0).
    pub peak: f32,
    /// Nanoseconds the reader spent filling rings (decode, resample, copy).
    pub reader_ns: u64,
    /// Nanoseconds of that inside the resampler.
    pub resample_ns: u64,
    /// Errors the device reported that end the stream (device gone, stream invalid, backend error).
    pub stream_errors: u64,
    /// Buffer under- or overruns the device reported (P3-5b: counted apart, they do not end the stream).
    pub stream_xruns: u64,
    /// Other notices of the device that leave the stream running (route changed, device busy, real-time
    /// scheduling refused).
    pub stream_warnings: u64,
    /// P4-7a: the layer voices started while playing (tracks that are heard; see [`layer`]).
    pub layer_voices: u64,
    /// P4-7a: the output frame of the last layer voice start (at half speed the frames run at twice
    /// the device rate); `None` before the first.
    pub layer_last_start: Option<i64>,
}

/// State shared by the control side, the reader and the callback.
#[derive(Debug)]
pub struct Shared {
    pub(crate) seek_req: AtomicU64,
    pub(crate) seek_target: AtomicU64,
    /// P3-15f: the tag of the requested seek; `seek_target` and `seek_tag` are one request
    /// under the sequence lock `seek_seq` (odd while a request is written).
    pub(crate) seek_tag: AtomicU64,
    /// P3-20b: the slowdown of the requested speed ([`Speed::slowdown`]), part of the seek request.
    pub(crate) seek_speed: AtomicU32,
    /// #166: the loop region of the request in session frames (start, exclusive end); written
    /// with the target under the sequence lock. A seek without a loop clears the region.
    pub(crate) seek_loop: AtomicBool,
    pub(crate) seek_loop_range: (AtomicU64, AtomicU64),
    /// #201: the window end of the request in session frames (an audition that ends in the
    /// engine); written with the target under the sequence lock. A seek without an end clears it.
    pub(crate) seek_end: AtomicU64,
    /// #201: whether the request carries a window end.
    pub(crate) seek_has_end: AtomicBool,
    pub(crate) seek_seq: AtomicU64,
    /// Makes the seek requests one writer at a time (control side only; the callback never takes it).
    seek_write: Mutex<()>,
    pub(crate) reader_paused: AtomicU64,
    pub(crate) cb_flushed: AtomicU64,
    pub(crate) flushed_target: AtomicU64,
    /// P3-20b: the slowdown of the flushed seek (the reader switches the sources to it).
    pub(crate) flushed_speed: AtomicU32,
    /// #166: whether the flushed seek armed a loop region (an audition that wraps in the engine).
    pub(crate) flushed_loop: AtomicBool,
    /// #166: the flushed loop region in output frames at the render rate (start, exclusive end).
    pub(crate) flushed_loop_range: (AtomicU64, AtomicU64),
    /// #166: the last request carried a loop region (the control side's mirror, for
    /// `Transport::loop_armed`).
    pub(crate) loop_armed: AtomicBool,
    /// #201: the last request carried a window end (the control side's mirror, for
    /// `Transport::end_armed`).
    pub(crate) end_armed: AtomicBool,
    pub(crate) playing: AtomicBool,
    pub(crate) master_gain: AtomicU32,
    pub(crate) gains: Box<[AtomicU32]>,
    /// P3-16: per track left, right ([`mixer::channel_gains`]) and the volume alone
    /// ([`mixer::db_to_gain`], the mono meter), 1.0 each.
    pub(crate) vols: Box<[AtomicU32]>,
    /// P3-16: the user's master volume (linear), before the master gain.
    pub(crate) master_volume: AtomicU32,
    /// P3-16: peaks since the last [`Control::take_meters`]: per track left, right, then
    /// master left, right (bits of positive `f32`).
    pub(crate) meters: Box<[AtomicU32]>,
    pub(crate) clicks_on: AtomicBool,
    pub(crate) stop: AtomicBool,
    pub(crate) stats: Stats,
    /// P3-5b: the mix latency probe. `set_gains` stores the time and then bumps `gain_seq`;
    /// the callback that first reads the gains of a sequence stores the time its block is
    /// heard in `gain_applied_ns` and the sequence in `gain_applied_seq`.
    pub(crate) gain_seq: AtomicU64,
    pub(crate) gain_set_ns: AtomicU64,
    pub(crate) gain_applied_seq: AtomicU64,
    pub(crate) gain_applied_ns: AtomicU64,
    /// #239: the callback count when `set_gains` stored the gains; the applying block
    /// subtracts it for the mix block counter.
    pub(crate) gain_set_cb: AtomicU64,
    /// #239: the callback count of the block that first used the last gain sequence,
    /// minus `gain_set_cb`: the mixed blocks between the gain set and the first block
    /// that carries the new gains (1 = the next block). A starved callback does not move
    /// it: no block runs, no count moves.
    pub(crate) gain_applied_blocks: AtomicU64,
    /// #430: per track, the frames the engine rendered as silence because the ring ran
    /// dry (the source is that many frames behind the engine position). The reader skips
    /// them in the source; the engine drops the stale frames the ring already holds.
    pub(crate) ring_lag: Box<[AtomicU64]>,
}

/// One complete seek request: the target, the tag, the speed (P3-15f), the loop region in
/// session frames (#166; `None`: the seek ends a loop) and the window end in session frames
/// (#201; `None`: the seek ends a windowed audition's end).
pub(crate) type SeekRequest = (u64, u64, Speed, Option<(u64, u64)>, Option<u64>);

impl Shared {
    fn new(n: usize, clicks_on: bool, speed: Speed) -> Shared {
        let stats = Stats::default();
        stats.frames_min.store(u64::MAX, Ordering::Relaxed);
        stats.layer_last.store(i64::MIN as u64, Ordering::Relaxed);
        Shared {
            seek_req: AtomicU64::new(0),
            seek_target: AtomicU64::new(0),
            seek_tag: AtomicU64::new(0),
            seek_speed: AtomicU32::new(speed.slowdown()),
            seek_loop: AtomicBool::new(false),
            seek_loop_range: (AtomicU64::new(0), AtomicU64::new(0)),
            seek_end: AtomicU64::new(0),
            seek_has_end: AtomicBool::new(false),
            seek_seq: AtomicU64::new(0),
            seek_write: Mutex::new(()),
            reader_paused: AtomicU64::new(0),
            cb_flushed: AtomicU64::new(0),
            flushed_target: AtomicU64::new(0),
            flushed_speed: AtomicU32::new(speed.slowdown()),
            flushed_loop: AtomicBool::new(false),
            flushed_loop_range: (AtomicU64::new(0), AtomicU64::new(0)),
            loop_armed: AtomicBool::new(false),
            end_armed: AtomicBool::new(false),
            playing: AtomicBool::new(false),
            master_gain: AtomicU32::new(1.0f32.to_bits()),
            gains: (0..n).map(|_| AtomicU32::new(1.0f32.to_bits())).collect(),
            vols: (0..3 * n).map(|_| AtomicU32::new(1.0f32.to_bits())).collect(),
            master_volume: AtomicU32::new(1.0f32.to_bits()),
            meters: (0..2 * n + 2).map(|_| AtomicU32::new(0)).collect(),
            clicks_on: AtomicBool::new(clicks_on),
            stop: AtomicBool::new(false),
            stats,
            gain_seq: AtomicU64::new(0),
            gain_set_ns: AtomicU64::new(0),
            gain_applied_seq: AtomicU64::new(0),
            gain_applied_ns: AtomicU64::new(0),
            gain_set_cb: AtomicU64::new(0),
            gain_applied_blocks: AtomicU64::new(0),
            ring_lag: (0..n).map(|_| AtomicU64::new(0)).collect(),
        }
    }

    fn snapshot(&self) -> StatsSnapshot {
        let s = &self.stats;
        let min = s.frames_min.load(Ordering::Relaxed);
        StatsSnapshot {
            callbacks: s.callbacks.load(Ordering::Relaxed),
            rendered_frames: s.rendered_frames.load(Ordering::Relaxed),
            frames_min: if min == u64::MAX { 0 } else { min },
            frames_max: s.frames_max.load(Ordering::Relaxed),
            underruns: s.underruns.load(Ordering::Relaxed),
            underrun_frames: s.underrun_frames.load(Ordering::Relaxed),
            peak: f32::from_bits(s.peak_bits.load(Ordering::Relaxed)),
            reader_ns: s.reader_ns.load(Ordering::Relaxed),
            resample_ns: s.resample_ns.load(Ordering::Relaxed),
            stream_errors: s.stream_errors.load(Ordering::Relaxed),
            stream_xruns: s.stream_xruns.load(Ordering::Relaxed),
            stream_warnings: s.stream_warnings.load(Ordering::Relaxed),
            layer_voices: s.layer_voices.load(Ordering::Relaxed),
            layer_last_start: Some(s.layer_last.load(Ordering::Relaxed) as i64).filter(|v| *v != i64::MIN),
        }
    }

    /// Request a seek to session frame `frame`; `tag` replaces the seek tag (`None` keeps the last
    /// one), `speed` the speed (P3-20b; `None` keeps it), `loop_range` the loop region the seek
    /// arms in session frames (#166; `None` clears it — every seek without a region ends a
    /// running loop), `end_at` the window end the seek arms in session frames (#201; `None`
    /// clears it — every seek without an end plays on). A region with an empty or inverted range
    /// is ignored. The target, the tag, the speed, the region and the end are written under the
    /// sequence lock, then `seek_req` grows: a callback that sees the new `seek_req` reads this
    /// request or a newer one, never a torn one.
    fn request_seek(
        &self,
        frame: u64,
        tag: Option<u64>,
        speed: Option<Speed>,
        loop_range: Option<(u64, u64)>,
        end_at: Option<u64>,
    ) {
        let _one_writer = self
            .seek_write
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let s = self.seek_seq.load(Ordering::Relaxed);
        self.seek_seq.store(s.wrapping_add(1), Ordering::Relaxed);
        std::sync::atomic::fence(Ordering::Release);
        self.seek_target.store(frame, Ordering::Relaxed);
        if let Some(t) = tag {
            self.seek_tag.store(t, Ordering::Relaxed);
        }
        if let Some(v) = speed {
            self.seek_speed.store(v.slowdown(), Ordering::Relaxed);
        }
        let looped = loop_range.is_some_and(|(ls, le)| le > ls);
        self.seek_loop.store(looped, Ordering::Relaxed);
        if let Some((ls, le)) = loop_range.filter(|_| looped) {
            self.seek_loop_range.0.store(ls, Ordering::Relaxed);
            self.seek_loop_range.1.store(le, Ordering::Relaxed);
        }
        self.loop_armed.store(looped, Ordering::Release);
        self.seek_has_end.store(end_at.is_some(), Ordering::Relaxed);
        if let Some(e) = end_at {
            self.seek_end.store(e, Ordering::Relaxed);
        }
        self.end_armed.store(end_at.is_some(), Ordering::Release);
        self.seek_seq.store(s.wrapping_add(2), Ordering::Release);
        self.seek_req.fetch_add(1, Ordering::AcqRel);
    }

    /// The last complete seek request (target, tag, speed, loop region and window end in
    /// session frames); `None` while a request is being written (the callback tries again at
    /// its next block). Lock-free, for the callback.
    pub(crate) fn seek_request(&self) -> Option<SeekRequest> {
        let s0 = self.seek_seq.load(Ordering::Acquire);
        if s0 % 2 == 1 {
            return None;
        }
        let target = self.seek_target.load(Ordering::Relaxed);
        let tag = self.seek_tag.load(Ordering::Relaxed);
        let speed = self.seek_speed.load(Ordering::Relaxed);
        let looped = self.seek_loop.load(Ordering::Relaxed);
        let loop_range = looped
            .then(|| {
                (
                    self.seek_loop_range.0.load(Ordering::Relaxed),
                    self.seek_loop_range.1.load(Ordering::Relaxed),
                )
            })
            .filter(|(ls, le)| le > ls);
        let has_end = self.seek_has_end.load(Ordering::Relaxed);
        let end_at = has_end.then(|| self.seek_end.load(Ordering::Relaxed));
        std::sync::atomic::fence(Ordering::Acquire);
        if self.seek_seq.load(Ordering::Relaxed) != s0 {
            return None;
        }
        Some((target, tag, Speed::from_slowdown(speed), loop_range, end_at))
    }

    /// The speed of the last request (the speed the transport plays at once the callback took it).
    fn requested_speed(&self) -> Speed {
        Speed::from_slowdown(self.seek_speed.load(Ordering::Relaxed))
    }

    fn set_gains(&self, gains: &[f32]) {
        for (a, g) in self.gains.iter().zip(gains) {
            a.store(g.to_bits(), Ordering::Relaxed);
        }
        self.gain_set_ns.store(clock::host_now_ns(), Ordering::Relaxed);
        self.gain_set_cb
            .store(self.stats.callbacks.load(Ordering::Relaxed), Ordering::Relaxed);
        self.gain_seq.fetch_add(1, Ordering::Release);
    }

    /// Nanoseconds from the last `set_gains` to the moment the block that first used those
    /// gains is heard; `None` before the first change or while no callback has applied it.
    fn mix_latency_ns(&self) -> Option<u64> {
        let seq = self.gain_seq.load(Ordering::Acquire);
        if seq == 0 || self.gain_applied_seq.load(Ordering::Acquire) != seq {
            return None;
        }
        Some(
            self.gain_applied_ns
                .load(Ordering::Relaxed)
                .saturating_sub(self.gain_set_ns.load(Ordering::Relaxed)),
        )
    }

    /// Mixed blocks between the last `set_gains` and the first block that carried the
    /// new gains: the applying block's callback count minus `gain_set_cb` (1 = the next
    /// block). `None` under the same conditions as `mix_latency_ns`.
    fn mix_latency_blocks(&self) -> Option<u64> {
        let seq = self.gain_seq.load(Ordering::Acquire);
        if seq == 0 || self.gain_applied_seq.load(Ordering::Acquire) != seq {
            return None;
        }
        Some(self.gain_applied_blocks.load(Ordering::Relaxed))
    }
}

/// The peak meters since the last take (P3-16): linear peaks, 1.0 is 0 dBFS.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MeterSnapshot {
    /// Per track `[left, right]`: post-fader (volume and pan), before mute and solo.
    pub tracks: Vec<[f32; 2]>,
    /// `[left, right]` of the mix after the master volume, before the master gain.
    pub master: [f32; 2],
}

/// The per-track facts the control side keeps to rebuild click banks.
#[derive(Debug, Clone)]
struct TrackMeta {
    rate: u32,
    /// Channels in the ring: 2 for a stereo file (played as stereo, P3-16), else 1.
    channels: usize,
    hits: Vec<u64>,
    pitch: f32,
}

fn click_bank(meta: &[TrackMeta], device_rate: u32, volume: f32) -> Box<ClickBank> {
    Box::new(
        meta.iter()
            .map(|m| TrackClicks::new(&m.hits, m.rate, device_rate, m.pitch, volume))
            .collect(),
    )
}

/// Everything [`Transport::open`] and [`Offline::new`] build before a device starts.
struct Parts {
    reader: Reader,
    engine: Engine,
    shared: Arc<Shared>,
    clock: Arc<Clock>,
    click_tx: Producer<Box<ClickBank>>,
    click_ret: Consumer<Box<ClickBank>>,
    layer_tx: Producer<Box<LayerBank>>,
    layer_ret: Consumer<Box<LayerBank>>,
    meta: Vec<TrackMeta>,
    session_frames: u64,
}

fn build(tracks: &[TrackSpec], device_rate: u32, cfg: &TransportConfig) -> Result<Parts, TransportError> {
    if tracks.is_empty() {
        return Err(TransportError::Arg("no tracks".into()));
    }
    if cfg.ring_frames < reader::REFILL_MIN * 2 || cfg.prefill_frames > cfg.ring_frames - reader::REFILL_MIN {
        return Err(TransportError::Arg(format!(
            "ring of {} frames with prefill {} is too small",
            cfg.ring_frames, cfg.prefill_frames
        )));
    }
    let mut sources = Vec::with_capacity(tracks.len());
    let out_rate = cfg.speed.render_rate(device_rate);
    for t in tracks {
        sources.push(TrackSource::open_at(
            &t.path,
            t.mode,
            device_rate,
            out_rate,
            cfg.residency,
        )?);
    }
    let meta: Vec<TrackMeta> = tracks
        .iter()
        .zip(&sources)
        .map(|(t, s)| TrackMeta {
            rate: s.track_rate(),
            channels: s.channels(),
            hits: t.hits.clone(),
            pitch: t.click_pitch,
        })
        .collect();
    let session_frames = sources
        .iter()
        .map(reader::TrackSource::device_frames)
        .max()
        .unwrap_or(0);
    let shared = Arc::new(Shared::new(tracks.len(), cfg.clicks_on, cfg.speed));
    let clock = Arc::new(Clock::new(device_rate));
    let chans: Vec<usize> = sources.iter().map(reader::TrackSource::channels).collect();
    let (producers, consumers): (Vec<_>, Vec<_>) = chans
        .iter()
        .map(|c| RingBuffer::<f32>::new(cfg.ring_frames * c))
        .unzip();
    let (click_tx, click_rx) = RingBuffer::new(8);
    let (ret_tx, click_ret) = RingBuffer::new(8);
    let (layer_tx, layer_rx) = RingBuffer::new(8);
    let (layer_ret_tx, layer_ret) = RingBuffer::new(8);
    let bank = click_bank(&meta, device_rate, cfg.click_volume);
    let mut engine = Engine::new(
        consumers.into_iter().zip(chans.iter().copied()).collect(),
        bank,
        click_rx,
        ret_tx,
        shared.clone(),
        clock.clone(),
        cfg.prefill_frames,
    );
    engine.set_layer_queue(layer_rx, layer_ret_tx);
    let reader = Reader::new(sources, producers, shared.clone());
    Ok(Parts {
        reader,
        engine,
        shared,
        clock,
        click_tx,
        click_ret,
        layer_tx,
        layer_ret,
        meta,
        session_frames,
    })
}

/// Click settings for [`Transport::set_clicks`] and [`Offline::set_clicks`].
#[derive(Debug, Clone, PartialEq)]
pub struct ClickSettings {
    /// Clicks on or off (takes effect at the next callback).
    pub enabled: bool,
    /// Click volume.
    pub volume: f32,
    /// Click pitch per track in Hz; `None` keeps the current pitches.
    pub pitches: Option<Vec<f32>>,
}

/// The control side shared by [`Transport`] and [`Offline`].
struct Control {
    shared: Arc<Shared>,
    clock: Arc<Clock>,
    click_tx: Producer<Box<ClickBank>>,
    click_ret: Consumer<Box<ClickBank>>,
    layer_tx: Producer<Box<LayerBank>>,
    layer_ret: Consumer<Box<LayerBank>>,
    meta: Vec<TrackMeta>,
    volume: f32,
    device_rate: u32,
    session_frames: u64,
}

impl Control {
    fn set_mix(&self, mute: &[bool], solo: &[bool]) -> Result<(), TransportError> {
        let n = self.meta.len();
        if mute.len() > n || solo.len() > n {
            return Err(TransportError::Arg(format!("mix lists longer than the {n} tracks")));
        }
        self.shared.set_gains(&mixer::audible_gains(n, mute, solo));
        Ok(())
    }

    fn set_track_gain(&self, track: usize, gain: f32) -> Result<(), TransportError> {
        let a = self
            .shared
            .gains
            .get(track)
            .ok_or_else(|| TransportError::Arg(format!("no track {track}")))?;
        a.store(gain.to_bits(), Ordering::Relaxed);
        Ok(())
    }

    fn set_volume_pan(&self, track: usize, volume_db: f32, pan: f32) -> Result<(), TransportError> {
        if track >= self.meta.len() {
            return Err(TransportError::Arg(format!("no track {track}")));
        }
        let (l, r) = mixer::channel_gains(volume_db, pan);
        self.shared.vols[3 * track].store(l.to_bits(), Ordering::Relaxed);
        self.shared.vols[3 * track + 1].store(r.to_bits(), Ordering::Relaxed);
        self.shared.vols[3 * track + 2].store(mixer::db_to_gain(volume_db).to_bits(), Ordering::Relaxed);
        Ok(())
    }

    fn set_master_volume_db(&self, volume_db: f32) {
        let g = mixer::db_to_gain(volume_db);
        self.shared.master_volume.store(g.to_bits(), Ordering::Relaxed);
    }

    fn take_meters(&self) -> MeterSnapshot {
        let v: Vec<f32> = self
            .shared
            .meters
            .iter()
            .map(|a| f32::from_bits(a.swap(0, Ordering::Relaxed)))
            .collect();
        let n = self.meta.len();
        MeterSnapshot {
            tracks: (0..n).map(|i| [v[2 * i], v[2 * i + 1]]).collect(),
            master: [v[2 * n], v[2 * n + 1]],
        }
    }

    fn send_bank(&mut self) -> Result<(), TransportError> {
        // drop the banks the callback handed back, outside the callback
        while self.click_ret.pop().is_ok() {}
        let bank = click_bank(&self.meta, self.device_rate, self.volume);
        self.click_tx
            .push(bank)
            .map_err(|_| TransportError::Arg("click update queue is full; retry after the next callback".into()))
    }

    fn set_clicks(&mut self, s: &ClickSettings) -> Result<(), TransportError> {
        if let Some(p) = &s.pitches
            && p.len() != self.meta.len()
        {
            return Err(TransportError::Arg(format!(
                "{} pitches for {} tracks",
                p.len(),
                self.meta.len()
            )));
        }
        self.shared.clicks_on.store(s.enabled, Ordering::Relaxed);
        let pitch_change = s
            .pitches
            .as_ref()
            .is_some_and(|p| p.iter().zip(&self.meta).any(|(a, m)| *a != m.pitch));
        if pitch_change || s.volume != self.volume {
            if let Some(p) = &s.pitches {
                for (m, f) in self.meta.iter_mut().zip(p) {
                    m.pitch = *f;
                }
            }
            self.volume = s.volume;
            self.send_bank()?;
        }
        Ok(())
    }

    /// P4-7a: hand the layer voices of every track to the callback (one bank; the old one comes
    /// back here to be dropped, outside the callback).
    fn set_layers(&mut self, bank: LayerBank) -> Result<(), TransportError> {
        if bank.len() != self.meta.len() {
            return Err(TransportError::Arg(format!(
                "{} layers for {} tracks",
                bank.len(),
                self.meta.len()
            )));
        }
        while self.layer_ret.pop().is_ok() {}
        self.layer_tx
            .push(Box::new(bank))
            .map_err(|_| TransportError::Arg("layer update queue is full; retry after the next callback".into()))
    }

    fn set_hits(&mut self, track: usize, hits: Vec<u64>) -> Result<(), TransportError> {
        let m = self
            .meta
            .get_mut(track)
            .ok_or_else(|| TransportError::Arg(format!("no track {track}")))?;
        m.hits = hits;
        self.send_bank()
    }

    fn set_all_hits(&mut self, hits: Vec<Vec<u64>>) -> Result<(), TransportError> {
        if hits.len() != self.meta.len() {
            return Err(TransportError::Arg(format!(
                "{} hit lists for {} tracks",
                hits.len(),
                self.meta.len()
            )));
        }
        for (m, h) in self.meta.iter_mut().zip(hits) {
            m.hits = h;
        }
        self.send_bank()
    }
}

/// Reads the callback count of a transport without a borrow of it (P3-15c: the app waits for
/// a seek without holding its own lock), and the tag of the last seek the callback has taken
/// (P3-15f). It stays readable after the transport is dropped; the count stops then.
#[derive(Debug, Clone)]
pub struct CallbackCounter {
    shared: Arc<Shared>,
    clock: Arc<Clock>,
}

impl CallbackCounter {
    /// The callbacks so far (the `callbacks` of [`Transport::stats`]).
    pub fn get(&self) -> u64 {
        self.shared.stats.callbacks.load(Ordering::Relaxed)
    }

    /// The tag of the last seek the callback has taken (`seek_tag` of [`Transport::playhead`]).
    pub fn seek_taken(&self) -> u64 {
        self.clock.read().seek_tag
    }
}

/// A running transport on a device (or the null device).
pub struct Transport {
    ctl: Control,
    reader_thread: Option<JoinHandle<()>>,
    running: Option<Running>,
    info: DeviceInfo,
}

impl Transport {
    /// Open the session on `output`: start the reader thread and the stream (paused at
    /// frame 0). The device rate is the device's current rate (see the module docs).
    ///
    /// # Errors
    ///
    /// [`TransportError::Arg`] when `tracks` is empty or the ring and prefill sizes of `cfg`
    /// are too small; [`TransportError::NoAnswer`] when the output device is still blocked in
    /// `CoreAudio` from an earlier open; [`TransportError::Track`] when a track cannot be opened
    /// or has a zero sample rate; [`TransportError::Device`] when the device cannot be found
    /// or queried, the reader thread or the stream cannot start, or the device rate changes
    /// while opening.
    pub fn open(tracks: &[TrackSpec], output: Output, cfg: TransportConfig) -> Result<Transport, TransportError> {
        // P3-16: a device whose last open is still blocked in CoreAudio is refused before the
        // tracks are opened
        let target = match &output {
            Output::Default => Some(device::target_name(None)?),
            Output::Device { name } => Some(name.clone()),
            Output::Null { .. } => None,
        };
        if let Some(n) = target.as_deref().filter(|n| device::is_blocked(n)) {
            return Err(TransportError::NoAnswer(format!(
                "the output device \"{n}\" does not answer (CoreAudio). Choose another output in Drum Spike > Settings or in the macOS Sound settings."
            )));
        }
        let rate = match &output {
            Output::Default => device::default_output_rate()?.0,
            Output::Device { name } => device::output_info(Some(name))?.sample_rate,
            Output::Null { sample_rate: Some(r) } => *r,
            Output::Null { sample_rate: None } => {
                let first = tracks.first().ok_or_else(|| TransportError::Arg("no tracks".into()))?;
                crate::io::MappedWav::open(&first.path)
                    .map_err(|e| TransportError::Track(format!("{}: {e}", first.path.display())))?
                    .info
                    .sample_rate
            }
        };
        let p = build(tracks, rate, &cfg)?;
        let reader_thread = std::thread::Builder::new()
            .name("drumspike-reader".into())
            .spawn({
                let reader = p.reader;
                move || reader.run()
            })
            .map_err(|e| TransportError::Device(format!("reader thread: {e}")))?;
        let started = match &output {
            Output::Default => device::start_cpal(p.engine, p.shared.clone(), cfg.buffer_frames, None),
            Output::Device { name } => device::start_cpal(p.engine, p.shared.clone(), cfg.buffer_frames, Some(name)),
            Output::Null { .. } => device::start_null(p.engine, rate, cfg.buffer_frames),
        };
        let (running, info) = match started {
            Ok(x) => x,
            Err(e) => {
                p.shared.stop.store(true, Ordering::Release);
                reader_thread.thread().unpark();
                let _ = reader_thread.join();
                return Err(e);
            }
        };
        if info.sample_rate != rate {
            // the device rate moved between the query and the stream: the sources were built
            // for the old rate, so stop everything and report it
            drop(running);
            p.shared.stop.store(true, Ordering::Release);
            reader_thread.thread().unpark();
            let _ = reader_thread.join();
            return Err(TransportError::Device(format!(
                "device rate changed from {rate} to {} Hz while opening",
                info.sample_rate
            )));
        }
        Ok(Transport {
            ctl: Control {
                shared: p.shared,
                clock: p.clock,
                click_tx: p.click_tx,
                click_ret: p.click_ret,
                layer_tx: p.layer_tx,
                layer_ret: p.layer_ret,
                meta: p.meta,
                volume: cfg.click_volume,
                device_rate: rate,
                session_frames: p.session_frames,
            },
            reader_thread: Some(reader_thread),
            running: Some(running),
            info,
        })
    }

    /// P3-16: the channels each track plays with: 2 for a stereo file, 1 for a mono file and
    /// for the mono fold of a file with more than 2 channels.
    pub fn track_channels(&self) -> Vec<usize> {
        self.ctl.meta.iter().map(|m| m.channels).collect()
    }

    /// The device: name, rate, channels, buffer.
    pub fn device(&self) -> &DeviceInfo {
        &self.info
    }

    /// The device rate (all frames of the transport are at this rate).
    pub fn sample_rate(&self) -> u32 {
        self.ctl.device_rate
    }

    /// Length of the longest track in device frames.
    pub fn session_frames(&self) -> u64 {
        self.ctl.session_frames
    }

    /// Start playback from the current position.
    pub fn play(&self) {
        self.ctl.shared.playing.store(true, Ordering::Release);
    }

    /// Pause; the position stays.
    pub fn pause(&self) {
        self.ctl.shared.playing.store(false, Ordering::Release);
    }

    /// Whether play was requested (the playhead's `playing` also covers the refill).
    pub fn is_playing(&self) -> bool {
        self.ctl.shared.playing.load(Ordering::Acquire)
    }

    /// Move to device frame `frame` (playing or paused). Playback resumes there once
    /// every ring holds the prefill. The reader is woken at once (it parks up to 10 ms when idle).
    /// The seek tag and the speed stay (see [`Transport::seek_tagged`], [`Transport::set_speed`]).
    /// Any loop region ends (see [`Transport::seek_tagged_with_loop`]).
    pub fn seek(&self, frame: u64) {
        self.ctl.shared.request_seek(frame, None, None, None, None);
        self.wake_reader();
    }

    /// P3-20b: play at `speed` from session frame `frame` (a device frame of the session, as
    /// [`Transport::seek`]), with the seek tag `tag` (`None` keeps it). It is one seek: the rings
    /// drain and refill at the new render rate, and the callback publishes the new speed in
    /// the playhead (`Playhead::slowdown`) together with the target. The first change to a
    /// speed builds the resamplers of its render rate in the reader thread; they are kept.
    /// Any loop region ends (see [`Transport::set_speed_loop`]).
    pub fn set_speed(&self, speed: Speed, frame: u64, tag: Option<u64>) {
        self.ctl.shared.request_seek(frame, tag, Some(speed), None, None);
        self.wake_reader();
    }

    /// P3-20b: the speed of the last request (the playhead's `slowdown` shows the speed heard).
    pub fn speed(&self) -> Speed {
        self.ctl.shared.requested_speed()
    }

    /// [`Transport::seek`] with a tag (P3-15f). The callback that takes this seek publishes
    /// `tag` in the playhead (`Playhead::seek_tag`) together with the target frame, so every
    /// later playhead carries it: a playhead with `seek_tag >= tag` shows a position from after
    /// this seek (or a newer one). Tags must not go down.
    pub fn seek_tagged(&self, frame: u64, tag: u64) {
        self.ctl.shared.request_seek(frame, Some(tag), None, None, None);
        self.wake_reader();
    }

    /// #166, #201: [`Transport::seek_tagged`] that arms a loop region and a window end (each
    /// `Some` or `None`; a seek replaces both). The loop is an audition that wraps in the
    /// engine. The end is an audition that ends in the engine: the engine delivers every
    /// frame before `end_at` (session frames) and pauses there, so the end is exact and waits
    /// for the delivery under load (#200/#201).
    pub fn seek_tagged_window(&self, frame: u64, tag: u64, loop_range: Option<(u64, u64)>, end_at: Option<u64>) {
        self.ctl.shared.request_seek(frame, Some(tag), None, loop_range, end_at);
        self.wake_reader();
    }

    /// #166: [`Transport::seek_tagged`] that arms a loop region (`Some((start, end))` in session
    /// frames, an audition that wraps in the engine; `None` ends one). The reader wraps its read
    /// at the region end and the engine wraps its playhead there, so the audio loops with no gap
    /// and no seek per pass. The region's frames are heard again and again until the next seek.
    pub fn seek_tagged_with_loop(&self, frame: u64, tag: u64, loop_range: Option<(u64, u64)>) {
        self.ctl.shared.request_seek(frame, Some(tag), None, loop_range, None);
        self.wake_reader();
    }

    /// #166: [`Transport::set_speed`] that arms a loop region like [`Transport::seek_tagged_with_loop`].
    pub fn set_speed_loop(&self, speed: Speed, frame: u64, tag: Option<u64>, loop_range: Option<(u64, u64)>) {
        self.ctl.shared.request_seek(frame, tag, Some(speed), loop_range, None);
        self.wake_reader();
    }

    /// #166, #201: [`Transport::set_speed`] that arms a loop region and a window end like
    /// [`Transport::seek_tagged_window`].
    pub fn set_speed_window(
        &self,
        speed: Speed,
        frame: u64,
        tag: Option<u64>,
        loop_range: Option<(u64, u64)>,
        end_at: Option<u64>,
    ) {
        self.ctl
            .shared
            .request_seek(frame, tag, Some(speed), loop_range, end_at);
        self.wake_reader();
    }

    /// #166: whether the last seek request armed a loop region. A play or seek that sends none
    /// must clear the region before it plays on (a plain play without a position sends no seek:
    /// it clears the loop with a seek to where the transport is).
    pub fn loop_armed(&self) -> bool {
        self.ctl.shared.loop_armed.load(Ordering::Acquire)
    }

    /// #201: whether the last seek request armed a window end (an audition that ends in the
    /// engine). A play or seek that sends none must clear the end before it plays on (a plain
    /// play without a position sends no seek: it clears the end with a seek to where the
    /// transport is, like the loop's).
    pub fn end_armed(&self) -> bool {
        self.ctl.shared.end_armed.load(Ordering::Acquire)
    }

    fn wake_reader(&self) {
        if let Some(t) = &self.reader_thread {
            t.thread().unpark();
        }
    }

    /// Move to `seconds` (rounded to the device frame).
    pub fn seek_seconds(&self, seconds: f64) {
        self.seek(self.frame_of(seconds));
    }

    /// The device frame of `seconds` (rounded; 0 for a negative time).
    pub fn frame_of(&self, seconds: f64) -> u64 {
        (seconds.max(0.0) * f64::from(self.ctl.device_rate)).round() as u64
    }

    /// Mute and solo (`audible()` of `app.js`), applied at the next callback.
    ///
    /// # Errors
    ///
    /// [`TransportError::Arg`] when `mute` or `solo` is longer than the number of tracks.
    pub fn set_mix(&self, mute: &[bool], solo: &[bool]) -> Result<(), TransportError> {
        self.ctl.set_mix(mute, solo)
    }

    /// Set one track's gain directly.
    ///
    /// # Errors
    ///
    /// [`TransportError::Arg`] when `track` is not a track of the session.
    pub fn set_track_gain(&self, track: usize, gain: f32) -> Result<(), TransportError> {
        self.ctl.set_track_gain(track, gain)
    }

    /// Gain after the mix (0.0 makes a silent run that still computes the full mix).
    pub fn set_master_gain(&self, gain: f32) {
        self.ctl.shared.master_gain.store(gain.to_bits(), Ordering::Relaxed);
    }

    /// P3-16: one track's volume (dB, [`mixer::MIN_DB`] and below is off, up to
    /// [`mixer::MAX_DB`]) and pan (-1 left to +1 right), applied at the next callback.
    ///
    /// # Errors
    ///
    /// [`TransportError::Arg`] when `track` is not a track of the session.
    pub fn set_volume_pan(&self, track: usize, volume_db: f32, pan: f32) -> Result<(), TransportError> {
        self.ctl.set_volume_pan(track, volume_db, pan)
    }

    /// P3-16: the user's master volume in dB (before the master gain of the tests).
    pub fn set_master_volume_db(&self, volume_db: f32) {
        self.ctl.set_master_volume_db(volume_db);
    }

    /// P3-16: the peak meters since the last call (and resets them).
    pub fn take_meters(&self) -> MeterSnapshot {
        self.ctl.take_meters()
    }

    /// Clicks on or off, volume and pitches.
    ///
    /// # Errors
    ///
    /// [`TransportError::Arg`] when `s.pitches` does not hold one pitch per track, or the click
    /// update queue is full (retry after the next callback).
    pub fn set_clicks(&mut self, s: &ClickSettings) -> Result<(), TransportError> {
        self.ctl.set_clicks(s)
    }

    /// Replace one track's hit samples (track rate) after an edit.
    ///
    /// # Errors
    ///
    /// [`TransportError::Arg`] when `track` is not a track of the session, or the click update
    /// queue is full (retry after the next callback).
    pub fn set_hits(&mut self, track: usize, hits: Vec<u64>) -> Result<(), TransportError> {
        self.ctl.set_hits(track, hits)
    }

    /// Replace the hit samples of every track at once (one click bank, not one per track).
    ///
    /// # Errors
    ///
    /// [`TransportError::Arg`] when `hits` does not hold one list per track, or the click
    /// update queue is full (retry after the next callback).
    pub fn set_all_hits(&mut self, hits: Vec<Vec<u64>>) -> Result<(), TransportError> {
        self.ctl.set_all_hits(hits)
    }

    /// P4-7a: the layer voices of every track (one entry per track, `None`: no layer), applied at
    /// the next callback. A transport without layers does no layer work in the callback.
    ///
    /// # Errors
    ///
    /// [`TransportError::Arg`] when `bank` does not hold one entry per track, or the layer
    /// update queue is full (retry after the next callback).
    pub fn set_layers(&mut self, bank: LayerBank) -> Result<(), TransportError> {
        self.ctl.set_layers(bank)
    }

    /// The playhead from the last callback.
    pub fn playhead(&self) -> Playhead {
        self.ctl.clock.read()
    }

    /// The counters.
    pub fn stats(&self) -> StatsSnapshot {
        self.ctl.shared.snapshot()
    }

    /// A [`CallbackCounter`] of this transport.
    pub fn callback_counter(&self) -> CallbackCounter {
        CallbackCounter {
            shared: Arc::clone(&self.ctl.shared),
            clock: Arc::clone(&self.ctl.clock),
        }
    }

    /// Nanoseconds from the last mute or solo change to the moment the first block with the
    /// new gains is heard (the callback's playback time). `None` before a change, and while
    /// the transport is paused (the callback reads the gains only while it plays).
    pub fn mix_latency_ns(&self) -> Option<u64> {
        self.ctl.shared.mix_latency_ns()
    }

    /// Mixed blocks between the last mute or solo change and the first block with the
    /// new gains (the applying block's callback count minus the count at set time; 1 =
    /// the next block). `None` under the same conditions as `mix_latency_ns`.
    pub fn mix_latency_blocks(&self) -> Option<u64> {
        self.ctl.shared.mix_latency_blocks()
    }

    /// The master gain now.
    pub fn master_gain(&self) -> f32 {
        f32::from_bits(self.ctl.shared.master_gain.load(Ordering::Relaxed))
    }
}

impl Drop for Transport {
    fn drop(&mut self) {
        self.running.take();
        self.ctl.shared.stop.store(true, Ordering::Release);
        self.wake_reader();
        if let Some(t) = self.reader_thread.take() {
            let _ = t.join();
        }
    }
}

/// The transport without a device or threads: the same reader and engine, driven in turn.
/// Renders are deterministic, so tests compare them bit for bit.
pub struct Offline {
    ctl: Control,
    reader: Reader,
    engine: Engine,
    block: usize,
}

impl Offline {
    /// Build the session at `device_rate`, playing, at frame 0, blocks of
    /// `cfg.buffer_frames`.
    ///
    /// # Errors
    ///
    /// [`TransportError::Arg`] when `tracks` is empty or the ring and prefill sizes of `cfg`
    /// are too small; [`TransportError::Track`] when a track cannot be opened or has a zero
    /// sample rate.
    pub fn new(tracks: &[TrackSpec], device_rate: u32, cfg: TransportConfig) -> Result<Offline, TransportError> {
        let p = build(tracks, device_rate, &cfg)?;
        p.shared.playing.store(true, Ordering::Release);
        Ok(Offline {
            ctl: Control {
                shared: p.shared,
                clock: p.clock,
                click_tx: p.click_tx,
                click_ret: p.click_ret,
                layer_tx: p.layer_tx,
                layer_ret: p.layer_ret,
                meta: p.meta,
                volume: cfg.click_volume,
                device_rate,
                session_frames: p.session_frames,
            },
            reader: p.reader,
            engine: p.engine,
            block: cfg.buffer_frames.max(1) as usize,
        })
    }

    /// Length of the longest track in device frames.
    pub fn session_frames(&self) -> u64 {
        self.ctl.session_frames
    }

    /// The channels each track plays with (see [`Transport::track_channels`]).
    pub fn track_channels(&self) -> Vec<usize> {
        self.ctl.meta.iter().map(|m| m.channels).collect()
    }

    /// Run the reader and the engine's state machine until the engine plays from the
    /// rings. The reader keeps every ring within `REFILL_MIN` frames of full, far above one
    /// block, so the next block never runs short.
    fn settle(&mut self) {
        for _ in 0..100_000 {
            self.engine.poll();
            self.reader.step();
            self.engine.poll();
            if self.engine.is_ready() {
                return;
            }
        }
        panic!("offline transport did not settle");
    }

    /// Render `frames` frames of interleaved stereo.
    pub fn render(&mut self, frames: usize) -> Vec<f32> {
        let mut out = vec![0.0f32; 2 * frames];
        let block = self.block;
        for chunk in out.chunks_mut(2 * block) {
            self.settle();
            self.engine.render(chunk, 2, 0);
        }
        out
    }

    /// Render without servicing the reader (the rings drain; for the underrun counter).
    pub fn render_starved(&mut self, frames: usize) -> Vec<f32> {
        let mut out = vec![0.0f32; 2 * frames];
        self.engine.render(&mut out, 2, 0);
        out
    }

    /// The device frame of the next rendered sample.
    pub fn frame_pos(&self) -> u64 {
        self.engine.frame_pos()
    }

    /// Seek (see [`Transport::seek`]).
    pub fn seek(&mut self, frame: u64) {
        self.ctl.shared.request_seek(frame, None, None, None, None);
    }

    /// Seek with a tag (see [`Transport::seek_tagged`]).
    pub fn seek_tagged(&mut self, frame: u64, tag: u64) {
        self.ctl.shared.request_seek(frame, Some(tag), None, None, None);
    }

    /// Seek with a tag and a loop region (see [`Transport::seek_tagged_with_loop`]).
    pub fn seek_tagged_with_loop(&mut self, frame: u64, tag: u64, loop_range: Option<(u64, u64)>) {
        self.ctl.shared.request_seek(frame, Some(tag), None, loop_range, None);
    }

    /// Seek with a tag, a loop region and a window end (see [`Transport::seek_tagged_window`]).
    pub fn seek_tagged_window(&mut self, frame: u64, tag: u64, loop_range: Option<(u64, u64)>, end_at: Option<u64>) {
        self.ctl.shared.request_seek(frame, Some(tag), None, loop_range, end_at);
    }

    /// Speed change (see [`Transport::set_speed`]).
    pub fn set_speed(&mut self, speed: Speed, frame: u64, tag: Option<u64>) {
        self.ctl.shared.request_seek(frame, tag, Some(speed), None, None);
    }

    /// Speed change with a loop region (see [`Transport::set_speed_loop`]).
    pub fn set_speed_loop(&mut self, speed: Speed, frame: u64, tag: Option<u64>, loop_range: Option<(u64, u64)>) {
        self.ctl.shared.request_seek(frame, tag, Some(speed), loop_range, None);
    }

    /// The requested speed (see [`Transport::speed`]).
    pub fn speed(&self) -> Speed {
        self.ctl.shared.requested_speed()
    }

    /// Mute and solo (see [`Transport::set_mix`]).
    ///
    /// # Errors
    ///
    /// [`TransportError::Arg`] when `mute` or `solo` is longer than the number of tracks.
    pub fn set_mix(&mut self, mute: &[bool], solo: &[bool]) -> Result<(), TransportError> {
        self.ctl.set_mix(mute, solo)
    }

    /// One track's gain (see [`Transport::set_track_gain`]).
    ///
    /// # Errors
    ///
    /// [`TransportError::Arg`] when `track` is not a track of the session.
    pub fn set_track_gain(&mut self, track: usize, gain: f32) -> Result<(), TransportError> {
        self.ctl.set_track_gain(track, gain)
    }

    /// Master gain (see [`Transport::set_master_gain`]).
    pub fn set_master_gain(&mut self, gain: f32) {
        self.ctl.shared.master_gain.store(gain.to_bits(), Ordering::Relaxed);
    }

    /// Volume and pan of one track (see [`Transport::set_volume_pan`]).
    ///
    /// # Errors
    ///
    /// [`TransportError::Arg`] when `track` is not a track of the session.
    pub fn set_volume_pan(&mut self, track: usize, volume_db: f32, pan: f32) -> Result<(), TransportError> {
        self.ctl.set_volume_pan(track, volume_db, pan)
    }

    /// Master volume (see [`Transport::set_master_volume_db`]).
    pub fn set_master_volume_db(&mut self, volume_db: f32) {
        self.ctl.set_master_volume_db(volume_db);
    }

    /// The meters (see [`Transport::take_meters`]).
    pub fn take_meters(&mut self) -> MeterSnapshot {
        self.ctl.take_meters()
    }

    /// Play or pause.
    pub fn set_playing(&mut self, on: bool) {
        self.ctl.shared.playing.store(on, Ordering::Release);
    }

    /// Clicks (see [`Transport::set_clicks`]).
    ///
    /// # Errors
    ///
    /// [`TransportError::Arg`] when `s.pitches` does not hold one pitch per track, or the click
    /// update queue is full (retry after the next callback).
    pub fn set_clicks(&mut self, s: &ClickSettings) -> Result<(), TransportError> {
        self.ctl.set_clicks(s)
    }

    /// Hits of one track (see [`Transport::set_hits`]).
    ///
    /// # Errors
    ///
    /// [`TransportError::Arg`] when `track` is not a track of the session, or the click update
    /// queue is full (retry after the next callback).
    pub fn set_hits(&mut self, track: usize, hits: Vec<u64>) -> Result<(), TransportError> {
        self.ctl.set_hits(track, hits)
    }

    /// The playhead (see [`Transport::playhead`]).
    pub fn playhead(&self) -> Playhead {
        self.ctl.clock.read()
    }

    /// The counters.
    pub fn stats(&self) -> StatsSnapshot {
        self.ctl.shared.snapshot()
    }

    /// See [`Transport::mix_latency_ns`] (the offline render passes host time 0).
    pub fn mix_latency_ns(&self) -> Option<u64> {
        self.ctl.shared.mix_latency_ns()
    }

    /// Replace the hit samples of every track at once (see [`Transport::set_all_hits`]).
    ///
    /// # Errors
    ///
    /// [`TransportError::Arg`] when `hits` does not hold one list per track, or the click
    /// update queue is full (retry after the next callback).
    pub fn set_all_hits(&mut self, hits: Vec<Vec<u64>>) -> Result<(), TransportError> {
        self.ctl.set_all_hits(hits)
    }

    /// The layer voices (see [`Transport::set_layers`]).
    ///
    /// # Errors
    ///
    /// [`TransportError::Arg`] when `bank` does not hold one entry per track, or the layer
    /// update queue is full (retry after the next callback).
    pub fn set_layers(&mut self, bank: LayerBank) -> Result<(), TransportError> {
        self.ctl.set_layers(bank)
    }
}

#[cfg(test)]
mod tests;
