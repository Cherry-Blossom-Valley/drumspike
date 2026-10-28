//! The registry of open projects (`server._open`, `server.OpenProject`): an id per project,
//! the dirty flag, and the resolved source of every track.
//!
//! The document stays the typed [`drumspike_core::Document`]. The JSON the page sees is made
//! from it with `serde_json::to_value`, so every key the Python layer wrote, known or not,
//! comes back as it was.

use std::collections::hash_map::RandomState;
use std::collections::HashMap;
use std::hash::{BuildHasher, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Instant, SystemTime};

use drumspike_core::project::{resolve_source, Source, Track};
use drumspike_core::{Document, Project, SourceStatus};
use serde_json::{json, Map, Value};

use crate::error::AppError;

/// Lock a mutex; a panic in another thread does not stop the app (the data stays usable).
/// With the feature `trace` (P3-15c) a wait for the lock is a span `lock_wait` that names the
/// caller's file and line.
#[cfg_attr(feature = "trace", track_caller)]
pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    #[cfg(feature = "trace")]
    {
        match m.try_lock() {
            Ok(g) => g,
            Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                let _wait = crate::trace::lock_wait(std::panic::Location::caller());
                m.lock().unwrap_or_else(|p| p.into_inner())
            }
        }
    }
    #[cfg(not(feature = "trace"))]
    {
        m.lock().unwrap_or_else(|p| p.into_inner())
    }
}

/// A function that sends an event to the page (`project-dirty`): name and JSON payload. The
/// Tauri crate sets one that calls `AppHandle::emit`; the test bridge queues the events.
pub type Emitter = Arc<dyn Fn(&str, Value) + Send + Sync>;

/// The emitter, set once after the service exists. Every open project holds a clone of the slot.
#[derive(Clone, Default)]
pub struct EmitterSlot(Arc<Mutex<Option<Emitter>>>);

impl EmitterSlot {
    /// Set the emitter (replaces an earlier one).
    pub fn set(&self, e: Emitter) {
        *lock(&self.0) = Some(e);
    }

    /// The emitter, when one is set.
    pub fn get(&self) -> Option<Emitter> {
        lock(&self.0).clone()
    }
}

impl std::fmt::Debug for EmitterSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "EmitterSlot({})", if self.get().is_some() { "set" } else { "empty" })
    }
}

/// #121: the start of the 409 text of a save that found another document on disk. The page looks for it to offer
/// Reload from disk and Save anyway.
pub const CHANGED_ON_DISK: &str = "the project file changed on disk";

/// #121: size and modification time of the document file, as this process read or wrote it last.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskStamp {
    /// Bytes.
    pub len: u64,
    /// `None` where the file system has no modification time.
    pub modified: Option<SystemTime>,
}

impl DiskStamp {
    /// The stamp of the file at `path`; `None` when there is no file.
    pub fn of(path: &Path) -> Option<DiskStamp> {
        let m = std::fs::metadata(path).ok().filter(std::fs::Metadata::is_file)?;
        Some(DiskStamp {
            len: m.len(),
            modified: m.modified().ok(),
        })
    }
}

/// An open project: the document plus the runtime state the commands need.
#[derive(Debug)]
pub struct OpenProject {
    /// The id the page uses (8 hex digits, like the server's).
    pub sid: String,
    /// Folder and document.
    pub project: Project,
    /// True after a change that is not saved; `project_save` and `project_save_as` clear it.
    pub dirty: bool,
    /// When the project became dirty (P3-4a, autosave); `None` while it is clean.
    pub dirty_since: Option<Instant>,
    /// P4-40a: the document as it was when the project was opened or saved last (what the file holds). `clean_if_saved`
    /// compares the open document with it: an undo back to the saved state clears the flag.
    pub saved_doc: Document,
    /// When the autosave wrote last while the project was dirty (P3-4a).
    pub last_autosave: Option<Instant>,
    /// Where the `project-dirty` events go (P3-4a).
    pub emitter: EmitterSlot,
    /// Track name to the resolved source file (`None` when missing).
    pub paths: HashMap<String, Option<PathBuf>>,
    /// Track name to `ok`, `changed` or `missing`.
    pub status: HashMap<String, SourceStatus>,
    /// P4-5: opened read-only: no lock is held, and a command that changes the project is refused.
    pub read_only: bool,
    /// P4-5: the cross-process lock of the folder (`lock::ProjectLock`), held while the project is open.
    pub lock: Option<crate::lock::ProjectLock>,
    /// P4-5: `held` or `unavailable: <why>` when this process locks (`Service::set_lock_client`), else `None`; it is the
    /// `lock` field of the open answer.
    pub lock_note: Option<String>,
    /// #121: the document file as this process read or wrote it last; `project_save` refuses to write over another one.
    pub disk: Option<DiskStamp>,
    /// #121: the outside stamp the autosave reported already (one `project-stale` event per outside change).
    pub stale_noted: Option<DiskStamp>,
}

