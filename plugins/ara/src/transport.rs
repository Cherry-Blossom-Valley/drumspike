//! The host transport (M1b5, #546): the position and the play state of the host, from the audio
//! thread to the page of the editor. The design is in `docs/design/daw-plugin/m1b5-host-transport.md`.
//!
//! - **Source.** The CLAP `process` call carries the transport (song position in seconds, the
//!   play flag). ARA has no call that gives a plugin the transport: the ARA playback controller
//!   only lets the plugin ask the host to play or to move. So the process transport is the only
//!   source. A host that makes no process call, or a call without a transport or without a seconds
//!   timeline, gives nothing: the page keeps its playhead.
//! - **Hand-over.** The audio thread only stores atomics ([`note`]): one sequence lock, no mutex,
//!   no allocation, no row, no log. Several plugin instances may call [`note`] at the same time;
//!   a writer that finds the lock taken skips its store (the other writer stores the same host
//!   transport). The editor thread reads with [`read`].
//! - **Pacing.** [`Pacer`] runs on the request thread of the long poll. It decides when the page
//!   gets a message: at the first one, at each change of the play state, at each jump of the
//!   position, and every 40 ms while the host plays. It sends nothing while the host is stopped
//!   and unchanged.
//! - **Mapping.** [`page_time`] maps the song position of the host to the time the page shows
//!   (source time), with the playback regions of the last round: the same regions the notes use.
use crate::service::RegionRec;
use std::sync::atomic::{fence, AtomicBool, AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

/// CLAP seconds are fixed point: 2^31 per second.
const SEC_FIXED: f64 = 2_147_483_648.0;
const FLAG_PLAYING: u32 = clap_sys::events::CLAP_TRANSPORT_IS_PLAYING;
const FLAG_SECONDS: u32 = clap_sys::events::CLAP_TRANSPORT_HAS_SECONDS_TIMELINE;

/// A host that makes no process call for this long while its last call said "playing" has
/// stopped (some hosts stop the process calls of a plugin when the transport stops). With large
/// blocks the time is three blocks: see [`stale_us`].
pub const STALE_US: u64 = 300_000;
/// The page gets a message at least this often while the host plays, and a jump message at most
/// this often (the newest jump wins).
pub const TICK_US: u64 = 40_000;
/// The page slews an error of up to 20 ms and ignores a larger one unless the message is a hard
/// anchor (a new epoch): an error above this floor is a jump. The limit grows with the block
/// ([`jump_s`]).
pub const JUMP_S: f64 = 0.018;
/// A position change while stopped that counts as a locate.
const STOP_MOVE_S: f64 = 0.0005;
/// The wait of the poll while the host is stopped and nothing changed (or went silent).
pub const IDLE_MS: u64 = 20;
/// The wait of the poll while the host plays.
pub const PLAY_MS: u64 = 8;
/// The region list is fetched again after this time at the most.
const REGION_TTL_US: u64 = 200_000;
/// Set in the flags of the slot when the instance that owned it was deactivated (our own bit,
/// CLAP uses the low bits).
const FLAG_CLEARED: u32 = 1 << 31;

/// The time without a process call after which a playing host counts as stopped: 300 ms, or
/// three blocks when that is longer (a block of 16384 frames at 44.1 kHz is 372 ms).
pub fn stale_us(frames: u32, rate: f64) -> u64 {
    if rate > 0.0 {
        STALE_US.max((3.0 * f64::from(frames) / rate * 1e6) as u64)
    } else {
        STALE_US
    }
}

/// The jump limit: 1.5 blocks, not below [`JUMP_S`]. A host that calls process in bursts or ahead
/// of time makes errors of up to a block; they are no jumps.
pub fn jump_s(frames: u32, rate: f64) -> f64 {
    if rate > 0.0 {
        JUMP_S.max(1.5 * f64::from(frames) / rate)
    } else {
        JUMP_S
    }
}

// ---------------------------------------------------------------- the hand-over (audio thread)

/// The fault switch `DS_ARA_TRANSPORT_FAULT=off` (read once, off the audio thread): the audio
/// thread stores nothing, the check's fault run.
static FAULT_OFF: AtomicBool = AtomicBool::new(false);
/// `DS_ARA_TRANSPORT_FAULT=alloc`: the audio thread allocates in its call (the fault run of the
/// unit test that names the hand-over).
static FAULT_ALLOC: AtomicBool = AtomicBool::new(false);

/// Read the test switches. Called by `activate` (never by the audio thread).
pub fn read_switches() {
    let v = crate::test_switch("DS_ARA_TRANSPORT_FAULT").unwrap_or_default();
    FAULT_OFF.store(v == "off", Ordering::Relaxed);
    FAULT_ALLOC.store(v == "alloc", Ordering::Relaxed);
}

fn start() -> &'static Instant {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now)
}

/// The monotonic time of this process in microseconds.
pub fn now_us() -> u64 {
    start().elapsed().as_micros() as u64
}

/// The shared slot: a sequence lock. `seq` is odd while a writer stores. Every field is an atomic,
/// so a torn read is only a wrong value that the sequence check throws away.
///
/// One instance owns the slot (`owner`, the address of its plugin object): another instance
/// stores only when the owner has gone quiet for [`stale_us`]. So two instances at different
/// positions (a render ahead, another context) never alternate: the page sees the owner.
struct Slot {
    seq: AtomicU64,
    owner: AtomicU64,
    pos_fixed: AtomicI64,
    seen_us: AtomicU64,
    flags: AtomicU32,
    frames: AtomicU32,
    rate_bits: AtomicU64,
}

