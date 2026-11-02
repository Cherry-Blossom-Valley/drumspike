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

use serde_json::{Value, json};

use crate::Service;
use crate::error::AppError;

/// The roles of a track.
pub const ROLES: [&str; 6] = ["kick", "snare", "tom", "hat", "cymbal", "other"];

/// The folder of the preset files in the home folder.
pub const PRESET_DIR: &str = "kit-presets";

/// The MIDI note of a role (`ROLE_DEFAULT_NOTE` in `static/app.js`).
#[must_use]
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
#[must_use]
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
#[must_use]
pub fn is_role(role: &str) -> bool {
    ROLES.contains(&role)
}

/// The name of a track as the guess reads it: lower case, without the stereo suffix (`.L`,
/// `.R`) and the take number (`_02`), with one space between the words. `K In_02.L` gives
/// `k in`; `Hi-Hat_03` gives `hi hat`. A `_<digits>` suffix on a bare tom name stays: it is
/// the drum number (#367 `tom_1`, `tom_2`, `tom_3` are three different drums).
pub fn normalize(stem: &str) -> String {
    let mut s = split_glued(stem.trim()).to_lowercase();
    for suffix in [".l", ".r"] {
        if let Some(rest) = s.strip_suffix(suffix) {
            s = rest.to_string();
            break;
        }
    }
    let digits = s.chars().rev().take_while(char::is_ascii_digit).count();
    if digits > 0 {
        let cut = s.len() - digits;
        if let Some(rest) = s[..cut].strip_suffix('_')
            && !is_bare_tom_name(rest)
        {
            s = rest.to_string();
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

/// #285: a space between the glued parts of a name, so the words come apart: at a lower-to-upper
/// case change (`KickIn` gives `Kick In`), before the last upper-case letter of a run that a
/// lower-case letter follows (`SNTop` gives `SN Top`). Letter-digit changes stay glued here (`t1`,
/// `Tom1HH`): [`guess_name`] splits them per word. The caller lowercases and splits at the rest.
fn split_glued(stem: &str) -> String {
    let chars: Vec<char> = stem.chars().collect();
    let mut out = String::with_capacity(stem.len() + 8);
    for (i, &c) in chars.iter().enumerate() {
        if i > 0 {
            let p = chars[i - 1];
            let case_change = p.is_lowercase() && c.is_uppercase();
            // a run of three or more capitals before a small letter: `SNTop`, `HHOpen`; a run of two
            // (`OHs`, `BDs`) is one word with a plural s
            let run_end = i > 1
                && chars[i - 2].is_uppercase()
                && p.is_uppercase()
                && c.is_uppercase()
                && chars.get(i + 1).is_some_and(|n| n.is_lowercase());
            if case_change || run_end {
                out.push(' ');
            }
        }
        out.push(c);
    }
    out
}

/// The `1` to `9` of the spelling `t 1` (after [`split_glued`] also `t1`).
fn is_tom_number(w: &str) -> bool {
    w.len() == 1 && w.as_bytes()[0].is_ascii_digit() && w != "0"
}

/// #285: the common terms of each role, as data. A one-word term matches a word of the name, a
/// two-word term two adjacent words. The owner's scheme (`k in`, `s top`, `hh`, `oh`, `ri`) is in
/// the table too; the tom numbers (`t 1`, `t1`) are a rule of [`guess_role_matched`]. Order: the
/// first role whose term matches a word wins, scanning the words of the name from the left.
pub const ROLE_TERMS: [(&str, &[&str]); 5] = [
    (
        "kick",
        &[
            "kick",
            "kik",
            "kck",
            "kd",
            "bd",
            "bass drum",
            "bassdrum",
            "bdrum",
            "kick drum",
            "kickdrum",
            "k in",
            "k out",
            "k sub",
            "kin",
            "kout",
            "ksub",
            "kick in",
            "kick out",
            "kick sub",
            "sub kick",
            "subkick",
            "foot",
        ],
    ),
    (
        "snare",
        &[
            "snare",
            "sn",
            "snr",
            "sd",
            "sna",
            "s top",
            "s bot",
            "s bottom",
            "snare drum",
            "snaredrum",
            "snare top",
            "snare bottom",
            "snare bot",
            "sntop",
            "snbot",
            "snrtop",
            "snrbot",
            "sdt",
            "sdb",
            "side stick",
            "sidestick",
            "rimshot",
        ],
    ),
    (
        "tom",
        &[
            "tom",
            "toms",
            "tomtom",
            "tom tom",
            "ft",
            "ht",
            "mt",
            "lt",
            "tt",
            "rack",
            "floor",
            "rack tom",
            "racktom",
            "floor tom",
            "floortom",
            "hi tom",
            "high tom",
            "mid tom",
            "lo tom",
            "low tom",
            "hitom",
            "hightom",
            "midtom",
            "lotom",
            "lowtom",
        ],
    ),
    (
        "hat",
        &[
            "hh",
            "hat",
            "hats",
            "hihat",
            "hi hat",
            "hihats",
            "hi hats",
            "hhat",
            "hat open",
            "hat closed",
            "hats open",
            "hats closed",
            "open hat",
            "closed hat",
        ],
    ),
    (
        "cymbal",
        &[
            "oh",
            "ohh",
            "ohs",
            "overhead",
            "overheads",
            "over head",
            "over heads",
            "ride",
            "ri",
            "crash",
            "crashes",
            "china",
            "splash",
            "cym",
            "cyms",
            "cymbal",
            "cymbals",
            "stack",
            "ride bell",
        ],
    ),
];

/// #285: a track named after what it hears gets no role from its name: room and ambience mics,
/// busses, mixes and prints. Anywhere in the name (`Rev Cym`, `Drum Bus`, `Snare Room`).
pub const EXCLUDED_WORDS: [&str; 16] = [
    "room",
    "rm",
    "amb",
    "ambient",
    "rev",
    "verb",
    "reverb",
    "bus",
    "buss",
    "mix",
    "print",
    "accompaniment",
    "sum",
    "master",
    "talkback",
    "click",
];

/// The owner's room mics: `r far`, `ro ks soap`, `r toms soap`. Only as the first word.
const EXCLUDED_FIRST_WORDS: [&str; 2] = ["r", "ro"];

/// The second half of a glued word (`kickin`, `sntop`, `hatopen`): a mic position, a side or a
/// number. [`POSITION_WORDS`] plus these.
const GLUED_SECOND_WORDS: [&str; 18] = [
    "drum", "l", "r", "c", "lo", "hi", "mid", "low", "high", "sub", "open", "closed", "di", "dyn", "cond", "mono",
    "st", "trig",
];

/// Short terms (one or two letters) that count anywhere in the name: the owner's scheme.
const SHORT_ANYWHERE: [&str; 4] = ["hh", "oh", "ri", "ft"];

/// The role of a one-word term, or `None`. Terms of one or two letters count only as the first
/// word of a name (`BD`, `SD`, `SN Top`, `FT2`), except the scheme's `hh`, `oh`, `ri`: a room mic is
/// often named after what it hears, and the first word tells what the track is.
fn term_role(word: &str, first: bool) -> Option<&'static str> {
    if word.len() <= 2 && !first && !SHORT_ANYWHERE.contains(&word) {
        return None;
    }
    ROLE_TERMS
        .iter()
        .find(|(_, terms)| terms.iter().any(|t| !t.contains(' ') && *t == word))
        .map(|(role, _)| *role)
}

/// The role of a two-word term (`k in`, `bass drum`, `snare top`), or `None`.
fn pair_role(word: &str, next: &str) -> Option<&'static str> {
    ROLE_TERMS
        .iter()
        .find(|(_, terms)| {
            terms
                .iter()
                .any(|t| t.split_once(' ').is_some_and(|(a, b)| a == word && b == next))
        })
        .map(|(role, _)| *role)
}

/// The role of a glued word `<term><second>` (`kickin`, `sntop`, `hatopen`, `oh2`): the term is a
/// one-word term of two letters or more, the rest a position, a side or digits. `tomato` is none.
fn glued_role(word: &str, first: bool) -> Option<(&'static str, String)> {
    for (role, terms) in &ROLE_TERMS {
        for t in terms.iter().filter(|t| !t.contains(' ') && t.len() >= 2) {
            if let Some(rest) = word.strip_prefix(t)
                && !rest.is_empty()
                && (t.len() > 2 || first || SHORT_ANYWHERE.contains(t))
                && (POSITION_WORDS.contains(&rest)
                    || GLUED_SECOND_WORDS.contains(&rest)
                    || rest.chars().all(|c| c.is_ascii_digit()))
            {
                return Some((role, format!("{t} {rest}")));
            }
        }
    }
    None
}

/// The edit distance of two short words: insert, delete, replace, or swap two neighbours, one
/// each (Damerau-Levenshtein, optimal string alignment), so `snaer` is one edit from `snare`.
#[allow(clippy::needless_range_loop)]
fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let (n, m) = (a.len(), b.len());
    let mut d = vec![vec![0usize; m + 1]; n + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for j in 0..=m {
        d[0][j] = j;
    }
    for i in 1..=n {
        for j in 1..=m {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut best = (d[i - 1][j] + 1).min(d[i][j - 1] + 1).min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                best = best.min(d[i - 2][j - 2] + 1);
            }
            d[i][j] = best;
        }
    }
    d[n][m]
}

