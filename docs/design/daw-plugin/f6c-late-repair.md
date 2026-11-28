# F6c (#605): a late try of the repair, a steadier fault run `editor-refuse-fault-late`

Status: fixed in the plugin. The standalone page, the app, `crates/` and `static/` are not changed.
Follow-up of #586 and #583 (see `f6b-repair-busy.md`).

## 1. The late try of the repair

### Problem

After the busy limit (100 tries of 100 ms, about 10 s) the repair of a restored track wrote its one row and then waited for
a later read of the audio or a press of Retry. A page job that ended later did not bring the track back. The model is
Melodyne: an offline track comes back by itself as soon as it can.

### The real triggers (read in the code)

The service gives the plugin no event for the end of a page job (the events `project-dirty` and `track-layer` are not
that). The plugin sees two real events of its own worker:

1. **A round of the document ended.** In the loop of the worker (`lib.rs`, `Session::run`), right after the round and
   `plan_retry`: `self.late_repair(&doc)`.
2. **The wait for a job of the page ended.** A round of the document that found `doc.page.job_running()` is put back into the
   queue; the document id goes into `WorkerState::job_waited`. The turn that finds no job any more takes the id out
   (`job_waited.remove`) and calls `late_repair` before the round runs. A round that waits for another reason (a 409 of
   the round itself, `run_job` returns true) is NOT this trigger: it would try every 100 ms.

The removal of a track (`Msg::Remove`) is no trigger of its own: a removal ends in a round, which is trigger 1.

### Rule

- The give-up on a busy service (`RepairPlan::GiveUp("busy")`) sets `repair_late` on the source. A later read that finds
  the service busy again sets it too. Nothing else sets it (the wait state is not covered: no audio file or no track yet
  is not a busy service; the next read or round handles it).
- `late_repair(doc)` takes, for each live source of the document with the mark `repair`, the flag `repair_late` (swap to
  false) and makes ONE try (`repair_restored`: one call of the service, `repoint_once`, no sleep). Source with no mark or
  no owed try: no call of the service (an online track costs nothing).
- Busy again: `repair_late` is set again and the source waits for the next trigger. At most ONE more row per restore
  (`repair_late_row`): `track_repair_failed`, `why` "busy", `late` true. A "not ok yet" answer keeps the try owed and
  writes no row (`repair_tries` is raised to 1 so the wait row of the first try is not written again).
- Done: the row `track_repair` (as before) and the row `track_repair_late`; the page is told by `announce_reload` and the
  states by `states_changed`. A new restore resets all flags.
- No timer, no loop, no poll. The try runs on the worker thread; the service lock is held for one call, as for any round.
  Nothing on the audio thread. The main thread is not touched.
- Fault run: `DS_ARA_RESTORE_FAULT=no-late-try` (no late try at all). The unit test fails with "no late try after the
  trigger".

Known cost: `Repair::Wait` in `late_repair` keeps the try owed with no limit, so a track that never comes costs one `tracks_get` per round.

### Limit that stays

A page job with no work of the plugin behind it (the user starts a job in the editor and nothing in the DAW changes)
gives no event. The track then waits for the next round, a read or a press of Retry. This is the price of "no poll": the
service has no job-end event that the plugin can listen to without a change under `crates/`.

### Proof

Unit test `a_repair_that_gave_up_on_a_busy_service_makes_one_late_try_per_trigger_and_never_loops` (a real service with a
restored `changed` track, the busy seam of the service answers 409): no late try before the limit; one row at the limit;
one try per trigger; at most one more row; no call without a trigger; the free service repairs the track; an online track
makes no call. The wiring of the two triggers in the loop of the worker is two lines and is not driven by a stage (no
stage can make a real page job at the moment of a restore).

## 2. `editor-refuse-fault-late`: the blind condition is an event

The fault run blinded the trigger of the late file for 1.5 s (`--trigger-blind-ms 1500`) and let the end of the run
finish at the report (`--end-on-report`). On a loaded machine the 1.5 s could pass before the report landed: the trigger
then saw the report first, added the late file, and the run failed for another reason than its name.

Now `--trigger-blind` makes the trigger blind **until the end has seen the report** (`end_saw_report`, set in the turn
that the end of the run sees the page's report). The wait is false before its action. The fault run (`--end-on-report`)
ends at that same sight, so the late file is never added, on any machine speed. The stage `editor-refuse-late-trigger`
(the same blind trigger, the fixed end) sees the report at the end first, then the trigger fires in the next turn, and
the run waits for the round of the late file as before. The stage `editor-refuse` is unchanged.

## 3. Edits of a track that is removed before its read are lost by design

A restored track whose source is destroyed before its audio was read (the host removes the track at once) is removed from
the project together with its record. Its hand edits live in that record and in the events of the track; they have no
other place to stay, because the source it belonged to is gone and a source that comes back is a new source with a new
analysis. Keeping them would hold edits of a track that the user has removed in the DAW. This is the rule of the removal
(`doc-readd-removed`), not a defect of the repair.

## 4. Can the restore call after a successful remove answer "busy"?

No. `tracks_restore` (`crates/app/src/tracks.rs`, `Service::tracks_restore`) takes no job check: it needs a writable
project (`get_writable`, a 409 only for a read-only project) and a free track name (a 409 when the name exists). Neither
is a busy state that ends by itself, and the repair already takes a 409 of the restore as a permanent error: it puts the
old record back and says so (`repoint_with`, the text starts with "the new record", not with `409`, so it is `Repair::Stop`,
never `Busy`). The call that CAN be busy after the restore is `project_save`; it is covered since #586 (`save_owed`,
`reload_owed`), and a try that is busy there is a `Busy` result like any other, so the late try covers it too (the
record is pointed in memory and the save is owed: the unit test `a_save_that_is_busy_after_the_repair_is_saved_and_announced_by_the_next_try` of #586).

Reading chosen where the ticket is open: the late try covers the busy state only (not the wait state), because the wait
state has its own limit and a round brings the record by itself.

## Proof of the steadier fault run

Under load (10 busy loops started by the check and stopped by PID): 20 of 20 runs of `editor-refuse` and 20 of 20 runs of
`editor-refuse-fault` (which holds `editor-refuse-fault-late`) behaved as designed; every fault run failed for the later round
alone ("1 rounds"), every normal run saw 2 rounds.
