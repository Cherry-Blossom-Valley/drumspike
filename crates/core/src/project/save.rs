//! Saving: the atomic document write, the pre-save backup with its rotation of ten,
//! `save_project` and `save_as`. The port of the `save` section of `project.py`.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{Local, TimeZone};

use super::document::{now, Document, FORMAT};
use super::source::{build_source, copy_into_project, resolve_source, splitext, SourceStatus};
use super::{
    abspath, check_name, document_path, ensure_layout, io_err, py_repr, Project, ProjectError, BACKUPS_DIR, DOC_EXT,
};

/// How many `.drumspike` files `Backups/` keeps (`project.BACKUPS_KEEP`).
pub const BACKUPS_KEEP: usize = 10;

/// `project.free_name`: `<base>` or `<stem> (2).<ext>`, `(3)`, ... the first that does not
/// exist in `dir`.
pub fn free_name(dir: &Path, base: &str) -> String {
    let (stem, ext) = splitext(base);
    let mut cand = base.to_string();
    let mut k = 1;
    while dir.join(&cand).exists() {
        k += 1;
        cand = format!("{stem} ({k}){ext}");
    }
    cand
}

/// `project.backup_name`: `<name> YYYY-MM-DD HH-MM-SS.drumspike` in local time
/// (`time.strftime('%Y-%m-%d %H-%M-%S', time.localtime(when))`); `when` is Unix seconds,
/// `None` for now. The name becomes one path component (finding G1): `/`, `\`, `:` and NUL
/// turn into `-`, and a name that is `.` or `..` is refused ("bad project name"), so a
/// document name can never move the backup out of `Backups/`.
pub fn backup_name(name: &str, when: Option<f64>) -> Result<String, ProjectError> {
    let safe: String = name
        .chars()
        .map(|c| if matches!(c, '/' | '\\' | ':' | '\0') { '-' } else { c })
        .collect();
    if safe == "." || safe == ".." {
        return Err(ProjectError::invalid(format!("bad project name: {}", py_repr(name))));
    }
    let secs = when.unwrap_or_else(now).floor() as i64;
    let stamp = match Local.timestamp_opt(secs, 0).single() {
        Some(t) => t.format("%Y-%m-%d %H-%M-%S").to_string(),
        None => "1970-01-01 00-00-00".to_string(),
    };
    Ok(format!("{safe} {stamp}{DOC_EXT}"))
}

/// `project.rotate_backups`: keep the newest `keep` documents in `Backups/` (by mtime,
/// then name) and remove the rest; returns the removed paths. A missing folder is a no-op.
pub fn rotate_backups(project_dir: &Path, keep: usize) -> Result<Vec<PathBuf>, ProjectError> {
    let bdir = project_dir.join(BACKUPS_DIR);
    if !bdir.is_dir() {
        return Ok(Vec::new());
    }
    let mut docs: Vec<(SystemTime, PathBuf)> = std::fs::read_dir(&bdir)
        .map_err(io_err(format!("list {}", bdir.display())))?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().ends_with(DOC_EXT))
        .map(|e| {
            let p = e.path();
            let m = e
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            (m, p)
        })
        .collect();
    docs.sort();
    let n_remove = docs.len().saturating_sub(keep);
    let mut removed = Vec::with_capacity(n_remove);
    for (_, p) in docs.into_iter().take(n_remove) {
        std::fs::remove_file(&p).map_err(io_err(format!("remove {}", p.display())))?;
        removed.push(p);
    }
    Ok(removed)
}

/// `project.write_document`: atomic write through the shared helper (`io::atomic`): a unique
/// `<path>.<pid>-<unpredictable>.tmp` file in the same folder, synced to disk, renamed over
/// `path`, the folder fsynced (R10). The layout is Python's `json.dump(doc, f, indent=1)`.
pub fn write_document(doc: &Document, path: &Path) -> Result<(), ProjectError> {
    let mut t = crate::io::TempFile::create(path).map_err(io_err(format!("create {}", path.display())))?;
    let text = doc.to_json_pretty();
    t.write_all(text.as_bytes())
        .map_err(io_err(format!("write {}", t.path().display())))?;
    let tmp = t.path().display().to_string();
    t.commit()
        .map_err(io_err(format!("rename {tmp} to {}", path.display())))
}

/// `shutil.copy2` for the backup fallback: bytes plus the modification time.
fn copy_with_mtime(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::copy(src, dst)?;
    if let Ok(m) = std::fs::metadata(src).and_then(|m| m.modified()) {
        let _ = File::options().write(true).open(dst).and_then(|f| f.set_modified(m));
    }
    Ok(())
}

/// The result of a save whose document is committed: the path on disk and, when the
/// post-commit housekeeping (the backup rotation) failed after the rename, its error
/// as a warning (R12). A failure before the commit is still an `Err`.
pub struct SaveReport {
    /// The document path that is on disk.
    pub path: PathBuf,
    /// The rotation failure, when the housekeeping after the commit failed.
    pub warning: Option<ProjectError>,
}

