//! M1d4 (#578): the states of the page at the first open.
//!
//! The design is `docs/design/daw-plugin/m1d4-first-open.md`. The plugin keeps ONE state per DAW track row and one for the document;
//! the page only draws them (the shell command `plugin_host_states` and the page event `plugin-states`). The page never guesses a
//! state from a timer or from "no track yet".
//!
//! This file has no host or ARA type in it, so a unit test reaches every case. The document gathers the facts of each source
//! ([`SrcFact`]) and calls [`build`].
//!
//! The limits only change the SHOWN state. They never cancel or kill work: a late arrival brings the real state back at once.

use serde_json::{json, Value};
use std::collections::BTreeMap;

/// Why a source failed (the reason the page words in DAW terms).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fail {
    /// The read of the audio failed.
    Read,
    /// The host turned the samples access off.
    AccessOff,
    /// The audio did not arrive in time (the audio deadline of a round).
    Late,
    /// A round ended with an error (the service could not analyse the track).
    Round,
    /// The worker stopped unexpectedly.
    Stopped,
    /// A limit passed (shown only; computed from the facts, never stored).
    TimedOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum State {
    Done,
    Waiting,
    Analysing,
    Asking,
    Reading,
    Offline,
    Failed,
}

impl State {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            State::Done => "done",
            State::Waiting => "waiting",
            State::Analysing => "analysing",
            State::Asking => "asking",
            State::Reading => "reading",
            State::Offline => "offline",
            State::Failed => "failed",
        }
    }
}

/// The time limits (milliseconds). `DS_ARA_FIRSTOPEN_MS` (`wait=..,read=..,round=..`) changes them for a stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// No read began: after this the row is offline.
    pub wait_ms: u64,
    /// A read with no progress for this long is failed.
    pub read_ms: u64,
    /// The least time an analysis may take before it is failed (3 x the audio length when that is longer).
    pub round_min_ms: u64,
    /// The fault run `no-limit`: none of the limits fires.
    pub none: bool,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            wait_ms: 30_000,
            read_ms: 120_000,
            round_min_ms: 600_000,
            none: false,
        }
    }
}

impl Limits {
    #[must_use]
    pub fn read() -> Limits {
        let mut l = Limits::default();
        if let Ok(v) = crate::test_switch("DS_ARA_FIRSTOPEN_MS") {
            for part in v.split(',') {
                let Some((k, n)) = part.split_once('=') else { continue };
                let Ok(n) = n.trim().parse::<u64>() else { continue };
                // clamped: no value may make a limit zero or absurd
                let n = n.clamp(50, 3_600_000);
                match k.trim() {
                    "wait" => l.wait_ms = n,
                    "read" => l.read_ms = n,
                    "round" => l.round_min_ms = n,
                    _ => {}
                }
            }
        }
        l.none = crate::test_switch("DS_ARA_FIRSTOPEN_FAULT").as_deref() == Ok("no-limit");
        l
    }
}

/// The fault runs of the check (`DS_ARA_FIRSTOPEN_FAULT`); a delivery build reads none.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Fault {
    /// A source with no track gives no state (the way before #578): the page shows its empty text.
    pub old_empty: bool,
    /// A read shows as waiting.
    pub no_reading: bool,
    /// The row names the machine name of the source.
    pub left_key: bool,
    /// A failed row has no reason.
    pub no_reason: bool,
}

impl Fault {
    #[must_use]
    pub fn read() -> Fault {
        match crate::test_switch("DS_ARA_FIRSTOPEN_FAULT").as_deref() {
            Ok("old-empty") => Fault {
                old_empty: true,
                ..Fault::default()
            },
            Ok("no-reading") => Fault {
                no_reading: true,
                ..Fault::default()
            },
            Ok("left-key") => Fault {
                left_key: true,
                ..Fault::default()
            },
            Ok("no-reason") => Fault {
                no_reason: true,
                ..Fault::default()
            },
            _ => Fault::default(),
        }
    }
}

