//! Opening from outside the window (P3-4b): a Finder double-click on a `.drumspike` document,
//! a drop on the Dock icon (`RunEvent::Opened`), `open -a <app> <path>`, and a drop of a folder or a
//! document on the window (`DragDropEvent::Drop`). All paths wait in a queue and the page gets the event
//! `open-paths` as a nudge; it takes the queue with the command `open_paths_take`. The queue also holds
//! the paths that arrive before the page has loaded (a cold start with `open -a`), so none is lost.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager, Runtime};

/// The document extension without the dot.
pub const DOC_EXT: &str = "drumspike";

/// The paths that wait for the page.
#[derive(Default)]
pub struct OpenQueue(pub Mutex<Vec<String>>);

/// A project folder is a folder with one `.drumspike` document (or one named like the folder).
fn is_project_folder(dir: &Path) -> bool {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return false;
    };
    let docs = rd
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == DOC_EXT) && e.path().is_file())
        .count();
    docs == 1
        || dir
            .file_name()
            .is_some_and(|n| dir.join(format!("{}.{DOC_EXT}", n.to_string_lossy())).is_file())
}

/// Sort `paths` for the page: `projects` are project folders (a document stands for its folder, a
/// folder counts when it holds a document), `others` is everything else (audio files and folders for
/// Add Tracks in P3-4c, backup documents, files that do not exist). Order and first occurrence stay.
pub fn classify(paths: &[String]) -> Value {
    let mut projects: Vec<String> = Vec::new();
    let mut others: Vec<String> = Vec::new();
    for p in paths {
        let path = Path::new(p);
        let is_doc = path.is_file() && path.extension().is_some_and(|x| x.eq_ignore_ascii_case(DOC_EXT));
        let folder: Option<PathBuf> = if is_doc {
            path.parent()
                .filter(|d| d.file_name().is_some_and(|n| n != "Backups"))
                .map(Path::to_path_buf)
        } else if path.is_dir() && is_project_folder(path) {
            Some(path.to_path_buf())
        } else {
            None
        };
        match folder {
            Some(f) => {
                let f = f.display().to_string();
                if !projects.contains(&f) {
                    projects.push(f);
                }
            }
            None => {
                if !others.contains(p) {
                    others.push(p.clone());
                }
            }
        }
    }
    json!({"projects": projects, "others": others})
}

/// Queue `paths` and tell the page. Called for every way in: `RunEvent::Opened`, a window drop and
/// the self-test.
pub fn push<R: Runtime>(app: &AppHandle<R>, paths: Vec<String>) {
    if paths.is_empty() {
        return;
    }
    if let Some(q) = app.try_state::<OpenQueue>() {
        if let Ok(mut l) = q.0.lock() {
            l.extend(paths.iter().cloned());
        }
    }
    #[cfg(debug_assertions)]
    log_for_tests(&paths);
    let _ = app.emit("open-paths", json!({"n": paths.len()}));
}

/// P4-7a: the window position (logical pixels) of the last drop on the window; a drop on the
/// Inspector's Layer section sets the layer of the selected track instead of adding tracks.
static LAST_DROP: Mutex<Option<(f64, f64)>> = Mutex::new(None);

/// [`push`] for a drop on the window at `pos` (logical pixels of the window).
pub fn push_drop<R: Runtime>(app: &AppHandle<R>, paths: Vec<String>, pos: (f64, f64)) {
    if let Ok(mut l) = LAST_DROP.lock() {
        *l = Some(pos);
    }
    push(app, paths);
}

/// Take the queue (the page does this on the nudge and once at start-up). P4-7a: `pos` is the
/// position of the drop that brought the paths (`null` for a Finder or Dock open).
pub fn take<R: Runtime>(app: &AppHandle<R>) -> Value {
    let paths: Vec<String> = app
        .try_state::<OpenQueue>()
        .and_then(|q| q.0.lock().ok().map(|mut l| std::mem::take(&mut *l)))
        .unwrap_or_default();
    let mut out = classify(&paths);
    let pos = LAST_DROP.lock().ok().and_then(|mut l| l.take());
    out["pos"] = pos.map_or(Value::Null, |(x, y)| json!({"x": x, "y": y}));
    out
}

