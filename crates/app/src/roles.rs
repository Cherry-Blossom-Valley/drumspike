//! Role guess and kit presets (P3-4c).
//!
//! The owner's names for the tracks of a drum session follow one scheme: `k in`, `k out`,
//! `k sub` (kick), `s top`, `s bot` (snare), `t1` to `t4` (toms; `t 1` too), `hh` (hi-hat), `oh`, `ohh`,
//! `ride` (`ri`), `crash` (cymbals). A Pro Tools export adds a take suffix (`k in_02.wav`) and a stereo
//! suffix (`oh_03.L.wav`). [`guess_role`] reads such a name and answers a role; the note is
//! [`default_note`] (the table `ROLE_DEFAULT_NOTE` of `static/app.js`, checked by a test).
//!
//! A kit preset is a list of rules `{pattern, role, note}` saved as JSON in the app data
//! folder (`<home>/kit-presets/<name>.json`). A rule matches a track name when the words of
//! its pattern are the first words of the normalised name. A matching rule wins over the
//! scheme.

use std::path::PathBuf;

use serde_json::{json, Value};

use crate::error::AppError;
use crate::Service;

/// The roles of a track.
pub const ROLES: [&str; 6] = ["kick", "snare", "tom", "hat", "cymbal", "other"];

/// The folder of the preset files in the home folder.
pub const PRESET_DIR: &str = "kit-presets";

/// The MIDI note of a role (`ROLE_DEFAULT_NOTE` in `static/app.js`).
pub fn default_note(role: &str) -> i64 {
    match role {
        "kick" => 36,
        "snare" => 38,
        "tom" => 48,
        "hat" => 42,
        "cymbal" => 49,
        _ => 36,
    }
}

/// The lane colour of a role (`ROLE_COLORS` of the server).
pub fn role_color(role: &str) -> &'static str {
    match role {
        "kick" => "#e05555",
        "snare" => "#e0a13c",
        "tom" => "#57b8e0",
        "hat" => "#9be055",
        "cymbal" => "#c9a0e0",
        _ => "#8a93a6",
    }
}

/// True for one of [`ROLES`].
pub fn is_role(role: &str) -> bool {
    ROLES.contains(&role)
}