/// The facts of one live source.
#[derive(Debug, Clone, PartialEq)]
pub struct SrcFact {
    /// The key of its track, once it has one.
    pub key: Option<String>,
    /// The DAW name (the display name rule of `names.rs`).
    pub name: String,
    /// The machine name of the source (used only by the fault run `left-key`).
    pub machine: String,
    /// The id of the DAW track row (`q<sequence id>`, else `s<creation index>`).
    pub row: String,
    /// The order of the row: the DAW order (`i32::MAX` when unknown), then the creation index.
    pub order: (i32, usize),
    /// The hits the source has (the notes).
    pub hits: usize,
    /// The source has its result, no failure, and nothing waits for new audio.
    pub done: bool,
    /// The audio file is here.
    pub wav: bool,
    /// A read runs now.
    pub reading: bool,
    /// Time since the read made progress (when `reading`).
    pub read_idle_ms: u64,
    /// A read is planned again after a failed try.
    pub read_retry: bool,
    /// Time since the source was made.
    pub age_ms: u64,
    /// Time since the audio was here and the analysis was waited for.
    pub analysing_ms: u64,
    /// The length of the audio in seconds.
    pub audio_s: f64,
    /// The reason of a failure, when it failed.
    pub fail: Option<Fail>,
    /// The worker tries the failed round again by itself (it has not given up).
    pub auto_retry: bool,
    /// Time since a press of Retry that is not finished (`None`: none).
    pub retry_ms: Option<u64>,
}

/// The state of one source.
#[must_use]
pub fn source_state(f: &SrcFact, lim: &Limits, fault: Fault) -> (State, Option<Fail>) {
    let cap = |ms: u64, limit: u64| !lim.none && ms > limit;
    let round_limit = lim.round_min_ms.max((f.audio_s * 3000.0) as u64);
    if let Some(k) = f.fail {
        // R3: while the worker tries a failed round by itself, the row says analysing
        if k == Fail::Round && f.auto_retry {
            return (State::Analysing, None);
        }
        if k == Fail::Late {
            return (State::Offline, Some(k));
        }
        return (State::Failed, Some(k));
    }
    if let Some(ms) = f.retry_ms {
        // a press of Retry: asking the DAW (no audio yet), then analysing
        if !f.wav && !f.reading {
            return if cap(ms, lim.wait_ms) {
                (State::Offline, Some(Fail::Late))
            } else {
                (State::Asking, None)
            };
        }
    }
    if f.reading || f.read_retry {
        if f.reading && cap(f.read_idle_ms, lim.read_ms) {
            return (State::Failed, Some(Fail::TimedOut));
        }
        return if fault.no_reading {
            (State::Waiting, None)
        } else {
            (State::Reading, None)
        };
    }
    if f.done {
        return (State::Done, None);
    }
    if f.wav {
        if cap(f.analysing_ms, round_limit) {
            return (State::Failed, Some(Fail::TimedOut));
        }
        return (State::Analysing, None);
    }
    if cap(f.age_ms, lim.wait_ms) {
        return (State::Offline, Some(Fail::Late));
    }
    (State::Waiting, None)
}

/// One DAW track row.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub id: String,
    pub name: String,
    pub srcs: Vec<String>,
    pub state: State,
    pub fail: Option<Fail>,
    pub hits: usize,
    /// The row failed or is offline and still shows the hits of an earlier round.
    pub old_hits: bool,
    pub order: (i32, usize),
}

impl Row {
    /// A failed or offline row has the control Retry.
    #[must_use]
    pub fn retry(&self) -> bool {
        matches!(self.state, State::Failed | State::Offline)
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("{n} {one}")
    } else {
        format!("{n} {many}")
    }
}

fn reason(fail: Option<Fail>) -> &'static str {
    match fail {
        Some(Fail::Read) => {
            "the DAW did not hand over the audio. Press Retry. If it fails again, check the clip in the DAW."
        }
        Some(Fail::AccessOff) => "the DAW stopped giving audio to the plugin. Press Retry.",
        Some(Fail::Round) => "the analysis did not finish. Press Retry.",
        Some(Fail::Stopped) => "the analysis stopped unexpectedly. Press Retry.",
        Some(Fail::TimedOut) => "reading or analysing took too long. Press Retry.",
        _ => "the audio did not arrive in time. Press Retry.",
    }
}

