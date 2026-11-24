//! M1c1 (#560): what leaves the plugin: the song time base of the export, the one folder exception of the rule of
//! #532, the folder sheet and the native file drag.
//!
//! The design is `docs/design/daw-plugin/m1c1-delivery.md`. This file holds the pure parts (the file stems, the
//! check of a `dest`, the check of a drag path) and the two parts that touch AppKit (the sheet and the drag session,
//! both on the host's main thread only).

use crate::editor::TrackInfoFn;
use crate::tempo_ara::Answer;
use drumspike_app::export_song::{ExportTime, SongRegion, SongTrack};
use drumspike_core::tempo::TempoMap;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// One playback region for the export: the slice of the source `[start_mod, start_mod + dur_mod)` (from the round of the
/// analysis, so it belongs to the hits) sits in the song from `start_play` for `dur_play` seconds (live).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExportRegion {
    pub start_mod: f64,
    pub dur_mod: f64,
    pub start_play: f64,
    pub dur_play: f64,
}

pub type ExportRegionsFn = Arc<dyn Fn() -> ExportRegions + Send + Sync>;

/// The regions of every source for the export, and the sources whose clips are all gone.
#[derive(Debug, Clone, Default)]
pub struct ExportRegions {
    pub regions: Vec<(usize, ExportRegion)>,
    /// Sources that had regions at the last round and have none now (every clip was deleted in the DAW): every hit of
    /// such a source is left out and counted. A source that never had a region keeps the identity time base.
    pub gone: Vec<usize>,
}

/// The regions of one source for the export, and whether the source is gone. The hits are in source time, so the LIVE
/// regions alone give the whole mapping (live slice, live position in the song, live length): a region the user trimmed
/// or moved in the DAW needs no new analysis round. A source with NO live region but regions at the round is gone (every
/// hit is left out); one that never had a region is the identity. `stale` (the fault run) takes the round alone, as the
/// notes of #532 do, so a moved clip exports at its old place. `lens` is the length in the song by region id.
#[must_use]
pub fn merge_regions(
    round: Option<&[crate::service::RegionRec]>,
    live: &[(u64, crate::service::RegionRec)],
    lens: &HashMap<u64, f64>,
    stale: bool,
) -> (Vec<ExportRegion>, bool) {
    let from_round = |round: &[crate::service::RegionRec]| -> Vec<ExportRegion> {
        round
            .iter()
            .map(|r| ExportRegion {
                start_mod: r.start_mod,
                dur_mod: r.dur_mod,
                start_play: r.start_play,
                dur_play: r.dur_mod,
            })
            .collect()
    };
    if stale {
        return (round.map(from_round).unwrap_or_default(), false);
    }
    if live.is_empty() {
        return (Vec::new(), round.is_some_and(|r| !r.is_empty()));
    }
    (
        live.iter()
            .map(|(id, r)| ExportRegion {
                start_mod: r.start_mod,
                dur_mod: r.dur_mod,
                start_play: r.start_play,
                dur_play: lens.get(id).copied().unwrap_or(r.dur_mod),
            })
            .collect(),
        false,
    )
}

type NamesFn = Arc<dyn Fn() -> HashMap<String, String> + Send + Sync>;
type TempoFn = Arc<dyn Fn() -> Answer + Send + Sync>;

/// The song time base of the host for the export of one document.
pub struct HostTime {
    pub regions: ExportRegionsFn,
    pub track_info: TrackInfoFn,
    pub names: NamesFn,
    pub tempo: TempoFn,
}

impl ExportTime for HostTime {
    fn track(&self, name: &str) -> Option<SongTrack> {
        let fault = crate::test_switch("DS_ARA_DELIVERY_FAULT").unwrap_or_default();
        let (seqs, _) = (self.track_info)(name);
        let names = (self.names)();
        let stem = file_stems(&names).get(name).cloned();
        if seqs.is_empty() {
            // no source of the host for this track: its files keep the host's name and every hit is left out
            return Some(SongTrack {
                regions: Vec::new(),
                stem,
                hide_all: true,
            });
        }
        let all = (self.regions)();
        let regions = if fault == "source-time" {
            // the fault run: the regions are left out, so the file keeps the time of the source
            Vec::new()
        } else {
            all.regions
                .into_iter()
                .filter(|(i, _)| seqs.contains(i))
                .map(|(_, r)| SongRegion {
                    start_mod: r.start_mod,
                    dur_mod: r.dur_mod,
                    start_play: r.start_play,
                    dur_play: r.dur_play,
                })
                .collect()
        };
        // every source of the track lost all its clips: every hit is left out and counted, none goes out in source time
        let gone = fault != "source-time" && regions.is_empty() && seqs.iter().all(|i| all.gone.contains(i));
        Some(SongTrack {
            regions,
            stem,
            hide_all: gone,
        })
    }

    fn tempo(&self) -> Option<TempoMap> {
        (self.tempo)().map
    }
}

/// A DAW track name as a file stem: the characters that a file name cannot hold (the rule of the app's
/// `multi_file_name`: control characters and `/ \ : * ? " < > |`) become `-`; leading dots go (no hidden file, no
/// `..`); at most [`STEM_BYTES`] bytes; empty gives `None`.
/// The longest file stem in bytes (a file name holds 255; the suffix and a number need room).
pub const STEM_BYTES: usize = 80;

