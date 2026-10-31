//! Copy jobs (P3-4d): Save As with copied audio, Consolidate, and the cancel that all of them
//! share (`job_cancel`), also for a copy-mode `tracks_add` and the relink search.
//!
//! | Command | Server route (route map row) |
//! |---|---|
//! | `project_save_as {sid, name, dir, copy_audio, progress}` | POST `/api/project/{sid}/save_as` (8) |
//! | `project_consolidate {sid, progress}` | POST `/api/project/{sid}/consolidate` (9) |
//! | `job_status {sid}` | none (the state of the last copy job of a project) |
//! | `job_cancel {sid}` | none (stops the running job of a project) |
//!
//! A job runs on its own thread and never holds the project lock while it copies: the project
//! stays usable (an `events_set` during the copy works). One job runs for a project at a time
//! (409 for a second one). The progress channel gets `{sid, copy: {id, kind, state, done, total,
//! bytes_done, bytes_total, file, ...}}` before and after each file and, at most every 100 ms,
//! while a large file is copied. `state` is `running` and then one of `done` (with `result`),
//! `cancelled` or `error` (with `error`).
//!
//! Save As copies into the new folder first; only when every file is there does it write the
//! document from the project as it is then (edits made during the copy are in the new
//! document), switch the project to the new folder and mark it clean. A cancel or an error
//! removes everything the job made (the temporary file, the copied files, the new folder when
//! the job created it) and leaves the open project as it was. Consolidate changes the records
//! only at the end; a cancel removes the files it copied.
//!
//! Files are copied in 4 MiB chunks (`copy_chunked`), so a cancel acts within one chunk.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use drumspike_core::project::{
    CopyCtl, Source, abspath, check_name, copy_into_project_ctl, document_path, ensure_layout, resolve_source,
    save_project_into,
};
use drumspike_core::{Project, SourceStatus};
use serde_json::{Value, json};

use crate::error::AppError;
use crate::registry::{Shared, lock};
use crate::reply::Channels;
use crate::{Progress, Service};

/// The commands of this card that are not in the lifecycle list.
pub const COPYJOB_COMMANDS: [&str; 3] = ["project_consolidate", "job_status", "job_cancel"];

/// Progress messages for the bytes of one file come at most this often.
const BYTES_EVERY: Duration = Duration::from_millis(100);

/// One running or finished job of a project: the cancel flag and the public state.
#[derive(Debug)]
pub struct JobCtl {
    id: String,
    kind: &'static str,
    /// Test pause after each chunk written, in ms (`Service::set_copy_delay`); 0 in the app.
    delay_ms: u64,
    cancel: AtomicBool,
    state: Mutex<Value>,
}

impl JobCtl {
    /// True after `job_cancel`.
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    /// The public state (a copy).
    pub fn snapshot(&self) -> Value {
        lock(&self.state).clone()
    }

    fn running(&self) -> bool {
        matches!(self.snapshot()["state"].as_str(), Some("running"))
    }

    fn update(&self, f: impl FnOnce(&mut Value)) -> Value {
        let mut g = lock(&self.state);
        f(&mut g);
        g.clone()
    }
}

/// The copy jobs of all projects, by project id: the running job or the last finished one.
#[derive(Default)]
pub struct CopyJobs {
    map: Mutex<HashMap<String, Arc<JobCtl>>>,
    seq: AtomicU64,
    delay_ms: AtomicU64,
}

impl CopyJobs {
    /// Register a new job of `kind` for `sid`; 409 while another job of that project runs.
    ///
    /// # Errors
    ///
    /// A 409 [`AppError`] naming the kind of the job when a job of `sid` is still `running`.
    pub fn start(&self, sid: &str, kind: &'static str) -> Result<Arc<JobCtl>, AppError> {
        let mut map = lock(&self.map);
        if let Some(j) = map.get(sid)
            && j.running()
        {
            return Err(AppError::conflict(format!(
                "another job ({}) is running for this project: wait for it or cancel it",
                j.kind
            )));
        }
        let id = format!("{kind}-{}", self.seq.fetch_add(1, Ordering::SeqCst) + 1);
        let job = Arc::new(JobCtl {
            id: id.clone(),
            kind,
            delay_ms: self.delay_ms.load(Ordering::Relaxed),
            cancel: AtomicBool::new(false),
            state: Mutex::new(json!({
                "id": id, "kind": kind, "state": "running", "done": 0, "total": 0,
                "bytes_done": 0, "bytes_total": 0, "file": "",
            })),
        });
        map.insert(sid.to_string(), Arc::clone(&job));
        Ok(job)
    }

