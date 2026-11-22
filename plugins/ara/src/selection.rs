//! The selection of the DAW (M1b13, #562): what the host says it selected and which tracks it hides, as the page
//! needs it.
//!
//! The design is `docs/design/daw-plugin/m1b13-selection.md`. This file holds the pure rules, with no host or ARA type
//! in them, so a unit test reaches every case. The callbacks of the host (`notifySelection`,
//! `notifyHideRegionSequences`) only copy ids and numbers into a [`RawSel`]; the rules here turn them into the payload
//! of the page event `plugin-selection`.
//!
//! Rules, short form:
//!
//! - A region sequence of the host (a DAW track) or a playback region selects the tracks of the page whose source has a
//!   region on that sequence, or that region. The tracks come in the order of the DAW (`orderIndex` of the sequence),
//!   then in the order of the page. The first is selected in the page; the others are marked.
//! - A selection that names a DAW track or a region, and no track of the plugin has it, changes nothing in the page.
//! - A time range is in song time, and the page shows song time since M1b8 (#555): the range goes to the page as it is, for
//!   every row. An empty range, or no range, removes the range. A range outside every clip is drawn all the same.
//! - A track of the page is hidden when every DAW track that holds one of its regions is hidden.

use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

/// What the host selected, as ids of the plugin (never refs of the host).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RawSel {
    /// The ids of the selected region sequences.
    pub sequences: Vec<u64>,
    /// The ids of the selected playback regions.
    pub regions: Vec<u64>,
    /// The time range `(start, duration)` in song seconds, when the host gave one.
    pub range: Option<(f64, f64)>,
}

impl RawSel {
    /// The selection names a DAW track or a region (it is not only a range, or nothing).
    #[must_use]
    pub fn names_track(&self) -> bool {
        !self.sequences.is_empty() || !self.regions.is_empty()
    }
}

/// One track of the page as the rules need it: the key, the creation index of its source (the page order) and its
/// playback regions as `(region id, id of its region sequence, 0 when none)`.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackFacts {
    pub track: String,
    pub seq: usize,
    pub regions: Vec<(u64, u64)>,
}

/// The fault runs of the check: each breaks one rule. A delivery build reads none (see `crate::test_switch`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Fault {
    /// The track of the selection is never given to the page.
    pub no_track: bool,
    /// Only the first track is given: the others are not marked.
    pub no_others: bool,
    /// The hidden tracks are ignored.
    pub no_hidden: bool,
    /// A selection of a track with no source of the plugin clears the page's range.
    pub unknown_clears: bool,
    /// An empty selection is not passed on: the page keeps its range.
    pub keep_range: bool,
    /// The pull of a page that opened later answers nothing: it sees only the events that come after it.
    pub no_pull: bool,
    /// Every change is pushed to the page at once and every change writes a row (no gate).
    pub no_throttle: bool,
    /// The row of a selection has no names and no result.
    pub no_row: bool,
    /// A selection starts an analysis round (every source counts as changed).
    pub round: bool,
}

impl Fault {
    #[must_use]
    pub fn read() -> Fault {
        let mut f = Fault::default();
        match crate::test_switch("DS_ARA_SELECTION_FAULT").as_deref() {
            Ok("no-track") => f.no_track = true,
            Ok("no-others") => f.no_others = true,
            Ok("no-hidden") => f.no_hidden = true,
            Ok("unknown-clears") => f.unknown_clears = true,
            Ok("keep-range") => f.keep_range = true,
            Ok("no-pull") => f.no_pull = true,
            Ok("no-throttle") => f.no_throttle = true,
            Ok("no-row") => f.no_row = true,
            Ok("round") => f.round = true,
            _ => {}
        }
        f
    }
}

/// The indexes (into `tracks`) of the tracks that a selection names, in the order of the DAW, then of the page.
#[must_use]
pub fn tracks_named(raw: &RawSel, tracks: &[TrackFacts], order: &HashMap<u64, Option<i32>>) -> Vec<usize> {
    let seqs: HashSet<u64> = raw.sequences.iter().copied().collect();
    let regs: HashSet<u64> = raw.regions.iter().copied().collect();
    let mut hit: Vec<(i32, usize, usize)> = Vec::new();
    for (i, t) in tracks.iter().enumerate() {
        let mut best: Option<i32> = None;
        for (rid, sid) in &t.regions {
            if regs.contains(rid) || (*sid != 0 && seqs.contains(sid)) {
                // a sequence with no order index sorts after every sequence with one
                let o = order.get(sid).copied().flatten().unwrap_or(i32::MAX);
                best = Some(best.map_or(o, |b| b.min(o)));
            }
        }
        if let Some(o) = best {
            hit.push((o, t.seq, i));
        }
    }
    hit.sort_unstable();
    hit.into_iter().map(|(_, _, i)| i).collect()
}

