//! The callback side of the transport: pop one block per track from the rings, mix to
//! stereo with the per-track gains, volume and pan, add the hit clicks, apply the master
//! volume and the master gain, and keep the peak meters.
//!
//! Mix order (the offline tests rebuild it bit for bit): for each frame and channel
//! `c` (left, right), `s_c = 0`, then `s_c += (gain_t * vol_t,c) * x_t` for the tracks in order,
//! then `s_c += gain_t * click_t` for the tracks in order, then
//! `out_c = (s_c * master_volume) * master_gain`. `gain_t` is mute and solo (or
//! [`super::Transport::set_track_gain`]); `vol_t,c` is the track's volume and pan
//! ([`channel_gains`]); at 0 dB and centre both are exactly 1.0, so the output is the
//! output of P3-5a bit for bit (left and right equal). A mono device gets
//! `(s_l + s_r) * 0.5`, which is `s` itself when left and right are equal. Tracks sum at unity
//! like the WebAudio graph of `app.js` (no 1/N scaling). The clicks follow mute and solo, not
//! the volume and pan (they are the metronome). The callback never allocates, locks or does
//! I/O.
//!
//! Stereo tracks (P3-16): a 2-channel file comes from the ring as left and right
//! ([`super::reader`]); its left goes only to the left mix and its right only to the right
//! mix, `s_l += (gain_t * vol_t,l) * x_t,l`, `s_r += (gain_t * vol_t,r) * x_t,r`: the pan control
//! is a balance with the same law ([`pan_gains`]; the centre keeps both sides at unity).
//!
//! Meters (P3-16): a mono track gives the peak of `|x_t| * volume_t` (post-fader, before the
//! pan) in both slots; a stereo track the peak of each side after volume and balance. Both are
//! before mute and solo, so a muted track still shows its level. Per channel, the peak of
//! `s_c * master_volume` (post-fader, before the master gain, which the tests set to 0 for
//! silence). The callback keeps the peaks of each callback in its own buffer and folds them
//! into atomics with `fetch_max` on the bits (a positive `f32` orders like its bits); the
//! control side takes them with `swap(0)`.
//!
//! Half speed (P3-20b): the engine counts output frames (`frame_pos`); the rings hold frames at
//! the render rate, so one output frame is half a session frame. A seek request carries the
//! speed; the callback takes it with the target, and publishes `frame_pos / slowdown` as the
//! session frame. The mix itself does not change.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use rtrb::{Consumer, Producer};

use super::clicks::TrackClicks;
use super::clock::Clock;
use super::layer::LayerBank;
use super::{Shared, Speed};

/// The largest block the engine mixes in one pass; longer device buffers are split.
pub const MAX_BLOCK: usize = 4_096;

/// The click banks the control side hands to the callback.
pub type ClickBank = Vec<TrackClicks>;

/// The top of the volume range in dB (Pro Tools faders end at +12 dB).
pub const MAX_DB: f32 = 12.0;
/// At or below this volume in dB the track is off (the fader's -inf).
pub const MIN_DB: f32 = -120.0;

/// Volume in dB to a linear gain: 0 dB is exactly 1.0, [`MIN_DB`] and below (and NaN) is 0.0,
/// above [`MAX_DB`] is the gain of [`MAX_DB`].
pub fn db_to_gain(db: f32) -> f32 {
    if db.is_nan() || db <= MIN_DB {
        return 0.0;
    }
    10f64.powf(db.min(MAX_DB) as f64 / 20.0) as f32
}