    /// `job_status`: the state of the last job of `sid`, or `{state: "idle"}`.
    pub fn status(&self, sid: &str) -> Value {
        lock(&self.map)
            .get(sid)
            .map_or_else(|| json!({"state": "idle"}), |j| j.snapshot())
    }

    /// `job_cancel`: set the cancel flag of the running job of `sid`. Answers
    /// `{sid, cancelling, id?, kind?}`; `cancelling` is false when no job runs.
    pub fn cancel(&self, sid: &str) -> Value {
        match lock(&self.map).get(sid) {
            Some(j) if j.running() => {
                j.cancel.store(true, Ordering::SeqCst);
                json!({"sid": sid, "cancelling": true, "id": j.id, "kind": j.kind})
            }
            _ => json!({"sid": sid, "cancelling": false}),
        }
    }

    /// The project was closed: cancel its job and forget it.
    pub fn forget(&self, sid: &str) {
        if let Some(j) = lock(&self.map).remove(sid) {
            j.cancel.store(true, Ordering::SeqCst);
        }
    }
}

/// Counts files and bytes of one job, keeps the state of the [`JobCtl`] current and sends the
/// progress messages.
pub struct Meter {
    job: Arc<JobCtl>,
    sid: String,
    progress: Option<Progress>,
    bytes: AtomicU64,
    last: Mutex<Instant>,
    sent: AtomicUsize,
}

impl Meter {
    /// A meter for `job`; `total` files and `bytes_total` bytes are known at the start.
    pub fn new(job: &Arc<JobCtl>, sid: &str, progress: Option<Progress>, total: usize, bytes_total: u64) -> Meter {
        job.update(|s| {
            s["total"] = json!(total);
            s["bytes_total"] = json!(bytes_total);
        });
        Meter {
            job: Arc::clone(job),
            sid: sid.to_string(),
            progress,
            bytes: AtomicU64::new(0),
            last: Mutex::new(Instant::now()),
            sent: AtomicUsize::new(0),
        }
    }

    fn send(&self, state: Value) {
        self.sent.fetch_add(1, Ordering::Relaxed);
        if let Some(p) = &self.progress {
            p(json!({"sid": self.sid, "copy": state}));
        }
    }

    /// File number `i` (0-based) starts.
    pub fn file_start(&self, i: usize, file: &str) {
        let st = self.job.update(|s| {
            s["done"] = json!(i);
            s["file"] = json!(file);
            s["bytes_done"] = json!(self.bytes.load(Ordering::Relaxed));
        });
        self.send(st);
    }

    /// File number `i` is finished.
    pub fn file_done(&self, i: usize, file: &str) {
        let st = self.job.update(|s| {
            s["done"] = json!(i + 1);
            s["file"] = json!(file);
            s["bytes_done"] = json!(self.bytes.load(Ordering::Relaxed));
        });
        self.send(st);
    }

    /// `n` more bytes were written.
    pub fn bytes(&self, n: u64) {
        let total = self.bytes.fetch_add(n, Ordering::Relaxed) + n;
        let mut last = lock(&self.last);
        if last.elapsed() >= BYTES_EVERY {
            *last = Instant::now();
            drop(last);
            let st = self.job.update(|s| s["bytes_done"] = json!(total));
            self.send(st);
        }
    }