/// The shortest time between two page messages of the selection. A drag in the ruler of the DAW sends many ranges; the page gets
/// at most one message in this time, always with the newest state, and the last state always comes.
pub const PUSH_GAP_MS: u64 = 50;

/// What the gate says about one message.
#[derive(Debug, Clone, PartialEq)]
pub enum Offer {
    /// Send it now.
    Now(Value),
    /// It waits (the newest one is kept). `schedule`: nobody is set to send it yet, the caller must.
    Later { schedule: bool },
}

/// The gate of the page messages: pure, the caller gives the time.
#[derive(Debug, Default)]
pub struct PushGate {
    /// The number of the newest message that was sent.
    sent_n: u64,
    last: Option<u64>,
    pending: Option<Value>,
    scheduled: bool,
}

impl PushGate {
    /// Two threads may offer in the wrong order: the message with the higher number `n` always wins, a lower one is dropped.
    pub fn offer(&mut self, event: Value, now_ms: u64, gap_ms: u64) -> Offer {
        let n = event["n"].as_u64().unwrap_or(0);
        if n != 0 && n <= self.sent_n {
            return Offer::Later { schedule: false };
        }
        if self.pending.as_ref().is_some_and(|p| p["n"].as_u64().unwrap_or(0) > n) {
            return Offer::Later { schedule: false };
        }
        if self.last.is_none_or(|l| now_ms.saturating_sub(l) >= gap_ms) {
            self.last = Some(now_ms);
            self.sent_n = n;
            self.pending = None;
            return Offer::Now(event);
        }
        self.pending = Some(event);
        let schedule = !self.scheduled;
        self.scheduled = true;
        Offer::Later { schedule }
    }

    /// The waiting message, when the sender that was scheduled runs.
    pub fn take(&mut self, now_ms: u64) -> Option<Value> {
        self.scheduled = false;
        let e = self
            .pending
            .take()
            .filter(|e| e["n"].as_u64().unwrap_or(0) > self.sent_n);
        if let Some(e) = &e {
            self.last = Some(now_ms);
            self.sent_n = e["n"].as_u64().unwrap_or(0);
        }
        e
    }
}

/// The tracks that the hidden region sequences hide: every sequence that holds a region of the track is hidden. A track
/// with a region on no sequence is never hidden.
#[must_use]
pub fn tracks_hidden(hidden: &[u64], tracks: &[TrackFacts]) -> Vec<usize> {
    if hidden.is_empty() {
        return Vec::new();
    }
    let set: HashSet<u64> = hidden.iter().copied().collect();
    tracks
        .iter()
        .enumerate()
        .filter(|(_, t)| !t.regions.is_empty() && t.regions.iter().all(|(_, sid)| *sid != 0 && set.contains(sid)))
        .map(|(i, _)| i)
        .collect()
}

/// What the plugin keeps of the view of the host between two calls.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ViewState {
    /// The last selection of the host.
    pub raw: RawSel,
    /// The last selection that named a track (the range of a later selection with no track maps through it).
    pub named: RawSel,
    /// The region sequences the host hides now.
    pub hidden: Vec<u64>,
    /// Whether the host has sent a selection at all.
    pub seen: bool,
}

