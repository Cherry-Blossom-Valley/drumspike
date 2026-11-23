//! M1b8 (#555): the rows of the page in song time.
//!
//! The design is `docs/design/daw-plugin/m1b8-song-time.md`. One row of the page is one DAW track (one region sequence
//! of ARA). A row holds the clips of every playback region on that sequence, over all the sources they use:
//! `{src, a, b, s, k}`, where the slice `[a, b)` of the source (seconds) lies in the song from `s`, stretched by `k`.
//! A hit with source time `t` has song time `s + (t - a) * k` (the formula of `to_song` of the export).
//!
//! This file has no host or ARA type in it, so a unit test reaches every case. The document builds the facts and
//! calls [`build`]; the page gets the answer as the shell command `plugin_host_regions` and as the page event
//! `plugin-regions`.
//!
//! The id of a row is the id of its region sequence, so it stays the same when a region moves in time or the DAW track
//! is renamed. Nothing stored uses it.

use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

/// One live playback region as the rows need it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RegionFact {
    /// The id of the region sequence (the DAW track), 0 when the host gave none.
    pub sequence: u64,
    pub start_mod: f64,
    pub dur_mod: f64,
    pub start_play: f64,
    /// The length in the song (`dur_mod` when the host does not stretch).
    pub dur_play: f64,
    /// The id of the playback region.
    pub region_id: u64,
}

/// One source with a track.
#[derive(Debug, Clone, PartialEq)]
pub struct SrcFacts {
    /// The key of the track (the page's key).
    pub key: String,
    /// The creation index of the source.
    pub seq: usize,
    /// The length of the source in seconds (the identity clip).
    pub len_s: f64,
    pub regions: Vec<RegionFact>,
    /// Every clip of this source was deleted in the DAW (it had regions at the last round and has none now).
    pub gone: bool,
    /// The name of the first region (by song position), for a row of a sequence with no name.
    pub region_name: Option<String>,
}

/// What the plugin keeps of a region sequence.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SeqFact {
    pub name: Option<String>,
    pub order: Option<i32>,
}

/// The fault runs of the check (`DS_ARA_SONG_FAULT`); a delivery build reads none (see `crate::test_switch`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Fault {
    /// The regions of the round are used instead of the live ones: a moved clip is not seen (the plugin passes them in).
    pub round_regions: bool,
}

