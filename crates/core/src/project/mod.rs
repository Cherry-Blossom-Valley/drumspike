//! The project document layer: the port of `project.py` (phase 0, format 1).
//!
//! A project is a user-visible folder (`docs/migration/project-format.md`):
//!
//! ```text
//! <Name>/
//!   <Name>.drumspike      the document: one JSON file, the only file save_project writes
//!   Audio Files/          copied source audio
//!   Analysis/             regenerable cache
//!   Exports/              MIDI and spike files
//!   Backups/              pre-save copies and autosaves (the last BACKUPS_KEEP are kept)
//! ```
//!
//! The module mirrors the Python functions one for one and keeps their error texts, so the
//! server's error mapping (`server.project_error`) works on either implementation:
//!
//! * [`document`]: the serde structs ([`Document`], [`Track`], [`Event`], [`Source`]).
//! * this file: names and paths, [`find_document`], [`load_project`], [`migrate_forward`]
//!   (format 0 -> 1), [`legacy_session_document`], [`new_project`], [`add_track`].
//! * [`source`]: source records (`build_source`, `head_sha1`, `bext_origination`), the
//!   resolution order, `copy_into_project`, `consolidate` and the `Analysis/` cache key.
//! * [`save`]: the atomic write, the backup rotation, `save_project` and `save_as`.
//!
//! Hit times and velocities pass through `serde_json` (`float_roundtrip`) untouched, so
//! they survive every round trip bit-exact (I1, I2). Source files are only read (I3).

pub mod document;
pub mod save;
pub mod source;

use std::fmt;
use std::path::{Component, Path, PathBuf};

use serde_json::{Map, Number, Value};

pub use document::{
    DEFAULT_MIX_MODE, Document, Event, FORMAT, MIX_MODES, SOURCE_MODES, Settings, Source, Track, empty_document, now,
};
pub use save::{
    BACKUPS_KEEP, backup_name, free_name, rotate_backups, save_as, save_project, save_project_into,
    save_project_report, write_document,
};
pub use source::{
    COPY_CHUNK, CacheKey, CopyCtl, CopyOutcome, HEAD_BYTES, HEAD_CACHE_MAX, HEAD_CACHE_SETTLE, HeadCacheCounts,
    SEARCH_MAX_FILES, SearchHit, SearchReport, SearchResult, SearchTarget, SourceStatus, bext_origination,
    build_source, cache_key, consolidate, copy_chunked, copy_into_project, copy_into_project_ctl, head_cache_counts,
    head_sha1, relink_search, relinked_source, resolve_source, source_candidates, source_matches,
};

/// Document file extension.
pub const DOC_EXT: &str = ".drumspike";
/// Folder of copied source audio.
pub const AUDIO_DIR: &str = "Audio Files";
/// Folder of the regenerable cache.
pub const ANALYSIS_DIR: &str = "Analysis";
/// Folder of exported MIDI and spike files.
pub const EXPORTS_DIR: &str = "Exports";
/// Folder of pre-save copies and autosaves.
pub const BACKUPS_DIR: &str = "Backups";
/// The folders `ensure_layout` creates.
pub const SUBDIRS: [&str; 4] = [AUDIO_DIR, ANALYSIS_DIR, EXPORTS_DIR, BACKUPS_DIR];

/// Any project document problem the caller should show to the user (`project.ProjectError`).
#[derive(Debug)]
pub enum ProjectError {
    /// The document was written by a newer app (`project.ProjectFormatError`).
    Format(String),
    /// Every other document problem, with the Python layer's message text.
    Invalid(String),
    /// The target project folder already holds a document (`project already exists: <dir>`).
    Exists(String),
    /// No document in the folder (`no .drumspike document at <path>`).
    NoDocument(String),
    /// A track's source is missing or changed, so Save As cannot copy it
    /// (`track <track>: source <reason>; relink it before Save As`).
    NeedsRelink {
        /// The track's name.
        track: String,
        /// The source's status (`missing`, `changed`).
        reason: String,
    },
    /// A source cannot be copied into the project (`cannot copy source (<reason>): <path>`).
    CopyFailed {
        /// The source's status (`missing`, `changed`).
        reason: String,
        /// The source's path, or its relative path.
        path: String,
    },
    /// A file-system error (Python lets the `OSError` through; here it carries the path).
    Io(String, std::io::Error),
}