#[must_use]
pub fn safe_stem(name: &str) -> Option<String> {
    let n: String = name
        .chars()
        .map(|c| {
            if c.is_control() || "/\\:*?\"<>|".contains(c) {
                '-'
            } else {
                c
            }
        })
        .collect();
    let n = n.trim().trim_start_matches('.').trim();
    // at most 80 bytes of UTF-8, cut on a character boundary
    let mut end = n.len().min(STEM_BYTES);
    while !n.is_char_boundary(end) {
        end -= 1;
    }
    let n = n[..end].trim_end();
    (!n.is_empty()).then(|| n.to_string())
}

/// The file stem of every track: its display name made safe; the key when the name gives none. Two tracks that
/// end on one stem (compared by the app's `file_name_key`, the key the plan check uses) number the later one by the
/// order of the keys.
#[must_use]
pub fn file_stems(display: &HashMap<String, String>) -> HashMap<String, String> {
    let mut keys: Vec<&String> = display.keys().collect();
    keys.sort();
    let mut seen: Vec<String> = Vec::new();
    let mut out = HashMap::new();
    for k in keys {
        let base = safe_stem(&display[k])
            .or_else(|| safe_stem(k))
            .unwrap_or_else(|| "track".to_string());
        let mut stem = base.clone();
        let mut n = 2;
        while seen.contains(&drumspike_app::export::file_name_key(&stem)) {
            stem = format!("{base} ({n})");
            n += 1;
        }
        seen.push(drumspike_app::export::file_name_key(&stem));
        out.insert(k.clone(), stem);
    }
    out
}

/// The one exception to the rule of #532: a `dest` that is a folder the user chose in the sheet of this editor.
/// `chosen` holds canonical paths. The command must be one that takes a `dest` for an export, the `dest` must be
/// absolute, and its canonical form must EQUAL an entry (no prefix, no subfolder): a folder that was replaced by a
/// link after the choice no longer equals. A `path` key never passes.
///
/// This is the rule of M1c1 with no default folder; [`dest_allowed_in`] adds the default folder of the document.
#[must_use]
pub fn dest_allowed(chosen: &[PathBuf], cmd: &str, args: &Value) -> bool {
    dest_allowed_in(chosen, None, cmd, args)
}

/// The path with the links of its existing part resolved, also when the end of it does not exist yet: the deepest
/// existing ancestor is made canonical and the missing names are put behind it. `None` for a relative path, and for a
/// path with a `..` in its missing part (it cannot be resolved without the folder). Two paths of one place give one answer.
#[must_use]
pub fn resolve_lenient(p: &Path) -> Option<PathBuf> {
    if !p.is_absolute() {
        return None;
    }
    let mut missing: Vec<std::ffi::OsString> = Vec::new();
    let mut cur = p.to_path_buf();
    loop {
        match std::fs::canonicalize(&cur) {
            Ok(mut c) => {
                for name in missing.iter().rev() {
                    c.push(name);
                }
                return Some(c);
            }
            Err(_) => {
                // a `..` or a `.` at the end has no file name: the path cannot be resolved
                let name = cur.file_name()?.to_os_string();
                missing.push(name);
                cur = cur.parent()?.to_path_buf();
            }
        }
    }
}