static SLOT: Slot = Slot {
    seq: AtomicU64::new(0),
    owner: AtomicU64::new(0),
    pos_fixed: AtomicI64::new(0),
    seen_us: AtomicU64::new(0),
    flags: AtomicU32::new(0),
    frames: AtomicU32::new(0),
    rate_bits: AtomicU64::new(0),
};

/// What the audio thread saw in one process call.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Snap {
    /// The generation: the number of stores so far (half the sequence).
    pub generation: u64,
    /// The song position of the host at the start of the block, in seconds. Valid only with
    /// `has_position`.
    pub pos_s: f64,
    /// The song position in samples of the host (seconds times the sample rate).
    pub pos_samples: i64,
    /// The time of the process call (microseconds, [`now_us`]).
    pub seen_us: u64,
    pub playing: bool,
    /// The transport had a seconds timeline. Without it the position is unknown.
    pub has_position: bool,
    /// The flags of the CLAP transport.
    pub flags: u32,
    pub frames: u32,
    pub rate: f64,
}

/// Store one transport for the instance `id`. Lock-free and wait-free: a failed compare-exchange
/// means another audio thread stores right now, a fresh owner means another instance owns the
/// slot; in both cases the call returns at once.
fn store_at(id: u64, new_owner: u64, pos_fixed: i64, flags: u32, frames: u32, rate: f64, seen_us: u64) -> bool {
    let owner = SLOT.owner.load(Ordering::Relaxed);
    if owner != 0 && owner != id {
        let last = SLOT.seen_us.load(Ordering::Relaxed);
        let fr = SLOT.frames.load(Ordering::Relaxed);
        let r = f64::from_bits(SLOT.rate_bits.load(Ordering::Relaxed));
        if seen_us.saturating_sub(last) < stale_us(fr, r) {
            return false;
        }
    }
    let s = SLOT.seq.load(Ordering::Relaxed);
    if s & 1 == 1 {
        return false;
    }
    if SLOT
        .seq
        .compare_exchange(s, s + 1, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        return false;
    }
    fence(Ordering::Release);
    SLOT.owner.store(new_owner, Ordering::Relaxed);
    SLOT.pos_fixed.store(pos_fixed, Ordering::Relaxed);
    SLOT.seen_us.store(seen_us, Ordering::Relaxed);
    SLOT.flags.store(flags, Ordering::Relaxed);
    SLOT.frames.store(frames, Ordering::Relaxed);
    SLOT.rate_bits.store(rate.to_bits(), Ordering::Relaxed);
    SLOT.seq.store(s + 2, Ordering::Release);
    true
}

/// The audio thread's part: a process call of the instance `id`. Atomics only. A call with no
/// transport stores nothing. A call with a transport but no seconds timeline stores its play state
/// with `has_position` false, so the reader does not trust the older position.
///
/// # Safety
/// `process` points to the host's valid `clap_process` of this call, and its `transport` is null
/// or valid (the CLAP rule for the call).
pub unsafe fn note(process: *const clap_sys::process::clap_process, rate: f64, id: u64) {
    if FAULT_OFF.load(Ordering::Relaxed) {
        return;
    }
    if FAULT_ALLOC.load(Ordering::Relaxed) {
        std::hint::black_box(Vec::<u8>::with_capacity(16));
    }
    let pr = &*process;
    if pr.transport.is_null() {
        return;
    }
    let t = &*pr.transport;
    let pos = if t.flags & FLAG_SECONDS != 0 {
        t.song_pos_seconds
    } else {
        0
    };
    store_at(id, id, pos, t.flags & !FLAG_CLEARED, pr.frames_count, rate, now_us());
}

/// The instance `id` was deactivated or reset: when it owns the slot, the slot is cleared, so a
/// new editor does not move the playhead to the position of an earlier session. Main thread.
pub fn clear(id: u64) {
    if SLOT.owner.load(Ordering::Relaxed) != id {
        return;
    }
    for _ in 0..1000 {
        if store_at(id, 0, 0, FLAG_CLEARED, 0, 0.0, now_us()) {
            return;
        }
        std::hint::spin_loop();
    }
}

/// The newest transport the audio thread stored, or `None` when no process call carried one (or
/// the owner was deactivated since). Never waits long: a reader that meets a writer tries again a
/// few times, then answers `None` (the next poll reads again).
pub fn read() -> Option<Snap> {
    for _ in 0..64 {
        let s1 = SLOT.seq.load(Ordering::Acquire);
        if s1 == 0 {
            return None;
        }
        if s1 & 1 == 1 {
            std::hint::spin_loop();
            continue;
        }
        let pos_fixed = SLOT.pos_fixed.load(Ordering::Relaxed);
        let seen_us = SLOT.seen_us.load(Ordering::Relaxed);
        let flags = SLOT.flags.load(Ordering::Relaxed);
        let frames = SLOT.frames.load(Ordering::Relaxed);
        let rate = f64::from_bits(SLOT.rate_bits.load(Ordering::Relaxed));
        fence(Ordering::Acquire);
        if SLOT.seq.load(Ordering::Relaxed) != s1 {
            continue;
        }
        if flags & FLAG_CLEARED != 0 {
            return None;
        }
        let pos_s = pos_fixed as f64 / SEC_FIXED;
        return Some(Snap {
            generation: s1 / 2,
            pos_s,
            pos_samples: (pos_s * rate).round() as i64,
            seen_us,
            playing: flags & FLAG_PLAYING != 0,
            has_position: flags & FLAG_SECONDS != 0,
            flags,
            frames,
            rate,
        });
    }
    None
}