    /// The hooks for `copy_into_project_ctl`.
    pub fn with_ctl<R>(&self, f: impl FnOnce(&CopyCtl<'_>) -> R) -> R {
        let cancelled = || self.job.cancelled();
        let delay = Duration::from_millis(self.job.delay_ms);
        let on_bytes = |n: u64| {
            self.bytes(n);
            if !delay.is_zero() {
                std::thread::sleep(delay); // tests only: a job that lasts long enough to be cancelled
            }
        };
        f(&CopyCtl {
            cancelled: &cancelled,
            on_bytes: &on_bytes,
        })
    }

    /// The job ends. `state` is `done`, `cancelled` or `error`; the final message carries
    /// `result` or `error`.
    pub fn finish(&self, state: &str, result: Option<Value>, error: Option<&str>) {
        let st = self.job.update(|s| {
            s["state"] = json!(state);
            s["bytes_done"] = json!(self.bytes.load(Ordering::Relaxed));
            if state == "done" {
                s["done"] = s["total"].clone();
            }
            if let Some(r) = result {
                s["result"] = r;
            }
            if let Some(e) = error {
                s["error"] = json!(e);
            }
        });
        self.send(st);
    }

    /// How many progress messages this meter sent.
    pub fn sent(&self) -> usize {
        self.sent.load(Ordering::Relaxed)
    }
}

impl Drop for Meter {
    /// A job that ends without `finish` (a panic) must not stay `running` for ever.
    fn drop(&mut self) {
        if self.job.running() {
            self.finish("error", None, Some("the job stopped unexpectedly"));
        }
    }
}

/// A track to copy: its name, the record it has now and the file the record resolved to.
#[derive(Debug, Clone)]
struct Item {
    track: String,
    /// P4-7a fix 6: the record is the track's layer file, not its source.
    layer: bool,
    record: Source,
    src: PathBuf,
}

/// The source record of the layer of `tr`, `None` for a track without a layer.
fn layer_record(tr: &drumspike_core::project::Track) -> Option<Source> {
    crate::layer::layer_of(&tr.extra).map(|l| l.source)
}

/// The record of the item `it`'s kind in `tr`.
fn record_of(tr: &drumspike_core::project::Track, layer: bool) -> Option<Source> {
    if layer {
        layer_record(tr)
    } else {
        tr.source_ref().cloned()
    }
}

/// Put `new` as the record of the item's kind in `tr`.
fn set_record(tr: &mut drumspike_core::project::Track, layer: bool, new: Source) {
    if !layer {
        tr.source = Some(Some(new));
    } else if let Some(mut l) = crate::layer::layer_of(&tr.extra) {
        l.source = new;
        tr.extra.insert("layer".into(), l.to_json());
    }
}

/// The tracks with a source record, in document order, each with its resolved file. 409 for a
/// track whose source is not online (`what` is the action: `Save As`, `Consolidate`).
fn snapshot_items(g: &crate::registry::OpenProject, only_referenced: bool, what: &str) -> Result<Vec<Item>, AppError> {
    let mut out = Vec::new();
    for (name, tr) in g.project.doc.tracks() {
        let Some(rec) = tr.source_ref() else { continue };
        if only_referenced && rec.mode.as_deref() != Some("referenced") {
            continue;
        }
        let status = g.status.get(name).copied().unwrap_or(SourceStatus::Missing);
        let src = g.paths.get(name).cloned().flatten();
        match (status, src) {
            (SourceStatus::Ok, Some(src)) => out.push(Item {
                track: name.clone(),
                layer: false,
                record: rec.clone(),
                src,
            }),
            (status, _) if only_referenced => {
                return Err(AppError::conflict(format!(
                    "cannot copy source ({}): {}",
                    status.as_str(),
                    rec.path_or_relative()
                )));
            }
            (status, _) => {
                return Err(AppError::conflict(format!(
                    "track {name}: source {}; relink it before {what}",
                    status.as_str()
                )));
            }
        }
    }
    // P4-7a fix 6: the layer files go with the sources. One that is missing or changed stays as it is (the
    // relink bar lists it); it does not stop the copy.
    for (name, tr) in g.project.doc.tracks() {
        let Some(l) = crate::layer::layer_of(&tr.extra) else {
            continue;
        };
        if only_referenced && l.source.mode.as_deref() != Some("referenced") {
            continue;
        }
        if let (Some(src), SourceStatus::Ok) = resolve_source(Some(&l.source), &g.project.dir) {
            out.push(Item {
                track: name.clone(),
                layer: true,
                record: l.source,
                src,
            });
        }
    }
    Ok(out)
}

fn bytes_of(items: &[Item]) -> u64 {
    items.iter().map(|i| i.record.size.unwrap_or(0).max(0) as u64).sum()
}

fn file_label(p: &Path) -> String {
    p.file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
}

/// What a worker did.
enum End {
    Done(Value),
    Cancelled,
}

/// What a Save As worker has made so far, so that a cancel or an error can remove it.
struct Made {
    /// The final folder `<dir>/<name>`: it exists only when the copy is complete (P4-5, L2).
    pdir: PathBuf,
    /// Where the copy is built, `<dir>/<name>.partial`; renamed to `pdir` as the last step. Removed whole on a cancel or an error.
    build: PathBuf,
    files: Vec<PathBuf>,
    /// P4-5 (M1): the lock of the new folder, taken before the job started; the project takes it when the job ends.
    new_folder: Option<crate::lifecycle::NewFolder>,
}

impl Made {
    fn remove(&mut self) {
        for f in &self.files {
            let _ = std::fs::remove_file(f);
        }
        let _ = std::fs::remove_dir_all(&self.build); // the partial folder is the job's own
        self.new_folder = None; // the lock goes (its file was in the partial folder)
    }
}

/// True when `pdir` is an empty folder (apart from a lock file): a copy may be renamed onto it.
pub(crate) fn folder_is_free(pdir: &Path) -> bool {
    match std::fs::read_dir(pdir) {
        Ok(rd) => rd
            .filter_map(Result::ok)
            .all(|e| e.file_name() == crate::lock::LOCK_FILE),
        Err(_) => false, // a file, or a folder that cannot be read
    }
}

/// `<dir>/<name>.partial` for the destination `<dir>/<name>`.
pub(crate) fn partial_of(pdir: &Path) -> PathBuf {
    let mut n = pdir.as_os_str().to_os_string();
    n.push(".partial");
    PathBuf::from(n)
}

/// The record `src` with an absolute `path` inside `from` moved to the same place inside `to`.
fn rebase(src: &Source, from: &Path, to: &Path) -> Source {
    let mut s = src.clone();
    if let Some(p) = &src.path
        && let Ok(rel) = Path::new(p).strip_prefix(from)
    {
        s.path = Some(to.join(rel).display().to_string());
    }
    s
}

impl Service {
    /// A `<name>.partial` folder an earlier, killed Save As left: removed before a new Save As to the name builds
    /// there. 409 when a process still holds its lock (a Save As to that name is running). Without a lock client
    /// (the tests) it is removed as it is.
    fn clear_stale_partial(&self, build: &Path) -> Result<(), AppError> {
        self.clear_stale_partial_with(build, || {})
    }

