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
//! - A time range is in song time. It is mapped to source time through the regions of the first selected track: each
//!   region keeps the part of the range that lies inside it, and the answer is the smallest range that holds those
//!   parts. A range that lies in no region is not drawn. An empty range, or no range, removes the range.
//! - A selection that has a range and no track maps the range through the first track of the last selection that named
//!   a track; with none yet, through the first track of the page.
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

/// One playback region for the mapping: the slice `[start_mod, start_mod + dur_mod)` of the source sits in the song from
/// `start_play` for `dur_play` seconds (the numbers of `deliver::ExportRegion`; they differ for a region that the host stretches).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Slice {
    pub start_mod: f64,
    pub dur_mod: f64,
    pub start_play: f64,
    pub dur_play: f64,
}

/// What the page is told about a time range.
#[derive(Debug, Clone, PartialEq)]
pub enum RangeOut {
    /// No range, or an empty one: the page removes its bands of the host.
    None,
    /// A range that lies in no region of the track: not drawn (the page removes its bands).
    Outside,
    /// The range in source time of the track: one `(t0, t1)` for each part, in order, parts that touch joined.
    Bands(Vec<(f64, f64)>),
}

/// The fault runs of the check: each breaks one rule. A delivery build reads none (see `crate::test_switch`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Fault {
    /// The track of the selection is never given to the page.
    pub no_track: bool,
    /// Only the first track is given: the others are not marked.
    pub no_others: bool,
    /// The range stays in song time (no mapping through the regions).
    pub song_range: bool,
    /// The hidden tracks are ignored.
    pub no_hidden: bool,
    /// A selection of a track with no source of the plugin clears the page's range.
    pub unknown_clears: bool,
    /// An empty selection is not passed on: the page keeps its range.
    pub keep_range: bool,
    /// A range outside every region is drawn at the nearest region.
    pub outside_draws: bool,
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
            Ok("song-range") => f.song_range = true,
            Ok("no-hidden") => f.no_hidden = true,
            Ok("unknown-clears") => f.unknown_clears = true,
            Ok("keep-range") => f.keep_range = true,
            Ok("outside-draws") => f.outside_draws = true,
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

/// Song seconds to source seconds through the regions of one track (the rules of the module header). `start` and
/// `duration` are the range of the host. This is the ONE place that maps the range of the host to the time of the page; ticket
/// #555 (the song-time timeline) replaces it. A region keeps the part of the range that lies in it, mapped linearly with the
/// length it has in the song (`dur_play`, so a stretched region is right); the parts come back as bands, one for each region, and
/// parts that touch are one band. An empty or invalid range, and a range in no region, are not bands.
#[must_use]
pub fn range_to_source(start: f64, duration: f64, regions: &[Slice]) -> RangeOut {
    if !start.is_finite() || !duration.is_finite() || duration <= 0.0 {
        return RangeOut::None;
    }
    let end = start + duration;
    let mut parts: Vec<(f64, f64)> = Vec::new();
    for r in regions.iter().filter(|r| {
        [r.start_mod, r.dur_mod, r.start_play, r.dur_play]
            .iter()
            .all(|v| v.is_finite())
            && r.dur_mod > 0.0
            && r.dur_play > 0.0
    }) {
        let s = start.max(r.start_play);
        let e = end.min(r.start_play + r.dur_play);
        if e > s {
            let k = r.dur_mod / r.dur_play;
            let (a, b) = (
                (r.start_mod + (s - r.start_play) * k).max(0.0),
                r.start_mod + (e - r.start_play) * k,
            );
            if a.is_finite() && b.is_finite() {
                parts.push((a, b));
            }
        }
    }
    parts.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut bands: Vec<(f64, f64)> = Vec::new();
    for p in parts {
        match bands.last_mut() {
            Some(last) if p.0 <= last.1 + 1e-9 => last.1 = last.1.max(p.1),
            _ => bands.push(p),
        }
    }
    if bands.is_empty() {
        RangeOut::Outside
    } else {
        RangeOut::Bands(bands)
    }
}

