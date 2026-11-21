//! M1c1 (#560): what leaves the plugin: the song time base of the export, the one folder exception of the rule of
//! #532, the folder sheet and the native file drag.
//!
//! The design is `docs/design/daw-plugin/m1c1-delivery.md`. This file holds the pure parts (the file stems, the
//! check of a `dest`, the check of a drag path) and the two parts that touch AppKit (the sheet and the drag session,
//! both on the host's main thread only).

use crate::editor::{RegionsFn, TrackInfoFn};
use crate::tempo_ara::Answer;
use drumspike_app::export_song::{ExportTime, SongRegion, SongTrack};
use drumspike_core::tempo::TempoMap;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

type NamesFn = Arc<dyn Fn() -> HashMap<String, String> + Send + Sync>;
type TempoFn = Arc<dyn Fn() -> Answer + Send + Sync>;

/// The song time base of the host for the export of one document.
pub struct HostTime {
    pub regions: RegionsFn,
    pub track_info: TrackInfoFn,
    pub names: NamesFn,
    pub tempo: TempoFn,
}

impl ExportTime for HostTime {
    fn track(&self, name: &str) -> Option<SongTrack> {
        let fault = crate::test_switch("DS_ARA_DELIVERY_FAULT").unwrap_or_default();
        let (seqs, _) = (self.track_info)(name);
        if seqs.is_empty() {
            return None;
        }
        let regions = if fault == "source-time" {
            // the fault run: the regions are left out, so the file keeps the time of the source
            Vec::new()
        } else {
            (self.regions)()
                .into_iter()
                .filter(|(i, _)| seqs.contains(i))
                .map(|(_, r)| SongRegion {
                    start_mod: r.start_mod,
                    dur_mod: r.dur_mod,
                    start_play: r.start_play,
                })
                .collect()
        };
        let names = (self.names)();
        let stem = file_stems(&names).get(name).cloned();
        Some(SongTrack { regions, stem })
    }

    fn tempo(&self) -> Option<TempoMap> {
        let a = (self.tempo)();
        if crate::test_switch("DS_ARA_DELIVERY_FAULT").as_deref() == Ok("one-tempo") {
            return a.map.map(|m| TempoMap { steps: Vec::new(), ..m });
        }
        a.map
    }
}

/// A DAW track name as a file stem: the characters that a file name cannot hold (the rule of the app's
/// `multi_file_name`: control characters and `/ \ : * ? " < > |`) become `-`; leading dots go (no hidden file, no
/// `..`); at most 100 characters; empty gives `None`.
#[must_use]
pub fn safe_stem(name: &str) -> Option<String> {
    let n: String = name
        .chars()
        .map(|c| if c.is_control() || "/\\:*?\"<>|".contains(c) { '-' } else { c })
        .collect();
    let n: String = n.trim().trim_start_matches('.').trim().chars().take(100).collect();
    (!n.is_empty()).then_some(n)
}

/// The file stem of every track: its display name made safe; the key when the name gives none. Two tracks that
/// end on one stem (compared without case, as APFS does) number the later one by the order of the keys.
#[must_use]
pub fn file_stems(display: &HashMap<String, String>) -> HashMap<String, String> {
    let mut keys: Vec<&String> = display.keys().collect();
    keys.sort();
    let mut seen: Vec<String> = Vec::new();
    let mut out = HashMap::new();
    for k in keys {
        let base = safe_stem(&display[k]).or_else(|| safe_stem(k)).unwrap_or_else(|| "track".to_string());
        let mut stem = base.clone();
        let mut n = 2;
        while seen.contains(&stem.to_lowercase()) {
            stem = format!("{base} ({n})");
            n += 1;
        }
        seen.push(stem.to_lowercase());
        out.insert(k.clone(), stem);
    }
    out
}