/// Held by every test that stores or reads the slot (it is process-global).
#[cfg(test)]
pub(crate) static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// For the tests: forget what the audio thread stored.
#[cfg(test)]
pub fn reset_for_test() {
    SLOT.seq.store(0, Ordering::SeqCst);
    SLOT.owner.store(0, Ordering::SeqCst);
}

// ---------------------------------------------------------------- the mapping

/// How [`page_time`] found its answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mapped {
    /// The position is inside a playback region.
    Inside,
    /// The position is outside every region: the nearest earlier region (or the first one, before all)
    /// gives the offset and the time runs on.
    Outside,
    /// The document has no region: the song position is the page time.
    NoRegions,
}

/// The page time (source time, seconds) of a song position. `regions` are `(source index,
/// region)` pairs of the last round.
///
/// - Inside a region: `start_in_source + (song - start_in_song)`. When several regions cover the
///   position (several sources at the same place, the usual drum session), the lowest source index
///   wins, then the earliest start; the other sources of such a session have the same offset.
/// - Outside every region: the region that ended last before the position gives the offset (the
///   first region, when the position is before all). The playhead runs on in the source time of
///   that region: the page cannot hold a playhead that plays, so it must not be asked to.
/// - No region: the song position.
/// - Never below 0 (the page time starts at 0).
///
/// The length of a region in song time is its length in source time: a region with a time
/// stretch is not handled (the plugin analyses the source, not the stretched audio).
pub fn page_time(song_s: f64, regions: &[(usize, RegionRec)]) -> (f64, Mapped) {
    let usable: Vec<&(usize, RegionRec)> = regions.iter().filter(|(_, r)| r.dur_mod > 0.0).collect();
    if usable.is_empty() {
        return (song_s.max(0.0), Mapped::NoRegions);
    }
    let end = |r: &RegionRec| r.start_play + r.dur_mod;
    let inside = usable
        .iter()
        .filter(|(_, r)| r.start_play <= song_s && song_s < end(r))
        .min_by(|a, b| a.0.cmp(&b.0).then(a.1.start_play.total_cmp(&b.1.start_play)));
    let (pick, kind) = match inside {
        Some(p) => (*p, Mapped::Inside),
        None => {
            let before = usable
                .iter()
                .filter(|(_, r)| end(r) <= song_s)
                .max_by(|a, b| end(&a.1).total_cmp(&end(&b.1)).then(b.0.cmp(&a.0)));
            let p = match before {
                Some(p) => *p,
                None => *usable
                    .iter()
                    .min_by(|a, b| a.1.start_play.total_cmp(&b.1.start_play).then(a.0.cmp(&b.0)))
                    .unwrap_or(&usable[0]),
            };
            (p, Mapped::Outside)
        }
    };
    ((pick.1.start_mod + (song_s - pick.1.start_play)).max(0.0), kind)
}

// ---------------------------------------------------------------- the pacing

/// Why a message goes to the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Why {
    /// The first message of this editor.
    First,
    /// The play state changed.
    State,
    /// The position jumped (a locate, a loop, a move of the mapping).
    Jump,
    /// The regular message while the host plays.
    Tick,
}

/// One message to the page: the interface of `B.hostTransport.onPosition` (without `sid`, which
/// the shell script adds).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Msg {
    pub epoch: u64,
    /// The time of the position, in nanoseconds of the plugin's own monotonic clock.
    pub t_ns: u64,
    /// The page time.
    pub pos: f64,
    pub playing: bool,
    pub why: Why,
    /// The song position of the host at `t_ns`.
    pub song_s: f64,
    pub mapped: Mapped,
}

#[derive(Clone, Copy)]
struct Last {
    mapped_s: f64,
    at_us: u64,
    playing: bool,
}

/// Decides when the page gets a message. One per editor.
pub struct Pacer {
    epoch: u64,
    last: Option<Last>,
    tick_us: u64,
    /// Set once: a playing host went silent (the row of the report).
    pub stale_seen: bool,
    /// Set once: a process call had a transport without a seconds timeline (a row of the report).
    pub no_position_seen: bool,
}

impl Pacer {
    pub fn new(tick_us: u64) -> Pacer {
        Pacer {
            epoch: 0,
            last: None,
            tick_us,
            stale_seen: false,
            no_position_seen: false,
        }
    }

    /// True when a playing host has made no process call for the stale time.
    pub fn is_stale(s: &Snap, now_us: u64) -> bool {
        s.playing && now_us.saturating_sub(s.seen_us) > stale_us(s.frames, s.rate)
    }

    /// One look at the transport. `map` gives the page time of a song position.
    pub fn step(&mut self, snap: Option<Snap>, now_us: u64, map: &dyn Fn(f64) -> (f64, Mapped)) -> Option<Msg> {
        let s = snap?;
        if !s.has_position {
            // the host plays or stops with no position: the old position must not be believed
            self.no_position_seen = true;
            return None;
        }
        let age = now_us.saturating_sub(s.seen_us);
        let stale = Self::is_stale(&s, now_us);
        if stale {
            self.stale_seen = true;
        }
        let playing = s.playing && !stale;
        let song = if playing { s.pos_s + age as f64 / 1e6 } else { s.pos_s };
        let (mapped_s, kind) = map(song);
        let jump_limit = jump_s(s.frames, s.rate);
        let why = match self.last {
            None => Some(Why::First),
            Some(l) if l.playing != playing => Some(Why::State),
            Some(l) if playing => {
                let since = now_us.saturating_sub(l.at_us);
                let expected = l.mapped_s + since as f64 / 1e6;
                if (mapped_s - expected).abs() > jump_limit {
                    // at most one jump message per tick: the newest position goes at the next look
                    (since >= self.tick_us).then_some(Why::Jump)
                } else if since >= self.tick_us {
                    Some(Why::Tick)
                } else {
                    None
                }
            }
            Some(l) => ((mapped_s - l.mapped_s).abs() > STOP_MOVE_S).then_some(Why::Jump),
        }?;
        if why != Why::Tick {
            self.epoch += 1;
        }
        self.last = Some(Last {
            mapped_s,
            at_us: now_us,
            playing,
        });
        Some(Msg {
            epoch: self.epoch,
            t_ns: now_us * 1000,
            pos: mapped_s,
            playing,
            why,
            song_s: song,
            mapped: kind,
        })
    }

