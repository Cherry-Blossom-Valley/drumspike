//! The `.drumspike` document (format 1) as serde structs.
//!
//! Every struct keeps the fields `docs/migration/project-format.md` names as typed fields
//! and everything else in a flattened map, so a document the Python server (or a newer
//! app) wrote round-trips without losing a key. Numbers go through `serde_json` with
//! `float_roundtrip`, so a hit time written by Python's `repr` reads back to the same
//! double, and the maps keep Python's key order (`preserve_order`, `IndexMap`).
//!
//! Two fields distinguish "absent" from `null`, because the Python layer does: a track's
//! `source` (`null` while no file is attached) and the document's `modified` (`null`
//! before the first save). They are `Option<Option<T>>`.
//!
//! The Python layer loads any JSON object and keeps every value as it is. So a typed field
//! here is filled only when the value has the field's type; a `null` on any other typed
//! key, or a value of another type (`"v": 90.0`, `"added": 1`, `"note": 36.0`, `"t": "0.5"`),
//! stays in the struct's `extra` map under its key and is written back unchanged. An event
//! without a numeric `t` cannot be typed, so the track keeps its whole `events` list in
//! `extra`. A document the Python layer accepts therefore loads and saves in the Rust
//! layer without a change of values. On writing, the typed fields that are set come first,
//! then the `extra` keys that no typed field wrote (a field the code set wins over an
//! ill-typed value of the same key).

use indexmap::IndexMap;
use serde::de::{DeserializeOwned, Deserializer, Error as _};
use serde::ser::{SerializeMap, Serializer};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value};

/// The format version this crate reads and writes (`project.FORMAT`).
pub const FORMAT: i64 = 1;
/// Default mix mode of a new project (`project.DEFAULT_MIX_MODE`); imported sessions keep `ffmpeg`.
pub const DEFAULT_MIX_MODE: &str = "average";
/// The accepted `mix_mode` values.
pub const MIX_MODES: [&str; 2] = ["ffmpeg", "average"];
/// The accepted source `mode` values.
pub const SOURCE_MODES: [&str; 2] = ["referenced", "copied"];

/// Move `key` out of `map` when its value reads as a `T`. A `null` or a value of another
/// type stays in `map` (it becomes part of `extra` and is written back as it was).
fn take<T: DeserializeOwned>(map: &mut Map<String, Value>, key: &str) -> Option<T> {
    let v = map.get(key)?;
    if v.is_null() {
        return None;
    }
    let x = T::deserialize(v).ok()?;
    map.shift_remove(key);
    Some(x)
}

/// [`take`] for an `Option<Option<T>>` field, where `null` is a value: absent is `None`,
/// `null` is `Some(None)`, a `T` is `Some(Some(_))`; a value of another type stays in `map`.
fn take_tri<T: DeserializeOwned>(map: &mut Map<String, Value>, key: &str) -> Option<Option<T>> {
    let x = match map.get(key)? {
        Value::Null => None,
        v => Some(T::deserialize(v).ok()?),
    };
    map.shift_remove(key);
    Some(x)
}

/// Writes a struct as a JSON object: the typed fields that are set, in struct order, then
/// the `extra` keys that no typed field wrote.
struct ObjectWriter<M> {
    map: M,
    written: Vec<&'static str>,
}

impl<M: SerializeMap> ObjectWriter<M> {
    fn start<S: Serializer<SerializeMap = M>>(s: S) -> Result<Self, S::Error> {
        Ok(ObjectWriter {
            map: s.serialize_map(None)?,
            written: Vec::new(),
        })
    }

    /// Write `key` when `v` is set (an `Option<Option<T>>` field passes `Some(&None)` for `null`).
    fn field<T: Serialize + ?Sized>(&mut self, key: &'static str, v: Option<&T>) -> Result<(), M::Error> {
        if let Some(v) = v {
            self.map.serialize_entry(key, v)?;
            self.written.push(key);
        }
        Ok(())
    }

    fn finish(mut self, extra: &Map<String, Value>) -> Result<M::Ok, M::Error> {
        for (k, v) in extra {
            if !self.written.contains(&k.as_str()) {
                self.map.serialize_entry(k, v)?;
            }
        }
        self.map.end()
    }
}

