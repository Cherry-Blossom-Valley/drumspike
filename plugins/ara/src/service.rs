//! The service job of the plugin: build a Drum Spike project from the ARA document (one
//! track per audio source, the host's playback regions as the segments of P1) in the
//! cache folder, and run the app service on it — the same `headless::analyze_projects`
//! the desktop tool calls, so the event lists are the service's own, byte for byte.
//!
//! Where things live (all below the one cache folder the README names):
//!
//! ```text
//! <cache>/audio/<seq>-<stem>.wav      the source audio, one file per ARA audio source
//! <cache>/session/                    the project folder (the document, the Analysis cache)
//! ```
//!
//! The track's timeline is the audio source's own timeline (ARA content is per source,
//! in the source's time): a playback region plays the slice
//! `[startInModificationTime, startInModificationTime + durationInModificationTime)` of
//! the source, so its segment places that slice at its own modification position. A
//! source with no playback regions is a plain track (the whole file from its first
//! sample).
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use drumspike_app::roles::{default_note, guess_role};
use drumspike_app::tracks::safe_name;
use drumspike_core::project::{build_source, write_document, Document, Segment, Source, Track, FORMAT};
use serde_json::{json, Value};

/// One playback region of a source, as the document controller recorded it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RegionRec {
    /// `startInModificationTime`.
    pub start_mod: f64,
    /// `durationInModificationTime`.
    pub dur_mod: f64,
    /// `startInPlaybackTime` (carried for the renderer; the analysis does not use it).
    pub start_play: f64,
}

/// What the job needs from one source (a snapshot, taken under the plugin's lock).
#[derive(Debug, Clone)]
pub struct SourceSnapshot {
    /// The creation index of the source (stable key for the results).
    pub seq: usize,
    /// The ARA name of the source (its file path in the test host).
    pub name: String,
    /// The persistent ID of the source.
    pub persistent_id: String,
    /// Sample rate.
    pub sample_rate: f64,
    /// Sample count.
    pub sample_count: i64,
    /// The cached WAV file of the source (written by the reader thread).
    pub wav: PathBuf,
    /// The playback regions of the source.
    pub regions: Vec<RegionRec>,
}

/// What one job produced.
#[derive(Debug, Clone)]
pub struct JobResult {
    /// The events object of the service answer, `{track: [events]}`, as the service
    /// wrote it (the bytes the check compares).
    pub events: serde_json::Map<String, Value>,
    /// The decode counter of the service after the job (per file).
    pub decode_counts: HashMap<PathBuf, usize>,
    /// The analysis wall time.
    pub analysis_ms: u64,
    /// The notes per source: `(sample in the source, volume)` per event.
    pub notes: HashMap<usize, Vec<[f64; 2]>>,
    /// The track name of every source.
    pub track_of: HashMap<usize, String>,
}

/// The file stem of a source name (the ARA name of the test host is the file path).
#[must_use]
pub fn stem_of(name: &str) -> String {
    Path::new(name)
        .file_stem()
        .map_or_else(|| name.to_string(), |s| s.to_string_lossy().into_owned())
}

/// The source sample of a timeline event `t`. The track's timeline is the source's own
/// timeline (each region places the slice `[startInModificationTime, ..)` at its own
/// modification position), so the event's sample is `t * rate`; a region must hold `t`
/// (a hit in a gap or a trimmed-away part does not exist).
#[must_use]
pub fn source_sample(t: f64, regions: &[RegionRec], rate: f64) -> Option<i64> {
    let in_a_region = regions.iter().any(|r| t >= r.start_mod && t < r.start_mod + r.dur_mod);
    if !regions.is_empty() && !in_a_region {
        return None;
    }
    Some((t * rate).round() as i64)
}