/// The text of a row (the DAW name first, plain words).
#[must_use]
pub fn row_text(r: &Row, fault: Fault) -> String {
    let n = &r.name;
    let body = match r.state {
        State::Waiting => "waiting for the audio from your DAW".to_string(),
        State::Asking => "asking the DAW for the audio".to_string(),
        State::Reading => "reading the audio".to_string(),
        State::Analysing => "analysing".to_string(),
        State::Done if r.hits == 0 => "no hits found: check the role and the threshold".to_string(),
        State::Done => plural(r.hits, "hit", "hits"),
        State::Offline => {
            "the DAW gives no audio for this track. In the DAW, make sure the clip is on a track and its audio file is present, then press Retry."
                .to_string()
        }
        State::Failed if fault.no_reason => "failed. Press Retry.".to_string(),
        State::Failed => reason(r.fail).to_string(),
    };
    let tail = if r.old_hits && r.hits > 0 && r.retry() {
        format!(
            " The {} shown are from before.",
            if r.hits == 1 { "hit" } else { "hits" }
        )
    } else {
        String::new()
    };
    format!("{n}: {body}{tail}")
}

/// The document: the line over the rows.
#[derive(Debug, Clone, PartialEq)]
pub struct Doc {
    /// `nocip`, `waiting`, `reading`, `analysing`, `attention`, `done`.
    pub state: &'static str,
    pub text: String,
    pub n: usize,
    pub m: usize,
    pub hits: usize,
    pub attention: usize,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct States {
    pub rows: Vec<Row>,
    pub fault_no_reason: bool,
}

/// The sources of the document grouped to rows, each with its state.
#[must_use]
pub fn build(facts: &[SrcFact], lim: &Limits, fault: Fault) -> States {
    let mut by: BTreeMap<String, Vec<&SrcFact>> = BTreeMap::new();
    for f in facts {
        if fault.old_empty && f.key.is_none() {
            continue; // the old way: no state for a source that has no track yet
        }
        by.entry(f.row.clone()).or_default().push(f);
    }
    let mut rows: Vec<Row> = Vec::new();
    for (id, mut list) in by {
        list.sort_by_key(|f| (f.order, f.row.clone()));
        let states: Vec<(State, Option<Fail>)> = list.iter().map(|f| source_state(f, lim, fault)).collect();
        // the row shows the most urgent state of its sources: failed, offline, reading, asking, analysing, waiting, done
        let (state, failv) = states
            .iter()
            .copied()
            .max_by_key(|(s, _)| *s)
            .unwrap_or((State::Waiting, None));
        let hits: usize = list.iter().map(|f| f.hits).sum();
        let first = list[0];
        rows.push(Row {
            id,
            name: if fault.left_key {
                first.machine.clone()
            } else {
                first.name.clone()
            },
            srcs: list.iter().filter_map(|f| f.key.clone()).collect(),
            state,
            fail: failv,
            hits,
            old_hits: hits > 0 && matches!(state, State::Failed | State::Offline),
            order: first.order,
        });
    }
    rows.sort_by(|a, b| a.order.cmp(&b.order).then(a.id.cmp(&b.id)));
    States {
        rows,
        fault_no_reason: fault.no_reason,
    }
}

impl States {
    fn fault(&self) -> Fault {
        Fault {
            no_reason: self.fault_no_reason,
            ..Fault::default()
        }
    }

    /// The line of the document.
    #[must_use]
    pub fn doc(&self) -> Doc {
        let m = self.rows.len();
        let hits: usize = self.rows.iter().map(|r| r.hits).sum();
        let attention = self.rows.iter().filter(|r| r.retry()).count();
        let count = |s: &[State]| self.rows.iter().filter(|r| s.contains(&r.state)).count();
        let tracks = plural(m, "track", "tracks");
        let (state, text, n) = if m == 0 {
            (
                "nocip",
                "No audio yet. In your DAW, add an audio clip on a track that has the Drum Spike plugin.".to_string(),
                0,
            )
        } else if attention > 0 {
            let t = if attention == 1 {
                format!("1 of {tracks} needs attention.")
            } else {
                format!("{attention} of {tracks} need attention.")
            };
            ("attention", t, m - attention)
        } else if count(&[State::Reading, State::Asking]) > 0 {
            let n = count(&[State::Analysing, State::Done]);
            ("reading", format!("Reading the audio: {n} of {tracks}."), n)
        } else if count(&[State::Analysing]) > 0 {
            let n = count(&[State::Done]);
            ("analysing", format!("Analysing: {n} of {tracks}."), n)
        } else if count(&[State::Waiting]) > 0 {
            ("waiting", format!("Waiting for the audio from your DAW ({tracks})."), 0)
        } else if m == 1 {
            (
                "done",
                format!("The track is ready: {}.", plural(hits, "hit", "hits")),
                1,
            )
        } else {
            (
                "done",
                format!("All {tracks} are ready: {}.", plural(hits, "hit", "hits")),
                m,
            )
        };
        Doc {
            state,
            text,
            n,
            m,
            hits,
            attention,
        }
    }

