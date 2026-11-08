//! #371: the agent fine-tune skill (`skills/drumspike-finetune/`) ships inside the app bundle (a Tauri resource) and one
//! command installs it where the agent looks for skills.
//!
//! * `agent_skill_install {target?}`: copies the bundled folder to `target` (default `~/.claude/skills/drumspike-finetune`,
//!   Claude Code's user skills), replaces an older copy, and writes the absolute path of the running binary into
//!   `SKILL.md` for the `<binary>` placeholders (the `<version>` placeholder gets the app version), so the skill needs
//!   no symlink. Answers `{installed, files, version}`; `installed` is the path.
//! * `agent_skill_status {target?}`: `{installed, path, version, bundled_version, current, binary, mcp_command}`;
//!   `version` is the one the installed `SKILL.md` carries (`null` when nothing is installed), `mcp_command` the one line
//!   that adds the MCP server for this binary.
//!
//! Two variables name the places for a test or a packaging step: `DRUMSPIKE_SKILL_SOURCE` (the bundled folder; else
//! `<app>/Contents/Resources/skills/drumspike-finetune` beside the running binary) and `DRUMSPIKE_SKILL_TARGET` (the default
//! target). The install refuses a target that exists, holds files and has no `SKILL.md`: it replaces a copy of the
//! skill, never another folder.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::Service;
use crate::error::AppError;

/// The commands of this module.
pub const AGENT_SKILL_COMMANDS: [&str; 2] = ["agent_skill_install", "agent_skill_status"];

/// The app version the bundled skill belongs to.
pub const BUNDLED_VERSION: &str = env!("CARGO_PKG_VERSION");

const SKILL: &str = "drumspike-finetune";
const BINARY_TAG: &str = "<binary>";
const VERSION_TAG: &str = "<version>";

/// The folder the bundle carries: the variable, else the resource folder beside the binary.
fn bundled_dir() -> Option<PathBuf> {
    if let Some(v) = std::env::var_os("DRUMSPIKE_SKILL_SOURCE").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(v));
    }
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?.parent()?.join("Resources").join("skills").join(SKILL);
    dir.is_dir().then_some(dir)
}

/// The default target: the variable, else Claude Code's user skills.
fn default_target() -> Result<PathBuf, AppError> {
    if let Some(v) = std::env::var_os("DRUMSPIKE_SKILL_TARGET").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(v));
    }
    let home = std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .ok_or_else(|| AppError::internal("no home folder: name a `target`"))?;
    Ok(PathBuf::from(home).join(".claude").join("skills").join(SKILL))
}

fn target_of(target: Option<&str>) -> Result<PathBuf, AppError> {
    match target {
        Some(t) if !t.trim().is_empty() => Ok(PathBuf::from(t)),
        _ => default_target(),
    }
}

fn binary_path() -> Result<PathBuf, AppError> {
    std::env::current_exe().map_err(|e| AppError::internal(format!("the path of the running binary: {e}")))
}

/// The one line that adds the MCP server for `binary`.
#[must_use]
pub fn mcp_command(binary: &Path) -> String {
    format!("claude mcp add --scope user drumspike -- \"{}\" mcp", binary.display())
}

/// The `version:` value of a `SKILL.md` text (`None` when it has none or still the placeholder).
fn version_of(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find_map(|l| l.strip_prefix("version:"))
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty() && v != VERSION_TAG)
}

/// Copy `from` to `to`, `SKILL.md` with the placeholders written; answers the number of files.
fn copy_tree(from: &Path, to: &Path, binary: &Path, version: &str) -> Result<usize, AppError> {
    let io = |e: std::io::Error| AppError::internal(format!("{}: {e}", to.display()));
    std::fs::create_dir_all(to).map_err(io)?;
    let mut files = 0;
    for entry in std::fs::read_dir(from).map_err(|e| AppError::internal(format!("{}: {e}", from.display())))? {
        let entry = entry.map_err(io)?;
        let (src, dst) = (entry.path(), to.join(entry.file_name()));
        if entry.file_type().map_err(io)?.is_dir() {
            files += copy_tree(&src, &dst, binary, version)?;
        } else if entry.file_name() == "SKILL.md" {
            let text = std::fs::read_to_string(&src).map_err(io)?;
            let text = text
                .replace(BINARY_TAG, &binary.display().to_string())
                .replace(VERSION_TAG, version);
            std::fs::write(&dst, text).map_err(io)?;
            files += 1;
        } else {
            std::fs::copy(&src, &dst).map_err(io)?;
            files += 1;
        }
    }
    Ok(files)
}

