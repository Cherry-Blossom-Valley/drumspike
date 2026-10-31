//! #471 measurement helper: the transport clock of the device, from outside.
//!
//! Not product code. It writes a sine wav (if the path does not exist), opens the transport on
//! the default output device, plays it for `--secs`, and prints one line per published playhead
//! pair while the transport plays: `frame host_time_ns`. Each callback publishes one pair, the
//! probe reads the clock every millisecond, so no pair is missed (a block is 5 ms at 256 frames).
//! The fit and the residuals are in `tools/clock_residuals.py` (stdlib only).
//!
//! Usage:
//! ```text
//! cargo run -p drumspike-core --release --example clock_probe -- --secs 20 --wav /path/tone.wav --out pairs.txt
//! ```
//!
//! `--out` writes the pairs to a file (default: stdout).

use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use drumspike_core::MixMode;
use drumspike_core::transport::device::Output;
use drumspike_core::transport::{TrackSpec, Transport, TransportConfig};

/// A 220 Hz sine, 16-bit mono at 48 kHz, `secs` long.
fn write_tone(path: &Path, secs: u32) -> std::io::Result<()> {
    let rate: u32 = 48_000;
    let n = rate * secs;
    let mut data = Vec::with_capacity(n as usize * 2);
    for i in 0..n {
        let s = (2.0 * std::f64::consts::PI * 220.0 * f64::from(i) / f64::from(rate)).sin() * 0.2;
        data.extend_from_slice(&((s * f64::from(i16::MAX)) as i16).to_le_bytes());
    }
    let mut out = Vec::with_capacity(44 + data.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&data);
    std::fs::write(path, out)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut secs: u64 = 20;
    let mut wav: Option<PathBuf> = None;
    let mut out_path: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--secs" => secs = args.next().ok_or("--secs needs a value")?.parse()?,
            "--wav" => wav = Some(PathBuf::from(args.next().ok_or("--wav needs a path")?)),
            "--out" => out_path = Some(PathBuf::from(args.next().ok_or("--out needs a path")?)),
            other => return Err(format!("unknown argument {other}").into()),
        }
    }
    let wav = wav.ok_or("--wav is required")?;
    if !wav.exists() {
        write_tone(&wav, secs as u32 + 5)?;
    }
    let tracks = [TrackSpec {
        path: wav,
        mode: MixMode::Average,
        hits: Vec::new(),
        click_pitch: 1000.0,
    }];
    let cfg = TransportConfig {
        clicks_on: false,
        ..TransportConfig::default()
    };
    let tr = Transport::open(&tracks, Output::Default, cfg)?;
    tr.play();
    let sink: Box<dyn Write> = match out_path {
        Some(p) => Box::new(std::fs::File::create(p)?),
        None => Box::new(std::io::stdout().lock()),
    };
    let mut out = BufWriter::new(sink);
    let t0 = Instant::now();
    let mut last = None;
    while t0.elapsed() < Duration::from_secs(secs) {
        let ph = tr.playhead();
        if ph.playing && last != Some(ph.frame) {
            last = Some(ph.frame);
            writeln!(out, "{} {}", ph.frame, ph.host_time_ns)?;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    out.flush()?;
    let s = tr.stats();
    eprintln!("clock_probe: {} callbacks, {} underruns", s.callbacks, s.underruns);
    Ok(())
}
