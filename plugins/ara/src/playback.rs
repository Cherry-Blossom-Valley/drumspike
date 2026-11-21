//! The page asks the host to play, stop or locate (M1b7, #554). The design is in
//! `docs/design/daw-plugin/m1b7-page-moves-host.md`.
//!
//! The way is: the page (`B.hostTransport.play/stop/locate`) -> the shell script -> the editor
//! route `plugin_host_play|stop|locate` (a request thread) -> [`plan`] (what to call, no host
//! call yet) -> the host's main thread -> the ARA playback controller of the host
//! ([`Host`]). The position of the host comes back through the transport follow of M1b5.
//!
//! The host calls are made on the main thread, the thread of the document controller. The
//! ARA header says of the playback controller only that its functions "may be called
//! concurrently, but not from render-threads"; the ARA test host of the SDK examples asserts
//! `wasCreatedOnCurrentThread ()` in every call of its host interfaces, and the example plug-in
//! sends its own playback requests through the main queue. So the plugin takes the stricter rule.

#![allow(clippy::missing_safety_doc)]

use crate::ara::*;
use crate::service::RegionRec;
use crate::transport::{song_time, LocateRule};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

/// The kinds of request of the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Play,
    Stop,
    Locate,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Play => "play",
            Kind::Stop => "stop",
            Kind::Locate => "locate",
        }
    }

    pub fn index(self) -> usize {
        self as usize
    }
}

/// Which calls of the playback controller the host gives.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Offer {
    /// `requestStartPlayback`.
    pub start: bool,
    /// `requestStopPlayback`.
    pub stop: bool,
    /// `requestSetPlaybackPosition`.
    pub position: bool,
}

impl Offer {
    pub fn has(self, kind: Kind) -> bool {
        match kind {
            Kind::Play => self.start,
            Kind::Stop => self.stop,
            Kind::Locate => self.position,
        }
    }

    pub fn any(self) -> bool {
        self.start || self.stop || self.position
    }
}

/// One call to the host.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Call {
    /// `requestSetPlaybackPosition`, seconds of the song.
    SetPosition(f64),
    Start,
    Stop,
}

/// The playback controller of one document controller, as the host gave it. The pointers stay
/// valid until the host destroys the document controller (the ARA rule); the plugin takes the
/// struct away from its document in `destroyDocumentController`, on the same thread that makes
/// the calls, under the same lock.
pub struct Host {
    host_ref: ARAPlaybackControllerHostRef,
    iface: *const ARAPlaybackControllerInterface,
    pub offer: Offer,
}

unsafe impl Send for Host {}
unsafe impl Sync for Host {}

/// What `probe` found, for the row of the report.
pub fn detail(declared: usize, needed: usize, iface_null: bool, iface_size: usize, offer: Offer) -> Value {
    json!({"host_struct_size": declared, "host_struct_size_needed": needed, "interface_given": !iface_null,
        "interface_struct_size": iface_size, "request_start": offer.start, "request_stop": offer.stop,
        "request_set_position": offer.position})
}

/// Read the playback controller of the host's instance struct. The interface is optional (the
/// header: "plug-in must check playbackControllerInterface is not NULL before calling"): the
/// struct the host gave must be long enough to hold it, the pointer must not be null, and each
/// function must lie inside the `structSize` of the interface and not be null.
pub unsafe fn probe(host: *const ARADocumentControllerHostInstance) -> (Option<Host>, Value) {
    if host.is_null() {
        return (None, detail(0, 0, true, 0, Offer::default()));
    }
    let declared = std::ptr::addr_of!((*host).structSize).read_unaligned();
    let needed = std::mem::offset_of!(ARADocumentControllerHostInstance, playbackControllerInterface)
        + std::mem::size_of::<usize>();
    if declared < needed {
        return (None, detail(declared, needed, true, 0, Offer::default()));
    }
    let iface = std::ptr::addr_of!((*host).playbackControllerInterface).read_unaligned();
    let host_ref = std::ptr::addr_of!((*host).playbackControllerHostRef).read_unaligned();
    if iface.is_null() {
        return (None, detail(declared, needed, true, 0, Offer::default()));
    }
    let size = std::ptr::addr_of!((*iface).structSize).read_unaligned();
    let slot = |off: usize, present: bool| size >= off + std::mem::size_of::<usize>() && present;
    let offer = Offer {
        start: slot(
            std::mem::offset_of!(ARAPlaybackControllerInterface, requestStartPlayback),
            std::ptr::addr_of!((*iface).requestStartPlayback)
                .read_unaligned()
                .is_some(),
        ),
        stop: slot(
            std::mem::offset_of!(ARAPlaybackControllerInterface, requestStopPlayback),
            std::ptr::addr_of!((*iface).requestStopPlayback)
                .read_unaligned()
                .is_some(),
        ),
        position: slot(
            std::mem::offset_of!(ARAPlaybackControllerInterface, requestSetPlaybackPosition),
            std::ptr::addr_of!((*iface).requestSetPlaybackPosition)
                .read_unaligned()
                .is_some(),
        ),
    };
    let d = detail(declared, needed, false, size, offer);
    if !offer.any() {
        return (None, d);
    }
    (Some(Host { host_ref, iface, offer }), d)
}

