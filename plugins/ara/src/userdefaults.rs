//! M1d3 (#577): the user defaults and the presets of the plugin.
//!
//! The design is `docs/design/daw-plugin/m1d3-defaults.md`. Three levels: built-in < the file of this module < the session
//! (the project document and the ARA archive). The file applies to NEW tracks only, at one place (`service.rs`
//! `make_new_track`); a restore never reads it.
//!
//! The files: `<dir>/defaults.json` and `<dir>/Presets/<stem>.json`, with `dir` =
//! `~/Library/Application Support/DrumSpike ARA`. The folder is made at the first Save, never on a read. A gate build reads
//! the folder from `DS_ARA_USER_DIR` and has NO path without it (the real folder is never reached by a stage); a delivery
//! build has the real path and reads no variable.
//!
//! Every value that is read is range-checked; a bad field gives its built-in value and ONE report row per changed file. A
//! file that is a link, a folder, too big or unreadable is "none". A report row never holds an absolute path: the folder is the
//! fixed word `defaults-folder`.

use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// The name of the plugin's own folder below Application Support.
pub const FOLDER: &str = "DrumSpike ARA";
/// The fixed word that stands for the folder in a report row.
pub const FOLDER_WORD: &str = "defaults-folder";
/// The largest defaults file that is read (about 1 KB is in use).
pub const MAX_BYTES: u64 = 256 * 1024;
/// The largest preset file that is read.
pub const PRESET_BYTES: u64 = 64 * 1024;
/// The most preset files that are listed.
pub const MAX_PRESETS: usize = 200;
/// The version this plugin writes and reads fully.
pub const VERSION: i64 = 1;
/// The roles of a track (`crates/app/src/roles.rs`).
pub const ROLES: [&str; 6] = ["kick", "snare", "tom", "hat", "cymbal", "other"];
/// The name of the built-in preset (not a file).
pub const BUILTIN: &str = "General MIDI";
/// The most notes a role entry holds.
pub const MAX_NOTES: usize = 8;

fn fault() -> String {
    crate::test_switch("DS_ARA_DEFAULTS_FAULT").unwrap_or_default()
}

// ---------------------------------------------------------------- the folder

/// The folder of the user files, or `None`. In a delivery build it is always the real folder. In a gate build it is the value of
/// `DS_ARA_USER_DIR`; without it the answer is `None` (a stage that forgot the switch cannot reach the real folder) and one row says so.
/// (Fault `real-path`: the real folder in a gate build too; the stage runs it with `HOME` below `out/`.)
#[must_use]
pub fn dir() -> Option<PathBuf> {
    #[cfg(feature = "test-switches")]
    {
        if let Ok(v) = crate::test_switch("DS_ARA_USER_DIR") {
            if !v.is_empty() {
                return Some(PathBuf::from(v));
            }
        }
        if fault() != "real-path" {
            once_row("path-refused", json!({"event": "defaults_path_refused", "folder": FOLDER_WORD}));
            return None;
        }
    }
    real_dir()
}

fn real_dir() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok().filter(|h| !h.is_empty())?;
    Some(PathBuf::from(home).join("Library").join("Application Support").join(FOLDER))
}

/// The real folder (for the unit test that a test build never resolves it).
#[must_use]
pub fn real_folder() -> Option<PathBuf> {
    real_dir()
}

static ROWS: Mutex<Option<HashSet<String>>> = Mutex::new(None);

/// One report row per key (a file that stays bad writes one row, not one per track).
fn once_row(key: &str, row: Value) {
    let first = {
        let mut g = ROWS.lock().unwrap_or_else(|e| e.into_inner());
        g.get_or_insert_with(HashSet::new).insert(key.to_string())
    };
    if first {
        crate::session().log(row.to_string());
    }
}

// ---------------------------------------------------------------- the checked values

/// What a role entry holds: the notes (1 to 8) and the detection settings (the keys of the table of the note).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RoleEntry {
    pub notes: Vec<u8>,
    pub params: Map<String, Value>,
}

/// The checked content of a defaults file or a preset: only values that passed.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Checked {
    pub roles: BTreeMap<String, RoleEntry>,
    pub follow: Map<String, Value>,
    pub export: Map<String, Value>,
}

fn int_in(v: &Value, lo: i64, hi: i64) -> Option<i64> {
    v.as_i64().filter(|n| (lo..=hi).contains(n))
}

fn num_in(v: &Value, lo: f64, hi: f64) -> Option<f64> {
    v.as_f64().filter(|n| n.is_finite() && (lo..=hi).contains(n))
}

fn notes_of(entry: &Map<String, Value>, path: &str, bad: &mut Vec<String>) -> Vec<u8> {
    let list: Vec<&Value> = match (entry.get("notes"), entry.get("note")) {
        (Some(Value::Array(a)), _) => a.iter().collect(),
        (Some(_), _) => {
            bad.push(format!("{path}.notes"));
            return Vec::new();
        }
        (None, Some(n)) => vec![n],
        (None, None) => return Vec::new(),
    };
    if list.is_empty() || list.len() > MAX_NOTES {
        bad.push(format!("{path}.notes"));
        return Vec::new();
    }
    let mut out = Vec::new();
    for v in &list {
        match int_in(v, 0, 127) {
            Some(n) => out.push(n as u8),
            None => {
                bad.push(format!("{path}.notes"));
                return Vec::new();
            }
        }
    }
    out
}

