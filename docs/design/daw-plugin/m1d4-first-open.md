# M1d4: clear states at the first open

Ticket #578, umbrella #503 (the DAW plugin program), part of M1 (#510). This note is the design only. No code is written before the
coordinator has ruled on the open questions in section 8.

Goal: at every moment after the plugin window opens, the user sees what the plugin does and what the user can do: waiting for audio,
reading audio, analysing n of m, done, failed (reason and Retry), offline, or "no clip: do this in the DAW". The role models show one
status per clip and a clear action when something is missing. Never an empty editor.

The standalone page and app are not changed. Every new path of the page is behind `hostMode()`. No existing expected value of a
standalone test changes. No new element is visible or focusable in the standalone page.

## 1. What the page shows today (facts read in the code on b40509b)

The page learns of a track only when the service has a project track for it. A source gets its track (`SourceState.track`,
`lib.rs:2606`) at the end of the FIRST round that includes it. `rows.rs` skips every source without a track (`lib.rs:1037`), so
`plugin_host_regions` has no row for it either. The consequences, case by case:

| Case at the first open | What happens in the plugin | What the page shows today |
|---|---|---|
| The window opens, the DAW has not given any audio | no source has a track | `HOST_EMPTY` (`static/tracks.js:11`): "No drum tracks yet. In your DAW, put the Drum Spike plugin on a drum track." This is wrong when the plugin is on a track already. |
| The host enables the samples access, the read runs (`Msg::Reading`, `lib.rs:4094`) | `reader_running`; no track | the same `HOST_EMPTY` text, nothing says "reading" |
| The audio is read, the round waits in the queue or runs | `has_wav`, no events, no track | `HOST_EMPTY_BUSY` "Analyzing the drum tracks. The hits appear here when it is done." only when the page found a service job by its 2 s poll (`hostWatchAnalysis`, `analysis.js:158`). Between the read and the job (worker settle time, deferral) the text is `HOST_EMPTY`. No n of m, no track names. |
| One of three tracks is analysed, two wait | the first round gives tracks to all its sources at the end | no row until the round ends, then all rows at once |
| The read of a source fails (no reader, spawn failed, panic, access turned off, retries used up) | `failed` and, at the end, `analysis_failed` (`lib.rs:2272`, `:1706`); the host is told "complete" | nothing. A source without a track has no row, so a failed first read is invisible. The report file has the row `source_read_failed`. |
| The audio does not arrive for 30 s (`audio_deadline`) | `source_late` (`lib.rs:2439`), `analysis_failed` | nothing (no row) |
| The round fails (the service gives an error) | `analysis_failed` for the sources of the round (`lib.rs:2497`), retries 3 times with growing wait, then `retry_gave_up` in the report | nothing for a source without a track. A track that has a row gets the red mark from `markTrackErrors` only after a page job; the text is the service's text ("analysis failed: ..."). |
| A restored track whose file is not matched yet | `changed` | "offline (changed)" and the words "relink it first" (an action that does not exist in a DAW; #582 fixed the cause, the wording is left open by #582) |
| No clip at all (no source, or every source has no region) | no row | `HOST_EMPTY` (the same text as above) |
| The DAW did not open the plugin window (a track with no clip of the plugin is selected, owner hand check 2026-11-22) | no window | nothing can show; the help text and the first-open line must say that the page shows when a track with the plugin is selected (comment on #578) |

So four facts are missing in the plugin and the page cannot guess them: a state per source before it has a track, a reason for a
failure (`analysis_failed` is one bool), a time limit for the waits, and a way to retry one track.

## 2. The source of truth

One new module `plugins/ara/src/firstopen.rs` (no host or ARA type in it, so a unit test reaches every case; the pattern of
`rows.rs`). The document builds the facts of each live source and the module returns:

```
{gen, doc: {state, text, n, m, hits, failed}, rows: [{id, name, srcs: [key|null...], state, text, why, hits, retry: bool}]}
```

- The page asks with the shell command `plugin_host_states` and gets pushes as the page event `plugin-states` through the same gate
  as `plugin-regions` (at most one message in 50 ms, the newest state always arrives, the document compares the text with what
  the page was told last, so a tick that changes nothing sends nothing).
- The page draws it and nothing else. It never derives a state from a timer, a poll or from "no track yet".
- The row id is the id of the region sequence (`q<id>`, as in `rows.rs`), else `s<creation index>` for a source with no region.
  A source with no track is in the list with its DAW name from the sequence (`names.rs`) or, with no sequence, the name of its
  first region, else the name the host gave the source. Rows of sources that have a track use the same ids as `plugin_host_regions`,
  so the page can put a state on the DOM row of the track.
- `hostMode()` off: the page never calls the command and never listens to the event.

### Facts to add in `SourceState`

| Fact | Why | Where it is set |
|---|---|---|
| `fail: Mutex<Option<Fail>>` with `Fail {kind, at}` | the reason of a failure. Today `analysis_failed` is a bool at 9 places. Kinds: `ReadFailed`, `AccessOff`, `AudioLate`, `RoundFailed(text)`, `Stopped` (worker restart), `NoAudioGiven` | next to each `analysis_failed.store(true)`: `lib.rs:1706`, `:2272`, `:2439`, `:2497`, `:2614`, the worker restart (`:1872`), and the read failure paths of `start_read` (`:4085`, `:4109`) |
| `created_at: Instant` | the limit for "waiting for audio" | at the creation of the source |
| `read_started_at` | the limit for "reading" | in `start_read` before the thread starts |
| `in_round: AtomicBool` | "analysing" for exactly the sources of the round that runs now, and the refusal of Retry while it runs | `WorkerState.current` is a worker-local list; the flag is the shared copy, set at `round_start`, cleared at `round_end` |
| `retrying: AtomicBool` | a second press does not start a second job | the retry command, see section 5 |
| the hit count | "done (n hits)" | `doc.events` holds the events by key; the module reads the count |

Existing `analysis_failed` and the host protocol (`isAudioSourceContentAnalysisIncomplete` and the rest) do not change.

## 3. The states and their exact texts

`<Name>` is the DAW track name (the label of #575; the plugin gives `name`, the page uses `trackLabel` for a key it knows). Texts are
plain words. No path, no source, round, sid, no host name. Counts use the singular ("1 hit").

### Row states

| State | When (the fact) | Row text | Under it or next to it |
|---|---|---|---|
| waiting | the source exists, no read started, not failed, younger than the limit | `<Name>: waiting for the audio from your DAW` | none |
| reading | `reader_running`, or a read retry planned, younger than the limit | `<Name>: reading the audio` | none |
| analysing | `has_wav`, no result yet (queued, or `in_round`) | `<Name>: analysing` | document line gives n of m |
| done | has events, not failed, not stale, no `reanalyse` | `<Name>: 142 hits` (or `1 hit`, `no hits found`) | none |
| failed | `fail` is set | `<Name>: <what failed>. <what to do>` | the control `Retry` |
| offline | the host gave no audio within the limit, or the read says the audio is not there | `<Name>: the DAW gives no audio for this track. In the DAW, make sure the clip is on a track and its audio file is present, then press Retry.` | the control `Retry` |

`offline` and `failed` both end with Retry. The difference is the cause: offline = the DAW never started to give audio (no
read began in the limit); failed = a read or round began and ended with an error. See open question Q2.

### Failed reasons (`why`, in DAW words)

| Kind | Text after the name |
|---|---|
| `ReadFailed` | `the DAW did not hand over the audio. Press Retry. If it fails again, check the clip in the DAW.` |
| `AccessOff` | `the DAW stopped giving audio to the plugin. Press Retry.` |
| `AudioLate` | `the audio did not arrive in time. Press Retry.` |
| `RoundFailed` | `the analysis did not finish. Press Retry.` The service text goes to the tooltip only, after a pass that removes the words of the app (relink, project, file path). |
| `Stopped` | `the analysis stopped unexpectedly. Press Retry.` |
| `TimedOut` (state limit, section 4) | `reading or analysing took too long. Press Retry.` |

### Document states (one line over the rows, `role=status`, `aria-live=polite`)

| State | When | Text |
|---|---|---|
| no clip | no live source, or every source has no clip in the DAW (`gone` or no region) | `No audio yet. In your DAW, add an audio clip on a track that has the Drum Spike plugin.` |
| waiting | at least one row waiting, none reading | `Waiting for the audio from your DAW (m tracks).` |
| reading | any row reading | `Reading the audio: n of m tracks.` |
| analysing | any row analysing | `Analysing: n of m tracks.` (n = rows done) |
| done | every row done | `All m tracks are ready: N hits.` and the line fades to the usual status after 8 s (the state stays in the data) |
| failed | any row failed or offline | `k of m tracks need attention.` plus the control `Retry all` when k is more than 1 |

Priority when rows differ: failed, then reading, then analysing, then waiting, then done. The line never says "reading" or
"analysing" when no row has that state. The line is the only place for `Retry all`.

"No clip" replaces `HOST_EMPTY`. `HOST_EMPTY` and `HOST_EMPTY_BUSY` and the 2 s poll text of `hostWatchAnalysis` are no longer drawn
in host mode (the progress bar of a page job stays).

### Where the page draws a row state

- A row that has a track (a DOM row exists): a one-line strip in the track header under the role line (class `hostState`). Done shows
  the hit count; every other state shows its text. `Retry` is a button in the strip.
- A row with no track yet (waiting, reading, analysing, failed before the first round): a card in the lanes area, one card per
  DAW track, in DAW order, same text and button. It is replaced by the real row when the track exists. The cards are built only in
  host mode, so the standalone DOM does not change.
- The strip and the cards are created only after the first `plugin-states` answer, so with the mode off no element exists.

## 4. Time limits (no wait for ever)

Each state that waits has a limit that turns into `failed` or `offline` with a reason. The values are constants in `firstopen.rs`;
a test switch (`DS_ARA_FIRSTOPEN_MS`, in `switches.json`) shortens them for a stage. A delivery build reads no switch.

| State | Limit | Turns into |
|---|---|---|
| waiting (no read began) | 30 s from `created_at` (the same number as `audio_deadline`, `lib.rs:1796`; the round goes on without the source then and `source_late` is logged) | offline |
| reading | 120 s with no read progress. Needs a progress counter in `read_samples`; if that is too heavy, a fixed 10 min from `read_started_at` | failed (`TimedOut`) |
| analysing | 10 min since `round_start` for the round that holds the source (the worker is in-process: the only hang is a stuck service job) | failed (`TimedOut`) |

The time limit is a fact computed by the plugin when it builds the answer, plus a timer in the plugin (not in the page) that pushes
the change when a limit passes while nothing else changes. The page never counts.

A state that ended in time limit does not stop the real work: if the audio arrives later the state changes to the real one by
itself (reading, analysing, done), as the audio deadline does today.

## 5. Retry

One control per failed or offline row, `Retry all` on the document line when more than one row is failed or offline.

The plugin command `plugin_host_retry {rows: [id...]}` (an editor route; also added to `SHELL_COMMANDS`, the lists in `editor.rs`,
`shell.js` and the test host). It runs for the sources of the named rows whose state is failed or offline:

1. Refused (answer `{ok: false, reason}`, the page shows the reason on the line) when the source is `in_round` or `reader_running`,
   or `retrying` is already set. `retrying` is set with `compare_exchange`, so a second press, a double click and `Retry all`
   together with a row Retry start one job. It is cleared when the state leaves failed.
2. The failure is cleared (`fail`, `analysis_failed`), `read_tries` is set to 0.
3. No audio file yet: the read is planned on the host's main thread through the due-time list that exists for the read retry
   (`read_retry_at = now`; `due_read_retries` and the main-thread pump read it). The reader is never made on a request thread (the
   ARA rule, `start_read`).
4. The audio is here: a `Msg::Request` for that source goes to the worker (the retry list of the document is cleared for it, as a
   request does today, `lib.rs:2240`). Other sources of the document are not asked.
5. The edits and settings of the other tracks stay: the round uses the existing rule that keeps the hits the user placed and the
   settings (`keep_settings`, the `keep` rule of the round). The state row of the other tracks is not touched.
6. Retry is not an undo step: it changes no project data of the user. The page puts nothing on the undo stack.
7. The automatic retry of the worker (3 tries, `RETRY_MAX`) is not changed. A row shows `failed` after it gave up
   (`retry_gave_up`), or at once for a kind that has no automatic retry. While the automatic retry is planned the row says
   `analysing` (the fact `retry due`); see Q3.

The report file gets one row `retry` per press (rows, accepted, refused with the reason).

## 6. Agreement with other parts

- Drag handles (#576): a row that is not `done` has no ready handle. Today `prepare.rs` says `none` when a row has no hits (and
  `preparing` while it waits for its pass, `prepare.rs:898`). The new rule goes in the plugin: `State::None` unless the row state is
  `done`, so a failed row with stale hits does not offer a drag. The page only draws what `plugin_deliver_state` says. Standalone is
  untouched (the handles do not exist without the mode).
- Selection follow (#562): the cards and strips carry the row id. A row that the host hides (`.hostHidden`) hides its card; a
  selected row gets the mark `.hostSel` on the card as on a track. The document line is always shown. The selected track of the
  page (`setSelTrack`) is not set for a card (no track yet).
- Reload after a round (`plugin-reload`): the page loads the project again. The states do not depend on it; they come as their own
  event. A strip is redrawn in `buildTrackDOM` from the last state answer, so a rebuild never loses it.
- Offline from the service (`t.offline`, "relink"): in host mode the text of the strip wins and the header badge "offline" with
  the word "relink" is replaced by the state text. The service text stays in the tooltip only. This is the open point left by #582.
- Screen readers: the document line is `role=status`, `aria-live=polite`, `aria-atomic=true`. The page writes its text only when
  the state or n or m changes, not at every poll or tick, so a change is announced once. Row strips carry no live region (the
  document line says the change); a row that turns failed also changes the document line, so it is announced there.
- Status bar and progress bar: the existing progress bar of a page job (`pollJob`) stays. The document line does not repeat it.

## 7. Stages and fault runs

New part of the page tests, `tests/e2e_app/test_host_firstopen.py`, with the mode on (`host_app`) and off (`app`): the page draws
each state from a given answer, no element exists with the mode off, no existing test value changes. New unit tests in
`firstopen.rs` for every state, every limit and every priority. Plugin stages in `check.py` (the real plugin, a real web view, the
test host, only the gate cache folder `out/`; the guard of #577 stays; no system drag, no system event, no panel, no pasteboard):

| Stage | Proves | Fault run (names the reason) |
|---|---|---|
| `editor-firstopen-waiting` | a source that the host has not read yet: the row and the document line say waiting, with the DAW name; no `HOST_EMPTY` text | `DS_ARA_FIRSTOPEN_FAULT=old-empty`: the plugin gives no state for a source without a track; the stage fails with "the old empty text shows" (line 4) |
| `editor-firstopen-reading` | a slow read (`slow-read` scenario of the test host) shows reading, then analysing n of m, then done with the hit counts | `no-reading`: the state skips reading; fails with "no reading state" |
| `editor-firstopen-analysing` | `n of m` follows the rows; each row has its DAW name; no key text | `left-key`: fails with "holds the key" |
| `editor-firstopen-done` | done shows counts; the handles are ready only for done rows | `handle-early`: a handle is ready before done; fails with "handle ready before done" |
| `editor-firstopen-failed` | a read that fails (`update-fail-reads`/`no-reader-thread` scenarios) shows failed with the reason text and Retry | `no-reason`: the row shows failed without a reason; fails with "no reason" |
| `editor-firstopen-retry` | Retry reads and analyses only that track; the user's edits and settings of the other tracks are byte for byte; a second press and a press during a round are refused; no undo step | `retry-all-tracks`: Retry runs a round for every track; fails with "other tracks touched" |
| `editor-firstopen-noclip` | a document with no clip shows the one line that says what to do; no empty page | `old-empty` as above |
| `editor-firstopen-limits` | the three limits with `DS_ARA_FIRSTOPEN_MS`: a wait ends in offline or failed with a reason | `no-limit`: no limit; fails with "still waiting after the limit" |
| `editor-firstopen-once` | a burst of ticks gives one change of the document line (the announce count) | `announce-every-tick`: fails with "announced more than once" |

A run with no usable page report fails with "no usable run" (as `editor-leftovers-fault-norun`). Every wait of a test is false before
its action (for example the "Retry pressed" wait starts from the state `failed`, not from a state that the fixture has already
reached). The standalone stage `standalone-diff` is expected to pass on this ticket (only `plugins/ara/` and `docs/` change besides
`static/` guarded calls; the note for the shared files of `static/` is in section 9).

## 8. Open questions for the coordinator

- Q1. Where does the state of a source with no track come from for the page row? My proposal: the plugin gives a list of its own
  (`plugin_host_states`), keyed by the sequence id, not an extension of `plugin_host_regions`. That keeps song time rows as they are.
  Alternative: give a source a "provisional track" at creation. That changes the project and the settings path and I do not want it.
- Q2. `offline` vs `failed`. One state `failed` with the reason `the DAW gave no audio` would be simpler (five states instead of six).
  The brief lists offline as its own state, so the note has it. Do you want both?
- Q3. While the automatic retry of the worker is planned (3 tries, 2 s then doubling), should the row say `analysing`, or `failed`
  with a note "trying again in 4 s"? My proposal: `analysing` until it gave up, then `failed`. A press of Retry restarts the count.
- Q4. Limits. The values in section 4 are mine: 30 s waiting, 120 s of read with no progress (or 10 min if no counter), 10 min
  analysing. A long take read from a slow disk may need more. Do you want other values, or a limit for reading only after
  progress stops?
- Q5. Done rows. Do you want the hit count text in every track header (a new line for every track, always), or only the
  document line "All m tracks are ready: N hits" and no strip for a done row? The strip for done is more text on every header.
  My proposal: the strip only for done rows that have 0 hits, and the count stays in the existing header and info line.
- Q6. Text for "no clip": `No audio yet. In your DAW, add an audio clip on a track that has the Drum Spike plugin.` Does that fit
  LUNA and the other ARA hosts without a host name? The comment on #578 asks that the help and first-run line also say the page
  shows when a track with the plugin is selected. I would extend the first-open line of #575 by that sentence in the same commit
  as the states (one text change in `static/state.js` behind `hostMode()`, plus the help text).
- Q7. Shared code in `static/`: `buildTrackDOM` (`tracks.js`), `hostEmptyText` and `setAnalysisButtons` (`analysis.js`) must call
  the new module by a guarded call. All new page code goes in a new file `static/hoststate.js` (in `JS_FILES`, in `index.html`
  after `hostdefaults.js`). Agreed?
- Q8. The retry of a read needs the host's main thread. The existing pump for due read retries runs in the host's
  `notifyModelUpdates`. A host that never calls it would never read again (no change from today, but Retry would then look
  dead). In that case the row says `failed` again after the next limit with the reason `timed out`. Acceptable?

## 9. What is not proved and what is left

- That this reads well in a real DAW. The stages run the page in a web view of the test host. The hand check is in section 10.
- The texts in section 3 are not tested with a user.
- The limit values are not measured on a real DAW with a long take.
- No change in `crates/`. Changes: `plugins/ara/` (`firstopen.rs`, `lib.rs`, `editor.rs`, `editor/shell.js`, `check.py`,
  `switches.json`, the README), `static/hoststate.js` (new) plus guarded calls in `static/tracks.js`, `static/analysis.js` and
  `static/state.js`, `static/style.css` (rules under a host class), `static/index.html` (one script line, no element), this note,
  `tests/e2e_app/test_host_firstopen.py` and the coverage list `tests/jscov.py`.

## 10. Hand check in a real DAW (after the code)

1. Add the plugin to a track that has no clip. Open the window. Expect: the one line that says what to do in the DAW; no empty page.
2. Add a clip with a long take. Expect: waiting or reading with the DAW name of the track, then analysing, then done with the count.
3. Add four tracks at once. Expect: `Analysing: n of 4 tracks` with n rising, one change at a time; a screen reader says each change once.
4. Remove or take offline the audio file of one clip in the DAW before the plugin reads it. Expect: that row says what failed and
   shows Retry; the other rows are done. Fix the file in the DAW, press Retry. Expect: that row only reads and analyses; hits you edited in
   other tracks are unchanged; Edit > Undo does not undo the retry.
5. Press Retry twice fast. Expect: one job.
6. Select a track in the DAW. Expect: the card or strip of that track has the selection mark; hidden tracks hide their cards.
7. Look for the words relink, source, round, a path, or a host name in any text of the page. Expect: none.

## 11. Rulings of the coordinator (2026-11-24), part of the design

- R1 A separate state list from the plugin (`firstopen.rs`, `plugin_host_states`, event `plugin-states`). No provisional track.
- R2 Two states stay. offline = the DAW gives no audio: the text says what to do in the DAW, with Retry. failed = an error of read or
  analysis: reason and Retry.
- R3 During the automatic tries of the worker the row says analysing. failed only when the worker gives up. No try counter in the text.
- R4 No fixed cap while progress is visible. reading: failed after 120 s WITHOUT progress only (no fixed 10 min). analysing: a progress
  fact of the worker if there is one (no progress for 120 s is failed); if there is none, the limit is max(10 min, 3 x the audio length
  of the track). waiting: 30 s to offline. A late arrival brings the real state back (tested). A limit never cancels or kills work; it
  changes only the shown state and allows Retry. A Retry while the old job is still alive starts no second job for that track (tested).
- R5 A strip only on rows that are not done or have 0 hits, plus the document line. A done row with 0 hits says: "no hits found: check
  the role and the threshold".
- R6 The no-clip text stays. The first-open line of #575 and the help get: "the page shows the tracks that have the plugin; select one in
  your DAW". No host name in any text.
- R7 `static/hoststate.js` with guarded calls only, in `tests/jscov.py` as `hostdefaults.js` is.
- R8 After a Retry of a read the row says "asking the DAW for the audio" and goes to offline after the 30 s wait limit with the DAW text.
- R9 A failed or offline row that still has hits from an earlier round keeps them visible; the strip says the hits are from before the
  failure; no ready drag handle. The edits of the user on that row are never dropped by a failure or by Retry (tested).
- R10 Report rows carry no path and no audio file name beyond what existing rows carry; DAW track names only under the existing report
  switch (the privacy class of #562).
- R11 `DS_ARA_FIRSTOPEN_MS` and the fault switch go through `test_switch` (inert in a delivery build) and into `switches.json` and the
  README. Each fault run asserts its reason text. A stage that runs ignored tests fails when 0 tests ran.
- R12 A panic or a poisoned lock in the state path never blanks the page: the command answers with the last good list, or an error that
  the page shows as one line.

## 12. As built (differences from sections 1 to 9)

- The state list is built from every live source: the row id is `q<region sequence id>` of the first region (else `s<creation index>`), the name is the display name rule of
  `names.rs` with the sources that have no track included. A source on two sequences is shown in the row of its first region.
- Retry analyses only the asked tracks: `DocService::only_sources` limits the round (the service rounds analysed every track before). The fault run `retry-all-tracks` ignores it.
- The automatic tries of the worker show as analysing for a time that is computed from the next due time plus 3 s (the entries of the worker are cleared by any change of the
  document, for example the poll of the page that reads a finished job, so the shown time ends by itself and never stands for ever).
- The page asks `plugin_host_states` every second while a row is not done and draws what it gets; the plugin pushes the same answer as the event `plugin-states`.
- Cards (rows with no project track yet) are not hidden by the DAW selection follow: they have no track key to map. A strip in a header follows its row.
- A done row with hits has no strip (R5); a done row with no hits has one with the hint. The line of a finished document goes quiet after 8 s.
- The handles: `Prep` takes a gate from the document; a row that is not done is `preparing` (in progress) or `none` (failed or offline) and `plugin_deliver_take` refuses it.
- Limits that no stage runs: the analysis limit (`max(10 min, 3 x audio)`) is covered by unit tests only, because 3 x the audio length of the fixtures is 9 s and the stage
  would need a longer wait than the gate allows. The read limit and the wait limit have stage runs (`editor-firstopen-limit`, `editor-firstopen-offline`).
- Stages: `editor-firstopen` (flow, offline with a late arrival, failed with Retry of one track, a limit run, stale hits kept) and `editor-firstopen-fault` (one fault run for each
  reason).

## 13. Round 2 (review of c952e19)

- M1: a press of Retry that is older than the wait limit is over, so a row that never gets audio can be retried again (Offline, Retry, asking for 30 s, Offline, Retry ...).
  The mark is taken again with a compare and exchange from the value that was seen, so two fast presses are still one job. The mark also ends when a round ends on the
  old audio. I did not clear it where the reader gives up (`due_read_retries` with the access off), because that would end the state "asking the DAW for the audio"
  at once instead of after the wait limit (R8). Stage: `editor-firstopen-offline` presses again after the limit; the fault run `retry-dead-end` shows the dead end.
- S2: the page poll goes on after an error answer and after a rejected call. S3: the analysis clock starts at the round start and ends at the success; a deferral
  does not count. S4: the Retry-only mark ends when a request has nothing to do and when the source leaves. S5: a source is in the row of every region sequence it
  has a clip on (its hits count once in the document); each row is in the list and gated. S6: a panic in the gate answers `*`: every row counts as preparing.
- N1: the poll is 1 s for any state in progress and for the first 30 s of every state, then 5 s while rows are empty, offline or failed; a new state or a press resets it.
  N2: the focus goes to the row strip when the Retry button goes. N3: `Limits::read` and `Fault::read` are read once per process.
- N4, what is NOT proved, plainly: (1) the analysis limit is covered by unit tests only, no stage waits that long; (2) the panic path (last good list, one error line,
  gate closed) is covered by unit tests only; (3) a Retry of a read works only when the host calls back (notifyModelUpdates): a host that never does leaves the row
  asking, then offline again. Nothing was tried in a real DAW.
