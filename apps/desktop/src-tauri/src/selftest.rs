//! Self-test mode (P3-7a, debug builds only). The app starts with
//! `--self-test <scenario> --out <report.json>`: the page runs the scenario in the real
//! `WKWebView` (`apps/desktop/selftest/driver.js`), sends one JSON report to the command
//! `selftest_report`, and the process writes the file and exits (0 = pass, 1 = fail, 2 =
//! the scenario could not start). It replaces the debug probe of P3-3b. The report holds
//! numbers and no screenshot, so macOS asks for no screen permission.
//!
//! This module is compiled only with `debug_assertions`: a release build contains neither
//! the driver, nor the scenario loader, nor the two commands.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use serde_json::{Value, json};

/// The page script that runs the scenario.
pub const DRIVER: &str = include_str!("../../selftest/driver.js");

/// The page script of the scenario `shell` (P3-4b): a scenario with `"driver": "shell"` runs this one.
pub const SHELL_DRIVER: &str = include_str!("../../selftest/shell_driver.js");

/// The page script of the tracks scenario (P3-4c): a folder drop to Export all in 4 user actions.
pub const TRACKS_DRIVER: &str = include_str!("../../selftest/tracks_driver.js");

/// The page script of the scenario `keys` (#206): every key of the key command table, both presets,
/// through the real paths (native menu for a `menu_key`, page keydown for a `page_keys` key).
pub const KEYS_DRIVER: &str = include_str!("../../selftest/keys_driver.js");

/// The page script of the scenario `probe` (#206 diagnosis, throwaway): which event variant fires which item.
pub const PROBE_DRIVER: &str = include_str!("../../selftest/probe_driver.js");

/// The driver script a scenario asks for (field `driver`: absent means the slice driver).
pub fn driver_for(scenario: &Value) -> &'static str {
    match scenario.get("driver").and_then(Value::as_str) {
        Some("shell") => SHELL_DRIVER,
        Some("tracks") => TRACKS_DRIVER,
        Some("keys") => KEYS_DRIVER,
        Some("probe") => PROBE_DRIVER,
        _ => DRIVER,
    }
}

/// The step "play" (P3-5b): defines `window.__selftestPlay`, loaded before the driver.
pub const PLAY: &str = include_str!("../../selftest/play.js");

/// The step "`playhead_steps`" (#463): defines `window.__selftestPlayheadSteps`, loaded before the driver.
pub const PLAYHEAD_STEPS: &str = include_str!("../../selftest/playhead_steps.js");

/// The step "memory" (P3-6b): defines `window.__selftestMemory`, loaded before the driver.
pub const MEMORY: &str = include_str!("../../selftest/memory.js");

/// What the command line asks for.
#[derive(Debug, PartialEq, Eq)]
pub struct Request {
    /// A scenario name (`slice`) or the path of a `.json` file.
    pub scenario: String,
    /// Where the report goes.
    pub out: PathBuf,
}

/// True while a self-test runs: `WebKit` fully suspends a page whose window is occluded (rAF and
/// timers alike), which hangs the in-page wall-clock guards, so lib.rs keeps the window front.
pub static FRONT_WINDOW: AtomicBool = AtomicBool::new(false);

/// `--self-test <scenario> --out <file>` in `args` (without the program name). `Ok(None)`
/// when the flag is absent; an error when it is incomplete.
///
/// # Errors
///
/// When `--self-test` is not followed by a scenario (a value that starts with `--` is none), when
/// `--out` is missing, or when `--out` is not followed by a file name.
pub fn parse_args(args: &[String]) -> Result<Option<Request>, String> {
    let Some(i) = args.iter().position(|a| a == "--self-test") else {
        return Ok(None);
    };
    let scenario = args
        .get(i + 1)
        .filter(|s| !s.starts_with("--"))
        .ok_or("--self-test needs a scenario name or file")?;
    let j = args
        .iter()
        .position(|a| a == "--out")
        .ok_or("--self-test needs --out <report.json>")?;
    let out = args
        .get(j + 1)
        .filter(|s| !s.starts_with("--"))
        .ok_or("--out needs a file name")?;
    Ok(Some(Request {
        scenario: scenario.clone(),
        out: PathBuf::from(out),
    }))
}

/// A name becomes `apps/desktop/selftest/<name>.json` of this checkout; a path stays.
#[must_use]
pub fn scenario_path(scenario: &str) -> PathBuf {
    if scenario.contains('/') || scenario.ends_with(".json") {
        PathBuf::from(scenario)
    } else {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../selftest")
            .join(format!("{scenario}.json"))
    }
}

/// Replace every string `"$NAME"` in the scenario with the environment variable `NAME`,
/// or with what `lookup` answers for it (#194: the test seam).
fn resolve_env_with(v: &mut Value, lookup: &dyn Fn(&str) -> Option<String>) -> Result<(), String> {
    match v {
        Value::String(s) => {
            if let Some(name) = s.strip_prefix('$') {
                *s = lookup(name).ok_or_else(|| format!("the scenario needs the environment variable {name}"))?;
            }
        }
        Value::Array(a) => a.iter_mut().try_for_each(|x| resolve_env_with(x, lookup))?,
        Value::Object(o) => o.values_mut().try_for_each(|x| resolve_env_with(x, lookup))?,
        _ => {}
    }
    Ok(())
}