/// The detection settings that may be defaults; every key alone, a bad key is dropped and named.
fn params_of(v: &Value, path: &str, bad: &mut Vec<String>) -> Map<String, Value> {
    let mut out = Map::new();
    let Some(m) = v.as_object() else {
        bad.push(path.to_string());
        return out;
    };
    for (k, val) in m {
        let ok = match k.as_str() {
            "thresholds" => val
                .as_array()
                .filter(|a| a.len() == 5 && a.iter().all(|x| num_in(x, 0.01, 0.85).is_some()))
                .is_some(),
            "min_gap_ms" => num_in(val, 1.0, 1000.0).is_some(),
            "refine" => val.is_boolean(),
            "vel_lo_db" | "vel_hi_db" | "vel_cut_db" => num_in(val, -96.0, 0.0).is_some(),
            _ => continue, // not a default (shift, click_freq, weak_signal, unknown): never read
        };
        if ok {
            out.insert(k.clone(), val.clone());
        } else {
            bad.push(format!("{path}.{k}"));
        }
    }
    // the ramp needs lo below hi, and a cut below the ceiling
    let get = |m: &Map<String, Value>, k: &str| m.get(k).and_then(Value::as_f64);
    if let (Some(lo), Some(hi)) = (get(&out, "vel_lo_db"), get(&out, "vel_hi_db")) {
        if lo >= hi {
            out.shift_remove("vel_lo_db");
            out.shift_remove("vel_hi_db");
            bad.push(format!("{path}.vel_lo_db"));
        }
    }
    if let (Some(cut), hi) = (get(&out, "vel_cut_db"), get(&out, "vel_hi_db").unwrap_or(-6.0)) {
        if cut >= hi {
            out.shift_remove("vel_cut_db");
            bad.push(format!("{path}.vel_cut_db"));
        }
    }
    out
}

/// The `roles` object of a defaults file or a preset.
pub fn roles_of(v: &Value, base: &str, bad: &mut Vec<String>) -> BTreeMap<String, RoleEntry> {
    let mut out = BTreeMap::new();
    let Some(m) = v.as_object() else {
        bad.push(base.to_string());
        return out;
    };
    for (role, entry) in m {
        if !ROLES.contains(&role.as_str()) {
            continue; // kept as unknown data in the raw file, never used
        }
        let path = format!("{base}.{role}");
        let Some(e) = entry.as_object() else {
            bad.push(path);
            continue;
        };
        let notes = notes_of(e, &path, bad);
        let params = e.get("params").map(|p| params_of(p, &format!("{path}.params"), bad)).unwrap_or_default();
        if !notes.is_empty() || !params.is_empty() {
            out.insert(role.clone(), RoleEntry { notes, params });
        }
    }
    out
}

fn follow_of(v: &Value, bad: &mut Vec<String>) -> Map<String, Value> {
    let mut out = Map::new();
    let Some(m) = v.as_object() else {
        bad.push("follow".into());
        return out;
    };
    for (k, val) in m {
        let ok = match k.as_str() {
            "mode" => val.as_str().is_some_and(|s| ["smooth", "page", "off"].contains(&s)),
            "suspend_on_edit" | "host_selection" => val.is_boolean(),
            _ => continue,
        };
        if ok {
            out.insert(k.clone(), val.clone());
        } else {
            bad.push(format!("follow.{k}"));
        }
    }
    out
}

fn export_of(v: &Value, bad: &mut Vec<String>) -> Map<String, Value> {
    let mut out = Map::new();
    let Some(m) = v.as_object() else {
        bad.push("export".into());
        return out;
    };
    for (k, val) in m {
        let ok = match k.as_str() {
            "formats" => val.as_array().is_some_and(|a| {
                let names: Vec<&str> = a.iter().filter_map(Value::as_str).collect();
                names.len() == a.len()
                    && !names.is_empty()
                    && names.iter().all(|n| ["midi", "spikes", "audio"].contains(n))
                    && names.iter().collect::<HashSet<_>>().len() == names.len()
            }),
            "midi_mode" => val.as_str().is_some_and(|s| s == "per_track" || s == "single"),
            "compensate" => val.is_boolean(),
            _ => continue,
        };
        if ok {
            out.insert(k.clone(), val.clone());
        } else {
            bad.push(format!("export.{k}"));
        }
    }
    out
}

/// Check the parsed JSON of a defaults file. The names of the bad fields are in `bad`.
#[must_use]
pub fn check_defaults(raw: &Value, bad: &mut Vec<String>) -> Checked {
    let mut c = Checked::default();
    let Some(m) = raw.as_object() else {
        bad.push("file".into());
        return c;
    };
    if let Some(r) = m.get("roles") {
        c.roles = roles_of(r, "roles", bad);
    }
    if let Some(f) = m.get("follow") {
        c.follow = follow_of(f, bad);
    }
    if let Some(e) = m.get("export") {
        c.export = export_of(e, bad);
    }
    c
}

// ---------------------------------------------------------------- reading a file

