//! The project lifecycle commands (P3-4a): New, Save, Save As (referenced), Close, Rename,
//! Reorder, Note and Open Recent, as in plan section 4. They are methods of [`Service`] like
//! the commands of `commands.rs` and keep the answers of the server routes:
//!
//! | Command | Server route (route map row) |
//! |---|---|
//! | `project_new` | POST `/api/project/new` (4), POST `/api/session` (15) |
//! | `project_save` | POST `/api/project/{sid}/save` (7) |
//! | `project_save_as` | POST `/api/project/{sid}/save_as` (8) |
//! | `project_close` | POST `/api/project/{sid}/close` (10), POST `/api/session/{sid}/archive` (17) |
//! | `project_rename` | POST `/api/session/{sid}/rename` (16) |
//! | `tracks_reorder` | POST `/api/reorder/{sid}` (22) |
//! | `track_set_note` | POST `/api/setnote/{sid}/{name}` (24) |
//! | `track_rename` | none (P3-14a Inspector: the name field) |
//! | `app_settings_get`, `app_settings_set` | none (P3-14a: the page's view state in `<home>/settings.json`) |
//! | `project_recent` | GET `/api/project/recent` (2) |
//! | `peaks_build_start`, `peaks_build_status` | none (lanes progress, `lanes.rs`) |
//!
//! The dirty flag, the autosave (`autosave.rs`) and the `project-dirty` event
//! (`{sid, dirty}`, on every change of the flag) are the other half of the card.
//!
//! Files of the app that are not projects live in the home folder (`set_home`, the app data
//! folder of the app; `DRUMSPIKE_HOME` for the test bridge): `recent.json`, the last ten
//! project folders, most recent first, in the format of the server's file. Without a home
//! folder the list stays empty and nothing is written.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use drumspike_core::project::{
    ANALYSIS_DIR, DEFAULT_MIX_MODE, Settings, abspath, check_name, check_track_name, document_path, new_project,
    save_as, save_project_report, write_document,
};
use serde_json::{Value, json};

use crate::autosave;
use crate::error::AppError;
use crate::jobs::to_json_indent1;
use crate::lanes::LaneJobs;
use crate::registry::{Emitter, lock};
use crate::reply::Channels;
use crate::{Progress, Service};

/// The commands of this card, as the route map names them.
pub const LIFECYCLE_COMMANDS: [&str; 16] = [
    "project_new",
    "project_save",
    "project_clean_check",
    "project_save_as",
    "project_close",
    "project_rename",
    "tracks_reorder",
    "track_set_note",
    "track_rename",
    "track_set_follow",
    "app_settings_get",
    "app_settings_set",
    "project_recent",
    "project_recent_clear",
    "peaks_build_start",
    "peaks_build_status",
];

/// How many folders `project_recent` keeps (`server.RECENT_KEEP`).
pub const RECENT_KEEP: usize = 10;
/// The name `project_new` uses when the caller gives none.
pub const UNTITLED: &str = "Untitled project";
/// The audio modes of `project_new` (`settings.audio_mode`).
pub const AUDIO_MODES: [&str; 2] = ["referenced", "copied"];
/// The longest project name `project_rename` keeps (`server.rename_session`).
pub const NAME_MAX_CHARS: usize = 80;

/// The state this card adds to the service: folders, the autosave interval and the locks.
pub struct State {
    home: Mutex<Option<PathBuf>>,
    projects_dir: Mutex<Option<PathBuf>>,
    autosave_ms: AtomicU64,
    /// Save, Save As and the autosave write one at a time.
    pub(crate) io: Mutex<()>,
    recent: Mutex<()>,
    /// The state of the lane preparation jobs (`lanes.rs`).
    pub(crate) lanes: LaneJobs,
}

impl State {
    /// `DRUMSPIKE_HOME` (home folder), `DRUMSPIKE_PROJECTS` (default folder of new projects)
    /// and `DRUMSPIKE_AUTOSAVE_SECONDS` (default 120) from the environment.
    pub fn from_env() -> State {
        let dir = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
        let secs = std::env::var("DRUMSPIKE_AUTOSAVE_SECONDS").ok();
        State {
            home: Mutex::new(dir("DRUMSPIKE_HOME")),
            projects_dir: Mutex::new(dir("DRUMSPIKE_PROJECTS")),
            autosave_ms: AtomicU64::new(autosave::interval_from(secs.as_deref()).as_millis() as u64),
            io: Mutex::new(()),
            recent: Mutex::new(()),
            lanes: LaneJobs::default(),
        }
    }
}