    /// The answer of `plugin_host_states` and the payload of the page event `plugin-states`.
    #[must_use]
    pub fn to_json(&self, gen: u64, doc_id: &str) -> Value {
        let d = self.doc();
        let f = self.fault();
        json!({
            "gen": gen,
            "doc_id": doc_id,
            "doc": {"state": d.state, "text": d.text, "n": d.n, "m": d.m, "hits": d.hits, "attention": d.attention},
            "rows": self.rows.iter().map(|r| json!({
                "id": r.id, "name": r.name, "srcs": r.srcs, "state": r.state.name(),
                "text": row_text(r, f), "hits": r.hits, "old_hits": r.old_hits, "retry": r.retry(),
            })).collect::<Vec<_>>(),
        })
    }

    /// The state of a row by id.
    #[must_use]
    pub fn row(&self, id: &str) -> Option<&Row> {
        self.rows.iter().find(|r| r.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fact(row: &str, name: &str) -> SrcFact {
        SrcFact {
            key: None,
            name: name.into(),
            machine: format!("m-{name}"),
            row: row.into(),
            order: (0, 0),
            hits: 0,
            done: false,
            wav: false,
            reading: false,
            read_idle_ms: 0,
            read_retry: false,
            age_ms: 0,
            analysing_ms: 0,
            audio_s: 10.0,
            fail: None,
            auto_retry: false,
            retry_ms: None,
        }
    }

    fn st(f: &SrcFact) -> State {
        source_state(f, &Limits::default(), Fault::default()).0
    }

    #[test]
    fn a_source_with_no_read_is_waiting_then_offline_after_the_wait_limit() {
        let mut f = fact("q1", "Kick");
        assert_eq!(st(&f), State::Waiting);
        f.age_ms = 31_000;
        assert_eq!(st(&f), State::Offline);
        // the limit changes only the shown state: a read that begins brings the real state back
        f.reading = true;
        assert_eq!(st(&f), State::Reading);
        f.reading = false;
        f.wav = true;
        assert_eq!(st(&f), State::Analysing);
        f.done = true;
        assert_eq!(st(&f), State::Done);
    }

    #[test]
    fn a_read_fails_only_after_the_limit_without_progress() {
        let mut f = fact("q1", "Kick");
        f.reading = true;
        f.read_idle_ms = 119_000;
        assert_eq!(st(&f), State::Reading, "a long read with progress is never failed");
        f.read_idle_ms = 121_000;
        assert_eq!(
            source_state(&f, &Limits::default(), Fault::default()),
            (State::Failed, Some(Fail::TimedOut))
        );
        f.read_idle_ms = 0;
        assert_eq!(st(&f), State::Reading, "progress brings the real state back");
    }

    #[test]
    fn the_analysis_limit_is_ten_minutes_or_three_times_the_audio() {
        let mut f = fact("q1", "Kick");
        f.wav = true;
        f.analysing_ms = 601_000;
        assert_eq!(st(&f), State::Failed);
        f.audio_s = 400.0; // 3 x 400 s = 20 min
        assert_eq!(st(&f), State::Analysing);
        f.analysing_ms = 1_201_000;
        assert_eq!(st(&f), State::Failed);
    }

    #[test]
    fn an_automatic_retry_shows_analysing_and_the_give_up_shows_failed() {
        let mut f = fact("q1", "Kick");
        f.wav = true;
        f.fail = Some(Fail::Round);
        f.auto_retry = true;
        assert_eq!(st(&f), State::Analysing);
        f.auto_retry = false;
        assert_eq!(
            source_state(&f, &Limits::default(), Fault::default()),
            (State::Failed, Some(Fail::Round))
        );
    }

    #[test]
    fn a_retry_with_no_audio_asks_the_daw_then_goes_offline() {
        let mut f = fact("q1", "Kick");
        f.retry_ms = Some(1000);
        assert_eq!(st(&f), State::Asking);
        f.retry_ms = Some(31_000);
        assert_eq!(st(&f), State::Offline);
    }

    #[test]
    fn the_row_shows_the_most_urgent_state_of_its_sources() {
        let mut a = fact("q1", "Kick");
        a.wav = true;
        a.done = true;
        a.hits = 5;
        let mut b = fact("q1", "Kick");
        b.fail = Some(Fail::Read);
        let s = build(&[a, b], &Limits::default(), Fault::default());
        assert_eq!(s.rows.len(), 1);
        assert_eq!(s.rows[0].state, State::Failed);
        assert!(
            s.rows[0].old_hits,
            "the hits from before stay and are said to be from before"
        );
        assert!(row_text(&s.rows[0], Fault::default()).contains("The hits shown are from before."));
    }

    #[test]
    fn the_document_line_follows_the_rows() {
        let none = build(&[], &Limits::default(), Fault::default());
        assert_eq!(none.doc().state, "nocip");
        assert!(none.doc().text.starts_with("No audio yet. In your DAW"));
        let mut a = fact("q1", "Kick");
        let mut b = fact("q2", "Snare");
        let w = build(&[a.clone(), b.clone()], &Limits::default(), Fault::default());
        assert_eq!(w.doc().text, "Waiting for the audio from your DAW (2 tracks).");
        a.reading = true;
        let r = build(&[a.clone(), b.clone()], &Limits::default(), Fault::default());
        assert_eq!(r.doc().text, "Reading the audio: 0 of 2 tracks.");
        a.reading = false;
        a.wav = true;
        b.wav = true;
        b.done = true;
        b.hits = 7;
        let an = build(&[a.clone(), b.clone()], &Limits::default(), Fault::default());
        assert_eq!(an.doc().text, "Analysing: 1 of 2 tracks.");
        a.done = true;
        a.hits = 3;
        let d = build(&[a.clone(), b.clone()], &Limits::default(), Fault::default());
        assert_eq!(d.doc().text, "All 2 tracks are ready: 10 hits.");
        a.done = false;
        a.fail = Some(Fail::Read);
        let f = build(&[a.clone(), b.clone()], &Limits::default(), Fault::default());
        assert_eq!(f.doc().text, "1 of 2 tracks needs attention.");
        assert!(f.rows.iter().find(|r| r.id == "q1").unwrap().retry());
        let one = build(&[b], &Limits::default(), Fault::default());
        assert_eq!(one.doc().text, "The track is ready: 7 hits.");
    }

    #[test]
    fn the_texts_use_the_daw_name_and_no_internal_word() {
        let mut a = fact("q1", "Snare Top");
        a.wav = true;
        a.done = true;
        let s = build(&[a], &Limits::default(), Fault::default());
        assert_eq!(
            row_text(&s.rows[0], Fault::default()),
            "Snare Top: no hits found: check the role and the threshold"
        );
        for k in [
            Fail::Read,
            Fail::AccessOff,
            Fail::Late,
            Fail::Round,
            Fail::Stopped,
            Fail::TimedOut,
        ] {
            let t = reason(Some(k));
            for w in ["source", "round", "sid", "relink", "/"] {
                assert!(!t.contains(w), "{t}");
            }
        }
    }

    #[test]
    fn the_fault_runs_change_exactly_their_part() {
        let a = fact("q1", "Kick");
        assert!(build(
            std::slice::from_ref(&a),
            &Limits::default(),
            Fault {
                old_empty: true,
                ..Fault::default()
            }
        )
        .rows
        .is_empty());
        let mut r = a.clone();
        r.reading = true;
        let s = build(
            &[r],
            &Limits::default(),
            Fault {
                no_reading: true,
                ..Fault::default()
            },
        );
        assert_eq!(s.rows[0].state, State::Waiting);
        let k = build(
            std::slice::from_ref(&a),
            &Limits::default(),
            Fault {
                left_key: true,
                ..Fault::default()
            },
        );
        assert_eq!(k.rows[0].name, "m-Kick");
        let lim = Limits {
            none: true,
            ..Limits::default()
        };
        let mut o = a;
        o.age_ms = 9_000_000;
        assert_eq!(source_state(&o, &lim, Fault::default()).0, State::Waiting);
    }
}
