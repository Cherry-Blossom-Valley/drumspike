# M1b14: a Learn rule acts on the whole row

Ticket #587, umbrella #503 (the DAW plugin program), part of M1 (#510). Follow-up of #555 (one row per DAW track, `m1b8-song-time.md`).
This note decides the design first. The code follows in later commits of the same ticket. The standalone app does not change.

Owner's model: "Forget This" marks an unwanted pattern and every similar hit of the track goes. "Learn This" brings back missed hits
that are similar to the marked one. In a DAW one row is one DAW track and can hold clips of several audio files. The user sees one
track, so a rule made on one clip must act on every audio file of the row.

## 1. What the code does today (facts checked on the base commit 2d76b47)

- The rule set of a file is its `learn` object: `{on, threshold, auto, method, marks: [{id, kind, t0, t1, at}], add?, dropped?}`
  (`crates/app/src/learn.rs`, module text). An example is a TIME (`at`, source seconds of that file). No fingerprint is stored.
- The scorer makes the fingerprint from the audio each time: the feature vector of the candidate at the example time
  (`similarity::candidate_features`, cached per file in `Analysis/<track>/learn.bin`), or, for a time far from every candidate, the
  features of the decoded audio at that time (`learn_score::extra_of`). `score_all` centres every vector on the mean of the file's own
  candidates, then `similarity::score` takes the cosine to the examples.
- The pick (`learn_filter::learn_pick`, called from `histogram.rs` and by the Learn commands through `learn_refilter`) marks a hit
  `filtered` when its score is under the threshold. The example TIMES also act directly on hits of the same file: a "this" example
  within `REACH` (30 ms) keeps a hit, the hit nearest to a "not" example always goes (`mark_out`), and a Forget-only file runs an
  attack gate that reads the attack time of the "not" examples in the file's audio (`forget_attack_gate`).
- What Learn does to hits the user placed or edited today (on one file): an edited hit (added or moved, `edit::is_kept`) is never taken
  out by the score (`EDITED_REACH`). The one exception is the hit nearest to a "not" example, which goes even when edited, because
  the user marked exactly that hit. A hit the user deleted stays deleted (`dropped`). A filtered hit stays in the document, grey
  (`"filtered": true`), and comes back when the rule goes. Learn never removes a hit from the document.
- The page calls `track_learn_mark` on the file of the hit or range, adopts the answer (`lnApply`), pushes one undo step of type
  `learn` (`lnPush`: `learn` before and after, plus the hits before and after) and runs the guard (`lnGuard`: the answer carries
  `learn_out_before/after`, `hits`, `hits_before`; a step that takes out many more hits asks for a decision). Undo calls
  `track_learn_set` with the stored `learn` and `events`. `lnApplyOp` runs in the Learn queue and returns nothing.
- A group undo step exists (`pushGroup`, `groupApplyOp`, `edit.js`): the steps of the files of a row, all or nothing. One step is
  pushed as itself, so the standalone path is the old one.

## 2. Decision 1: where the rule set of a row lives

Options:

| Option | Where the marks live | Cost |
|---|---|---|
| A. One set per row | A new row object in the document. | The row has no identity in the document (m1b8: the row id is a counter of one plugin run, nothing persistent uses it). A document format change. Rejected. |
| B. A mark is copied to every file of the row, with its origin | Each file keeps its own `learn` object. A copy carries `from: "<track key of the file where the mark was made>"`. The id of the mark is the same in every copy. | One new optional key per mark. The standalone project never has it. Chosen. |
| C. The page sends the row at every pick | Nothing stored; the row is a call argument. | A re-analysis from the service (an agent call, a sensitivity change on one file) would lose the rule. Rejected. |

Choice B. The rule set of the row is the union of the marks of its files, each file holding what it needs to apply it. Every file
works on its own, with no DAW, with the rules it has: the standalone app can open the project and the filter gives the same answer.

A copy (an "adopted mark") measures its example on the audio of the file named in `from` (that file stays a track of the project).
The copy acts on the new file ONLY through the score. The time rules of section 1 (a "this" example keeps a hit, the nearest hit to a
"not" example goes) read example times of the file's own audio, so they apply to own marks only. A time of file A means nothing in
file B. This keeps the existing decision byte-identical and changes only which audio feeds it.

**When a file joins or leaves a row, and DAW undo.** A copy is made at the moment of a Learn action (decision 3), for the files the
row has then. Nothing runs when a clip is added, moved to another DAW track or deleted, and nothing runs when the DAW undoes that.
Reasons: a row change is not a user edit of the page and has no undo entry of the page; a silent re-filter of a file at a DAW undo
would change hits with nothing to undo. Consequences, stated as limits:

- A file that joins a row later does not have the earlier marks. The page tells it: the Learn section of the Inspector shows "marks
  of this row: N, of this file: M" and offers one button, "Apply row marks to this file" (one undo step, the same command as
  decision 3). No automatic catch-up. (Open question 1.)