/// Debug builds only: with `DRUMSPIKE_OPEN_LOG=<file>` every path that arrives is appended to the file,
/// so a cold `open -a <app> <document>` can be checked without the page.
#[cfg(debug_assertions)]
fn log_for_tests(paths: &[String]) {
    use std::io::Write;
    if let Some(f) = std::env::var_os("DRUMSPIKE_OPEN_LOG") {
        if let Ok(mut h) = std::fs::OpenOptions::new().create(true).append(true).open(f) {
            for p in paths {
                let _ = writeln!(h, "{p}");
            }
        }
    }
}

/// The file paths of the URLs of `RunEvent::Opened` (other schemes are not paths).
pub fn paths_of_urls<'a>(urls: impl IntoIterator<Item = &'a tauri::Url>) -> Vec<String> {
    urls.into_iter()
        .filter(|u| u.scheme() == "file")
        .filter_map(|u| u.to_file_path().ok())
        .map(|p| p.display().to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ds_open_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn s(p: &Path) -> String {
        p.display().to_string()
    }

    #[test]
    fn a_document_stands_for_its_folder_and_a_folder_with_a_document_is_a_project() {
        let root = tmp("classify");
        let proj = root.join("Song");
        std::fs::create_dir_all(proj.join("Backups")).unwrap();
        std::fs::write(proj.join("Song.drumspike"), b"{}").unwrap();
        std::fs::write(proj.join("Backups/Song 2026-10-11 10-00-00.drumspike"), b"{}").unwrap();
        let audio = root.join("Audio");
        std::fs::create_dir_all(&audio).unwrap();
        std::fs::write(audio.join("Kick.wav"), b"x").unwrap();
        let v = classify(&[
            s(&proj.join("Song.drumspike")),
            s(&proj),
            s(&audio),
            s(&audio.join("Kick.wav")),
            s(&proj.join("Backups/Song 2026-10-11 10-00-00.drumspike")),
            s(&root.join("missing.drumspike")),
        ]);
        assert_eq!(v["projects"], json!([s(&proj)]));
        assert_eq!(
            v["others"],
            json!([
                s(&audio),
                s(&audio.join("Kick.wav")),
                s(&proj.join("Backups/Song 2026-10-11 10-00-00.drumspike")),
                s(&root.join("missing.drumspike")),
            ])
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_folder_with_two_documents_needs_one_named_like_the_folder() {
        let root = tmp("two");
        let two = root.join("Two");
        std::fs::create_dir_all(&two).unwrap();
        std::fs::write(two.join("A.drumspike"), b"{}").unwrap();
        std::fs::write(two.join("B.drumspike"), b"{}").unwrap();
        assert_eq!(classify(&[s(&two)])["projects"], json!([]));
        std::fs::write(two.join("Two.drumspike"), b"{}").unwrap();
        assert_eq!(classify(&[s(&two)])["projects"], json!([s(&two)]));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_extension_check_ignores_case_and_a_path_comes_once() {
        let root = tmp("case");
        let p = root.join("X");
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(p.join("X.DRUMSPIKE"), b"{}").unwrap();
        let v = classify(&[s(&p.join("X.DRUMSPIKE")), s(&p.join("X.DRUMSPIKE")), s(&p)]);
        assert_eq!(v["projects"], json!([s(&p)]));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn file_urls_become_paths_and_other_schemes_are_dropped() {
        let urls: Vec<tauri::Url> = [
            "file:///tmp/A%20B/x.drumspike",
            "https://example.org/x",
            "file:///tmp/y",
        ]
        .iter()
        .map(|u| u.parse().unwrap())
        .collect();
        assert_eq!(paths_of_urls(&urls), ["/tmp/A B/x.drumspike", "/tmp/y"]);
    }

    #[test]
    fn an_empty_list_classifies_to_two_empty_lists() {
        assert_eq!(classify(&[]), json!({"projects": [], "others": []}));
    }
}