/// The server's default folder for new projects: `~/Documents/Drumspike Projects`.
pub fn default_projects_dir() -> PathBuf {
    match std::env::var_os("HOME").filter(|v| !v.is_empty()) {
        Some(h) => PathBuf::from(h).join("Documents").join("Drumspike Projects"),
        None => std::env::temp_dir().join("Drumspike Projects"),
    }
}

/// P4-4: true when the two paths are one existing folder (the same device and inode), as `kick`
/// and `Kick` are on a disk that ignores case.
fn same_dir(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::metadata(a), std::fs::metadata(b)) {
        (Ok(x), Ok(y)) => x.dev() == y.dev() && x.ino() == y.ino(),
        _ => false,
    }
}

/// `server.unique_folder`: `<base>`, `<base> 2`, `<base> 3` ... the first that does not exist in `dir`.
pub fn unique_folder(dir: &Path, base: &str) -> String {
    let mut name = base.to_string();
    let mut k = 1;
    while dir.join(&name).exists() {
        k += 1;
        name = format!("{base} {k}");
    }
    name
}

/// The app's JSON writers go through the shared atomic-file helper (`io::atomic`): a unique
/// temporary file next to `path`, synced, renamed, the folder fsynced (R10).
pub(crate) fn write_json_atomic(path: &Path, v: &Value) -> Result<(), AppError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| AppError::internal(format!("{}: {e}", parent.display())))?;
    }
    drumspike_core::io::write_atomic(path, to_json_indent1(v).as_bytes())
        .map_err(|e| AppError::internal(format!("{}: {e}", path.display())))
}

impl Service {
    /// Set the home folder (the app data folder); it holds `recent.json`.
    pub fn set_home(&self, dir: impl Into<PathBuf>) {
        *lock(&self.inner.life.home) = Some(dir.into());
    }

    /// The home folder, when one is set.
    pub fn home(&self) -> Option<PathBuf> {
        lock(&self.inner.life.home).clone()
    }

    /// Set the default folder of new projects.
    pub fn set_projects_dir(&self, dir: impl Into<PathBuf>) {
        *lock(&self.inner.life.projects_dir) = Some(dir.into());
    }

    /// The default folder of new projects: the one set, else `DRUMSPIKE_PROJECTS`, else
    /// `~/Documents/Drumspike Projects`.
    pub fn projects_dir(&self) -> PathBuf {
        lock(&self.inner.life.projects_dir)
            .clone()
            .unwrap_or_else(default_projects_dir)
    }

    /// Set the autosave interval (tests use 1 s; the default is 120 s).
    pub fn set_autosave_interval(&self, d: Duration) {
        self.inner
            .life
            .autosave_ms
            .store(d.as_millis().max(1) as u64, Ordering::Relaxed);
    }

    /// The autosave interval.
    pub fn autosave_interval(&self) -> Duration {
        Duration::from_millis(self.inner.life.autosave_ms.load(Ordering::Relaxed))
    }

    /// Set where the events (`project-dirty`) go: name and JSON payload.
    pub fn set_emitter(&self, f: Emitter) {
        self.inner.registry.emitter.set(f);
    }

    /// `touch_recent`: put a project folder at the top of `recent.json` (no duplicates, ten at
    /// most). Does nothing without a home folder; a failed write is not an error of the command.
    pub fn touch_recent(&self, folder: &Path, name: &str) {
        let Some(home) = self.home() else { return };
        let _g = lock(&self.inner.life.recent);
        let path = home.join("recent.json");
        let folder_s = folder.display().to_string();
        let mut list: Vec<Value> = read_recent(&path)
            .into_iter()
            .filter(|e| e["folder"].as_str() != Some(folder_s.as_str()))
            .collect();
        list.insert(
            0,
            json!({"folder": folder_s, "name": name, "opened": drumspike_core::project::now()}),
        );
        list.truncate(RECENT_KEEP);
        if let Err(e) = write_json_atomic(&path, &Value::Array(list)) {
            eprintln!("recent.json: {e}");
        }
    }

