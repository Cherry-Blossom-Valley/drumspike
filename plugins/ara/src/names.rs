//! The display names of the tracks (M1b10, #558).
//!
//! The identity of a track (the key of its hits, its file names, the ARA archive) is the track name
//! the service gave it, made from the name of the audio source. A DAW may give the audio source a
//! machine name (LUNA: 32 hex characters), so the page shows another name: the name that the user
//! sees in the DAW. The display name is a label. It is never a key.
//!
//! The rule, in this order, for the first playback region of a track (the lowest start in song
//! time; the region that the host made first wins a tie):
//!
//! 1. the name of the region sequence (the DAW track) of that region,
//! 2. the name of that playback region,
//! 3. the name of the audio source: the track name as it was before this change.
//!
//! A track without a playback region has the third. Two tracks that get the same display name show
//! a difference: the later one (by the creation order of the sources) gets a number, `Kick (2)`.
//! The function has no host or ARA type in it, so a unit test can reach every case.

use std::collections::HashMap;

/// The longest display name in characters (the page shows it in one line).
const MAX_CHARS: usize = 120;

/// One playback region of a track as the rule needs it.
#[derive(Debug, Clone, PartialEq)]
pub struct RegionView {
    /// Where the region starts in song time (seconds).
    pub start_play: f64,
    /// The creation order of the region (a tie in `start_play` goes to the lower one).
    pub id: u64,
    /// The name of the region sequence of the region, when it has one and the host gave it a name.
    pub sequence_name: Option<String>,
    /// The name of the playback region.
    pub region_name: Option<String>,
    /// The `orderIndex` of the region sequence (the DAW track order).
    pub sequence_order: Option<i32>,
}

/// One track: its identity, the creation order of its source and its regions.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackView {
    /// The track name (the key; never changed here).
    pub track: String,
    /// The creation index of the source of the track.
    pub seq: usize,
    pub regions: Vec<RegionView>,
}

/// The switch of the check: the fault runs break one part of the rule each. A delivery build
/// reads none (see `crate::test_switch`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Fault {
    /// The host names are ignored: every track keeps its own name.
    pub ignore: bool,
    /// The name of the region wins over the name of the region sequence.
    pub region_first: bool,
    /// Two tracks with one name stay with one name.
    pub no_number: bool,
}

impl Fault {
    #[must_use]
    pub fn read() -> Fault {
        match crate::test_switch("DS_ARA_NAMES_FAULT").as_deref() {
            Ok("ignore") => Fault {
                ignore: true,
                ..Fault::default()
            },
            Ok("region-first") => Fault {
                region_first: true,
                ..Fault::default()
            },
            Ok("no-number") => Fault {
                no_number: true,
                ..Fault::default()
            },
            _ => Fault::default(),
        }
    }
}

/// A host name made safe for a label: control characters leave, white space is trimmed and cut to
/// [`MAX_CHARS`]. An empty result is no name.
fn clean(name: &str) -> Option<String> {
    let kept: String = name.chars().filter(|c| !c.is_control()).collect();
    let kept = kept.trim();
    if kept.is_empty() {
        return None;
    }
    Some(kept.chars().take(MAX_CHARS).collect())
}

/// The label of a region for the role guess: the name of its region sequence, else its own name,
/// each cleaned (the rule of the module text, items 1 and 2).
#[must_use]
pub fn pick_label(sequence_name: Option<&str>, region_name: Option<&str>) -> Option<String> {
    sequence_name.and_then(clean).or_else(|| region_name.and_then(clean))
}

/// The first region of a track by song position (a tie goes to the region made first).
fn first_region(t: &TrackView) -> Option<&RegionView> {
    t.regions
        .iter()
        .min_by(|a, b| a.start_play.total_cmp(&b.start_play).then(a.id.cmp(&b.id)))
}

/// The display name of one track before the numbers: the rule of the module text.
fn raw_name(t: &TrackView, fault: Fault) -> String {
    if fault.ignore {
        return t.track.clone();
    }
    let Some(r) = first_region(t) else {
        return t.track.clone();
    };
    let seq = r.sequence_name.as_deref().and_then(clean);
    let region = r.region_name.as_deref().and_then(clean);
    let pick = if fault.region_first {
        region.or(seq)
    } else {
        seq.or(region)
    };
    pick.unwrap_or_else(|| t.track.clone())
}