/// True when `dest` is the default export folder of the document (`default`, `Exports/` of its project): the same
/// place by [`resolve_lenient`], so also while the folder does not exist yet. Exact: no prefix, no subfolder.
#[must_use]
pub fn is_default_dest(default: &Path, dest: &Path) -> bool {
    match (resolve_lenient(default), resolve_lenient(dest)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

/// [`dest_allowed`] with the default export folder of the document (M1c2, #579): the page names it as an explicit `dest`
/// (it takes it from `export_existing`), so it is always allowed, also when it does not exist yet. It is the one folder
/// below the cache folder that the page may name; the rule stays exact. `DS_ARA_EDITOR_FAULT=old-dest` is the rule of
/// M1c1 (the default folder is refused like any other).
#[must_use]
pub fn dest_allowed_in(chosen: &[PathBuf], default: Option<&Path>, cmd: &str, args: &Value) -> bool {
    if crate::editor::fault() == "any-dest" {
        return true;
    }
    if !matches!(
        cmd,
        "export_all" | "export_existing" | "export_dest_set" | "plugin_deliver_plan" | "plugin_deliver_export"
    ) {
        return false;
    }
    if args.get("path").and_then(Value::as_str).is_some_and(|s| !s.is_empty()) {
        return false;
    }
    let Some(dest) = args.get("dest").and_then(Value::as_str).filter(|s| !s.is_empty()) else {
        return true;
    };
    let p = Path::new(dest);
    if !p.is_absolute() {
        return false;
    }
    if crate::editor::fault() != "old-dest" && default.is_some_and(|d| is_default_dest(d, p)) {
        return true;
    }
    let Ok(canon) = std::fs::canonicalize(p) else {
        return false;
    };
    if crate::editor::fault() == "prefix-dest" {
        return chosen.iter().any(|c| canon.starts_with(c));
    }
    chosen.contains(&canon)
}

// ---------------------------------------------------------------- the folders of the document

/// The file in the cache folder of a document that lists the folders the user chose in the sheet (M1c2, #579). Only the
/// plugin writes it, from the sheet; no command of the page reaches it (it lies next to `project/`, not inside).
pub const FOLDERS_FILE: &str = "export-folders.json";

/// The most folders the list keeps (the oldest go first).
pub const FOLDERS_MAX: usize = 32;

/// The largest list file the plugin reads (a bigger file is not ours).
const FOLDERS_BYTES: u64 = 64 * 1024;

/// The folders of the file `file` that can still be a `dest`: each entry is a string that is an absolute, canonical path of
/// a folder that exists NOW (equal to its own `canonicalize`, so a folder that was replaced by a link, or a path that is a file
/// now, drops out). A missing file, a file that is too big or not JSON of the shape `{"v":1,"folders":[...]}`, and every entry of
/// another type or form give nothing for that entry. Never an error: the list is a convenience.
#[must_use]
pub fn load_folders(file: &Path) -> Vec<PathBuf> {
    let Ok(meta) = std::fs::metadata(file) else {
        return Vec::new();
    };
    if !meta.is_file() || meta.len() > FOLDERS_BYTES {
        return Vec::new();
    }
    let Ok(text) = std::fs::read_to_string(file) else {
        return Vec::new();
    };
    let Ok(v) = serde_json::from_str::<Value>(&text) else {
        return Vec::new();
    };
    let Some(list) = v.get("folders").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = Vec::new();
    for e in list.iter().take(FOLDERS_MAX * 4) {
        let Some(s) = e.as_str() else { continue };
        let p = Path::new(s);
        if !p.is_absolute() {
            continue;
        }
        let Ok(canon) = std::fs::canonicalize(p) else {
            continue;
        };
        if canon != p || !canon.is_dir() || out.contains(&canon) {
            continue;
        }
        out.push(canon);
    }
    out
}

/// Write the list: a temporary file next to it, then a rename, so a reader sees the old or the new list.
///
/// # Errors
/// The error of the file system.
pub fn save_folders(file: &Path, list: &[PathBuf]) -> std::io::Result<()> {
    let start = list.len().saturating_sub(FOLDERS_MAX);
    let names: Vec<String> = list[start..].iter().map(|p| p.display().to_string()).collect();
    let body = serde_json::to_string(&serde_json::json!({"v": 1, "folders": names}))?;
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = file.with_extension("json.tmp");
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, file)
}

/// Add the canonical folder `canon` to the list in `file` (the other entries are checked again as in [`load_folders`]).
///
/// # Errors
/// The error of the file system.
pub fn remember_folder(file: &Path, canon: &Path) -> std::io::Result<()> {
    let mut list = load_folders(file);
    list.retain(|p| p != canon);
    list.push(canon.to_path_buf());
    save_folders(file, &list)
}

/// M1c3 (R6): a drag carries 1 to [`crate::prepare::MAX_DRAG_FILES`] files; more is refused whole, never cut.
#[must_use]
pub fn drag_count_ok(n: usize) -> bool {
    (1..=crate::prepare::MAX_DRAG_FILES).contains(&n)
}

/// A path the page may hand to a drag: absolute, a regular file, its folder is one of `roots` (canonical), and the
/// plugin instance wrote it itself in this process (`written`: the paths that its exports and drag takes answered).
/// Returns the canonical path.
#[must_use]
pub fn drag_path_ok(roots: &[PathBuf], written: &std::collections::HashSet<PathBuf>, path: &str) -> Option<PathBuf> {
    if crate::editor::fault() == "any-path" {
        return std::fs::canonicalize(path).ok();
    }
    let p = Path::new(path);
    if !p.is_absolute() {
        return None;
    }
    let canon = std::fs::canonicalize(p).ok()?;
    if !canon.is_file() {
        return None;
    }
    let parent = canon.parent()?;
    (roots.iter().any(|r| r == parent) && written.contains(&canon)).then_some(canon)
}

/// The longest a folder sheet may stay open before the plugin ends it (D8 of the review).
pub const SHEET_LIMIT: std::time::Duration = std::time::Duration::from_secs(600);

/// True when the wait for the sheet is over: it ran for the limit, or the editor closed.
#[must_use]
pub fn sheet_wait_over(waited: std::time::Duration, closed: bool) -> bool {
    closed || waited >= SHEET_LIMIT
}

// ---------------------------------------------------------------- AppKit (main thread only)

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSEvent, NSModalResponseOK, NSOpenPanel, NSView};
use objc2_foundation::{MainThreadMarker, NSString};
use raw_window_handle::{AppKitWindowHandle, HandleError, HasWindowHandle, RawWindowHandle, WindowHandle};
use std::ffi::c_void;
use std::ptr::NonNull;

/// The raw view of the web view as a window handle for the drag crate.
struct ViewHandle(*mut c_void);

impl HasWindowHandle for ViewHandle {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let nn = NonNull::new(self.0).ok_or(HandleError::Unavailable)?;
        // the view outlives the call: it is the web view of the editor, used on the main thread inside the job
        Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::AppKit(AppKitWindowHandle::new(nn))) })
    }
}

/// A gate run (M1c2 round 3): no system drag session may start. True when the test host drives the plugin
/// (`DS_ARA_EDITOR_MODE`, set by the test host for every editor run) or `DS_ARA_DRAG_TEST=1`; both are read only by a build
/// with the feature `test-switches`, so a delivery build is always false. A drag session that is dropped lands on whatever
/// window is under the real pointer: on 2026-11-22 a stage did that to the owner's desktop apps.
#[must_use]
pub fn drag_is_test() -> bool {
    crate::test_switch("DS_ARA_DRAG_TEST").as_deref() == Ok("1")
        || crate::test_switch("DS_ARA_EDITOR_MODE").is_ok_and(|v| !v.is_empty())
}

/// The left mouse button is down now (AppKit's own state, readable from any thread).
#[must_use]
pub fn left_button_down() -> bool {
    if crate::test_switch("DS_ARA_DRAG_PRESSED").as_deref() == Ok("1") {
        return true; // the test host has no real mouse: the stage says the button is down
    }
    NSEvent::pressedMouseButtons() & 1 != 0
}