/// How a read ended.
#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    /// The file was read (some fields may be bad: `bad`).
    Ok,
    /// No file (the normal case before the first Save).
    None,
    /// A file that is not usable: the reason in words, built-in values apply.
    Damaged(String),
    /// The file is from a newer plugin: read, never written.
    Newer,
}

/// A defaults file as it was read.
#[derive(Debug, Clone)]
pub struct Loaded {
    pub status: Status,
    pub checked: Checked,
    /// The names of the fields that failed their range check.
    pub bad: Vec<String>,
    /// The whole object of the file, for a rewrite that keeps the fields this version does not know.
    pub raw: Map<String, Value>,
}

impl Loaded {
    fn empty(status: Status) -> Loaded {
        Loaded {
            status,
            checked: Checked::default(),
            bad: Vec::new(),
            raw: Map::new(),
        }
    }
}

/// Read one plain file of at most `max` bytes. Err: the reason (a link, a folder, too big, unreadable); `Ok(None)` when it is not there.
fn read_plain(path: &Path, max: u64) -> Result<Option<Vec<u8>>, String> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("unreadable".into()),
    };
    if !meta.is_file() {
        return Err("not a plain file".into());
    }
    if meta.len() > max {
        return Err("too big".into());
    }
    std::fs::read(path).map(Some).map_err(|_| "unreadable".to_string())
}

/// Parse and check the bytes of a defaults file or a preset.
fn parse(bytes: &[u8], check: impl FnOnce(&Value, &mut Vec<String>) -> Checked) -> Loaded {
    let Ok(v) = serde_json::from_slice::<Value>(bytes) else {
        return Loaded::empty(Status::Damaged("not JSON".into()));
    };
    let Some(m) = v.as_object() else {
        return Loaded::empty(Status::Damaged("not an object".into()));
    };
    let Some(ver) = m.get("v").and_then(Value::as_i64).filter(|n| *n >= 1) else {
        return Loaded::empty(Status::Damaged("no valid version".into()));
    };
    let mut bad = Vec::new();
    let checked = check(&v, &mut bad);
    Loaded {
        status: if ver > VERSION { Status::Newer } else { Status::Ok },
        checked,
        bad,
        raw: m.clone(),
    }
}

/// Read `<dir>/defaults.json`. Never an error; one report row for a file that is bad.
#[must_use]
pub fn load_at(dir: &Path) -> Loaded {
    let path = dir.join("defaults.json");
    let bytes = match read_plain(&path, MAX_BYTES) {
        Ok(Some(b)) => b,
        Ok(None) => return Loaded::empty(Status::None),
        Err(why) => {
            let l = Loaded::empty(Status::Damaged(why.clone()));
            once_row(
                &format!("unreadable:{why}:{}", stamp(&path)),
                json!({"event": "defaults_invalid", "folder": FOLDER_WORD, "file": "defaults.json", "why": why}),
            );
            return l;
        }
    };
    let l = if fault() == "trust" {
        trusting(&bytes)
    } else {
        parse(&bytes, check_defaults)
    };
    report(&l, "defaults.json", &path);
    l
}

/// The fault run `trust`: no range check (a bad note reaches the record).
fn trusting(bytes: &[u8]) -> Loaded {
    let mut l = parse(bytes, |_, _| Checked::default());
    if let Ok(Value::Object(m)) = serde_json::from_slice::<Value>(bytes) {
        if let Some(Value::Object(roles)) = m.get("roles") {
            for (role, e) in roles {
                let notes: Vec<u8> = e
                    .get("note")
                    .or_else(|| e.get("notes").and_then(|n| n.get(0)))
                    .and_then(Value::as_i64)
                    .map(|n| vec![n as u8])
                    .unwrap_or_default();
                l.checked.roles.insert(role.clone(), RoleEntry { notes, params: Map::new() });
            }
        }
    }
    l
}

fn stamp(path: &Path) -> String {
    std::fs::symlink_metadata(path)
        .ok()
        .map(|m| {
            let t = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_nanos());
            format!("{}:{t}", m.len())
        })
        .unwrap_or_default()
}

/// ONE row for a file that is damaged or has bad fields, keyed by the size and time of the file.
fn report(l: &Loaded, name: &str, path: &Path) {
    let (why, first): (Option<String>, Vec<&String>) = match &l.status {
        Status::Damaged(w) => (Some(w.clone()), Vec::new()),
        _ if !l.bad.is_empty() => (Some("bad fields".into()), l.bad.iter().take(5).collect()),
        _ => (None, Vec::new()),
    };
    if let Some(why) = why {
        once_row(
            &format!("invalid:{name}:{}", stamp(path)),
            json!({"event": "defaults_invalid", "folder": FOLDER_WORD, "file": name, "why": why,
                   "count": l.bad.len(), "first": first}),
        );
    }
}

static CACHE: Mutex<Option<(PathBuf, String, Arc<Loaded>)>> = Mutex::new(None);

/// The defaults as they are now, read again only when the file changed (size and time).
#[must_use]
pub fn current() -> Arc<Loaded> {
    let Some(dir) = dir() else {
        return Arc::new(Loaded::empty(Status::None));
    };
    let key = stamp(&dir.join("defaults.json"));
    let mut g = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((d, k, l)) = g.as_ref() {
        if *d == dir && *k == key {
            return l.clone();
        }
    }
    let l = Arc::new(load_at(&dir));
    *g = Some((dir, key, l.clone()));
    l
}

