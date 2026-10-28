//! Detector plug-ins 2 (#387, `docs/design/detector-plugins.md` sections 3, 6 and 7, app
//! side): the folder scan and the manifest reader, the acceptance of a licence and the
//! commands the page calls.
//!
//! A plugin is one folder per plugin under
//! `~/Library/Application Support/Drum Spike/Detectors/<id>/` (`DRUMSPIKE_DETECTORS_DIR`
//! overrides the folder, for the tests). A manifest that misses a required field, or whose
//! executable does not exist, makes the plugin invisible in the UI and logs the reason. The
//! app never downloads a plugin; the folder scan is the only source.
//!
//! Commands (the page only; not on the agent surface):
//! | `plugins_list {sid?}` | the installed plugins; with `sid`, each row carries `accepted` |
//! | `plugin_accept {sid, id}` | record the acceptance of the licence in the project |

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::AppError;

/// The env var that moves the Detectors folder (the tests).
pub const DETECTORS_DIR_ENV: &str = "DRUMSPIKE_DETECTORS_DIR";

/// The manifest size limit (the design section 5).
pub const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;

/// The sample rates a plugin may list (the design section 3).
pub const SAMPLE_RATES: [u32; 5] = [44_100, 48_000, 88_200, 96_000, 192_000];

/// The manifest of one plugin (unknown fields are ignored).
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Manifest {
    /// Lowercase letters, digits and `-`, at most 40 characters. Same as the folder name.
    pub id: String,
    /// The name the UI shows, at most 60 characters.
    pub name: String,
    /// Semver-like `major.minor.patch`.
    pub version: String,
    /// The protocol major the executable speaks; 1 only in this design.
    pub protocol: u32,
    /// Path relative to the folder. One file.
    pub executable: String,
    /// Subset of `kick`, `snare`, `tom`, `hat`, `cymbal`, `other`.
    #[serde(default)]
    pub roles: Vec<String>,
    /// The rates the plugin accepts, from [`SAMPLE_RATES`].
    #[serde(default)]
    pub sample_rates: Vec<u32>,
    /// The licence name, for example `MIT`.
    pub licence: String,
    /// Path relative to the folder of the full licence text.
    pub licence_file: String,
    /// URL, informational.
    #[serde(default)]
    pub homepage: Option<String>,
}

/// One installed plugin: the manifest, the folder, the resolved executable and the licence text.
#[derive(Debug, Clone, PartialEq)]
pub struct Installed {
    /// The validated manifest.
    pub manifest: Manifest,
    /// The plugin folder (`Detectors/<id>/`).
    pub folder: PathBuf,
    /// The executable, resolved against the folder.
    pub executable: PathBuf,
    /// The text of `licence_file`.
    pub licence_text: String,
}