/// The full-word terms a typo is measured against: the glued abbreviations (`sntop`, `kout`) are
/// left out, or `Stop` would be a snare.
const FUZZY_TERMS: [(&str, &[&str]); 5] = [
    ("kick", &["kick", "bassdrum", "kickdrum"]),
    ("snare", &["snare", "snaredrum"]),
    ("tom", &["toms", "tomtom", "floortom", "racktom"]),
    ("hat", &["hihat", "hats", "hihats"]),
    (
        "cymbal",
        &[
            "overhead",
            "overheads",
            "ride",
            "crash",
            "crashes",
            "china",
            "splash",
            "cymbal",
            "cymbals",
        ],
    ),
];

/// A typo of a full-word term (`snaer`, `kcik`, `crahs`): one edit away, both the word and the term
/// four letters or longer. Shorter words carry too little to guess.
fn fuzzy_role(word: &str) -> Option<&'static str> {
    if word.len() < 4 {
        return None;
    }
    FUZZY_TERMS
        .iter()
        .find(|(_, terms)| {
            terms
                .iter()
                .any(|t| t.len().abs_diff(word.len()) <= 1 && edit_distance(word, t) <= 1)
        })
        .map(|(role, _)| *role)
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
#[must_use]
pub fn guess_role(stem: &str) -> Option<&'static str> {
    guess_role_matched(stem).map(|(role, _)| role)
}