/// The display name of every track, by track name. Equal names (compared without case) get a
/// number in the order of the DAW tracks (the `orderIndex` of the region sequence of the first
/// region, then the creation index of the source): the first keeps its name, the next ones are
/// `name (2)`, `name (3)`. A numbered name that another track already shows is skipped. The order
/// does not depend on the order in which the host made its objects.
#[must_use]
pub fn display_names(tracks: &[TrackView], fault: Fault) -> HashMap<String, String> {
    let mut order: Vec<&TrackView> = tracks.iter().collect();
    order.sort_by_key(|t| {
        (
            first_region(t).and_then(|r| r.sequence_order).unwrap_or(i32::MAX),
            t.seq,
        )
    });
    let mut taken: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out: HashMap<String, String> = HashMap::new();
    for t in order {
        let base = raw_name(t, fault);
        let mut shown = base.clone();
        if !fault.no_number {
            let mut n = 2;
            while taken.contains(&shown.to_lowercase()) {
                shown = format!("{base} ({n})");
                n += 1;
            }
        }
        taken.insert(shown.to_lowercase());
        out.insert(t.track.clone(), shown);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region(start: f64, id: u64, seq: Option<&str>, name: Option<&str>) -> RegionView {
        RegionView {
            start_play: start,
            id,
            sequence_name: seq.map(str::to_string),
            region_name: name.map(str::to_string),
            sequence_order: None,
        }
    }

    fn track_of(name: &str, seq: usize, regions: Vec<RegionView>) -> TrackView {
        TrackView {
            track: name.into(),
            seq,
            regions,
        }
    }

    #[test]
    fn the_region_sequence_name_wins_then_the_region_name_then_the_source() {
        let t = [
            track_of("aa11", 0, vec![region(0.0, 1, Some("Kick In"), Some("take 3"))]),
            track_of("bb22", 1, vec![region(0.0, 2, None, Some("Snare take"))]),
            track_of("cc33", 2, vec![region(0.0, 3, Some("  "), None)]),
            track_of("dd44", 3, vec![]),
        ];
        let n = display_names(&t, Fault::default());
        assert_eq!(n["aa11"], "Kick In");
        assert_eq!(n["bb22"], "Snare take");
        assert_eq!(
            n["cc33"], "cc33",
            "an empty name is no name: the name of the source stays"
        );
        assert_eq!(
            n["dd44"], "dd44",
            "a source with no region shows the name of the source"
        );
    }

    #[test]
    fn the_first_region_by_song_position_decides() {
        let t = [track_of(
            "aa11",
            0,
            vec![region(9.0, 1, Some("Late"), None), region(2.0, 5, Some("Early"), None)],
        )];
        assert_eq!(display_names(&t, Fault::default())["aa11"], "Early");
        let tie = [track_of(
            "aa11",
            0,
            vec![
                region(1.0, 7, Some("Second"), None),
                region(1.0, 4, Some("First"), None),
            ],
        )];
        assert_eq!(display_names(&tie, Fault::default())["aa11"], "First");
    }

    #[test]
    fn two_tracks_with_one_name_get_a_number_and_keep_their_keys() {
        let t = [
            track_of("aa11", 1, vec![region(0.0, 2, Some("Toms"), None)]),
            track_of("bb22", 0, vec![region(0.0, 1, Some("toms"), None)]),
            track_of("cc33", 2, vec![region(0.0, 3, Some("Toms"), None)]),
        ];
        let n = display_names(&t, Fault::default());
        assert_eq!(n["bb22"], "toms", "the first source keeps the name");
        assert_eq!(n["aa11"], "Toms (2)");
        assert_eq!(n["cc33"], "Toms (3)");
        assert_eq!(n.len(), 3, "three tracks stay three tracks");
    }

    #[test]
    fn the_number_follows_the_order_of_the_daw_tracks_not_the_order_of_creation() {
        let with_order = |track: &str, seq: usize, order: i32| {
            let mut r = region(0.0, seq as u64 + 1, Some("Overheads"), None);
            r.sequence_order = Some(order);
            track_of(track, seq, vec![r])
        };
        // the host made the lower DAW track second, then in the other order: the same names
        let a = [with_order("aa11", 0, 5), with_order("bb22", 1, 2)];
        let b = [with_order("aa11", 1, 5), with_order("bb22", 0, 2)];
        for t in [&a, &b] {
            let n = display_names(t, Fault::default());
            assert_eq!(
                n["bb22"], "Overheads",
                "the DAW track with the lower order keeps the name"
            );
            assert_eq!(n["aa11"], "Overheads (2)");
        }
    }

    #[test]
    fn two_sources_on_one_daw_track_stay_two_tracks_with_a_number() {
        let t = [
            track_of("aa11", 0, vec![region(0.0, 1, Some("Toms"), None)]),
            track_of("bb22", 1, vec![region(0.0, 2, Some("Toms"), None)]),
        ];
        let n = display_names(&t, Fault::default());
        assert_eq!((n["aa11"].as_str(), n["bb22"].as_str()), ("Toms", "Toms (2)"));
    }

    #[test]
    fn a_numbered_name_never_lands_on_a_name_that_is_taken() {
        let t = [
            track_of("aa11", 0, vec![region(0.0, 1, Some("Kick"), None)]),
            track_of("bb22", 1, vec![region(0.0, 2, Some("Kick (2)"), None)]),
            track_of("cc33", 2, vec![region(0.0, 3, Some("Kick"), None)]),
        ];
        let n = display_names(&t, Fault::default());
        assert_eq!(n["aa11"], "Kick");
        assert_eq!(n["bb22"], "Kick (2)");
        assert_eq!(n["cc33"], "Kick (3)");
    }

    #[test]
    fn a_host_name_is_cleaned_and_cut() {
        let t = [track_of(
            "aa11",
            0,
            vec![region(0.0, 1, Some("  Ki\tck\u{7} In  "), None)],
        )];
        assert_eq!(display_names(&t, Fault::default())["aa11"], "Kick In");
        let long = "x".repeat(500);
        let t = [track_of("aa11", 0, vec![region(0.0, 1, Some(&long), None)])];
        assert_eq!(display_names(&t, Fault::default())["aa11"].chars().count(), MAX_CHARS);
    }

    #[test]
    fn the_faults_break_one_part_each() {
        let t = [
            track_of("aa11", 0, vec![region(0.0, 1, Some("Kick"), Some("take"))]),
            track_of("bb22", 1, vec![region(0.0, 2, Some("Kick"), None)]),
        ];
        let ign = display_names(
            &t,
            Fault {
                ignore: true,
                ..Fault::default()
            },
        );
        assert_eq!(ign["aa11"], "aa11");
        let rf = display_names(
            &t,
            Fault {
                region_first: true,
                ..Fault::default()
            },
        );
        assert_eq!(rf["aa11"], "take");
        let nn = display_names(
            &t,
            Fault {
                no_number: true,
                ..Fault::default()
            },
        );
        assert_eq!(nn["bb22"], "Kick");
    }
}