    /// `GET /api/project/recent`: the last ten project folders that still exist, most recent first.
    pub fn project_recent(&self) -> Value {
        let Some(home) = self.home() else {
            return json!([]);
        };
        let _g = lock(&self.inner.life.recent);
        let list: Vec<Value> = read_recent(&home.join("recent.json"))
            .into_iter()
            .filter(|e| e["folder"].as_str().is_some_and(|f| Path::new(f).is_dir()))
            .take(RECENT_KEEP)
            .collect();
        Value::Array(list)
    }

    /// #441 Clear Recent: empty the stored recent list. Answers the number of entries the
    /// file held. Without a home folder nothing is written and the answer is `{"cleared": 0}`.
    pub fn project_recent_clear(&self) -> Value {
        let Some(home) = self.home() else {
            return json!({"cleared": 0});
        };
        let _g = lock(&self.inner.life.recent);
        let path = home.join("recent.json");
        let had = read_recent(&path).len();
        if let Err(e) = write_json_atomic(&path, &Value::Array(Vec::new())) {
            eprintln!("recent.json: {e}");
        }
        json!({"cleared": had})
    }

    /// `POST /api/project/new` and `POST /api/session`: create `<dir>/<name>/` with the layout
    /// and open it. Without `name` the project is "Untitled project" (" 2", " 3" when the
    /// folder exists); a given name that exists is 409. `dir` defaults to [`Service::projects_dir`].
    /// `audio_mode` (`referenced`, the default, or `copied`) is kept in `settings.audio_mode`
    /// for the track import. Answers the summary of `project_open`.
    ///
    /// # Errors
    ///
    /// Returns a 400 error when `audio_mode` is neither `referenced` nor `copied` or the name is not a valid
    /// folder name, a 409 error when the project folder already exists or its lock is held by another process,
    /// and an internal error when the target folder or the document cannot be written.
    pub fn project_new(
        &self,
        name: Option<&str>,
        dir: Option<&str>,
        audio_mode: Option<&str>,
    ) -> Result<Value, AppError> {
        let mode = audio_mode.unwrap_or(AUDIO_MODES[0]);
        if !AUDIO_MODES.contains(&mode) {
            return Err(AppError::bad(format!(
                "audio_mode must be referenced or copied, not {mode:?}"
            )));
        }
        let dir = match dir.filter(|d| !d.trim().is_empty()) {
            Some(d) => PathBuf::from(d),
            None => self.projects_dir(),
        };
        std::fs::create_dir_all(&dir).map_err(|e| AppError::internal(format!("{}: {e}", dir.display())))?;
        let name = match name.map(str::trim).filter(|n| !n.is_empty()) {
            Some(n) => n.to_string(),
            None => unique_folder(&dir, UNTITLED),
        };
        let mut project = new_project(&dir, &name)?;
        project
            .doc
            .settings
            .get_or_insert_with(|| Settings {
                mix_mode: Some(DEFAULT_MIX_MODE.to_string()),
                ..Settings::default()
            })
            .extra
            .insert("audio_mode".into(), Value::from(mode));
        write_document(&project.doc, &project.path())?;
        let (dir, name) = (project.dir.clone(), project.name());
        let op = self.register_project(project, false)?;
        self.touch_recent(&dir, &name);
        let g = lock(&op);
        Ok(g.summary())
    }

    /// `POST /api/project/{sid}/save`: write the document atomically, the previous one goes to
    /// `Backups/` (ten are kept). Clears the dirty flag. Answers `{saved, dirty: false}`.
    ///
    /// P3-15c: the files are written from a copy of the project, without the project lock. When
    /// the project did not change during the write, the copy (with the `modified` time of the
    /// file) becomes the project and the flag is cleared; after a change in the meantime the
    /// project stays dirty (the file holds the state of the call) and the answer says so.
    ///
    /// #121: the document on disk must be the one this process read or wrote last (size and modification time); else
    /// nothing is written and the answer is 409 ([`crate::registry::CHANGED_ON_DISK`]). See [`Service::project_save_with`].
    ///
    /// # Errors
    ///
    /// The errors of [`Service::project_save_with`] with `force` false.
    pub fn project_save(&self, sid: &str) -> Result<Value, AppError> {
        self.project_save_with(sid, false)
    }

