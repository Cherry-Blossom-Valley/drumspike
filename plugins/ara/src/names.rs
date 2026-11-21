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

/// The display name of one track before the numbers: the rule of the module text.
fn raw_name(t: &TrackView, fault: Fault) -> String {
    if fault.ignore {
        return t.track.clone();
    }
    let first = t
        .regions
        .iter()
        .min_by(|a, b| a.start_play.total_cmp(&b.start_play).then(a.id.cmp(&b.id)));
    let Some(r) = first else {
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
/// number in the order of the sources: the first keeps its name, the next ones are `name (2)`,
/// `name (3)`. A numbered name that another track already shows is skipped.
#[must_use]
pub fn display_names(tracks: &[TrackView], fault: Fault) -> HashMap<String, String> {
    let mut order: Vec<&TrackView> = tracks.iter().collect();
    order.sort_by_key(|t| t.seq);
    let mut taken: Vec<String> = Vec::new();
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
        taken.push(shown.to_lowercase());
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
        }
    }

    fn track(name: &str, seq: usize, regions: Vec<RegionView>) -> TrackView {
        TrackView {
            track: name.into(),
            seq,
            regions,
        }
    }

    #[test]
    fn the_region_sequence_name_wins_then_the_region_name_then_the_source() {
        let t = [
            track("aa11", 0, vec![region(0.0, 1, Some("Kick In"), Some("take 3"))]),
            track("bb22", 1, vec![region(0.0, 2, None, Some("Snare take"))]),
            track("cc33", 2, vec![region(0.0, 3, Some("  "), None)]),
            track("dd44", 3, vec![]),
        ];
        let n = display_names(&t, Fault::default());
        assert_eq!(n["aa11"], "Kick In");
        assert_eq!(n["bb22"], "Snare take");
        assert_eq!(n["cc33"], "cc33", "an empty name is no name: the name of the source stays");
        assert_eq!(n["dd44"], "dd44", "a source with no region shows the name of the source");
    }

    #[test]
    fn the_first_region_by_song_position_decides() {
        let t = [track(
            "aa11",
            0,
            vec![region(9.0, 1, Some("Late"), None), region(2.0, 5, Some("Early"), None)],
        )];
        assert_eq!(display_names(&t, Fault::default())["aa11"], "Early");
        let tie = [track(
            "aa11",
            0,
            vec![region(1.0, 7, Some("Second"), None), region(1.0, 4, Some("First"), None)],
        )];
        assert_eq!(display_names(&tie, Fault::default())["aa11"], "First");
    }

    #[test]
    fn two_tracks_with_one_name_get_a_number_and_keep_their_keys() {
        let t = [
            track("aa11", 1, vec![region(0.0, 2, Some("Toms"), None)]),
            track("bb22", 0, vec![region(0.0, 1, Some("toms"), None)]),
            track("cc33", 2, vec![region(0.0, 3, Some("Toms"), None)]),
        ];
        let n = display_names(&t, Fault::default());
        assert_eq!(n["bb22"], "toms", "the first source keeps the name");
        assert_eq!(n["aa11"], "Toms (2)");
        assert_eq!(n["cc33"], "Toms (3)");
        assert_eq!(n.len(), 3, "three tracks stay three tracks");
    }

    #[test]
    fn a_numbered_name_never_lands_on_a_name_that_is_taken() {
        let t = [
            track("aa11", 0, vec![region(0.0, 1, Some("Kick"), None)]),
            track("bb22", 1, vec![region(0.0, 2, Some("Kick (2)"), None)]),
            track("cc33", 2, vec![region(0.0, 3, Some("Kick"), None)]),
        ];
        let n = display_names(&t, Fault::default());
        assert_eq!(n["aa11"], "Kick");
        assert_eq!(n["bb22"], "Kick (2)");
        assert_eq!(n["cc33"], "Kick (3)");
    }

    #[test]
    fn a_host_name_is_cleaned_and_cut() {
        let t = [track("aa11", 0, vec![region(0.0, 1, Some("  Ki\tck\u{7} In  "), None)])];
        assert_eq!(display_names(&t, Fault::default())["aa11"], "Kick In");
        let long = "x".repeat(500);
        let t = [track("aa11", 0, vec![region(0.0, 1, Some(&long), None)])];
        assert_eq!(display_names(&t, Fault::default())["aa11"].chars().count(), MAX_CHARS);
    }

    #[test]
    fn the_faults_break_one_part_each() {
        let t = [
            track("aa11", 0, vec![region(0.0, 1, Some("Kick"), Some("take"))]),
            track("bb22", 1, vec![region(0.0, 2, Some("Kick"), None)]),
        ];
        let ign = display_names(&t, Fault { ignore: true, ..Fault::default() });
        assert_eq!(ign["aa11"], "aa11");
        let rf = display_names(&t, Fault { region_first: true, ..Fault::default() });
        assert_eq!(rf["aa11"], "take");
        let nn = display_names(&t, Fault { no_number: true, ..Fault::default() });
        assert_eq!(nn["bb22"], "Kick");
    }
}