/// [`guess_role`] with the words of the name that matched (normalised, space-separated; the
/// hint of the Inspector shows them: `k in = kick`). Two-word matches carry both words.
///
/// # Panics
///
/// Contains an `unwrap` of the next word that is reached only when that word exists, so it does not panic in practice.
#[must_use]
pub fn guess_role_matched(stem: &str) -> Option<(&'static str, String)> {
    guess_name(stem).map(|g| (g.role, g.matched))
}

/// A role guessed from a name: the role, the words that matched, and whether the match was a
/// typo guess (one edit away) rather than a term of the table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameGuess {
    /// One of [`ROLES`] except `other`.
    pub role: &'static str,
    /// The normalised words that matched (`kick in`, `t 1`, `snaer`).
    pub matched: String,
    /// True for a Levenshtein guess.
    pub fuzzy: bool,
}

/// #285: the role of a name from the term table, else `None`. In order: an export name of #216 and
/// a name with an excluded word ([`EXCLUDED_WORDS`], a room mic, a bus) give no role; then, word
/// by word from the left, the first hit decides: a two-word term (`k in`, `bass drum`), `t` plus a
/// tom number (`t 1`, `t1`), a one-word term (`kick`, `BD` as the first word), a glued word
/// (`kickin`, `sntop`), and last a typo one edit away from a term of four letters or more
/// (`snaer`). Names like `k samp savior`, `s trigger`, `Clap` or `tomato` stay `None`.
#[must_use]
pub fn guess_name(stem: &str) -> Option<NameGuess> {
    if is_export_name(stem) {
        return None;
    }
    let normal = normalize(stem);
    let w = words(&normal);
    if w.iter().any(|x| EXCLUDED_WORDS.contains(x)) || w.first().is_some_and(|x| EXCLUDED_FIRST_WORDS.contains(x)) {
        return None;
    }
    let hit = |role: &'static str, matched: String, fuzzy: bool| Some(NameGuess { role, matched, fuzzy });
    for (i, word) in w.iter().enumerate() {
        let next = w.get(i + 1).copied();
        let first = i == 0;
        if let Some(role) = term_role(word, first) {
            return hit(role, (*word).to_string(), false);
        }
        if let Some(n) = next
            && let Some(role) = pair_role(word, n)
        {
            return hit(role, format!("{word} {n}"), false);
        }
        if *word == "t" && next.is_some_and(is_tom_number) {
            return hit("tom", format!("t {}", next.unwrap_or_default()), false);
        }
        // a term glued to a number (`t1`, `tom1`, `ft2`, `oh2`, `tom1hh`): the letters before the
        // number decide. A number with a leading zero (`ft0`, `t01`) is no drum number: no role
        // from this word.
        let parts = tokens(word);
        if parts.len() > 1 {
            let (head, second) = (parts[0].as_str(), parts[1].as_str());
            if !second.starts_with('0') {
                if head == "t" && is_tom_number(second) {
                    return hit("tom", (*word).to_string(), false);
                }
                if let Some(role) = term_role(head, first) {
                    return hit(role, (*word).to_string(), false);
                }
            }
            continue;
        }
        if let Some((role, matched)) = glued_role(word, first) {
            return hit(role, matched, false);
        }
    }
    for word in &w {
        if let Some(role) = fuzzy_role(word) {
            return hit(role, (*word).to_string(), true);
        }
    }
    None
}