// ---------------------------------------------------------------- the choice of a note

/// The note a NEW track of a role takes: the first of `notes` that no other track of the role uses; when all are used, the last.
#[must_use]
pub fn pick_note(notes: &[u8], used: &[i64]) -> Option<u8> {
    notes
        .iter()
        .find(|n| !used.contains(&i64::from(**n)))
        .or(notes.last())
        .copied()
}

// ---------------------------------------------------------------- writing

fn write_json(path: &Path, v: &Value) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(v).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("the file would be too big".into());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    if fault() == "in-place" {
        // the fault run: not atomic (a reader can see a half file)
        use std::io::Write;
        let mut f = std::fs::File::create(path).map_err(|e| e.to_string())?;
        for chunk in bytes.chunks(64) {
            f.write_all(chunk).map_err(|e| e.to_string())?;
            f.flush().ok();
            std::thread::sleep(std::time::Duration::from_micros(200));
        }
        return Ok(());
    }
    drumspike_core::io::write_atomic(path, &bytes).map_err(|e| e.to_string())
}

static WRITE: Mutex<()> = Mutex::new(());

fn save_error(e: &str) -> String {
    format!("the defaults could not be saved: {e}")
}

fn role_json(e: &RoleEntry) -> Value {
    let mut m = Map::new();
    if !e.notes.is_empty() {
        m.insert("notes".into(), json!(e.notes));
    }
    if !e.params.is_empty() {
        m.insert("params".into(), Value::Object(e.params.clone()));
    }
    Value::Object(m)
}

/// Read the file now, change ONLY the role (`Some`: set it, `None`: remove it), keep every other field, write atomically.
///
/// # Errors
/// The text for the status line: a file of a newer plugin, a role that is no role, a write that failed.
pub fn save_role_at(dir: &Path, role: &str, entry: Option<&RoleEntry>) -> Result<(), String> {
    if !ROLES.contains(&role) {
        return Err(save_error("no such role"));
    }
    let _g = WRITE.lock().unwrap_or_else(|e| e.into_inner());
    let path = dir.join("defaults.json");
    let cur = load_at(dir);
    if cur.status == Status::Newer {
        return Err("The defaults file is from a newer version of the plugin. Update the plugin, or move the file.".into());
    }
    let mut raw = cur.raw;
    if let Status::Damaged(_) = cur.status {
        // keep the user's text once
        if path.is_file() {
            let _ = std::fs::copy(&path, dir.join("defaults.bad.json"));
        }
        raw = Map::new();
    }
    raw.insert("v".into(), json!(VERSION));
    let mut roles = match raw.remove("roles") {
        Some(Value::Object(m)) => m,
        _ => Map::new(),
    };
    match entry {
        Some(e) => {
            // unknown fields of the role are kept
            let mut base = roles.get(role).and_then(Value::as_object).cloned().unwrap_or_default();
            base.remove("note");
            if let Value::Object(n) = role_json(e) {
                base.remove("notes");
                base.remove("params");
                for (k, v) in n {
                    base.insert(k, v);
                }
            }
            roles.insert(role.to_string(), Value::Object(base));
        }
        None => {
            roles.remove(role);
        }
    }
    if fault() == "all-roles" {
        if let Some(e) = entry {
            for r in ROLES {
                roles.insert(r.to_string(), role_json(e));
            }
        }
    }
    raw.insert("roles".into(), Value::Object(roles));
    write_json(&path, &Value::Object(raw)).map_err(|e| save_error(&e))
}

/// Make `s` the content of the `follow` or `export` part of the file (the page has no button for it yet; the file may be edited by hand).
///
/// # Errors
/// As [`save_role_at`].
pub fn save_part_at(dir: &Path, part: &str, value: Value) -> Result<(), String> {
    if part != "follow" && part != "export" {
        return Err(save_error("no such part"));
    }
    let _g = WRITE.lock().unwrap_or_else(|e| e.into_inner());
    let cur = load_at(dir);
    if cur.status == Status::Newer {
        return Err("The defaults file is from a newer version of the plugin. Update the plugin, or move the file.".into());
    }
    let mut raw = if matches!(cur.status, Status::Damaged(_)) { Map::new() } else { cur.raw };
    raw.insert("v".into(), json!(VERSION));
    raw.insert(part.into(), value);
    write_json(&dir.join("defaults.json"), &Value::Object(raw)).map_err(|e| save_error(&e))
}

// ---------------------------------------------------------------- presets

/// The safe file stem of a preset name: the rule of the delivery names (`deliver::safe_stem`), nothing else.
#[must_use]
pub fn preset_stem(name: &str) -> Option<String> {
    crate::deliver::safe_stem(name.trim())
}

fn key_of(stem: &str) -> String {
    drumspike_app::export::file_name_key(stem)
}

/// The built-in General MIDI map. It is code, not a file.
#[must_use]
pub fn general_midi() -> Checked {
    let mut c = Checked::default();
    let mut put = |r: &str, n: &[u8]| {
        c.roles.insert(r.into(), RoleEntry { notes: n.to_vec(), params: Map::new() });
    };
    put("kick", &[36]);
    put("snare", &[38]);
    put("tom", &[50, 48, 47, 45, 43, 41]);
    put("hat", &[42]);
    put("cymbal", &[49]);
    c
}