/// The name of a track as the guess reads it: lower case, without the stereo suffix (`.L`,
/// `.R`) and the take number (`_02`), with one space between the words. `K In_02.L` gives
/// `k in`; `Hi-Hat_03` gives `hi hat`. A `_<digits>` suffix on a bare tom name stays: it is
/// the drum number (#367 `tom_1`, `tom_2`, `tom_3` are three different drums).
pub fn normalize(stem: &str) -> String {
    let mut s = stem.trim().to_lowercase();
    for suffix in [".l", ".r"] {
        if let Some(rest) = s.strip_suffix(suffix) {
            s = rest.to_string();
            break;
        }
    }
    let digits = s.chars().rev().take_while(char::is_ascii_digit).count();
    if digits > 0 {
        let cut = s.len() - digits;
        if let Some(rest) = s[..cut].strip_suffix('_') {
            if !is_bare_tom_name(rest) {
                s = rest.to_string();
            }
        }
    }
    s.split(|c: char| c.is_whitespace() || c == '_' || c == '-' || c == '.')
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// True when `rest`, after removing a `_<digits>` suffix, is a bare tom name: a name whose last
/// word is `tom` and that holds no number of its own (`tom`, `floor tom`). A `_<digits>` suffix on
/// such a name is the drum number, not a Pro Tools take suffix: #367 `tom_1`, `tom_2`, `tom_3`
/// are three different drums, while `t1_01` and `k in_02` keep stripping the take suffix.
fn is_bare_tom_name(rest: &str) -> bool {
    !rest.chars().any(|c| c.is_ascii_digit()) && rest.split(' ').next_back() == Some("tom")
}

fn words(normal: &str) -> Vec<&str> {
    normal.split(' ').filter(|w| !w.is_empty()).collect()
}

/// A `t1` to `t9`, or `tom`, `tom1`, `tom2` ...
fn is_tom_word(w: &str) -> bool {
    let b = w.as_bytes();
    (b.len() == 2 && b[0] == b't' && b[1].is_ascii_digit() && b[1] != b'0')
        || w.strip_prefix("tom")
            .is_some_and(|rest| rest.chars().all(|c| c.is_ascii_digit()))
}

/// The `1` to `9` of the spelling `t 1`.
fn is_tom_number(w: &str) -> bool {
    w.len() == 1 && w.as_bytes()[0].is_ascii_digit() && w != "0"
}

/// X-6 (P3-9b): common abbreviations of other studios (`BD`, `SD`, `Snr`, `FT`, `Toms`, `China` ...).
/// They count only as the first word of a name: a room mic is often named after what it hears
/// (`r  toms soap`, `rev cym`), and such a track must keep no role. `ft`, `ht`, `mt`, `lt` and `tt` may
/// have a number (`FT2`).
fn first_word_role(word: &str, next: Option<&str>) -> Option<&'static str> {
    let base = word.trim_end_matches(|c: char| c.is_ascii_digit());
    let numbered = base.len() < word.len();
    if numbered && word[base.len()..].starts_with('0') {
        return None;
    }
    match base {
        "ft" | "ht" | "mt" | "lt" | "tt" => return Some("tom"),
        _ if numbered => return None,
        _ => {}
    }
    match word {
        "bd" | "kd" | "kik" | "kck" => Some("kick"),
        "bass" if next == Some("drum") => Some("kick"),
        "sd" | "sn" | "snr" | "sna" => Some("snare"),
        "toms" | "rack" | "floor" | "racktom" | "floortom" => Some("tom"),
        "hats" | "hhat" => Some("hat"),
        "china" | "splash" | "cym" | "cyms" | "cymbal" | "cymbals" | "ohs" => Some("cymbal"),
        _ => None,
    }
}

/// The file-name suffixes of Drum Spike's own exports: the synthetic spike (`<track>_spike.wav`),
/// the audio spike (#209, `<track>_audio_spike.wav`) and the layer render (#212,
/// `<track>_layer.wav`).
const EXPORT_SUFFIXES: [&str; 3] = ["_spike", "_audio_spike", "_layer"];

/// True for the stem of a Drum Spike export file: a synthetic spike, an audio spike or a layer
/// render, with or without Pro Tools' numbered-copy suffix (`s top_02_spike_01`, the owner's file
/// of #216). Such a file is the app's own output, not a recorded track: it gets no role, so the
/// scan does not offer it as a drum track. The stem of a real track never ends in one of the
/// suffixes (`s top_02`, `k in_02`, `layer_02` and `spike` stay false).
pub fn is_export_name(stem: &str) -> bool {
    let mut s = stem.trim().to_lowercase();
    loop {
        let digits = s.chars().rev().take_while(char::is_ascii_digit).count();
        let rest = s[..s.len() - digits].strip_suffix('_');
        match (digits > 0, rest) {
            (true, Some(rest)) => s = rest.to_string(),
            _ => break,
        }
    }
    EXPORT_SUFFIXES
        .iter()
        .any(|suf| s.strip_suffix(suf).is_some_and(|rest| !rest.is_empty()))
}

/// The role of a name that follows the owner's scheme, else `None`. The first word group that
/// matches decides (`t1 hh` is a tom). Names like `k samp savior`, `s trigger` or `r far` do
/// not follow the scheme: they stay `None`. The first word may also be a common abbreviation
/// (`BD`, `SD`, `Snr Top`, `FT`, `Toms`, `China`; see `first_word_role`).
pub fn guess_role(stem: &str) -> Option<&'static str> {
    guess_role_matched(stem).map(|(role, _)| role)
}