    /// The slot is gone (cleared by the owner's `deactivate` or `reset`) or has no position: when the
    /// page was told "playing", it is told "stopped" at the last position it got, once. Without this the
    /// page would run on for ever from the old position.
    pub fn stop_at_last(&mut self, now_us: u64) -> Option<Msg> {
        let l = self.last.filter(|l| l.playing)?;
        self.epoch += 1;
        self.last = Some(Last {
            mapped_s: l.mapped_s,
            at_us: now_us,
            playing: false,
        });
        Some(Msg {
            epoch: self.epoch,
            t_ns: now_us * 1000,
            pos: l.mapped_s,
            playing: false,
            why: Why::State,
            song_s: f64::NAN,
            mapped: Mapped::Inside,
        })
    }

    fn has_sent(&self) -> bool {
        self.last.is_some()
    }
}

/// The poll of one editor: the pacer, the region list, and the rule of when to look and how long
/// to wait. While the host is stopped and nothing changed it does no work and waits [`IDLE_MS`];
/// the region list is fetched only when a look needs it, at most every 200 ms.
pub struct Poller {
    pub pacer: Pacer,
    last_key: Option<(u64, bool, bool)>,
    regions: Vec<(usize, RegionRec)>,
    fetched_at: Option<u64>,
    /// How often the region list was fetched (the unit test counts it).
    pub fetches: usize,
    stale_reported: bool,
    no_position_reported: bool,
    /// The change counter of the regions at the last fetch.
    regions_gen: u64,
    /// The fault run `keep-playing`: no stop message when the slot is cleared (the round 2 way).
    pub no_stop_fault: bool,
}

impl Poller {
    pub fn new(tick_us: u64) -> Poller {
        Poller {
            pacer: Pacer::new(tick_us),
            last_key: None,
            regions: Vec::new(),
            fetched_at: None,
            fetches: 0,
            stale_reported: false,
            no_position_reported: false,
            regions_gen: 0,
            no_stop_fault: false,
        }
    }

    fn stop_msg(&mut self, now_us: u64) -> Option<Msg> {
        if self.no_stop_fault {
            None
        } else {
            self.pacer.stop_at_last(now_us)
        }
    }

    /// The number of regions of the last fetch.
    pub fn region_count(&self) -> usize {
        self.regions.len()
    }

    /// True once, when a playing host went silent.
    pub fn take_stale_row(&mut self) -> bool {
        let new = self.pacer.stale_seen && !self.stale_reported;
        self.stale_reported |= self.pacer.stale_seen;
        new
    }

    /// True once, when the host sent a transport without a position.
    pub fn take_no_position_row(&mut self) -> bool {
        let new = self.pacer.no_position_seen && !self.no_position_reported;
        self.no_position_reported |= self.pacer.no_position_seen;
        new
    }