/// Run one job: write the project, analyse it with the service, map the events back into
/// the sources.
///
/// # Errors
///
/// A message when the project cannot be written or the service refuses the analysis.
pub fn run(sources: &[SourceSnapshot], cache: &Path) -> Result<JobResult, String> {
    let started = std::time::Instant::now();
    let dir = cache.join("session");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let audio_dir = cache.join("audio");
    std::fs::create_dir_all(&audio_dir).map_err(|e| format!("{}: {e}", audio_dir.display()))?;

    let mut doc = Document {
        format: FORMAT,
        name: Some(
            dir.file_name()
                .map_or_else(|| "ara".into(), |s| s.to_string_lossy().into_owned()),
        ),
        created: None,
        modified: None,
        settings: None,
        order: Some(Vec::new()),
        tracks: Some(indexmap::IndexMap::new()),
        view: None,
        extra: Default::default(),
    };
    let mut track_of: HashMap<usize, String> = HashMap::new();
    let mut wav_of_track: HashMap<String, PathBuf> = HashMap::new();
    for (i, s) in sources.iter().enumerate() {
        let stem = stem_of(&s.name);
        let name = safe_name(&stem, i);
        let role = guess_role(&stem).unwrap_or("other");
        let segments: Option<Vec<Segment>> = if s.regions.is_empty() {
            None
        } else {
            Some(
                s.regions
                    .iter()
                    .map(|r| Segment {
                        start: r.start_mod,
                        offset: r.start_mod,
                        length: r.dur_mod,
                        source: Some(Some(wav_source(&s.wav, &dir))),
                        extra: Default::default(),
                    })
                    .collect(),
            )
        };
        let track = Track {
            role: Some(role.to_string()),
            note: Some(default_note(role)),
            mix_mode: Some("average".into()),
            source: segments.is_none().then(|| Some(wav_source(&s.wav, &dir))),
            events: Some(Vec::new()),
            segments,
            ..Track::default()
        };
        doc.order.as_mut().unwrap().push(name.clone());
        doc.tracks.as_mut().unwrap().insert(name.clone(), track);
        track_of.insert(s.seq, name.clone());
        wav_of_track.insert(name, s.wav.clone());
    }
    let doc_path = dir.join(format!(
        "{}.drumspike",
        dir.file_name().unwrap_or_default().to_string_lossy()
    ));
    write_document(&doc, &doc_path).map_err(|e| format!("{}: {e}", doc_path.display()))?;

    let svc = drumspike_app::Service::new(drumspike_app::ServiceConfig::from_env(Path::new("/")));
    let dirs = vec![dir.display().to_string()];
    let answer = drumspike_app::headless::analyze_projects(&svc, &dirs)?;
    let analysis_ms = started.elapsed().as_millis() as u64;
    let first = answer
        .first()
        .ok_or_else(|| "the service returned no answer".to_string())?;
    let events = first
        .get("events")
        .and_then(Value::as_object)
        .cloned()
        .ok_or_else(|| "the service answer carries no events object".to_string())?;

    // the notes: the events of the track per source, mapped back into the source
    let mut notes: HashMap<usize, Vec<[f64; 2]>> = HashMap::new();
    for s in sources {
        let track_name = &track_of[&s.seq];
        let list = events
            .get(track_name)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut per_source: Vec<[f64; 2]> = Vec::with_capacity(list.len());
        for ev in &list {
            let t = ev.get("t").and_then(Value::as_f64).unwrap_or(f64::NAN);
            let v = ev.get("v").and_then(Value::as_i64).unwrap_or(100);
            if let Some(sample) = source_sample(t, &s.regions, s.sample_rate) {
                per_source.push([sample as f64, (v.clamp(0, 127) as f64) / 127.0]);
            }
        }
        notes.insert(s.seq, per_source);
    }
    let decode_counts = svc.decode_counts();
    Ok(JobResult {
        events,
        decode_counts,
        analysis_ms,
        notes,
        track_of,
    })
}

/// The source record of a cached WAV (size and head hash included, so the service
/// resolves it).
fn wav_source(wav: &Path, project_dir: &Path) -> Source {
    build_source(wav, project_dir, "referenced").unwrap_or_else(|_| Source {
        mode: Some("referenced".into()),
        path: Some(wav.display().to_string()),
        ..Source::default()
    })
}

/// The events object as one JSON line value (the bytes the check compares).
#[must_use]
pub fn events_line(events: &serde_json::Map<String, Value>) -> String {
    serde_json::to_string(&json!(events)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_sample_maps_the_timeline_back_into_the_file() {
        let regions = vec![
            RegionRec {
                start_mod: 0.0,
                dur_mod: 1.0,
                start_play: 0.0,
            },
            RegionRec {
                start_mod: 2.0,
                dur_mod: 0.5,
                start_play: 5.0,
            },
        ];
        let rate = 48_000.0;
        // the timeline is the source's own: an event at t sits at sample t * rate
        assert_eq!(source_sample(0.25, &regions, rate), Some(12_000));
        assert_eq!(source_sample(2.1, &regions, rate), Some(100_800));
        // a gap between the regions has no audio
        assert_eq!(source_sample(1.5, &regions, rate), None);
        // without regions the whole file exists
        assert_eq!(source_sample(0.25, &[], rate), Some(12_000));
    }

    #[test]
    fn stem_of_takes_the_file_stem_of_a_path() {
        assert_eq!(stem_of("/a/b/kick_48k.wav"), "kick_48k");
        assert_eq!(stem_of("kick.wav"), "kick");
    }
}
