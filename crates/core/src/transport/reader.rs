//! The reader side of the transport: one source per track that decodes blocks at the
//! track's native rate and format, folds them with the track's [`MixMode`], resamples to
//! the device rate when the rates differ, and fills the track's ring.
//!
//! P3-16: a 2-channel file is not folded for playback: the source keeps left and right
//! (interleaved in the ring, two samples per frame, each converted as a mono file's sample)
//! and the resampler runs on both channels. A file with 1 or more than 2 channels plays the
//! mono fold of its [`MixMode`] as before. Detection, the waveform and the exports read the
//! mono fold elsewhere and do not change.
//!
//! One reader thread runs [`Reader::step`] in a loop. It never touches the audio callback's
//! state except through the rings and the atomics of [`Shared`].
//!
//! Residency: with [`Residency::Pread`] (the default) the reader parses the header with
//! [`MappedWav`] and then reads the data with positioned reads (`read_at`) of at least
//! [`READ_BLOCK`] bytes into a small per-track buffer, so no file page stays in the process
//! (P3-15a: one read per 256 KiB instead of one `lseek` and `read` per 1,024 frames). With [`Residency::Madvise`] the source stays mapped
//! and the reader releases the mapped pages behind its read position
//! (`MappedWav::release`). On macOS that does not lower the resident set: measured on
//! arm64-mac, RSS grew by the read rate (124 MB in 15 s for 30 tracks at 96 kHz 24-bit), so
//! the default is `Pread` (0 MB growth in the same run).
//!
//! Half speed (P3-20b): the source resamples to an output rate of twice the device rate
//! ([`TrackSource::set_out_rate`]); a track at that rate (96 kHz on a 48 kHz device) needs no
//! resampler then. The resampler of each output rate is built on first use and kept.
//!
//! Resampling (device rate differs from the track rate): rubato's synchronous FFT resampler
//! with fixed input and output blocks. A seek to device frame `P` starts the input on the
//! resampler's block grid some blocks before `P` (so the filter has real history) and drops
//! the output before `P` plus the resampler delay; the result does not depend on where the
//! previous position was.

use std::fs::File;
use std::os::unix::fs::FileExt;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;

use rtrb::Producer;
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler, WindowFunction};

use super::{Shared, Speed, TransportError};
use crate::io::{MappedWav, MixMode, Plan, WavInfo};

/// How the reader keeps the source bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Residency {
    /// Memory-map the file and release the pages behind the read position (the RSS does
    /// not drop on macOS; see the module docs).
    Madvise,
    /// Positioned reads into a small buffer; no mapping (the default).
    Pread,
}

/// Release mapped pages once at least this many bytes lie behind the read position.
const RELEASE_STEP: usize = 1 << 20;
/// The reader refills a ring when at least this many slots are free.
pub(crate) const REFILL_MIN: usize = 8_192;
/// Resampler block size hint (input frames per call).
const RESAMPLE_CHUNK: usize = 1_024;
/// Smallest positioned read of [`Residency::Pread`] (bytes); a larger request reads its own size.
pub const READ_BLOCK: usize = 256 * 1024;
/// Reader sleep when no ring needs data (P3-15a; the rings hold 2.7 s at 48 kHz).
pub const IDLE_SLEEP: std::time::Duration = std::time::Duration::from_millis(10);
/// Reader sleep while it waits for the callback during a seek handshake.
pub const SEEK_SLEEP: std::time::Duration = std::time::Duration::from_millis(1);

/// The bytes `[from, from + len)` of the file, read ahead with one positioned read.
struct ReadAhead {
    buf: Vec<u8>,
    from: usize,
    len: usize,
}

