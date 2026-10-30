//! The importer of old server sessions (P3-4e). The zip import and export of projects was removed in P4-30 (owner
//! decision 2026-10-14: the project zips are not needed). The commands keep the results of the server's importer
//! (`server.import_legacy_session`):
//!
//! | Command | Server route (route map row) |
//! |---|---|
//! | `legacy_list {folder}` | GET `/api/project/legacy` (3) |
//! | `project_import {session_dir}` | POST `/api/project/import` (13) |
//!
//! A session folder of the old server (`meta.json` and `tracks/<name>/{events.json, source.wav}`) opens as a new
//! project folder in the default projects folder; its audio is copied into `Audio Files/<track>.wav`. The input is
//! only read. A folder made by a failed import is removed again.

use std::path::{Path, PathBuf};

use drumspike_core::Project;
use drumspike_core::project::{
    BACKUPS_DIR, abspath, copy_into_project, ensure_layout, legacy_session_document, migrate_forward, save_project,
};
use serde_json::{Value, json};

use crate::Service;
use crate::error::AppError;
use crate::lifecycle::unique_folder;
use crate::registry::lock;

/// The commands of this card, as the route map names them.
pub const IMPORT_COMMANDS: [&str; 2] = ["legacy_list", "project_import"];

/// `server.folder_name`: a project name as a folder name. Runs of `/`, `\`, `:` and NUL become
/// one `-`; the result is stripped of white space and dots, cut to 80 characters, and
/// `Imported project` when nothing is left.
pub fn folder_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut in_run = false;
    for c in name.chars() {
        if matches!(c, '/' | '\\' | ':' | '\0') {
            if !in_run {
                out.push('-');
            }
            in_run = true;
        } else {
            out.push(c);
            in_run = false;
        }
    }
    let s: String = out.trim().trim_matches('.').chars().take(80).collect();
    if s.is_empty() {
        "Imported project".to_string()
    } else {
        s
    }
}

impl Service {
    /// Make the new project folder `<projects>/<folder name>` (a number is added when it exists).
    fn new_import_folder(&self, name: &str) -> Result<PathBuf, AppError> {
        let base = self.projects_dir();
        std::fs::create_dir_all(&base).map_err(|e| AppError::internal(format!("{}: {e}", base.display())))?;
        let pdir = base.join(unique_folder(&base, &folder_name(name)));
        ensure_layout(&pdir)?;
        Ok(pdir)
    }

    /// Register a saved project, put it in the recent list, answer its summary.
    fn finish_import(&self, project: Project) -> Result<Value, AppError> {
        let (dir, name) = (project.dir.clone(), project.name());
        let op = self.register_project(project, false)?;
        self.touch_recent(&dir, &name);
        let g = lock(&op);
        Ok(g.summary())
    }