impl Fault {
    #[must_use]
    pub fn read() -> Fault {
        match crate::test_switch("DS_ARA_SONG_FAULT").as_deref() {
            Ok("round-regions") => Fault { round_regions: true },
            _ => Fault::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Clip {
    pub src: String,
    pub a: f64,
    pub b: f64,
    pub s: f64,
    pub k: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub id: String,
    pub name: String,
    /// The keys of the sources of the row by creation index; the first is the home source of the settings.
    pub srcs: Vec<String>,
    pub clips: Vec<Clip>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Rows {
    pub rows: Vec<Row>,
    /// Sources that have no clip in the DAW any more: no row.
    pub gone: Vec<String>,
    /// The end of the last clip in the song.
    pub song_end: f64,
    /// Clips dropped for a value that is not finite or a stretch outside [`K_MIN`, `K_MAX`] (the plugin writes one log row).
    pub bad: usize,
}

/// The stretch of a clip (`dur_play / dur_mod`) must lie in this range: a host that stretches a clip by more is not a stretch the page can draw.
pub const K_MIN: f64 = 1.0 / 64.0;
pub const K_MAX: f64 = 64.0;

fn k_of(r: &RegionFact) -> f64 {
    if r.dur_play > 0.0 {
        r.dur_play / r.dur_mod
    } else {
        1.0
    }
}

/// A region the page can draw: finite numbers and a stretch in range (a length of 0 or less is no clip, as before, and is not counted).
fn drawable(r: &RegionFact) -> bool {
    [r.start_mod, r.dur_mod, r.start_play, r.dur_play]
        .iter()
        .all(|v| v.is_finite())
        && (K_MIN..=K_MAX).contains(&k_of(r))
}

/// A name for a row: the name of the sequence, else the name of its first region, else the key of its first source. `nf` is the fault run
/// of the names (`DS_ARA_NAMES_FAULT`): it breaks the same rule here as for the display names of the tracks.
fn pick(seq: Option<&SeqFact>, region_name: Option<&str>, key: &str, nf: crate::names::Fault) -> String {
    if nf.ignore {
        return key.to_string();
    }
    let seq_name = seq.and_then(|q| q.name.as_deref());
    let picked = if nf.region_first {
        crate::names::pick_label(region_name, seq_name)
    } else {
        crate::names::pick_label(seq_name, region_name)
    };
    picked.unwrap_or_else(|| key.to_string())
}

/// The rows of a document. `seqs` are the region sequences by id.
#[must_use]
pub fn build(sources: &[SrcFacts], seqs: &HashMap<u64, SeqFact>) -> Rows {
    build_with(sources, seqs, crate::names::Fault::default())
}

/// [`build`] with the fault run of the names (`DS_ARA_NAMES_FAULT`).
#[must_use]
pub fn build_with(sources: &[SrcFacts], seqs: &HashMap<u64, SeqFact>, nf: crate::names::Fault) -> Rows {
    struct Acc {
        id: String,
        /// 0: a DAW track; 1: a row of a source with no sequence or no region
        group: u8,
        order: i32,
        first_seq: usize,
        name: String,
        srcs: Vec<(usize, String)>,
        clips: Vec<Clip>,
    }
    let mut accs: Vec<Acc> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut out = Rows::default();
    let mut srcs: Vec<&SrcFacts> = sources.iter().collect();
    srcs.sort_by_key(|s| s.seq);
    for src in srcs {
        if src.gone {
            out.gone.push(src.key.clone());
            continue;
        }
        let usable: Vec<&RegionFact> = src.regions.iter().filter(|r| r.dur_mod > 0.0 && drawable(r)).collect();
        out.bad += src
            .regions
            .iter()
            .filter(|r| !(r.dur_mod.is_finite() && r.dur_mod <= 0.0) && !drawable(r))
            .count();
        let mut place = |id: String, group: u8, seq_id: u64, clip: Clip| {
            let at = *index.entry(id.clone()).or_insert_with(|| {
                let q = seqs.get(&seq_id);
                accs.push(Acc {
                    name: pick(q, src.region_name.as_deref(), &src.key, nf),
                    id,
                    group,
                    order: q.and_then(|q| q.order).unwrap_or(i32::MAX),
                    first_seq: src.seq,
                    srcs: Vec::new(),
                    clips: Vec::new(),
                });
                accs.len() - 1
            });
            let a = &mut accs[at];
            if !a.srcs.iter().any(|(_, k)| *k == src.key) {
                a.srcs.push((src.seq, src.key.clone()));
            }
            a.clips.push(clip);
        };
        if usable.is_empty() {
            // no region ever: the identity clip, song time = source time
            let clip = Clip {
                src: src.key.clone(),
                a: 0.0,
                b: if src.len_s.is_finite() { src.len_s.max(0.0) } else { 0.0 },
                s: 0.0,
                k: 1.0,
            };
            place(format!("s{}", src.seq), 1, 0, clip);
            continue;
        }
        for r in usable {
            let k = k_of(r);
            let clip = Clip {
                src: src.key.clone(),
                a: r.start_mod,
                b: r.start_mod + r.dur_mod,
                s: r.start_play,
                k,
            };
            if r.sequence == 0 {
                place(format!("s{}", src.seq), 1, 0, clip);
            } else {
                place(format!("q{}", r.sequence), 0, r.sequence, clip);
            }
        }
    }
    accs.sort_by(|x, y| (x.group, x.order, x.first_seq, &x.id).cmp(&(y.group, y.order, y.first_seq, &y.id)));
    // equal names (without case) show a number, as the tracks did: the first keeps the name
    let mut taken: HashSet<String> = HashSet::new();
    for a in accs {
        let base = a.name.clone();
        let mut shown = base.clone();
        let mut n = 2;
        while !nf.no_number && taken.contains(&shown.to_lowercase()) {
            shown = format!("{base} ({n})");
            n += 1;
        }
        taken.insert(shown.to_lowercase());
        let mut clips = a.clips;
        clips.sort_by(|p, q| p.s.total_cmp(&q.s).then(p.src.cmp(&q.src)));
        let mut srcs = a.srcs;
        srcs.sort_by_key(|(i, _)| *i);
        for c in &clips {
            out.song_end = out.song_end.max(c.s + (c.b - c.a) * c.k);
        }
        out.rows.push(Row {
            id: a.id,
            name: shown,
            srcs: srcs.into_iter().map(|(_, k)| k).collect(),
            clips,
        });
    }
    out
}

impl Rows {
    /// The answer of `plugin_host_regions` and the payload of the page event `plugin-regions`.
    #[must_use]
    pub fn to_json(&self, gen: u64, doc: &str) -> Value {
        json!({
            "gen": gen,
            "n": gen,
            "doc": doc,
            "rows": self.rows.iter().map(|r| json!({
                "id": r.id, "name": r.name, "srcs": r.srcs,
                "clips": r.clips.iter().map(|c| json!({"src": c.src, "a": c.a, "b": c.b, "s": c.s, "k": c.k})).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
            "song_end": self.song_end,
            "gone": self.gone,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reg(sequence: u64, a: f64, d: f64, p: f64) -> RegionFact {
        RegionFact {
            sequence,
            start_mod: a,
            dur_mod: d,
            start_play: p,
            dur_play: d,
            region_id: 1,
        }
    }
    fn src(key: &str, seq: usize, regions: Vec<RegionFact>) -> SrcFacts {
        SrcFacts {
            key: key.into(),
            seq,
            len_s: 30.0,
            regions,
            gone: false,
            region_name: None,
        }
    }
    fn seq(name: &str, order: i32) -> SeqFact {
        SeqFact {
            name: Some(name.into()),
            order: Some(order),
        }
    }

    #[test]
    fn one_sequence_with_two_sources_is_one_row_with_both_clips_at_their_song_positions() {
        // the owner's case: clip 1 at song 0 to 10 s from one file, clip 2 at 12 to 20 s from another
        let s = [
            src("f1", 0, vec![reg(7, 0.0, 10.0, 0.0)]),
            src("f2", 1, vec![reg(7, 0.0, 8.0, 12.0)]),
        ];
        let seqs = HashMap::from([(7, seq("Kick", 0))]);
        let r = build(&s, &seqs);
        assert_eq!(r.rows.len(), 1);
        assert_eq!(r.rows[0].id, "q7");
        assert_eq!(r.rows[0].name, "Kick");
        assert_eq!(r.rows[0].srcs, ["f1", "f2"]);
        assert_eq!(r.rows[0].clips.len(), 2);
        assert_eq!((r.rows[0].clips[1].src.as_str(), r.rows[0].clips[1].s), ("f2", 12.0));
        assert_eq!(r.song_end, 20.0);
    }

    #[test]
    fn rows_follow_the_daw_order_and_a_double_name_gets_a_number_between_rows_only() {
        let s = [
            src("a", 0, vec![reg(2, 0.0, 5.0, 0.0)]),
            src("b", 1, vec![reg(1, 0.0, 5.0, 0.0)]),
            src("c", 2, vec![reg(3, 0.0, 5.0, 0.0)]),
            src("d", 3, vec![reg(3, 0.0, 5.0, 9.0)]),
        ];
        let seqs = HashMap::from([(1, seq("Snare", 0)), (2, seq("Snare", 1)), (3, seq("Tom", 2))]);
        let r = build(&s, &seqs);
        let names: Vec<_> = r.rows.iter().map(|x| (x.id.as_str(), x.name.as_str())).collect();
        assert_eq!(names, [("q1", "Snare"), ("q2", "Snare (2)"), ("q3", "Tom")]);
        assert_eq!(
            r.rows[2].srcs,
            ["c", "d"],
            "two sources of one DAW track give no number"
        );
    }

    #[test]
    fn a_source_with_no_region_is_its_own_row_with_an_identity_clip_after_the_daw_rows() {
        let s = [src("free", 0, vec![]), src("a", 1, vec![reg(5, 2.0, 4.0, 6.0)])];
        let r = build(&s, &HashMap::from([(5, seq("Hat", 0))]));
        assert_eq!(r.rows[0].id, "q5");
        assert_eq!(r.rows[1].id, "s0");
        assert_eq!(r.rows[1].name, "free");
        assert_eq!(
            r.rows[1].clips,
            [Clip {
                src: "free".into(),
                a: 0.0,
                b: 30.0,
                s: 0.0,
                k: 1.0
            }]
        );
    }

    #[test]
    fn a_source_whose_clips_are_all_gone_has_no_row_and_is_counted() {
        let mut gone = src("x", 0, vec![]);
        gone.gone = true;
        let r = build(&[gone, src("a", 1, vec![reg(5, 0.0, 4.0, 0.0)])], &HashMap::new());
        assert_eq!((r.rows.len(), r.gone.as_slice()), (1, ["x".to_string()].as_slice()));
    }

    #[test]
    fn a_region_with_no_sequence_makes_a_row_of_its_source_and_a_source_on_two_sequences_is_in_both_rows() {
        let s = [
            src("a", 0, vec![reg(0, 0.0, 4.0, 0.0)]),
            src("b", 1, vec![reg(1, 0.0, 4.0, 0.0), reg(2, 1.0, 2.0, 8.0)]),
        ];
        let r = build(&s, &HashMap::from([(1, seq("One", 0)), (2, seq("Two", 1))]));
        let ids: Vec<_> = r.rows.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(ids, ["q1", "q2", "s0"]);
        assert_eq!(r.rows[0].srcs, ["b"]);
        assert_eq!(r.rows[1].srcs, ["b"]);
        assert_eq!(
            (r.rows[1].clips[0].a, r.rows[1].clips[0].s),
            (1.0, 8.0),
            "only the clip of that sequence"
        );
    }

    #[test]
    fn a_stretched_clip_carries_k_and_ends_at_its_length_in_the_song() {
        let mut r = reg(1, 0.0, 4.0, 10.0);
        r.dur_play = 6.0;
        let out = build(&[src("a", 0, vec![r])], &HashMap::new());
        assert_eq!(out.rows[0].clips[0].k, 1.5);
        assert_eq!(out.song_end, 16.0);
    }

    #[test]
    fn the_id_of_a_row_survives_a_region_move_and_a_rename_of_the_daw_track() {
        let before = build(
            &[src("a", 0, vec![reg(7, 1.0, 4.0, 2.0)])],
            &HashMap::from([(7, seq("Kick", 0))]),
        );
        let after = build(
            &[src("a", 0, vec![reg(7, 1.5, 3.0, 30.0)])],
            &HashMap::from([(7, seq("Kick In", 0))]),
        );
        assert_eq!(before.rows[0].id, after.rows[0].id);
        assert_ne!(before.rows[0].name, after.rows[0].name);
        assert_eq!(after.rows[0].clips[0].s, 30.0);
    }

    #[test]
    fn a_clip_with_a_value_that_is_not_finite_or_a_stretch_out_of_range_is_dropped_and_counted() {
        let mut nan = reg(1, 0.0, 4.0, 0.0);
        nan.start_play = f64::NAN;
        let mut inf = reg(1, 0.0, 4.0, 0.0);
        inf.dur_play = f64::INFINITY;
        let mut wild = reg(1, 0.0, 1.0, 0.0);
        wild.dur_play = 1e6;
        let mut tiny = reg(1, 0.0, 1.0, 0.0);
        tiny.dur_play = 1e-6;
        let ok = reg(1, 0.0, 4.0, 2.0);
        let r = build(&[src("a", 0, vec![nan, inf, wild, tiny, ok])], &HashMap::new());
        assert_eq!(r.bad, 4);
        assert_eq!(r.rows.len(), 1);
        assert_eq!(r.rows[0].clips.len(), 1);
        assert!(r.song_end.is_finite());
        // a source whose every region is bad has no usable region: the identity clip
        let r = build(&[src("b", 0, vec![reg(1, f64::NAN, 1.0, 0.0)])], &HashMap::new());
        assert_eq!((r.bad, r.rows[0].id.as_str()), (1, "s0"));
        // a stretch at the edges of the range is still drawn
        let mut edge = reg(1, 0.0, 1.0, 0.0);
        edge.dur_play = K_MAX;
        assert_eq!(build(&[src("c", 0, vec![edge])], &HashMap::new()).bad, 0);
    }

    #[test]
    fn the_json_answer_has_the_shape_the_page_reads() {
        let r = build(
            &[src("a", 0, vec![reg(7, 1.0, 4.0, 2.0)])],
            &HashMap::from([(7, seq("Kick", 0))]),
        );
        let v = r.to_json(3, "d");
        assert_eq!(
            (v["gen"].clone(), v["n"].clone(), v["doc"].clone()),
            (json!(3), json!(3), json!("d"))
        );
        assert_eq!(v["rows"][0]["clips"][0]["s"], 2.0);
        assert_eq!(v["rows"][0]["srcs"][0], "a");
        assert_eq!(v["song_end"], 6.0);
    }
}