/// Install the skill folder `source` to `target` for `binary`; an older copy is replaced.
///
/// # Errors
///
/// 404 when `source` has no `SKILL.md`; 409 when `target` is a file or a folder that is not a copy of the skill; 500
/// when a file cannot be written.
pub fn install(source: &Path, target: &Path, binary: &Path, version: &str) -> Result<Value, AppError> {
    if !source.join("SKILL.md").is_file() {
        return Err(AppError::not_found(
            "the app carries no skill folder (no SKILL.md in the bundle)",
        ));
    }
    if target.exists() {
        let foreign = !target.is_dir()
            || (!target.join("SKILL.md").is_file() && std::fs::read_dir(target).is_ok_and(|mut d| d.next().is_some()));
        if foreign {
            return Err(AppError::conflict(format!(
                "{} exists and is not a copy of the skill: name another `target`",
                target.display()
            )));
        }
    }
    let name = target
        .file_name()
        .map_or_else(|| SKILL.into(), |n| n.to_string_lossy().into_owned());
    let stage = target.with_file_name(format!(".{name}.installing"));
    let _ = std::fs::remove_dir_all(&stage);
    let files = copy_tree(source, &stage, binary, version).inspect_err(|_| {
        let _ = std::fs::remove_dir_all(&stage);
    })?;
    if target.exists() {
        std::fs::remove_dir_all(target).map_err(|e| AppError::internal(format!("{}: {e}", target.display())))?;
    }
    std::fs::rename(&stage, target).map_err(|e| AppError::internal(format!("{}: {e}", target.display())))?;
    Ok(json!({ "installed": target.display().to_string(), "files": files, "version": version }))
}

/// The state of the copy at `target` for `binary` (see the module docs).
#[must_use]
pub fn status(target: &Path, binary: &Path, bundled_version: &str) -> Value {
    let text = std::fs::read_to_string(target.join("SKILL.md")).ok();
    let version = text.as_deref().and_then(version_of);
    let current = text
        .as_deref()
        .is_some_and(|t| version.as_deref() == Some(bundled_version) && t.contains(&binary.display().to_string()));
    json!({
        "installed": text.is_some(),
        "path": target.display().to_string(),
        "version": version,
        "bundled_version": bundled_version,
        "current": current,
        "binary": binary.display().to_string(),
        "mcp_command": mcp_command(binary),
    })
}

impl Service {
    /// `agent_skill_install {target?}` (see the module docs).
    ///
    /// # Errors
    ///
    /// See [`install`]; 500 when there is no home folder and no `target`.
    pub fn agent_skill_install(&self, target: Option<&str>) -> Result<Value, AppError> {
        let source = bundled_dir().ok_or_else(|| AppError::not_found("the app carries no skill folder"))?;
        install(&source, &target_of(target)?, &binary_path()?, BUNDLED_VERSION)
    }

    /// `agent_skill_status {target?}` (see the module docs).
    ///
    /// # Errors
    ///
    /// 500 when there is no home folder and no `target`.
    pub fn agent_skill_status(&self, target: Option<&str>) -> Result<Value, AppError> {
        Ok(status(&target_of(target)?, &binary_path()?, BUNDLED_VERSION))
    }
}

/// R3: the handlers of [`AGENT_SKILL_COMMANDS`] for [`crate::call::run`].
pub(crate) const HANDLERS: [crate::call::Route; 2] = crate::call::routes(AGENT_SKILL_COMMANDS, |s, _, cmd, args| {
    crate::call::json_reply(cmd, dispatch(s, cmd, args))
});

/// Run a command of this module; `None` when `cmd` is not one of them.
#[must_use]
pub fn dispatch(svc: &Service, cmd: &str, args: &Value) -> Option<Result<Value, AppError>> {
    if !AGENT_SKILL_COMMANDS.contains(&cmd) {
        return None;
    }
    Some((|| {
        let target = match args.get("target") {
            None | Some(Value::Null) => None,
            Some(Value::String(t)) => Some(t.as_str()),
            Some(_) => return Err(AppError::bad("wrong argument `target` (string)")),
        };
        if cmd == "agent_skill_install" {
            svc.agent_skill_install(target)
        } else {
            svc.agent_skill_status(target)
        }
    })())
}

#[cfg(test)]
mod tests;
