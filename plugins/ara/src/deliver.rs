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
    let Ok(canon) = std::fs::canonicalize(p) else {
        return false;
    };
    if crate::editor::fault() == "prefix-dest" {
        return chosen.iter().any(|c| canon.starts_with(c));
    }
    chosen.contains(&canon)
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
pub unsafe fn start_drag(
    view: usize,
    files: Vec<PathBuf>,
    png: Vec<u8>,
    done: impl Fn(&str) + Send + 'static,
) -> Result<(), String> {
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

/// The test host has no hand to let go of the mouse: post an in-process mouse up to the window of `view`, so the drag
/// session that the stage started ends (the callback then says `cancelled`). Only `DS_ARA_DRAG_PRESSED` runs it. Main thread
/// only. No event of the system is posted: this one goes to the application's own queue.
///
/// # Safety
/// `view` is the live web view of the editor and the caller is on the host's main thread.
pub unsafe fn release_test_drag(view: usize) {
    use objc2_app_kit::{NSApplication, NSEventModifierFlags, NSEventType};
    use objc2_foundation::NSPoint;
    let v = view as *mut NSView;
    let (Some(window), Some(mtm)) = ((*v).window(), MainThreadMarker::new()) else {
        return;
    };
    let at = window.mouseLocationOutsideOfEventStream();
    if let Some(up) = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
        NSEventType::LeftMouseUp,
        NSPoint::new(at.x, at.y),
        NSEventModifierFlags::empty(),
        0.0,
        window.windowNumber(),
        None,
        0,
        1,
        0.0,
    ) {
        NSApplication::sharedApplication(mtm).postEvent_atStart(&up, true);
    }
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
