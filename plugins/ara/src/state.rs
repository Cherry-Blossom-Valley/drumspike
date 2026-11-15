//! The state the plugin keeps in the ARA archive of the session: one JSON blob with a
//! version field, the archive ID, the events object of the last analysis and the notes
//! of every source. The blob is what `storeObjectsToArchive` writes and what
//! `restoreObjectsFromArchive` reads; a restored process sets its notes from it and
//! decodes no file again.

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
}

/// The blob.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchiveState {
    /// The version field of the archive (the gate: the archive carries one).
    pub version: i64,
    /// The document archive ID of the factory that wrote it.
    pub archive_id: String,
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
            events,
            sources: vec![ArchiveSource {
                persistent_id: "id-1".into(),
                name: "kick.wav".into(),
                sample_rate: 48_000.0,
                notes: vec![[12_000.0, 0.78], [72_000.0, 0.7]],
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
