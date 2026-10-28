//! #378 evaluation helper: the audio-spike snippet lengths of one wav at given hit times.
//!
//! Not product code: the Python eval tool `tools/eval/spike_len.py` calls this per mic and
//! item. It reads the wav (mono, any supported rate) and prints one JSON line:
//! `{"lengths_ms": [...]}`, one entry per distinct hit ([`audio_snippet_lengths`]).
//!
//! Usage:
//! ```text
//! cargo run -p drumspike-core --example spike_len -- --wav X.wav --times 0.1,0.2,0.3
//! ```

use std::path::PathBuf;

use drumspike_core::{audio_snippet_lengths, read_wav_mono, MixMode};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut wav: Option<PathBuf> = None;
    let mut times_in = String::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--wav" => wav = Some(PathBuf::from(args.next().expect("--wav needs a path"))),
            "--times" => times_in = args.next().expect("--times needs a list").replace(',', " "),
            other => return Err(format!("unknown argument {other}").into()),
        }
    }
    let wav = wav.ok_or("--wav is required")?;
    let times: Vec<f64> = times_in
        .split_whitespace()
        .map(|t| t.parse())
        .collect::<Result<_, _>>()?;
    let (signal, info) = read_wav_mono(&wav, MixMode::Average)?;
    let lens = audio_snippet_lengths(&signal, info.sample_rate, &times);
    let ms: Vec<f64> = lens.iter().map(|l| 1000.0 * l).collect();
    println!("{{\"lengths_ms\": {ms:?}}}");
    Ok(())
}
