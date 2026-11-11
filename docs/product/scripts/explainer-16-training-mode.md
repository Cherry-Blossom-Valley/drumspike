# Explainer 16: Training Mode (chapter 16)

Chapter 16 of `docs/product/feature-path.md`. Narration for `tools/media/tts.py`: one `##` heading
per scene; lines that start with `>` are visual notes and are not spoken. Target length: about
2.5 minutes. s01 is the purpose (3 sentences at most, no API name); s02 to s07 are the user's
process end to end, in this order: on, work, log, export, measure, loop. The work scene uses the
demo session (the snare close mic `s top`) as explainer 15 does.

Facts used (all checked against the code, not from memory):
- View > Training Mode is a check item, off by default, a page setting that survives a restart;
  while it is on the status line reads `training: N records` for the open project:
  `static/keymap.js` (`training_mode`), `static/status.js` (`trainingOn`, `drawTraining`,
  `trainingRefresh`), `crates/app/src/training.rs` (`training_status` answers
  `{on, records, path}`).
- A move, add, delete or confirm, and every Learn This or Forget This mark, appends one record
  per affected hit to `<home>/training/<project id>.jsonl` after the edit applied; a failed log
  never fails the edit: `crates/app/src/training.rs` (`training_examples`,
  `training_confirm_examples`, `training_learn_examples`, the module docs), `static/keymap.js`
  (`confirm_hit`).
- The file name is the first 12 hex digits of the SHA-256 of the project's creation time (else of
  its folder name), so a rename keeps the file; the log is never part of the project:
  `crates/app/src/training.rs` (`project_key`, the module docs).
- A record is self-contained: the audio window is 40 ms before and 10 ms after the position
  (`BEFORE_MS`, `AFTER_MS`); it carries the rule versions in force (`refine`, `detect`), the
  hit's confidence, the view with its `precision` (`sample` at a pixel per sample or more, `ms`
  at a coarser zoom) and, for a move, add, delete or confirm, the `placement` measures `edge`,
  `line_noise`, `line_slope`, `onset_spread`, computed on demand, so it can be read without the
  session: `crates/app/src/training.rs` (`BEFORE_MS`, `AFTER_MS`, `training_log`, the module
  docs), the release notes 0.9.23 (the view), 0.9.25 (the placement measures).
- The record shown is the recorded tutorial's own move (part-agent ra-move): the k in hit at
  sample 180008 moved onto the last line sample 180006 (probed on the recording page, 2026-11-08).
- With Training Mode on, the review queue also doubts the placement of a hit: no steep edge at
  the stored position, a line before it that is noisy or sloping, or onset estimates that
  disagree by 4 samples or more: the release notes 0.9.22 (#344), `docs/guide/09-doubtful-hits.md`.
- `tools/training_export.py` prints the aggregates per role and op (the `to - from` histogram,
  the precisions, the count per refine rule version, exact and approximate records) and writes
  the truth list `[{track, role, from, to, op, precision, exact, edge, line_noise, line_slope,
  onset_spread}]` in samples; the printout carries no project names and no paths, the truth
  file carries the track names: `tools/training_export.py` (the module docs).
- The truth list is what the placement is measured against, and the measures let a fit weight a
  clear stroke over a wobbly one: `tools/training_export.py` (#363 note), the release notes
  0.9.25.
- The counts shown are Session F's real aggregates (the coordinator's check of 0.9.26 on one
  internal session): 522 hits seen, 124 moved, 314 confirmed - the same counts explainer 15
  shows.
- Off, nothing changes: no new field, no new file, the same responses: the release notes 0.9.22.

## s01-why

> One line across the screen: your corrections, the record, the truth list, a better placement. No menu yet, no log yet.

Training Mode exists so that Drum Spike can learn to place hits the way you do. It writes down every hit you correct, with the sound around it, and those corrections become the truth the placement is measured against. It never changes a hit on its own, and when it is off the app behaves exactly as without it.

## s02-on

> The View menu open with Training Mode checked; the status line of the app reads training: 0 records.

Turn it on in the View menu: Training Mode. The switch belongs to the app, not to the project, and it survives a restart. While it is on, the status line counts the records of the open project.

## s03-work

> The s top lane of the demo session at sample zoom around one hit, its last line sample marked; on the right, the edits that write a record: Hits > Next Doubtful Hit (D), Edit > Nudge Hit Earlier (Left), Edit > Nudge Hit Later (Right), Edit > Add Hit at Playhead (H), Edit > Delete Hit, Hits > Confirm Hit.

Then work as you always do. The review queue walks you to the doubtful hits; at sample zoom you nudge a hit by whole samples onto its last line sample. Add a hit, delete one, confirm one as reviewed: every one of these writes one record. An agent writes the same kind of record, and each record says whether a person or an agent made it.

## s04-log

> The record of the recorded tutorial's own move as text: the action, the track, by person, from and to (k in 180008 moved onto 180006), precision sample, and the audio window, 40 ms before and 10 ms after; the title names the file: <home>/training/<project id>.jsonl.

One file per project, in the app's data folder, named by the project's id: a rename keeps the file. A record says what you did, the old and the new position, and the sound around the hit.

## s05-export

> A terminal runs the export over the training logs; the export takes only your own records by default (--by person); the printout counts the records per role and op (Session F: 124 moved, 314 confirmed) and writes the truth list.

From the source checkout, one script turns the logs into the truth list. The printout names no project and no path, so it is safe to paste; the truth file carries track names, so keep it with the logs.

## s06-measure

> The truth row of the move at the top; below it two boxes, the version before and the version after, each scored against your rows; the note says why: a clear stroke weighs more than a wobbly one.

The truth list is the yardstick: when the placement rule changes, the new version is scored against your rows, next to the old one. A clear stroke weighs more than a wobbly one: every record carries the line measures. The next version must place your hits as you did.

## s07-loop

> The loop as four boxes with arrows and one arrow back: your edits, the log, the truth list, the next release. The note at the end: the records stay on your machine until you send them.

That closes the loop: the hits you fix are what the next placement rule gets measured against. Nothing leaves your machine by itself; the log stays in your data folder, and you decide what to send.
