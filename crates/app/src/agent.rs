//! P4-5: the entry of the two window-less agent modes, `cli` and `mcp`, for every binary that carries them (the
//! app's own binary, `service_tool` for the tests). The binary calls [`entry`] with its arguments before it starts
//! anything else; `Some(code)` is the exit code, `None` means the arguments are not for this module.
//!
//! The service of these modes locks every project it opens for writing (`lock::ProjectLock`, client `cli` or `mcp`),
//! keeps its settings and presets in the app's data folder (`DRUMSPIKE_HOME`, else
//! `~/Library/Application Support/com.cbv.drumspike` on a Mac) and puts new projects where the app does.

use std::path::{Path, PathBuf};

use crate::{Service, ServiceConfig, cli, mcp};

/// The name the user gives the symlink to the app binary: called by this name, the binary is the command-line tool.
pub const TOOL_NAME: &str = "drumspike";

/// The app's data folder, as Tauri's `app_data_dir` names it (the identifier of `tauri.conf.json`).
pub fn default_home() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").filter(|h| !h.is_empty())?;
    cfg!(target_os = "macos").then(|| PathBuf::from(home).join("Library/Application Support/com.cbv.drumspike"))
}

/// A service for an agent mode.
pub fn service(models_dir: &Path, client: &str) -> Service {
    let svc = Service::new(ServiceConfig::from_env(models_dir));
    svc.set_lock_client(client);
    // a test hook, debug builds only (never in the release app): every copy job pauses this many ms after each chunk, so
    // the exit tests have a job that is still running
    #[cfg(debug_assertions)]
    if let Some(ms) = std::env::var("DRUMSPIKE_TEST_COPY_DELAY_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
    {
        svc.set_copy_delay(std::time::Duration::from_millis(ms));
    }
    if svc.home().is_none()
        && let Some(h) = default_home()
    {
        svc.set_home(h);
    }
    svc
}

/// Run `cli` or `mcp` when `args` (the process arguments after the program name) ask for it, or when `program` is
/// named [`TOOL_NAME`] (then every argument belongs to the command line, and `mcp` as the first one starts the
/// server). The exit code, or `None` for any other arguments.
pub fn entry(program: &str, args: &[String], models_dir: &Path) -> Option<i32> {
    let by_name = Path::new(program).file_name().is_some_and(|n| n == TOOL_NAME);
    let (mode, rest): (&str, &[String]) = match args.first().map(String::as_str) {
        Some("mcp") => ("mcp", &args[1..]),
        Some("cli") if !by_name => ("cli", &args[1..]),
        _ if by_name => ("cli", args),
        _ => return None,
    };
    if mode == "mcp" {
        let svc = service(models_dir, "mcp");
        mcp::serve(
            &svc,
            std::io::BufReader::new(std::io::stdin()),
            Box::new(std::io::stdout()),
        );
        return Some(0);
    }
    let svc = service(models_dir, "cli");
    Some(cli::run(&svc, rest, &mut cli::Io::stdio()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn other_arguments_are_not_for_the_agent_modes() {
        let m = Path::new("/nonexistent");
        let a = |s: &[&str]| s.iter().map(std::string::ToString::to_string).collect::<Vec<_>>();
        assert_eq!(entry("/x/drumspike-desktop", &a(&["--app-info"]), m), None);
        assert_eq!(entry("/x/drumspike-desktop", &[], m), None);
        assert_eq!(entry("/x/drumspike-desktop", &a(&["help"]), m), None);
        // the tool name: every argument is the command line
        assert_eq!(entry("/usr/local/bin/drumspike", &a(&["help", "nope"]), m), Some(3));
        assert_eq!(entry("/x/app", &a(&["cli", "help", "nope"]), m), Some(3));
    }

    /// The copy-delay hook must not exist in a release build (the release app's `cli` and `mcp` modes read the environment).
    #[test]
    fn the_copy_delay_hook_is_compiled_in_debug_builds_only() {
        let src = include_str!("agent.rs");
        let body = &src[..src.find("#[cfg(test)]").unwrap()];
        let at = body
            .find("std::env::var(\"DRUMSPIKE_TEST_COPY_DELAY_MS\")")
            .expect("the hook");
        let cfg = body[..at]
            .rfind("#[cfg(debug_assertions)]")
            .expect("a cfg attribute before the hook");
        let between = body[cfg + "#[cfg(debug_assertions)]".len()..at].trim();
        assert!(
            between.starts_with("if let Some(ms)"),
            "the attribute must guard the hook, not something earlier: {between}"
        );
    }

    #[test]
    fn the_home_is_the_app_data_folder_on_a_mac() {
        if cfg!(target_os = "macos") {
            assert!(
                default_home()
                    .unwrap()
                    .ends_with("Library/Application Support/com.cbv.drumspike")
            );
        }
    }
}