    /// [`Self::clear_stale_partial`] with a hook that runs after the lock is taken and before the removal (tests).
    /// P4-5 (L3): the lock is taken (its file is made when it is missing) before anything is deleted, so a Save As
    /// that has just made the folder and not yet its lock file cannot lose it.
    fn clear_stale_partial_with(&self, build: &Path, before_remove: impl FnOnce()) -> Result<(), AppError> {
        if !build.exists() {
            return Ok(());
        }
        let held = match self.lock_client() {
            Some(client) => Some(crate::lock::ProjectLock::acquire(build, &client)?),
            None => None,
        };
        before_remove();
        let r = std::fs::remove_dir_all(build);
        drop(held);
        r.map_err(|e| AppError::conflict(format!("cannot remove the unfinished copy {}: {e}", build.display())))
    }

    /// The copy of Save As (`project_save_as` with `copy_audio`): checks, then the worker
    /// thread. Answers `{job: <state>}` with the state `running`; the end comes on the progress
    /// channel (`result` is the summary of the new project) and from `job_status`.
    pub(crate) fn save_as_copy_job(
        &self,
        sid: &str,
        name: &str,
        dir: &Path,
        progress: Option<Progress>,
    ) -> Result<Value, AppError> {
        let name = check_name(name)?;
        let pdir = abspath(dir).join(&name);
        if document_path(&pdir).exists() {
            return Err(AppError::conflict(format!(
                "project already exists: {}",
                pdir.display()
            )));
        }
        // P4-5 (L1): an existing folder without a document is fine only when it is empty (the rename of the finished
        // copy onto it replaces an empty folder, never a full one): refused before any file is copied. A lock file
        // that an earlier refused call left does not count.
        if pdir.exists() && !folder_is_free(&pdir) {
            return Err(AppError::conflict(format!(
                "the folder {} exists and is not empty: choose another name or empty it",
                pdir.display()
            )));
        }
        let op = self.inner.registry.get(sid)?;
        let items = {
            let mut g = lock(&op);
            g.resolve(); // a file that vanished since the last look is offline now
            snapshot_items(&g, false, "Save As")?
        };
        // P4-5 (M1): the new folder's lock before anything is written or a job exists (409 when it is held)
        // P4-5 (L2): the copy is built in `<name>.partial` and renamed to `<name>` when complete, so a killed process
        // never leaves a destination folder without a document. The lock is on the partial folder (its lock file
        // moves with the rename); a lock held on an existing destination folder refuses the call too.
        let build = partial_of(&pdir);
        if pdir.join(crate::lock::LOCK_FILE).exists()
            && let Some(client) = self.lock_client()
        {
            crate::lock::ProjectLock::acquire(&pdir, &client)?; // 409 when it is held; dropped at once
        }
        self.clear_stale_partial(&build)?;
        let new_folder = self.lock_new_folder(&build)?; // dropped on every error path below: what this call made goes
        let job = self.inner.copyjobs.start(sid, "save_as")?;
        let meter = Meter::new(&job, sid, progress, items.len(), bytes_of(&items));
        let first = job.snapshot();
        let svc = self.clone();
        let sid = sid.to_string();
        let sid_run = sid.clone();
        let spawned = std::thread::Builder::new().name("save-as-copy".into()).spawn(move || {
            let mut made = Made {
                pdir: pdir.clone(),
                build: build.clone(),
                files: Vec::new(),
                new_folder,
            };
            let end = svc.run_save_as(&sid_run, &op, &meter, items, &name, &mut made);
            match end {
                Ok(End::Done(summary)) => meter.finish("done", Some(summary), None),
                Ok(End::Cancelled) => {
                    made.remove();
                    meter.finish("cancelled", None, None);
                }
                Err(e) => {
                    made.remove();
                    meter.finish("error", None, Some(&e.message));
                }
            }
        });
        if let Err(e) = spawned {
            self.inner.copyjobs.forget(&sid);
            return Err(AppError::internal(format!("cannot start the copy: {e}")));
        }
        Ok(json!({"job": first}))
    }

