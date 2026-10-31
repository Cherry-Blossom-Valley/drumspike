//! `ds-bridge` (P3-7a): the test bridge. Serves `static/` and `POST /invoke/<command>` on
//! 127.0.0.1 so the app tests drive the page in Chromium on both hosts. Debug builds only:
//! a release build refuses to start, and the Tauri bundle does not contain it.
//!
//! Usage: `ds-bridge [--port N] [--static DIR] [--exit-on-stdin-eof]`. The first output line is
//! `ds-bridge listening on http://127.0.0.1:<port>/`. The model-free main runs the DSP detector.

use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::Arc;

use drumspike_app::bridge::{self, Bridge};
use drumspike_app::{Service, ServiceConfig};

struct Args {
    port: u16,
    static_dir: PathBuf,
    /// #4: opt-in. With it, the bridge exits when its stdin reaches EOF (the e2e fixtures keep
    /// the pipe open; a killed pytest closes it, so the bridge ends with the test process).
    exit_on_stdin_eof: bool,
}

fn parse_args(argv: &[String]) -> Result<Args, String> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut a = Args {
        port: 0,
        static_dir: root.join("static"),
        exit_on_stdin_eof: false,
    };
    let mut it = argv.iter();
    while let Some(k) = it.next() {
        // #4: a boolean flag takes no value; every other option needs one.
        if k.as_str() == "--exit-on-stdin-eof" {
            a.exit_on_stdin_eof = true;
            continue;
        }
        let v = it.next().ok_or_else(|| format!("{k} needs a value"))?;
        match k.as_str() {
            "--port" => a.port = v.parse().map_err(|_| format!("bad port {v}"))?,
            "--static" => a.static_dir = PathBuf::from(v),
            other => return Err(format!("unknown option {other}")),
        }
    }
    Ok(a)
}

fn main() {
    if !cfg!(debug_assertions) {
        eprintln!("ds-bridge is for debug builds only");
        std::process::exit(2);
    }
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = match parse_args(&argv) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("ds-bridge: {e}");
            std::process::exit(2);
        }
    };
    let listener = match bridge::listen(args.port) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("ds-bridge: cannot listen: {e}");
            std::process::exit(1);
        }
    };
    let port = listener.local_addr().map_or(args.port, |a| a.port());
    let svc = Service::new(ServiceConfig::from_env(&PathBuf::new()));
    let bridge = Arc::new(Bridge::new(svc, args.static_dir));
    println!("ds-bridge listening on http://127.0.0.1:{port}/");
    let _ = std::io::stdout().flush();
    if args.exit_on_stdin_eof {
        // #4: read stdin in the background; on EOF (the parent died or closed the pipe) end the
        // process, so a SIGKILLed pytest run leaves no bridge behind. Without the flag: no change.
        std::thread::spawn(|| {
            let stdin = std::io::stdin();
            let mut lock = stdin.lock();
            let mut buf = Vec::new();
            loop {
                match lock.read_until(b'\n', &mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => buf.clear(),
                }
            }
            std::process::exit(0);
        });
    }
    bridge::serve(listener, bridge);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(std::string::ToString::to_string).collect()
    }

    #[test]
    fn parse_args_reads_the_options_and_refuses_the_rest() {
        let a = parse_args(&s(&["--port", "9", "--static", "/s"])).unwrap();
        assert_eq!((a.port, a.static_dir), (9, PathBuf::from("/s")));
        let d = parse_args(&[]).unwrap();
        assert_eq!(d.port, 0);
        assert!(d.static_dir.ends_with("static"));
        assert!(parse_args(&s(&["--port"])).is_err());
        assert!(parse_args(&s(&["--port", "x"])).is_err());
        assert!(parse_args(&s(&["--what", "1"])).is_err());
        assert!(!parse_args(&s(&[])).unwrap().exit_on_stdin_eof);
        assert!(parse_args(&s(&["--exit-on-stdin-eof"])).unwrap().exit_on_stdin_eof);
    }
}