impl ReadAhead {
    /// The bytes `[from, to)`: from the buffer when it holds them, else after one read of
    /// `max(to - from, READ_BLOCK)` bytes from `from` (not past `end`, the end of the data).
    /// When that read fails (a file shorter than its header), it reads `[from, to)` alone, as
    /// the reader did before P3-15a, so a short file gives the same zeros. `None`: that read
    /// failed too.
    fn get(&mut self, file: &File, from: usize, to: usize, end: usize, reads: &mut u64) -> Option<&[u8]> {
        if from < self.from || to > self.from + self.len {
            let len = (to - from).max(READ_BLOCK).min(end.max(to) - from);
            if self.buf.len() < len {
                self.buf.resize(len, 0);
            }
            *reads += 1;
            self.len = 0;
            if file.read_exact_at(&mut self.buf[..len], from as u64).is_ok() {
                self.len = len;
            } else if len > to - from {
                *reads += 1;
                if file.read_exact_at(&mut self.buf[..to - from], from as u64).is_ok() {
                    self.len = to - from;
                }
            }
            self.from = from;
            if self.len == 0 {
                return None;
            }
        }
        Some(&self.buf[from - self.from..to - self.from])
    }
}

enum Bytes {
    Mapped(Box<MappedWav>),
    File { file: File, ahead: ReadAhead, reads: u64 },
}

struct Resample {
    fft: Fft<f32>,
    in_chunk: usize,
    out_chunk: usize,
    /// Output frames per grid unit and input frames per grid unit (rates over their gcd).
    out_unit: u64,
    in_unit: u64,
    /// Grid units of priming before a seek target.
    prime_units: u64,
    /// Output samples (frames times channels); `out_len`, `out_read` and `skip` count frames.
    out_buf: Vec<f32>,
    out_len: usize,
    out_read: usize,
    skip: usize,
}

/// One track: decoder, optional resampler and read position.
pub struct TrackSource {
    info: WavInfo,
    plan: Plan,
    bytes: Bytes,
    track_rate: u32,
    device_rate: u32,
    /// P3-20b: the rate of the frames in the ring: the device rate, or twice it at half speed.
    out_rate: u32,
    /// Next track frame to decode (negative while the resampler is primed before frame 0).
    in_pos: i64,
    released_to: usize,
    in_buf: Vec<f32>,
    rs: Option<Resample>,
    /// P3-20b: the resamplers of the other output rates used so far (rate, resampler).
    spare: Vec<(u32, Option<Resample>)>,
    /// P3-20b: the resampler of the output rate could not be built: the source gives zeros.
    silent: bool,
    /// Channels in the ring: 2 for a stereo file, else 1 (the mono fold).
    chans: usize,
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 { a } else { gcd(b, a % b) }
}

impl Resample {
    /// The resampler from `track_rate` to `out_rate` for `chans` channels, or `None` when the
    /// rates are equal.
    fn new(track_rate: u32, out_rate: u32, chans: usize) -> Result<Option<Resample>, TransportError> {
        if track_rate == out_rate {
            return Ok(None);
        }
        let g = gcd(u64::from(track_rate), u64::from(out_rate));
        let out_unit = u64::from(out_rate) / g;
        let in_unit = u64::from(track_rate) / g;
        // FFT blocks of an even input size near 512 frames on the rates' grid: with an odd
        // size the true delay is half an input frame off `output_delay()` (measured: a
        // 0.54-frame shift for 44.1 to 48 kHz with rubato's default 1029-frame block).
        let mut m = (512 / in_unit as usize).max(1);
        if (in_unit as usize * m) % 2 == 1 {
            m += 1;
        }
        let sub = in_unit as usize * m;
        let n_sub = (RESAMPLE_CHUNK / sub).max(1);
        let fft = Fft::<f32>::new_custom(
            track_rate as usize,
            out_rate as usize,
            sub * n_sub,
            n_sub,
            chans,
            WindowFunction::BlackmanHarris2,
            FixedSync::Both,
        )
        .map_err(|e| TransportError::Track(format!("resampler {track_rate} to {out_rate} Hz: {e}")))?;
        if fft.fft_size_in() % 2 == 1 {
            return Err(TransportError::Track(format!(
                "resampler {track_rate} to {out_rate} Hz: odd FFT size {}",
                fft.fft_size_in()
            )));
        }
        let out_chunk = fft.output_frames_next();
        let in_chunk = fft.input_frames_next();
        // history of at least twice the resampler block before the target
        let need = 2 * (2 * fft.output_delay() as u64).max(out_chunk as u64);
        let prime_units = need.div_ceil(out_unit);
        Ok(Some(Resample {
            in_chunk,
            out_chunk,
            out_unit,
            in_unit,
            prime_units,
            out_buf: vec![0.0; fft.output_frames_max() * chans],
            out_len: 0,
            out_read: 0,
            skip: 0,
            fft,
        }))
    }
}

