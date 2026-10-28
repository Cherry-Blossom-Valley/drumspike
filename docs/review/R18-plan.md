# R18: detection/re-pick orchestration as an explicit versioned pipeline — plan

step 1, plan only. no code changed. references are to the #40 tree
(a local worktree at 3cb834e = origin/main + #40 policy work) unless a file
differs from origin/main — only `policy.rs` does (it is #40, not on main); every other
file:line below is valid on origin/main (6f9268c).

## the problem

the pipeline of a detection is real but implicit. it exists twice, in two places, and the
two copies already disagree:

1. **the analysis job** (`jobs::run_job`, crates/app/src/jobs.rs:687): per track
   prepare → detect → weak-retry → candidates → learn → store (jobs.rs:899-1004), then
   after all tracks: cross_bleed → ghost_gate → tom_bleed → flam, hard-coded in that order
   at jobs.rs:743-801.
2. **the stored re-pick** (`histogram::track_repick_keep`, crates/app/src/histogram.rs:489):
   pick from the kept candidates → cut marks → learn → store, then only tom_bleed
   (histogram.rs:604-608) and flam (histogram.rs:609-611).

the disagreement: a re-pick store runs **no** ghost_gate and **no** cross_bleed step. an
analysis at the same params would clear a busy soft-gated ghost note (ghost_gate.rs:1-24)
or a cross-track bleed hit; the re-pick of the same track at the same params leaves it.
today that is survivable only because the rules refuse to run without a stored candidate
list / session context — it is an accident of guards, not a decision.

the versioning is per-rule and ad hoc. each rule has its own `analysis_*_rule`,
`recorded_*_rule` pair with **different missing-record fallbacks**:

| stage | new analysis | re-pick fallback when the record lacks the key |
|---|---|---|
| refine (jobs.rs:409-428) | `RefineRule::CURRENT` or pin | missing record = **version 1** (Reference) |
| tom bleed (tom_bleed.rs:65-80) | `BLEED_RULE` or pin | missing record = **0** (no rule) |
| flam (flam.rs:54-60) | `FLAM_RULE` or pin | missing record = **0** |
| cross_bleed (cross_bleed.rs:47-51) | `CROSS_BLEED` or pin | n/a (analysis only) |
| ghost_gate (ghost_gate.rs:37-38) | `GHOST_GATE` or pin | n/a (analysis only) |

each rule module also writes its own outcome back into the track's `detector` record
piecemeal: tom_bleed.rs:283-290 (`bleed_rule` + `bleed`), flam.rs:295 (`flam_rule`),
ghost_gate.rs:233, cross_bleed.rs. adding a rule means touching `run_job`,
`track_repick_keep`, `ServiceConfig` (lib.rs:90-110), and a new `recorded_*` fn — five
places for one stage.

the ticket's gate says it should be: one recorded recipe, one shared ordered runner, and
analysis vs cached re-pick agreeing under the same params/edits **by construction**.

## today's state, precisely

### the job

- `analyze_start` → `jobs::start` (jobs.rs:657) → `Jobs::queue` mints a generation
  `JobToken` (jobs.rs:64-80, 171-183); stale generations write nothing (jobs.rs:266-274).
- `run_job` (jobs.rs:687): two-stage overlap pipeline (P3-15b) — `prepare_one`
  (jobs.rs:858: decode+downmix on the prep pool) one track ahead of `finish_one`
  (jobs.rs:899, the job thread): rule pin (921), follows derive (922-934),
  `analyze_kept` DSP detection (936-940), plugin licence record (944-963), `weak_retry`
  (966-971), `candidates::write_candidates` (972-978), learn filter (980-1002), then
  `store_analysis` (1003).
- `store_analysis` (jobs.rs:1015): backup of previous events (1059-1061, first re-pick
  after an analysis only, `Jobs::needs_backup` jobs.rs:144-146), events + `detector`
  record into `tr.extra` (1063-1068), weak-signal thresholds into params (1071-1089),
  `mark_cut_events` (1055), follower refresh (1100).
- the cross-track stages, after every track is stored (jobs.rs:743-801), in order:
  1. `cross_bleed::run_step` (jobs.rs:747-759, #281 A6)
  2. `ghost_gate::run_step` (jobs.rs:763-775, #281 A1)
  3. `tom_bleed::run_step` (jobs.rs:776-788, P4-14)
  4. `flam::run_step` (jobs.rs:789-801, P4-13 stage B)
  the comment at jobs.rs:745-747 documents why the order matters: cross_bleed re-detects
  tom targets first, ghost_gate re-detects snares, "the flam rule and the snare rescue
  judge the hits it leaves", tom bleed "judges the hits it leaves".

### the re-pick

- `track_repick` (histogram.rs:347) → `track_repick_keep` (histogram.rs:489).
  `pick_inputs` (histogram.rs:361) parses params, pins the **recorded** refine rule
  (histogram.rs:381-385 via `jobs::recorded_rule`), loads the kept candidates from
  `Analysis/<track>/candidates.bin` (candidates.rs:27-29, 409 without them).
- the pick itself never re-runs detection: `analyze_track` with
  `Detector::Candidates` (histogram.rs:514-522). the gate scale is the slider's
  (histogram.rs:513, `sensitivity::scale_of`).
- the store path (histogram.rs:547-621): `begin_write` guard (549), backup only if
  `needs_backup` (550), params in first (551-555), fresh `detector_record` (557),
  recorded bleed + flam versions (559-567), learn pick (568-591), `store_analysis`
  (592-603), `tom_bleed::repick_targets` — the track plus every other tom with its own
  recorded version (tom_bleed.rs:350-364) — (605), flam for the one track (609),
  `repick_stored` (613).

### manual/edited/kept hits

- the one predicate: `edit::is_kept` (edit.rs:83) and `policy::is_kept`
  (policy.rs:39-43, #40; not on main yet) — a hand-added/moved hit or `edited: true`.
- `edit::merge_edited` (edit.rs:291, 20 ms `MERGE_WINDOW`): kept hits of the current
  events stay and win over picked ones, in `store_analysis` when `keep`
  (jobs.rs:1041-1049) and in the preview (histogram.rs:536-546).
- learn marks survive via `learn_filter::edited_times` (learn_filter.rs:329) feeding
  `stored_learn_events`, and `refilter_merge` (learn_filter.rs:477) keeps kept hits.
- the rules themselves guard: bleed marks skip edited hits (tom_bleed.rs:265-267,
  `!edited(e)`), flam likewise (learn_filter.rs:603).
- the backup discipline: `Jobs::repicked` set (jobs.rs:97) — the first re-pick store
  after an analysis backs up the pre-re-pick hits, later ones keep that backup
  (jobs.rs:142-157); the job resets it per track (jobs.rs:725).

### job kinds and what is versioned where

- `Kind::Dsp` only since #281 D16 (jobs.rs:430-435); the model path lives on a branch.
- versions live in the track's `detector` record (`tr.extra["detector"]`, written by
  `store_analysis` jobs.rs:1068 and mutated in place by the rule modules) and the
  track's `params` (thresholds/sensitivity/weak_signal, jobs.rs:1071-1089). nothing
  records "the pipeline version" as one thing; the doc carries the parts.

## target shape

a `pipeline` module (crates/app/src/pipeline.rs) owning three things:

1. **the stage table** — one ordered list of the stages, each a
   `{ name, version(cfg) -> u32, recorded(detector) -> u32, run(...) }`. the per-track
   stages (detect, weak-retry, candidates, learn, store) and the cross-track stages
   (cross_bleed, ghost_gate, tom_bleed, flam) in one place; `run_job` and
   `track_repick_keep` both iterate it. the order comment of jobs.rs:743-747 becomes the
   table's doc comment.
2. **one recipe record** — a `pipeline` object in the detector record (or beside it):
   `{ stages: {refine: v, bleed: v, flam: v, ghost_gate: v, cross_bleed: v}, ... }`,
   written once at store time. the per-rule `recorded_*_rule` fns collapse onto one
   resolution with the current per-key fallbacks kept and documented (refine missing = 1,
   the others missing = 0 — see the risk below).
3. **the re-pick as the same runner minus detection** — a stored re-pick runs every
   recorded stage the track qualifies for, driven by the record, so a re-pick at the
   stored params of a recorded recipe reproduces the analysis's stage sequence. the
   re-pick's extra session-wide rule (tom bleed over all toms, tom_bleed.rs:350-364)
   stays, expressed as the same stage with a wider target list.

this is a refactor with one deliberate behaviour change (PR 3 below), called out for the
coordinator.

## migration in small prs, each keeping main green

**pr 1 — extract the stage table, no behaviour change.**
new `pipeline` module with the cross-track stage list; `run_job` (jobs.rs:743-801) and
`track_repick_keep` (histogram.rs:604-611) call through it instead of hard-coding. the
module also owns the `analysis_*_rule` / `recorded_*_rule` re-exports so callers stop
importing four rule modules. output identical; the diff is mechanical.
tests: a table-order test (the stages run in the documented order — assert via a fake
runner recording stage names), plus the existing per-rule tests unchanged
(tom_bleed.rs:571, flam.rs:328, jobs/tests.rs:410, 447).

**pr 2 — one version-resolution helper.**
`pipeline::recorded_version(detector, key, missing)` replaces the four `recorded_*` match
bodies; each rule fn keeps its name and its documented fallback (refine 1, bleed 0,
flam 0). `ServiceConfig` pins unchanged.
tests: the old/missing-version matrix — a record from before P4-12/P4-14/P4-13 resolves
as today; a version the build does not know clamps to current (tom_bleed.rs:604,
flam.rs:344); the pin wins over the record (tom_bleed.rs:607). jobs/tests.rs:447
(`the_rule_of_an_analysis_and_of_a_repick`) extended to bleed/flam/ghost_gate.

**pr 3 — the re-pick runs the recorded stages (the behaviour change; needs the
coordinator's ok).**
`track_repick_keep` after the store runs ghost_gate and cross_bleed for the repicked
track when its recorded version > 0 and the guards hold (stored candidate list, DSP
detector, session context), same as the analysis job does. guarded behind nothing — but
the plan is to land it as its own PR so the page-visible delta is bisectable to one
commit.
tests: a recorded-recipe agreement test — analyse a session, re-pick one track at the
stored params with `store: true`, and compare the stage outcomes (`bleed`, `flam`,
`ghost_gate` entries and the event sets modulo the 20 ms merge) against the analysis's.
plus: manual hits added before the re-pick survive ghost_gate's re-detection
(`store_analysis` `keep`, jobs.rs:1041) — the same assertion shape the bleed rule's
`!edited(e)` test uses (tom_bleed.rs:265).

**pr 4 — record the recipe.**
`store_analysis` writes a `pipeline` object into the detector record with every stage
version that ran (and `0`/absent for stages that did not apply, e.g. a plugin detection
has no candidate list). readers prefer the per-stage keys as today; the object is
additive so old builds and old documents load unchanged (the `AnalyzeParams.roll`
precedent, core/src/analyze.rs:58-60).
tests: round-trip — analyse, save, reload, re-pick; the record's versions drive the
re-pick; a document without the key behaves exactly as before (the missing-version
matrix of pr 2 re-run against real project files).

**pr 5 — manual-hit preservation, end to end.**
mostly a test PR: for every stage, a kept hit (`edit::is_kept`) survives the stage and
carries no out mark (`policy::is_out`, #40). covers merge_edited in the job path and the
re-pick path, the repicked-backup rule (first re-pick after an analysis writes
`Backups/<track>.events.prev.json`, later ones don't — jobs.rs:142-157,
histogram.rs:550/612-614), and learn kept hits through `refilter_merge`.
tests: `edit/tests.rs` and `histogram/tests.rs` gains; the fixture truth table
(tests/fixtures/event-policy/table.json, #40) stays the source of truth.

prs 1-2 are pure refactor; 3 is the only user-visible change; 4-5 are additive. each PR
leaves `cargo test -p drumspike-app` and the e2e gate green, so any of them can be the
stopping point.

## risks

- **pr 3 changes re-pick output** — tracks whose analysis ran ghost_gate/cross_bleed
  will lose hits on the next re-pick that they would have kept today. that is the point
  (analysis and re-pick must agree) but it is page-visible; it needs the coordinator's
  explicit ok and a release-note line.
- **the fallback asymmetry is load-bearing.** refine missing = version 1 (the frozen
  engine's rule, jobs.rs:415-416) but bleed/flam missing = 0 (no rule). a unified
  `recorded_version` must not "simplify" this to one default, or old documents change
  behaviour. pr 2 keeps per-key defaults and pins them with tests first.
- **cross_bleed and ghost_gate re-detect**, they don't just mark: ghost_gate replaces a
  snare track's events from a fresh detection with exclusions (ghost_gate.rs:19-24). a
  re-pick that re-detects drifts from "no model run" (api.rs:319 promises exactly that)
  — but so does the analysis; the agreement is with the analysis, not with
  "candidates only". worth stating in the api doc string when pr 3 lands.
- **plugin detections have no candidate list** (jobs.rs:502-505, `None`), so a re-pick
  409s today ("no kept detection: re-analyze once", histogram.rs:388-393). the pipeline
  table must treat detect-inputs as a stage property, not assume candidates exist.
- **#40 policy work is in flight** and touches `is_kept`/`is_out` call sites
  (policy.rs is new there). pr 5 builds on it; sequence pr 5 after #40 lands or rebase
  the test surface onto the predicate module it provides.
- **the two-writer discipline** (`begin_write`, histogram.rs:746-752; the store guard,
  jobs.rs:101-140) must keep covering every stage the re-pick gains; ghost_gate's
  re-detect writes events through `store_analysis`, which is already inside the guard —
  keep it that way when extracting.
- generation/staleness: `run_job` checks `jobs.active` per track; the cross-track stages
  run after `overlap::run` without a token check on each `run_step` (jobs.rs:744-801).
  extracting them into the table is the moment to decide whether that is intentional
  (the project is open the whole time) and write it down, not change it silently.

## tests that prove each step (summary)

| pr | proof |
|---|---|
| 1 | stage-order test; existing rule tests untouched and green |
| 2 | old/missing/unknown/pinned version matrix per stage |
| 3 | recorded-recipe agreement (analyse vs stored re-pick at the same params, stage outcomes and events equal modulo the 20 ms merge); manual hits survive the gained stages |
| 4 | round-trip through save/reload; documents without the recipe key unchanged |
| 5 | kept-hit preservation per stage; backup-once-per-analysis discipline; learn kept hits through refilter |

the ticket's gate then reads as: pr 1+3 are the "shared ordered pipeline runner" and the
"recorded recipe" is pr 4; "analysis and cached re-pick agree under the same
params/edits" is pr 3's agreement test; "stage ordering, manual-hit preservation and
old/missing versions" are the pr 1/5/2 tests respectively.