/// [`guess_role`] with the words of the name that matched (normalised, space-separated; the
/// hint of the Inspector shows them: `k in = kick`). Two-word matches carry both words.
pub fn guess_role_matched(stem: &str) -> Option<(&'static str, String)> {
    if is_export_name(stem) {
        return None;
    }
    let normal = normalize(stem);
    let w = words(&normal);
    for (i, word) in w.iter().enumerate() {
        let next = w.get(i + 1).copied();
        let role = match *word {
            "kick" => Some("kick"),
            "k" if matches!(next, Some("in" | "out" | "sub")) => Some("kick"),
            "snare" => Some("snare"),
            "s" if matches!(next, Some("top" | "bot" | "bottom")) => Some("snare"),
            "hh" | "hat" | "hihat" => Some("hat"),
            "oh" | "ohh" | "overhead" | "overheads" | "ride" | "ri" | "crash" => Some("cymbal"),
            "t" if next.is_some_and(is_tom_number) => Some("tom"),
            w if is_tom_word(w) => Some("tom"),
            w if i == 0 => first_word_role(w, next),
            _ => None,
        };
        if let Some(role) = role {
            // the word takes the next one with it when the match needs it (`k in`, `s top`, `t 1`, `bass drum`)
            let takes_next = match (*word, next) {
                ("k", Some("in" | "out" | "sub")) | ("s", Some("top" | "bot" | "bottom")) | ("bass", Some("drum")) => {
                    true
                }
                ("t", Some(n)) => is_tom_number(n),
                _ => false,
            };
            let matched = if takes_next {
                format!("{word} {}", next.unwrap())
            } else {
                (*word).to_string()
            };
            return Some((role, matched));
        }
    }
    None
}

/// P4-14: a sample replacement or trigger track (the name holds `samp`, `smpl` or `trig`, or a word `smp` or
/// `ssd...`: `t1 sampl`, `t samp`, `t trigger`, `T1 Smpl`, `Tom1 SSD`, `k samp savior`). Such a track is never a tom
/// close mic of the bleed rule. Its role stays what the name table gives (`t1 sampl` is a tom of the owner's scheme: it
/// detects as a tom, it only is no source or target of the rule).
pub fn is_sample_name(stem: &str) -> bool {
    let normal = normalize(stem);
    ["samp", "smpl", "trig"].iter().any(|k| normal.contains(k))
        || tokens(&normal).iter().any(|t| t == "smp" || t.starts_with("ssd"))
}

/// The words of a normalised name split further at letter-digit changes (`t1hh` gives `t`, `1`, `hh`).
fn tokens(normal: &str) -> Vec<String> {
    let mut out = Vec::new();
    for w in words(normal) {
        let mut cur = String::new();
        for c in w.chars() {
            if !cur.is_empty() && cur.chars().last().unwrap().is_ascii_digit() != c.is_ascii_digit() {
                out.push(std::mem::take(&mut cur));
            }
            cur.push(c);
        }
        if !cur.is_empty() {
            out.push(cur);
        }
    }
    out
}

/// Words that name the place of a mic on its drum or the instrument next to it, not the drum (`t1 top`, `Tom Lo In`,
/// `t1 hh`, `t4 ri`): [`drum_of`] drops them.
pub const POSITION_WORDS: [&str; 26] = [
    "top", "bot", "bottom", "btm", "in", "out", "inside", "outside", "up", "down", "under", "over", "close", "rim",
    "reso", "batter", "front", "back", "side", "shell", "mic", "hh", "hat", "ri", "ride", "cym",
];

/// P4-14: the drum a tom track belongs to, from its name: the name's words without the position words
/// ([`POSITION_WORDS`]), split at letter-digit changes, `tom` written `t`, numbers kept. `t1 top`, `t1 bot`, `T1 HH`
/// and `Tom 1` give `t 1`; `Tom Hi Top` and `Tom Hi Bot` give `t hi`; `Floor Tom 1` and `Floor Tom 2` stay two drums.
/// Fix round 4: when the name has a number, the drum is the words up to and including the first number (`Tom 1 Top
/// 421`, `Tom 1 Bot e604` and `Tom 1 Top DI` give `t 1`; `t4Ri` gives `t 4`; `Floor Tom 1` gives `floor t 1`): what
/// follows is a mic model, a channel or a take. `None` when nothing is left (`Top`): the name does not tell the drum.
pub fn drum_of(stem: &str) -> Option<String> {
    let mut key: Vec<String> = tokens(&normalize(stem))
        .into_iter()
        .filter(|t| !POSITION_WORDS.contains(&t.as_str()))
        .map(|t| if t == "tom" { "t".to_string() } else { t })
        .collect();
    if let Some(i) = key.iter().position(|t| t.starts_with(|c: char| c.is_ascii_digit())) {
        key.truncate(i + 1);
    }
    (!key.is_empty()).then(|| key.join(" "))
}