impl TrackSource {
    /// Open `path` for playback at `device_rate`: a stereo file keeps both channels, any
    /// other file is folded with `mode`.
    ///
    /// # Errors
    ///
    /// As [`TrackSource::open_at`] with `out_rate` equal to `device_rate`.
    pub fn open(
        path: &Path,
        mode: MixMode,
        device_rate: u32,
        residency: Residency,
    ) -> Result<TrackSource, TransportError> {
        Self::open_at(path, mode, device_rate, device_rate, residency)
    }

    /// P3-20b: [`TrackSource::open`] with the frames resampled to `out_rate` (twice the
    /// device rate at half speed, see [`super::Speed::render_rate`]). [`TrackSource::device_frames`]
    /// still counts frames at `device_rate`.
    ///
    /// # Errors
    ///
    /// [`TransportError::Track`] when the file cannot be mapped or opened as a WAV, has no
    /// mix plan for `mode`, or the track rate, `device_rate` or `out_rate` is 0; also the
    /// error of building the resampler for the track rate and `out_rate`.
    pub fn open_at(
        path: &Path,
        mode: MixMode,
        device_rate: u32,
        out_rate: u32,
        residency: Residency,
    ) -> Result<TrackSource, TransportError> {
        let map = MappedWav::open(path).map_err(|e| TransportError::Track(format!("{}: {e}", path.display())))?;
        let info = map.info.clone();
        let plan = info
            .mix_plan(mode)
            .map_err(|e| TransportError::Track(format!("{}: {e}", path.display())))?;
        let bytes = match residency {
            Residency::Madvise => Bytes::Mapped(Box::new(map)),
            Residency::Pread => {
                drop(map);
                let file = File::open(path).map_err(|e| TransportError::Track(format!("{}: {e}", path.display())))?;
                Bytes::File {
                    file,
                    ahead: ReadAhead {
                        buf: Vec::new(),
                        from: 0,
                        len: 0,
                    },
                    reads: 0,
                }
            }
        };
        let track_rate = info.sample_rate;
        let chans = if info.channels == 2 { 2 } else { 1 };
        if track_rate == 0 || device_rate == 0 || out_rate == 0 {
            return Err(TransportError::Track(format!("{}: sample rate 0", path.display())));
        }
        let rs = Resample::new(track_rate, out_rate, chans)?;
        let mut s = TrackSource {
            info,
            plan,
            bytes,
            track_rate,
            device_rate,
            out_rate,
            in_pos: 0,
            released_to: 0,
            in_buf: Vec::new(),
            rs,
            spare: Vec::new(),
            silent: false,
            chans,
        };
        s.seek(0);
        Ok(s)
    }

    /// P3-20b: resample to `out_rate` from now on (the reader calls it at a speed change, then
    /// [`TrackSource::seek`]). The resampler of the old rate is kept for the next change back,
    /// so each rate builds its resampler once. When the resampler cannot be built, the source
    /// gives silence until the next change and the error is returned.
    ///
    /// # Errors
    ///
    /// [`TransportError::Track`] when `out_rate` is 0; otherwise the error of building a
    /// resampler from the track rate to `out_rate` (none when `out_rate` is unchanged or a
    /// resampler for it is kept).
    pub fn set_out_rate(&mut self, out_rate: u32) -> Result<(), TransportError> {
        if out_rate == self.out_rate {
            return Ok(());
        }
        let old = (self.out_rate, self.rs.take());
        let found = self.spare.iter().position(|(r, _)| *r == out_rate);
        let next = match found {
            Some(i) => Ok(self.spare.swap_remove(i).1),
            None if out_rate == 0 => Err(TransportError::Track("sample rate 0".into())),
            None => Resample::new(self.track_rate, out_rate, self.chans),
        };
        if !self.silent {
            self.spare.push(old);
        }
        self.out_rate = out_rate;
        match next {
            Ok(rs) => {
                self.rs = rs;
                self.silent = false;
                Ok(())
            }
            Err(e) => {
                self.silent = true;
                Err(e)
            }
        }
    }

