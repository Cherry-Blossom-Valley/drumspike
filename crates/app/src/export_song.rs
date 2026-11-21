//! #560 (M1c1): the song time base of an export.
//!
//! The standalone app exports in the time of the source: file time 0 is sample 0 of the audio file. A DAW
//! plugin exports in song time: every file starts at song position 0 and holds each hit at its song position
//! through the playback regions of the host. The plugin installs an [`ExportTime`] object on the service
//! ([`Service::set_export_time`]); with none installed (the app, the command line, the MCP server) the export
//! code runs its old path unchanged. The page cannot set it: it is not an argument of any command.

use std::sync::Arc;

use drumspike_core::tempo::TempoMap;
use serde_json::Value;

use crate::Service;

/// One playback region of a source: the slice `[start_mod, start_mod + dur_mod)` of the source sits in the
/// song from `start_play` on (all in seconds).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SongRegion {
    /// Start in the source.
    pub start_mod: f64,
    /// Length of the slice.
    pub dur_mod: f64,
    /// Start in the song.
    pub start_play: f64,
}

/// What the host says about one track.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SongTrack {
    /// The regions of the track's source; empty: the source has no region (identity time base).
    pub regions: Vec<SongRegion>,
    /// The file stem of the track's files (the DAW track name); `None`: the track name.
    pub stem: Option<String>,
}

/// The song time base of the host, asked once per export.
pub trait ExportTime: Send + Sync {
    /// The regions and the file stem of a track; `None`: the track is not known to the host (old path for it).
    fn track(&self, name: &str) -> Option<SongTrack>;
    /// The tempo map of the host in song seconds; `None`: the project's own map is used.
    fn tempo(&self) -> Option<TempoMap>;
}

/// The hits that a song-time export left out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SongCounts {
    /// Hits in no region.
    pub no_region: usize,
    /// Hits whose song time is below 0.
    pub before_start: usize,
}

/// The end of the last region in song time; `None` without a region.
#[must_use]
pub fn song_end(regions: &[SongRegion]) -> Option<f64> {
    regions.iter().map(|r| r.start_play + r.dur_mod).reduce(f64::max)
}

/// Map the events of a track (source time) to song time: one copy of an event per region that holds it, its `t`
/// replaced by `start_play + (t - start_mod)`. A hit in no region, or with a song time below 0, is left out
/// and counted. No region: the events as they are (identity). Every other key of an event stays.
#[must_use]
pub fn to_song(events: &[Value], regions: &[SongRegion]) -> (Vec<Value>, SongCounts) {
    if regions.is_empty() {
        return (events.to_vec(), SongCounts::default());
    }
    let mut out = Vec::with_capacity(events.len());
    let mut counts = SongCounts::default();
    for e in events {
        let Some(t) = e.get("t").and_then(Value::as_f64) else {
            out.push(e.clone()); // the writers refuse an event with no time, with their own message
            continue;
        };
        let holding: Vec<f64> = regions
            .iter()
            .filter(|r| t >= r.start_mod && t < r.start_mod + r.dur_mod)
            .map(|r| r.start_play + (t - r.start_mod))
            .collect();
        if holding.is_empty() {
            counts.no_region += 1;
            continue;
        }
        let mut kept = false;
        for s in holding.into_iter().filter(|s| *s >= 0.0) {
            kept = true;
            let mut c = e.clone();
            c["t"] = Value::from(s);
            out.push(c);
        }
        if !kept {
            counts.before_start += 1;
        }
    }
    (out, counts)
}

impl Service {
    /// Install the song time base of the host (the DAW plugin), or remove it.
    pub fn set_export_time(&self, t: Option<Arc<dyn ExportTime>>) {
        *self.inner.export_time.write().unwrap_or_else(|e| e.into_inner()) = t;
    }

    pub(crate) fn export_time(&self) -> Option<Arc<dyn ExportTime>> {
        self.inner.export_time.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// The file stem of a track's export files: the host's name for it, else the track name.
    pub(crate) fn out_stem(&self, name: &str) -> String {
        self.export_time()
            .and_then(|t| t.track(name))
            .and_then(|t| t.stem)
            .unwrap_or_else(|| name.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn r(a: f64, d: f64, p: f64) -> SongRegion {
        SongRegion {
            start_mod: a,
            dur_mod: d,
            start_play: p,
        }
    }
    fn ts(v: &[Value]) -> Vec<f64> {
        v.iter().map(|e| e["t"].as_f64().unwrap()).collect()
    }

    #[test]
    fn a_hit_goes_to_its_song_position_through_the_region() {
        let ev = vec![json!({"t": 1.5, "v": 90, "keep": true})];
        let (o, c) = to_song(&ev, &[r(1.0, 2.0, 10.0)]);
        assert_eq!(ts(&o), vec![10.5]);
        assert_eq!(o[0]["v"], 90);
        assert_eq!(o[0]["keep"], true, "the other keys stay");
        assert_eq!(c, SongCounts::default());
    }

    #[test]
    fn a_source_with_two_regions_gives_its_hit_once_per_region() {
        let ev = vec![json!({"t": 1.5, "v": 90})];
        let (o, _) = to_song(&ev, &[r(1.0, 2.0, 10.0), r(0.0, 4.0, 20.0)]);
        assert_eq!(ts(&o), vec![10.5, 21.5]);
    }

    #[test]
    fn a_hit_in_no_region_or_before_song_zero_is_left_out_and_counted() {
        let ev = vec![
            json!({"t": 0.5}),
            json!({"t": 1.5}),
            json!({"t": 5.0}),
            json!({"t": 2.5}),
        ];
        // regions: [1,2) at -1 (hit 1.5 -> -0.5 before the start), [2,3) at 0 (hit 2.5 -> 0.5)
        let (o, c) = to_song(&ev, &[r(1.0, 1.0, -1.0), r(2.0, 1.0, 0.0)]);
        assert_eq!(ts(&o), vec![0.5]);
        assert_eq!(
            c,
            SongCounts {
                no_region: 2,
                before_start: 1
            }
        );
    }

    #[test]
    fn a_source_with_no_region_keeps_its_time() {
        let ev = vec![json!({"t": 1.5})];
        let (o, c) = to_song(&ev, &[]);
        assert_eq!(o, ev);
        assert_eq!(c, SongCounts::default());
        assert_eq!(song_end(&[]), None);
        assert_eq!(song_end(&[r(0.0, 2.0, 3.0), r(0.0, 1.0, 9.0)]), Some(10.0));
    }
}