impl Host {
    /// Make the calls, in order. Main thread only (the caller checks). A call the host does not
    /// offer is skipped; the answer is the number of calls made.
    pub unsafe fn run(&self, calls: &[Call]) -> usize {
        let mut made = 0;
        for c in calls {
            match *c {
                Call::Start if self.offer.start => {
                    if let Some(f) = std::ptr::addr_of!((*self.iface).requestStartPlayback).read_unaligned() {
                        f(self.host_ref);
                        made += 1;
                    }
                }
                Call::Stop if self.offer.stop => {
                    if let Some(f) = std::ptr::addr_of!((*self.iface).requestStopPlayback).read_unaligned() {
                        f(self.host_ref);
                        made += 1;
                    }
                }
                Call::SetPosition(s) if self.offer.position => {
                    if let Some(f) = std::ptr::addr_of!((*self.iface).requestSetPlaybackPosition).read_unaligned() {
                        f(self.host_ref, s);
                        made += 1;
                    }
                }
                _ => {}
            }
        }
        made
    }
}

// ---------------------------------------------------------------- the queue of host requests

/// What became of a request that went to the main thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The calls were made (the number).
    Made(usize),
    /// A newer locate came before this one ran, or a later request already ran: it is not an error.
    Superseded,
    /// The editor closed before the request ran.
    Closed,
}

/// The queue is full.
#[derive(Debug)]
pub struct Full;

/// One request waiting for the main thread.
pub struct Job {
    /// The order of arrival at the editor (the scheme handler runs on the main thread, in order).
    pub seq: u64,
    pub kind: Kind,
    pub calls: Vec<Call>,
    pub reply: Sender<Outcome>,
    /// Set by the request thread that stopped waiting: the job then does nothing.
    pub late: Arc<AtomicBool>,
}

/// At most this many host requests wait for the main thread. The rest are answered at once.
pub const MAX_PENDING: usize = 8;

#[derive(Default)]
struct QueueState {
    jobs: VecDeque<Job>,
    scheduled: bool,
    last_run: Option<u64>,
}

/// One queue for the requests of the page to the host, per editor. A request keeps its place by
/// its sequence number; for locate the newest wins; the main thread runs the whole queue in one job.
#[derive(Default)]
pub struct Queue {
    state: Mutex<QueueState>,
}

impl Queue {
    /// Add a job in the order of its sequence number. `Err`: the queue is full, the job is not
    /// added. `Ok(true)`: the caller must start a drain job on the main thread.
    pub fn push(&self, job: Job) -> Result<bool, Full> {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if st.jobs.len() >= MAX_PENDING {
            return Err(Full);
        }
        let at = st.jobs.iter().position(|j| j.seq > job.seq).unwrap_or(st.jobs.len());
        st.jobs.insert(at, job);
        let start = !st.scheduled;
        st.scheduled = true;
        Ok(start)
    }