    /// P3-20b: the rate of the frames [`TrackSource::fill`] gives (the device rate at normal speed).
    pub fn out_rate(&self) -> u32 {
        self.out_rate
    }

    /// The track's own sample rate.
    pub fn track_rate(&self) -> u32 {
        self.track_rate
    }

    /// Track length in device frames (rounded up).
    pub fn device_frames(&self) -> u64 {
        (self.info.n_frames as u64 * u64::from(self.device_rate)).div_ceil(u64::from(self.track_rate))
    }

    /// Channels the source puts in its ring per frame: 2 for a stereo file, else 1.
    pub fn channels(&self) -> usize {
        self.chans
    }

    /// Whether this track is resampled.
    pub fn resampled(&self) -> bool {
        self.rs.is_some()
    }

    /// Position the source so the next [`TrackSource::fill`] starts at output frame `frame` (a
    /// frame at [`TrackSource::out_rate`]; the device frame at normal speed).
    pub fn seek(&mut self, frame: u64) {
        match &mut self.rs {
            None => self.in_pos = frame.min(i64::MAX as u64) as i64,
            Some(r) => {
                let b = frame / r.out_unit;
                let b0 = b as i64 - r.prime_units as i64;
                self.in_pos = b0 * r.in_unit as i64;
                r.fft.reset();
                r.out_len = 0;
                r.out_read = 0;
                // output frame m of the resampler is device frame b0 * out_unit + m - delay
                r.skip = (frame as i64 - b0 * r.out_unit as i64) as usize + r.fft.output_delay();
            }
        }
        let start = self.in_pos.max(0) as usize;
        self.released_to = self.info.data_offset + start.min(self.info.n_frames) * self.info.block_align;
    }

    /// Decode `out_len` track frames from `self.in_pos` into `in_buf` (`chans` samples per
    /// frame; zeros outside the track) and advance; release the pages behind the read position.
    fn decode(&mut self, out_len: usize) {
        let n = self.info.n_frames as i64;
        let ba = self.info.block_align;
        let pos = self.in_pos;
        let c = self.chans;
        let mut buf = std::mem::take(&mut self.in_buf);
        buf.clear();
        buf.resize(out_len * c, 0.0);
        let a = pos.clamp(0, n);
        let b = (pos + out_len as i64).clamp(0, n);
        if b > a {
            let (a, b) = (a as usize, b as usize);
            let dst = &mut buf[(a as i64 - pos) as usize * c..(b as i64 - pos) as usize * c];
            let stereo = c == 2;
            let from = self.info.data_offset + a * ba;
            let to = self.info.data_offset + b * ba;
            match &mut self.bytes {
                Bytes::Mapped(m) => {
                    if stereo {
                        self.info.decode_channels_into(&m.bytes()[from..to], dst);
                    } else {
                        self.info.decode_frames_into(&self.plan, &m.bytes()[from..to], dst);
                    }
                    if to - self.released_to >= RELEASE_STEP {
                        m.release(self.released_to, to);
                        self.released_to = to;
                    }
                }
                Bytes::File { file, ahead, reads } => {
                    let end = self.info.data_offset + self.info.n_frames * ba;
                    match ahead.get(file, from, to, end, reads) {
                        Some(raw) if stereo => {
                            self.info.decode_channels_into(raw, dst);
                        }
                        Some(raw) => {
                            self.info.decode_frames_into(&self.plan, raw, dst);
                        }
                        None => dst.fill(0.0),
                    }
                }
            }
        }
        self.in_pos = pos + out_len as i64;
        self.in_buf = buf;
    }

    /// Positioned reads so far ([`Residency::Pread`]; 0 for a mapped source).
    pub fn reads(&self) -> u64 {
        match &self.bytes {
            Bytes::File { reads, .. } => *reads,
            Bytes::Mapped(_) => 0,
        }
    }

