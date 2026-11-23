//! The state the plugin keeps in the ARA archive of the session: one JSON blob with a
//! version field, the archive ID, the document id, the events object of the project and
//! the notes of every source. The blob is what `storeObjectsToArchive` writes and what
//! `restoreObjectsFromArchive` reads; a restored process sets its notes from it, opens the
//! project folder of the stored document id with the stored events and decodes no file.
//!
//! Version 1 is the only version. M1b1 added two fields, `doc_id` (the document id, which
//! names the project folder) and `track` (the track name of a source); both are optional
//! on reading (`serde(default)`), so an archive of M1a loads: it has no document id, and
//! the restore makes a fresh one. A reader of M1a ignores the new fields (no field is
//! denied), so no version bump is needed; the committed fixture
//! `fixtures/archive-v1.ara` is an archive of M1a and a test loads it.

use serde::{Deserialize, Serialize};

/// The version of the blob the current plugin writes (a restore refuses another one).
pub const ARCHIVE_VERSION: i64 = 1;

/// The notes of one source: `(sample position in the source, volume 0..1)`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchiveSource {
    /// The persistent ID of the ARA audio source.
    #[serde(rename = "persistent_id")]
    pub persistent_id: String,
    /// The name of the ARA audio source (the fallback key when the ID is missing).
    pub name: String,
    /// Sample rate of the source.
    pub sample_rate: f64,
    /// One note per event: the exact sample and the volume.
    pub notes: Vec<[f64; 2]>,
    /// The track of the source in the project (empty when the source was not analysed, and
    /// in an archive of M1a: the restore then derives it from the name).
    #[serde(default)]
    pub track: String,
    /// The events of the track are STALE: its last round ended with an error and the events
    /// are from an earlier round. After a restore the track needs a new analysis (an archive
    /// without the field loads as not stale).
    #[serde(default)]
    pub stale: bool,
    /// M1b10 (N10): everything the user set on the track record, as the service holds it (role,
    /// `role_source`, note, colour, detector parameters, mix, Learn data, layer, sampler ...), without the
    /// events (they are in `events`) and without what the plugin makes from the source (segments, source,
    /// rate, length). A restore puts it back and guesses nothing for such a track. Empty (and absent in the
    /// file) in an archive of an earlier plugin: the restore then guesses the role, as it always did. The
    /// version of the archive stays 1: the field is optional in both directions (an older plugin ignores it).
    #[serde(
        default,
        deserialize_with = "lenient_map",
        skip_serializing_if = "serde_json::Map::is_empty"
    )]
    pub settings: serde_json::Map<String, serde_json::Value>,
    /// M1b10 (R4): the sample count of the source when the archive was stored (0: not stored, an archive of an
    /// earlier plugin). A restore whose source has another count knows that the audio changed while the session
    /// was closed: the track is analysed again (the rule of a stale track), the hand edits stay.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub sample_count: i64,
    /// F6 (#582): the hash (FNV-1a, 16 hex digits) of the samples the plugin read when the archive was stored
    /// (empty: not read in that session, or an archive of an earlier plugin). A restore that reads the audio again
    /// compares it: the same hash keeps the hits and the settings, another one makes a new analysis.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub content_hash: String,
}

fn is_zero(n: &i64) -> bool {
    *n == 0
}

/// `settings` is read leniently (R2): `null`, a list or anything that is not an object is "no settings", so a
/// damaged field never makes the whole archive (the hits too) fail to parse.
fn lenient_map<'de, D: serde::Deserializer<'de>>(d: D) -> Result<serde_json::Map<String, serde_json::Value>, D::Error> {
    Ok(match serde_json::Value::deserialize(d)? {
        serde_json::Value::Object(m) => m,
        _ => serde_json::Map::new(),
    })
}

/// The blob.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchiveState {
    /// The version field of the archive (the gate: the archive carries one).
    pub version: i64,
    /// The document archive ID of the factory that wrote it.
    pub archive_id: String,
    /// The id of the document: the name of its project folder below the cache folder
    /// (empty in an archive of M1a).
    #[serde(default)]
    pub doc_id: String,
    /// The events object of the last analysis, as the service wrote it (`{track: [...]}`).
    #[serde(default)]
    pub events: serde_json::Map<String, serde_json::Value>,
    /// The notes of every source.
    #[serde(default)]
    pub sources: Vec<ArchiveSource>,
}

