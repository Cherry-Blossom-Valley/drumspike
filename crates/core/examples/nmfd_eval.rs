//! A7 (#353) evaluation helper: one wav through the kit-template NMFD detection function.
//!
//! Not product code: the Python eval tool `tools/eval/nmfd.py` calls this per mic and item. It
//! reads the wav (mono, any supported rate), computes the pipeline's spectrogram
//! ([`drumspike_core::detect::prepare`]), builds one template per `--template` argument from the
//! given hit times ([`template_from_hits`]), runs the NMFD once with all templates and prints one
//! JSON line: `{"frames": N, "hits": [[s, ...], ...]}` (the picked hits per template, in order).
//!
//! A7c (#409): kit templates pooled per kit (per ENST drummer, like a full song). A template may
//! be built from several wavs: `--template-wav` sets the source wav of the following
//! `--template-times` hit times, `--template-new` starts the next template. `--max-hits` caps the
//! hits per template (default 10). `--save-template <path>` writes the built templates as JSON
//! (`{"templates": [[bands ...]]}`); `--templates-file <path>` loads templates instead of
//! building them. With `--save-template` or `--templates-file` and no `--wav`, only the build or
//! the load happens.
//!
//! Usage:
//! ```text
//! cargo run -p drumspike-core --example nmfd_eval -- --wav X.wav --iters 64 \
//!     --template 0.1,0.2,0.3 --template 1.5,1.6
//! cargo run -p drumspike-core --example nmfd_eval -- --template-wav kit1.wav \
//!     --template-times 0.1,0.2 --template-wav kit2.wav --template-times 0.3,0.4 \
//!     --template-new ... --save-template kit.json --max-hits 40
//! ```

use std::path::{Path, PathBuf};

use drumspike_core::detect::nmfd::{self, NmfdParams, Template};
use drumspike_core::detect::Chain;

fn parse_times(s: &str) -> Result<Vec<f64>, String> {
    s.split(',')
        .filter(|t| !t.trim().is_empty())
        .map(|t| t.trim().parse::<f64>().map_err(|e| format!("bad time {t:?}: {e}")))
        .collect()
}

fn spec_of(wav: &Path) -> Result<Vec<f32>, Box<dyn std::error::Error>> {
    let (signal, info) = drumspike_core::read_wav_mono(wav, drumspike_core::MixMode::Average)?;
    Ok(drumspike_core::detect::prepare(&signal, info.sample_rate, Chain::Mix)?)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut wav: Option<PathBuf> = None;
    // one template per group: the (source wav, hit times) pairs of its hits
    let mut groups: Vec<Vec<(Option<PathBuf>, Vec<f64>)>> = vec![Vec::new()];
    let mut src_wav: Option<PathBuf> = None;
    let mut templates_file: Option<PathBuf> = None;
    let mut save_template: Option<PathBuf> = None;
    let mut iters: usize = 64;
    let mut max_hits: usize = 10;
    let mut params = NmfdParams::default();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--wav" => wav = Some(PathBuf::from(args.next().expect("--wav needs a path"))),
            "--template" => {
                let ts = parse_times(&args.next().expect("--template needs times"))?;
                groups.last_mut().expect("a template group").push((None, ts));
            }
            "--template-wav" => src_wav = Some(PathBuf::from(args.next().expect("--template-wav needs a path"))),
            "--template-times" => {
                let w = src_wav.clone().ok_or("--template-times needs a --template-wav first")?;
                let ts = parse_times(&args.next().expect("--template-times needs times"))?;
                groups.last_mut().expect("a template group").push((Some(w), ts));
            }
            "--template-new" => groups.push(Vec::new()),
            "--save-template" => {
                save_template = Some(PathBuf::from(args.next().expect("--save-template needs a path")))
            }
            "--templates-file" => {
                templates_file = Some(PathBuf::from(args.next().expect("--templates-file needs a path")))
            }
            "--iters" => iters = args.next().expect("--iters needs a number").parse()?,
            "--max-hits" => max_hits = args.next().expect("--max-hits needs a number").parse()?,
            "--thresh" => params.thresh_rel = args.next().expect("--thresh needs a number").parse()?,
            "--thresh-ref" => {
                let v = args.next().expect("--thresh-ref needs max or median").to_lowercase();
                params.thresh_ref = match v.as_str() {
                    "max" => nmfd::ThreshRef::Max,
                    "median" => nmfd::ThreshRef::Median,
                    other => return Err(format!("unknown --thresh-ref {other}").into()),
                };
            }
            "--gap" => params.min_gap_s = args.next().expect("--gap needs a number").parse()?,
            other => return Err(format!("unknown argument {other}").into()),
        }
    }
    if groups.last().is_some_and(|g| g.is_empty()) {
        groups.pop(); // a trailing --template-new
    }

    let templates: Vec<Template> = if let Some(f) = &templates_file {
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(f)?)?;
        v["templates"]
            .as_array()
            .ok_or("--templates-file needs {\"templates\": [[...]]}")?
            .iter()
            .map(|t| -> Result<Template, serde_json::Error> {
                let bands: Vec<f32> = serde_json::from_value(t.clone())?;
                Ok(Template { bands })
            })
            .collect::<Result<_, _>>()?
    } else {
        let mut out = Vec::new();
        for group in &groups {
            let mut specs = Vec::new();
            let mut hit_sets = Vec::new();
            for (w, ts) in group {
                let w = w
                    .as_ref()
                    .map(|p| p.as_path())
                    .or(wav.as_deref())
                    .ok_or("a --template without --template-wav needs the analyzed --wav")?;
                specs.push(spec_of(w)?);
                hit_sets.push(ts.as_slice());
            }
            let set_refs: Vec<&[f32]> = specs.iter().map(|s| s.as_slice()).collect();
            out.push(
                nmfd::template_from_hits_multi(&set_refs, &hit_sets, 1, max_hits)
                    .ok_or("a drum has too few template hits")?,
            );
        }
        out
    };
    if let Some(f) = &save_template {
        let bands: Vec<Vec<f32>> = templates.iter().map(|t| t.bands.clone()).collect();
        std::fs::write(f, serde_json::json!({ "templates": bands }).to_string())?;
    }
    if wav.is_none() && (templates_file.is_some() || save_template.is_some()) {
        println!("{}", serde_json::json!({ "templates": templates.len() }));
        return Ok(());
    }
    let wav = wav.ok_or("--wav is required")?;
    if templates.is_empty() {
        return Err("at least one template is required".into());
    }
    let (signal, info) = drumspike_core::read_wav_mono(&wav, drumspike_core::MixMode::Average)?;
    let spec = drumspike_core::detect::prepare(&signal, info.sample_rate, Chain::Mix)?;
    let hits: Vec<Vec<f64>> = (0..templates.len())
        .map(|j| nmfd::nmfd_hits(&spec, &templates, j, iters, &params))
        .collect();
    println!(
        "{}",
        serde_json::json!({ "frames": spec.len() / drumspike_core::detect::features::N_BANDS, "hits": hits })
    );
    Ok(())
}