    /// Fill `out` with the next device frames (`out.len() / channels()` of them, interleaved;
    /// zeros past the end of the track). Returns the nanoseconds spent inside the resampler.
    ///
    /// # Panics
    ///
    /// Panics (an `expect` or `unwrap`) only if the resampler's fixed adapter or block sizes
    /// are inconsistent, or the resampler is missing inside the resampling loop. Neither
    /// happens while the source's own invariants hold.
    pub fn fill(&mut self, out: &mut [f32]) -> u64 {
        let c = self.chans;
        let frames = out.len() / c;
        let out = &mut out[..frames * c];
        if self.silent {
            out.fill(0.0);
            return 0;
        }
        if self.rs.is_none() {
            self.decode(frames);
            out.copy_from_slice(&self.in_buf);
            return 0;
        }
        let mut ns = 0u64;
        let mut done = 0;
        while done < frames {
            let r = self.rs.as_mut().unwrap();
            if r.out_read < r.out_len {
                let k = (r.out_len - r.out_read).min(frames - done);
                out[done * c..(done + k) * c].copy_from_slice(&r.out_buf[r.out_read * c..(r.out_read + k) * c]);
                r.out_read += k;
                done += k;
                continue;
            }
            let in_chunk = r.in_chunk;
            self.decode(in_chunk);
            let r = self.rs.as_mut().unwrap();
            let t0 = Instant::now();
            let input = InterleavedSlice::new(&self.in_buf[..in_chunk * c], c, in_chunk).expect("input adapter");
            let out_chunk = r.out_chunk;
            let mut output =
                InterleavedSlice::new_mut(&mut r.out_buf[..out_chunk * c], c, out_chunk).expect("output adapter");
            let (_, produced) = r
                .fft
                .process_into_buffer(&input, &mut output, None)
                .expect("resampler block sizes are fixed");
            ns += t0.elapsed().as_nanos() as u64;
            r.out_len = produced;
            let s = r.skip.min(produced);
            r.out_read = s;
            r.skip -= s;
        }
        ns
    }
}

/// The reader: all track sources and the producer side of their rings.
pub struct Reader {
    sources: Vec<TrackSource>,
    producers: Vec<Producer<f32>>,
    scratch: Vec<f32>,
    shared: Arc<Shared>,
    seen_gen: u64,
    waiting: bool,
    /// #166: the armed loop region in output frames at the render rate (start, exclusive end);
    /// the reader wraps its read position there so the rings carry the region again and again
    /// with no flush and no gap. `None`: no loop.
    loop_range: Option<(u64, u64)>,
    /// #166: the next output frame each source would produce (at its render rate); the reader
    /// tracks it to hit the wrap exactly (a `fill` always produces the frames it is asked for).
    next_out: Vec<u64>,
}

impl Reader {
    pub(crate) fn new(sources: Vec<TrackSource>, producers: Vec<Producer<f32>>, shared: Arc<Shared>) -> Reader {
        let cap = producers.iter().map(|p| p.buffer().capacity()).max().unwrap_or(0);
        // #166: one "next output frame" per source (at its render rate)
        let next_out = vec![0u64; sources.len()];
        Reader {
            sources,
            producers,
            scratch: vec![0.0; cap],
            shared,
            seen_gen: 0,
            waiting: false,
            loop_range: None,
            next_out,
        }
    }

