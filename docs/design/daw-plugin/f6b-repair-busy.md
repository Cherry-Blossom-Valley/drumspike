# F6b (#586, #583): the repair of a restored track on a busy service, two stages, and the stage `editor-refuse`

Status: fixed in the plugin. The standalone page, the app and `static/` are not changed.

## #586: the repair of a restored track (follow-up of F6, #582)

### What the second read found

The repair of a restored track (`repair_restored`, after the read of its audio) called the service through the retry
helper `retry_busy`. A busy service (a job of the page, 409) made the helper sleep up to 1 s with the service lock held,
on the worker thread. The worker then planned the repair again every 100 ms, up to 6000 times, and each try could
sleep again. One counter served the wait for the restore and the busy case, so many busy tries could end the wait at
once.

### Fix rule

1. The repair makes ONE try per call of the service (`DocService::repoint_once`, `call_busy(false, ..)`). A 409 comes
   back at once as the error text that starts with `409`, and nothing sleeps with the lock held. The round keeps its
   own use of the retry helper (`repoint`, called by `analyse`): there the worker defers the whole round on a 409, and
   its bounded retry (at most 10 sleeps of 100 ms) is unchanged.
2. The deferral does the waiting: `repair_and_retry` plans the next try after 100 ms (`defer_msg`). The busy state is
   limited to 100 tries (about 10 s). The wait state (no audio file yet, no track in the project yet) is limited to 100
   tries (about 10 s) as before. The two states have their own counters (`repair_busy`, `repair_tries`); the decision is
   the pure function `repair_plan`.
3. When a limit is reached, ONE row is written: `track_repair_failed` with `error` "gave up" and `why` "busy" or
   "wait". The count then stays above the limit, so later reads write no second row.
4. After the limit: the mark `repair` stays. No timer plans more tries. The next read of the audio (the host turns the
   samples access off and on, or sends an update of the content) makes one try, and a try that works clears the mark.
   Until then the track shows the "offline (changed)" text of the app. A new restore (a new session open) starts both
   counts again.
5. A busy save after a record was pointed: the pointed record is in memory and the save is still owed (`save_owed`,
   as before). New: the page reload that tells the page about the pointed record is owed too (`reload_owed`). The next
   call that ends well saves, and reports the track as pointed (`repointed`), so the page reads the project again. Before,
   that call found the track `ok` and said nothing, and the page kept the offline track. (Reading chosen where the
   ticket is open: the safer one is that the page is told.)

### Limit that stays (the statement the ticket asks for)

Two different files of EQUAL length are not told apart when the archive has no content hash (an archive of an earlier
plugin). With no hash the sample count decides: an equal count is the same audio (the rule of the restore, R4), so the
stored hits stay on a file that is another file of the same length. A hash in the archive (every archive that this plugin
stores) decides exactly. The user can run Re-analyze on the track. This is the price of not analysing every restored track
again; the other choice (analyse again when there is no hash) would change the hits of every old project at its first open.

### Stages

| Stage | Proves | Fault run |
|---|---|---|
| `doc-readd-failed-read` | the first read of a restored track fails, the host reads again: the repair row comes after the failed read and after the good read, the record is online on disk, the hits are as stored, no round | `doc-readd-failed-read-fault` (`DS_ARA_RESTORE_FAULT=fail-clears-mark`: a failed read gives up the repair; no repair row, the record stays offline) |
| `doc-readd-removed` | a restored track is removed (its source destroyed before any read) and brought back (the same file added again) in one document: the project holds the archive's tracks once each, every one online on disk, a round analysed the returned track, no failure row | `doc-readd-removed-fault` (`keep-removed`: the removed track stays, the returned one is `..._2`, the check refuses the track list) |

Host modes: `restore --fail-reads <k>:<n> --reread <k>` and `restore-readd --readd <k>` (`src/bin/ara-host.rs`).

Unit tests: `the_repair_makes_one_try_on_a_busy_service_and_never_sleeps` (a seam in `service.rs` makes the service answer
409: one call, no sleep for the repair; 11 calls and 10 sleeps through the retry helper, so the round keeps its bound),
`a_save_that_is_busy_after_the_repair_is_saved_and_announced_by_the_next_try`,
`the_repair_counts_the_wait_and_the_busy_state_apart_with_a_limit_and_one_row`.

Not proved: the test host and the unit tests make the service busy by a seam, not by a real job of the page; a real job
of the page that starts between two calls of the repair is not driven by a stage.

## #583: the stage `editor-refuse` under load

### Cause (read in the report of a failing run)

A failing run had no late file at all: no `request` row for it. The host mode `refuse` adds a late file when the page's
`refuse` report is seen (the trigger) and ends when the same report is seen (the end). Both read the report file, once
each in a turn of the host loop, and the report can land in the `pump` between the two reads. The run then ended at the
report, before the late file was added, and the check found one round. It is not a fixed wait; it is a trigger and an end
that look at one event at two times.

### Fix rule

- The end of the mode `refuse` with late files needs the late file to be added (`late_done`, false until the action) and
  then the report. The host waits for the round of the late files by a job row that lists every source of the document
  (the first round has fewer tracks, so a first-round row that is written late cannot satisfy it); the wait is bounded at
  20 s and fails with a text that starts with "time limit".
- Proof that the stage can see the defect: the host option `--trigger-blind-ms 1500` (it is `--trigger-blind` since #605, see `f6c-late-repair.md`) makes the trigger blind for 1.5 s while
  the end can see the report (the order of events that a loaded machine gives now and then, made certain). With the fix the
  stage passes in this order (`editor-refuse-late-trigger`). The fault run `editor-refuse-fault-late` adds `--end-on-report`
  (the old end): it must fail on the later round alone, with all 14 commands answered 403. The fault run
  `editor-refuse-fault` (the page may change the track set) is unchanged and fails for its own reason.

### The other stages of #570

- `doc-restore-busy`: the host waited a fixed 1.2 s for the other document's round to start. It now waits for the
  `round_start` row (false before the request, bounded at 30 s). The pass condition of the check is the same.
- `no-request-update`: left. Its failure (a host notification that came late) is another wait, in the host's check of the
  model update; the pattern of this fix (one event seen at two times) is not what it shows, and a change needs its own look.