/// What [`start_drag`] did.
#[derive(Debug, PartialEq, Eq)]
pub enum DragStart {
    /// The system drag session started; the files are on the drag pasteboard.
    Real,
    /// A gate run: the items were written to a private pasteboard and read back (their paths); no session started.
    Test(Vec<String>),
}

/// Start the native file drag of `files` from the web view `view`. Main thread only. `Err` is the reason text.
/// In a gate run ([`drag_is_test`]) everything up to the session happens (the button check, the view and window, the items),
/// the items go to a PRIVATE pasteboard that is read back and released, and no system drag session starts.
///
/// # Safety
/// `view` is the live web view of the editor (an `NSView`), and the caller is on the host's main thread.
pub unsafe fn start_drag(
    view: usize,
    files: Vec<PathBuf>,
    png: Vec<u8>,
    done: impl Fn(&str) + Send + 'static,
) -> Result<DragStart, String> {
    let v = view as *mut AnyObject;
    if v.is_null() {
        return Err("the editor has no view".into());
    }
    let window: *mut AnyObject = objc2::msg_send![v, window];
    if window.is_null() {
        return Err("the editor has no window".into());
    }
    if crate::editor::fault() != "no-button-check" && !left_button_down() {
        return Err("the mouse button was released".into());
    }
    if drag_is_test() {
        if png.is_empty() {
            return Err("the drag image is empty".into());
        }
        let paths = private_pasteboard_paths(&files)?;
        done("test");
        return Ok(DragStart::Test(paths));
    }
    start_drag_system(v, files, png, done).map(|()| DragStart::Real)
}

/// THE ONLY place that starts a system drag session (the stage `drag-source` reads the source for exactly one call of the
/// crate `drag` here). It refuses in a gate run, so even a wrong caller cannot start a session there.
unsafe fn start_drag_system(
    v: *mut AnyObject,
    files: Vec<PathBuf>,
    png: Vec<u8>,
    done: impl Fn(&str) + Send + 'static,
) -> Result<(), String> {
    if drag_is_test() {
        return Err("a gate run starts no system drag session".into());
    }
    let handle = ViewHandle(v.cast());
    drag::start_drag(
        &handle,
        drag::DragItem::Files(files),
        drag::Image::Raw(png),
        move |res, _pos| {
            done(match res {
                drag::DragResult::Dropped => "dropped",
                drag::DragResult::Cancel => "cancelled",
            })
        },
        drag::Options::default(),
    )
    .map_err(|e| format!("{e}"))
}

/// The file URLs of `pb`'s items (what a drop target reads).
fn pasteboard_paths(pb: &objc2_app_kit::NSPasteboard) -> Vec<String> {
    use objc2_foundation::NSURL;
    let mut out = Vec::new();
    if let Some(items) = pb.pasteboardItems() {
        for it in items.iter() {
            let t = NSString::from_str("public.file-url");
            if let Some(s) = it.stringForType(&t) {
                if let Some(u) = NSURL::URLWithString(&s) {
                    if let Some(p) = u.path() {
                        out.push(p.to_string());
                    }
                }
            }
        }
    }
    out
}

/// A gate run: write the files as items to a private pasteboard with a unique name (never the general or the drag
/// pasteboard of the system), read the paths back and release it.
fn private_pasteboard_paths(files: &[PathBuf]) -> Result<Vec<String>, String> {
    use objc2::runtime::ProtocolObject;
    use objc2_app_kit::{NSPasteboard, NSPasteboardWriting};
    use objc2_foundation::{NSArray, NSURL};
    let pb = NSPasteboard::pasteboardWithUniqueName();
    pb.clearContents();
    let urls: Vec<_> = files
        .iter()
        .map(|f| {
            let u = NSURL::fileURLWithPath(&NSString::from_str(&f.display().to_string()));
            ProtocolObject::<dyn NSPasteboardWriting>::from_retained(u)
        })
        .collect();
    let ok = pb.writeObjects(&NSArray::from_retained_slice(&urls));
    let paths = pasteboard_paths(&pb);
    let _: () = unsafe { objc2::msg_send![&*pb, releaseGlobally] };
    if ok {
        Ok(paths)
    } else {
        Err("the items could not be written to the pasteboard".into())
    }
}

/// The file URLs on the drag pasteboard now (what a drop target reads) after a REAL session started. Never read in a gate run.
#[must_use]
pub fn drag_pasteboard_paths() -> Vec<String> {
    use objc2_app_kit::{NSPasteboard, NSPasteboardNameDrag};
    unsafe { pasteboard_paths(&NSPasteboard::pasteboardWithName(NSPasteboardNameDrag)) }
}