impl fmt::Display for ProjectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProjectError::Format(m)
            | ProjectError::Invalid(m)
            | ProjectError::Exists(m)
            | ProjectError::NoDocument(m) => f.write_str(m),
            ProjectError::NeedsRelink { track, reason } => {
                write!(f, "track {track}: source {reason}; relink it before Save As")
            }
            ProjectError::CopyFailed { reason, path } => {
                write!(f, "cannot copy source ({reason}): {path}")
            }
            ProjectError::Io(what, e) => write!(f, "{what}: {e}"),
        }
    }
}

impl std::error::Error for ProjectError {}

impl ProjectError {
    fn invalid(msg: impl Into<String>) -> ProjectError {
        ProjectError::Invalid(msg.into())
    }

    /// True for the newer-format error (`isinstance(e, ProjectFormatError)`).
    #[must_use]
    pub fn is_format(&self) -> bool {
        matches!(self, ProjectError::Format(_))
    }
}

pub(crate) fn io_err(what: impl Into<String>) -> impl FnOnce(std::io::Error) -> ProjectError {
    let what = what.into();
    move |e| ProjectError::Io(what, e)
}

/// An open project: its folder and its document (`project.Project`).
///
/// `name()` is the display name; the folder name is the file-system identity. The document
/// normally carries the folder's name; a project loaded from a folder whose only document
/// has another name keeps that document path (`doc_path`) and saves back to it.
#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    /// Absolute project folder.
    pub dir: PathBuf,
    /// The document.
    pub doc: Document,
    /// The document path when it is not `<dir>/<dir name>.drumspike`.
    pub doc_path: Option<PathBuf>,
}

impl Project {
    /// A project in `dir` with `doc`; `doc_path` as `load_project` found it.
    pub fn new(dir: &Path, doc: Document, doc_path: Option<&Path>) -> Project {
        Project {
            dir: abspath(dir),
            doc,
            doc_path: doc_path.map(abspath),
        }
    }

    /// `doc.name` or the folder's name.
    #[must_use]
    pub fn name(&self) -> String {
        match self.doc.name.as_deref() {
            Some(n) if !n.is_empty() => n.to_string(),
            _ => basename(&self.dir).to_string(),
        }
    }

    /// The document path (`doc_path` or the folder's own name).
    #[must_use]
    pub fn path(&self) -> PathBuf {
        self.doc_path.clone().unwrap_or_else(|| document_path(&self.dir))
    }

    /// `<dir>/<which>`.
    #[must_use]
    pub fn subdir(&self, which: &str) -> PathBuf {
        self.dir.join(which)
    }
}

// ---------- paths (os.path semantics) ----------

/// `os.path.normpath`: collapse `.`/`..` lexically and repeated separators; no symlink lookup.
#[must_use]
pub fn normpath(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    let mut depth = 0usize;
    for c in p.components() {
        match c {
            Component::RootDir | Component::Prefix(_) => out.push(c.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if depth > 0 {
                    out.pop();
                    depth -= 1;
                } else if !out.has_root() {
                    out.push("..");
                }
            }
            Component::Normal(s) => {
                out.push(s);
                depth += 1;
            }
        }
    }
    if out.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        out
    }
}

/// `os.path.abspath`: the current directory joined and normalised, no symlink lookup.
#[must_use]
pub fn abspath(p: &Path) -> PathBuf {
    if p.is_absolute() {
        normpath(p)
    } else {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        normpath(&cwd.join(p))
    }
}

/// `os.path.basename`: the last component as a string (empty for a root).
#[must_use]
pub fn basename(p: &Path) -> &str {
    p.file_name().and_then(|s| s.to_str()).unwrap_or("")
}

/// `os.path.relpath(path, start)` with posix separators: both made absolute, the common
/// prefix dropped, `..` for every remaining component of `start`.
pub fn relpath(path: &Path, start: &Path) -> String {
    let a: Vec<String> = abspath(path)
        .components()
        .skip(1)
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let b: Vec<String> = abspath(start)
        .components()
        .skip(1)
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let common = a.iter().zip(b.iter()).take_while(|(x, y)| x == y).count();
    let mut parts: Vec<&str> = vec![".."; b.len() - common];
    parts.extend(a[common..].iter().map(std::string::String::as_str));
    if parts.is_empty() {
        ".".to_string()
    } else {
        parts.join("/")
    }
}