/// The payload of the page event `plugin-selection`: the state of the selection of the host as the page applies it.
/// `tracks` are page keys in the order of the DAW; `none` is true when the selection names a DAW track or a region that
/// no track of the plugin has (the page changes nothing); `ranges` is the range of the host as one `{t0, t1}` in song time (M1b8), empty for none;
/// `hidden` is the list of hidden page keys.
#[must_use]
pub fn payload(
    state: &ViewState,
    tracks: &[TrackFacts],
    order: &HashMap<u64, Option<i32>>,
    fault: Fault,
) -> Value {
    let hidden: Vec<&str> = if fault.no_hidden {
        Vec::new()
    } else {
        tracks_hidden(&state.hidden, tracks)
            .into_iter()
            .map(|i| tracks[i].track.as_str())
            .collect()
    };
    if !state.seen {
        return json!({"seen": false, "tracks": [], "none": false, "ranges": [], "range_why": "none", "hidden": hidden});
    }
    let raw = &state.raw;
    let named = if raw.names_track() {
        tracks_named(raw, tracks, order)
    } else {
        Vec::new()
    };
    let none = raw.names_track() && named.is_empty();
    // the range is in song time and the page shows song time: it goes as it is
    let mut ranges: Vec<Value> = Vec::new();
    let mut why = "none";
    if let Some((start, dur)) = raw.range {
        if start.is_finite() && dur.is_finite() && dur > 0.0 {
            ranges = vec![json!({"t0": start, "t1": start + dur})];
            why = "ok";
        } else {
            why = "empty";
        }
    }
    let mut keys: Vec<&str> = named.iter().map(|i| tracks[*i].track.as_str()).collect();
    if fault.no_track {
        keys.clear();
    } else if fault.no_others {
        keys.truncate(1);
    }
    let none = if fault.unknown_clears { false } else { none };
    json!({"seen": true, "tracks": keys, "none": none, "ranges": ranges, "range_why": why,
        "hidden": hidden})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> Vec<TrackFacts> {
        // kick: region 10 on sequence 1; snare: region 11 on sequence 2; tom: regions 12 on sequence 3 and 13 on 4
        vec![
            TrackFacts {
                track: "kick".into(),
                seq: 0,
                regions: vec![(10, 1)],
            },
            TrackFacts {
                track: "snare".into(),
                seq: 1,
                regions: vec![(11, 2)],
            },
            TrackFacts {
                track: "tom".into(),
                seq: 2,
                regions: vec![(12, 3), (13, 4)],
            },
        ]
    }

    fn order() -> HashMap<u64, Option<i32>> {
        // the DAW shows the snare track first, the kick track last; sequence 4 has no order
        HashMap::from([(1, Some(5)), (2, Some(0)), (3, Some(2)), (4, None)])
    }

    fn state(raw: RawSel) -> ViewState {
        ViewState {
            raw: raw.clone(),
            named: raw,
            hidden: Vec::new(),
            seen: true,
        }
    }

    #[test]
    fn a_region_sequence_selects_the_track_that_has_a_region_on_it() {
        let raw = RawSel {
            sequences: vec![2],
            ..RawSel::default()
        };
        assert_eq!(tracks_named(&raw, &facts(), &order()), vec![1]);
        let raw = RawSel {
            regions: vec![13],
            ..RawSel::default()
        };
        assert_eq!(
            tracks_named(&raw, &facts(), &order()),
            vec![2],
            "a region selects its track"
        );
    }

    #[test]
    fn several_tracks_come_in_the_order_of_the_daw_then_of_the_page() {
        let raw = RawSel {
            sequences: vec![1, 3, 2],
            ..RawSel::default()
        };
        // snare (order 0), tom (order 2), kick (order 5): the list order of the host does not matter
        assert_eq!(tracks_named(&raw, &facts(), &order()), vec![1, 2, 0]);
        // no order index at all: the page order decides
        let none: HashMap<u64, Option<i32>> = HashMap::new();
        assert_eq!(tracks_named(&raw, &facts(), &none), vec![0, 1, 2]);
        // a track on two selected sequences is listed once
        let two = RawSel {
            sequences: vec![3, 4],
            ..RawSel::default()
        };
        assert_eq!(tracks_named(&two, &facts(), &order()), vec![2]);
    }

    #[test]
    fn a_selection_of_a_track_with_no_source_names_nothing_and_says_so() {
        let raw = RawSel {
            sequences: vec![99],
            range: Some((1.0, 1.0)),
            ..RawSel::default()
        };
        let p = payload(&state(raw), &facts(), &order(), Fault::default());
        assert_eq!(p["none"], json!(true), "the page changes nothing");
        assert_eq!(p["tracks"], json!([]));
        assert_eq!(
            p["ranges"],
            json!([]),
            "the range of such a selection is not used either"
        );
    }

    #[test]
    fn the_range_goes_to_the_page_as_it_is_in_song_time_for_any_track() {
        let sel = |range| {
            state(RawSel {
                sequences: vec![2],
                range: Some(range),
                ..RawSel::default()
            })
        };
        // the snare lies at song 3 to 5: a range inside it, a range that leaves it and one in no clip are all song time
        for (start, dur) in [(3.5, 1.0), (4.5, 1.0), (20.0, 1.0)] {
            let p = payload(&sel((start, dur)), &facts(), &order(), Fault::default());
            assert_eq!(p["ranges"], json!([{"t0": start, "t1": start + dur}]));
            assert_eq!(p["range_why"], json!("ok"));
            assert_eq!(p["tracks"], json!(["snare"]), "the track is selected all the same");
        }
    }

    #[test]
    fn an_empty_or_broken_range_is_no_range_and_an_empty_selection_removes_it() {
        for (s, d) in [(1.0, 0.0), (1.0, -2.0), (f64::NAN, 1.0), (1.0, f64::INFINITY)] {
            let st = state(RawSel {
                range: Some((s, d)),
                ..RawSel::default()
            });
            let p = payload(&st, &facts(), &order(), Fault::default());
            assert_eq!((p["ranges"].clone(), p["range_why"].clone()), (json!([]), json!("empty")), "{s} {d}");
        }
        let p = payload(&state(RawSel::default()), &facts(), &order(), Fault::default());
        assert_eq!((p["ranges"].clone(), p["none"].clone()), (json!([]), json!(false)));
    }

    #[test]
    fn a_range_alone_selects_no_track_and_needs_none() {
        let st = state(RawSel {
            range: Some((3.5, 1.0)),
            ..RawSel::default()
        });
        let p = payload(&st, &facts(), &order(), Fault::default());
        assert_eq!(p["tracks"], json!([]));
        assert_eq!(p["ranges"], json!([{"t0": 3.5, "t1": 4.5}]));
    }

    #[test]
    fn a_track_is_hidden_when_every_sequence_with_a_region_of_it_is_hidden() {
        assert_eq!(tracks_hidden(&[1, 2], &facts()), vec![0, 1]);
        assert_eq!(
            tracks_hidden(&[3], &facts()),
            Vec::<usize>::new(),
            "the tom has a region on sequence 4 too"
        );
        assert_eq!(tracks_hidden(&[3, 4], &facts()), vec![2]);
        assert_eq!(
            tracks_hidden(&[], &facts()),
            Vec::<usize>::new(),
            "an empty list shows every track"
        );
        // a region on no sequence is never hidden
        let loose = vec![TrackFacts {
            track: "x".into(),
            seq: 0,
            regions: vec![(1, 0)],
        }];
        assert_eq!(tracks_hidden(&[0], &loose), Vec::<usize>::new());
    }

    #[test]
    fn the_payload_before_any_selection_has_only_the_hidden_list() {
        let st = ViewState {
            hidden: vec![1],
            ..ViewState::default()
        };
        let p = payload(&st, &facts(), &order(), Fault::default());
        assert_eq!(p["seen"], json!(false));
        assert_eq!(p["hidden"], json!(["kick"]));
    }

    #[test]
    fn the_gate_sends_the_first_message_at_once_holds_a_burst_and_keeps_the_newest() {
        let m = |n: u64| json!({"n": n});
        let mut g = PushGate::default();
        assert_eq!(g.offer(m(1), 0, 50), Offer::Now(m(1)));
        assert_eq!(g.offer(m(2), 10, 50), Offer::Later { schedule: true });
        assert_eq!(
            g.offer(m(3), 20, 50),
            Offer::Later { schedule: false },
            "one sender is enough"
        );
        assert_eq!(
            g.take(50),
            Some(m(3)),
            "the newest state comes, the one before it is gone"
        );
        assert_eq!(g.take(60), None, "nothing waits any more");
        assert_eq!(
            g.offer(m(4), 70, 50),
            Offer::Later { schedule: true },
            "50 ms count from the last send"
        );
        assert_eq!(g.take(100), Some(m(4)));
        assert_eq!(g.offer(m(5), 400, 50), Offer::Now(m(5)));
        // a message that goes now takes the place of one that waits
        assert_eq!(g.offer(m(6), 410, 50), Offer::Later { schedule: true });
        assert_eq!(g.offer(m(7), 460, 50), Offer::Now(m(7)));
        assert_eq!(g.take(470), None);
    }

    #[test]
    fn the_gate_keeps_the_higher_number_when_two_threads_offer_in_the_wrong_order() {
        let m = |n: u64| json!({"n": n});
        // waiting: 6 arrives before 5
        let mut g = PushGate::default();
        assert_eq!(g.offer(m(1), 0, 50), Offer::Now(m(1)));
        assert_eq!(g.offer(m(6), 10, 50), Offer::Later { schedule: true });
        assert_eq!(g.offer(m(5), 11, 50), Offer::Later { schedule: false });
        assert_eq!(g.take(60), Some(m(6)), "the page ends on the newer state");
        // direct: 6 went out, 5 comes late and is dropped, also after the gap
        assert_eq!(g.offer(m(5), 500, 50), Offer::Later { schedule: false });
        assert_eq!(g.take(510), None);
        assert_eq!(g.offer(m(7), 520, 50), Offer::Now(m(7)));
        // the other order: 5 waits, 6 replaces it
        let mut g = PushGate::default();
        assert_eq!(g.offer(m(1), 0, 50), Offer::Now(m(1)));
        assert_eq!(g.offer(m(5), 10, 50), Offer::Later { schedule: true });
        assert_eq!(g.offer(m(6), 11, 50), Offer::Later { schedule: false });
        assert_eq!(g.take(60), Some(m(6)));
    }

    #[test]
    fn each_fault_breaks_its_rule() {
        let raw = RawSel {
            sequences: vec![1, 2],
            range: Some((3.5, 1.0)),
            ..RawSel::default()
        };
        let st = state(raw);
        let f = |g: Fault| payload(&st, &facts(), &order(), g);
        assert_eq!(f(Fault::default())["tracks"], json!(["snare", "kick"]));
        assert_eq!(
            f(Fault {
                no_track: true,
                ..Fault::default()
            })["tracks"],
            json!([])
        );
        assert_eq!(
            f(Fault {
                no_others: true,
                ..Fault::default()
            })["tracks"],
            json!(["snare"])
        );
    }
}