    /// [`Service::project_save`]; with `force` (Save anyway, #121) the document is written also when it changed on disk.
    ///
    /// # Errors
    ///
    /// Returns a 404 error for an unknown project id, a 409 error when the project is open read-only or (without
    /// `force`) when the document on disk changed since it was read or written, and an error when the document
    /// or its backup cannot be written.
    pub fn project_save_with(&self, sid: &str, force: bool) -> Result<Value, AppError> {
        let op = self.inner.registry.get_writable(sid)?;
        let _io = lock(&self.inner.life.io);
        let before = {
            let g = lock(&op);
            if !force {
                g.check_disk()?;
            }
            g.project.clone()
        };
        let mut saved = before.clone();
        let report = {
            let _span = crate::trace::stage("save:write");
            // R12: a rotation failure after the commit is a warning; the baseline still moves.
            save_project_report(&mut saved)?
        };
        let (dir, name) = (saved.dir.clone(), saved.name());
        let dirty = {
            let mut g = lock(&op);
            if g.project == before {
                g.project = saved;
                g.mark_clean();
            } else {
                g.saved_doc = before.doc.clone(); // the file holds the state of the call (P4-40a: the baseline of an undo)
            }
            g.restamp(); // #121: in both cases the file is the one this call wrote
            g.dirty
        };
        self.touch_recent(&dir, &name);
        let path = report.path.display().to_string();
        let warning = report.warning.map(|e| e.to_string());
        Ok(json!({"saved": path, "dirty": dirty, "warning": warning}))
    }

    /// `project_clean_check {sid}` (P4-40a): clears the dirty flag when the open document equals the one saved last (the
    /// page calls it after an undo or redo; the project is the saved state again). Changes nothing else. Answers `{dirty}`.
    ///
    /// # Errors
    ///
    /// Returns a 404 error when the project id is unknown.
    pub fn project_clean_check(&self, sid: &str) -> Result<Value, AppError> {
        let op = self.inner.registry.get(sid)?;
        let dirty = lock(&op).clean_if_saved();
        Ok(json!({"dirty": dirty}))
    }

    /// `POST /api/project/{sid}/save_as`: write the project into the new folder `<dir>/<name>/`
    /// with referenced records for every source (a copied file of the old project becomes a
    /// referenced file of the new one); the id points at the new project afterwards and the
    /// project is clean. Answers the summary. With `copy_audio` the audio is copied into the
    /// new `Audio Files/` as a job (`copyjob.rs`): the answer is `{job: <state>}` and the summary
    /// comes as the `result` of the finished job; `progress` gets `{sid, copy: ...}`.
    ///
    /// # Errors
    ///
    /// Returns a 400 error for a missing or invalid `name`, a 404 error for an unknown project id, and a 409
    /// error when `<dir>/<name>/` already holds a project, a copy to that name is running, or its lock is held
    /// by another process. Writing the new folder or document can also fail with an internal error.
    pub fn project_save_as(
        &self,
        sid: &str,
        name: Option<&str>,
        dir: Option<&str>,
        copy_audio: bool,
        progress: Option<Progress>,
    ) -> Result<Value, AppError> {
        let name = name
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .ok_or_else(|| AppError::bad("name required"))?;
        let dir = match dir.filter(|d| !d.trim().is_empty()) {
            Some(d) => PathBuf::from(d),
            None => self.projects_dir(),
        };
        if copy_audio {
            return self.save_as_copy_job(sid, name, &dir, progress);
        }
        let op = self.inner.registry.get(sid)?;
        let _io = lock(&self.inner.life.io);
        // P4-5 (M1): the new folder's lock first; on any error nothing changes (old project, folder and lock). An
        // existing project is refused before a lock or a lock file is made (it may be one nobody holds).
        let pdir = abspath(&dir).join(check_name(name)?);
        if document_path(&pdir).exists() {
            return Err(AppError::conflict(format!(
                "project already exists: {}",
                pdir.display()
            )));
        }
        // P4-5 (L2): a copy Save As to this name that is running holds `<name>.partial`; 409 now, not after its copy
        let partial = crate::copyjob::partial_of(&pdir);
        if partial.join(crate::lock::LOCK_FILE).exists()
            && let Some(client) = self.lock_client()
        {
            crate::lock::ProjectLock::acquire(&partial, &client)?; // dropped at once
        }
        let new_folder = self.lock_new_folder(&pdir)?;
        let (new_dir, new_name, summary) = {
            let mut g = lock(&op);
            // an error drops `new_folder`: the lock file this call made goes, then the lock, then an empty new folder
            let newp = save_as(&g.project, &dir, name, false)?;
            g.project = newp;
            g.switch_lock(new_folder.and_then(NewFolder::into_lock));
            g.mark_clean();
            g.restamp(); // #121
            g.resolve();
            (g.project.dir.clone(), g.project.name(), g.summary())
        };
        self.touch_recent(&new_dir, &new_name);
        Ok(summary)
    }

