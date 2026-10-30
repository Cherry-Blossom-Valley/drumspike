//! The transport clock: which device frame is heard at which host time.
//!
//! The audio callback publishes, once per block, the frame of the block's first sample and
//! the time at which the device plays it (the callback's playback timestamp, mapped to the
//! transport's monotonic clock). Readers on other threads get a consistent pair and
//! extrapolate. This is the spike's conclusion: the playhead comes from the callback
//! timestamps, not from a wall clock started at play.
//!
//! One writer (the callback), any number of readers; a sequence lock on atomics, so the
//! writer never blocks or allocates.
//!
//! P3-20b: at half speed the session position moves by half a frame per output frame. The
//! callback publishes the session frame (at the device rate) and the slowdown (1 or 2), and
//! [`Playhead::frame_at`] extrapolates at `sample_rate / slowdown` session frames per second.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::Instant;

/// One playhead sample: `frame` (device rate) is heard at `host_time_ns`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Playhead {
    /// Device frame of the transport position (0 = session start).
    pub frame: u64,
    /// The device sample rate.
    pub sample_rate: u32,
    /// When `frame` is heard, in nanoseconds of [`host_now_ns`].
    pub host_time_ns: u64,
    /// The transport plays (false while paused or while it refills after a seek).
    pub playing: bool,
    /// P3-15f: the tag of the last seek the callback has taken (see `Transport::seek_tagged`), in
    /// the same snapshot as `frame`: a reader knows whether this position is from after a given
    /// seek. 0 before the first tagged seek.
    pub seek_tag: u64,
    /// P3-20b: output frames per session frame: 1 at normal speed, 2 at half speed.
    pub slowdown: u32,
}

impl Playhead {
    /// The position in seconds.
    pub fn seconds(&self) -> f64 {
        self.frame as f64 / self.sample_rate as f64
    }

    /// The frame at host time `now_ns`: extrapolated at the sample rate divided by the
    /// slowdown while playing, `frame` otherwise; never before `frame`.
    pub fn frame_at(&self, now_ns: u64) -> u64 {
        if !self.playing || now_ns <= self.host_time_ns {
            return self.frame;
        }
        let dt = (now_ns - self.host_time_ns) as u128;
        let per = 1_000_000_000u128 * self.slowdown.max(1) as u128;
        self.frame + (dt * self.sample_rate as u128 / per) as u64
    }

    /// P3-20b: session seconds per second of playback (1.0 or 0.5).
    pub fn rate(&self) -> f64 {
        1.0 / self.slowdown.max(1) as f64
    }
}

/// Nanoseconds on the process's monotonic clock (zero at the first call in the process).
pub fn host_now_ns() -> u64 {
    static BASE: OnceLock<Instant> = OnceLock::new();
    BASE.get_or_init(Instant::now).elapsed().as_nanos() as u64
}

/// The number of recent blocks [`StampMap`] takes the minimum offset over (about 0.3 s at 256
/// frames, 48 kHz).
const STAMP_WINDOW: usize = 64;

/// Maps the device's playback instants onto [`host_now_ns`] (#471).
///
/// The callback's playback instant is read on the device's clock, once per block: it has no
/// wake-up jitter. The host clock read inside the callback has it, because the callback thread
/// wakes up late by a varying amount. So `host - callback` is the true offset plus that delay,
/// never less than the true offset; the minimum over the last [`STAMP_WINDOW`] blocks is the
/// offset with the least delay, and it tracks a slow drift of the device clock.
#[derive(Debug)]
pub struct StampMap {
    recent: [i128; STAMP_WINDOW],
    filled: usize,
    next: usize,
}

impl Default for StampMap {
    fn default() -> Self {
        Self::new()
    }
}

impl StampMap {
    /// An empty map: the first block sets the offset.
    pub fn new() -> StampMap {
        StampMap {
            recent: [0; STAMP_WINDOW],
            filled: 0,
            next: 0,
        }
    }

    /// The host time (ns) at which the block's first sample plays. `callback` and `playback` are
    /// the device-clock instants (ns) of the callback and of the playback; `host_at_callback` is
    /// [`host_now_ns`] read in the callback.
    pub fn host_play_ns(&mut self, callback: i128, playback: i128, host_at_callback: u64) -> u64 {
        self.recent[self.next] = host_at_callback as i128 - callback;
        self.next = (self.next + 1) % STAMP_WINDOW;
        self.filled = (self.filled + 1).min(STAMP_WINDOW);
        let offset = self.recent[..self.filled].iter().copied().min().unwrap_or(0);
        (playback + offset).max(0) as u64
    }
}

/// The shared clock (sequence lock).
#[derive(Debug)]
pub struct Clock {
    seq: AtomicU64,
    frame: AtomicU64,
    host_ns: AtomicU64,
    playing: AtomicBool,
    seek_tag: AtomicU64,
    slowdown: AtomicU32,
    sample_rate: u32,
}

impl Clock {
    /// A clock at frame 0, not playing.
    pub fn new(sample_rate: u32) -> Clock {
        Clock {
            seq: AtomicU64::new(0),
            frame: AtomicU64::new(0),
            host_ns: AtomicU64::new(0),
            playing: AtomicBool::new(false),
            seek_tag: AtomicU64::new(0),
            slowdown: AtomicU32::new(1),
            sample_rate,
        }
    }