/// P4-14: a sample replacement or trigger track (the name holds `samp`, `smpl` or `trig`, or a word `smp` or
/// `ssd...`: `t1 sampl`, `t samp`, `t trigger`, `T1 Smpl`, `Tom1 SSD`, `k samp savior`). Such a track is never a tom
/// close mic of the bleed rule. Its role stays what the name table gives (`t1 sampl` is a tom of the owner's scheme: it
/// detects as a tom, it only is no source or target of the rule).
#[must_use]
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
#[must_use]
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
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({"pattern": self.pattern, "role": self.role, "note": self.note})
    }

    /// A rule from JSON; 400 for a pattern without words, an unknown role or a note outside 0 to 127.
    ///
    /// # Errors
    ///
    /// Returns a 400 error for a pattern without words, an unknown role or a note outside 0 to 127.
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
    /// #285: a typo guess from the name, one edit away from a term (`snaer`): the page marks it.
    Fuzzy,
}

impl Source {
    /// The word the page gets.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Preset => "preset",
            Source::Scheme => "scheme",
            Source::None => "none",
            Source::Audio => "audio",
            Source::Fuzzy => "fuzzy",
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
#[must_use]
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
    match guess_name(stem) {
        Some(g) => Guess {
            role: g.role.to_string(),
            note: default_note(g.role),
            source: if g.fuzzy { Source::Fuzzy } else { Source::Scheme },
            matched: Some(g.matched),
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
    ///
    /// # Errors
    ///
    /// Returns a 409 error when the service has no home folder.
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
    ///
    /// # Errors
    ///
    /// Returns a 400 error for an empty name, a `rules` value that is not a list, or an invalid rule; a 409 error
    /// without a home folder; and an internal error when the preset folder or file cannot be written.
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
    ///
    /// # Errors
    ///
    /// Returns a 404 error when no preset file has this name, a 400 error for an empty name or a damaged preset
    /// file, and a 409 error without a home folder.
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