    /// P4-5 (M1): the lock of the folder `pdir` that Save As is about to write, taken before anything is written.
    /// `Ok(None)` when this process does not lock. The folder is made (empty) to hold the lock file. 409 when another
    /// process holds a lock on it; a folder that cannot hold a lock is [`crate::lock::Lock::Unavailable`]. The
    /// [`NewFolder`] removes what this call made unless it is turned into the project's lock.
    pub(crate) fn lock_new_folder(&self, pdir: &Path) -> Result<Option<NewFolder>, AppError> {
        let Some(client) = self.lock_client() else {
            return Ok(None);
        };
        let dir_existed = pdir.exists();
        let lock_file_existed = pdir.join(crate::lock::LOCK_FILE).exists();
        let _ = std::fs::create_dir_all(pdir);
        let mut nf = NewFolder {
            pdir: pdir.to_path_buf(),
            dir_existed,
            lock_file_existed,
            lock: None,
            keep: false,
        };
        nf.lock = Some(crate::lock::ProjectLock::acquire(pdir, &client)?); // an error drops `nf`: the made folder goes
        Ok(Some(nf))
    }

    /// `POST /api/project/{sid}/close` and `/api/session/{sid}/archive`: take the project out of
    /// the registry. 409 while it is dirty unless `force`. Answers `{closed, folder}`.
    ///
    /// # Errors
    ///
    /// Returns a 404 error when the project id is unknown and a 409 error when the project has unsaved
    /// changes and `force` is false.
    pub fn project_close(&self, sid: &str, force: bool) -> Result<Value, AppError> {
        let op = self.inner.registry.get(sid)?;
        let (folder, drag) = {
            let g = lock(&op);
            if g.dirty && !force {
                return Err(AppError::conflict(
                    "project has unsaved changes: save it or close with force=true",
                ));
            }
            (g.project.dir.display().to_string(), crate::shell::drag_dir_of(&g))
        };
        self.inner.registry.remove(sid)?;
        crate::shell::drag_clear_dir(&drag); // #433: the pre-rendered drag files (never a file of Exports/)
        self.inner.jobs.clear(sid);
        self.inner.copyjobs.forget(sid);
        self.inner.life.lanes.clear(sid);
        self.transport_close_project(sid); // P3-5b: a transport that plays this project is released
        Ok(json!({"closed": sid, "folder": folder}))
    }

    /// `POST /api/session/{sid}/rename`: the display name only (folder and document keep their
    /// names). Trimmed, cut to 80 characters, no path parts. Marks the project dirty.
    ///
    /// # Errors
    ///
    /// Returns a 404 error for an unknown project id, a 409 error when the project is open read-only, and a
    /// 400 error for an empty name or one that is not a valid file name.
    pub fn project_rename(&self, sid: &str, name: &str) -> Result<Value, AppError> {
        let op = self.inner.registry.get_writable(sid)?;
        let name = name.trim();
        if name.is_empty() {
            return Err(AppError::bad("empty name"));
        }
        let name = check_name(&name.chars().take(NAME_MAX_CHARS).collect::<String>())?;
        let mut g = lock(&op);
        g.project.doc.name = Some(name.clone());
        g.mark_dirty();
        Ok(json!({"name": name}))
    }