/// The one exception to the rule of #532: a `dest` that is a folder the user chose in the sheet of this editor.
/// `chosen` holds canonical paths. The command must be one that takes a `dest` for an export, the `dest` must be
/// absolute, and its canonical form must EQUAL an entry (no prefix, no subfolder): a folder that was replaced by a
/// link after the choice no longer equals. A `path` key never passes.
#[must_use]
pub fn dest_allowed(chosen: &[PathBuf], cmd: &str, args: &Value) -> bool {
    if crate::editor::fault() == "any-dest" {
        return true;
    }
    if !matches!(cmd, "export_all" | "export_existing" | "export_dest_set") {
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
    let Ok(canon) = std::fs::canonicalize(p) else { return false };
    if crate::editor::fault() == "prefix-dest" {
        return chosen.iter().any(|c| canon.starts_with(c));
    }
    chosen.iter().any(|c| *c == canon)
}

/// A path the page may hand to a drag: absolute, a regular file, and its folder is one of `roots` (canonical).
/// Returns the canonical path.
#[must_use]
pub fn drag_path_ok(roots: &[PathBuf], path: &str) -> Option<PathBuf> {
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
    roots.iter().any(|r| r == parent).then_some(canon)
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

/// The left mouse button is down now (AppKit's own state, readable from any thread).
#[must_use]
pub fn left_button_down() -> bool {
    if crate::test_switch("DS_ARA_DRAG_PRESSED").as_deref() == Ok("1") {
        return true; // the test host has no real mouse: the stage says the button is down
    }
    NSEvent::pressedMouseButtons() & 1 != 0
}

/// Start the native file drag of `files` from the web view `view`. Main thread only. `Err` is the reason text.
///
/// # Safety
/// `view` is the live web view of the editor (an `NSView`), and the caller is on the host's main thread.
pub unsafe fn start_drag(view: usize, files: Vec<PathBuf>, png: Vec<u8>, done: impl Fn(&str) + Send + 'static) -> Result<(), String> {
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

/// The file URLs on the drag pasteboard now (what a drop target reads); the stage compares them with the files.
#[must_use]
pub fn drag_pasteboard_paths() -> Vec<String> {
    use objc2_app_kit::{NSPasteboard, NSPasteboardNameDrag};
    use objc2_foundation::NSURL;
    unsafe {
        let pb = NSPasteboard::pasteboardWithName(NSPasteboardNameDrag);
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
}

/// Show the folder sheet on the window of `view` and call `done` with the chosen folder (`None`: cancelled) on the
/// main thread. A view with no window answers `Err` at once. Main thread only.
///
/// # Safety
/// `view` is the live web view of the editor and the caller is on the host's main thread.
pub unsafe fn folder_sheet(view: usize, title: &str, start: &str, done: impl FnOnce(Option<PathBuf>) + 'static) -> Result<Retained<AnyObject>, String> {
    let v = view as *mut NSView;
    let window = (*v).window().ok_or_else(|| "the editor has no window".to_string())?;
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
        assert_eq!(safe_stem(&"x".repeat(150)).unwrap().len(), 100);
        let m: HashMap<String, String> = [("k1", "Snare"), ("k2", "snare"), ("k3", "A/B"), ("k4", "A:B"), ("k5", "")]
            .into_iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect();
        let s = file_stems(&m);
        assert_eq!((s["k1"].as_str(), s["k2"].as_str()), ("Snare", "snare (2)"));
        assert_eq!((s["k3"].as_str(), s["k4"].as_str()), ("A-B", "A-B (2)"));
        assert_eq!(s["k5"], "k5", "no name: the key");
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
        assert!(!dest_allowed(&chosen, "export_all", &args(other.to_str().unwrap())), "another folder");
        assert!(!dest_allowed(&chosen, "export_all", &args(sub.to_str().unwrap())), "a subfolder");
        assert!(!dest_allowed(&chosen, "export_all", &args("relative")));
        assert!(!dest_allowed(&chosen, "export_all", &args("/definitely/not/there")));
        assert!(!dest_allowed(&[], "export_all", &args(a.to_str().unwrap())), "nothing chosen yet");
        for cmd in ["export_drag_prepare", "export_midi", "export_spikes", "export_layer", "sampler_calibration_midi"] {
            assert!(!dest_allowed(&chosen, cmd, &args(a.to_str().unwrap())), "{cmd}");
        }
        assert!(!dest_allowed(&chosen, "export_all", &json!({"dest": a.to_str().unwrap(), "path": "/x"})));
        // a dest with a dot-dot that leads to the chosen folder is the same folder; one that leads elsewhere is not
        let dd = format!("{}/sub/..", a.display());
        assert!(dest_allowed(&chosen, "export_all", &args(&dd)));
        let dd2 = format!("{}/..", a.display());
        assert!(!dest_allowed(&chosen, "export_all", &args(&dd2)));
        // a folder replaced by a link to another folder no longer equals the chosen one
        let link = other.join("link");
        std::os::unix::fs::symlink(&a, &link).unwrap();
        assert!(dest_allowed(&chosen, "export_all", &args(link.to_str().unwrap())), "a link to the chosen folder is that folder");
        let b2 = d("c");
        let chosen2 = vec![b2.clone()];
        assert!(!dest_allowed(&chosen2, "export_all", &args(link.to_str().unwrap())), "a link to another folder is refused");
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
        let roots = vec![a.clone()];
        assert_eq!(drag_path_ok(&roots, f.to_str().unwrap()), Some(f.clone()));
        assert_eq!(drag_path_ok(&roots, sub.join("y.mid").to_str().unwrap()), None, "a file in a subfolder");
        assert_eq!(drag_path_ok(&roots, a.to_str().unwrap()), None, "a folder");
        assert_eq!(drag_path_ok(&roots, "/etc/hosts"), None, "a file elsewhere");
        assert_eq!(drag_path_ok(&roots, "x_spike.mid"), None, "a relative path");
    }
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