- A file that leaves a row keeps its copies. They are data of that file. "Forget Examples of Track" removes them like own marks.
- Remove Example, change of kind (flip) and Forget This of a mark whose id exists in several files of the row go to every file of the
  row that holds that id, as one undo step. A file that left the row keeps its copy until the user acts on that file.
- The clip under the pointer decides the file of a new mark (as today). A mark made on a hit that lies in two overlapping clips of two
  files belongs to the file of the clip on top.

## 3. Decision 2: may examples of two files mix in one rule set

Yes, by choice, with the effect shown. Two files of one DAW track can be two microphones or two takes of one drum. The risk: a rule
made on file A scores the candidates of file B by A's pattern. If B is another microphone, bleed or tone differ, and the rule can take
out good hits of B (a Forget This) or add wrong hits (a Learn This). The rule is the same as on one file (the cosine to the
examples), so the risk is the same kind as a wrong example on one file today. The new part is that the user does not see file B when
marking on file A.

Protection, in the order the user meets it:

1. After the action the status line and the Learn section give the count PER FILE: "Forget This: file A 12 hits out, file B 9 hits
   out (of 410)". The page already has the numbers (`learn_out_before/after`, `hits`, `hits_before` in the answer of each file).
2. The guard (`lnGuard`) runs for each file, with the file name in the message. One refused file does not hide another. The choice is
   made for the whole step (decision 4): "Keep" or "Undo" (undo = the one group step).
3. One undo takes all files back (decision 4).
4. A per-file switch is not built. A user who wants file B to stay out turns Learn off on file B (the existing "Learn Filter" check
   of that file); the copies stay but the filter does not run there.

Not built: a count BEFORE the action. The rule does not exist before the mark, so a count before needs a dry run of the pick on each
file (a full pick of each file without storing it). That is the cost of one more Learn step per file and doubles the wait. It is
left out; the one-step undo is the safety (open question 2).

## 4. Decision 3: the service command

New command `track_learn_adopt {sid, name, from, ids?}`:

- `name`: the file that receives the copies (the file of another clip of the row). `from`: the file where the marks were made.
- `ids`: the ids of the marks to copy. Without `ids`: every mark of `from` that `name` does not hold yet (this is the "Apply row marks
  to this file" button).
- It copies the marks with `from` set, checks the cap `MAX_EXAMPLES` per kind (the same 400 as `track_learn_mark`), then re-picks
  `name` with the existing `learn_refilter`. The answer has the shape of `track_learn_mark` (`learn`, `filter`, the guard counts,
  `track`), plus `adopted: n`.
- A mark that exists in `name` with the same id is skipped (idempotent). A copy of a copy is not made: `from` always names the file
  where the mark was made.
- Remove, flip and Forget at a mark: no new command. The page calls the existing `track_learn_remove`, `track_learn_flip` and
  `track_learn_forget_at` for each file of the row that holds the id. The existing remove and flip find marks by id already.

Measurement of the examples: the scorer takes the feature vector of an adopted example on the audio of `from` (the raw vector of
`candidate_features(A, t)`, from the `learn.bin` cache of `from` when the time is near a candidate, else computed from its decode,
as `extra_of` does for a far time). The vector is then prepared on file B exactly as B's own examples are (centred on B's mean, then
weighed). Rate: the features read `sr` of their own file (`Extractor::new(sr, ..)`), so files of two rates are comparable; a test
with 44.1 and 48 kHz proves it. For the attack gate of a Forget-only file the attack time of an adopted "not" example is measured on
the audio of `from` (`attack_ms`) and joins the span of the marks.

Changes in `crates/` (small, additive; the old forms stay as thin callers, no signature changes):

| File | Change | Lines (rough) |
|---|---|---|
| `crates/app/src/learn_score.rs` | `Foreign {this, not, not_attack_ms}` (vectors prepared by the caller), `score_all_with(.., &Foreign)`; `score_all` calls it with an empty `Foreign`. `example_times` skips marks that have `from`. The empty `Foreign` takes the old code path. | 50 |
| `crates/app/src/learn_filter.rs` | `learn_pick` takes the `Foreign` as one more argument; the Forget-only condition and the attack gate count the foreign "not" too. | 30 |
| `crates/app/src/histogram.rs`, `reanalyze.rs` | Resolve the foreign vectors of the track before the pick (one helper in `learn.rs`); `reanalyze` skips adopted times where it reads own example times. | 30 |
| `crates/app/src/learn.rs` | `track_learn_adopt`, `LEARN_COMMANDS` (7 to 8), the helper that measures an adopted mark on `from`, `check_learn` accepts `from` (a string that names a track). | 120 |
| `crates/app/src/call.rs`, `docs/agent-api.md`, shell command lists | Route and text of the new command; `cargo test -p drumspike-app` (registry test). | 20 |
| Tests (`crates/app/src/learn/tests.rs`, `crates/app/tests/learn.rs`) | New tests only. | 150 |