/// The pan law: constant power with -3 dB at the centre against a hard pan, normalised so
/// the centre is unity (Logic's "-3 dB compensated"): `pan` -1 (left) to +1 (right),
/// `theta = (pan + 1) * pi / 4`, left `sqrt(2) cos theta`, right `sqrt(2) sin theta`. The centre is
/// exactly (1.0, 1.0); a hard pan gives +3 dB on its side and 0.0 on the other. A value outside
/// -1 to +1 is clamped; NaN is the centre.
pub fn pan_gains(pan: f32) -> (f32, f32) {
    if pan.is_nan() || pan == 0.0 {
        return (1.0, 1.0);
    }
    let p = pan.clamp(-1.0, 1.0) as f64;
    let theta = (p + 1.0) * std::f64::consts::FRAC_PI_4;
    // cos(pi / 2) is 6e-17 in f64: a hard pan gives exactly 0.0 on the other side
    let side = |v: f64| {
        if v.abs() < 1e-9 {
            0.0
        } else {
            (std::f64::consts::SQRT_2 * v) as f32
        }
    };
    (side(theta.cos()), side(theta.sin()))
}

/// The left and right gain of a track at `volume_db` and `pan` ([`db_to_gain`] times
/// [`pan_gains`]); 0 dB at the centre is exactly (1.0, 1.0).
pub fn channel_gains(volume_db: f32, pan: f32) -> (f32, f32) {
    let g = db_to_gain(volume_db);
    let (l, r) = pan_gains(pan);
    (g * l, g * r)
}

/// The largest absolute value of `x` (0.0 for an empty slice; NaN is ignored).
fn abs_peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0f32, |m, v| m.max(v.abs()))
}

/// Fold `v` into the peak atomic `a` (bits of a positive `f32` order like the value).
fn store_peak(a: &std::sync::atomic::AtomicU32, v: f32) {
    if v > 0.0 {
        a.fetch_max(v.to_bits(), Ordering::Relaxed);
    }
}