/// One hit. `t` is the only required key (a number; an event without one keeps its track's
/// `events` in `extra`); every other key the UI or the engine writes is typed when known
/// (`v`, `added`, `orig`, `edited`, `rise`, `rise_db`, `id`) and carried as it is otherwise.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Event {
    /// Hit time in seconds (never rounded).
    pub t: f64,
    /// MIDI velocity 1 to 127 (`engine.export_midi` defaults a missing one to 100).
    pub v: Option<i64>,
    /// Present and `true` for a hand-added hit.
    pub added: Option<bool>,
    /// The detected time of a hit that was moved by hand.
    pub orig: Option<f64>,
    /// Reserved flag from the plan; carried when present.
    pub edited: Option<bool>,
    /// Envelope rise in dB at the anchor as `analyze_track` stores it (rounded to 0.1 dB).
    pub rise: Option<f64>,
    /// The plan's name for the same value; carried when present.
    pub rise_db: Option<f64>,
    /// C16 (#352): the spread of the three onset estimates (envelope crossing, AIC change
    /// point, cross-correlation lag) in samples. `None` when it was not computed.
    pub timing_confidence: Option<f64>,
    /// C16 (#352): the polarity sign of the hit against the track template, `+1` or `-1`
    /// (`0` when it was not computed).
    pub polarity: Option<i64>,
    /// UI identity (a string like `"l4"`), carried as written.
    pub id: Option<Value>,
    /// Every other key of the event, and a typed key whose value is `null` or of another type.
    pub extra: Map<String, Value>,
}

impl Event {
    /// A plain hit with a time and a velocity.
    pub fn new(t: f64, v: i64) -> Event {
        Event {
            t,
            v: Some(v),
            ..Default::default()
        }
    }
}

/// A source record: where the audio is and enough identity to find it again.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Source {
    /// `referenced` or `copied`.
    pub mode: Option<String>,
    /// Absolute path at the time of the record (the zip export writes `null`, which stays in `extra` as `null`).
    pub path: Option<String>,
    /// Path relative to the project folder, posix separators.
    pub relative: Option<String>,
    /// File size in bytes.
    pub size: Option<i64>,
    /// Modification time, whole seconds.
    pub mtime: Option<i64>,
    /// SHA-1 of the first MiB, lower-case hex.
    pub head_sha1: Option<String>,
    /// `YYYY-MM-DDThh:mm:ssZ` from a Broadcast Wave `bext` chunk; only when the file has one.
    pub bext_origination: Option<String>,
    /// Every other key, and a typed key that is `null` or of another type (a security-scoped bookmark is reserved here for a later build).
    pub extra: Map<String, Value>,
}

impl Source {
    /// The display path of a record: `path`, else `relative`, else empty.
    pub fn path_or_relative(&self) -> &str {
        self.path.as_deref().or(self.relative.as_deref()).unwrap_or("")
    }
}

/// One track of the document.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Track {
    /// `kick`, `snare`, `tom`, `hat`, `cymbal`, `other`.
    pub role: Option<String>,
    /// MIDI note for the export.
    pub note: Option<i64>,
    /// Native sample rate as analysed.
    pub sr: Option<i64>,
    /// Sample count as analysed.
    pub n_samples: Option<i64>,
    /// Duration in seconds as analysed.
    pub duration: Option<f64>,
    /// Lane colour.
    pub color: Option<String>,
    /// Analysis parameters (`thresholds`, `min_gap_ms`, `refine`, `vel_lo_db`, `vel_hi_db`, `shift_samples`).
    pub params: Option<Map<String, Value>>,
    /// `ffmpeg` or `average`.
    pub mix_mode: Option<String>,
    /// The source record; `Some(None)` is the document's `null` (no file attached).
    pub source: Option<Option<Source>>,
    /// The hits.
    pub events: Option<Vec<Event>>,
    /// P3-16: playback volume in dB (-120 and below is off, up to +12); absent is 0 dB. A
    /// monitor setting: no export reads it.
    pub volume_db: Option<f64>,
    /// P3-16: playback pan, -1 (left) to +1 (right); absent is the centre. A monitor setting.
    pub pan: Option<f64>,
    /// P3-18: the name of the track whose hits this track takes (a kick sub mic follows the
    /// kick in). Absent: the track's own detection. The events of a follower are derived.
    pub follows: Option<String>,
    /// 0.9.1: where the role comes from: `guess` (from the file name), `user` (set in the
    /// Inspector or at add time), `audio` (reserved for the audio suggestion of #225). Absent
    /// in older projects: the loader answers `None` and the page treats it as a guess.
    pub role_source: Option<String>,
    /// 0.9.1: the words of the name that matched (`k in`); the Inspector's hint shows them.
    /// Absent with a role the user or the audio set, and in older projects.
    pub role_match: Option<String>,
    /// Every other key, and a typed key that is `null` or of another type.
    pub extra: Map<String, Value>,
}