    /// One look: the message to send, if any, and how long to wait before the next look.
    pub fn look(
        &mut self,
        snap: Option<Snap>,
        now_us: u64,
        regions_gen: u64,
        fetch: &mut dyn FnMut() -> Vec<(usize, RegionRec)>,
    ) -> (Option<Msg>, u64) {
        let Some(s) = snap else {
            return (self.stop_msg(now_us), IDLE_MS);
        };
        let key = (s.pos_s.to_bits(), s.playing, s.has_position);
        let old = self
            .fetched_at
            .is_some_and(|at| now_us.saturating_sub(at) > REGION_TTL_US);
        // stopped and unchanged: nothing to do, unless the regions changed (looked at every 200 ms at
        // the most, and only when the change counter moved)
        if !s.playing
            && self.pacer.has_sent()
            && self.last_key == Some(key)
            && !(old && regions_gen != self.regions_gen)
        {
            return (None, IDLE_MS);
        }
        self.last_key = Some(key);
        if !s.has_position {
            self.pacer.step(Some(s), now_us, &|x| (x, Mapped::NoRegions));
            return (self.stop_msg(now_us), IDLE_MS);
        }
        if self
            .fetched_at
            .is_none_or(|at| now_us.saturating_sub(at) > REGION_TTL_US)
        {
            self.regions = fetch();
            self.regions_gen = regions_gen;
            self.fetched_at = Some(now_us);
            self.fetches += 1;
        }
        let regions = &self.regions;
        let msg = self.pacer.step(Some(s), now_us, &|song| page_time(song, regions));
        let wait = if s.playing && !Pacer::is_stale(&s, now_us) {
            PLAY_MS
        } else {
            IDLE_MS
        };
        (msg, wait)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn reg(a: f64, d: f64, p: f64) -> RegionRec {
        RegionRec {
            start_mod: a,
            dur_mod: d,
            start_play: p,
        }
    }

    #[test]
    fn a_position_inside_a_region_maps_with_its_offsets() {
        let r = [(0, reg(0.4, 2.4, 2.0))];
        assert_eq!(page_time(3.0, &r), (1.4, Mapped::Inside));
        assert_eq!(page_time(2.0, &r).0, 0.4);
    }

    #[test]
    fn a_position_outside_every_region_runs_on_with_the_nearest_earlier_region() {
        let r = [(0, reg(0.4, 2.4, 2.0))];
        // after the end (4.4): the same offset, the time runs on
        let (t, k) = page_time(4.5, &r);
        assert!((t - 2.9).abs() < 1e-12 && k == Mapped::Outside);
        // before the first region: the first region's offset, never below 0
        assert_eq!(page_time(1.0, &r), (0.0, Mapped::Outside));
        let r2 = [(0, reg(1.0, 2.0, 2.0))];
        assert!((page_time(1.5, &r2).0 - 0.5).abs() < 1e-12);
        // two regions with a gap: the position in the gap follows the earlier region
        let two = [(0, reg(0.0, 1.0, 0.0)), (0, reg(5.0, 1.0, 3.0))];
        let (t, k) = page_time(2.0, &two);
        assert!((t - 2.0).abs() < 1e-12 && k == Mapped::Outside);
        assert_eq!(page_time(3.5, &two), (5.5, Mapped::Inside));
    }

    #[test]
    fn the_lowest_source_wins_where_regions_overlap_and_no_region_is_identity() {
        let r = [(1, reg(10.0, 5.0, 0.0)), (0, reg(0.0, 5.0, 0.0))];
        assert_eq!(page_time(1.0, &r), (1.0, Mapped::Inside));
        assert_eq!(page_time(1.25, &[]), (1.25, Mapped::NoRegions));
        assert_eq!(page_time(-1.0, &[]).0, 0.0);
        // a region of length 0 is not used
        assert_eq!(page_time(1.0, &[(0, reg(0.0, 0.0, 0.0))]).1, Mapped::NoRegions);
    }

    fn snap(pos_s: f64, playing: bool, seen_us: u64) -> Snap {
        Snap {
            generation: 1,
            pos_s,
            pos_samples: (pos_s * 48000.0) as i64,
            seen_us,
            playing,
            has_position: true,
            flags: 0,
            frames: 512,
            rate: 48000.0,
        }
    }

    fn ident(s: f64) -> (f64, Mapped) {
        (s, Mapped::Inside)
    }

    #[test]
    fn the_pacer_sends_nothing_without_a_transport_and_nothing_while_stopped_and_unchanged() {
        let mut p = Pacer::new(TICK_US);
        assert!(p.step(None, 1_000, &ident).is_none());
        let first = p.step(Some(snap(2.0, false, 0)), 1_000, &ident).unwrap();
        assert_eq!((first.why, first.epoch, first.playing), (Why::First, 1, false));
        for t in 2..200u64 {
            assert!(p.step(Some(snap(2.0, false, 0)), t * 10_000, &ident).is_none());
        }
    }

    #[test]
    fn the_pacer_sends_state_changes_jumps_and_a_tick_every_40_ms_while_playing() {
        let mut p = Pacer::new(TICK_US);
        p.step(Some(snap(2.0, false, 0)), 0, &ident).unwrap();
        // play: one state message
        let m = p.step(Some(snap(2.0, true, 100_000)), 100_000, &ident).unwrap();
        assert_eq!((m.why, m.epoch, m.playing), (Why::State, 2, true));
        // one second of play: a message every 40 ms, the epoch stays (the page slews)
        let mut n = 0;
        let mut t = 100_000u64;
        while t < 1_100_000 {
            t += 8_000;
            // the host calls process every 10.67 ms: the block start moves with its call time
            let seen = (t / 10_667) * 10_667;
            let pos = 2.0 + (seen - 100_000) as f64 / 1e6;
            if let Some(m) = p.step(Some(snap(pos, true, seen)), t, &ident) {
                assert_eq!((m.why, m.epoch), (Why::Tick, 2), "at {t}");
                n += 1;
            }
        }
        assert!((22..=26).contains(&n), "{n} messages in one second of play");
        // a locate while playing: a jump with a new epoch
        // (a jump goes out at most once per tick: not within 40 ms of the last message)
        assert!(p.step(Some(snap(7.0, true, t)), t + 1_000, &ident).is_none());
        let t = t + 44_000;
        let m = p.step(Some(snap(7.0, true, t)), t + 1_000, &ident).unwrap();
        assert_eq!((m.why, m.epoch), (Why::Jump, 3));
        assert!((m.pos - 7.001).abs() < 1e-9);
        // a stop and a locate while stopped
        let m = p.step(Some(snap(7.2, false, t + 20_000)), t + 21_000, &ident).unwrap();
        assert_eq!((m.why, m.playing), (Why::State, false));
        let m = p.step(Some(snap(1.0, false, t + 40_000)), t + 41_000, &ident).unwrap();
        assert_eq!((m.why, m.epoch, m.pos), (Why::Jump, 5, 1.0));
    }

    #[test]
    fn a_host_that_went_silent_while_playing_counts_as_stopped_at_its_last_position() {
        let mut p = Pacer::new(TICK_US);
        p.step(Some(snap(3.0, true, 0)), 1_000, &ident).unwrap();
        let m = p.step(Some(snap(3.0, true, 0)), STALE_US + 10_000, &ident).unwrap();
        assert_eq!((m.why, m.playing, m.pos), (Why::State, false, 3.0));
        assert!(p.stale_seen);
        // and sends nothing more
        assert!(p.step(Some(snap(3.0, true, 0)), STALE_US + 500_000, &ident).is_none());
    }

    #[test]
    fn a_loop_of_the_host_is_a_jump_each_time_and_a_mapping_step_is_a_jump_too() {
        let mut p = Pacer::new(TICK_US);
        let r = [(0, reg(0.0, 1.0, 0.0)), (0, reg(5.0, 1.0, 1.0))];
        let map = |s: f64| page_time(s, &r);
        p.step(Some(snap(0.9, true, 0)), 0, &map).unwrap();
        // the song passes 1.0: the second region starts, the page time steps from 1.0 to 5.0
        let m = p.step(Some(snap(1.05, true, 150_000)), 150_000, &map).unwrap();
        assert_eq!(m.why, Why::Jump);
        // the host loops back to 0.2
        let m = p.step(Some(snap(0.2, true, 300_000)), 300_000, &map).unwrap();
        assert_eq!(m.why, Why::Jump);
    }

    #[test]
    fn the_audio_thread_stores_only_atomics_and_a_reader_never_sees_a_torn_value() {
        let _g = super::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_for_test();
        // no transport, no seconds timeline: nothing stored
        let mut process: clap_sys::process::clap_process = unsafe { std::mem::zeroed() };
        unsafe { note(&process, 48000.0, 1) };
        assert!(read().is_none());
        let mut t: clap_sys::events::clap_event_transport = unsafe { std::mem::zeroed() };
        let tp: *mut clap_sys::events::clap_event_transport = &mut t;
        process.transport = tp;
        unsafe { note(&process, 48000.0, 1) };
        let s0 = read().expect("a transport without seconds is stored as a play state");
        assert!(!s0.has_position, "no seconds timeline: no position");
        unsafe {
            (*tp).flags = FLAG_SECONDS | FLAG_PLAYING;
            (*tp).song_pos_seconds = (2.5 * SEC_FIXED) as i64;
        }
        process.frames_count = 256;
        unsafe { note(&process, 48000.0, 1) };
        let s = read().expect("stored");
        assert_eq!((s.pos_s, s.pos_samples, s.playing, s.frames), (2.5, 120_000, true, 256));
        // two writers and a reader: the position and the sample count of every read agree
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let writers: Vec<_> = (0..2)
            .map(|_| {
                let stop = stop.clone();
                std::thread::spawn(move || {
                    let mut k = 0i64;
                    while !stop.load(Ordering::Relaxed) {
                        k += 1;
                        // the value 1000 k seconds goes with k + 1000 frames: a torn read breaks this
                        store_at(
                            1,
                            1,
                            (k * 1000) << 31,
                            FLAG_SECONDS,
                            (k + 1000) as u32,
                            48000.0,
                            k as u64,
                        );
                    }
                })
            })
            .collect();
        let mut seen = 0;
        for _ in 0..200_000 {
            if let Some(s) = read() {
                let k = (s.pos_s / 1000.0).round() as i64;
                if k > 0 {
                    assert_eq!(s.frames as i64, k + 1000, "a torn read");
                    assert_eq!(s.seen_us as i64, k, "a torn read");
                    seen += 1;
                }
            }
        }
        stop.store(true, Ordering::Relaxed);
        for w in writers {
            w.join().unwrap();
        }
        assert!(seen > 1000, "{seen} reads");
    }

    thread_local! {
        static ALLOCS: Cell<usize> = const { Cell::new(0) };
    }

    /// Counts the allocations of the calling thread (the test below runs on its own thread).
    struct Counting;
    unsafe impl std::alloc::GlobalAlloc for Counting {
        unsafe fn alloc(&self, l: std::alloc::Layout) -> *mut u8 {
            let _ = ALLOCS.try_with(|c| c.set(c.get() + 1));
            std::alloc::System.alloc(l)
        }
        unsafe fn dealloc(&self, p: *mut u8, l: std::alloc::Layout) {
            std::alloc::System.dealloc(p, l)
        }
    }
    #[global_allocator]
    static COUNTING: Counting = Counting;

    fn allocs() -> usize {
        ALLOCS.with(|c| c.get())
    }

    #[test]
    fn the_audio_thread_call_allocates_nothing_and_takes_no_lock() {
        let _g = super::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut t: clap_sys::events::clap_event_transport = unsafe { std::mem::zeroed() };
        let tp: *mut clap_sys::events::clap_event_transport = &mut t;
        unsafe { (*tp).flags = FLAG_SECONDS | FLAG_PLAYING };
        let mut process: clap_sys::process::clap_process = unsafe { std::mem::zeroed() };
        process.transport = tp;
        process.frames_count = 512;
        read_switches(); // the fault run sets DS_ARA_TRANSPORT_FAULT=alloc
        unsafe { note(&process, 48000.0, 1) }; // the first call makes the clock
        let before = allocs();
        for k in 0..10_000i64 {
            unsafe {
                (*tp).song_pos_seconds = k << 20;
                note(&process, 48000.0, 1);
            }
        }
        assert_eq!(allocs() - before, 0, "the audio thread's call allocated");
        // the slot holds only atomics: its size is the sum of the atomics (no Mutex, no pointer)
        assert_eq!(std::mem::size_of::<Slot>(), 8 * 5 + 4 * 2);
    }

    fn region(a: f64, d: f64, p: f64) -> (usize, RegionRec) {
        (0, reg(a, d, p))
    }

    #[test]
    fn the_tie_break_of_overlapping_regions_is_right_for_a_negative_start() {
        // both regions cover song -0.5; the earlier start (-3.0) must win (to_bits order would pick -1.0)
        let r = [region(0.0, 2.0, -1.0), region(5.0, 4.0, -3.0)];
        let (t, k) = page_time(-0.5, &r);
        assert_eq!(k, Mapped::Inside);
        assert!((t - 7.5).abs() < 1e-12, "{t}");
    }

    fn calls(id: u64, pos: f64, playing: bool, frames: u32) {
        let mut t: clap_sys::events::clap_event_transport = unsafe { std::mem::zeroed() };
        t.flags = FLAG_SECONDS | if playing { FLAG_PLAYING } else { 0 };
        t.song_pos_seconds = (pos * SEC_FIXED) as i64;
        let mut p: clap_sys::process::clap_process = unsafe { std::mem::zeroed() };
        p.transport = &t;
        p.frames_count = frames;
        unsafe { note(&p, 48000.0, id) };
    }

    #[test]
    fn two_instances_at_different_positions_give_one_owner_and_the_other_takes_over_when_it_is_quiet() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_for_test();
        calls(11, 10.0, true, 512);
        let mut seen = std::collections::BTreeSet::new();
        for k in 0..200 {
            calls(22, 500.0 + k as f64, true, 512); // another instance, far away
            calls(11, 10.0 + k as f64 * 0.01, true, 512);
            let s = read().unwrap();
            seen.insert((s.pos_s / 100.0).floor() as i64);
        }
        assert_eq!(
            seen.into_iter().collect::<Vec<_>>(),
            vec![0],
            "the reader saw only the owner"
        );
        // the owner goes quiet for longer than the stale time: the other takes over
        std::thread::sleep(std::time::Duration::from_millis(350));
        calls(22, 500.0, true, 512);
        assert_eq!(read().unwrap().pos_s, 500.0);
        // and the old owner does not take it back while the new one writes
        calls(11, 10.0, true, 512);
        assert_eq!(read().unwrap().pos_s, 500.0);
    }