    pub fn pending(&self) -> usize {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).jobs.len()
    }

    /// Which jobs of this order are dropped: a locate that has a later locate in the list.
    pub fn superseded(kinds: &[Kind]) -> Vec<bool> {
        if crate::test_switch("DS_ARA_PLAYBACK_FAULT").as_deref() == Ok("no-supersede") {
            return vec![false; kinds.len()]; // the fault run: every locate runs
        }
        (0..kinds.len())
            .map(|i| kinds[i] == Kind::Locate && kinds[i + 1..].contains(&Kind::Locate))
            .collect()
    }

    /// Run the queue (on the main thread): in order, newest locate wins; `closed` says the editor
    /// is closed, then no call is made. `run` makes the calls and answers the number made.
    pub fn drain(&self, closed: &dyn Fn() -> bool, run: &mut dyn FnMut(&[Call]) -> usize) {
        let (jobs, mut last) = {
            let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
            st.scheduled = false;
            (std::mem::take(&mut st.jobs), st.last_run)
        };
        let kinds: Vec<Kind> = jobs.iter().map(|j| j.kind).collect();
        let drop_it = Queue::superseded(&kinds);
        for (job, dropped) in jobs.into_iter().zip(drop_it) {
            if job.late.load(Ordering::SeqCst) {
                continue;
            }
            let out = if closed() {
                Outcome::Closed
            } else if dropped || last.is_some_and(|l| job.seq < l) {
                // a request older than one that already ran must not run after it
                Outcome::Superseded
            } else {
                last = Some(job.seq);
                Outcome::Made(run(&job.calls))
            };
            let _ = job.reply.send(out);
        }
        self.state.lock().unwrap_or_else(|e| e.into_inner()).last_run = last;
    }
}

/// What the plugin decided for one request of the page.
#[derive(Clone, Debug, PartialEq)]
pub enum Plan {
    /// Call the host with these calls (possibly none: a fault run).
    Send {
        calls: Vec<Call>,
        /// The song position the locate goes to, when the request has one.
        song_s: Option<f64>,
        rule: Option<LocateRule>,
    },
    /// Send nothing; the page shows this text.
    Refuse(String),
}

/// The fault switch `DS_ARA_PLAYBACK_FAULT` (`switches.json`), read where the plan is made.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Fault {
    /// The locate goes to the host with the page time as song position (no regions).
    pub identity: bool,
    /// The plugin makes no call to the host and says it did.
    pub skip: bool,
    /// The refusal text does not name the host.
    pub anonymous: bool,
}

/// The fault values that only the shell script obeys (they go to it in the answer of
/// `plugin_host_offer`): `no-drag-guard`, `keep-error`.
pub fn shell_fault() -> String {
    match crate::test_switch("DS_ARA_PLAYBACK_FAULT").as_deref() {
        Ok(v @ ("no-drag-guard" | "keep-error")) => v.to_string(),
        _ => String::new(),
    }
}

impl Fault {
    pub fn read() -> Fault {
        let v = crate::test_switch("DS_ARA_PLAYBACK_FAULT").unwrap_or_default();
        Fault {
            identity: v == "identity",
            skip: v == "skip",
            anonymous: v == "anonymous",
        }
    }
}

/// The song position of the host as the transport slot has it (M1b5): seconds, and whether it plays.
#[derive(Clone, Copy, Debug)]
pub struct HostAt {
    pub song_s: f64,
    pub playing: bool,
}

/// A play of the page from a position that is more than this far from the host position first
/// asks the host to move.
pub const PLAY_MOVE_S: f64 = 0.005;

/// The text of a refusal. It names the host and says what is missing, in one short sentence.
pub fn unavailable_text(host_name: &str, offer: Offer, kind: Kind, fault: Fault) -> String {
    let who = if host_name.trim().is_empty() || fault.anonymous {
        "the host"
    } else {
        host_name.trim()
    };
    if !offer.any() {
        return format!("{who} gives plugins no control of play, stop or locate");
    }
    let what = match kind {
        Kind::Play => "start of playback",
        Kind::Stop => "stop of playback",
        Kind::Locate => "a change of the playback position",
    };
    format!("{who} does not offer {what} to plugins")
}

/// A page time above this many seconds (24 h) is refused; nothing in a drum session is longer.
pub const MAX_PAGE_S: f64 = 86_400.0;

/// Bound the page time of a locate before it is mapped: a time that is not a number, or is
/// above [`MAX_PAGE_S`], is refused (`None`); a time below 0 or above the length of the source
/// (`limit_s`, when known) is clamped to it.
pub fn bound_time(t: f64, limit_s: Option<f64>) -> Option<f64> {
    if !t.is_finite() || t > MAX_PAGE_S {
        return None;
    }
    let top = limit_s.filter(|l| l.is_finite() && *l > 0.0).unwrap_or(MAX_PAGE_S);
    Some(t.clamp(0.0, top))
}