    fn run_save_as(
        &self,
        sid: &str,
        op: &Shared,
        meter: &Meter,
        mut items: Vec<Item>,
        name: &str,
        made: &mut Made,
    ) -> Result<End, AppError> {
        let pdir = made.pdir.clone();
        let build = made.build.clone();
        ensure_layout(&build)?;
        let mut copies: HashMap<(String, bool), (Source, Source)> = HashMap::new();
        let mut n = 0usize;
        for _round in 0..5 {
            for it in &items {
                if meter.job.cancelled() {
                    return Ok(End::Cancelled);
                }
                let label = file_label(&it.src);
                meter.file_start(n, &label);
                let out = {
                    let _span = crate::trace::stage("copy:file");
                    meter.with_ctl(|ctl| copy_into_project_ctl(&it.src, &it.record, &build, None, ctl))?
                };
                let Some(out) = out else { return Ok(End::Cancelled) };
                if let Some(f) = out.created {
                    made.files.push(f);
                }
                copies.insert((it.track.clone(), it.layer), (it.record.clone(), out.source));
                meter.file_done(n, &label);
                n += 1;
            }
            // Is the project the one we copied? Tracks may have changed during the copy.
            let _io = lock(&self.inner.life.io);
            let mut g = lock(op);
            if meter.job.cancelled() || self.inner.registry.get(sid).is_err() {
                return Ok(End::Cancelled);
            }
            let mut pending = Vec::new();
            for (tname, tr) in g.project.doc.tracks() {
                for layer in [false, true] {
                    let Some(rec) = record_of(tr, layer) else { continue };
                    match copies.get(&(tname.clone(), layer)) {
                        Some((old, _)) if *old == rec => {}
                        // a layer file that was not copied because it is missing stays as it is
                        None if layer && resolve_source(Some(&rec), &g.project.dir).1 != SourceStatus::Ok => {}
                        _ => pending.push(tname.clone()),
                    }
                }
            }
            if pending.is_empty() {
                let mut doc = g.project.doc.clone();
                doc.name = Some(name.to_string());
                for (tname, tr) in doc.tracks_mut().iter_mut() {
                    for layer in [false, true] {
                        if let Some((_, new)) = copies.get(&(tname.clone(), layer)) {
                            set_record(tr, layer, rebase(new, &build, &pdir));
                        }
                    }
                }
                // the document goes into the partial folder, then the rename makes the copy a project in one step
                let mut newp = Project::new(&pdir, doc, None);
                save_project_into(&mut newp, &build)?;
                // P4-5 (L1): a destination that holds only a stale lock file (nobody holds it: the lock was tried at the
                // start) would make the rename fail as "not empty"; the lock that counts is the one in `build`, which
                // moves along. So the stale file goes first.
                if folder_is_free(&pdir) {
                    let _ = std::fs::remove_file(pdir.join(crate::lock::LOCK_FILE));
                }
                std::fs::rename(&build, &pdir).map_err(|e| {
                    AppError::conflict(format!("cannot move the finished copy to {}: {e}", pdir.display()))
                })?;
                g.project = newp;
                g.switch_lock(made.new_folder.take().and_then(crate::lifecycle::NewFolder::into_lock));
                g.mark_clean();
                g.restamp(); // #121
                for ((tname, layer), (_, new)) in &copies {
                    if *layer {
                        continue; // a layer file has no entry in the track paths: its record resolves it
                    }
                    g.paths
                        .insert(tname.clone(), rebase(new, &build, &pdir).path.map(PathBuf::from));
                    g.status.insert(tname.clone(), SourceStatus::Ok);
                }
                let (dir, pname, summary) = (g.project.dir.clone(), g.project.name(), g.summary());
                drop(g);
                drop(_io);
                self.touch_recent(&dir, &pname);
                return Ok(End::Done(summary));
            }
            // a track was added or its audio changed during the copy: copy those too
            items = Vec::new();
            for tname in pending {
                items.extend(
                    snapshot_items(&g, false, "Save As")?
                        .into_iter()
                        .filter(|i| i.track == tname),
                );
            }
        }
        Err(AppError::conflict(
            "the project changed during the copy: try Save As again",
        ))
    }