    /// One pass: follow a pending seek, then refill every ring with enough free slots.
    /// Returns true when it pushed anything.
    pub fn step(&mut self) -> bool {
        let sh = &self.shared;
        let g = sh.seek_req.load(Ordering::Acquire);
        if g != self.seen_gen {
            // stop pushing old data; the callback drains the rings once it sees this
            self.seen_gen = g;
            self.waiting = true;
            sh.reader_paused.store(g, Ordering::Release);
        }
        if self.waiting {
            if sh.cb_flushed.load(Ordering::Acquire) != g {
                return false;
            }
            let target = sh.flushed_target.load(Ordering::Acquire);
            // P3-20b: the speed of the seek (a speed change is a seek); the callback stores it before
            // `cb_flushed`
            let speed = Speed::from_slowdown(sh.flushed_speed.load(Ordering::Acquire));
            // #166: the loop region the callback flushed with the seek, in output frames at the
            // render rate (the callback stored it converted); a seek without one ends a loop
            self.loop_range = sh
                .flushed_loop
                .load(Ordering::Acquire)
                .then(|| {
                    (
                        sh.flushed_loop_range.0.load(Ordering::Acquire),
                        sh.flushed_loop_range.1.load(Ordering::Acquire),
                    )
                })
                .filter(|(ls, le)| le > ls);
            for (s, next) in self.sources.iter_mut().zip(self.next_out.iter_mut()) {
                // a failed resampler leaves that track silent (see `set_out_rate`)
                let _ = s.set_out_rate(speed.render_rate(s.device_rate));
                // a loop starts wrapped when the target is at or past the region end, exactly
                // like the engine wraps its playhead (the content and the counter stay in step)
                let t = match self.loop_range {
                    Some((ls, le)) if target >= le => ls + (target - ls) % (le - ls),
                    _ => target,
                };
                s.seek(t);
                *next = t;
            }
            self.waiting = false;
        }
        let t0 = Instant::now();
        let mut wrote = false;
        let mut rs_ns = 0;
        for (i, (src, prod)) in self.sources.iter_mut().zip(self.producers.iter_mut()).enumerate() {
            let c = src.channels();
            // #430: frames the engine rendered as silence while this ring was dry are late:
            // skip them in the source, so the next frames sit at the engine position again
            // (the engine drops the stale frames the ring already holds, see the mixer).
            let mut lag = sh.ring_lag[i].swap(0, Ordering::Acquire);
            while lag > 0 {
                // wrap at the loop end before it is crossed, exactly like the push below
                if let Some((ls, le)) = self.loop_range
                    && self.next_out[i] >= le
                {
                    src.seek(ls);
                    self.next_out[i] = ls;
                }
                let room = match self.loop_range {
                    Some((_, le)) => (le - self.next_out[i]).min(lag),
                    None => lag,
                };
                let d = room.min((self.scratch.len() / c) as u64).max(1);
                let buf = &mut self.scratch[..d as usize * c];
                rs_ns += src.fill(buf);
                self.next_out[i] += d;
                lag -= d;
            }
            let free = prod.slots() / c * c; // whole frames
            if free >= REFILL_MIN * c {
                let mut left = free / c; // frames still to fill in this pass
                while left > 0 {
                    // #166: wrap at the loop end before it is crossed, so the ring carries the
                    // region again without a flush (no gap at the wrap point)
                    if let Some((ls, le)) = self.loop_range
                        && self.next_out[i] >= le
                    {
                        src.seek(ls);
                        self.next_out[i] = ls;
                    }
                    let want = match self.loop_range {
                        Some((_, le)) => (le - self.next_out[i]).min(left as u64),
                        None => left as u64,
                    };
                    debug_assert!(want >= 1);
                    let buf = &mut self.scratch[..want as usize * c];
                    rs_ns += src.fill(buf);
                    let (_, rest) = prod.push_partial_slice(buf);
                    debug_assert!(rest.is_empty());
                    self.next_out[i] += want;
                    left -= want as usize;
                }
                wrote = true;
            }
        }
        if wrote {
            sh.stats
                .reader_ns
                .fetch_add(t0.elapsed().as_nanos() as u64, Ordering::Relaxed);
            sh.stats.resample_ns.fetch_add(rs_ns, Ordering::Relaxed);
        }
        wrote
    }

    /// Run until `shared.stop` is set (the reader thread's body). The thread runs at `QoS`
    /// user-interactive. When no ring needs data it parks for [`IDLE_SLEEP`], during a seek
    /// handshake for [`SEEK_SLEEP`]; `Transport::seek` and the drop of the transport unpark it.
    pub(crate) fn run(mut self) {
        set_reader_qos();
        while !self.shared.stop.load(Ordering::Acquire) {
            if !self.step() {
                std::thread::park_timeout(if self.waiting { SEEK_SLEEP } else { IDLE_SLEEP });
            }
        }
    }
}

/// Ask the system to run the calling thread at `QoS` user-interactive (the reader thread,
/// P3-15a); macOS then keeps it on a performance core under load. Returns whether the call
/// succeeded (always false outside macOS).
pub fn set_reader_qos() -> bool {
    #[cfg(target_os = "macos")]
    {
        // SAFETY: the call changes only the QoS class of the calling thread.
        unsafe { libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_USER_INTERACTIVE, 0) == 0 }
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

#[cfg(test)]
mod tests;