impl Track {
    /// The source record when one is attached.
    pub fn source_ref(&self) -> Option<&Source> {
        self.source.as_ref().and_then(|s| s.as_ref())
    }

    /// The hits (empty when the key is absent).
    pub fn events(&self) -> &[Event] {
        self.events.as_deref().unwrap_or(&[])
    }

    /// The track's mix mode, `ffmpeg` when absent (what the server assumes).
    pub fn mix_mode_or_default(&self) -> &str {
        self.mix_mode.as_deref().unwrap_or("ffmpeg")
    }
}

/// Project settings.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Settings {
    /// Default mix mode for new tracks.
    pub mix_mode: Option<String>,
    /// Every other key, and a typed key that is `null` or of another type.
    pub extra: Map<String, Value>,
}

/// The document. Only `format` is required; `project.migrate_forward` accepts a format-1
/// document with missing keys, and so does this loader.
#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    /// Format version; always [`FORMAT`] after `migrate_forward`.
    pub format: i64,
    /// Display name (the folder name is the file-system identity).
    pub name: Option<String>,
    /// Unix time of creation; kept as the number Python wrote (int or float).
    pub created: Option<Number>,
    /// Unix time of the last save; `Some(None)` is `null` (never saved).
    pub modified: Option<Option<f64>>,
    /// Project settings.
    pub settings: Option<Settings>,
    /// Display order of the track names; tracks missing here go last.
    pub order: Option<Vec<String>>,
    /// Track name -> track record, in the order written.
    pub tracks: Option<IndexMap<String, Track>>,
    /// View state, carried as written.
    pub view: Option<Map<String, Value>>,
    /// Every other key, and a typed key that is `null` or of another type.
    pub extra: Map<String, Value>,
}

impl Document {
    /// The tracks (empty when the key is absent).
    pub fn tracks(&self) -> impl Iterator<Item = (&String, &Track)> {
        self.tracks.iter().flat_map(|m| m.iter())
    }

    /// The tracks map, created when absent (`doc.setdefault("tracks", {})`).
    pub fn tracks_mut(&mut self) -> &mut IndexMap<String, Track> {
        self.tracks.get_or_insert_with(IndexMap::new)
    }

    /// One track by name.
    pub fn track(&self, name: &str) -> Option<&Track> {
        self.tracks.as_ref().and_then(|m| m.get(name))
    }

    /// One track by name, mutable.
    pub fn track_mut(&mut self, name: &str) -> Option<&mut Track> {
        self.tracks.as_mut().and_then(|m| m.get_mut(name))
    }

    /// `settings.mix_mode` or [`DEFAULT_MIX_MODE`] (`project.default_mix_mode`).
    pub fn default_mix_mode(&self) -> &str {
        self.settings
            .as_ref()
            .and_then(|s| s.mix_mode.as_deref())
            .unwrap_or(DEFAULT_MIX_MODE)
    }

    /// The display order: the stored `order` entries that name a track, then every other
    /// track in document order (`server.track_order`).
    pub fn track_order(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .order
            .iter()
            .flatten()
            .filter(|n| self.track(n).is_some())
            .cloned()
            .collect();
        for (n, _) in self.tracks() {
            if !out.contains(n) {
                out.push(n.clone());
            }
        }
        out
    }