/// Replace every string `"$NAME"` in the scenario with the environment variable `NAME`.
///
/// # Errors
///
/// When a `$NAME` string names an environment variable that is not set (or not valid Unicode).
pub fn resolve_env(v: &mut Value) -> Result<(), String> {
    resolve_env_with(v, &|name| std::env::var(name).ok())
}

/// The scenario file as JSON with `$NAME` strings resolved; it must have a `name`.
///
/// # Errors
///
/// When the scenario file cannot be read, is not valid JSON, has no string `name`, or needs an
/// environment variable that is not set.
pub fn load_scenario(req: &Request) -> Result<Value, String> {
    let path = scenario_path(&req.scenario);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("cannot read scenario {}: {e}", path.display()))?;
    let mut v: Value = serde_json::from_str(&text).map_err(|e| format!("scenario {}: {e}", path.display()))?;
    if v.get("name").and_then(Value::as_str).is_none() {
        return Err(format!("scenario {} has no name", path.display()));
    }
    resolve_env(&mut v)?;
    Ok(v)
}

/// [`load_scenario`] with the `$NAME` variables taken from `vars` instead of the
/// process environment (#194): a test loads a scenario without writing process env that
/// other parallel tests read. Later pairs win on a duplicate name.
///
/// # Errors
///
/// When the scenario file cannot be read, is not valid JSON, has no string `name`, or needs a
/// variable that `vars` does not hold.
pub fn load_scenario_from(req: &Request, vars: &[(&str, &str)]) -> Result<Value, String> {
    let path = scenario_path(&req.scenario);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("cannot read scenario {}: {e}", path.display()))?;
    let mut v: Value = serde_json::from_str(&text).map_err(|e| format!("scenario {}: {e}", path.display()))?;
    if v.get("name").and_then(Value::as_str).is_none() {
        return Err(format!("scenario {} has no name", path.display()));
    }
    resolve_env_with(&mut v, &|name| {
        vars.iter()
            .rev()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| (*v).to_string())
    })?;
    Ok(v)
}

/// The script the page runs after `load`: the scenario as `window.__SELFTEST__`, then the driver.
#[must_use]
pub fn init_script(scenario: &Value) -> String {
    format!(
        "window.addEventListener('load', () => {{ if (/splash\\.html$/.test(location.pathname)) return; window.__SELFTEST__ = {scenario};\n{PLAY}\n{PLAYHEAD_STEPS}\n{MEMORY}\n{driver}\n}});",
        scenario = scenario,
        driver = driver_for(scenario),
        PLAYHEAD_STEPS = PLAYHEAD_STEPS
    )
}

/// One `WebKit` helper process (`ps` row).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Helper {
    pub pid: u32,
    pub rss_kb: u64,
    /// `WebContent`, `GPU`, `Networking` (the last part of `com.apple.WebKit.<name>`).
    pub kind: String,
}

/// The `WebKit` helper processes in the output of `ps -ax -o pid=,rss=,comm=`.
#[must_use]
pub fn parse_helpers(ps: &str) -> Vec<Helper> {
    let mut out = Vec::new();
    for line in ps.lines() {
        let mut it = line.split_whitespace();
        let (Some(pid), Some(rss), Some(comm)) = (it.next(), it.next(), it.next()) else {
            continue;
        };
        let Some(kind) = comm
            .rsplit('/')
            .next()
            .and_then(|c| c.strip_prefix("com.apple.WebKit."))
        else {
            continue;
        };
        if let (Ok(pid), Ok(rss_kb)) = (pid.parse(), rss.parse()) {
            out.push(Helper {
                pid,
                rss_kb,
                kind: kind.to_string(),
            });
        }
    }
    out
}

fn helpers_now() -> Vec<Helper> {
    let out = std::process::Command::new("/bin/ps")
        .args(["-ax", "-o", "pid=,rss=,comm="])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    parse_helpers(&out)
}

static HELPERS_BEFORE: std::sync::OnceLock<Vec<u32>> = std::sync::OnceLock::new();

/// Call once before the web view exists: the `WebKit` helpers of other apps are not ours.
pub fn snapshot_helpers() {
    HELPERS_BEFORE.get_or_init(|| helpers_now().iter().map(|h| h.pid).collect());
}

/// Physical footprint in bytes of each pid (`footprint --json`); a pid that is gone is missing.
#[must_use]
pub fn footprints(pids: &[u32]) -> std::collections::HashMap<u32, u64> {
    footprint_report(pids).0
}

/// The largest memory categories (dirty bytes, compressed pages included) of a `footprint --json` process entry:
/// `[["WebKit Malloc", 123.4], ...]` in MB, at most `n` entries.
#[must_use]
pub fn top_categories(process: &Value, n: usize) -> Vec<Value> {
    // `dirty` already includes the compressed (swapped) pages; the sum of `dirty` is the footprint
    let mut rows: Vec<(String, f64)> = process["categories"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(k, v)| (k.clone(), v["dirty"].as_f64().unwrap_or(0.0)))
        .collect();
    rows.sort_by(|a, b| b.1.total_cmp(&a.1));
    rows.into_iter()
        .take(n)
        .map(|(k, b)| json!([k, (b / 1_048_576.0 * 10.0).round() / 10.0]))
        .collect()
}