impl OpenProject {
    /// A project with every source resolved.
    pub fn new(sid: String, project: Project) -> OpenProject {
        let saved_doc = project.doc.clone();
        let disk = DiskStamp::of(&project.path());
        let mut op = OpenProject {
            sid,
            project,
            dirty: false,
            dirty_since: None,
            saved_doc,
            last_autosave: None,
            emitter: EmitterSlot::default(),
            paths: HashMap::new(),
            status: HashMap::new(),
            read_only: false,
            lock: None,
            lock_note: None,
            disk,
            stale_noted: None,
        };
        op.resolve();
        op
    }

    /// #121: the stamp of the document file now, when it is not the one this process read or wrote last. A missing
    /// file is no change (a save writes it again and loses nothing).
    pub fn disk_change(&self) -> Option<DiskStamp> {
        DiskStamp::of(&self.project.path()).filter(|now| Some(*now) != self.disk)
    }

    /// #121: 409 when the document on disk changed since this process read or wrote it ([`CHANGED_ON_DISK`]).
    pub fn check_disk(&self) -> Result<(), AppError> {
        match self.disk_change() {
            None => Ok(()),
            Some(_) => Err(AppError::conflict(format!(
                "{CHANGED_ON_DISK} since Drum Spike read it: {}. Reload from disk, or Save anyway",
                self.project.path().display()
            ))),
        }
    }

    /// #121: the document file is the one this process just wrote.
    pub fn restamp(&mut self) {
        self.disk = DiskStamp::of(&self.project.path());
        self.stale_noted = None;
    }

    /// #121: send `project-stale` `{sid, message}` once per outside change of the document file (the autosave looks).
    /// Answers true when it sent one.
    pub fn note_stale(&mut self) -> bool {
        let Some(now) = self.disk_change() else { return false };
        if self.stale_noted == Some(now) {
            return false;
        }
        self.stale_noted = Some(now);
        let message = self.check_disk().err().map(|e| e.message).unwrap_or_default();
        if let Some(f) = self.emitter.get() {
            f("project-stale", json!({"sid": self.sid, "message": message}));
        }
        true
    }

    /// P4-5: Save As took the lock of the new folder before it wrote anything (`Service::lock_new_folder`); now the
    /// project holds it instead of the old one (the old one is released) and is a writable project, also when it was
    /// opened read-only. `lock` is `None` when the process does not lock or no lock was available.
    pub fn switch_lock(&mut self, lock: Option<crate::lock::Lock>) {
        self.read_only = false;
        self.lock_note = lock.as_ref().map(crate::lock::Lock::note);
        self.lock = lock.and_then(crate::lock::Lock::into_held);
    }

    /// `OpenProject.resolve`: find every track's source again (absolute path, relative
    /// path, `Audio Files/<name>`).
    pub fn resolve(&mut self) {
        let _span = crate::trace::stage("resolve");
        let mut paths = HashMap::new();
        let mut status = HashMap::new();
        for (name, tr) in self.project.doc.tracks() {
            let (p, s) = resolve_source(tr.source_ref(), &self.project.dir);
            paths.insert(name.clone(), p);
            status.insert(name.clone(), s);
        }
        self.paths = paths;
        self.status = status;
    }