/// One rule of a kit preset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    /// The words a name starts with (normalised).
    pub pattern: String,
    /// One of [`ROLES`].
    pub role: String,
    /// MIDI note 0 to 127.
    pub note: i64,
}

impl Rule {
    /// The JSON object of the rule.
    pub fn to_json(&self) -> Value {
        json!({"pattern": self.pattern, "role": self.role, "note": self.note})
    }

    /// A rule from JSON; 400 for a pattern without words, an unknown role or a note outside 0 to 127.
    pub fn from_json(v: &Value) -> Result<Rule, AppError> {
        let pattern = normalize(v.get("pattern").and_then(Value::as_str).unwrap_or(""));
        if pattern.is_empty() {
            return Err(AppError::bad("a rule needs a pattern"));
        }
        let role = v.get("role").and_then(Value::as_str).unwrap_or("");
        if !is_role(role) {
            return Err(AppError::bad(format!("rule {pattern:?}: unknown role {role:?}")));
        }
        let note = match v.get("note") {
            None | Some(Value::Null) => default_note(role),
            Some(n) => n
                .as_i64()
                .filter(|n| (0..=127).contains(n))
                .ok_or_else(|| AppError::bad(format!("rule {pattern:?}: the note must be 0 to 127")))?,
        };
        Ok(Rule {
            pattern,
            role: role.to_string(),
            note,
        })
    }
}

/// Where a role came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// A rule of the kit preset.
    Preset,
    /// The owner's naming scheme.
    Scheme,
    /// No rule and no scheme match: role `other`.
    None,
    /// #225: the audio suggested the role (the scan of a file whose name gives none).
    Audio,
}

impl Source {
    /// The word the page gets.
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Preset => "preset",
            Source::Scheme => "scheme",
            Source::None => "none",
            Source::Audio => "audio",
        }
    }
}

/// The result of a guess.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Guess {
    /// One of [`ROLES`].
    pub role: String,
    /// MIDI note.
    pub note: i64,
    /// Where the role came from.
    pub source: Source,
    /// The words of the name that matched (the scheme) or the rule pattern (a kit preset);
    /// `None` when no rule matched (role `other`).
    pub matched: Option<String>,
}

/// The role and note of a name: the longest matching rule of `rules`, else the scheme, else `other`.
pub fn guess(stem: &str, rules: &[Rule]) -> Guess {
    let normal = normalize(stem);
    let w = words(&normal);
    let best = rules
        .iter()
        .filter(|r| {
            let p = words(&r.pattern);
            !p.is_empty() && w.len() >= p.len() && w[..p.len()] == p[..]
        })
        .max_by_key(|r| r.pattern.len());
    if let Some(r) = best {
        return Guess {
            role: r.role.clone(),
            note: r.note,
            source: Source::Preset,
            matched: Some(r.pattern.clone()),
        };
    }
    match guess_role_matched(stem) {
        Some((role, matched)) => Guess {
            role: role.to_string(),
            note: default_note(role),
            source: Source::Scheme,
            matched: Some(matched),
        },
        None => Guess {
            role: "other".into(),
            note: default_note("other"),
            source: Source::None,
            matched: None,
        },
    }
}

