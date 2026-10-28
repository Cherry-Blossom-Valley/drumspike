//! Drum Spike desktop app: entry point. All code lives in the library (`lib.rs`).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // P3-11: `--app-info`, `--analyze`, `cli` and `mcp` (P4-5) run without a window (lib.rs, `headless`)
    let program = std::env::args().next().unwrap_or_default();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = drumspike_desktop::headless_as(&program, &args) {
        std::process::exit(code);
    }
    drumspike_desktop::run();
}