/// `project.document_path`: `<project_dir>/<basename of the absolute folder>.drumspike`.
#[must_use]
pub fn document_path(project_dir: &Path) -> PathBuf {
    let abs = abspath(project_dir);
    project_dir.join(format!("{}{}", basename(&abs), DOC_EXT))
}

/// Create the four sub-folders (`project.ensure_layout`).
///
/// # Errors
///
/// [`ProjectError::Io`] when a sub-folder cannot be created.
pub fn ensure_layout(project_dir: &Path) -> Result<(), ProjectError> {
    for d in SUBDIRS {
        let p = project_dir.join(d);
        std::fs::create_dir_all(&p).map_err(io_err(format!("mkdir {}", p.display())))?;
    }
    Ok(())
}

// ---------- names ----------

/// A Python `repr()` of a string for the error texts (single quotes, escaped control characters).
fn py_repr(s: &str) -> String {
    let mut out = String::from("'");
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '\0' => out.push_str("\\x00"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out.push('\'');
    out
}

/// `project.check_name`: a new project's name becomes its folder name as typed; no path
/// separators, no NUL, no `:`; returns the stripped name.
///
/// # Errors
///
/// [`ProjectError::Invalid`] when the stripped name is empty, `.` or `..`, or contains `/`, `\`,
/// `:` or NUL.
pub fn check_name(name: &str) -> Result<String, ProjectError> {
    let n = name.trim();
    if n.is_empty()
        || n == "."
        || n == ".."
        || n.contains('/')
        || n.contains('\\')
        || n.contains(':')
        || n.contains('\0')
    {
        return Err(ProjectError::invalid(format!("bad project name: {}", py_repr(n))));
    }
    Ok(n.to_string())
}

/// `project.check_track_name`: a track name is a folder name under `Analysis/` and a file
/// stem under `Backups/` and `Exports/`; no path separators, no NUL, not `.`/`..`, not blank.
///
/// # Errors
///
/// [`ProjectError::Invalid`] when the name is blank, `.` or `..`, or contains `/`, `\` or NUL.
pub fn check_track_name(name: &str) -> Result<&str, ProjectError> {
    if name.trim().is_empty()
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
        || name.contains('\0')
    {
        return Err(ProjectError::invalid(format!("bad track name: {}", py_repr(name))));
    }
    Ok(name)
}

// ---------- load ----------

/// `project.find_document`: folder or document path -> document path (the folder's own
/// name first, else the only `.drumspike` file in the folder).
///
/// # Errors
///
/// [`ProjectError::NoDocument`] when the path is neither a file nor a folder with exactly one
/// document (and none under the folder's own name); [`ProjectError::Io`] when the folder cannot
/// be listed.
pub fn find_document(path: &Path) -> Result<PathBuf, ProjectError> {
    let path = abspath(path);
    if path.is_file() {
        return Ok(path);
    }
    if path.is_dir() {
        let own = document_path(&path);
        if own.is_file() {
            return Ok(own);
        }
        let mut docs: Vec<String> = std::fs::read_dir(&path)
            .map_err(io_err(format!("list {}", path.display())))?
            .filter_map(std::result::Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(DOC_EXT))
            .collect();
        docs.sort();
        if docs.len() == 1 {
            return Ok(path.join(&docs[0]));
        }
    }
    Err(ProjectError::NoDocument(format!(
        "no {DOC_EXT} document at {}",
        path.display()
    )))
}

/// Read a JSON file as text and parse it. Bytes that are not UTF-8 and text that is not
/// JSON (a byte order mark included) are `"<path> is not valid JSON: ..."`, as Python's
/// `open` (UTF-8) plus `json.load` raise a `ValueError` for both (finding m2).
fn read_json(path: &Path) -> Result<(Value, String), ProjectError> {
    let bytes = std::fs::read(path).map_err(io_err(format!("read {}", path.display())))?;
    let not_json = |e: &dyn fmt::Display| ProjectError::invalid(format!("{} is not valid JSON: {e}", path.display()));
    let text = String::from_utf8(bytes).map_err(|e| not_json(&e))?;
    let value = serde_json::from_str(&text).map_err(|e| not_json(&e))?;
    Ok((value, text))
}

/// `project.load_project`: open a project from its folder or its document; older formats
/// are migrated in memory, the file on disk is not touched.
///
/// # Errors
///
/// [`ProjectError::NoDocument`] or [`ProjectError::Io`] when no document is found or it cannot
/// be read; [`ProjectError::Invalid`] when it is not valid UTF-8 JSON, not an object, or has a
/// bad `format` field or track name; [`ProjectError::Format`] when it was written by a newer app.
pub fn load_project(path: &Path) -> Result<Project, ProjectError> {
    let doc_path = find_document(path)?;
    let (value, text) = read_json(&doc_path)?;
    if !value.is_object() {
        return Err(ProjectError::invalid(format!(
            "{} is not a project document",
            doc_path.display()
        )));
    }
    let doc = migrate_forward_from_text(value, &text)?;
    let dir = doc_path.parent().map(Path::to_path_buf).unwrap_or_default();
    Ok(Project {
        dir,
        doc,
        doc_path: Some(doc_path),
    })
}

/// `project.new_project`: create `<dir>/<name>/` with the layout and an empty document.
///
/// # Errors
///
/// [`ProjectError::Invalid`] for a bad `name`; [`ProjectError::Exists`] when the folder already
/// holds a document; [`ProjectError::Io`] when the layout or the first save fails.
pub fn new_project(dir: &Path, name: &str) -> Result<Project, ProjectError> {
    let name = check_name(name)?;
    let pdir = abspath(dir).join(&name);
    if document_path(&pdir).exists() {
        return Err(ProjectError::Exists(format!(
            "project already exists: {}",
            pdir.display()
        )));
    }
    ensure_layout(&pdir)?;
    let mut p = Project::new(&pdir, empty_document(&name, DEFAULT_MIX_MODE), None);
    save_project(&mut p)?;
    Ok(p)
}

// ---------- format versions ----------

/// A Python `repr()` of the JSON value in the `format` field.
fn format_repr(v: &Value) -> String {
    match v {
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::Null => "None".into(),
        Value::String(s) => py_repr(s),
        other => other.to_string(),
    }
}

/// `project.migrate_forward`: upgrade a document of any older format to [`FORMAT`] in
/// memory and check the track names. Newer formats give [`ProjectError::Format`].
///
/// # Errors
///
/// [`ProjectError::Format`] for a format newer than [`FORMAT`]; [`ProjectError::Invalid`] for a
/// bad or negative `format` field, a malformed document, or a bad track name.
pub fn migrate_forward(value: Value) -> Result<Document, ProjectError> {
    migrate_with(value, None)
}

/// [`migrate_forward`] for a document parsed from `text`. `serde_json` reads an integer
/// beyond 64 bits as a float, Python's json as an int: the literal in `text` decides, so
/// `"format": 100000000000000000000000` is the newer-format error, as in Python, and
/// `"format": 1e23` stays a bad format field (review round 3, finding m3).
///
/// # Errors
///
/// The same as [`migrate_forward`].
pub fn migrate_forward_from_text(value: Value, text: &str) -> Result<Document, ProjectError> {
    let big = match value.get("format") {
        Some(Value::Number(n)) if n.as_f64().is_some_and(|f| f.abs() >= 2f64.powi(63)) => big_int_format(text),
        _ => None,
    };
    migrate_with(value, big)
}

/// The literal of the top-level `format` field of `text` when it is an integer (digits
/// with an optional minus sign), else `None`.
fn big_int_format(text: &str) -> Option<String> {
    let fields: indexmap::IndexMap<String, &serde_json::value::RawValue> = serde_json::from_str(text).ok()?;
    let lit = fields.get("format")?.get();
    let digits = lit.strip_prefix('-').unwrap_or(lit);
    (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())).then(|| lit.to_string())
}