/// Footprints and the top categories of each pid, from one `footprint --json` call.
#[must_use]
pub fn footprint_report(
    pids: &[u32],
) -> (
    std::collections::HashMap<u32, u64>,
    std::collections::HashMap<u32, Vec<Value>>,
) {
    let mut map = std::collections::HashMap::new();
    let mut cats = std::collections::HashMap::new();
    if pids.is_empty() {
        return (map, cats);
    }
    let file = std::env::temp_dir().join(format!("ds_footprint_{}.json", std::process::id()));
    let mut cmd = std::process::Command::new("/usr/bin/footprint");
    for p in pids {
        cmd.arg("-p").arg(p.to_string());
    }
    let ok = cmd.arg("--json").arg(&file).output().is_ok();
    if ok
        && let Some(v) = std::fs::read_to_string(&file)
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
    {
        for p in v["processes"].as_array().into_iter().flatten() {
            if let (Some(pid), Some(fp)) = (p["pid"].as_u64(), p["footprint"].as_u64()) {
                map.insert(pid as u32, fp);
                cats.insert(pid as u32, top_categories(p, 6));
            }
        }
    }
    let _ = std::fs::remove_file(&file);
    (map, cats)
}

/// Memory of the `WebKit` helpers that started after `snapshot_helpers` (this app's web view):
/// `{processes: [{pid, kind, footprint_mb, rss_mb, top: [[category, MB]]}], web_content_mb, web_content_rss_mb, gpu_mb,
/// web_content_count}`. `web_content_mb` is the largest footprint among the new `WebContent` processes.
pub fn memory() -> Value {
    let before = HELPERS_BEFORE.get().cloned().unwrap_or_default();
    let ours: Vec<Helper> = helpers_now().into_iter().filter(|h| !before.contains(&h.pid)).collect();
    let (fp, cats) = footprint_report(&ours.iter().map(|h| h.pid).collect::<Vec<_>>());
    let mb = |b: u64| (b as f64 / 1_048_576.0 * 10.0).round() / 10.0;
    let procs: Vec<Value> = ours
        .iter()
        .map(|h| {
            json!({"pid": h.pid, "kind": h.kind, "footprint_mb": mb(fp.get(&h.pid).copied().unwrap_or(0)),
                   "rss_mb": mb(h.rss_kb * 1024), "top": cats.get(&h.pid).cloned().unwrap_or_default()})
        })
        .collect();
    let max_of = |kind: &str, key: &str| {
        procs
            .iter()
            .filter(|p| p["kind"] == kind)
            .filter_map(|p| p[key].as_f64())
            .fold(0.0, f64::max)
    };
    json!({"processes": procs, "web_content_count": procs.iter().filter(|p| p["kind"] == "WebContent").count(),
           "web_content_mb": max_of("WebContent", "footprint_mb"), "web_content_rss_mb": max_of("WebContent", "rss_mb"),
           "gpu_mb": max_of("GPU", "footprint_mb")})
}

/// The facts about this build that go into every report.
#[must_use]
pub fn host_info() -> Value {
    json!({
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "profile": if cfg!(debug_assertions) { "debug" } else { "release" },
        "core_version": drumspike_core::VERSION,
    })
}

/// A report for a run that did not reach the driver.
#[must_use]
pub fn failure_report(scenario: &str, why: &str) -> Value {
    json!({"scenario": scenario, "pass": false, "steps": [], "metrics": {}, "errors": [why]})
}

/// Add the host facts and the scenario, write the report as pretty JSON; returns whether it passed.
///
/// # Errors
///
/// When the report cannot be serialized, or the file `out` cannot be written.
pub fn write_report(out: &Path, mut report: Value, cold_ms: f64) -> Result<bool, String> {
    let pass = report.get("pass").and_then(Value::as_bool).unwrap_or(false);
    if let Some(o) = report.as_object_mut() {
        o.insert("host".into(), host_info());
        o.insert("process_ms".into(), json!((cold_ms * 100.0).round() / 100.0));
    }
    let text = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
    std::fs::write(out, text + "\n").map_err(|e| format!("cannot write {}: {e}", out.display()))?;
    Ok(pass)
}

/// A thread that writes a failure report and ends the process when the scenario takes too
/// long (its `timeout_s`, default 300, plus 30 s for the start).
pub fn spawn_watchdog(scenario: &Value, out: PathBuf, name: String) {
    let secs = scenario.get("timeout_s").and_then(Value::as_f64).unwrap_or(300.0) + 30.0;
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs_f64(secs));
        let _ = write_report(
            &out,
            failure_report(&name, &format!("timeout after {secs} s")),
            secs * 1000.0,
        );
        eprintln!("self-test {name}: timeout after {secs} s");
        std::process::exit(1);
    });
}

#[cfg(test)]
mod tests;