    /// Parse a document from JSON text (no migration, no checks; see `migrate_forward`).
    ///
    /// # Errors
    ///
    /// The `serde_json` error when `text` is not valid JSON or does not fit the document shape.
    pub fn from_json(text: &str) -> Result<Document, serde_json::Error> {
        serde_json::from_str(text)
    }

    /// Compact JSON of the document (the binding hands this to Python's `json.loads`).
    ///
    /// # Panics
    ///
    /// Panics through `expect("a document serialises")` if `serde_json` fails to serialise the
    /// document, which a well-formed document (finite numbers, string keys) never does.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("a document serialises")
    }

    /// The document as Python's `json.dump(doc, f, indent=1)` lays it out: one space per
    /// level, `": "` after a key, one item per line. Floats print in the shortest form that
    /// reads back to the same double (`1e-7` where Python writes `1e-07`; the value is the same).
    ///
    /// # Panics
    ///
    /// Panics through `expect("a document serialises")` if serialisation fails, which a
    /// well-formed document never does; the `expect("serde_json writes UTF-8")` on the output
    /// cannot fire because `serde_json` writes UTF-8.
    pub fn to_json_pretty(&self) -> String {
        let mut buf = Vec::new();
        let fmt = serde_json::ser::PrettyFormatter::with_indent(b" ");
        let mut ser = serde_json::Serializer::with_formatter(&mut buf, fmt);
        self.serialize(&mut ser).expect("a document serialises");
        String::from_utf8(buf).expect("serde_json writes UTF-8")
    }
}

// ---------- serde: typed when the type fits, `extra` otherwise (see the module docs) ----------

impl<'de> Deserialize<'de> for Event {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let mut m = Map::deserialize(d)?;
        let t = take(&mut m, "t").ok_or_else(|| D::Error::custom("event without a numeric `t`"))?;
        Ok(Event {
            t,
            v: take(&mut m, "v"),
            added: take(&mut m, "added"),
            orig: take(&mut m, "orig"),
            edited: take(&mut m, "edited"),
            rise: take(&mut m, "rise"),
            rise_db: take(&mut m, "rise_db"),
            timing_confidence: take(&mut m, "timing_confidence"),
            polarity: take(&mut m, "polarity"),
            id: take(&mut m, "id"),
            extra: m,
        })
    }
}

impl Serialize for Event {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut w = ObjectWriter::start(s)?;
        w.field("t", Some(&self.t))?;
        w.field("v", self.v.as_ref())?;
        w.field("added", self.added.as_ref())?;
        w.field("orig", self.orig.as_ref())?;
        w.field("edited", self.edited.as_ref())?;
        w.field("rise", self.rise.as_ref())?;
        w.field("rise_db", self.rise_db.as_ref())?;
        w.field("timing_confidence", self.timing_confidence.as_ref())?;
        w.field("polarity", self.polarity.as_ref())?;
        w.field("id", self.id.as_ref())?;
        w.finish(&self.extra)
    }
}

impl<'de> Deserialize<'de> for Source {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let mut m = Map::deserialize(d)?;
        Ok(Source {
            mode: take(&mut m, "mode"),
            path: take(&mut m, "path"),
            relative: take(&mut m, "relative"),
            size: take(&mut m, "size"),
            mtime: take(&mut m, "mtime"),
            head_sha1: take(&mut m, "head_sha1"),
            bext_origination: take(&mut m, "bext_origination"),
            extra: m,
        })
    }
}

impl Serialize for Source {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut w = ObjectWriter::start(s)?;
        w.field("mode", self.mode.as_ref())?;
        w.field("path", self.path.as_ref())?;
        w.field("relative", self.relative.as_ref())?;
        w.field("size", self.size.as_ref())?;
        w.field("mtime", self.mtime.as_ref())?;
        w.field("head_sha1", self.head_sha1.as_ref())?;
        w.field("bext_origination", self.bext_origination.as_ref())?;
        w.finish(&self.extra)
    }
}