    /// `GET /api/project/legacy`: the sessions of the old server in `folder`: every sub-folder
    /// with a readable `meta.json` (sorted by name), as `{session_id, name, tracks, dir}`. When
    /// `folder` holds a `meta.json` itself, it is the one session. 404 when `folder` is no folder.
    pub fn legacy_list(&self, folder: &str) -> Result<Value, AppError> {
        let root = Path::new(folder);
        if folder.trim().is_empty() || !root.is_dir() {
            return Err(AppError::not_found(format!("no such folder: {folder}")));
        }
        let row = |dir: &Path| -> Option<Value> {
            let text = std::fs::read_to_string(dir.join("meta.json")).ok()?;
            let m: Value = serde_json::from_str(&text).ok()?;
            let m = m.as_object()?;
            let id = dir.file_name()?.to_string_lossy().into_owned();
            let tracks: Vec<Value> = m
                .get("tracks")
                .and_then(Value::as_object)
                .map(|t| t.keys().map(|k| Value::from(k.as_str())).collect())
                .unwrap_or_default();
            Some(json!({
                "session_id": id,
                "name": m.get("name").cloned().unwrap_or_else(|| Value::from(id.as_str())),
                "tracks": tracks,
                "dir": dir.display().to_string(),
            }))
        };
        if root.join("meta.json").is_file() {
            return Ok(Value::Array(row(&abspath(root)).into_iter().collect()));
        }
        if abspath(root)
            .ancestors()
            .any(|a| a.to_string_lossy().ends_with(".partial"))
        {
            return Ok(Value::Array(Vec::new())); // P4-5 (M1): inside an unfinished Save As
        }
        let mut dirs: Vec<PathBuf> = std::fs::read_dir(root)
            .map_err(|e| AppError::internal(format!("{folder}: {e}")))?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.is_dir() && !p.to_string_lossy().ends_with(".partial")) // P4-5 (M1): unfinished Save As
            .collect();
        dirs.sort();
        Ok(Value::Array(dirs.iter().filter_map(|d| row(&abspath(d))).collect()))
    }

    /// `POST /api/project/import`: open a session folder of the old server (`session_dir`) as a new project.
    /// Answers the summary of `project_open`. 400 for an empty `session_dir`, 404 for a folder that is not there.
    pub fn project_import(&self, session_dir: &str) -> Result<Value, AppError> {
        let _span = crate::trace::stage("import");
        let dir = session_dir.trim();
        if dir.is_empty() {
            return Err(AppError::bad("session_dir required"));
        }
        let dir = Path::new(dir);
        if !dir.is_dir() {
            return Err(AppError::not_found(format!(
                "no such session folder: {}",
                dir.display()
            )));
        }
        self.finish_import(self.import_legacy_session(dir)?)
    }

    /// `server.import_legacy_session`: a session folder to a new project folder with copied
    /// audio. The session is only read; a failed import removes the new folder.
    fn import_legacy_session(&self, session_dir: &Path) -> Result<Project, AppError> {
        let doc = migrate_forward(legacy_session_document(session_dir)?)?;
        let pdir = self.new_import_folder(doc.name.as_deref().unwrap_or(""))?;
        let run = || -> Result<Project, AppError> {
            let mut p = Project::new(&pdir, doc, None);
            let names: Vec<String> = p.doc.tracks().map(|(n, _)| n.clone()).collect();
            for tname in names {
                let rec = p.doc.track(&tname).and_then(|t| t.source_ref().cloned());
                if let Some(rec) = rec {
                    let copied = copy_into_project(&rec, &pdir, Some(&format!("{tname}.wav")))?;
                    if let Some(t) = p.doc.track_mut(&tname) {
                        t.source = Some(Some(copied));
                    }
                }
                let prev = session_dir.join("tracks").join(&tname).join("events.json.prev.json");
                if prev.is_file() {
                    let dst = pdir.join(BACKUPS_DIR).join(format!("{tname}.events.prev.json"));
                    std::fs::copy(&prev, &dst)
                        .map_err(|e| AppError::internal(format!("copy {}: {e}", prev.display())))?;
                }
            }
            save_project(&mut p)?;
            Ok(p)
        };
        run().inspect_err(|_| {
            let _ = std::fs::remove_dir_all(&pdir);
        })
    }
}

// ---------- the bridge ----------

fn opt_str<'a>(args: &'a Value, key: &str) -> Result<Option<&'a str>, AppError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_str()
            .map(Some)
            .ok_or_else(|| AppError::bad(format!("wrong argument `{key}` (string)"))),
    }
}

fn arg_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, AppError> {
    opt_str(args, key)?.ok_or_else(|| AppError::bad(format!("missing or wrong argument `{key}` (string)")))
}

/// Run a command of this card for the bridge; `None` when `cmd` is not one of them. The
/// argument names are those of the Tauri wrappers in `apps/desktop/src-tauri/src/import.rs`.
pub fn dispatch(svc: &Service, cmd: &str, args: &Value) -> Option<Result<Value, AppError>> {
    let run = || -> Result<Value, AppError> {
        match cmd {
            "legacy_list" => svc.legacy_list(arg_str(args, "folder")?),
            "project_import" => svc.project_import(arg_str(args, "session_dir")?),
            _ => Err(AppError::not_found(format!("unknown command {cmd}"))),
        }
    };
    IMPORT_COMMANDS.contains(&cmd).then(run)
}

#[cfg(test)]
mod tests;
