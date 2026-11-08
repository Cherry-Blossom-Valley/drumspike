# Explainer 15: An agent fine-tunes the song (chapter 15)

Chapter 15 of `docs/product/feature-path.md`. Narration for `tools/media/tts.py`: one `##` heading
per scene; lines that start with `>` are visual notes and are not spoken. Target length: about
2.5 minutes; it is the explainer half of `explainer-15-agent.mp4` (the recorded tutorial half is
`rec-agent.md`). Use the demo session (the snare close mic `s top`, the kit-check hits) as
explainer 10 does.

Facts used (all checked against the code, not from memory):
- The owner's convention (#351): a hit sits on the last line sample before the stroke's first
  motion of either sign; a pre-ripple, bump or dip that leads into the stroke belongs to the
  stroke: `static/agent.js` (the comment above `judge`), `docs/guide/04-first-sample.md`.
- The sample-zoom window of a look is 0.9 ms before and 0.6 ms after the hit;
  `samples` in the answer are shares of the local peak and `hit_at` is the place of the hit's
  own sample in them: `static/agent.js` (`SAMPLE_BEFORE_MS`, `SAMPLE_AFTER_MS`, `view`).
- `look(track, index)` selects the hit and sets the view; `move(track, index, samples)` nudges
  by whole samples with the view at sample zoom, so the record says precision `sample`, and
  answers `{moved, index, from, to}`: `static/agent.js`, `docs/agent-api.md` (Page helpers).
- `confirm(track, index)` is Hits > Confirm Hit: the op applies nothing, the record says
  `from` = `to` = the stored position: `docs/agent-api.md`, `crates/app/src/training.rs`.
- Every move and confirm runs through the code the keyboard uses (`nudgeSelected`,
  `confirmHit`), so the training record of an agent is byte for byte the record of a person:
  `static/agent.js` header, `docs/guide/15-agent-finetune.md`.
- With Training Mode on, one JSON line per affected hit goes to
  `<DRUMSPIKE_HOME>/training/<project id>.jsonl`; a record is self-contained (the samples
  around the hit, 40 ms before and 10 ms after) and a `move`, `add`, `delete` or `confirm`
  record carries `placement` `{edge, line_noise, line_slope, onset_spread}`:
  `crates/app/src/training.rs` (`BEFORE_MS`, `AFTER_MS`, the `placement` note).
- The judgment stops on a wobbly line: the line check wants a lead spread of 0.03 or less
  (shares of the local peak) over the first 12 samples, and a departure far from the hit
  (more than 12 samples before or 6 after it) gets no suggestion: `static/agent.js`
  (`LINE_LEAD`, `LINE_MAX_SPREAD`, `DEPART_BEFORE`, `DEPART_AFTER`, the `why` note).
- `plan(track, from, to)` looks over a range of hits without moving; its rows carry the judge's
  `doubt` and `why` (the reasons of the review queue, `track_confidence` with `training: true`);
  the agent works in chunks of about 100 hits: `docs/agent-api.md` (Page helpers),
  `docs/guide/15-agent-finetune.md`.
- `tools/training_export.py` reads the records and prints the truth list
  (`{track, role, from, to, op, precision, exact, edge, ...}`): the person's and the agent's
  corrections are the truth the placement is measured against.

## s01-hand

> The s top lane of the demo session at sample zoom (a 1.5 ms window) around one snare hit of the kit check: the flat line before the stroke, its last sample marked, the stroke's first motion after it. The status line says what the nudge did.

This is what you do by hand. Zoom to a hit until one sample is a pixel: a flat line before the stroke, then the stroke's first motion. The hit belongs on the last sample of that line, and the nudge moves it there by whole samples. Hits > Confirm Hit marks it reviewed.

## s02-agent

> The same hit, the same window. A second pointer, the agent's, does the same three steps, drawn as `DSAgent.look`, `DSAgent.move`, `DSAgent.confirm`. The status line is the same.

An agent does the same three steps through the app's own page. It looks at the hit, moves it by whole samples, and confirms it. Every move and confirm runs through the code your keyboard uses, so the record of an agent is byte for byte the record of a person.

## s03-look

> The lane picture on the left; on the right, the same window as numbers: the shares of the local peak as digits, eight per row, the hit's own sample marked as hit_at, the judge's level line and first departure marked.

What the agent sees is the same picture and the same samples as numbers: the window is 0.9 milliseconds before and 0.6 after the hit, the samples are shares of the local peak, and hit_at points at the hit's own sample. From these the judge reads the line's level and its spread, and where the stroke departs.

## s04-judge

> The same numbers for a wobbly hit: the lead spread over the first 12 samples is over the 0.03 limit; the word doubt and the reason stand next to the hit, and the hit does not move.

The judgment stops where the line is not a line. A wobbly lead, a spread over 0.03 of the local peak over the first 12 samples, gets no suggestion. The hit lands on the doubtful list with its reason, and stays for you.

## s05-training

> The training log line of the move as text: the op, the track, from and to (the recorded tutorial's own move: the k_in hit 180008 moved onto the last line sample 180006), precision sample, and the placement measures edge, line_noise, line_slope, onset_spread; the audio window is carried in the record.

With Training Mode on, every move and confirm writes one line to the project's training log: the op, the position, the precision, and the placement measures of the hit. The record carries the samples around it, 40 milliseconds before and 10 after, so it can be read without the session.

## s06-loop

> The loop as three boxes: the agent's moves and confirms, the log, and your corrections in the review queue feeding the same log; training_export prints the truth list the placement is measured against.

That log is the loop. The agent's records and your corrections in the review queue land in the same file, and the export turns them into the truth list the placement is measured against. The more you correct, the better the measures say what a right hit looks like.

## s07-result

> The counts of one real run, Session F (the coordinator's check of 0.9.26 on one internal session): 522 hits seen, 124 moved, 314 confirmed, 84 left for the owner.

The agent reports counts only: how many hits it saw, moved, confirmed and left for you. You stay the judge: every move is a normal edit you can undo, and the doubtful hits wait in the review queue.