/// The most bands of the range of the host that the page is told about.
pub const MAX_BANDS: usize = 64;

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
/// no track of the plugin has (the page changes nothing); `ranges` is a list of `{t0, t1}` in source time of `tracks[0]`, one for each region part, empty for none;
/// `hidden` is the list of hidden page keys.
#[must_use]
pub fn payload(
    state: &ViewState,
    tracks: &[TrackFacts],
    order: &HashMap<u64, Option<i32>>,
    regions_of: &dyn Fn(usize) -> Vec<Slice>,
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
    // the track the range maps through: the selected one, else the one of the last selection that named a track
    let map_via: Option<usize> = if !named.is_empty() {
        Some(named[0])
    } else if !raw.names_track() {
        tracks_named(&state.named, tracks, order)
            .first()
            .copied()
            .or(if tracks.is_empty() { None } else { Some(0) })
    } else {
        None
    };
    let mut ranges: Vec<Value> = Vec::new();
    let mut why = "none";
    if let (Some((start, dur)), Some(i)) = (raw.range, map_via) {
        if fault.song_range && dur > 0.0 {
            ranges = vec![json!({"t0": start, "t1": start + dur})];
            why = "ok";
        } else {
            let regs = regions_of(tracks[i].seq);
            match range_to_source(start, dur, &regs) {
                RangeOut::Bands(list) => {
                    // a range over very many regions: the first MAX_BANDS are drawn, the rest is dropped and the reason says so
                    why = if list.len() > MAX_BANDS { "capped" } else { "ok" };
                    ranges = list
                        .iter()
                        .take(MAX_BANDS)
                        .map(|(a, b)| json!({"t0": a, "t1": b}))
                        .collect();
                }
                RangeOut::Outside => {
                    why = "outside";
                    if fault.outside_draws {
                        if let Some(r) = regs
                            .iter()
                            .min_by(|x, y| (x.start_play - start).abs().total_cmp(&(y.start_play - start).abs()))
                        {
                            ranges = vec![json!({"t0": r.start_mod, "t1": r.start_mod + r.dur_mod.min(dur)})];
                        }
                    }
                }
                RangeOut::None => why = "empty",
            }
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
        "track_range": map_via.map(|i| tracks[i].track.as_str()), "hidden": hidden})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(start_mod: f64, dur_mod: f64, start_play: f64) -> Slice {
        Slice {
            start_mod,
            dur_mod,
            start_play,
            dur_play: dur_mod,
        }
    }

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

    fn regions_of(seq: usize) -> Vec<Slice> {
        match seq {
            0 => vec![rec(0.0, 3.0, 0.0)],
            1 => vec![rec(0.0, 2.0, 3.0)],
            _ => vec![rec(0.0, 1.0, 5.0), rec(2.0, 1.0, 9.0)],
        }
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
        let p = payload(&state(raw), &facts(), &order(), &regions_of, Fault::default());
        assert_eq!(p["none"], json!(true), "the page changes nothing");
        assert_eq!(p["tracks"], json!([]));
        assert_eq!(
            p["ranges"],
            json!([]),
            "the range of such a selection is not used either"
        );
    }

    #[test]
    fn the_range_goes_from_song_time_to_source_time_through_the_regions_of_the_first_track() {
        // the snare region holds source 0..2 at song 3..5: the range 3.5..4.5 is source 0.5..1.5
        assert_eq!(
            range_to_source(3.5, 1.0, &regions_of(1)),
            RangeOut::Bands(vec![(0.5, 1.5)])
        );
        // a trimmed region (source from 0.4, song from 2.0, length 2.4): song 3.0 is source 1.4
        let RangeOut::Bands(list) = range_to_source(3.0, 1.0, &[rec(0.4, 2.4, 2.0)]) else {
            panic!("inside the region");
        };
        let (a, b) = list[0];
        assert!((a - 1.4).abs() < 1e-9 && (b - 2.4).abs() < 1e-9, "{a} {b}");
        // the same mapping as the follow of the transport: the start of the range is page_time of its start
        let (page, _) = crate::transport::page_time(
            3.5,
            &[(
                1,
                crate::service::RegionRec {
                    start_mod: 0.0,
                    dur_mod: 2.0,
                    start_play: 3.0,
                },
            )],
        );
        assert_eq!(
            range_to_source(3.5, 1.0, &regions_of(1)),
            RangeOut::Bands(vec![(page, page + 1.0)])
        );
    }

    #[test]
    fn a_range_that_leaves_the_region_is_cut_and_one_inside_no_region_is_not_drawn() {
        // 4.5..5.5 holds only 4.5..5.0 of the region
        assert_eq!(
            range_to_source(4.5, 1.0, &regions_of(1)),
            RangeOut::Bands(vec![(1.5, 2.0)])
        );
        // before the region, and behind it
        assert_eq!(range_to_source(0.5, 1.0, &regions_of(1)), RangeOut::Outside);
        assert_eq!(
            range_to_source(5.0, 1.0, &regions_of(1)),
            RangeOut::Outside,
            "the end of a region is not in it"
        );
        // a track with no region at all
        assert_eq!(range_to_source(1.0, 1.0, &[]), RangeOut::Outside);
        // two regions: one band for each part, not one over the gap
        assert_eq!(
            range_to_source(5.5, 4.0, &regions_of(2)),
            RangeOut::Bands(vec![(0.5, 1.0), (2.0, 2.5)])
        );
        // regions that touch in the source are one band
        let touching = [rec(0.0, 1.0, 0.0), rec(1.0, 1.0, 1.0)];
        assert_eq!(range_to_source(0.5, 1.0, &touching), RangeOut::Bands(vec![(0.5, 1.5)]));
    }

    #[test]
    fn a_stretched_region_maps_with_its_length_in_the_song() {
        let ok = |got: RangeOut, want: (f64, f64)| {
            let RangeOut::Bands(l) = got else { panic!("no band") };
            assert!(
                (l[0].0 - want.0).abs() < 1e-9 && (l[0].1 - want.1).abs() < 1e-9,
                "{l:?} want {want:?}"
            );
        };
        // the source slice 0..2 plays in 1 s of the song (double speed): song 0.25..0.75 is source 0.5..1.5
        let fast = [Slice {
            start_mod: 0.0,
            dur_mod: 2.0,
            start_play: 0.0,
            dur_play: 1.0,
        }];
        ok(range_to_source(0.25, 0.5, &fast), (0.5, 1.5));
        // the slice 0..1 plays in 2 s (half speed): song 1.0..2.0 is source 0.5..1.0, and the part behind 2 s is cut
        let slow = [Slice {
            start_mod: 0.0,
            dur_mod: 1.0,
            start_play: 0.0,
            dur_play: 2.0,
        }];
        ok(range_to_source(1.0, 5.0, &slow), (0.5, 1.0));
        // one sample (at 48 kHz) of the song is half a sample of the source in the slow region
        let one = 1.0 / 48_000.0;
        ok(range_to_source(0.0, one, &slow), (0.0, one / 2.0));
    }

    #[test]
    fn an_empty_or_broken_range_is_no_range() {
        for (s, d) in [(1.0, 0.0), (1.0, -2.0), (f64::NAN, 1.0), (1.0, f64::INFINITY)] {
            assert_eq!(range_to_source(s, d, &regions_of(0)), RangeOut::None, "{s} {d}");
        }
    }

    #[test]
    fn a_range_alone_maps_through_the_last_named_track_else_the_first_track() {
        let named = RawSel {
            sequences: vec![2],
            ..RawSel::default()
        };
        let mut st = state(RawSel {
            range: Some((3.5, 1.0)),
            ..RawSel::default()
        });
        st.named = named;
        let p = payload(&st, &facts(), &order(), &regions_of, Fault::default());
        assert_eq!(p["tracks"], json!([]), "a range alone selects no track");
        assert_eq!(p["track_range"], json!("snare"));
        assert_eq!(p["ranges"], json!([{"t0": 0.5, "t1": 1.5}]));
        // no track named yet: the first track of the page
        st.named = RawSel::default();
        st.raw.range = Some((1.0, 1.0));
        let p = payload(&st, &facts(), &order(), &regions_of, Fault::default());
        assert_eq!(p["track_range"], json!("kick"));
        assert_eq!(p["ranges"], json!([{"t0": 1.0, "t1": 2.0}]));
    }

    #[test]
    fn an_empty_selection_removes_the_range_and_a_range_outside_says_why() {
        let p = payload(
            &state(RawSel::default()),
            &facts(),
            &order(),
            &regions_of,
            Fault::default(),
        );
        assert_eq!((p["ranges"].clone(), p["none"].clone()), (json!([]), json!(false)));
        let out = RawSel {
            sequences: vec![2],
            range: Some((20.0, 1.0)),
            ..RawSel::default()
        };
        let p = payload(&state(out), &facts(), &order(), &regions_of, Fault::default());
        assert_eq!(
            (p["ranges"].clone(), p["range_why"].clone()),
            (json!([]), json!("outside"))
        );
        assert_eq!(p["tracks"], json!(["snare"]), "the track is selected all the same");
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
        let p = payload(&st, &facts(), &order(), &regions_of, Fault::default());
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
    fn a_region_with_a_number_that_is_not_finite_gives_no_band_and_many_bands_are_capped() {
        let bad = vec![
            Slice {
                start_mod: 0.0,
                dur_mod: f64::NAN,
                start_play: 0.0,
                dur_play: 1.0,
            },
            Slice {
                start_mod: f64::INFINITY,
                dur_mod: 1.0,
                start_play: 0.0,
                dur_play: 1.0,
            },
        ];
        assert_eq!(range_to_source(0.0, 1.0, &bad), RangeOut::Outside);
        let sel = |range| {
            state(RawSel {
                sequences: vec![2],
                range: Some(range),
                ..RawSel::default()
            })
        };
        let p = payload(&sel((3.0, 1.0)), &facts(), &order(), &|_| bad.clone(), Fault::default());
        assert_eq!(
            (p["ranges"].clone(), p["range_why"].clone()),
            (json!([]), json!("outside")),
            "not ok"
        );
        // 200 regions that do not touch: 64 bands and the reason says so
        let many: Vec<Slice> = (0..200)
            .map(|i| rec(f64::from(i) * 2.0, 0.5, f64::from(i) * 2.0))
            .collect();
        let p = payload(
            &sel((0.0, 1000.0)),
            &facts(),
            &order(),
            &|_| many.clone(),
            Fault::default(),
        );
        assert_eq!(p["ranges"].as_array().unwrap().len(), MAX_BANDS);
        assert_eq!(p["range_why"], json!("capped"));
    }

    #[test]
    fn each_fault_breaks_its_rule() {
        let raw = RawSel {
            sequences: vec![1, 2],
            range: Some((3.5, 1.0)),
            ..RawSel::default()
        };
        let st = state(raw);
        let f = |g: Fault| payload(&st, &facts(), &order(), &regions_of, g);
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
        assert_eq!(
            f(Fault {
                song_range: true,
                ..Fault::default()
            })["ranges"],
            json!([{"t0": 3.5, "t1": 4.5}])
        );
    }
}