    /// The folder and the source record of every track (what [`OpenProject::resolve`] reads).
    pub fn records(&self) -> (PathBuf, Vec<(String, Option<Source>)>) {
        let recs = self
            .project
            .doc
            .tracks()
            .map(|(n, tr)| (n.clone(), tr.source_ref().cloned()))
            .collect();
        (self.project.dir.clone(), recs)
    }

    /// True unless the source was found with the recorded size and head hash.
    pub fn offline(&self, name: &str) -> bool {
        self.status.get(name) != Some(&SourceStatus::Ok)
    }

    /// The status text of a track (`missing` for an unknown name, like the server).
    pub fn status_str(&self, name: &str) -> &'static str {
        self.status.get(name).map_or("missing", |s| s.as_str())
    }

    /// One track; 400 `unknown track` like `OpenProject.track`.
    pub fn track(&self, name: &str) -> Result<&Track, AppError> {
        self.project
            .doc
            .track(name)
            .ok_or_else(|| AppError::bad("unknown track"))
    }

    /// One track, mutable.
    pub fn track_mut(&mut self, name: &str) -> Result<&mut Track, AppError> {
        self.project
            .doc
            .track_mut(name)
            .ok_or_else(|| AppError::bad("unknown track"))
    }

    /// The path of the track's audio; 409 when the track is offline.
    pub fn source(&self, name: &str) -> Result<PathBuf, AppError> {
        self.track(name)?;
        if self.offline(name) {
            return Err(AppError::conflict(format!(
                "track {name} is offline ({}): relink it first",
                self.status_str(name)
            )));
        }
        self.paths
            .get(name)
            .cloned()
            .flatten()
            .ok_or_else(|| AppError::conflict(format!("track {name} is offline (missing): relink it first")))
    }

    /// `mark_dirty`: sets the flag; the change from clean to dirty starts the autosave
    /// clock and sends `project-dirty`.
    pub fn mark_dirty(&mut self) {
        if !self.dirty {
            self.dirty = true;
            self.dirty_since = Some(Instant::now());
            self.last_autosave = None;
            self.emit_dirty();
        }
    }

    /// `mark_clean`: after a save. A change from dirty to clean sends `project-dirty`.
    pub fn mark_clean(&mut self) {
        self.saved_doc = self.project.doc.clone();
        if self.dirty {
            self.dirty = false;
            self.dirty_since = None;
            self.last_autosave = None;
            self.emit_dirty();
        }
    }

    /// P4-40a: a dirty project whose document equals the saved one is clean again (an undo back to the saved state).
    /// Sends `project-dirty` like a save. Answers the flag after the call.
    pub fn clean_if_saved(&mut self) -> bool {
        if self.dirty && self.project.doc == self.saved_doc {
            self.dirty = false;
            self.dirty_since = None;
            self.last_autosave = None;
            self.emit_dirty();
        }
        self.dirty
    }

    /// Send `track-layer` `{sid, name}` after the layer or the sampler calibration of a track changed (P4-33): a page that is
    /// open shows the change of another client (the agent commands) without a reload.
    pub fn emit_layer(&self, name: &str) {
        if let Some(f) = self.emitter.get() {
            f("track-layer", json!({"sid": self.sid, "name": name}));
        }
    }

    /// Send `project-dirty` `{sid, dirty}` when an emitter is set.
    fn emit_dirty(&self) {
        if let Some(f) = self.emitter.get() {
            f("project-dirty", json!({"sid": self.sid, "dirty": self.dirty}));
        }
    }

    /// The track record as a JSON object (every key the document holds).
    pub fn track_object(&self, name: &str) -> Result<Map<String, Value>, AppError> {
        let tr = self.track(name)?;
        match serde_json::to_value(tr) {
            Ok(Value::Object(m)) => Ok(m),
            _ => Err(AppError::internal("a track does not serialise as an object")),
        }
    }

    /// `OpenProject.public_track`: the record without `events` and `source`, plus
    /// `source_mode`, `status`, `offline` and `n_events`. P3-15c: the events are not serialised
    /// (see [`track_object_light`]); `tracks_get` holds the project lock a short time only.
    pub fn public_track(&self, name: &str) -> Result<Value, AppError> {
        let tr = self.track(name)?;
        let mut m = track_object_light(tr)?;
        m.insert(
            "source_mode".into(),
            tr.source_ref()
                .and_then(|s| s.mode.clone())
                .map_or(Value::Null, Value::from),
        );
        m.insert("status".into(), Value::from(self.status_str(name)));
        m.insert("offline".into(), Value::from(self.offline(name)));
        // P4-7a fix 6: the state of the track's layer file (the relink bar lists a layer file that is not `ok`)
        if let Some(l) = crate::layer::layer_of(&tr.extra) {
            let (_, st) = resolve_source(Some(&l.source), &self.project.dir);
            m.insert("layer_status".into(), Value::from(st.as_str()));
        }
        m.insert("n_events".into(), Value::from(self.n_events(name)));
        Ok(Value::Object(m))
    }

    /// `len(tr.get("events") or [])`: the typed list, or the untyped one a document kept.
    pub fn n_events(&self, name: &str) -> usize {
        let Ok(tr) = self.track(name) else { return 0 };
        match tr.events {
            Some(ref ev) => ev.len(),
            None => tr.extra.get("events").and_then(Value::as_array).map_or(0, Vec::len),
        }
    }

    /// `OpenProject.summary`: what `project_open` and `project_get` answer. P3-15c: the
    /// document keys come from a copy without tracks (see [`doc_head_value`]).
    pub fn summary(&self) -> Value {
        let doc = &self.project.doc;
        let dv = doc_head_value(doc);
        let field = |k: &str| dv.get(k).cloned().unwrap_or(Value::Null);
        let mut tracks = Map::new();
        for (n, tr) in doc.tracks() {
            let rec = tr.source_ref();
            let source = rec.map_or(
                Value::Null,
                |s| json!({"mode": s.mode, "path": s.path, "relative": s.relative}),
            );
            tracks.insert(
                n.clone(),
                json!({
                    "role": tr.role,
                    "note": tr.note,
                    "mix_mode": tr.mix_mode,
                    "n_events": self.n_events(n),
                    "status": self.status_str(n),
                    "offline": self.offline(n),
                    "path": self.paths.get(n).cloned().flatten().map(|p| p.display().to_string()),
                    "source": source,
                }),
            );
        }
        let mut out = json!({
            "sid": self.sid,
            "name": self.project.name(),
            "folder": self.project.dir.display().to_string(),
            "document": self.project.path().display().to_string(),
            "dirty": self.dirty,
            "format": field("format"),
            "created": field("created"),
            "modified": field("modified"),
            "settings": dv.get("settings").cloned().unwrap_or_else(|| json!({})),
            "order": doc.track_order(),
            "tracks": Value::Object(tracks),
        });
        if self.read_only {
            out["read_only"] = Value::Bool(true); // P4-5: only a read-only project has the key
        }
        if let Some(note) = &self.lock_note {
            out["lock"] = Value::String(note.clone()); // P4-5: only a process that locks has the key
        }
        out
    }

    /// The modification time the project list sorts by: `modified`, else `created`, else 0.
    pub fn mtime(&self) -> f64 {
        let doc = &self.project.doc;
        match doc.modified {
            Some(Some(m)) if m != 0.0 => m,
            _ => doc.created.as_ref().and_then(|n| n.as_f64()).unwrap_or(0.0),
        }
    }
}