fn migrate_with(mut value: Value, big: Option<String>) -> Result<Document, ProjectError> {
    let fmt = match value.get("format") {
        None => 0,
        Some(Value::Number(n)) if n.is_i64() => n.as_i64().expect("i64"),
        Some(Value::Number(n)) if n.is_u64() || big.as_ref().is_some_and(|l| !l.starts_with('-')) => {
            let lit = big.unwrap_or_else(|| n.to_string());
            return Err(ProjectError::Format(format!(
                "project format {lit} is newer than this app (format {FORMAT}); update the app to open it"
            )));
        }
        Some(Value::Number(_)) if big.is_some() => {
            return Err(ProjectError::invalid(format!(
                "bad format field: {}",
                big.expect("literal")
            )));
        }
        Some(other) => {
            return Err(ProjectError::invalid(format!(
                "bad format field: {}",
                format_repr(other)
            )));
        }
    };
    if fmt < 0 {
        return Err(ProjectError::invalid(format!("bad format field: {fmt}")));
    }
    if fmt > FORMAT {
        return Err(ProjectError::Format(format!(
            "project format {fmt} is newer than this app (format {FORMAT}); update the app to open it"
        )));
    }
    if fmt == 0 {
        value = migrate_0_to_1(value)?;
    }
    match value.get("tracks") {
        Some(Value::Object(tracks)) => {
            // on the raw names, like Python: a track that is not an object still has a name
            for name in tracks.keys() {
                check_track_name(name)?;
            }
        }
        Some(Value::Null) | None => {}
        Some(_) => return Err(ProjectError::invalid("tracks must be an object")),
    }
    // Only a missing or non-integer `format` fails here, and it was checked above: every
    // other value the typed fields cannot hold stays in an `extra` map (document.rs).
    serde_json::from_value(value)
        .map_err(|e| ProjectError::invalid(format!("document does not match format {FORMAT}: {e}")))
}