impl<'de> Deserialize<'de> for Track {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let mut m = Map::deserialize(d)?;
        Ok(Track {
            role: take(&mut m, "role"),
            note: take(&mut m, "note"),
            sr: take(&mut m, "sr"),
            n_samples: take(&mut m, "n_samples"),
            duration: take(&mut m, "duration"),
            color: take(&mut m, "color"),
            params: take(&mut m, "params"),
            mix_mode: take(&mut m, "mix_mode"),
            source: take_tri(&mut m, "source"),
            events: take(&mut m, "events"),
            volume_db: take(&mut m, "volume_db"),
            pan: take(&mut m, "pan"),
            follows: take(&mut m, "follows"),
            role_source: take(&mut m, "role_source"),
            role_match: take(&mut m, "role_match"),
            extra: m,
        })
    }
}

impl Serialize for Track {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut w = ObjectWriter::start(s)?;
        w.field("role", self.role.as_ref())?;
        w.field("note", self.note.as_ref())?;
        w.field("sr", self.sr.as_ref())?;
        w.field("n_samples", self.n_samples.as_ref())?;
        w.field("duration", self.duration.as_ref())?;
        w.field("color", self.color.as_ref())?;
        w.field("params", self.params.as_ref())?;
        w.field("mix_mode", self.mix_mode.as_ref())?;
        w.field("source", self.source.as_ref())?;
        w.field("events", self.events.as_ref())?;
        w.field("volume_db", self.volume_db.as_ref())?;
        w.field("pan", self.pan.as_ref())?;
        w.field("follows", self.follows.as_ref())?;
        w.field("role_source", self.role_source.as_ref())?;
        w.field("role_match", self.role_match.as_ref())?;
        w.finish(&self.extra)
    }
}

impl<'de> Deserialize<'de> for Settings {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let mut m = Map::deserialize(d)?;
        Ok(Settings {
            mix_mode: take(&mut m, "mix_mode"),
            extra: m,
        })
    }
}

impl Serialize for Settings {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut w = ObjectWriter::start(s)?;
        w.field("mix_mode", self.mix_mode.as_ref())?;
        w.finish(&self.extra)
    }
}

impl<'de> Deserialize<'de> for Document {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let mut m = Map::deserialize(d)?;
        let format = take(&mut m, "format").ok_or_else(|| D::Error::custom("missing or non-integer `format`"))?;
        Ok(Document {
            format,
            name: take(&mut m, "name"),
            created: take(&mut m, "created"),
            modified: take_tri(&mut m, "modified"),
            settings: take(&mut m, "settings"),
            order: take(&mut m, "order"),
            tracks: take(&mut m, "tracks"),
            view: take(&mut m, "view"),
            extra: m,
        })
    }
}

impl Serialize for Document {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut w = ObjectWriter::start(s)?;
        w.field("format", Some(&self.format))?;
        w.field("name", self.name.as_ref())?;
        w.field("created", self.created.as_ref())?;
        w.field("modified", self.modified.as_ref())?;
        w.field("settings", self.settings.as_ref())?;
        w.field("order", self.order.as_ref())?;
        w.field("tracks", self.tracks.as_ref())?;
        w.field("view", self.view.as_ref())?;
        w.finish(&self.extra)
    }
}

/// `project.empty_document(name, mix_mode)`.
///
/// # Panics
///
/// Panics at the `expect("finite time")` on `Number::from_f64(now())` if the
/// clock value were not finite; `now()` returns a finite number of seconds, so
/// this does not happen.
pub fn empty_document(name: &str, mix_mode: &str) -> Document {
    Document {
        format: FORMAT,
        name: Some(name.to_string()),
        created: Some(Number::from_f64(now()).expect("finite time")),
        modified: Some(None),
        settings: Some(Settings {
            mix_mode: Some(mix_mode.to_string()),
            extra: Map::new(),
        }),
        order: Some(Vec::new()),
        tracks: Some(IndexMap::new()),
        view: Some(Map::new()),
        extra: Map::new(),
    }
}

/// `time.time()`: seconds since the Unix epoch as a double.
pub fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
}

#[cfg(test)]
mod tests;