/// A preset as the page shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Preset {
    pub name: String,
    pub builtin: bool,
    pub checked: Checked,
}

fn presets_dir(dir: &Path) -> PathBuf {
    dir.join("Presets")
}

/// The presets: the built-in one first, then the files by name. A file that fails is skipped with one row.
#[must_use]
pub fn list_at(dir: &Path) -> Vec<Preset> {
    let mut out = vec![Preset {
        name: BUILTIN.into(),
        builtin: true,
        checked: general_midi(),
    }];
    let pdir = presets_dir(dir);
    let Ok(rd) = std::fs::read_dir(&pdir) else {
        return out;
    };
    let mut files: Vec<PathBuf> = rd
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    let mut seen = HashSet::new();
    for p in files.into_iter().take(MAX_PRESETS) {
        match read_preset(&p) {
            Some(pr) if seen.insert(key_of(&pr.name)) && key_of(&pr.name) != key_of(BUILTIN) => out.push(pr),
            Some(_) => {}
            None => {}
        }
    }
    out[1..].sort_by_key(|p| p.name.to_lowercase());
    out
}

fn read_preset(path: &Path) -> Option<Preset> {
    let fname = path.file_name()?.to_string_lossy().into_owned();
    let bytes = match read_plain(path, PRESET_BYTES) {
        Ok(Some(b)) => b,
        Ok(None) => return None,
        Err(why) => {
            once_row(
                &format!("preset:{fname}:{why}"),
                json!({"event": "preset_invalid", "folder": FOLDER_WORD, "why": why}),
            );
            return None;
        }
    };
    let l = parse(&bytes, |v, bad| {
        let mut c = Checked::default();
        if let Some(r) = v.get("roles") {
            c.roles = roles_of(r, "roles", bad);
        }
        c
    });
    let name = l
        .raw
        .get("name")
        .and_then(Value::as_str)
        .map(|s| s.trim().chars().take(80).collect::<String>())
        .filter(|s| !s.is_empty());
    match (&l.status, name) {
        (Status::Ok | Status::Newer, Some(name)) => {
            if !l.bad.is_empty() {
                once_row(
                    &format!("preset-bad:{fname}:{}", stamp(path)),
                    json!({"event": "preset_invalid", "folder": FOLDER_WORD, "why": "bad fields", "count": l.bad.len()}),
                );
            }
            Some(Preset { name, builtin: false, checked: l.checked })
        }
        _ => {
            once_row(
                &format!("preset:{fname}:{}", stamp(path)),
                json!({"event": "preset_invalid", "folder": FOLDER_WORD, "why": "not a preset"}),
            );
            None
        }
    }
}

/// A preset by name (the built-in one or a file).
#[must_use]
pub fn get_at(dir: &Path, name: &str) -> Option<Preset> {
    list_at(dir).into_iter().find(|p| key_of(&p.name) == key_of(name.trim()))
}

/// Write a preset.
///
/// # Errors
/// The text for the page: a bad or reserved name, too many presets, a failed write.
pub fn save_preset_at(dir: &Path, name: &str, roles: &BTreeMap<String, RoleEntry>) -> Result<(), String> {
    let name = name.trim();
    let shown: String = name.chars().take(80).collect();
    let Some(stem) = preset_stem(&shown).filter(|_| !shown.is_empty()) else {
        return Err("Type a name for the preset.".into());
    };
    if key_of(&stem) == key_of(BUILTIN) || key_of(&shown) == key_of(BUILTIN) {
        return Err(format!("A preset named {BUILTIN} is built in. Choose another name."));
    }
    let _g = WRITE.lock().unwrap_or_else(|e| e.into_inner());
    let path = presets_dir(dir).join(format!("{stem}.json"));
    // a file of another name that has the same stem key is the same file on this file system: refuse a link or a folder at the path
    if let Ok(m) = std::fs::symlink_metadata(&path) {
        if !m.is_file() {
            return Err(save_error("not a plain file"));
        }
    } else if std::fs::read_dir(presets_dir(dir)).map_or(0, |r| r.filter_map(Result::ok).count()) >= MAX_PRESETS {
        return Err(format!("There are {MAX_PRESETS} presets already. Delete one first."));
    }
    let mut obj = Map::new();
    // the unknown fields of a file that is written again are kept
    if let Ok(Some(b)) = read_plain(&path, PRESET_BYTES) {
        if let Ok(Value::Object(old)) = serde_json::from_slice::<Value>(&b) {
            obj = old;
        }
    }
    obj.insert("v".into(), json!(VERSION));
    obj.insert("name".into(), json!(shown));
    let roles_json: Map<String, Value> = roles.iter().map(|(k, v)| (k.clone(), role_json(v))).collect();
    obj.insert("roles".into(), Value::Object(roles_json));
    write_json(&path, &Value::Object(obj)).map_err(|e| save_error(&e))
}