/// Python truthiness of a JSON value (`doc.get(k) or default`).
fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) | Some(Value::Bool(false)) => false,
        Some(Value::Number(n)) => n.as_f64() != Some(0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
        Some(Value::Bool(true)) => true,
    }
}

/// Python `str()` of a JSON value, for the `name` of a format-0 document.
fn py_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        other => other.to_string(),
    }
}

/// `project._migrate_0_to_1`. Format 0 is the server session: `meta.json` plus one
/// `events.json` per track, read as one object with an `events` key (track name ->
/// events.json content, `{"events": [...]}` or the bare list). Imported sessions were
/// analysed through ffmpeg's `-ac 1` sum, so every track gets `mix_mode: "ffmpeg"`.
fn migrate_0_to_1(doc: Value) -> Result<Value, ProjectError> {
    let events: Map<String, Value> = match doc.get("events") {
        v if !truthy(v) => Map::new(),
        Some(Value::Object(m)) => m.clone(),
        _ => return Err(ProjectError::invalid("format 0: events must be an object")),
    };
    let tracks: Map<String, Value> = match doc.get("tracks") {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(m)) => m.clone(),
        Some(_) => return Err(ProjectError::invalid("format 0: tracks must be an object")),
    };
    let name = match doc.get("name") {
        v if truthy(v) => py_str(v.expect("truthy")),
        _ => "Imported project".to_string(),
    };
    let created = match doc.get("created") {
        v if truthy(v) => v.expect("truthy").clone(),
        _ => Value::Number(Number::from_f64(now()).expect("finite time")),
    };
    let stored_order: Vec<&str> = match doc.get("order") {
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    };
    let mut order: Vec<Value> = stored_order
        .iter()
        .filter(|n| tracks.contains_key(**n))
        .map(|n| Value::from(*n))
        .collect();
    for n in tracks.keys() {
        if !stored_order.contains(&n.as_str()) {
            order.push(Value::from(n.as_str()));
        }
    }
    let mut out_tracks = Map::new();
    for (tname, tr) in &tracks {
        let mut t = match tr {
            Value::Object(m) => m.clone(),
            _ => {
                return Err(ProjectError::invalid(format!(
                    "format 0: track {tname} must be an object"
                )));
            }
        };
        t.entry("mix_mode").or_insert_with(|| Value::from("ffmpeg"));
        t.entry("source").or_insert(Value::Null);
        let ev = match events.get(tname) {
            None => Vec::new(),
            Some(Value::Object(m)) => match m.get("events") {
                v if !truthy(v) => Vec::new(),
                Some(Value::Array(a)) => a.clone(),
                _ => {
                    return Err(ProjectError::invalid(format!(
                        "format 0: events of {tname} must be a list"
                    )));
                }
            },
            Some(Value::Array(a)) => a.clone(),
            Some(_) => {
                return Err(ProjectError::invalid(format!(
                    "format 0: events of {tname} must be a list"
                )));
            }
        };
        t.insert("events".into(), Value::Array(ev));
        out_tracks.insert(tname.clone(), Value::Object(t));
    }
    let mut out = Map::new();
    out.insert("format".into(), Value::from(1));
    out.insert("name".into(), Value::from(name));
    out.insert("created".into(), created);
    out.insert("modified".into(), Value::Null);
    out.insert("settings".into(), serde_json::json!({"mix_mode": "ffmpeg"}));
    out.insert("tracks".into(), Value::Object(out_tracks));
    out.insert("view".into(), Value::Object(Map::new()));
    out.insert("order".into(), Value::Array(order));
    Ok(Value::Object(out))
}