    /// `POST /api/reorder/{sid}`: the display order, a permutation of the track names. Marks
    /// the project dirty. Answers `{order}`.
    ///
    /// # Errors
    ///
    /// Returns a 404 error for an unknown project id, a 409 error when the project is open read-only, and a
    /// 400 error when `order` is not a list of strings that is a permutation of the track names.
    pub fn tracks_reorder(&self, sid: &str, order: &Value) -> Result<Value, AppError> {
        let op = self.inner.registry.get_writable(sid)?;
        let mut g = lock(&op);
        let names: Option<Vec<String>> = order
            .as_array()
            .and_then(|a| a.iter().map(|v| v.as_str().map(str::to_string)).collect());
        let mut want: Vec<String> = g.project.doc.tracks().map(|(n, _)| n.clone()).collect();
        let bad = || AppError::bad("order must be a permutation of the track names");
        let names = names.ok_or_else(bad)?;
        let mut got = names.clone();
        got.sort();
        want.sort();
        if got != want {
            return Err(bad());
        }
        g.project.doc.order = Some(names);
        g.mark_dirty();
        Ok(json!({"order": order}))
    }

    /// `POST /api/setnote/{sid}/{name}`: the MIDI note of a track, 0 to 127 (the server only
    /// takes an integer). Marks the project dirty. Answers `{note}`.
    ///
    /// # Errors
    ///
    /// Returns a 404 error for an unknown project id, a 409 error when the project is open read-only, and a
    /// 400 error for an unknown track or a `note` outside 0 to 127.
    pub fn track_set_note(&self, sid: &str, name: &str, note: i64) -> Result<Value, AppError> {
        let op = self.inner.registry.get_writable(sid)?;
        let mut g = lock(&op);
        g.track(name)?;
        if !(0..=127).contains(&note) {
            return Err(AppError::bad(format!("note must be 0 to 127, not {note}")));
        }
        g.track_mut(name)?.note = Some(note);
        g.mark_dirty();
        Ok(json!({"note": note}))
    }

    /// P3-14a (Inspector): rename a track. The name is the key of the track record, an entry of
    /// `order`, and the folder `Analysis/<name>/` (the peak cache), so all three change. The
    /// track keeps its place. 400 for a bad or unknown name, 409 when the new name is taken or
    /// when an analysis, a lane build or a copy job of the project runs (they write under the
    /// old name). Marks the project dirty. Answers `{name}` (the new name).
    ///
    /// # Errors
    ///
    /// Returns a 404 error for an unknown project id, a 400 error for an unknown track or a new name that is too
    /// long or invalid, and a 409 error when the project is open read-only, the new name is taken, or an
    /// analysis, lane build or copy job of the project runs.
    pub fn track_rename(&self, sid: &str, name: &str, new_name: &str) -> Result<Value, AppError> {
        let op = self.inner.registry.get_writable(sid)?;
        let new_name = new_name.trim();
        if new_name.chars().count() > NAME_MAX_CHARS {
            return Err(AppError::bad(format!(
                "the name is longer than {NAME_MAX_CHARS} characters"
            )));
        }
        let new_name = check_track_name(new_name)?.to_string();
        let mut g = lock(&op);
        g.track(name)?;
        if new_name == name {
            return Ok(json!({"name": new_name}));
        }
        // P4-4: a name that differs only in case is taken too (one file name on a disk that ignores case)
        if let Some((taken, _)) = g
            .project
            .doc
            .tracks()
            .find(|(n, _)| n.as_str() != name && crate::export::same_file_name(n, &new_name))
        {
            return Err(AppError::conflict(format!("a track named {taken:?} exists")));
        }
        for (what, state) in [
            ("an analysis", self.inner.jobs.status(sid)["state"].clone()),
            ("a lane build", self.inner.life.lanes.status(sid)["state"].clone()),
            ("a copy job", self.inner.copyjobs.status(sid)["state"].clone()),
        ] {
            if state == "queued" || state == "running" {
                return Err(AppError::conflict(format!(
                    "{what} of this project runs: rename the track after it"
                )));
            }
        }
        let adir = g.project.subdir(ANALYSIS_DIR);
        let (old_dir, new_dir) = (adir.join(name), adir.join(&new_name));
        if old_dir.is_dir() {
            // P4-4: a rename that changes only the case is the same folder on a disk that ignores
            // case; removing the "other" folder removed the track's own cache, then the rename failed
            if new_dir.exists() && !same_dir(&old_dir, &new_dir) {
                std::fs::remove_dir_all(&new_dir)
                    .map_err(|e| AppError::internal(format!("{}: {e}", new_dir.display())))?;
            }
            std::fs::rename(&old_dir, &new_dir)
                .map_err(|e| AppError::internal(format!("{}: {e}", old_dir.display())))?;
        }
        let tracks = g.project.doc.tracks_mut();
        let index = tracks.get_index_of(name).unwrap_or(tracks.len());
        if let Some(tr) = tracks.shift_remove(name) {
            tracks.shift_insert(index.min(tracks.len()), new_name.clone(), tr);
        }
        if let Some(order) = g.project.doc.order.as_mut() {
            for n in order.iter_mut().filter(|n| n.as_str() == name) {
                *n = new_name.clone();
            }
        }
        // P3-18: the followers of the renamed track follow the new name
        for (_, tr) in g.project.doc.tracks_mut().iter_mut() {
            if tr.follows.as_deref() == Some(name) {
                tr.follows = Some(new_name.clone());
            }
        }
        if let Some(status) = g.status.remove(name) {
            g.status.insert(new_name.clone(), status);
        }
        if let Some(path) = g.paths.remove(name) {
            g.paths.insert(new_name.clone(), path);
        }
        g.mark_dirty();
        Ok(json!({"name": new_name}))
    }

