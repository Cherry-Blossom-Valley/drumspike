//! `ds-bridge` (P3-7a): the test bridge. Serves `static/` and `POST /invoke/<command>` on
//! 127.0.0.1 so the app tests drive the page in Chromium on both hosts. Debug builds only:
//! a release build refuses to start, and the Tauri bundle does not contain it.
//!
//! Usage: `ds-bridge [--port N] [--static DIR]`. The first output line is
//! `ds-bridge listening on http://127.0.0.1:<port>/`. The model-free main runs the DSP detector.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use drumspike_app::bridge::{self, Bridge};
use drumspike_app::{Service, ServiceConfig};

struct Args {
    port: u16,
    static_dir: PathBuf,
}

fn parse_args(argv: &[String]) -> Result<Args, String> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut a = Args {
        port: 0,
        static_dir: root.join("static"),
    };
    let mut it = argv.iter();
    while let Some(k) = it.next() {
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
    bridge::serve(listener, bridge);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
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
    }
}