/// The regions of the selected track (the sources whose track is `seqs`). With no selected
/// track, or a track with no region, all regions: the rule of `song_time` then picks.
pub fn regions_for(regions: &[(usize, RegionRec)], seqs: &[usize]) -> Vec<(usize, RegionRec)> {
    let own: Vec<(usize, RegionRec)> = regions.iter().filter(|(i, _)| seqs.contains(i)).copied().collect();
    if own.is_empty() {
        regions.to_vec()
    } else {
        own
    }
}

/// The text when the time of a locate maps to no region.
pub const NO_REGION_TEXT: &str = "no playback region maps this time yet: the host cannot be moved";

/// Decide what one request of the page does. Pure: no host call, no thread.
///
/// - `play`: needs `requestStartPlayback`. When the page plays from a time (`page_s`) that maps to
///   a song position more than [`PLAY_MOVE_S`] from the host position, and the host stands still,
///   the host is moved first (the page's click in the ruler of a stopped host is a locate, the
///   Play that follows starts there).
/// - `stop`: needs `requestStopPlayback`.
/// - `locate`: needs `requestSetPlaybackPosition` and a region: [`song_time`] of the regions of the
///   last round. No region, nothing is sent.
pub fn plan(
    kind: Kind,
    page_s: Option<f64>,
    offer: Offer,
    host_name: &str,
    regions: &[(usize, RegionRec)],
    host_at: Option<HostAt>,
    fault: Fault,
) -> Plan {
    if !offer.has(kind) {
        return Plan::Refuse(unavailable_text(host_name, offer, kind, fault));
    }
    if fault.skip {
        return Plan::Send {
            calls: Vec::new(),
            song_s: None,
            rule: None,
        };
    }
    let map = |t: f64| -> Option<(f64, Option<LocateRule>)> {
        if fault.identity {
            return Some((t.max(0.0), None));
        }
        song_time(t, host_at.map(|h| h.song_s), regions).map(|(s, r)| (s, Some(r)))
    };
    match kind {
        Kind::Stop => Plan::Send {
            calls: vec![Call::Stop],
            song_s: None,
            rule: None,
        },
        Kind::Locate => match page_s.and_then(map) {
            Some((s, rule)) => Plan::Send {
                calls: vec![Call::SetPosition(s)],
                song_s: Some(s),
                rule,
            },
            None => Plan::Refuse(NO_REGION_TEXT.to_string()),
        },
        Kind::Play => {
            let target = page_s.and_then(map);
            let move_first = offer.position
                && target
                    .is_some_and(|(s, _)| host_at.is_some_and(|h| !h.playing && (s - h.song_s).abs() > PLAY_MOVE_S));
            match (move_first, target) {
                (true, Some((s, rule))) => Plan::Send {
                    calls: vec![Call::SetPosition(s), Call::Start],
                    song_s: Some(s),
                    rule,
                },
                _ => Plan::Send {
                    calls: vec![Call::Start],
                    song_s: None,
                    rule: None,
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reg(a: f64, d: f64, p: f64) -> RegionRec {
        RegionRec {
            start_mod: a,
            dur_mod: d,
            start_play: p,
        }
    }

    const FULL: Offer = Offer {
        start: true,
        stop: true,
        position: true,
    };

    #[test]
    fn a_locate_goes_to_the_host_as_a_song_position_through_the_regions() {
        let r = [(0, reg(0.4, 2.4, 2.0))];
        let p = plan(Kind::Locate, Some(1.0), FULL, "Host", &r, None, Fault::default());
        assert_eq!(
            p,
            Plan::Send {
                calls: vec![Call::SetPosition(2.6)],
                song_s: Some(2.6),
                rule: Some(LocateRule::Time)
            }
        );
    }

    #[test]
    fn a_locate_with_no_region_sends_nothing_and_says_why() {
        let p = plan(Kind::Locate, Some(1.0), FULL, "Host", &[], None, Fault::default());
        assert_eq!(p, Plan::Refuse(NO_REGION_TEXT.to_string()));
        // a locate with no time at all is the same
        assert!(matches!(
            plan(
                Kind::Locate,
                None,
                FULL,
                "Host",
                &[(0, reg(0.0, 1.0, 0.0))],
                None,
                Fault::default()
            ),
            Plan::Refuse(_)
        ));
    }

    #[test]
    fn a_host_without_a_call_gets_no_call_and_the_text_names_the_host() {
        let none = Offer::default();
        for k in [Kind::Play, Kind::Stop, Kind::Locate] {
            let p = plan(
                k,
                Some(1.0),
                none,
                "LUNA",
                &[(0, reg(0.0, 2.0, 0.0))],
                None,
                Fault::default(),
            );
            let Plan::Refuse(t) = p else {
                panic!("{k:?} must refuse")
            };
            assert!(t.contains("LUNA"), "{t}");
        }
        // a host that gives start and stop but no position: locate is refused, play is not
        let part = Offer {
            start: true,
            stop: true,
            position: false,
        };
        assert!(matches!(
            plan(Kind::Locate, Some(1.0), part, "LUNA", &[(0, reg(0.0, 2.0, 0.0))], None, Fault::default()),
            Plan::Refuse(t) if t.contains("LUNA") && t.contains("position")
        ));
        assert!(matches!(
            plan(
                Kind::Play,
                Some(1.0),
                part,
                "LUNA",
                &[(0, reg(0.0, 2.0, 0.0))],
                None,
                Fault::default()
            ),
            Plan::Send { .. }
        ));
        // the host has no name
        assert!(unavailable_text("", none, Kind::Play, Fault::default()).starts_with("the host"));
    }

    #[test]
    fn a_play_moves_a_stopped_host_first_when_the_page_stands_elsewhere() {
        let r = [(0, reg(0.0, 4.0, 1.0))];
        let at = |s: f64, playing: bool| Some(HostAt { song_s: s, playing });
        // the page stands at 2.0 (song 3.0), the host at song 1.0: move, then start
        let p = plan(Kind::Play, Some(2.0), FULL, "H", &r, at(1.0, false), Fault::default());
        assert!(
            matches!(&p, Plan::Send { calls, .. } if calls == &vec![Call::SetPosition(3.0), Call::Start]),
            "{p:?}"
        );
        // the page stands where the host stands: only start
        let p = plan(Kind::Play, Some(2.0), FULL, "H", &r, at(3.001, false), Fault::default());
        assert!(
            matches!(&p, Plan::Send { calls, .. } if calls == &vec![Call::Start]),
            "{p:?}"
        );
        // a playing host is not moved by a play
        let p = plan(Kind::Play, Some(2.0), FULL, "H", &r, at(1.0, true), Fault::default());
        assert!(
            matches!(&p, Plan::Send { calls, .. } if calls == &vec![Call::Start]),
            "{p:?}"
        );
        // no region: only start
        let p = plan(Kind::Play, Some(2.0), FULL, "H", &[], at(1.0, false), Fault::default());
        assert!(
            matches!(&p, Plan::Send { calls, .. } if calls == &vec![Call::Start]),
            "{p:?}"
        );
    }

    /// The fault switch `DS_ARA_PLAYBACK_FAULT=identity` is the plan without the regions: the
    /// stage `playback-map` runs this test with the switch set, and it fails by 1.6 s.
    #[test]
    fn the_locate_of_the_plan_uses_the_regions_not_the_page_time_itself() {
        let r = [(0, reg(0.4, 2.4, 2.0))];
        let p = plan(Kind::Locate, Some(1.0), FULL, "H", &r, None, Fault::read());
        let Plan::Send { calls, .. } = p else {
            panic!("a locate with a region is sent")
        };
        let Call::SetPosition(s) = calls[0] else {
            panic!("a locate is a position")
        };
        assert!(
            (s - 2.6).abs() < 1e-9,
            "the locate of page time 1.0 s went to song {s} s (want 2.6 s: off by {:.1} s)",
            (s - 2.6).abs()
        );
    }
    fn job(seq: u64, kind: Kind, calls: Vec<Call>) -> (Job, std::sync::mpsc::Receiver<Outcome>, Arc<AtomicBool>) {
        let (tx, rx) = std::sync::mpsc::channel();
        let late = Arc::new(AtomicBool::new(false));
        (
            Job {
                seq,
                kind,
                calls,
                reply: tx,
                late: late.clone(),
            },
            rx,
            late,
        )
    }

    #[test]
    fn requests_run_in_the_order_of_arrival_and_the_newest_locate_wins() {
        let q = Queue::default();
        // arrival order: locate A, locate B, stop, locate C, play; pushed out of order
        let mut rx = Vec::new();
        for (seq, kind, call) in [
            (4, Kind::Locate, Call::SetPosition(3.0)),
            (1, Kind::Locate, Call::SetPosition(1.0)),
            (3, Kind::Stop, Call::Stop),
            (2, Kind::Locate, Call::SetPosition(2.0)),
            (5, Kind::Play, Call::Start),
        ] {
            let (j, r, _) = job(seq, kind, vec![call]);
            assert!(q.push(j).is_ok());
            rx.push((seq, r));
        }
        let mut ran: Vec<Call> = Vec::new();
        q.drain(&|| false, &mut |c| {
            ran.extend_from_slice(c);
            1
        });
        // A and B are dropped (a newer locate C is in the list); stop, C, play run in arrival order
        assert_eq!(
            ran,
            vec![Call::Stop, Call::SetPosition(3.0), Call::Start],
            "the newest locate wins: an older locate that did not run is dropped"
        );
        rx.sort_by_key(|(s, _)| *s);
        let got: Vec<Outcome> = rx.iter().map(|(_, r)| r.try_recv().unwrap()).collect();
        assert_eq!(
            got,
            vec![
                Outcome::Superseded,
                Outcome::Superseded,
                Outcome::Made(1),
                Outcome::Made(1),
                Outcome::Made(1)
            ]
        );
        // a job older than one that ran is not run after it
        let (j, r, _) = job(2, Kind::Stop, vec![Call::Stop]);
        q.push(j).unwrap();
        q.drain(&|| false, &mut |_| panic!("an old request must not run"));
        assert_eq!(r.try_recv().unwrap(), Outcome::Superseded);
    }

    #[test]
    fn the_queue_holds_a_few_requests_and_the_drain_job_starts_once() {
        let q = Queue::default();
        let mut starts = 0;
        for i in 0..MAX_PENDING as u64 {
            let (j, _r, _) = job(i, Kind::Stop, vec![Call::Stop]);
            if q.push(j).unwrap() {
                starts += 1;
            }
        }
        assert_eq!(starts, 1, "one drain job for the whole queue");
        let (j, _r, _) = job(99, Kind::Stop, vec![Call::Stop]);
        assert!(q.push(j).is_err(), "the next one is refused at once");
        assert_eq!(q.pending(), MAX_PENDING);
    }

    #[test]
    fn a_request_queued_before_the_editor_closed_makes_no_call() {
        let q = Queue::default();
        let (j, r, _) = job(1, Kind::Play, vec![Call::Start]);
        q.push(j).unwrap();
        q.drain(&|| true, &mut |_| panic!("the editor is closed: no call"));
        assert_eq!(r.try_recv().unwrap(), Outcome::Closed);
        // a request whose thread stopped waiting does nothing either
        let (j, r, late) = job(2, Kind::Play, vec![Call::Start]);
        late.store(true, Ordering::SeqCst);
        q.push(j).unwrap();
        q.drain(&|| false, &mut |_| panic!("a late request must not run"));
        assert!(r.try_recv().is_err());
    }

    #[test]
    fn a_time_is_bounded_before_it_reaches_the_host() {
        assert_eq!(bound_time(1.5, Some(3.0)), Some(1.5));
        assert_eq!(bound_time(9.0, Some(3.0)), Some(3.0));
        assert_eq!(bound_time(-1.0, Some(3.0)), Some(0.0));
        assert_eq!(bound_time(1e12, Some(3.0)), None);
        assert_eq!(bound_time(f64::INFINITY, None), None);
        assert_eq!(bound_time(f64::NAN, None), None);
        assert_eq!(bound_time(5000.0, None), Some(5000.0));
    }

    #[test]
    fn the_regions_of_the_selected_track_are_used_else_all() {
        // two stems of one length at different places of the song: source 0 at 0 s, source 1 at 10 s
        let r = [(0, reg(0.0, 3.0, 0.0)), (1, reg(0.0, 3.0, 10.0))];
        // no track selected: today's rule, the lowest source index
        let all = regions_for(&r, &[]);
        assert_eq!(song_time(1.0, None, &all).unwrap().0, 1.0);
        // the track of source 1 is selected: its region decides
        let own = regions_for(&r, &[1]);
        assert_eq!(song_time(1.0, None, &own).unwrap().0, 11.0);
        // a track with no source: all regions
        assert_eq!(regions_for(&r, &[7]).len(), 2);
    }
}