    /// P3-14a: the app settings (`<home>/settings.json`) as one object; `{}` without a home folder
    /// or a file. The page keeps its view state here (`inspector`).
    pub fn app_settings_get(&self) -> Value {
        self.home()
            .map(|h| h.join("settings.json"))
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}))
    }

    /// P3-14a: write one key of the app settings; the other keys stay. The key is a short
    /// identifier (`[a-z_]`, 1 to 32 characters); the audio keys (`audio_output`, `master_db`)
    /// belong to the transport (400). Answers `{key, value}`; without a home folder nothing is
    /// written.
    ///
    /// # Errors
    ///
    /// Returns a 400 error when `key` is not 1 to 32 characters of `[a-z_]` or is an audio key, and an internal
    /// error when `settings.json` in the home folder cannot be written.
    pub fn app_settings_set(&self, key: &str, value: &Value) -> Result<Value, AppError> {
        let short = (1..=32).contains(&key.len()) && key.bytes().all(|b| b.is_ascii_lowercase() || b == b'_');
        if !short {
            return Err(AppError::bad(format!("bad settings key {key:?}")));
        }
        if key == "audio_output" || key == "master_db" {
            return Err(AppError::bad(format!("{key} is set by the audio commands")));
        }
        if let Some(path) = self.home().map(|h| h.join("settings.json")) {
            let _hold = lock(&self.inner.life.recent); // one writer of the home files at a time
            let mut v = self.app_settings_get();
            v[key] = value.clone();
            write_json_atomic(&path, &v)?;
        }
        Ok(json!({"key": key, "value": value}))
    }
}

/// The entries of `recent.json`; a missing or damaged file is an empty list.
fn read_recent(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| match v {
            Value::Array(a) => Some(a),
            _ => None,
        })
        .map(|a| a.into_iter().filter(serde_json::Value::is_object).collect())
        .unwrap_or_default()
}

// ---------- the bridge ----------

fn arg_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, AppError> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::bad(format!("missing or wrong argument `{key}` (string)")))
}

fn opt_str<'a>(args: &'a Value, key: &str) -> Result<Option<&'a str>, AppError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_str()
            .map(Some)
            .ok_or_else(|| AppError::bad(format!("wrong argument `{key}` (string)"))),
    }
}

/// The progress callback of an argument `{"__channel": id}` (see `bridge.rs`).
fn channel_progress(args: &Value, channels: &Arc<Channels>) -> Option<Progress> {
    let id = args.get("progress")?.get("__channel")?.as_str()?.to_string();
    let ch = Arc::clone(channels);
    let p: Progress = Arc::new(move |v: Value| ch.push(&id, v));
    Some(p)
}