    /// Publish a sample, the tag of the last seek taken and the slowdown (writer side; one
    /// writer only).
    pub fn publish(&self, frame: u64, host_time_ns: u64, playing: bool, seek_tag: u64, slowdown: u32) {
        let s = self.seq.load(Ordering::Relaxed);
        self.seq.store(s.wrapping_add(1), Ordering::Relaxed);
        std::sync::atomic::fence(Ordering::Release);
        self.frame.store(frame, Ordering::Relaxed);
        self.host_ns.store(host_time_ns, Ordering::Relaxed);
        self.playing.store(playing, Ordering::Relaxed);
        self.seek_tag.store(seek_tag, Ordering::Relaxed);
        self.slowdown.store(slowdown, Ordering::Relaxed);
        self.seq.store(s.wrapping_add(2), Ordering::Release);
    }

    /// Read a consistent pair (spins only while a publish is in flight).
    pub fn read(&self) -> Playhead {
        loop {
            let s0 = self.seq.load(Ordering::Acquire);
            if s0 % 2 == 1 {
                std::hint::spin_loop();
                continue;
            }
            let frame = self.frame.load(Ordering::Relaxed);
            let host_time_ns = self.host_ns.load(Ordering::Relaxed);
            let playing = self.playing.load(Ordering::Relaxed);
            let seek_tag = self.seek_tag.load(Ordering::Relaxed);
            let slowdown = self.slowdown.load(Ordering::Relaxed);
            std::sync::atomic::fence(Ordering::Acquire);
            if self.seq.load(Ordering::Relaxed) == s0 {
                return Playhead {
                    frame,
                    sample_rate: self.sample_rate,
                    host_time_ns,
                    playing,
                    seek_tag,
                    slowdown,
                };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn publish_then_read_and_extrapolate() {
        let c = Clock::new(48_000);
        assert_eq!(c.read().frame, 0);
        assert!(!c.read().playing);
        assert_eq!(c.read().seek_tag, 0);
        c.publish(48_000, 1_000_000_000, true, 7, 1);
        let p = c.read();
        assert_eq!(p.frame, 48_000);
        assert_eq!(p.seek_tag, 7);
        assert_eq!(p.seconds(), 1.0);
        assert_eq!(p.frame_at(1_500_000_000), 72_000);
        assert_eq!(p.frame_at(900_000_000), 48_000, "never before the published frame");
        assert_eq!((p.slowdown, p.rate()), (1, 1.0));
        c.publish(48_000, 1_000_000_000, false, 7, 1);
        assert_eq!(c.read().frame_at(5_000_000_000), 48_000, "paused: no extrapolation");
    }

    /// P3-20b: at half speed the session position moves at half the sample rate.
    #[test]
    fn half_speed_extrapolates_at_half_the_rate() {
        let c = Clock::new(48_000);
        c.publish(48_000, 1_000_000_000, true, 3, 2);
        let p = c.read();
        assert_eq!((p.slowdown, p.rate(), p.seconds()), (2, 0.5, 1.0));
        assert_eq!(
            p.frame_at(2_000_000_000),
            72_000,
            "one second of playback is half a second of the session"
        );
        assert_eq!(p.frame_at(1_500_000_000), 60_000);
        c.publish(48_000, 1_000_000_000, true, 3, 1);
        assert_eq!(c.read().frame_at(2_000_000_000), 96_000, "back to normal speed");
    }

    /// #471: the device's playback instants, mapped onto the host clock, stay smooth when the
    /// callback thread wakes up late by 0..200 us; the host read in the callback does not.
    #[test]
    fn stamp_map_is_smooth_under_wake_up_jitter() {
        // 256 frames at 48 kHz: a block every 5.333 ms, played 10 ms after its callback.
        const PERIOD: i128 = 5_333_333;
        const LATENCY: i128 = 10_000_000;
        const OFFSET: i128 = 7_000_000_000;
        let mut map = StampMap::new();
        let mut seed: u64 = 1;
        let (mut worst, mut naive_worst) = (0i128, 0i128);
        for k in 0..1000i128 {
            let callback = 1_000_000_000 + k * PERIOD;
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let wake = ((seed >> 33) % 200_000) as i128;
            let host = (callback + OFFSET + wake) as u64;
            let play = map.host_play_ns(callback, callback + LATENCY, host) as i128;
            let ideal = callback + LATENCY + OFFSET;
            naive_worst = naive_worst.max((host as i128 + LATENCY - ideal).abs());
            if k >= STAMP_WINDOW as i128 {
                worst = worst.max((play - ideal).abs());
            }
        }
        assert!(worst <= 30_000, "mapped playback is off by {worst} ns");
        assert!(
            naive_worst >= 100_000,
            "the jitter is in the host read: {naive_worst} ns"
        );
    }

    #[test]
    fn host_now_is_monotonic() {
        let a = host_now_ns();
        let b = host_now_ns();
        assert!(b >= a);
    }

    #[test]
    fn readers_see_consistent_pairs_under_a_writer() {
        // the writer keeps host_ns = 2 * frame and seek_tag = frame + 1; a torn read would break
        // a relation
        let c = Arc::new(Clock::new(48_000));
        let w = {
            let c = c.clone();
            std::thread::spawn(move || {
                for f in 0..200_000u64 {
                    c.publish(f, 2 * f, true, f + 1, 1 + (f % 2) as u32);
                }
            })
        };
        let mut checked = 0;
        while !w.is_finished() || checked < 1000 {
            let p = c.read();
            assert_eq!(p.host_time_ns, 2 * p.frame);
            if p.host_time_ns > 0 {
                assert_eq!(p.seek_tag, p.frame + 1);
                assert_eq!(p.slowdown, 1 + (p.frame % 2) as u32);
            }
            checked += 1;
        }
        w.join().unwrap();
    }
}