/// Remove a preset file. The built-in preset cannot be removed.
///
/// # Errors
/// The text for the page.
pub fn delete_preset_at(dir: &Path, name: &str) -> Result<(), String> {
    if key_of(name.trim()) == key_of(BUILTIN) {
        return Err(format!("{BUILTIN} is built in and cannot be deleted."));
    }
    let Some(stem) = preset_stem(name) else {
        return Err("No such preset.".into());
    };
    let path = presets_dir(dir).join(format!("{stem}.json"));
    match std::fs::symlink_metadata(&path) {
        Ok(m) if m.is_file() => std::fs::remove_file(&path).map_err(|e| format!("The preset could not be deleted: {e}")),
        _ => Err("No such preset.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("drumspike-ara-ud-{tag}-{}-{}", std::process::id(), crate::service::new_doc_id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn write(dir: &Path, text: &str) {
        std::fs::write(dir.join("defaults.json"), text).unwrap();
    }

    #[test]
    fn a_good_file_is_read_and_a_single_note_is_a_list_of_one() {
        let d = scratch("good");
        write(
            &d,
            r#"{"v":1,"roles":{"snare":{"note":40,"params":{"min_gap_ms":30,"refine":false,"vel_lo_db":-50,"vel_hi_db":-10}},
                "tom":{"notes":[50,48,47]}},"follow":{"mode":"page","suspend_on_edit":true},"export":{"formats":["midi"],"midi_mode":"single","compensate":false}}"#,
        );
        let l = load_at(&d);
        assert_eq!(l.status, Status::Ok);
        assert!(l.bad.is_empty(), "{:?}", l.bad);
        assert_eq!(l.checked.roles["snare"].notes, [40]);
        assert_eq!(l.checked.roles["tom"].notes, [50, 48, 47]);
        assert_eq!(l.checked.roles["snare"].params["min_gap_ms"], 30);
        assert_eq!(l.checked.follow["mode"], "page");
        assert_eq!(l.checked.export["midi_mode"], "single");
    }

    #[test]
    fn a_missing_folder_or_file_is_none_and_nothing_is_made() {
        let d = scratch("none").join("not-there");
        assert_eq!(load_at(&d).status, Status::None);
        assert!(!d.exists(), "a read makes no folder");
    }

    #[test]
    fn a_hostile_file_gives_built_in_values_for_the_bad_fields_only() {
        let d = scratch("hostile");
        let big = format!(r#"{{"v":1,"pad":"{}"}}"#, "x".repeat(300_000));
        let cases: Vec<(&str, String)> = vec![
            ("not json", "{{{".into()),
            ("array", "[1,2]".into()),
            ("no version", r#"{"roles":{}}"#.into()),
            ("version 0", r#"{"v":0,"roles":{"kick":{"note":40}}}"#.into()),
            ("version text", r#"{"v":"1","roles":{"kick":{"note":40}}}"#.into()),
            ("too big", big),
            ("deep", format!("{}1{}", "[".repeat(5000), "]".repeat(5000))),
        ];
        for (why, text) in cases {
            write(&d, &text);
            let l = load_at(&d);
            assert!(matches!(l.status, Status::Damaged(_)), "{why}: {:?}", l.status);
            assert!(l.checked.roles.is_empty(), "{why}");
        }
        // field by field: the good ones stay
        write(
            &d,
            r#"{"v":1,"roles":{"kick":{"note":300,"params":{"min_gap_ms":20}},"snare":{"notes":[38.5]},"tom":{"notes":[1,2,3,4,5,6,7,8,9]},
                "hat":{"notes":[]},"cymbal":{"note":"49"},"other":{"params":{"thresholds":[0.2,0.2,0.2,0.2]}},"bogus":{"note":1},
                "snare2":5},"follow":{"mode":"zoom","suspend_on_edit":1},"export":{"formats":["midi","midi"],"midi_mode":"x","compensate":"y"}}"#,
        );
        let l = load_at(&d);
        assert_eq!(l.status, Status::Ok);
        assert!(l.checked.roles["kick"].notes.is_empty(), "the note 300 is dropped");
        assert_eq!(l.checked.roles["kick"].params["min_gap_ms"], 20, "the good field of the same role stays");
        for r in ["snare", "tom", "hat", "cymbal", "other", "bogus"] {
            assert!(!l.checked.roles.contains_key(r), "{r}");
        }
        assert!(l.checked.follow.is_empty() && l.checked.export.is_empty());
        assert!(l.bad.len() >= 9, "{:?}", l.bad);
    }

    #[test]
    fn the_ranges_of_the_velocity_setting_and_the_thresholds() {
        let mut bad = Vec::new();
        let p = params_of(&json!({"vel_lo_db": -10, "vel_hi_db": -20, "vel_cut_db": -5, "thresholds": [0.2,0.2,0.2,0.2,0.9]}), "p", &mut bad);
        assert!(p.is_empty(), "{p:?}");
        let p = params_of(&json!({"vel_lo_db": -50, "vel_hi_db": -10, "vel_cut_db": -60, "shift_samples": 4, "min_gap_ms": f64::MAX}), "p", &mut bad);
        assert_eq!(p.len(), 3, "the shift is no default and the gap is out of range: {p:?}");
        let mut bad = Vec::new();
        let p = params_of(&json!({"vel_lo_db": -50, "vel_hi_db": -10, "vel_cut_db": -5}), "p", &mut bad);
        assert!(!p.contains_key("vel_cut_db"), "a cut above the ceiling is dropped");
    }

    #[test]
    fn a_link_a_folder_and_an_unreadable_file_are_none_and_never_followed() {
        let d = scratch("link");
        let target = d.join("real.json");
        std::fs::write(&target, r#"{"v":1,"roles":{"kick":{"note":40}}}"#).unwrap();
        std::os::unix::fs::symlink(&target, d.join("defaults.json")).unwrap();
        let l = load_at(&d);
        assert!(matches!(l.status, Status::Damaged(_)) && l.checked.roles.is_empty(), "a link is not followed");
        std::fs::remove_file(d.join("defaults.json")).unwrap();
        std::fs::create_dir(d.join("defaults.json")).unwrap();
        assert!(matches!(load_at(&d).status, Status::Damaged(_)), "a folder");
        std::fs::remove_dir(d.join("defaults.json")).unwrap();
        write(&d, r#"{"v":1}"#);
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(d.join("defaults.json"), std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read(d.join("defaults.json")).is_err() {
            assert!(matches!(load_at(&d).status, Status::Damaged(_)), "unreadable");
        }
        std::fs::set_permissions(d.join("defaults.json"), std::fs::Permissions::from_mode(0o644)).unwrap();
    }

    #[test]
    fn a_newer_file_is_read_and_never_written() {
        let d = scratch("newer");
        let text = r#"{"v":2,"roles":{"kick":{"note":40}},"future":{"a":1}}"#;
        write(&d, text);
        let l = load_at(&d);
        assert_eq!(l.status, Status::Newer);
        assert_eq!(l.checked.roles["kick"].notes, [40]);
        let e = save_role_at(&d, "snare", Some(&RoleEntry { notes: vec![38], params: Map::new() })).unwrap_err();
        assert!(e.contains("newer version"), "{e}");
        assert_eq!(std::fs::read_to_string(d.join("defaults.json")).unwrap(), text, "the file is as it was");
    }

    #[test]
    fn a_save_changes_one_role_and_keeps_every_unknown_field() {
        let d = scratch("save");
        write(&d, r#"{"v":1,"future":{"a":1},"roles":{"kick":{"notes":[36],"extra":true},"hat":{"notes":[42]},"alien":{"x":1}}}"#);
        let mut params = Map::new();
        params.insert("min_gap_ms".into(), json!(25));
        save_role_at(&d, "kick", Some(&RoleEntry { notes: vec![35], params })).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(d.join("defaults.json")).unwrap()).unwrap();
        assert_eq!(v["future"]["a"], 1);
        assert_eq!(v["roles"]["kick"]["notes"], json!([35]));
        assert_eq!(v["roles"]["kick"]["extra"], true, "a field of the role that we do not know stays");
        assert_eq!(v["roles"]["hat"]["notes"], json!([42]), "another role is not touched");
        assert_eq!(v["roles"]["alien"]["x"], 1);
        save_role_at(&d, "kick", None).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(d.join("defaults.json")).unwrap()).unwrap();
        assert!(v["roles"].get("kick").is_none() && v["roles"].get("hat").is_some());
    }

    #[test]
    fn a_damaged_file_is_copied_once_before_it_is_replaced() {
        let d = scratch("bad");
        write(&d, "my own text");
        save_role_at(&d, "snare", Some(&RoleEntry { notes: vec![38], params: Map::new() })).unwrap();
        assert_eq!(std::fs::read_to_string(d.join("defaults.bad.json")).unwrap(), "my own text");
        assert_eq!(load_at(&d).checked.roles["snare"].notes, [38]);
    }

    #[test]
    fn a_save_into_a_folder_that_cannot_be_written_says_why_and_breaks_nothing() {
        let d = scratch("ro");
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o500)).unwrap();
        let r = save_role_at(&d, "snare", Some(&RoleEntry { notes: vec![38], params: Map::new() }));
        let writable = std::fs::write(d.join("probe"), b"x").is_ok(); // root ignores the mode
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o700)).unwrap();
        if !writable {
            let e = r.unwrap_err();
            assert!(e.starts_with("the defaults could not be saved: "), "{e}");
        }
        assert_eq!(load_at(&d).status, Status::None);
    }

    #[test]
    fn a_note_is_the_first_that_no_other_track_of_the_role_uses_and_the_last_when_all_are_used() {
        let toms = [50u8, 48, 47];
        assert_eq!(pick_note(&toms, &[]), Some(50));
        assert_eq!(pick_note(&toms, &[50]), Some(48));
        assert_eq!(pick_note(&toms, &[48, 50]), Some(47));
        assert_eq!(pick_note(&toms, &[50, 48, 47]), Some(47));
        // more toms than notes: five new toms in one round
        let mut used: Vec<i64> = Vec::new();
        let mut got = Vec::new();
        for _ in 0..5 {
            let n = pick_note(&toms, &used).unwrap();
            used.push(i64::from(n));
            got.push(n);
        }
        assert_eq!(got, [50, 48, 47, 47, 47]);
        assert_eq!(pick_note(&[], &[]), None);
        assert_eq!(pick_note(&[38], &[38]), Some(38));
    }

    #[test]
    fn the_general_midi_preset_is_built_in_and_cannot_be_overwritten_or_deleted() {
        let d = scratch("gm");
        let list = list_at(&d);
        assert_eq!(list[0].name, BUILTIN);
        assert!(list[0].builtin);
        assert_eq!(list[0].checked.roles["kick"].notes, [36]);
        assert_eq!(list[0].checked.roles["tom"].notes, [50, 48, 47, 45, 43, 41]);
        assert!(!list[0].checked.roles.contains_key("other"));
        for name in ["General MIDI", "general midi", " GENERAL MIDI "] {
            let e = save_preset_at(&d, name, &BTreeMap::new()).unwrap_err();
            assert!(e.contains("built in"), "{name}: {e}");
        }
        assert!(delete_preset_at(&d, BUILTIN).is_err());
        // a file that shadows it is not listed
        std::fs::create_dir_all(d.join("Presets")).unwrap();
        std::fs::write(d.join("Presets/gm.json"), r#"{"v":1,"name":"General MIDI","roles":{"kick":{"note":1}}}"#).unwrap();
        assert_eq!(list_at(&d).len(), 1);
    }

    #[test]
    fn presets_are_saved_listed_loaded_and_deleted_by_a_safe_name() {
        let d = scratch("pre");
        let mut roles = BTreeMap::new();
        roles.insert("kick".to_string(), RoleEntry { notes: vec![35], params: Map::new() });
        assert!(save_preset_at(&d, "  ", &roles).is_err());
        save_preset_at(&d, "My Kit/../x", &roles).unwrap();
        let files: Vec<_> = std::fs::read_dir(d.join("Presets")).unwrap().filter_map(Result::ok).collect();
        assert_eq!(files.len(), 1);
        assert!(!files[0].file_name().to_string_lossy().contains('/'));
        let list = list_at(&d);
        assert_eq!(list.len(), 2);
        assert_eq!(list[1].name, "My Kit/../x", "the typed name is kept inside the file");
        assert_eq!(get_at(&d, "my kit/../x").unwrap().checked.roles["kick"].notes, [35]);
        delete_preset_at(&d, "My Kit/../x").unwrap();
        assert_eq!(list_at(&d).len(), 1);
        assert!(delete_preset_at(&d, "nothing").is_err());
    }

    #[test]
    fn a_preset_file_that_is_a_link_a_folder_or_hostile_is_skipped_and_the_list_goes_on() {
        let d = scratch("prehostile");
        let p = d.join("Presets");
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(p.join("good.json"), r#"{"v":1,"name":"Good","roles":{"snare":{"note":38}}}"#).unwrap();
        std::fs::write(p.join("junk.json"), "{{").unwrap();
        std::fs::write(p.join("big.json"), "x".repeat(70_000)).unwrap();
        std::fs::write(p.join("noname.json"), r#"{"v":1,"roles":{}}"#).unwrap();
        std::fs::write(p.join("target.txt"), r#"{"v":1,"name":"Linked","roles":{}}"#).unwrap();
        std::os::unix::fs::symlink(p.join("target.txt"), p.join("link.json")).unwrap();
        std::fs::create_dir(p.join("dir.json")).unwrap();
        let names: Vec<String> = list_at(&d).into_iter().map(|x| x.name).collect();
        assert_eq!(names, [BUILTIN, "Good"]);
    }

    #[test]
    fn at_most_two_hundred_presets_are_listed() {
        let d = scratch("many");
        let p = d.join("Presets");
        std::fs::create_dir_all(&p).unwrap();
        for i in 0..230 {
            std::fs::write(p.join(format!("p{i:03}.json")), format!(r#"{{"v":1,"name":"P{i:03}","roles":{{}}}}"#)).unwrap();
        }
        assert_eq!(list_at(&d).len(), 1 + MAX_PRESETS);
        let e = save_preset_at(&d, "one more", &BTreeMap::new()).unwrap_err();
        assert!(e.contains("200"), "{e}");
    }

    #[test]
    fn a_test_build_never_resolves_the_real_folder() {
        // a gate build without the switch has no folder; with the switch it is the folder of the switch
        let real = real_folder();
        if let Some(r) = &real {
            if let Some(d) = dir() {
                assert_ne!(&d, r, "a test build must not point at the real folder");
            }
        }
    }

    #[test]
    fn many_threads_that_save_never_leave_a_half_file() {
        let d = scratch("race");
        write(&d, r#"{"v":1}"#);
        let stop = std::sync::atomic::AtomicBool::new(false);
        let bad = std::sync::atomic::AtomicUsize::new(0);
        std::thread::scope(|s| {
            s.spawn(|| {
                while !stop.load(std::sync::atomic::Ordering::SeqCst) {
                    let t = std::fs::read(d.join("defaults.json")).unwrap_or_default();
                    if serde_json::from_slice::<Value>(&t).is_err() {
                        bad.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    }
                }
            });
            let writers: Vec<_> = (0..8)
                .map(|i| {
                    let d = d.clone();
                    s.spawn(move || {
                        for k in 0..30 {
                            let role = ROLES[(i + k) % 5];
                            let _ = save_role_at(&d, role, Some(&RoleEntry { notes: vec![(30 + i) as u8], params: Map::new() }));
                        }
                    })
                })
                .collect();
            for w in writers {
                w.join().unwrap();
            }
            stop.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        assert_eq!(bad.load(std::sync::atomic::Ordering::SeqCst), 0, "a reader saw a half file");
        assert_eq!(load_at(&d).status, Status::Ok);
    }
}