/// Show the folder sheet on the window of `view` and call `done` with the chosen folder (`None`: cancelled) on the
/// main thread. A view with no window answers `Err` at once. Main thread only.
///
/// # Safety
/// `view` is the live web view of the editor and the caller is on the host's main thread.
pub unsafe fn folder_sheet(
    view: usize,
    title: &str,
    start: &str,
    done: impl FnOnce(Option<PathBuf>) + 'static,
) -> Result<Retained<AnyObject>, String> {
    let v = view as *mut NSView;
    let window = (*v).window().ok_or_else(|| "the editor has no window".to_string())?;
    if !window.isVisible() {
        return Err("the plugin window is not visible".to_string());
    }
    let mtm = MainThreadMarker::new().ok_or("the sheet needs the main thread")?;
    let panel = NSOpenPanel::openPanel(mtm);
    panel.setCanChooseDirectories(true);
    panel.setCanChooseFiles(false);
    panel.setCanCreateDirectories(true);
    panel.setAllowsMultipleSelection(false);
    panel.setMessage(Some(&NSString::from_str(title)));
    panel.setPrompt(Some(&NSString::from_str("Choose")));
    if !start.is_empty() && Path::new(start).is_dir() {
        let url = objc2_foundation::NSURL::fileURLWithPath_isDirectory(&NSString::from_str(start), true);
        panel.setDirectoryURL(Some(&url));
    }
    let cell = std::cell::RefCell::new(Some(done));
    let p2 = panel.clone();
    let block = block2::RcBlock::new(move |resp: isize| {
        let chosen = if resp == NSModalResponseOK {
            p2.URL().and_then(|u| u.path()).map(|p| PathBuf::from(p.to_string()))
        } else {
            None
        };
        if let Some(f) = cell.borrow_mut().take() {
            f(chosen);
        }
    });
    panel.beginSheetModalForWindow_completionHandler(&window, &block);
    Ok(Retained::cast_unchecked(panel))
}

/// End the sheet `panel` (a pointer made by [`folder_sheet`] that is still shown). Main thread only.
///
/// # Safety
/// `panel` is a live `NSOpenPanel` shown as a sheet.
pub unsafe fn end_sheet(panel: usize) {
    let p = panel as *mut AnyObject;
    if p.is_null() {
        return;
    }
    let parent: *mut AnyObject = objc2::msg_send![p, sheetParent];
    if !parent.is_null() {
        let _: () = objc2::msg_send![parent, endSheet: p];
    }
}