    /// `project_consolidate {sid, progress}` (route 9): copy every referenced audio file into
    /// `Audio Files/` as a job and rewrite the records at the end. Answers `{job: <state>}`;
    /// when nothing is referenced the job is `done` at once with `result`
    /// `{consolidated: [], dirty}`. The result of a finished job is `{consolidated: [track
    /// names], dirty}` as in the server.
    ///
    /// # Errors
    ///
    /// A 404 [`AppError`] for an unknown `sid`; a 409 when the project is open read-only, a referenced track's
    /// source is offline, or another copy job of the project is running; a 500 when the worker thread cannot
    /// be spawned. A failure while copying is reported in the job state, not here.
    pub fn project_consolidate(&self, sid: &str, progress: Option<Progress>) -> Result<Value, AppError> {
        let op = self.inner.registry.get_writable(sid)?;
        let (items, dir) = {
            let mut g = lock(&op);
            g.resolve(); // a file that vanished since the last look is offline now
            (snapshot_items(&g, true, "Consolidate")?, g.project.dir.clone())
        };
        let job = self.inner.copyjobs.start(sid, "consolidate")?;
        let meter = Meter::new(&job, sid, progress, items.len(), bytes_of(&items));
        if items.is_empty() {
            let dirty = lock(&op).dirty;
            meter.finish("done", Some(json!({"consolidated": [], "dirty": dirty})), None);
            return Ok(json!({"job": job.snapshot()}));
        }
        let first = job.snapshot();
        let sid = sid.to_string();
        let spawned = std::thread::Builder::new().name("consolidate".into()).spawn(move || {
            let mut created: Vec<PathBuf> = Vec::new();
            let end = run_consolidate(&op, &meter, &items, &dir, &mut created);
            match end {
                Ok(End::Done(r)) => meter.finish("done", Some(r), None),
                Ok(End::Cancelled) => {
                    for f in &created {
                        let _ = std::fs::remove_file(f);
                    }
                    meter.finish("cancelled", None, None);
                }
                Err(e) => {
                    for f in &created {
                        let _ = std::fs::remove_file(f);
                    }
                    meter.finish("error", None, Some(&e.message));
                }
            }
        });
        if let Err(e) = spawned {
            self.inner.copyjobs.forget(&sid);
            return Err(AppError::internal(format!("cannot start the copy: {e}")));
        }
        Ok(json!({"job": first}))
    }

    /// Tests only (the bridge command `test_copy_delay`): every copy job started from now on
    /// pauses `d` after each chunk it writes, so that a test can act while the job runs. The
    /// app never calls this.
    pub fn set_copy_delay(&self, d: Duration) {
        self.inner
            .copyjobs
            .delay_ms
            .store(d.as_millis() as u64, Ordering::Relaxed);
    }