/// A shared, lockable open project.
pub type Shared = Arc<Mutex<OpenProject>>;

/// The JSON object of a track without `events` and `source`, in the key order of the full
/// record (`serde_json::to_value` of the track minus those two keys), without serialising or
/// copying the events (P3-15c: 6,000 hits made `tracks_get` hold the project lock for 55 ms
/// in the debug app).
pub fn track_object_light(tr: &Track) -> Result<Map<String, Value>, AppError> {
    // every field is named: a new field of `Track` must be decided here
    let Track {
        role,
        note,
        sr,
        n_samples,
        duration,
        color,
        params,
        mix_mode,
        source: _,
        events: _,
        volume_db,
        pan,
        follows,
        role_source,
        role_match,
        extra,
    } = tr;
    let light = Track {
        role: role.clone(),
        note: *note,
        sr: *sr,
        n_samples: *n_samples,
        duration: *duration,
        color: color.clone(),
        params: params.clone(),
        mix_mode: mix_mode.clone(),
        source: None,
        events: None,
        volume_db: *volume_db,
        pan: *pan,
        follows: follows.clone(),
        role_source: role_source.clone(),
        role_match: role_match.clone(),
        extra: extra
            .iter()
            .filter(|(k, _)| *k != "events" && *k != "source")
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
    };
    match serde_json::to_value(&light) {
        Ok(Value::Object(m)) => Ok(m),
        _ => Err(AppError::internal("a track does not serialise as an object")),
    }
}