/// The file name of a preset: letters, digits, space, `_` and `-` stay, the rest becomes `_`.
fn preset_file_stem(name: &str) -> Result<String, AppError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AppError::bad("a kit preset needs a name"));
    }
    let stem: String = name
        .chars()
        .take(80)
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, ' ' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let stem = stem.trim_matches(|c: char| c == '.' || c == ' ').to_string();
    if stem.is_empty() {
        return Err(AppError::bad("a kit preset needs a name"));
    }
    Ok(stem)
}

impl Service {
    /// The folder of the preset files, `<home>/kit-presets`; 409 while the service has no home folder.
    fn preset_dir(&self) -> Result<PathBuf, AppError> {
        self.home()
            .map(|h| h.join(PRESET_DIR))
            .ok_or_else(|| AppError::conflict("no app data folder: kit presets cannot be stored"))
    }

    /// `kit_presets_list`: `[{name, rules}]` sorted by name. A file that is no preset is skipped.
    pub fn kit_presets_list(&self) -> Result<Value, AppError> {
        let dir = self.preset_dir()?;
        let mut out: Vec<(String, usize)> = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.filter_map(Result::ok) {
                let p = e.path();
                if p.extension().is_none_or(|x| x != "json") {
                    continue;
                }
                if let Some(Ok((name, rules))) = std::fs::read_to_string(&p).ok().map(|t| parse_preset(&t)) {
                    out.push((name, rules.len()));
                }
            }
        }
        out.sort_by_key(|(n, _)| n.to_lowercase());
        Ok(Value::Array(
            out.into_iter().map(|(n, r)| json!({"name": n, "rules": r})).collect(),
        ))
    }

    /// `kit_preset_save {name, rules}`: write `<home>/kit-presets/<name>.json` (a preset of the
    /// same name is replaced). Answers `{name, rules, path}`.
    pub fn kit_preset_save(&self, name: &str, rules: &Value) -> Result<Value, AppError> {
        let stem = preset_file_stem(name)?;
        let list = rules
            .as_array()
            .ok_or_else(|| AppError::bad("`rules` must be a list"))?
            .iter()
            .map(Rule::from_json)
            .collect::<Result<Vec<_>, _>>()?;
        let dir = self.preset_dir()?;
        std::fs::create_dir_all(&dir).map_err(|e| AppError::internal(format!("{}: {e}", dir.display())))?;
        let path = dir.join(format!("{stem}.json"));
        let doc = json!({"name": name.trim(), "rules": list.iter().map(Rule::to_json).collect::<Vec<_>>()});
        crate::lifecycle::write_json_atomic(&path, &doc)?;
        Ok(json!({"name": name.trim(), "rules": list.len(), "path": path.display().to_string()}))
    }

    /// `kit_preset_load {name}`: `{name, rules: [{pattern, role, note}]}`; 404 for an unknown name.
    pub fn kit_preset_load(&self, name: &str) -> Result<Value, AppError> {
        let (name, rules) = self.load_rules(name)?;
        Ok(json!({"name": name, "rules": rules.iter().map(Rule::to_json).collect::<Vec<_>>()}))
    }

    /// The rules of the preset `name`.
    pub(crate) fn load_rules(&self, name: &str) -> Result<(String, Vec<Rule>), AppError> {
        let path = self.preset_dir()?.join(format!("{}.json", preset_file_stem(name)?));
        let text = std::fs::read_to_string(&path)
            .map_err(|_| AppError::not_found(format!("no kit preset named {:?}", name.trim())))?;
        parse_preset(&text)
    }
}

fn parse_preset(text: &str) -> Result<(String, Vec<Rule>), AppError> {
    let v: Value = serde_json::from_str(text).map_err(|_| AppError::bad("the kit preset file is not valid JSON"))?;
    let name = v
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::bad("the kit preset file has no name"))?;
    let rules = v
        .get("rules")
        .and_then(Value::as_array)
        .ok_or_else(|| AppError::bad("the kit preset file has no rules"))?
        .iter()
        .map(Rule::from_json)
        .collect::<Result<Vec<_>, _>>()?;
    Ok((name.to_string(), rules))
}

#[cfg(test)]
mod tests;