/// Mute and solo to per-track gains (`audible()` in `app.js`): with any solo, the soloed
/// tracks play; otherwise every track that is not muted plays. Missing entries are false.
pub fn audible_gains(n: usize, mute: &[bool], solo: &[bool]) -> Vec<f32> {
    let any_solo = solo.iter().take(n).any(|s| *s);
    (0..n)
        .map(|i| {
            let s = solo.get(i).copied().unwrap_or(false);
            let m = mute.get(i).copied().unwrap_or(false);
            if s || (!any_solo && !m) { 1.0 } else { 0.0 }
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// A seek is pending: wait for the reader to stop, then drain the rings.
    Flushing,
    /// Wait until every ring holds the prefill.
    Refilling,
    /// Play from the rings.
    Ready,
}

/// The audio callback's state.
pub struct Engine {
    consumers: Vec<Consumer<f32>>,
    scratch: Vec<Vec<f32>>,
    mix: Vec<f32>,
    mix_r: Vec<f32>,
    gains: Vec<f32>,
    /// Per track left, right, volume: the gains read for this callback.
    vols: Vec<f32>,
    /// Per track the channels in its ring (1 or 2).
    chans: Vec<usize>,
    /// #430: scratch for the stale frames dropped from a recovered ring.
    drop_buf: Vec<f32>,
    /// Per track left, right, then master left, right: the peaks of this callback.
    peaks: Vec<f32>,
    clicks: Box<ClickBank>,
    click_rx: Consumer<Box<ClickBank>>,
    click_ret: Producer<Box<ClickBank>>,
    /// P4-7a: the layer voices per track (empty until the control side sends a bank).
    layers: Box<LayerBank>,
    layer_rx: Option<Consumer<Box<LayerBank>>>,
    layer_ret: Option<Producer<Box<LayerBank>>>,
    shared: Arc<Shared>,
    clock: Arc<Clock>,
    /// The output frame of the next sample (the session frame times the slowdown).
    frame_pos: u64,
    /// #166: the armed loop region in output frames at the render rate (start, exclusive end);
    /// the engine wraps `frame_pos` there so clicks, layer voices and the playhead follow the
    /// looped content. `None`: no loop.
    loop_range: Option<(u64, u64)>,
    /// #201: the window end of an audition in output frames at the render rate (exclusive);
    /// the engine delivers every frame before it and pauses there, so the end is exact and
    /// waits for the delivery under load. `None`: no end.
    end_at: Option<u64>,
    /// P3-20b: the speed of the last seek taken.
    speed: Speed,
    state: State,
    seen_gen: u64,
    /// The tag of the last seek taken; published with every playhead (P3-15f).
    seek_tag: u64,
    seen_gain_seq: u64,
    prefill: usize,
}

impl Engine {
    /// `rings`: per track the consumer of its ring and the channels per frame in it.
    pub(crate) fn new(
        rings: Vec<(Consumer<f32>, usize)>,
        clicks: Box<ClickBank>,
        click_rx: Consumer<Box<ClickBank>>,
        click_ret: Producer<Box<ClickBank>>,
        shared: Arc<Shared>,
        clock: Arc<Clock>,
        prefill: usize,
    ) -> Engine {
        // P3-20b: the speed at open is the speed of the shared state (`TransportConfig::speed`)
        let speed = Speed::from_slowdown(shared.seek_speed.load(Ordering::Relaxed));
        let n = rings.len();
        let (consumers, chans): (Vec<_>, Vec<_>) = rings.into_iter().unzip();
        Engine {
            scratch: chans.iter().map(|c| vec![0.0; MAX_BLOCK * c]).collect(),
            consumers,
            mix: vec![0.0; MAX_BLOCK],
            mix_r: vec![0.0; MAX_BLOCK],
            gains: vec![1.0; n],
            vols: vec![1.0; 3 * n],
            chans,
            drop_buf: vec![0.0; MAX_BLOCK * 2],
            peaks: vec![0.0; 2 * n + 2],
            clicks,
            click_rx,
            click_ret,
            layers: Box::default(),
            layer_rx: None,
            layer_ret: None,
            shared,
            clock,
            frame_pos: 0,
            loop_range: None,
            end_at: None,
            speed,
            state: State::Refilling,
            seen_gen: 0,
            seek_tag: 0,
            seen_gain_seq: 0,
            prefill,
        }
    }

    /// P4-7a: the queue of the layer banks (a new bank in, the old one back to the control side).
    pub(crate) fn set_layer_queue(&mut self, rx: Consumer<Box<LayerBank>>, ret: Producer<Box<LayerBank>>) {
        self.layer_rx = Some(rx);
        self.layer_ret = Some(ret);
    }

    /// The session frame (at the device rate) of the next sample the engine renders; at half
    /// speed the output frame divided by 2, rounded down.
    pub fn frame_pos(&self) -> u64 {
        self.frame_pos / self.speed.slowdown() as u64
    }

    /// Whether the engine plays from the rings (no seek or refill pending).
    pub fn is_ready(&self) -> bool {
        self.state == State::Ready
    }

    /// Run the seek and refill state machine (also called at the start of each render).
    pub fn poll(&mut self) {
        let sh = &self.shared;
        let g = sh.seek_req.load(Ordering::Acquire);
        if g != self.seen_gen {
            // P3-15f: target and tag of one request; while a request is being written, the next
            // block takes it
            if let Some((target, tag, speed, loop_range, end_at)) = sh.seek_request() {
                self.seen_gen = g;
                self.speed = speed;
                let slowdown = speed.slowdown() as u64;
                self.frame_pos = target.saturating_mul(slowdown);
                // #166: the region of the request in output frames at the render rate; a seek
                // without one ends a running loop. A target at or past the end starts wrapped.
                self.loop_range = loop_range.map(|(ls, le)| (ls.saturating_mul(slowdown), le.saturating_mul(slowdown)));
                // #201: the window end of the request in output frames at the render rate; a
                // seek without one clears the end (a plain play plays on)
                self.end_at = end_at.map(|e| e.saturating_mul(slowdown));
                if let Some((ls, le)) = self.loop_range
                    && self.frame_pos >= le
                {
                    self.frame_pos = ls + (self.frame_pos - ls) % (le - ls);
                }
                self.seek_tag = tag;
                // #430: a seek repositions every source; the lag of the old position is void
                for a in &sh.ring_lag {
                    a.store(0, Ordering::Release);
                }
                self.state = State::Flushing;
            }
        }
        if self.state == State::Flushing {
            let g = self.seen_gen;
            if sh.reader_paused.load(Ordering::Acquire) != g {
                return;
            }
            // the reader has stopped pushing old data: drop what is left in the rings
            for c in &mut self.consumers {
                let n = c.slots();
                if let Ok(chunk) = c.read_chunk(n) {
                    chunk.commit_all();
                }
            }
            sh.flushed_target.store(self.frame_pos, Ordering::Release);
            sh.flushed_speed.store(self.speed.slowdown(), Ordering::Release);
            // #166: the loop region the reader refills with (out frames at the render rate)
            if let Some((ls, le)) = self.loop_range {
                sh.flushed_loop_range.0.store(ls, Ordering::Release);
                sh.flushed_loop_range.1.store(le, Ordering::Release);
            }
            sh.flushed_loop.store(self.loop_range.is_some(), Ordering::Release);
            sh.cb_flushed.store(g, Ordering::Release);
            self.state = State::Refilling;
        }
        if self.state == State::Refilling
            && self
                .consumers
                .iter()
                .zip(&self.chans)
                .all(|(c, k)| c.slots() >= self.prefill * k)
        {
            self.state = State::Ready;
        }
        // a new click bank from the control side; the old one goes back to be dropped there
        if let Ok(bank) = self.click_rx.pop() {
            let old = std::mem::replace(&mut self.clicks, bank);
            let _ = self.click_ret.push(old);
        }
        // P4-7a: a new layer bank, the same way
        if let Some(Ok(bank)) = self.layer_rx.as_mut().map(rtrb::Consumer::pop) {
            let old = std::mem::replace(&mut self.layers, bank);
            if let Some(ret) = self.layer_ret.as_mut() {
                let _ = ret.push(old);
            }
        }
    }

    /// Render `out` (interleaved, `channels` per frame; the first two get the stereo mix,
    /// the rest silence). `host_play_ns` is when the first frame of `out` is heard
    /// ([`super::clock::host_now_ns`] time); the clock is published with it.
    pub fn render(&mut self, out: &mut [f32], channels: usize, host_play_ns: u64) {
        let channels = channels.max(1);
        let frames = out.len() / channels;
        self.poll();
        let sh = self.shared.clone();
        // #201: an audition that ends in the engine pauses itself once its window is out; the
        // playhead then freezes at the end (every frame of the window was delivered)
        if self.end_at.is_some_and(|end| self.frame_pos >= end) {
            sh.playing.store(false, Ordering::Release);
        }
        let playing = sh.playing.load(Ordering::Relaxed);
        let live = playing && self.state == State::Ready;
        let slowdown = self.speed.slowdown();
        self.clock.publish(
            self.frame_pos / slowdown as u64,
            host_play_ns,
            live,
            self.seek_tag,
            slowdown,
        );
        sh.stats.callbacks.fetch_add(1, Ordering::Relaxed);
        sh.stats.frames_min.fetch_min(frames as u64, Ordering::Relaxed);
        sh.stats.frames_max.fetch_max(frames as u64, Ordering::Relaxed);
        if !live {
            out.fill(0.0);
            return;
        }
        let gain_seq = sh.gain_seq.load(Ordering::Acquire);
        for (i, g) in self.gains.iter_mut().enumerate() {
            *g = f32::from_bits(sh.gains[i].load(Ordering::Relaxed));
        }
        for (v, a) in self.vols.iter_mut().zip(sh.vols.iter()) {
            *v = f32::from_bits(a.load(Ordering::Relaxed));
        }
        if gain_seq != self.seen_gain_seq {
            self.seen_gain_seq = gain_seq;
            sh.gain_applied_ns.store(host_play_ns, Ordering::Relaxed);
            sh.gain_applied_blocks.store(
                sh.stats
                    .callbacks
                    .load(Ordering::Relaxed)
                    .saturating_sub(sh.gain_set_cb.load(Ordering::Relaxed)),
                Ordering::Relaxed,
            );
            sh.gain_applied_seq.store(gain_seq, Ordering::Release);
        }
        let master = f32::from_bits(sh.master_gain.load(Ordering::Relaxed));
        let mvol = f32::from_bits(sh.master_volume.load(Ordering::Relaxed));
        let clicks_on = sh.clicks_on.load(Ordering::Relaxed);
        self.peaks.fill(0.0);
        let nt = self.gains.len();
        let mut done = 0;
        let mut peak = 0.0f32;
        while done < frames {
            // #166: a block never spans the wrap point of a loop: the clicks and the layer
            // voices of a block are placed at its first frame, so a block that runs over the
            // region end would misplace them on the far side of the wrap
            if let Some((ls, le)) = self.loop_range
                && self.frame_pos >= le
            {
                self.frame_pos = ls + (self.frame_pos - ls) % (le - ls);
            }
            let mut n = (frames - done).min(MAX_BLOCK);
            if let Some((_, le)) = self.loop_range {
                n = n.min((le - self.frame_pos) as usize).max(1);
            }
            if let Some(end) = self.end_at {
                if self.frame_pos >= end {
                    // #201: the window is fully delivered: the audition ends here; the rest of
                    // the buffer is silence (the next render publishes the paused end)
                    sh.playing.store(false, Ordering::Release);
                    break;
                }
                // the end is exclusive: a block never renders past it
                n = n.min((end - self.frame_pos) as usize).max(1);
            }
            let ml = &mut self.mix[..n];
            let mr = &mut self.mix_r[..n];
            ml.fill(0.0);
            mr.fill(0.0);
            let mut short = false;
            for (i, (c, x)) in self.consumers.iter_mut().zip(self.scratch.iter_mut()).enumerate() {
                let k = self.chans[i];
                // #430: after a dry ring the source sits behind the engine position: the
                // frames the reader pushes first are the ones the engine already rendered
                // as silence. Drop them here (the reader skips the rest in the source, see
                // `Reader::step`), so the track does not stay late after the recovery.
                let lag = sh.ring_lag[i].swap(0, Ordering::Acquire);
                if lag > 0 {
                    let avail = c.slots() / k;
                    let want = lag.min(avail as u64) as usize;
                    let mut dropped = 0usize;
                    while dropped < want {
                        let chunk = (want - dropped).min(MAX_BLOCK);
                        let buf = &mut self.drop_buf[..chunk * k];
                        let (popped, _) = c.pop_partial_slice(buf);
                        let frames = popped.len() / k;
                        dropped += frames;
                        if frames == 0 {
                            break;
                        }
                    }
                    let rest_lag = lag - dropped as u64;
                    if rest_lag > 0 {
                        sh.ring_lag[i].fetch_add(rest_lag, Ordering::Release);
                    }
                }
                let x = &mut x[..n * k];
                let (_, rest) = c.pop_partial_slice(x);
                if !rest.is_empty() {
                    short = true;
                    let missing = rest.len();
                    sh.stats
                        .underrun_frames
                        .fetch_add(missing.div_ceil(k) as u64, Ordering::Relaxed);
                    sh.ring_lag[i].fetch_add(missing.div_ceil(k) as u64, Ordering::Release);
                    x[n * k - missing..].fill(0.0);
                }
                let g = self.gains[i];
                let (vl, vr, vg) = (self.vols[3 * i], self.vols[3 * i + 1], self.vols[3 * i + 2]);
                let (gl, gr) = (g * vl, g * vr);
                if k == 2 {
                    // stereo: left to left, right to right, the pan is a balance
                    let (mut pl, mut pr) = (0.0f32, 0.0f32);
                    for ((l, r), f) in ml.iter_mut().zip(mr.iter_mut()).zip(x.as_chunks::<2>().0) {
                        *l += gl * f[0];
                        *r += gr * f[1];
                        pl = pl.max(f[0].abs());
                        pr = pr.max(f[1].abs());
                    }
                    self.peaks[2 * i] = self.peaks[2 * i].max(pl * vl.abs());
                    self.peaks[2 * i + 1] = self.peaks[2 * i + 1].max(pr * vr.abs());
                    continue;
                }
                // the meter of a mono track: post-fader, before the pan, mute and solo
                if vg != 0.0 {
                    let pk = abs_peak(x) * vg.abs();
                    self.peaks[2 * i] = self.peaks[2 * i].max(pk);
                    self.peaks[2 * i + 1] = self.peaks[2 * i + 1].max(pk);
                }
                if gl == gr {
                    if gl != 0.0 {
                        for ((l, r), v) in ml.iter_mut().zip(mr.iter_mut()).zip(x.iter()) {
                            let y = gl * v;
                            *l += y;
                            *r += y;
                        }
                    }
                } else {
                    for ((l, r), v) in ml.iter_mut().zip(mr.iter_mut()).zip(x.iter()) {
                        *l += gl * v;
                        *r += gr * v;
                    }
                }
            }
            if short {
                sh.stats.underruns.fetch_add(1, Ordering::Relaxed);
            }
            // P4-7a: the layer voices, through the track's mute, solo, volume and pan
            for (i, lv) in self.layers.iter().enumerate() {
                let Some(lv) = lv else { continue };
                let g = self.gains.get(i).copied().unwrap_or(0.0);
                if g == 0.0 {
                    continue;
                }
                let (gl, gr) = (g * self.vols[3 * i], g * self.vols[3 * i + 1]);
                let (n_started, last) = lv.add_block(ml, mr, self.frame_pos, gl, gr, self.speed);
                if n_started > 0 {
                    sh.stats.layer_voices.fetch_add(n_started, Ordering::Relaxed);
                }
                if let Some(f) = last {
                    sh.stats.layer_last.store(f as u64, Ordering::Relaxed);
                }
            }
            if clicks_on {
                for (tc, g) in self.clicks.iter().zip(&self.gains) {
                    if *g != 0.0 {
                        tc.add_block_at(ml, self.frame_pos, *g, self.speed);
                        tc.add_block_at(mr, self.frame_pos, *g, self.speed);
                    }
                }
            }
            let (mut pl, mut pr) = (self.peaks[2 * nt], self.peaks[2 * nt + 1]);
            let o = &mut out[done * channels..(done + n) * channels];
            for ((f, l), r) in o.chunks_exact_mut(channels).zip(ml.iter()).zip(mr.iter()) {
                peak = peak.max(l.abs()).max(r.abs());
                let (l, r) = (l * mvol, r * mvol);
                pl = pl.max(l.abs());
                pr = pr.max(r.abs());
                if channels > 1 {
                    f[0] = l * master;
                    f[1] = r * master;
                    f[2..].fill(0.0);
                } else {
                    f[0] = ((l + r) * 0.5) * master;
                }
            }
            self.peaks[2 * nt] = pl;
            self.peaks[2 * nt + 1] = pr;
            self.frame_pos += n as u64;
            done += n;
        }
        // an early break (the window end of #201) leaves the rest of the buffer unrendered
        out[done * channels..].fill(0.0);
        for (a, v) in sh.meters.iter().zip(&self.peaks) {
            store_peak(a, *v);
        }
        sh.stats.peak_bits.fetch_max(peak.to_bits(), Ordering::Relaxed);
        sh.stats.rendered_frames.fetch_add(frames as u64, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_to_gain_maps_0_db_to_exactly_one_and_the_ends() {
        assert_eq!(db_to_gain(0.0).to_bits(), 1.0f32.to_bits());
        assert!((db_to_gain(-6.0) - 0.501_187).abs() < 1e-6);
        assert!((db_to_gain(6.0) - 1.995_262).abs() < 1e-6);
        assert!((db_to_gain(12.0) - 3.981_072).abs() < 1e-6);
        assert_eq!(db_to_gain(20.0), db_to_gain(MAX_DB), "clamped at +12 dB");
        assert_eq!(db_to_gain(MIN_DB), 0.0);
        assert_eq!(db_to_gain(f32::NEG_INFINITY), 0.0);
        assert_eq!(db_to_gain(f32::NAN), 0.0);
        assert!(db_to_gain(-119.0) > 0.0);
    }

    #[test]
    fn pan_law_is_constant_power_with_minus_3_db_at_the_centre() {
        assert_eq!(pan_gains(0.0), (1.0, 1.0), "centre is exactly unity");
        assert_eq!(pan_gains(-0.0), (1.0, 1.0));
        assert_eq!(pan_gains(f32::NAN), (1.0, 1.0));
        let (l, r) = pan_gains(-1.0);
        assert!((l - std::f32::consts::SQRT_2).abs() < 1e-6 && r == 0.0, "{l} {r}");
        let (l, r) = pan_gains(1.0);
        assert!(l == 0.0 && (r - std::f32::consts::SQRT_2).abs() < 1e-6, "{l} {r}");
        assert_eq!(pan_gains(5.0), pan_gains(1.0), "clamped");
        for k in -10..=10 {
            let p = k as f32 / 10.0;
            let (l, r) = pan_gains(p);
            // constant power: l^2 + r^2 = 2 (the centre's power)
            assert!(((l * l + r * r) - 2.0).abs() < 1e-5, "pan {p}: {l} {r}");
            // the centre is 3 dB below a hard pan
            let (hl, _) = pan_gains(-1.0);
            assert!((20.0 * (1.0 / hl).log10() + 3.0103).abs() < 1e-3);
            // mirror
            let (ml, mr) = pan_gains(-p);
            assert!((ml - r).abs() < 1e-6 && (mr - l).abs() < 1e-6, "pan {p}");
        }
        assert!(pan_gains(-0.5).0 > pan_gains(-0.5).1);
    }

    #[test]
    fn channel_gains_combine_volume_and_pan() {
        assert_eq!(channel_gains(0.0, 0.0), (1.0, 1.0));
        let (l, r) = channel_gains(-6.0, 0.0);
        assert_eq!(l, r);
        assert!((l - 0.501_187).abs() < 1e-6);
        let (l, r) = channel_gains(MIN_DB, -1.0);
        assert_eq!((l, r), (0.0, 0.0));
        let (l, r) = channel_gains(6.0, 1.0);
        assert!(l.abs() < 1e-6 && (r - 1.995_262 * std::f32::consts::SQRT_2).abs() < 1e-5);
    }

    #[test]
    fn abs_peak_and_store_peak() {
        assert_eq!(abs_peak(&[]), 0.0);
        assert_eq!(abs_peak(&[0.25, -0.75, 0.5, f32::NAN]), 0.75);
        let a = std::sync::atomic::AtomicU32::new(0);
        store_peak(&a, 0.5);
        store_peak(&a, 0.25);
        assert_eq!(f32::from_bits(a.load(Ordering::Relaxed)), 0.5);
        store_peak(&a, 1.5);
        assert_eq!(f32::from_bits(a.load(Ordering::Relaxed)), 1.5);
        store_peak(&a, 0.0);
        assert_eq!(f32::from_bits(a.load(Ordering::Relaxed)), 1.5);
    }

    #[test]
    fn audible_gains_follow_app_js() {
        assert_eq!(audible_gains(3, &[], &[]), vec![1.0, 1.0, 1.0]);
        assert_eq!(audible_gains(3, &[false, true, false], &[]), vec![1.0, 0.0, 1.0]);
        assert_eq!(
            audible_gains(3, &[true, false, false], &[true, false, true]),
            vec![1.0, 0.0, 1.0],
            "solo wins over mute"
        );
        assert_eq!(
            audible_gains(2, &[false, false, true], &[false, false, true]),
            vec![1.0, 1.0],
            "entries past n are ignored"
        );
    }
}