/// Run a command of this card for the bridge; `None` when `cmd` is not one of them. The
/// argument names are those of the Tauri wrappers in `apps/desktop/src-tauri/src/lifecycle.rs`.
pub fn dispatch(svc: &Service, channels: &Arc<Channels>, cmd: &str, args: &Value) -> Option<Result<Value, AppError>> {
    let run = || -> Result<Value, AppError> {
        match cmd {
            "project_new" => svc.project_new(
                opt_str(args, "name")?,
                opt_str(args, "dir")?,
                opt_str(args, "audio_mode")?,
            ),
            "project_save" => svc.project_save_with(
                arg_str(args, "sid")?,
                args.get("force").and_then(Value::as_bool).unwrap_or(false),
            ),
            "project_clean_check" => svc.project_clean_check(arg_str(args, "sid")?),
            "project_save_as" => svc.project_save_as(
                arg_str(args, "sid")?,
                opt_str(args, "name")?,
                opt_str(args, "dir")?,
                args.get("copy_audio").and_then(Value::as_bool).unwrap_or(false),
                channel_progress(args, channels),
            ),
            "project_close" => svc.project_close(
                arg_str(args, "sid")?,
                args.get("force").and_then(Value::as_bool).unwrap_or(false),
            ),
            "project_rename" => svc.project_rename(arg_str(args, "sid")?, arg_str(args, "name")?),
            "tracks_reorder" => svc.tracks_reorder(arg_str(args, "sid")?, args.get("order").unwrap_or(&Value::Null)),
            "track_set_note" => svc.track_set_note(
                arg_str(args, "sid")?,
                arg_str(args, "name")?,
                args.get("note")
                    .and_then(Value::as_i64)
                    .ok_or_else(|| AppError::bad("missing or wrong argument `note` (integer)"))?,
            ),
            "track_rename" => svc.track_rename(
                arg_str(args, "sid")?,
                arg_str(args, "name")?,
                arg_str(args, "new_name")?,
            ),
            "track_set_follow" => {
                svc.track_set_follow(arg_str(args, "sid")?, arg_str(args, "name")?, opt_str(args, "source")?)
            }
            "app_settings_get" => Ok(svc.app_settings_get()),
            "app_settings_set" => {
                svc.app_settings_set(arg_str(args, "key")?, args.get("value").unwrap_or(&Value::Null))
            }
            "project_recent" => Ok(svc.project_recent()),
            "project_recent_clear" => Ok(svc.project_recent_clear()),
            "peaks_build_start" => {
                let tracks = match args.get("tracks") {
                    None | Some(Value::Null) => None,
                    Some(Value::Array(a)) => Some(
                        a.iter()
                            .map(|v| {
                                v.as_str()
                                    .map(str::to_string)
                                    .ok_or_else(|| AppError::bad("`tracks` must hold strings"))
                            })
                            .collect::<Result<Vec<_>, _>>()?,
                    ),
                    Some(_) => return Err(AppError::bad("`tracks` must be a list")),
                };
                svc.peaks_build_start(arg_str(args, "sid")?, tracks, channel_progress(args, channels))
            }
            "peaks_build_status" => Ok(svc.peaks_build_status(arg_str(args, "sid")?)),
            _ => Err(AppError::not_found(format!("unknown command {cmd}"))),
        }
    };
    LIFECYCLE_COMMANDS.contains(&cmd).then(run)
}

/// P4-5: the folder a Save As is about to write and its lock (see `Service::lock_new_folder`). Dropped without
/// [`NewFolder::into_lock`] (a refused or failed Save As, a job that did not start) it removes only what the call made:
/// the lock file (while the lock is still held, so no one can lock the unlinked file), then the lock, then the folder
/// when it did not exist before and is empty. A folder or lock file that was there before stays.
pub(crate) struct NewFolder {
    pdir: PathBuf,
    dir_existed: bool,
    lock_file_existed: bool,
    lock: Option<crate::lock::Lock>,
    keep: bool,
}

impl NewFolder {
    /// The lock, for the project that now lives in the folder; nothing is removed.
    pub(crate) fn into_lock(mut self) -> Option<crate::lock::Lock> {
        self.keep = true;
        self.lock.take()
    }
}

impl Drop for NewFolder {
    fn drop(&mut self) {
        if self.keep {
            return;
        }
        if !self.lock_file_existed {
            let _ = std::fs::remove_file(self.pdir.join(crate::lock::LOCK_FILE));
        }
        drop(self.lock.take());
        if !self.dir_existed {
            let _ = std::fs::remove_dir(&self.pdir); // only when empty
        }
    }
}

#[cfg(test)]
mod tests;