/// The bytes of a base64 text (the PNG of the drag image); `None` for a character that is not base64.
#[must_use]
pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in text.bytes().filter(|c| !c.is_ascii_whitespace()) {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn d(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("ds-deliver-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        std::fs::canonicalize(p).unwrap()
    }

    #[test]
    fn a_track_name_becomes_a_safe_file_stem_and_double_names_are_numbered() {
        assert_eq!(safe_stem("Kick In/Out: 2").as_deref(), Some("Kick In-Out- 2"));
        assert_eq!(safe_stem("..hidden").as_deref(), Some("hidden"));
        assert_eq!(safe_stem(".."), None);
        assert_eq!(safe_stem("   "), None);
        assert_eq!(safe_stem(&"x".repeat(150)).unwrap().len(), STEM_BYTES);
        // 100 emoji (4 bytes each): cut on a character boundary, at most 80 bytes
        let e = safe_stem(&"\u{1F941}".repeat(100)).unwrap();
        assert_eq!((e.len(), e.chars().count()), (80, 20));
        let m: HashMap<String, String> = [
            ("k1", "Snare"),
            ("k2", "snare"),
            ("k3", "A/B"),
            ("k4", "A:B"),
            ("k5", ""),
        ]
        .into_iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect();
        let s = file_stems(&m);
        assert_eq!((s["k1"].as_str(), s["k2"].as_str()), ("Snare", "snare (2)"));
        assert_eq!((s["k3"].as_str(), s["k4"].as_str()), ("A-B", "A-B (2)"));
        assert_eq!(s["k5"], "k5", "no name: the key");
        // names that the plan check sees as one file name (the app's file_name_key) are numbered, not refused later
        let m: HashMap<String, String> = [
            ("a", "Stra\u{DF}e"),
            ("b", "STRASSE"),
            ("c", "Cafe\u{301}"),
            ("d", "Caf\u{E9}"),
        ]
        .into_iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect();
        let s = file_stems(&m);
        assert_eq!(
            (s["a"].as_str(), s["b"].as_str()),
            ("Stra\u{DF}e", "STRASSE (2)"),
            "a sharp s and SS"
        );
        assert_eq!(s["d"], "Caf\u{E9} (2)", "a decomposed and a composed accent");
        let keys: std::collections::HashSet<String> =
            s.values().map(|v| drumspike_app::export::file_name_key(v)).collect();
        assert_eq!(keys.len(), 4, "no two stems share a key");
    }

    #[test]
    fn only_a_folder_chosen_in_this_session_is_a_dest() {
        let a = d("a");
        let sub = a.join("sub");
        std::fs::create_dir(&sub).unwrap();
        let other = d("b");
        let chosen = vec![a.clone()];
        let args = |dest: &str| json!({"dest": dest});
        assert!(dest_allowed(&chosen, "export_all", &args(a.to_str().unwrap())));
        assert!(dest_allowed(&chosen, "export_existing", &args(a.to_str().unwrap())));
        assert!(dest_allowed(&chosen, "export_dest_set", &args(a.to_str().unwrap())));
        assert!(
            !dest_allowed(&chosen, "export_all", &args(other.to_str().unwrap())),
            "another folder"
        );
        assert!(
            !dest_allowed(&chosen, "export_all", &args(sub.to_str().unwrap())),
            "a subfolder"
        );
        assert!(!dest_allowed(&chosen, "export_all", &args("relative")));
        assert!(!dest_allowed(&chosen, "export_all", &args("/definitely/not/there")));
        assert!(
            !dest_allowed(&[], "export_all", &args(a.to_str().unwrap())),
            "nothing chosen yet"
        );
        for cmd in [
            "export_drag_prepare",
            "export_midi",
            "export_spikes",
            "export_layer",
            "sampler_calibration_midi",
        ] {
            assert!(!dest_allowed(&chosen, cmd, &args(a.to_str().unwrap())), "{cmd}");
        }
        assert!(!dest_allowed(
            &chosen,
            "export_all",
            &json!({"dest": a.to_str().unwrap(), "path": "/x"})
        ));
        // a dest with a dot-dot that leads to the chosen folder is the same folder; one that leads elsewhere is not
        let dd = format!("{}/sub/..", a.display());
        assert!(dest_allowed(&chosen, "export_all", &args(&dd)));
        let dd2 = format!("{}/..", a.display());
        assert!(!dest_allowed(&chosen, "export_all", &args(&dd2)));
        // a folder replaced by a link to another folder no longer equals the chosen one
        let link = other.join("link");
        std::os::unix::fs::symlink(&a, &link).unwrap();
        assert!(
            dest_allowed(&chosen, "export_all", &args(link.to_str().unwrap())),
            "a link to the chosen folder is that folder"
        );
        let b2 = d("c");
        let chosen2 = vec![b2.clone()];
        assert!(
            !dest_allowed(&chosen2, "export_all", &args(link.to_str().unwrap())),
            "a link to another folder is refused"
        );
    }

    #[test]
    fn the_default_folder_is_a_dest_also_before_it_exists_and_the_rule_stays_exact() {
        let doc = d("default");
        let default = doc.join("project").join("Exports");
        let args = |dest: &Path| json!({"dest": dest.to_str().unwrap()});
        // nothing chosen, the folder does not exist yet
        assert!(!default.exists());
        for cmd in ["export_all", "export_existing", "export_dest_set"] {
            assert!(dest_allowed_in(&[], Some(&default), cmd, &args(&default)), "{cmd}");
        }
        // the rule of M1c1 (no default given) refuses it: the defect of #579
        assert!(!dest_allowed(&[], "export_existing", &args(&default)));
        // the same place by another spelling (a dot, a double slash, a link to the project folder)
        assert!(dest_allowed_in(
            &[],
            Some(&default),
            "export_existing",
            &json!({"dest": format!("{}//./Exports", doc.join("project").display())})
        ));
        let link = doc.join("link");
        std::os::unix::fs::symlink(doc.join("project"), &link).unwrap();
        std::fs::create_dir_all(doc.join("project")).unwrap();
        assert!(dest_allowed_in(
            &[],
            Some(&default),
            "export_existing",
            &args(&link.join("Exports"))
        ));
        // exact: no subfolder, no parent, no sibling, no other name, no folder with a double dot in the missing part
        for bad in [
            default.join("sub"),
            default.join(".drag"),
            doc.join("project"),
            doc.join("project").join("Exports2"),
            doc.join("project").join("exports-x"),
            doc.join("other").join("Exports"),
            doc.join("project").join("missing").join("..").join("Exports"),
        ] {
            assert!(
                !dest_allowed_in(&[], Some(&default), "export_existing", &args(&bad)),
                "{bad:?}"
            );
        }
        assert!(
            !dest_allowed_in(&[], Some(&default), "export_existing", &json!({"dest": "Exports"})),
            "relative"
        );
        // the other commands that take a dest stay refused, and a `path` never passes
        for cmd in [
            "export_drag_prepare",
            "export_midi",
            "export_spikes",
            "export_layer",
            "sampler_calibration_midi",
        ] {
            assert!(!dest_allowed_in(&[], Some(&default), cmd, &args(&default)), "{cmd}");
        }
        assert!(!dest_allowed_in(
            &[],
            Some(&default),
            "export_all",
            &json!({"dest": default.to_str().unwrap(), "path": "/x"})
        ));
        // the folder that exists now is the same answer
        std::fs::create_dir_all(&default).unwrap();
        assert!(dest_allowed_in(&[], Some(&default), "export_all", &args(&default)));
        assert!(!dest_allowed_in(
            &[],
            Some(&default),
            "export_all",
            &args(&default.join("sub"))
        ));
    }

    #[test]
    fn a_missing_end_of_a_path_is_resolved_behind_its_existing_part() {
        let a = d("lenient");
        assert_eq!(resolve_lenient(&a.join("x").join("y")), Some(a.join("x").join("y")));
        assert_eq!(resolve_lenient(&a), Some(a.clone()));
        assert_eq!(resolve_lenient(Path::new("relative/x")), None);
        assert_eq!(
            resolve_lenient(&a.join("x").join("..").join("y")),
            None,
            "a double dot in the missing part"
        );
    }

    fn list_file(tag: &str) -> (PathBuf, PathBuf) {
        let dir = d(tag);
        let file = dir.join(FOLDERS_FILE);
        (dir, file)
    }

    #[test]
    fn a_remembered_folder_is_kept_in_the_file_and_loaded_again() {
        let (dir, file) = list_file("rem1");
        let (a, b) = (dir.join("a"), dir.join("b"));
        std::fs::create_dir(&a).unwrap();
        std::fs::create_dir(&b).unwrap();
        assert!(load_folders(&file).is_empty(), "no file yet");
        remember_folder(&file, &a).unwrap();
        remember_folder(&file, &b).unwrap();
        remember_folder(&file, &a).unwrap();
        assert_eq!(
            load_folders(&file),
            vec![b.clone(), a.clone()],
            "no double entry, the newest last"
        );
        // the folder is gone: it falls out of the list
        std::fs::remove_dir(&b).unwrap();
        assert_eq!(load_folders(&file), vec![a.clone()]);
        // the list keeps the newest FOLDERS_MAX entries
        let many: Vec<PathBuf> = (0..FOLDERS_MAX + 3)
            .map(|i| {
                let p = dir.join(format!("m{i}"));
                std::fs::create_dir(&p).unwrap();
                p
            })
            .collect();
        save_folders(&file, &many).unwrap();
        let got = load_folders(&file);
        assert_eq!(got.len(), FOLDERS_MAX);
        assert_eq!(got.last(), many.last());
        // the temporary file does not stay
        assert!(!file.with_extension("json.tmp").exists());
    }

    #[test]
    fn hostile_content_of_the_folder_list_gives_no_folder() {
        let (dir, file) = list_file("rem2");
        let ok = dir.join("ok");
        std::fs::create_dir(&ok).unwrap();
        let a_file = dir.join("now-a-file");
        std::fs::write(&a_file, b"x").unwrap();
        let moved = dir.join("moved");
        let target = dir.join("target");
        std::fs::create_dir(&target).unwrap();
        std::fs::create_dir(&moved).unwrap();
        // a folder that was a folder when it was chosen and is a link to another folder now
        std::fs::remove_dir(&moved).unwrap();
        std::os::unix::fs::symlink(&target, &moved).unwrap();
        let ds = |v: &Value| std::fs::write(&file, v.to_string()).unwrap();
        // wrong types at the top: not an object, no list, a list that is a string, a number
        for top in [
            json!([]),
            json!("x"),
            json!(7),
            json!(null),
            json!({"v": 1}),
            json!({"folders": "/tmp"}),
            json!({"folders": 3}),
        ] {
            ds(&top);
            assert!(load_folders(&file).is_empty(), "{top}");
        }
        // entries of the wrong type or form: dropped one by one, the good one stays
        ds(&json!({"v": 1, "folders": [
            7, null, true, ["/tmp"], {"p": "/tmp"}, "", "relative/path", "~/x", "../x",
            a_file.to_str().unwrap(),     // a path that is a file now
            moved.to_str().unwrap(),      // a link that moved: not its own canonical form
            dir.join("missing").to_str().unwrap(),
            format!("{}/./ok", dir.display()),   // not canonical
            format!("{}/ok/", dir.display()),    // not canonical
            ok.to_str().unwrap(),
        ]}));
        assert_eq!(load_folders(&file), vec![ok.clone()]);
        // not JSON, an empty file, a file that is too big, a folder in place of the file
        std::fs::write(&file, b"{not json").unwrap();
        assert!(load_folders(&file).is_empty());
        std::fs::write(&file, b"").unwrap();
        assert!(load_folders(&file).is_empty());
        std::fs::write(&file, vec![b' '; (FOLDERS_BYTES as usize) + 1]).unwrap();
        assert!(load_folders(&file).is_empty());
        std::fs::remove_file(&file).unwrap();
        std::fs::create_dir(&file).unwrap();
        assert!(load_folders(&file).is_empty());
        // a symlink that moved after the choice is not allowed as a dest either
        std::fs::remove_dir(&file).unwrap();
        save_folders(&file, std::slice::from_ref(&ok)).unwrap();
        std::fs::remove_dir(&ok).unwrap();
        std::os::unix::fs::symlink(&target, &ok).unwrap();
        assert!(
            load_folders(&file).is_empty(),
            "the folder is a link to another one now"
        );
    }

    #[test]
    fn the_items_of_a_drag_go_to_a_private_pasteboard_and_read_back() {
        let a = d("pb");
        let (f1, f2) = (a.join("one_spike.mid"), a.join("two spike.wav"));
        std::fs::write(&f1, b"1").unwrap();
        std::fs::write(&f2, b"2").unwrap();
        let got = private_pasteboard_paths(&[f1.clone(), f2.clone()]).unwrap();
        assert_eq!(got, vec![f1.display().to_string(), f2.display().to_string()]);
        assert!(private_pasteboard_paths(&[]).unwrap().is_empty());
    }

    #[cfg(not(feature = "test-switches"))]
    #[test]
    fn a_delivery_build_is_never_a_test_drag_whatever_the_environment_says() {
        std::env::set_var("DS_ARA_DRAG_TEST", "1");
        std::env::set_var("DS_ARA_EDITOR_MODE", "delivery");
        assert!(
            !drag_is_test(),
            "the switches are inert in a delivery build: the drag is the real one"
        );
        std::env::remove_var("DS_ARA_DRAG_TEST");
        std::env::remove_var("DS_ARA_EDITOR_MODE");
    }

    #[cfg(feature = "test-switches")]
    #[test]
    fn a_gate_run_is_a_test_drag_and_the_system_session_function_refuses() {
        // the unit tests run with the feature test-switches; DS_ARA_DRAG_TEST makes the run a test run
        std::env::set_var("DS_ARA_DRAG_TEST", "1");
        assert!(drag_is_test());
        // the function that starts the system session refuses before it touches the crate `drag`
        let r = unsafe { start_drag_system(std::ptr::null_mut(), Vec::new(), Vec::new(), |_| {}) };
        assert_eq!(r, Err("a gate run starts no system drag session".to_string()));
        std::env::remove_var("DS_ARA_DRAG_TEST");
    }

    #[test]
    fn base64_decodes_the_drag_image() {
        assert_eq!(base64_decode("aGVsbG8=").as_deref(), Some(&b"hello"[..]));
        assert_eq!(base64_decode("aGVs bG8").as_deref(), Some(&b"hello"[..]));
        assert_eq!(base64_decode("a*b"), None);
    }

    #[test]
    fn a_drag_takes_only_a_file_directly_in_a_root() {
        let a = d("drag");
        let f = a.join("x_spike.mid");
        std::fs::write(&f, b"x").unwrap();
        let sub = a.join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("y.mid"), b"y").unwrap();
        let other = a.join("notes.txt");
        std::fs::write(&other, b"z").unwrap();
        let roots = vec![a.clone()];
        let written: std::collections::HashSet<PathBuf> = [f.clone(), sub.join("y.mid")].into_iter().collect();
        assert_eq!(drag_path_ok(&roots, &written, f.to_str().unwrap()), Some(f.clone()));
        assert_eq!(
            drag_path_ok(&roots, &written, sub.join("y.mid").to_str().unwrap()),
            None,
            "a file in a subfolder"
        );
        assert_eq!(drag_path_ok(&roots, &written, a.to_str().unwrap()), None, "a folder");
        assert_eq!(drag_path_ok(&roots, &written, "/etc/hosts"), None, "a file elsewhere");
        assert_eq!(drag_path_ok(&roots, &written, "x_spike.mid"), None, "a relative path");
        assert_eq!(
            drag_path_ok(&roots, &written, other.to_str().unwrap()),
            None,
            "a file in the folder that the plugin did not write"
        );
        assert_eq!(
            drag_path_ok(&roots, &std::collections::HashSet::new(), f.to_str().unwrap()),
            None,
            "nothing written yet"
        );
    }

    fn rec(a: f64, d: f64, p: f64) -> crate::service::RegionRec {
        crate::service::RegionRec {
            start_mod: a,
            dur_mod: d,
            start_play: p,
        }
    }

    #[test]
    fn a_moved_or_trimmed_region_exports_through_its_live_values() {
        let round = [rec(0.5, 2.0, 10.0), rec(5.0, 1.0, 20.0)];
        // the host moved the first region to 30 s, trimmed it to [1.0, 2.0) at 30.5 s, and deleted the second (no new round)
        let live = [(7u64, rec(1.0, 1.0, 30.5))];
        let lens: HashMap<u64, f64> = [(7, 1.0)].into_iter().collect();
        let (got, gone) = merge_regions(Some(&round), &live, &lens, false);
        assert_eq!(
            got,
            vec![ExportRegion {
                start_mod: 1.0,
                dur_mod: 1.0,
                start_play: 30.5,
                dur_play: 1.0
            }]
        );
        assert!(!gone);
        // a stretched region carries its play length
        let lens: HashMap<u64, f64> = [(7, 2.5)].into_iter().collect();
        assert_eq!(merge_regions(Some(&round), &live, &lens, false).0[0].dur_play, 2.5);
        // no round yet: the live regions
        assert_eq!(merge_regions(None, &live, &lens, false).0.len(), 1);
        // the fault run keeps the round
        assert_eq!(merge_regions(Some(&round), &live, &lens, true).0[0].start_play, 10.0);
    }

    #[test]
    fn a_source_whose_clips_are_all_deleted_is_gone_and_one_that_never_had_a_region_is_the_identity() {
        let round = [rec(0.5, 2.0, 10.0)];
        // no live region, regions at the round: gone (every hit is left out), never the round's positions
        assert_eq!(merge_regions(Some(&round), &[], &HashMap::new(), false), (vec![], true));
        // no region at the round and none now: the identity time base
        assert_eq!(merge_regions(Some(&[]), &[], &HashMap::new(), false), (vec![], false));
        assert_eq!(merge_regions(None, &[], &HashMap::new(), false), (vec![], false));
        // the fault run keeps the old positions of the round
        assert_eq!(merge_regions(Some(&round), &[], &HashMap::new(), true).0.len(), 1);
        // the track of a gone source loses every hit
        let t = HostTime {
            regions: Arc::new(|| ExportRegions {
                regions: vec![],
                gone: vec![3],
            }),
            track_info: Arc::new(|_| (vec![3], 1.0)),
            names: Arc::new(|| [("k".to_string(), "Floor Tom".to_string())].into_iter().collect()),
            tempo: Arc::new(|| crate::tempo_ara::choose(None, "", None)),
        };
        let st = t.track("k").unwrap();
        assert_eq!(
            (st.hide_all, st.regions.len(), st.stem.as_deref()),
            (true, 0, Some("Floor Tom"))
        );
        // a track with a source that never had a region keeps the identity
        let t2 = HostTime {
            track_info: Arc::new(|_| (vec![4], 1.0)),
            ..t
        };
        assert!(!t2.track("k").unwrap().hide_all);
    }

    #[test]
    fn a_track_with_no_source_keeps_the_host_name_and_loses_every_hit() {
        let t = HostTime {
            regions: Arc::new(ExportRegions::default),
            track_info: Arc::new(|_| (Vec::new(), 0.0)),
            names: Arc::new(|| [("key1".to_string(), "Snare Top".to_string())].into_iter().collect()),
            tempo: Arc::new(|| crate::tempo_ara::choose(None, "", None)),
        };
        let st = t.track("key1").unwrap();
        assert_eq!(
            (st.stem.as_deref(), st.hide_all, st.regions.len()),
            (Some("Snare Top"), true, 0)
        );
    }

    #[test]
    fn the_wait_for_the_sheet_ends_with_the_limit_or_a_closed_editor() {
        use std::time::Duration;
        assert!(!sheet_wait_over(Duration::from_secs(599), false));
        assert!(sheet_wait_over(SHEET_LIMIT, false));
        assert!(sheet_wait_over(Duration::from_secs(1), true));
    }
}