/// The document as JSON without `tracks`, `order` and `view`: the keys `format`, `name`,
/// `created`, `modified`, `settings` and the unknown ones, as the full document writes them.
pub fn doc_head_value(doc: &Document) -> Value {
    let Document {
        format,
        name,
        created,
        modified,
        settings,
        order: _,
        tracks: _,
        view: _,
        extra,
    } = doc;
    let light = Document {
        format: *format,
        name: name.clone(),
        created: created.clone(),
        modified: *modified,
        settings: settings.clone(),
        order: None,
        tracks: None,
        view: None,
        extra: extra.clone(),
    };
    serde_json::to_value(&light).unwrap_or(Value::Null)
}

/// [`OpenProject::resolve`] without holding the project lock while the files are read
/// (P3-15c): the records are copied under the lock, resolved outside it, and stored when no
/// record and not the folder changed in the meantime; else the project resolves again under
/// the lock (the head-hash cache of the core makes that second pass cheap).
pub fn resolve_shared(op: &Shared) {
    let (dir, records) = lock(op).records();
    let resolved: Vec<(Option<PathBuf>, SourceStatus)> = {
        let _span = crate::trace::stage("resolve:unlocked");
        records.iter().map(|(_, r)| resolve_source(r.as_ref(), &dir)).collect()
    };
    let mut g = lock(op);
    if g.records() == (dir, records.clone()) {
        let mut paths = HashMap::new();
        let mut status = HashMap::new();
        for ((name, _), (p, s)) in records.into_iter().zip(resolved) {
            paths.insert(name.clone(), p);
            status.insert(name, s);
        }
        g.paths = paths;
        g.status = status;
    } else {
        g.resolve();
    }
}

/// The open projects, in the order they were opened.
#[derive(Default)]
pub struct Registry {
    open: Mutex<Vec<(String, Shared)>>,
    /// The emitter every project registered here sends its events to (P3-4a).
    pub emitter: EmitterSlot,
}

impl Registry {
    /// An empty registry.
    pub fn new() -> Registry {
        Registry::default()
    }

    /// A fresh id: 8 hex digits that no open project uses.
    fn new_sid(open: &[(String, Shared)]) -> String {
        loop {
            let mut h = RandomState::new().build_hasher();
            h.write_usize(open.len());
            let sid = format!("{:08x}", h.finish() as u32);
            if !open.iter().any(|(s, _)| *s == sid) {
                return sid;
            }
        }
    }

    /// The open projects: a snapshot of their handles (the registry lock is not held while a
    /// caller works with them). R01 (#248) uses it for the protected files of every open project.
    pub fn all_open(&self) -> Vec<Shared> {
        lock(&self.open).iter().map(|(_, op)| Arc::clone(op)).collect()
    }

    /// The open project of a folder, when there is one.
    fn find_dir(open: &[(String, Shared)], dir: &std::path::Path) -> Option<Shared> {
        open.iter()
            .find(|(_, op)| lock(op).project.dir == dir)
            .map(|(_, op)| Arc::clone(op))
    }