    /// P4-5 (L3): wait until no analysis, waveform build or copy job of an open project is queued or running. A process
    /// that ends at once (`drumspike cli batch` at the end of its input, the MCP server when stdin closes, a one-shot
    /// command) calls this first, so a job's destination is complete and not half made. `say` gets
    /// `{"waiting": [{sid, job}]}` about once a second.
    pub fn wait_idle(&self, say: &dyn Fn(Value)) {
        let mut last = std::time::Instant::now() - Duration::from_secs(1);
        loop {
            let mut busy = Vec::new();
            for (sid, _) in self.inner.registry.all() {
                for (kind, st) in [
                    ("analysis", self.analyze_status(&sid)),
                    ("peaks", self.peaks_build_status(&sid)),
                    ("copy", self.job_status(&sid)),
                ] {
                    if matches!(st["state"].as_str(), Some("queued" | "running")) {
                        busy.push(json!({"sid": sid, "job": kind}));
                    }
                }
            }
            if busy.is_empty() {
                return;
            }
            if last.elapsed() >= Duration::from_secs(1) {
                say(json!({"waiting": busy}));
                last = std::time::Instant::now();
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    /// `job_status {sid}`: the state of the running job or of the last one.
    #[must_use]
    pub fn job_status(&self, sid: &str) -> Value {
        self.inner.copyjobs.status(sid)
    }

    /// `job_cancel {sid}`: stop the running job of the project (Save As copy, Consolidate, a
    /// copy-mode `tracks_add`, a relink search). The job removes what it made and ends with the
    /// state `cancelled`. Answers `{sid, cancelling}`. P3-15b: without a copy job, a queued or
    /// running analysis of the project is asked to stop instead (answer `{sid, cancelling:
    /// true, kind: "analyze"}`); it ends after the track in its model step, and the tracks that
    /// did not run get the error `analysis cancelled`.
    #[must_use]
    pub fn job_cancel(&self, sid: &str) -> Value {
        let v = self.inner.copyjobs.cancel(sid);
        if v["cancelling"] == true {
            return v;
        }
        if self.inner.jobs.cancel(sid) {
            return json!({"sid": sid, "cancelling": true, "kind": "analyze"});
        }
        v
    }
}

fn run_consolidate(
    op: &Shared,
    meter: &Meter,
    items: &[Item],
    dir: &Path,
    created: &mut Vec<PathBuf>,
) -> Result<End, AppError> {
    let mut copies: Vec<(String, bool, Source, Source)> = Vec::new();
    for (i, it) in items.iter().enumerate() {
        if meter.job.cancelled() {
            return Ok(End::Cancelled);
        }
        let label = file_label(&it.src);
        meter.file_start(i, &label);
        let out = {
            let _span = crate::trace::stage("copy:file");
            meter.with_ctl(|ctl| copy_into_project_ctl(&it.src, &it.record, dir, None, ctl))?
        };
        let Some(out) = out else { return Ok(End::Cancelled) };
        if let Some(f) = out.created {
            created.push(f);
        }
        copies.push((it.track.clone(), it.layer, it.record.clone(), out.source));
        meter.file_done(i, &label);
    }
    if meter.job.cancelled() {
        return Ok(End::Cancelled);
    }
    let mut g = lock(op);
    let mut changed = Vec::new();
    for (tname, layer, old, new) in &copies {
        let same = g.project.doc.track(tname).and_then(|t| record_of(t, *layer)).as_ref() == Some(old);
        if !same {
            continue; // the track was removed or got other audio (or another layer file) during the copy
        }
        if let Some(tr) = g.project.doc.track_mut(tname) {
            set_record(tr, *layer, new.clone());
        }
        if !*layer {
            g.paths.insert(tname.clone(), new.path.clone().map(PathBuf::from));
            g.status.insert(tname.clone(), SourceStatus::Ok);
        }
        if !changed.contains(tname) {
            changed.push(tname.clone());
        }
    }
    if !changed.is_empty() {
        g.mark_dirty();
    }
    Ok(End::Done(json!({"consolidated": changed, "dirty": g.dirty})))
}

// ---------- the bridge ----------

fn arg_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, AppError> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::bad(format!("missing or wrong argument `{key}` (string)")))
}

/// The progress callback of an argument `{"__channel": id}` (see `bridge.rs`).
pub(crate) fn channel_progress(args: &Value, channels: &Arc<Channels>) -> Option<Progress> {
    let id = args.get("progress")?.get("__channel")?.as_str()?.to_string();
    let ch = Arc::clone(channels);
    let p: Progress = Arc::new(move |v: Value| ch.push(&id, v));
    Some(p)
}

/// Run a command of this card for the bridge; `None` when `cmd` is not one of them.
pub fn dispatch(svc: &Service, channels: &Arc<Channels>, cmd: &str, args: &Value) -> Option<Result<Value, AppError>> {
    let run = || -> Result<Value, AppError> {
        match cmd {
            "project_consolidate" => svc.project_consolidate(arg_str(args, "sid")?, channel_progress(args, channels)),
            "job_status" => Ok(svc.job_status(arg_str(args, "sid")?)),
            "job_cancel" => Ok(svc.job_cancel(arg_str(args, "sid")?)),
            _ => Err(AppError::not_found(format!("unknown command {cmd}"))),
        }
    };
    COPYJOB_COMMANDS.contains(&cmd).then(run)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// P4-5 (L3): a stale partial folder without a lock file is locked (its file is made) before it is deleted.
    #[test]
    fn clear_stale_partial_takes_the_lock_before_it_deletes_a_folder_without_a_lock_file() {
        let d = std::env::temp_dir().join(format!("ds_copyjob_l3_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let build = d.join("Moved.partial");
        std::fs::create_dir_all(build.join("Audio Files")).unwrap();
        assert!(!build.join(crate::lock::LOCK_FILE).exists());
        let svc = Service::new(crate::ServiceConfig {
            refine_rule: None,
            bleed_rule: None,
            flam_rule: None,
            cross_bleed: None,
            ghost_gate: None,
        });
        svc.set_lock_client("app");
        let mut seen = None;
        svc.clear_stale_partial_with(&build, || {
            seen = Some(
                crate::lock::ProjectLock::acquire(&build, "cli")
                    .map(|_| ())
                    .map_err(|e| e.code),
            );
        })
        .unwrap();
        assert_eq!(seen, Some(Err(409)), "the folder was locked when it was deleted");
        assert!(!build.exists());
        // without a lock client (the tests) it is removed as it is
        std::fs::create_dir_all(&build).unwrap();
        let plain = Service::new(crate::ServiceConfig {
            refine_rule: None,
            bleed_rule: None,
            flam_rule: None,
            cross_bleed: None,
            ghost_gate: None,
        });
        plain.clear_stale_partial(&build).unwrap();
        assert!(!build.exists());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn one_job_per_project_and_the_cancel_flag() {
        let jobs = CopyJobs::default();
        assert_eq!(jobs.status("a")["state"], "idle");
        assert_eq!(jobs.cancel("a")["cancelling"], false);
        let j = jobs.start("a", "consolidate").unwrap();
        assert_eq!(jobs.start("a", "save_as").unwrap_err().code, 409);
        assert!(jobs.start("b", "save_as").is_ok(), "another project is free");
        assert!(!j.cancelled());
        let c = jobs.cancel("a");
        assert_eq!(c["cancelling"], true);
        assert_eq!(c["kind"], "consolidate");
        assert!(j.cancelled());
        let m = Meter::new(&j, "a", None, 1, 10);
        m.finish("cancelled", None, None);
        assert_eq!(jobs.status("a")["state"], "cancelled");
        assert_eq!(jobs.cancel("a")["cancelling"], false, "a finished job is not cancelled");
        assert!(
            jobs.start("a", "save_as").is_ok(),
            "a finished job does not block the next"
        );
    }

    #[test]
    fn the_meter_counts_files_and_throttles_bytes() {
        let jobs = CopyJobs::default();
        let j = jobs.start("a", "save_as").unwrap();
        let seen = Arc::new(Mutex::new(Vec::<Value>::new()));
        let s2 = Arc::clone(&seen);
        let p: Progress = Arc::new(move |v| lock(&s2).push(v));
        let m = Meter::new(&j, "a", Some(p), 2, 20);
        m.file_start(0, "a.wav");
        for _ in 0..1000 {
            m.bytes(1); // far faster than every 100 ms: at most a few messages
        }
        m.file_done(0, "a.wav");
        m.file_start(1, "b.wav");
        m.file_done(1, "b.wav");
        m.finish("done", Some(json!({"x": 1})), None);
        let v = lock(&seen);
        assert!(v.len() >= 5 && v.len() < 20, "{}", v.len()); // 2 starts, 2 ends, the final one; bytes are throttled
        assert_eq!(v[0]["copy"]["file"], "a.wav");
        let last = v.last().unwrap();
        assert_eq!(last["copy"]["state"], "done");
        assert_eq!(last["copy"]["done"], 2);
        assert_eq!(last["copy"]["result"]["x"], 1);
        assert_eq!(last["sid"], "a");
        assert_eq!(m.sent(), v.len());
    }

    #[test]
    fn a_meter_dropped_while_running_marks_the_job_as_an_error() {
        let jobs = CopyJobs::default();
        let j = jobs.start("a", "save_as").unwrap();
        drop(Meter::new(&j, "a", None, 1, 1));
        assert_eq!(jobs.status("a")["state"], "error");
    }

    #[test]
    fn forget_cancels_and_removes() {
        let jobs = CopyJobs::default();
        let j = jobs.start("a", "save_as").unwrap();
        jobs.forget("a");
        assert!(j.cancelled());
        assert_eq!(jobs.status("a")["state"], "idle");
    }

    #[test]
    fn dispatch_ignores_other_commands() {
        let svc = Service::new(crate::ServiceConfig {
            refine_rule: None,
            bleed_rule: None,
            flam_rule: None,
            cross_bleed: None,
            ghost_gate: None,
        });
        let ch = Arc::new(Channels::default());
        assert!(dispatch(&svc, &ch, "project_open", &json!({})).is_none());
        assert_eq!(
            dispatch(&svc, &ch, "job_status", &json!({"sid": "x"}))
                .unwrap()
                .unwrap()["state"],
            "idle"
        );
        assert_eq!(
            dispatch(&svc, &ch, "job_cancel", &json!({"sid": "x"}))
                .unwrap()
                .unwrap()["cancelling"],
            false
        );
        assert!(dispatch(&svc, &ch, "job_status", &json!({})).unwrap().is_err());
    }
}