Total: 5 to 6 files in `crates/`, about 250 lines of code and about 150 lines of tests. No change in `crates/core`.

Where it runs. Never on the audio thread. The command is a service call like `track_learn_mark`: it runs on the bridge thread of the
service, behind the page's Learn queue (`lnQueue`), and it takes the project lock only for the copy, as `learn_edit` does. The decode
and the features run outside the lock (as in `learn_ctx`). Nothing new blocks the main thread of the host or the page.

Cost for a dense track of 10 minutes (estimate from the code, to be measured in phase 2 on a synthetic file, with the number in a
report file): the extra work over one `track_learn_mark` on file B is the features of at most 64 + 64 examples of `from` (the cache
of `from` covers most; a computed one is a few ms each) and the decode of `from` (cached by `decode_cached` after the first use). The
scoring of B is the cosine of every candidate to the examples: the example count goes from M to M + M', with M and M' under 128, so
the scoring grows by the same factor. The pick of B is the same pick as today (it dominates). So an adopt costs about one Learn step
on B, and a Learn on a row of N files costs N Learn steps one after the other. The page waits on the queue as today.

Proof that the standalone app is unchanged:

1. A standalone project has no mark with `from`. `Foreign` is empty then, and the code path of `score_all` is the old one.
2. No expected value of an existing test changes: `git diff origin/main -- tests/ crates/*/tests crates/*/src/**/tests.rs` shows only
   added tests (checked by the diff in the report).
3. A byte check on a fixture: the export (MIDI, spikes) of a synthetic project with Learn marks, before and after the change, is
   byte-identical (`tools/identity.py snapshot` on the base commit, `compare` on the head).
4. A fault run `foreign-leak` takes the examples of another file into the score of a file with no `from` mark. A test that fails
   with the fault proves that the empty `Foreign` is empty.

## 5. Decision 4: undo is one step

The page makes the action in a row with N files as follows:

1. `track_learn_mark` on the file of the clip (as today, one `learn` step).
2. For every other file of the row (`HT.srcsOf`): `track_learn_adopt` with the id of the new mark, one `learn` step each.
3. All steps go to `pushGroup("Learn This" / "Forget This", steps)`. With one file `pushGroup` pushes the plain step: the standalone
   path and a row with one file are the old code (`HT.on()` guard; no change of an expected value).

Two points of the code have to change for this and are part of the ticket:

- `lnApplyOp` (undo and redo of a `learn` step) runs inside `lnQueue` and returns nothing, so `groupApplyOp` takes it for done at
  once and cannot take a later step back. It returns the promise of its queue entry (`true` when the service accepted, `false` when
  not). For the standalone single step `undoStep` already waits on a promise result; the stack moves after the service answered,
  which is what the other step types do. This is behind no flag but changes no result: the old code ignored a refused Learn undo
  (it moved the stack anyway). The fix makes a refused undo leave the step where it was. A test shows it.
- Redo of the group runs the steps in order: the mark first, then the copies (each copy needs the mark of `from` to exist in
  `learn`; a copy is stored with its `from` and its times, so the order is only for the guard counts).

The guard of decision 2 runs once per file after the step; "Undo" in its message is the one group undo.

## 6. Decision 5: the three small points

1. **A nudge into the next clip of the same file.** `HT.clampTo(src, from, to)` (`hosttime.js:119`) finds the FIRST clip of the source
   that holds `from` and keeps `to` inside that clip. Read from the code, this stops a nudge at the edge of one clip. The ticket says a
   nudge can still reach the next clip when two clips of one file touch; the mechanism is not proved yet. Candidates: the hit lies in
   two clips of the same file (overlap or a touching edge, `from >= c.a && from < c.b` holds for both), the first one in the list is
   not the one the user sees (the later clip is drawn on top, m1b8 section 3), so the hit is clamped to the wrong clip and leaves the
   visible one. Phase 2 starts with a page test that reproduces it (two touching clips of one file, a nudge across the join) and then
   fixes the cause. Decision now: the clamp uses the clip that is drawn on top at the hit (the rule of m1b8), a nudge stops at its
   edge, and the status line says "the hit is at the edge of its clip". If the test shows another cause, the note is amended in the
   same ticket before the fix. Standalone twin: no clips, the nudge is unchanged (`HT.on()` guard stays the first line).
2. **A failed roll-back step of a group undo is reported.** In `groupApplyOp` the roll-back loop runs `await run(d, !reverse)` and
   drops the result. A step that cannot be taken back leaves the project in a mixed state with a message that says "the steps before
   it were taken back". Decision: the loop counts the failures; the message names each step that did not go back ("a step of X was
   refused; Y could not be taken back, check it") and the status shows it as a warning. The stacks stay as the service is, as today.
   Test: a fake apply that refuses step 2 and step 1's roll-back; the message names step 1.