/// Serialise the state.
#[must_use]
pub fn encode(state: &ArchiveState) -> Vec<u8> {
    serde_json::to_vec(state).unwrap_or_default()
}

/// Parse a blob.
///
/// # Errors
///
/// A message when the bytes are not an [`ArchiveState`] or carry another version.
pub fn decode(bytes: &[u8]) -> Result<ArchiveState, String> {
    let state: ArchiveState =
        serde_json::from_slice(bytes).map_err(|e| format!("the archive is not a Drum Spike ARA state: {e}"))?;
    if state.version != ARCHIVE_VERSION {
        return Err(format!(
            "the archive carries version {}, this plugin reads {ARCHIVE_VERSION}",
            state.version
        ));
    }
    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blob_round_trips_and_carries_the_version_field() {
        let mut events = serde_json::Map::new();
        events.insert(
            "kick".into(),
            serde_json::json!([{"t": 0.25, "v": 100}, {"t": 1.5, "v": 90}]),
        );
        let state = ArchiveState {
            version: ARCHIVE_VERSION,
            archive_id: "com.cherryblossomvalley.drumspike.ara.archive.v1".into(),
            doc_id: "0123456789abcdef".into(),
            events,
            sources: vec![ArchiveSource {
                persistent_id: "id-1".into(),
                name: "kick.wav".into(),
                sample_rate: 48_000.0,
                notes: vec![[12_000.0, 0.78], [72_000.0, 0.7]],
                track: "kick".into(),
                stale: true,
                settings: serde_json::Map::new(),
                sample_count: 0,
                content_hash: String::new(),
            }],
        };
        let bytes = encode(&state);
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(
            text.contains("\"version\":1"),
            "the archive carries a version field: {text}"
        );
        let back = decode(&bytes).unwrap();
        assert_eq!(back, state);
        assert!(
            back.sources[0].stale,
            "the stale mark of a track survives store and restore"
        );
    }

    #[test]
    fn an_archive_of_m1a_without_document_id_and_track_loads() {
        let bytes = br#"{"version":1,"archive_id":"x","events":{"kick":[{"t":0.1,"v":90}]},
            "sources":[{"persistent_id":"p","name":"kick.wav","sample_rate":48000.0,"notes":[[4800.0,0.7]]}]}"#;
        let st = decode(bytes).unwrap();
        assert_eq!(st.doc_id, "");
        assert_eq!(st.sources[0].track, "");
        assert!(!st.sources[0].stale, "an archive without the mark loads as not stale");
        assert_eq!(st.sources[0].notes, vec![[4800.0, 0.7]]);
    }

    #[test]
    fn the_committed_fixture_of_m1a_loads() {
        let st = decode(include_bytes!("../fixtures/archive-v1.ara")).unwrap();
        assert_eq!(st.version, 1);
        assert_eq!(st.doc_id, "", "the fixture is an archive of M1a: no document id");
        assert!(!st.sources.is_empty() && !st.events.is_empty());
        assert!(st.sources.iter().all(|s| s.track.is_empty()));
    }

    #[test]
    fn a_reader_of_m1a_ignores_the_new_fields() {
        // the blob M1a read: no deny_unknown_fields, so the new fields do not refuse it
        #[derive(serde::Deserialize)]
        struct OldState {
            version: i64,
            #[allow(dead_code)]
            sources: Vec<OldSource>,
        }
        #[derive(serde::Deserialize)]
        struct OldSource {
            #[allow(dead_code)]
            name: String,
        }
        let state = ArchiveState {
            version: ARCHIVE_VERSION,
            archive_id: "x".into(),
            doc_id: "abc".into(),
            events: serde_json::Map::new(),
            sources: vec![ArchiveSource {
                persistent_id: "p".into(),
                name: "kick.wav".into(),
                sample_rate: 48_000.0,
                notes: Vec::new(),
                track: "kick".into(),
                stale: false,
                settings: serde_json::Map::new(),
                sample_count: 0,
                content_hash: String::new(),
            }],
        };
        let old: OldState = serde_json::from_slice(&encode(&state)).unwrap();
        assert_eq!(old.version, 1);
    }

    #[test]
    fn a_damaged_settings_field_never_fails_the_archive_and_an_old_reader_ignores_a_full_one() {
        for bad in ["null", "[1,2]", "\"x\"", "7"] {
            let text = format!(
                r#"{{"version":1,"archive_id":"x","events":{{"k":[{{"t":0.1}}]}},"sources":[{{"persistent_id":"p","name":"k.wav","sample_rate":48000.0,"notes":[],"settings":{bad}}}]}}"#
            );
            let st = decode(text.as_bytes()).unwrap_or_else(|e| panic!("{bad}: {e}"));
            assert!(st.sources[0].settings.is_empty(), "{bad}: no settings");
            assert_eq!(st.events.len(), 1, "{bad}: the hits are still there");
        }
        // a reader of an earlier plugin (no `settings`, no `sample_count`) reads an archive that has both
        #[derive(serde::Deserialize)]
        struct OldSource {
            name: String,
            notes: Vec<[f64; 2]>,
        }
        #[derive(serde::Deserialize)]
        struct OldState {
            version: i64,
            sources: Vec<OldSource>,
        }
        let mut settings = serde_json::Map::new();
        settings.insert("role".into(), serde_json::json!("snare"));
        settings.insert("params".into(), serde_json::json!({"min_gap_ms": 30}));
        let state = ArchiveState {
            version: ARCHIVE_VERSION,
            archive_id: "x".into(),
            doc_id: "abc".into(),
            events: serde_json::Map::new(),
            sources: vec![ArchiveSource {
                persistent_id: "p".into(),
                name: "snare.wav".into(),
                sample_rate: 48_000.0,
                notes: vec![[1.0, 0.5]],
                track: "snare".into(),
                stale: false,
                settings,
                sample_count: 96_000,
                content_hash: String::new(),
            }],
        };
        let bytes = encode(&state);
        let old: OldState = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            (old.version, old.sources[0].name.as_str(), old.sources[0].notes.len()),
            (1, "snare.wav", 1)
        );
        assert_eq!(decode(&bytes).unwrap(), state, "and the new reader round-trips it");
    }

    #[test]
    fn the_content_hash_round_trips_and_an_archive_without_it_loads() {
        let text = r#"{"version":1,"archive_id":"x","events":{},"sources":[{"persistent_id":"p","name":"k.wav","sample_rate":48000.0,"notes":[],"content_hash":"00000000000000ff"}]}"#;
        let st = decode(text.as_bytes()).unwrap();
        assert_eq!(st.sources[0].content_hash, "00000000000000ff");
        assert!(String::from_utf8(encode(&st)).unwrap().contains("content_hash"));
        let old = decode(br#"{"version":1,"archive_id":"x","events":{},"sources":[{"persistent_id":"p","name":"k.wav","sample_rate":48000.0,"notes":[]}]}"#).unwrap();
        assert_eq!(old.sources[0].content_hash, "");
        assert!(
            !String::from_utf8(encode(&old)).unwrap().contains("content_hash"),
            "nothing is written for a source that was not read"
        );
    }

    #[test]
    fn a_reader_of_an_earlier_plugin_loads_an_archive_with_a_content_hash() {
        #[derive(serde::Deserialize)]
        struct OldSource {
            name: String,
        }
        #[derive(serde::Deserialize)]
        struct OldState {
            version: i64,
            sources: Vec<OldSource>,
        }
        let text = r#"{"version":1,"archive_id":"x","events":{},"sources":[{"persistent_id":"p","name":"k.wav","sample_rate":48000.0,"notes":[],"content_hash":"00000000000000ff"}]}"#;
        let old: OldState = serde_json::from_str(text).unwrap();
        assert_eq!((old.version, old.sources[0].name.as_str()), (1, "k.wav"));
    }

    #[test]
    fn a_foreign_version_is_refused() {
        let bytes = br#"{"version":99,"archive_id":"x","events":{},"sources":[]}"#;
        let e = decode(bytes).unwrap_err();
        assert!(e.contains("version 99"), "{e}");
    }

    #[test]
    fn not_a_state_is_refused() {
        assert!(decode(b"not json").is_err());
    }
}