/// `project.save_project`: set `format`, `name` and `modified`, write the document
/// atomically; the previous document is hard-linked (copied where links fail) into
/// `Backups/<Name> <timestamp>.drumspike` first, then the folder is rotated to
/// [`BACKUPS_KEEP`]. Returns the document path. A rotation failure after the commit
/// is an error here (the strict contract of [`save_project`]); use
/// [`save_project_report`] to get the committed path with the warning instead.
pub fn save_project(project: &mut Project) -> Result<PathBuf, ProjectError> {
    let report = save_project_with(project, |a, b| std::fs::hard_link(a, b))?;
    match report.warning {
        Some(e) => Err(e),
        None => Ok(report.path),
    }
}

/// [`save_project`] with the R12 policy: a backup-rotation failure after the document
/// is renamed into place does not undo the commit; it comes back as a `warning` next
/// to the path. A failure to make the pre-save backup or to write the document is
/// still an `Err` (nothing is committed, no baseline moves).
pub fn save_project_report(project: &mut Project) -> Result<SaveReport, ProjectError> {
    save_project_with(project, |a, b| std::fs::hard_link(a, b))
}

/// [`save_project`] with the hard-link step as a parameter: the tests pass a link that
/// fails to reach the copy fallback (the Python test patches `os.link`). The rotation
/// failure after the commit rides as [`SaveReport::warning`].
pub(crate) fn save_project_with(
    project: &mut Project,
    link: impl Fn(&Path, &Path) -> std::io::Result<()>,
) -> Result<SaveReport, ProjectError> {
    let name = project.name();
    project.doc.format = FORMAT;
    project.doc.name = Some(name.clone());
    project.doc.modified = Some(Some(now()));
    ensure_layout(&project.dir)?;
    let path = project.path();
    if path.is_file() {
        let bdir = project.subdir(BACKUPS_DIR);
        let bk = bdir.join(free_name(&bdir, &backup_name(&name, None)?));
        if link(&path, &bk).is_err() {
            copy_with_mtime(&path, &bk).map_err(io_err(format!("backup {} to {}", path.display(), bk.display())))?;
        }
    }
    write_document(&project.doc, &path)?;
    // R12: the document is committed from here. A failing rotation is housekeeping,
    // not a failed save: the path is real and the caller updates its baseline.
    let warning = rotate_backups(&project.dir, BACKUPS_KEEP).err();
    Ok(SaveReport { path, warning })
}

/// Write the document of `project` into the folder `dir` instead of its own folder (same file name as
/// `project.path()`), setting `format`, `name` and `modified` as [`save_project`] does, without backups. For a copy that is
/// built in `<name>.partial` and renamed to `<name>` when it is complete (P4-5): the document is in place before the rename.
pub fn save_project_into(project: &mut Project, dir: &Path) -> Result<PathBuf, ProjectError> {
    let name = project.name();
    project.doc.format = FORMAT;
    project.doc.name = Some(name);
    project.doc.modified = Some(Some(super::document::now()));
    ensure_layout(dir)?;
    let file = project
        .path()
        .file_name()
        .map(PathBuf::from)
        .ok_or_else(|| ProjectError::invalid("project without a document name"))?;
    let path = dir.join(file);
    write_document(&project.doc, &path)?;
    Ok(path)
}

/// `project.save_as`: write the project into a new folder `<dir>/<name>/` and return the
/// new project. `copy_audio` copies every source into the new `Audio Files/`; otherwise
/// every source is referenced where it is now (a copied file of the old project becomes a
/// referenced file of the new one). An offline source is an error before anything is written
/// to the new folder's document.
pub fn save_as(project: &Project, dir: &Path, name: &str, copy_audio: bool) -> Result<Project, ProjectError> {
    let name = check_name(name)?;
    let pdir = abspath(dir).join(&name);
    if document_path(&pdir).exists() {
        return Err(ProjectError::Exists(format!(
            "project already exists: {}",
            pdir.display()
        )));
    }
    ensure_layout(&pdir)?;
    let mut doc = project.doc.clone();
    doc.name = Some(name);
    if let Some(tracks) = doc.tracks.as_mut() {
        for (tname, tr) in tracks.iter_mut() {
            let rec = match tr.source_ref() {
                Some(r) => r.clone(),
                None => continue,
            };
            let (src, status) = resolve_source(Some(&rec), &project.dir);
            let src = match (src, status) {
                (Some(s), SourceStatus::Ok) => s,
                (_, status) => {
                    return Err(ProjectError::NeedsRelink {
                        track: tname.to_string(),
                        reason: status.as_str().to_string(),
                    })
                }
            };
            let referenced = build_source(&src, &pdir, "referenced")?;
            tr.source = Some(Some(if copy_audio {
                copy_into_project(&referenced, &pdir, None)?
            } else {
                referenced
            }));
        }
    }
    let mut newp = Project::new(&pdir, doc, None);
    save_project(&mut newp)?;
    Ok(newp)
}

#[cfg(test)]
mod tests;