impl Installed {
    /// The SHA-256 of the licence text, hex (the project record of section 6).
    pub fn licence_sha256(&self) -> String {
        use sha2::{Digest, Sha256};
        let hash = Sha256::digest(self.licence_text.as_bytes());
        hash.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// True when the manifest lists this role.
    pub fn supports_role(&self, role: &str) -> bool {
        self.manifest.roles.iter().any(|r| r == role)
    }

    /// The rate the app passes: the file's own rate when the plugin lists it, else the first
    /// listed rate when the app can resample to it (48 or 96 kHz source to 44.1 kHz),
    /// else `None` (the track fails with a readable error).
    pub fn pass_rate(&self, sr: u32) -> Option<u32> {
        if self.manifest.sample_rates.contains(&sr) {
            return Some(sr);
        }
        let first = *self.manifest.sample_rates.first()?;
        if first == 44_100 && matches!(sr, 48_000 | 96_000) {
            return Some(first);
        }
        None
    }

    /// The free-form params of the request: the track's params without the app's own keys.
    pub fn request_params(params: &serde_json::Map<String, Value>) -> Value {
        const RESERVED: [&str; 14] = [
            "detector",
            "timeout_s",
            "min_gap_ms",
            "refine",
            "refine_lo_ms",
            "refine_hi_ms",
            "shift_samples",
            "vel_lo_db",
            "vel_hi_db",
            "vel_cut_db",
            "thresholds",
            "sensitivity",
            "weak_signal",
            "flam_split",
        ];
        let mut out = serde_json::Map::new();
        for (k, v) in params {
            if !RESERVED.contains(&k.as_str()) {
                out.insert(k.clone(), v.clone());
            }
        }
        Value::Object(out)
    }

    /// The per-track timeout: a param `timeout_s` can lower the 120 s default, not raise it.
    pub fn timeout_of(params: &serde_json::Map<String, Value>) -> u64 {
        params
            .get("timeout_s")
            .and_then(Value::as_u64)
            .map_or(drumspike_core::backend::DEFAULT_TIMEOUT_S, |t| {
                t.min(drumspike_core::backend::DEFAULT_TIMEOUT_S)
            })
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 40
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn valid_version(v: &str) -> bool {
    let parts: Vec<&str> = v.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

/// Load and validate one plugin folder: the manifest and the licence text. The error is the
/// reason the plugin is invisible (the design section 3).
pub fn load_one(folder: &Path) -> Result<Installed, String> {
    let manifest_path = folder.join("manifest.json");
    let meta = std::fs::metadata(&manifest_path).map_err(|e| format!("no manifest.json: {e}"))?;
    if meta.len() > MAX_MANIFEST_BYTES {
        return Err(format!(
            "manifest of {} bytes over the limit of {MAX_MANIFEST_BYTES}",
            meta.len()
        ));
    }
    let text = std::fs::read_to_string(&manifest_path).map_err(|e| format!("manifest unreadable: {e}"))?;
    let m: Manifest = serde_json::from_str(&text).map_err(|e| format!("invalid manifest: {e}"))?;
    if !valid_id(&m.id) {
        return Err(format!(
            "invalid id {:?} (lowercase letters, digits and -, at most 40)",
            m.id
        ));
    }
    let folder_id = folder.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    if m.id != folder_id {
        return Err(format!("id {:?} is not the folder name {:?}", m.id, folder_id));
    }
    if m.name.is_empty() || m.name.len() > 60 {
        return Err(format!("invalid name {:?} (1 to 60 characters)", m.name));
    }
    if !valid_version(&m.version) {
        return Err(format!("invalid version {:?} (major.minor.patch)", m.version));
    }
    if m.protocol != 1 {
        return Err(format!(
            "manifest speaks protocol {}, this app speaks protocol 1",
            m.protocol
        ));
    }
    if m.executable.is_empty() {
        return Err("no executable".into());
    }
    let roles_ok = ["kick", "snare", "tom", "hat", "cymbal", "other"];
    if m.roles.is_empty() || m.roles.iter().any(|r| !roles_ok.contains(&r.as_str())) {
        return Err("roles must be a non-empty subset of kick, snare, tom, hat, cymbal, other".into());
    }
    if m.sample_rates.is_empty() || m.sample_rates.iter().any(|r| !SAMPLE_RATES.contains(r)) {
        return Err(format!("sample_rates must be a non-empty subset of {SAMPLE_RATES:?}"));
    }
    if m.licence.is_empty() {
        return Err("no licence name".into());
    }
    if m.licence_file.is_empty() {
        return Err("no licence_file".into());
    }
    let executable = folder.join(&m.executable);
    if !executable.is_file() {
        return Err(format!("executable {:?} does not exist", m.executable));
    }
    let licence_text = std::fs::read_to_string(folder.join(&m.licence_file))
        .map_err(|e| format!("licence file {:?} unreadable: {e}", m.licence_file))?;
    Ok(Installed {
        manifest: m,
        folder: folder.to_path_buf(),
        executable,
        licence_text,
    })
}

/// The Detectors folder: `$DRUMSPIKE_DETECTORS_DIR`, else
/// `~/Library/Application Support/Drum Spike/Detectors`.
pub fn detectors_dir() -> PathBuf {
    if let Ok(d) = std::env::var(DETECTORS_DIR_ENV) {
        if !d.is_empty() {
            return PathBuf::from(d);
        }
    }
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home).join("Library/Application Support/Drum Spike/Detectors")
}

/// Every installed plugin, sorted by name. An invalid folder is invisible: the reason goes
/// to the log (the design section 3).
pub fn list() -> Vec<Installed> {
    let dir = detectors_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut folders: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    folders.sort();
    let mut out = Vec::new();
    for f in folders {
        match load_one(&f) {
            Ok(i) => out.push(i),
            Err(e) => {
                let name = f.file_name().and_then(|n| n.to_str()).unwrap_or("?");
                eprintln!("plugin {name} invisible: {e}");
            }
        }
    }
    out.sort_by(|a, b| {
        a.manifest
            .name
            .cmp(&b.manifest.name)
            .then(a.manifest.id.cmp(&b.manifest.id))
    });
    out
}

/// One installed plugin by id; `None` when it is not installed (an invalid manifest counts
/// as missing: the plugin is invisible).
pub fn find(id: &str) -> Option<Installed> {
    list().into_iter().find(|i| i.manifest.id == id)
}

/// The accepted licences of the open project (`doc.extra.plugin_licences`):
/// id -> `{name, licence, sha256}`.
pub fn accepted_of(doc_extra: &serde_json::Map<String, Value>) -> BTreeMap<String, Value> {
    doc_extra
        .get("plugin_licences")
        .and_then(Value::as_object)
        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default()
}

/// Record the acceptance of a plugin's licence in the project (the design section 5): the
/// acceptance, the licence name and the hash of the licence text.
pub fn record_acceptance(op: &mut crate::registry::OpenProject, installed: &Installed) -> Value {
    let entry = json!({
        "name": installed.manifest.name,
        "licence": installed.manifest.licence,
        "sha256": installed.licence_sha256(),
    });
    let extra = &mut op.project.doc.extra;
    let licences = extra
        .entry("plugin_licences".to_string())
        .or_insert_with(|| Value::Object(Default::default()));
    if let Some(m) = licences.as_object_mut() {
        m.insert(installed.manifest.id.clone(), entry.clone());
    }
    op.mark_dirty();
    entry
}

/// The executable path as the dialogs show it: the home part as `~` (no host user name in any
/// dialog; the guide picture must show `~/Library/Application Support/...`).
fn shown_path(p: &std::path::Path) -> String {
    let s = p.display().to_string();
    match std::env::var("HOME") {
        Ok(h) if !h.is_empty() && (s == h || s.starts_with(&format!("{h}/"))) => {
            format!("~{}", &s[h.len()..])
        }
        _ => s,
    }
}

/// `plugins_list {sid?}`: the installed plugins; with `sid`, each row carries `accepted`.
fn plugins_list(svc: &crate::Service, args: &Value) -> Result<Value, AppError> {
    let list = list();
    let accepted: Option<BTreeMap<String, Value>> = match args.get("sid").and_then(Value::as_str) {
        Some(sid) => {
            let op = svc.inner.registry.get(sid)?;
            let extra = crate::registry::lock(&op).project.doc.extra.clone();
            Some(accepted_of(&extra))
        }
        None => None,
    };
    let plugins: Vec<Value> = list
        .iter()
        .map(|i| {
            let mut row = json!({
                "id": i.manifest.id,
                "name": i.manifest.name,
                "version": i.manifest.version,
                "roles": i.manifest.roles,
                "sample_rates": i.manifest.sample_rates,
                "licence": i.manifest.licence,
                "licence_text": i.licence_text,
                "executable": shown_path(&i.executable),
            });
            if let Some(a) = &accepted {
                row["accepted"] = Value::Bool(a.contains_key(&i.manifest.id));
            }
            row
        })
        .collect();
    Ok(json!({ "plugins": plugins }))
}

/// `plugin_accept {sid, id}`: record the acceptance of the licence in the project.
fn plugin_accept(svc: &crate::Service, args: &Value) -> Result<Value, AppError> {
    let sid = args
        .get("sid")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::bad("missing argument `sid` (string)"))?;
    let id = args
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::bad("missing argument `id` (string)"))?;
    let installed = find(id).ok_or_else(|| AppError::not_found(format!("plugin {id} is not installed")))?;
    let op = svc.inner.registry.get(sid)?;
    let entry = record_acceptance(&mut crate::registry::lock(&op), &installed);
    Ok(json!({"accepted": true, "id": id, "licence": installed.manifest.licence, "sha256": entry["sha256"]}))
}

/// The dispatcher of the plug-in commands (`None` for a name this module does not run).
pub fn dispatch(svc: &crate::Service, cmd: &str, args: &Value) -> Option<Result<Value, AppError>> {
    match cmd {
        "plugins_list" => Some(plugins_list(svc, args)),
        "plugin_accept" => Some(plugin_accept(svc, args)),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