3. **The refine-window setting follows the row.** `paramsFieldCommit` (`histograms.js`) writes `refine_lo_ms` / `refine_hi_ms` for one
   file. Role, note, sensitivity, loudness and shift already go to every file of the row as one group step (`m1b8` section 10). The
   refine window does the same: `HT.srcsOf(name)`, one `track_params_set` per file, one `pushGroup`. Without host mode `srcsOf`
   gives `[name]` and `pushGroup` gives the plain step: the old code. Test in host mode with two files; standalone twin.

## 7. Decision 6: the Gate mapped to tests

| Gate line | Test |
|---|---|
| Learn This, host mode, two audio files on one row | `tests/e2e_app/test_host_learn_row.py`: both files hold the mark (one own, one with `from`); hits of file B that look like the example are in; the count per file is in the status line |
| Forget This, same | Hits of both files similar to the marked one go (filtered); a hit of B unlike it stays; an edited hit of B stays (decision 2 limit of section 1) |
| Undo as one step | One undo takes back the mark and the copy; the hits of both files are as before (byte-equal list); redo does both; a refused step of the group leaves the stacks as the service is |
| Standalone twin of each | The same actions in a plain page with two tracks: only the own file changes, one undo step per file, the call list has no `track_learn_adopt` |
| Dense kick or snare track, no loss | `crates/app/tests/learn.rs`: a synthetic dense kick track (about 10 minutes, generated in the test with a fixed seed; never an owner session) in two files of two rates. Count of output hits before and after a Learn This of a kick example: no hit lost on file B that was not the target of the rule. A Forget This of a planted false pattern takes out exactly the planted hits. The counts go in a report file |
| Service tests | `track_learn_adopt`: idempotent, the cap, a gone `from`, a read-only session (409), the shape of the answer; the registry test of `cargo test -p drumspike-app` |
| Standalone proof | Section 4 proof 1 to 4 |
| Complete app gate | Once before hand-back, scoped like CI: G1 (fmt, clippy `--all-targets`, `cargo test --workspace`), `cargo test -p drumspike-app` (new command), the `e2e_app` tests (page change), the JS coverage of `learn.js` and `hosttime.js` (70 %), `tools/build_help.py --check` if a guide page changes, the docs wording test, `tools/check_private.py --diff origin/main` |

The ENST gate and the real-session check are not run: no detector code and no similarity decision changes (the rule of the ticket).

## 8. What this ticket does NOT do

- It does not change what "similar" means: no threshold, no weight, no recipe, no rule version (`similarity`, `learn_filter` decisions
  stay). It changes only which audio feeds the existing score.
- No automatic catch-up of a file that joins a row, no re-filter at a DAW undo, no per-file switch, no count before the action.
- No row object in the document, no format change (one optional key per mark), no change in `crates/core`.
- No detection change, no export change (the export per DAW track is #576).
- No change of the Learn histogram of the Inspector (it shows the file of the selected hit; the per-file counts are in the status
  line and in the guard).
- Nothing outside host mode in the page; the standalone app behaves as today.

## 9. Risks

1. Mixed microphones (section 3). Stated, shown per file after the action, one undo.
2. A project saved with adopted marks and opened without the file named in `from` (the file was removed from the project): the copy
   has no audio to measure. The scorer leaves that example out (like `extra` returning `None` today) and the Learn section lists it as
   "example of a file that is gone". Test: remove `from`, the pick runs with the own marks.
3. A copy whose `from` file is re-analysed or its audio changed (a new take with the same name): the example time points to other
   audio. The existing `learn` has the same property for own marks; the rule for both stays "the user removes the example".
4. The cap of 64 examples per kind: an adopted mark counts against the cap of the file that receives it. A full file refuses the copy
   with the 400 of the mark command, the group step stops, and the page undoes the steps before it (the group rule).

## 10. Open questions for the coordinator (each with my pick)

1. A file joins a row later: copy the marks automatically, or only on the user's button? Pick: only on the button (no silent change,
   nothing to undo).
2. Count before the action (dry run), or the count after with one undo? Pick: after, with one undo (a dry run doubles the wait).
3. Copy the mark to the other files at the time of the action (this note), or one rule set per row that the files read live (a row
   object in the document)? Pick: copy at the time of the action (no document format change, the standalone app opens the project).
4. A hit nearest to an adopted "not" example in file B: does it go even when edited, as for an own "not" example? Pick: no. An
   adopted example acts through the score only, and the score never takes out an edited hit (section 1).
5. The nudge at the edge of a clip: stop at the edge (this note), or jump into the next clip of the same file? Pick: stop.