    #[test]
    fn a_deactivated_owner_clears_the_slot_and_another_instance_does_not() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_for_test();
        calls(11, 10.0, false, 512);
        clear(22); // not the owner
        assert!(read().is_some());
        clear(11);
        assert!(read().is_none(), "a new editor sees no position of the earlier session");
        calls(22, 3.0, false, 512);
        assert_eq!(read().unwrap().pos_s, 3.0);
    }

    #[test]
    fn a_call_without_a_seconds_timeline_stores_the_play_state_and_the_pacer_names_it() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_for_test();
        calls(11, 5.0, false, 512);
        let mut t: clap_sys::events::clap_event_transport = unsafe { std::mem::zeroed() };
        t.flags = FLAG_PLAYING; // plays, no position
        let mut p: clap_sys::process::clap_process = unsafe { std::mem::zeroed() };
        p.transport = &t;
        p.frames_count = 512;
        unsafe { note(&p, 48000.0, 11) };
        let s = read().unwrap();
        assert!(s.playing && !s.has_position);
        let mut pacer = Pacer::new(TICK_US);
        assert!(
            pacer.step(Some(s), 0, &ident).is_none(),
            "the old position 5.0 is not believed"
        );
        assert!(pacer.no_position_seen);
    }

    #[test]
    fn the_stale_time_is_three_blocks_for_large_blocks() {
        assert_eq!(stale_us(512, 48000.0), STALE_US);
        // 16384 frames at 44.1 kHz = 372 ms: three blocks = 1.1 s
        assert!(stale_us(16384, 44100.0) > 1_100_000);
        let mut p = Pacer::new(TICK_US);
        let big = |pos: f64, seen: u64| Snap {
            frames: 16384,
            rate: 44100.0,
            ..snap(pos, true, seen)
        };
        p.step(Some(big(1.0, 0)), 0, &ident).unwrap();
        // 700 ms after the last call of a host with 372 ms blocks: still playing, no flip
        let m = p.step(Some(big(1.0, 0)), 700_000, &ident);
        assert!(m.is_none_or(|m| m.playing), "no flip to stopped");
        assert!(!p.stale_seen);
        assert!(p.step(Some(big(1.0, 0)), 1_300_000, &ident).is_some_and(|m| !m.playing));
    }

    #[test]
    fn a_block_of_23_ms_is_no_jump_and_an_offline_bounce_gives_at_most_25_jumps_a_second() {
        // blocks of 1024 frames at 44.1 kHz: a host that calls two blocks at once, then none
        let snapb = |pos: f64, seen: u64| Snap {
            frames: 1024,
            rate: 44100.0,
            ..snap(pos, true, seen)
        };
        assert!(jump_s(1024, 44100.0) > 0.034);
        let mut p = Pacer::new(TICK_US);
        p.step(Some(snapb(0.0, 0)), 0, &ident).unwrap();
        let mut jumps = 0;
        let mut t = 0u64;
        while t < 2_000_000 {
            t += 8_000;
            // the call comes 23 ms early: the block start is one block ahead of the wall clock
            let pos = t as f64 / 1e6 + 0.0232;
            if let Some(m) = p.step(Some(snapb(pos, t)), t, &ident) {
                if m.why == Why::Jump {
                    jumps += 1;
                }
            }
        }
        assert_eq!(jumps, 0, "a block ahead of the clock is no jump");
        // an offline bounce: the host runs at 10 times real time
        let mut p = Pacer::new(TICK_US);
        p.step(Some(snapb(0.0, 0)), 0, &ident).unwrap();
        let (mut jumps, mut last_jump) = (0, 0u64);
        let mut t = 0u64;
        while t < 2_000_000 {
            t += 8_000;
            let pos = t as f64 / 1e5;
            if let Some(m) = p.step(Some(snapb(pos, t)), t, &ident) {
                if m.why == Why::Jump {
                    jumps += 1;
                    assert!(t - last_jump >= TICK_US, "two jumps {} us apart", t - last_jump);
                    last_jump = t;
                }
            }
        }
        assert!(jumps <= 50, "{jumps} jumps in 2 s"); // 25 a second
        assert!(jumps >= 30, "the bounce still follows: {jumps}");
    }

    #[test]
    fn a_stopped_host_costs_no_region_fetch_and_a_playing_one_at_most_five_a_second() {
        let mut poller = Poller::new(TICK_US);
        let mut fetch = || vec![(0usize, reg(0.0, 3.0, 0.0))];
        // 2 s of a stopped host that calls process every 10 ms with the same position
        let mut t = 0u64;
        let mut msgs = 0;
        let mut looks = 0;
        while t < 2_000_000 {
            let (m, wait) = poller.look(Some(snap(2.0, false, t)), t, 0, &mut fetch);
            msgs += usize::from(m.is_some());
            looks += 1;
            t += wait * 1000;
        }
        assert_eq!(msgs, 1, "the first message only");
        assert!(
            poller.fetches <= 2,
            "{} region fetches in 2 s of a stopped host (bound 2)",
            poller.fetches
        );
        assert!(looks <= 105, "{looks} looks in 2 s: the wait is 20 ms");
        // the host plays for 2 s
        let mut poller = Poller::new(TICK_US);
        let mut t = 0u64;
        while t < 2_000_000 {
            let seen = t - t % 10_000;
            let (_, wait) = poller.look(Some(snap(seen as f64 / 1e6, true, seen)), t, 0, &mut fetch);
            t += wait * 1000;
        }
        assert!(
            poller.fetches <= 11,
            "{} region fetches in 2 s of play (bound 11)",
            poller.fetches
        );
    }

    #[test]
    fn a_cleared_slot_while_the_page_plays_sends_stopped_at_the_last_position_once() {
        let mut poller = Poller::new(TICK_US);
        let mut fetch = || Vec::new();
        let m = poller.look(Some(snap(2.0, true, 0)), 0, 0, &mut fetch).0.unwrap();
        assert!(m.playing);
        let m = poller
            .look(Some(snap(2.04, true, 40_000)), 40_000, 0, &mut fetch)
            .0
            .unwrap();
        assert_eq!(m.why, Why::Tick);
        // the host stopped, made no stopped process call, and deactivated: the slot is gone
        let m = poller.look(None, 80_000, 0, &mut fetch).0.expect("a stop message");
        assert_eq!((m.playing, m.why), (false, Why::State));
        assert!((m.pos - 2.04).abs() < 1e-9, "the last position the page got");
        assert!(poller.look(None, 100_000, 0, &mut fetch).0.is_none(), "once");
        // the fault run: the round 2 way sends nothing
        let mut p2 = Poller::new(TICK_US);
        p2.no_stop_fault = true;
        p2.look(Some(snap(2.0, true, 0)), 0, 0, &mut fetch);
        assert!(p2.look(None, 80_000, 0, &mut fetch).0.is_none());
        // a host that was stopped sends nothing when the slot is cleared
        let mut p3 = Poller::new(TICK_US);
        p3.look(Some(snap(2.0, false, 0)), 0, 0, &mut fetch);
        assert!(p3.look(None, 80_000, 0, &mut fetch).0.is_none());
    }

    #[test]
    fn a_host_that_plays_with_no_position_stops_the_page_once() {
        let mut poller = Poller::new(TICK_US);
        let mut fetch = || Vec::new();
        poller.look(Some(snap(2.0, true, 0)), 0, 0, &mut fetch).0.unwrap();
        let nopos = Snap {
            has_position: false,
            ..snap(0.0, true, 50_000)
        };
        let m = poller
            .look(Some(nopos), 50_000, 0, &mut fetch)
            .0
            .expect("a stop message");
        assert!(!m.playing && (m.pos - 2.0).abs() < 1e-9);
        assert!(poller.take_no_position_row());
        assert!(poller
            .look(
                Some(Snap {
                    seen_us: 60_000,
                    ..nopos
                }),
                60_000,
                0,
                &mut fetch
            )
            .0
            .is_none());
        assert!(!poller.take_no_position_row(), "the row is written once");
    }

    #[test]
    fn a_change_of_the_regions_while_stopped_is_fetched_once_after_the_wait() {
        let mut poller = Poller::new(TICK_US);
        let shift = std::cell::Cell::new(0.0);
        let mut fetch = || vec![(0usize, reg(0.0 + shift.get(), 5.0, 0.0))];
        let m = poller.look(Some(snap(2.0, false, 0)), 0, 1, &mut fetch).0.unwrap();
        assert_eq!(m.pos, 2.0);
        shift.set(1.0); // the regions changed: the counter moves
                        // within 200 ms: no fetch
        assert!(poller
            .look(Some(snap(2.0, false, 0)), 100_000, 2, &mut fetch)
            .0
            .is_none());
        assert_eq!(poller.fetches, 1);
        // after 200 ms: one fetch and one message with the new mapping
        let m = poller
            .look(Some(snap(2.0, false, 0)), 250_000, 2, &mut fetch)
            .0
            .expect("the new mapping");
        assert_eq!((m.pos, poller.fetches), (3.0, 2));
        // and then nothing, and no fetch on every look
        for k in 0..50 {
            assert!(poller
                .look(Some(snap(2.0, false, 0)), 300_000 + k * 20_000, 2, &mut fetch)
                .0
                .is_none());
        }
        assert_eq!(poller.fetches, 2);
    }
}