/// `project.legacy_session_document`: read `sessions/<id>/` (`meta.json` +
/// `tracks/<name>/{events.json, source.wav}`) as a format-0 document value. Sources are
/// referenced records that point at the session's `source.wav`; nothing in the session
/// folder is changed.
///
/// # Errors
///
/// [`ProjectError::Invalid`] when `meta.json` is missing, not JSON, not an object, has `tracks`
/// that is not an object, or names a bad track; [`ProjectError::Io`] when a file cannot be read
/// or a source cannot be hashed.
///
/// # Panics
///
/// Panics through `expect("a record serialises")` if a source record cannot be converted to a
/// JSON value, which a plain record never fails to do.
pub fn legacy_session_document(session_dir: &Path) -> Result<Value, ProjectError> {
    let mp = session_dir.join("meta.json");
    if !mp.is_file() {
        return Err(ProjectError::invalid(format!(
            "no meta.json in {}",
            session_dir.display()
        )));
    }
    let (mut doc, _) = read_json(&mp)?;
    let Some(root) = doc.as_object_mut() else {
        return Err(ProjectError::invalid(format!(
            "{} is not a session document",
            mp.display()
        )));
    };
    root.insert("format".into(), Value::from(0));
    let mut events = Map::new();
    let names: Vec<String> = match root.get("tracks") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Object(m)) => m.keys().cloned().collect(),
        Some(_) => return Err(ProjectError::invalid("format 0: tracks must be an object")),
    };
    for name in names {
        check_track_name(&name)?; // before any path is built from it
        let td = session_dir.join("tracks").join(&name);
        let ep = td.join("events.json");
        if ep.is_file() {
            let (v, _) = read_json(&ep)?;
            events.insert(name.clone(), v);
        }
        let wav = td.join("source.wav");
        if wav.is_file() {
            let rec = build_source(&wav, session_dir, "referenced")?;
            if let Some(Value::Object(tr)) = root.get_mut("tracks").and_then(|t| t.get_mut(&name)) {
                tr.insert("source".into(), serde_json::to_value(rec).expect("a record serialises"));
            }
        }
    }
    root.insert("events".into(), Value::Object(events));
    Ok(doc)
}

// ---------- tracks ----------

/// `project.add_track`: register a track with its source record (`None` while no file is
/// attached) and append it to `order`; the caller sets `sr`, `n_samples`, `duration`,
/// `color` and `params` on the returned track.
///
/// # Errors
///
/// [`ProjectError::Invalid`] when a track called `name` exists or `mix_mode` is not one of the
/// mix modes.
///
/// # Panics
///
/// Panics through `expect("just inserted")` only if the track inserted a line earlier is
/// missing, which cannot happen.
pub fn add_track<'a>(
    project: &'a mut Project,
    name: &str,
    source: Option<Source>,
    role: &str,
    note: i64,
    mix_mode: Option<&str>,
) -> Result<&'a mut Track, ProjectError> {
    if project.doc.track(name).is_some() {
        return Err(ProjectError::invalid(format!("track exists: {name}")));
    }
    let mix_mode = mix_mode.unwrap_or_else(|| project.doc.default_mix_mode()).to_string();
    if !MIX_MODES.contains(&mix_mode.as_str()) {
        return Err(ProjectError::invalid(format!("bad mix_mode: {}", py_repr(&mix_mode))));
    }
    let tr = Track {
        role: Some(role.to_string()),
        note: Some(note),
        mix_mode: Some(mix_mode),
        source: Some(source),
        events: Some(Vec::new()),
        ..Default::default()
    };
    project.doc.order.get_or_insert_with(Vec::new).push(name.to_string());
    let tracks = project.doc.tracks_mut();
    tracks.insert(name.to_string(), tr);
    Ok(tracks.get_mut(name).expect("just inserted"))
}

#[cfg(test)]
pub(crate) mod tests;