    /// Put a loaded project into the registry; a folder that is open already keeps its entry
    /// (`server._register`). P3-15c: the sources are resolved (file heads hashed) before the
    /// registry lock is taken, so other commands do not wait for the disk.
    pub fn register(&self, project: Project) -> Shared {
        self.register_with(project, false, None)
            .expect("registering without a lock cannot fail")
    }

    /// [`Registry::register`] for a process that locks (P4-5): with `client` and not `read_only` the folder's
    /// [`crate::lock::ProjectLock`] is taken first (409 when another process holds it); `read_only` marks the
    /// project so that every command that changes it is refused.
    pub fn register_with(&self, project: Project, read_only: bool, client: Option<&str>) -> Result<Shared, AppError> {
        if let Some(op) = Registry::find_dir(&lock(&self.open), &project.dir) {
            return Ok(op);
        }
        let mut new = OpenProject::new(String::new(), project);
        new.emitter = self.emitter.clone();
        new.read_only = read_only;
        if let (Some(client), false) = (client, read_only) {
            // another open of the same folder in this process may be about to register: wait for it, do not fail
            let mut tries = 0;
            loop {
                match crate::lock::ProjectLock::acquire(&new.project.dir, client) {
                    Ok(l) => {
                        new.lock_note = Some(l.note());
                        new.lock = l.into_held();
                        break;
                    }
                    Err(e) => {
                        let mine = crate::lock::ProjectLock::holder_of(&new.project.dir)
                            .is_some_and(|(pid, _)| pid == std::process::id());
                        if let Some(op) = Registry::find_dir(&lock(&self.open), &new.project.dir) {
                            return Ok(op);
                        }
                        tries += 1;
                        if !mine || tries > 40 {
                            return Err(e);
                        }
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                }
            }
        }
        let mut open = lock(&self.open);
        if let Some(op) = Registry::find_dir(&open, &new.project.dir) {
            return Ok(op); // opened by another call while this one resolved
        }
        let sid = Registry::new_sid(&open);
        new.sid = sid.clone();
        let op = Arc::new(Mutex::new(new));
        open.push((sid, Arc::clone(&op)));
        Ok(op)
    }

    /// `server.proj`: the project with this id; 404 `no such project`.
    pub fn get(&self, sid: &str) -> Result<Shared, AppError> {
        lock(&self.open)
            .iter()
            .find(|(s, _)| s == sid)
            .map(|(_, op)| Arc::clone(op))
            .ok_or_else(|| AppError::not_found("no such project"))
    }

    /// [`Registry::get`] for a command that changes the project (R06 #237): 409 when the session is
    /// open read-only (no write lock is held). This is the one check of the service mutation
    /// boundary: every changing command fetches its project through it, so the Tauri wrappers, the
    /// command-line tool, the MCP server and the test bridge share the policy. The documented
    /// exceptions fetch with [`Registry::get`]: `project_save_as` (the way from a read-only project
    /// to a writable copy) and a `track_repick` without `store` (the preview).
    pub fn get_writable(&self, sid: &str) -> Result<Shared, AppError> {
        let op = self.get(sid)?;
        if lock(&op).read_only {
            return Err(AppError::conflict(format!(
                "project {sid} is open read-only: the change is refused; open it for writing (project_open without read_only)"
            )));
        }
        Ok(op)
    }

    /// P3-15e: whether `op` is still in the registry (false: a close took it out). Only the
    /// list is scanned under the registry lock; no project lock is taken.
    pub fn is_open(&self, op: &Shared) -> bool {
        lock(&self.open).iter().any(|(_, o)| Arc::ptr_eq(o, op))
    }

    /// Take the project with this id out of the registry (`project_close`); 404 when unknown.
    pub fn remove(&self, sid: &str) -> Result<Shared, AppError> {
        let mut open = lock(&self.open);
        let i = open
            .iter()
            .position(|(s, _)| s == sid)
            .ok_or_else(|| AppError::not_found("no such project"))?;
        Ok(open.remove(i).1)
    }

    /// Every open project with its id, oldest open first.
    pub fn all(&self) -> Vec<(String, Shared)> {
        lock(&self.open).clone()
    }
}

#[cfg(test)]
mod tests;
