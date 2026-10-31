//! Track-name patterns for the agent surface: exact names, globs and numbered ranges.
//!
//! A pattern is
//!
//! - an exact track name (`k in`),
//! - a glob (`t*`, `s?bot`): `*` stands for any run of characters, `?` for one; the match is
//!   not case-sensitive,
//! - a numbered range (`t1-t4`): `<prefix><lo>-<hi>` — both sides share the prefix and end in a
//!   number, so it matches every track `<prefix><n>` with `lo <= n <= hi` (`t1` to `t4`).
//!
//! [`expand`] resolves a list of patterns against the track names of a project (or the stems of
//! a scan) and keeps the order of `all`. A pattern that matches nothing is an error that names
//! it, so a typo in a script is not a silent empty export.

use crate::error::AppError;

/// The tracks of `all` that the patterns stand for, in the order of `all`, without duplicates.
///
/// # Errors
///
/// A 400 [`AppError`] naming the first pattern that matches no track of `all`.
pub fn expand(all: &[String], patterns: &[String]) -> Result<Vec<String>, AppError> {
    for p in patterns {
        if !all.iter().any(|n| matches(n, p)) {
            return Err(AppError::bad(format!(
                "the pattern {p:?} matches no track (the tracks: {})",
                all.join(", ")
            )));
        }
    }
    Ok(all
        .iter()
        .filter(|n| patterns.iter().any(|p| matches(n, p)))
        .cloned()
        .collect())
}

/// True when the track name `name` matches the pattern `p` (see the module docs). The match is
/// not case-sensitive and takes no note of spaces against underscores (`k in` names the track
/// `k_in`, the way `safe_name` writes it), so a script can use the file's stem as the pattern.
#[must_use]
pub fn matches(name: &str, p: &str) -> bool {
    let (name, p) = (key(name), key(p));
    if let Some(r) = range(&name, &p) {
        return r;
    }
    glob(&name, &p)
}

/// The comparison form: lowercase, every run of characters that is no ASCII letter or digit
/// as one `_` (what [`crate::tracks::safe_name`] keeps).
fn key(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut gap = false;
    for c in s.chars().flat_map(char::to_lowercase) {
        if c == '*' || c == '?' || c == '-' {
            // the glob knobs stay knobs
            if gap && !out.is_empty() {
                out.push('_');
            }
            gap = false;
            out.push(c);
        } else if c.is_ascii_alphanumeric() {
            if gap && !out.is_empty() {
                out.push('_');
            }
            gap = false;
            out.push(c);
        } else {
            gap = true;
        }
    }
    out
}

/// The numbered-range match: the pattern splits at the last `-` into a left side that ends in a
/// number and a right side that is a number (or ends in one) with the same prefix.
fn range(name: &str, p: &str) -> Option<bool> {
    let (l, r) = p.rsplit_once('-')?;
    let (lp, lo) = prefix_number(l)?;
    let (rp, hi) = prefix_number(r)?;
    if lp != rp {
        return Some(false);
    }
    let rest = name.strip_prefix(&lp)?;
    let n: u64 = rest.parse().ok()?;
    Some(n >= lo && n <= hi)
}

/// `<prefix><number>` with a non-empty number: the prefix and the number.
fn prefix_number(s: &str) -> Option<(String, u64)> {
    let digits = s.len() - s.trim_end_matches(|c: char| c.is_ascii_digit()).len();
    if digits == 0 {
        return None;
    }
    let (prefix, num) = s.split_at(s.len() - digits);
    Some((prefix.to_string(), num.parse().ok()?))
}

/// The glob match: `*` is any run of characters, `?` one character, everything else literal.
fn glob(name: &str, p: &str) -> bool {
    if !p.contains('*') && !p.contains('?') {
        return name == p;
    }
    let (n, p) = (name.as_bytes(), p.as_bytes());
    // the classic two-pointer glob with a star backtrack
    let (mut i, mut j) = (0usize, 0usize);
    let (mut star, mut back) = (None::<usize>, 0usize);
    while i < n.len() {
        if j < p.len() && (p[j] == b'?' || p[j] == n[i]) {
            i += 1;
            j += 1;
        } else if j < p.len() && p[j] == b'*' {
            star = Some(j);
            j += 1;
            back = i;
        } else if let Some(s) = star {
            j = s + 1;
            back += 1;
            i = back;
        } else {
            return false;
        }
    }
    while j < p.len() && p[j] == b'*' {
        j += 1;
    }
    j == p.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all() -> Vec<String> {
        ["k in", "k out", "s top", "s bot", "t1", "t2", "t3", "t4", "hh"]
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    #[test]
    fn exact_names_pass_through() {
        let a = all();
        assert_eq!(
            expand(&a, &["k in".into(), "t2".into()]).unwrap(),
            vec!["k in".to_string(), "t2".to_string()]
        );
    }

    #[test]
    fn a_glob_matches_case_insensitively_and_keeps_the_order_of_all() {
        let a = all();
        assert_eq!(
            expand(&a, &["T*".into(), "K?IN".into()]).unwrap(),
            vec![
                "k in".to_string(),
                "t1".to_string(),
                "t2".to_string(),
                "t3".to_string(),
                "t4".to_string()
            ]
        );
    }

    #[test]
    fn a_range_covers_its_numbers_including_its_ends() {
        let a = all();
        assert_eq!(
            expand(&a, &["t1-t3".into()]).unwrap(),
            vec!["t1".to_string(), "t2".to_string(), "t3".to_string()]
        );
        assert_eq!(
            expand(&a, &["t2-t4".into()]).unwrap(),
            vec!["t2".to_string(), "t3".to_string(), "t4".to_string()]
        );
        assert_eq!(expand(&a, &["t4-t4".into()]).unwrap(), vec!["t4".to_string()]);
    }

    #[test]
    fn a_range_with_prefixes_that_differ_is_no_range() {
        let a = all();
        // `k in-k out` has no trailing number: it is an exact name, and no track has it
        assert!(expand(&a, &["k in-k out".into()]).is_err());
    }

    #[test]
    fn a_pattern_that_matches_nothing_is_an_error_that_names_it() {
        let a = all();
        let e = expand(&a, &["x*".into()]).unwrap_err();
        assert_eq!(e.code, 400);
        assert!(e.message.contains("x*"));
        let e = expand(&a, &["t5-t9".into()]).unwrap_err();
        assert_eq!(e.code, 400);
        assert!(e.message.contains("t5-t9"));
    }

    #[test]
    fn a_pattern_list_deduplicates() {
        let a = all();
        assert_eq!(
            expand(&a, &["t*".into(), "t1".into()]).unwrap(),
            vec!["t1".to_string(), "t2".to_string(), "t3".to_string(), "t4".to_string()]
        );
    }

    #[test]
    fn the_glob_knobs_do_what_the_docs_say() {
        assert!(matches("s top", "s?top"));
        assert!(matches("s top", "s*"));
        assert!(matches("t10", "t1*"));
        assert!(!matches("t1", "t1*0"));
        assert!(!matches("hh", "t*"));
        assert!(matches("t1-t4", "t1-t4"));
    }

    #[test]
    fn a_stem_with_spaces_names_the_track_with_underscores() {
        assert!(matches("k_in", "k in"));
        assert!(matches("k in", "k_in"));
        assert!(matches("S_TOP", "s top"));
    }
}
