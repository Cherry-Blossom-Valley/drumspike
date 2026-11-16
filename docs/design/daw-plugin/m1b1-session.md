# M1b1: one service and one project for each ARA document

Ticket #537, umbrella #503 (the DAW plugin program), part of M1 (#510). Follows M1a (#525)
and is the base of the editor window (M1b2). The plugin lives in `plugins/ara/`.

## What changed

Before: the plugin made a new Drum Spike service for every analysis job and dropped it. It
wrote one fixed project folder, and a later job wrote the document from only its own
sources, so the earlier tracks left the project. A destroyed source stayed in the plugin's
list until the controller ended. Two documents in one process shared the folder, the source
numbering and the file names.

Now: one `drumspike_app::Service` and one project folder for each ARA document controller,
for the life of the controller. Every analysed source stays a track. A destroyed source leaves
the list and the project. All tracks of a document are analysed together (the rule below). A
restore opens the project with the stored events and decodes nothing. Nothing in `crates/`
changed.

Review round 2 (the coordinator, after an independent read of the diff) found ten defects
and four smaller points. Round 3 fixed the ten and left some of the four open. The section
"Review round 2" lists each finding, the fix and the stage that proves it.

## The rule: all tracks together

Whenever the set of analysed tracks of a document changes (a request adds sources, a source
is destroyed, or the regions or the properties of a source change), the result of ALL tracks
is byte for byte the result of one analysis of all those tracks together, as the service
reference gives it. Every round takes a fresh snapshot of every source.

Why: the session steps of the service (cross bleed, ghost gate, tom bleed, flam) run over the
tracks of one job only. A DAW requests the analysis of sources one by one, as the user sets the
plugin on each track. If a round analysed only the new tracks, no track would ever see the
other microphones, and the plugin's result would not be the app's result. The first version of
this work had that fault (and a gate line that asked for it); the fault switch
`DS_ARA_ROUND_FAULT=new-only` brings it back so the check can refuse it.

Note on the ticket: gate line 2 of the ticket asks for the events of the first two tracks to
stay byte for byte after the second request. That is not the rule. The rule is the one above;
the stage `doc-tracks` proves it.

How:

- Each round runs `analyze_start` over every track whose audio is here. The decode is cached by
  the service. A track whose audio is not here is not analysed; it keeps its stored events and
  the round is marked partial (see "Results of a round").
- Requests that arrive close together are one round: the worker waits until no message came
  for 250 ms (at most 2 s in all). Eight tracks added one after the other are one analysis,
  not eight. The report names the number of rounds.
- After `destroyAudioSource` the remaining tracks are analysed together again, and their result
  equals the reference of the remaining set.
- A change of the playback regions or of the properties of a source that took part in the
  analysis marks the document as changed and starts a round (through the same settle time).
  A change that alters nothing ends in a round that does nothing.
- The notes of an earlier source can change in a later round (a hit that the app's own
  `policy::is_out` calls out of the output: cut, bleed, filtered, or deactivated unless kept,
  is no note). The plugin tells the host: the sources are collected, and in
  `notifyModelUpdates` (the only call in which ARA lets the plugin call back) the plugin calls
  the host's `notifyAudioSourceContentChanged` for exactly those sources. The test host
  records every notification. A source whose notes did not change gets none; a new source gets
  none (the host polls for its completion, as in M1a). A source that had no result and gets
  one in a later round is told as well.

## The design

- **Service life.** The service is made in `createDocumentControllerWithDocument` and
  dropped when the controller is destroyed. The report names the services made and dropped.
  The service home is set inside the cache folder, so the service never writes below the
  user's home.
- **Folder per document.** `<cache>/documents/<id>/` holds `audio/` (the cached WAV of each
  source, numbered per document), `project/` (the Drum Spike project) and `service/` (the
  home of the service). Two documents never share any of it.
- **Document id.** The plugin makes the id once (64 random bits as hex: exactly 16 lowercase
  hex digits) and stores it in the ARA archive (`doc_id`). The archive also stores the track
  name of each source (`track`). Both fields are optional on reading, so an archive of M1a
  loads and the committed fixture `fixtures/archive-v1.ara` still loads. A reader of M1a
  ignores the new fields. The archive version stays 1: no bump was needed. An archive without
  an id restores into a fresh id.
- **The archive is input from a file the user can edit.** The plugin takes the stored id only
  when it has the exact generated form; an absolute path, `..`, an empty string or any other
  value gets the fresh id the document has, and the report has a `doc_id_refused` row. A
  stored track name with a path separator, `..` or a control character is replaced by the name
  derived from the source name; events keyed by such a name are dropped; both give a
  `restore_refused` row. `folder_of` maps an id that is not valid to a fixed name, so an
  invalid id never reaches the file system.
- **Registry of the ids.** A process-wide list holds the ids of the live documents. A restore
  that finds its stored id in the list (a session file that was duplicated, or one that is
  open twice) does not take it: the document keeps its fresh id, the report has a
  `doc_id_taken` row, and its folder starts from the archive data, not from the files of the
  other document. The id leaves the list when the controller is destroyed.
- **Tracks are added.** The sources are added to the open project with `tracks_restore` (one
  track record each, with the segments of the host's playback regions), then the round runs.
  A source is one track for its life, keyed by its creation index: a round that failed leaves
  the tracks, and a retry makes no second track. A new track takes its place by the creation
  index of its source, so the order of the project and of the events is the order of the
  sources, however the requests came.
- **Tracks are removed.** `destroyAudioSource` takes the source out of the controller's list
  at once. The worker removes the track with `tracks_remove`, saves, deletes the cached WAV of
  the source, and starts a round over the tracks that are left. A numbering counter per
  document makes sure an index is never reused.
- **Restore.** `restoreObjectsFromArchive` sets the notes from the blob, takes the stored id
  (see above) and sets the events of the document on the calling thread. The project part
  goes to the worker: it writes the project document from the stored events and opens it. The
  worker is shared by all documents and may be busy for minutes, so an archive that the host
  stores right after the restore (an autosave) must not depend on the worker. The decode
  counter of the service is part of the report and must be 0.
- **What a restored track points to.** The source record of a restored track (and the segments
  of a segmented track) points at the cache path `<cache>/documents/<id>/audio/<n>-<stem>.wav`
  where the reader would write the audio. In a new process that file does not exist until the
  host enables the samples access again. Before that: `project_open` and `events_get` work (the
  stored events are in the document), the track is offline, and a call that needs the audio
  answers the app's normal offline error, not a panic. The crate test
  `a_restored_track_without_audio_answers_the_offline_error_not_a_panic` checks this. A round
  skips such a track, names it in `skipped` and marks the round partial.
- **Results of a round.** The service gives a result for each track. The plugin reads it: a
  track with an error (for example a WAV the service cannot read) is named in `errors` and its
  source ends without a new result: complete for the host (the host must not wait for ever, and
  the ARA protocol has no error channel). A new request for that source runs a round again. The
  rule for its events: a track that never had events has none and no notes. A track that had
  events from an earlier round KEEPS them (in the document, in the archive and as notes) and the
  `job` row names it in `stale`: they are never presented as the result of this round. A WAV in the document folder is used only when
  this process wrote it and its size is the size of the samples the reader wrote; a leftover
  file of an earlier session is not trusted. The `job` row lists the tracks that took part
  (`analysed`), the tracks left out (`skipped`, with the reason), the errors, and `partial`
  (true when a track was left out, had an error, or a source was late).
- **Late audio.** A request waits for the audio of its source at most 30 s (counted from the
  request, not from the end of the settle time; the scenario `audio-deadline-ms:<n>` changes it
  for a stage). After that the round goes on without the source: a `source_late` row, the name
  in `late` in the `job` row, `partial`. The source ends without a result and a later request
  starts a round again. A reader that cannot be made also sends the failure message now.
- **A failed or cancelled round.** The tracks stay. The sources of the round get no result,
  are complete for the host, and the next request for them starts a round again (the
  `job_failed` row names the error). `isAudioSourceContentAnalysisIncomplete` is false for a
  source with a result and for one whose analysis ended without one; it is never stuck.
- **A request for a source that is complete** changes nothing and starts no round.
- **Regions.** Only the modification range of a region (`startInModificationTime`, duration)
  is compared. A move in playback time alone changes nothing the analysis uses: no round, the
  notes stay. A change of a source that has no audio here is not lost: the old snapshot stays
  until a round with audio sees the change.
- **Host strings and paths.** No string from the host becomes part of a path unchecked. The
  name of the cached WAV is `<number>-<stem>.wav` where `stem_of` gives one safe component for
  every input (a name that ends in `..`, an absolute path, an empty name, control characters);
  the track name goes through the app's `safe_name`. The persistent id of a source is only
  compared.
- **Limits of the design.** (a) The registry of the ids holds for one process. Two processes
  that open the same session file share `documents/<id>`, with no lock; a host must not do
  that. (b) The shipped build reads the `DS_ARA_*` test switches from the environment. Two of
  them weaken safety: `DS_ARA_ID_FAULT=trust` and `DS_ARA_HOME_FAULT=1`. Compiling them out of
  a delivery build (a cargo feature) is an open point for M1c. (c) The timing limits of the
  close stages and the values measured are in the result table below.
- **Close.** `destroyDocumentController` stops the readers, cancels a running analysis, sends
  a close message and waits for the worker for the rest of 500 ms (counted from the start of the call), then returns. What the plugin does not bound: the join of a reader lasts as long as the host's read call inside it. The cancel flag also
  covers a round that has not started its analysis yet: that round ends without running it.
  The worker owns the document through shared ownership (`Arc`); it finishes the save or drops
  the service on its own. Nothing of the controller is used after it is freed: the readers are
  joined first, the worker holds only the shared document.
- **The worker thread and the unload of the library.** The worker lives for the process. When
  the host unloads the plugin library (`clap_entry.deinit`), the plugin marks every live
  document as closing, cancels its round (the cancel takes effect between two tracks), sends
  the worker a quit message and waits for it at most 1.5 s. A worker that has ended is joined.
  A worker that is still inside one track after 1.5 s stays behind: a thread cannot be stopped
  from outside, so the report says `ended: false` and the unload goes on. That is a risk for a
  host that unloads while a very long single track runs; the host normally destroys the
  controllers first, and a destroy cancels the round. A host that loads the library again in
  the same process gets a new worker.
- **Thread guards.** The reader threads and the worker thread run inside the panic guard. A
  panic in a reader ends that read as a failure and sends the failure message, so the round of
  the document does not wait for the source. A panic in the worker ends the round it happened
  in (its sources end without a result) and the worker starts again with a `worker_restarted`
  row. No function pointer of the host is unwrapped: a missing table or function ends the read
  as a failure.

## Rules that still hold, and the stage that proves each

| Rule | Stage |
| --- | --- |
| Nothing of this runs on the audio thread: 0 overruns while the analysis runs | threads, threads-blocks (fault: threads-fault) |
| The analysis runs on the worker thread, not on the request caller's thread | threads-off-request (fault: threads-fault-thread) |
| The host's main thread does no service work and never waits long: the destroy of a controller during an analysis returns in under 1 s | doc-close (faults: doc-close-fault) |
| Readers are created and destroyed on the main thread; ARA calls follow the SDK's own tests | conformance (10 runs of the ARATestHost) |
| Every `extern "C"` function and every thread of the plugin runs inside the panic guard; no unwrap or expect on a host pointer, or anywhere else, in the non-test code | crate-clippy, the ARATestHost runs, doc-panic (fault: doc-panic-fault), no-unwrap (fault: no-unwrap-fault) |
| The service writes only below the cache folder | doc-home (fault: doc-home-fault), files-repo, doc-id-hostile |

## The check

Host modes: `grow` (sources arrive in batches, or one after the other with `--gap-ms`, one can
be destroyed, `--rerequest` asks again for the sources that are complete), `docs` (two document
controllers in one process), `close-busy` (destroy a controller while its analysis runs, time
the call, stay alive for `--after-ms`, `--deinit` unloads the library), `restore2` (one archive
restored into two controllers of one process), `restore-busy` (another document works, this
one restores and stores at once) and `regions --update-after` (analyse, then change a region).
The test host implements the model update controller and prints every
`notifyAudioSourceContentChanged` as `host_content_changed`.

The plugin reports one `job` row per round (document id, folder, the tracks analysed, left
out and with an error, `partial`, the events as the bytes the service gave, the sources whose
notes changed), `round_start` and `round_end` rows with the clock (the outcome is `done`,
`cancelled`, `failed` or `idle`), a `doc_closed` row, a `worker_unloaded` row, a `services` row
(made, dropped, rounds), `doc_restored` rows and `source_destroyed` rows. The rows with the
clock let a stage order the events of the plugin against the events of the host.

The fixture that makes the split result differ: two toms that hear the snare (a quieter copy
2 and 3 ms after each of its 8 hits), the snare with 8 hits, and the kick. Analysed together,
the tom bleed rule marks the 16 bleed hits (`bleed: true`, out of the output, so no note).
Analysed in two requests of two without the others, it cannot. Several tries first: the M1a
fixture (the bleed of 3 snare hits) and a snare with 3 hits give no mark, because the rule
needs at least 5 hits of the source to learn the transfer gain. The cross bleed rule alone
only adds evidence keys (`a6_lead_ms`), which also differ in the bytes but give no note change.

Each new stage has a fault run. A switch of the plugin brings back one wrong behaviour, and
the same check function must refuse it, on the line that the switch breaks (a fault run that
fails for any other reason does not count: the stages `doc-two-fault` and `doc-close-fault`
name the line they require).

### The list of the test switches

`plugins/ara/switches.json` is the one list of the `DS_ARA_*` variables. The README table is
made from it (`check.py write-readme`). The stage `switches` reads the source, `check.sh` and
the README, and fails for a variable that is used and not listed, one that is listed and not
used, or a README table that was edited by hand. It also proves that the plugin variables are
cleared from the environment of every program the check starts (a normal run never inherits a
fault switch), and that `check.sh guard` fails, naming the variable, when a fault or scenario
switch is set in the caller's environment. `check.sh` runs that guard first in every mode.
`switches-fault` proves the check can fail.

## Review round 2: finding, fix, stage

Each row names the stage that proves the fix and the switch that brings the old behaviour back
(the fault run). The fault runs are the "fails before the fix" proof: they run the fixed
binary with the old behaviour switched on. The new stages were not run against the build of
91c580a itself.

| # | Finding | Fix | Stage and fault run |
| --- | --- | --- | --- |
| 1 | `doc_id` of the archive joined to the folder path: `..` or an absolute path leaves the documents folder | the id is taken only in the generated form; other values get a fresh id and a report row; track names and the keys of `events` are checked the same way; unit tests of both validators | `doc-id-hostile` (5 ids and 1 archive with bad track names; no file outside `documents/<id>/`); fault `DS_ARA_ID_FAULT=trust` (`doc-id-hostile-fault`) |
| 2 | two live documents with one id share one folder | registry of the ids in use; the second claimant keeps its fresh id | `doc-same-id`; fault `DS_ARA_REGISTRY_FAULT=off` |
| 3 | an archive stored before the worker ran the restore loses the events | the events of the document are set on the calling thread | `doc-restore-busy` and `doc-restore-busy-again`; fault `DS_ARA_RESTORE_FAULT=late-events` |
| 4 | a failed round leaves the sources stuck and a retry makes duplicate tracks | a source is one track for its life, keyed by its index; a failed round ends its sources as complete without a result; the next request runs a round again | `doc-retry`; faults `DS_ARA_RETRY_FAULT=duplicate`; the scenario `DS_ARA_SCENARIO=fail-first-round` makes the round fail |
| 5 | errors of single tracks are swallowed; a skipped track is silent; a leftover WAV counts as present | the per-track result is read; `errors`, `skipped` and `partial` in the `job` row; a WAV counts only when this process wrote it and its size matches | `doc-errors`; fault `DS_ARA_ERRORS_FAULT=swallow`; the scenario `corrupt-wav:<n>` damages one WAV; unit tests |
| 6 | the regions and the properties of an earlier source are frozen | a change of the regions or the properties of a source that took part starts a round with fresh snapshots of all sources | `doc-regions-change`; fault `DS_ARA_TOUCH_FAULT=ignore`; the stages `regions-update-record` and `regions-destroy-record` still pass |
| 7 | `doc-close` can pass without proof | the host stays alive 5 s after the destroy; the stage proves the round was running, the cancel took effect in a stated time, nothing is written after the close, the worker ends at the unload, and the document on disk is consistent (a new analysis in its folder works); a cancel before `analyze_start` ends the round without running it | `doc-close`, `doc-close-early`; faults `DS_ARA_CLOSE_FAULT=wait`, `=nocancel`, `DS_ARA_UNLOAD_FAULT=skip` |
| 8 | fault switches leak into normal runs | `switches.json`, the guard of `check.sh`, `no_switch()` clears every variable; `doc-two-fault` requires the own-folders line | `switches`, `switches-fault`; `doc-two-fault` |
| 9 | reader and worker threads outside the guard; an unwrap on a host pointer | both guarded; a panic in a reader sends the failure; the worker starts again; no unwrap or expect in the non-test code | `doc-panic`, `no-unwrap`; faults `DS_ARA_GUARD_FAULT=off` and `no-unwrap-fault`; the scenario `DS_ARA_PANIC_TEST` makes a thread panic once |
| 10 | a request for a complete source starts a full round with the reason `removal` | a request for a source that is complete is dropped before the settle time | `doc-rerequest`; fault `DS_ARA_ROUND_FAULT=always` |

The four smaller points of the review:

- `find_archived`: two sources of one name and no persistent id matched one archive entry.
  Done: an entry that an earlier source took is not taken again (unit test
  `two_sources_of_one_name_without_persistent_id_get_two_entries`).
- The wording of the notes (this note said only "bleed"): done, see "The rule".
- A restore that changes the id on the main thread before the worker's `set_id`: open, see
  "Open points".
- The folders of the documents are never removed: the rule is stated in the README (never
  removed, with the size of a folder); a clean-up rule is open, see "Open points".

## Review round 3 (the independent read of round 3)

One MUST and several smaller points. Each has a test or a stage; a fault switch brings the old
behaviour back for the stages.

| # | Finding | Fix | Test, stage and fault run |
| --- | --- | --- | --- |
| 1 | A host source name that ends in `..` (`a/..`) gave `Path::file_stem` `None`; the old `stem_of` then used the whole name, so `audio/NNN-<name>.wav` could leave the cache folder (created, written and deleted there) | `stem_of` gives one safe component for every input; the other host strings (persistent id, track name) were checked: the id is only compared, the track name goes through `safe_name` | unit test `a_host_source_name_never_makes_a_path_out_of_the_audio_folder` (`../../x/..`, `a/..`, `..`, an absolute path, an empty name, a name with a NUL and with a backslash); stage `doc-source-names` (4 hostile names, one source destroyed: 0 files outside `documents/<id>/`); fault `DS_ARA_ID_FAULT=trust`: the folder `escape-name` appears next to the cache |
| 2 | A move of a region in playback time alone started a round | only the modification range is compared; the update skips the touch | unit test `a_move_in_playback_time_alone_is_no_change`; stage `doc-region-move` (1 round, notes unchanged); fault `DS_ARA_TOUCH_FAULT=playback-time` (2 rounds) |
| 3 | A changed source without audio lost its change | its old snapshot stays until a round with audio | unit test `a_change_of_a_source_without_audio_is_not_lost` |
| 4 | One requested source without audio blocked the document for ever; `start_read` set `failed` without the message | a deadline of 30 s after the request; the late source is left out, named in `late`, the round is `partial`; the failure message is sent | stage `doc-late-audio` (the samples access of one source never comes: a round for the other source, byte for byte its reference, `late` names the source); fault `DS_ARA_ROUND_FAULT=no-deadline` (host timeout, no round) |
| 5 | After an unload with `ended: false` a second init got no worker | `start_worker` tests the flag of the worker, the unload clears the sender, a generation number keeps an old worker from clearing the flag of a new one | unit test `a_second_init_gets_a_worker_when_the_first_has_ended` |
| 6 | No null check on the archiving table | checked like the audio access table | stage `no-unwrap` now also finds a direct dereference of a host table (a pattern check: it finds that form, it cannot prove every access is checked); `no-unwrap-fault` has a sample |
| 7 | The README said the destroy returns within half a second in every case | the timer starts at the first line of the call; README and this note say what is bounded: the plugin's own work, 500 ms; not the join of a reader, which lasts as long as one host read call | text |
| 8 | An errored track that had events kept them while the README said it has none | rule: the old events and notes STAY and the `job` row names the track in `stale`; the README says so | unit test `a_track_with_an_error_keeps_its_old_events_and_says_they_are_stale` |
| 9 | Limits not stated | stated in "The design" (limits of the design) and in the README | text |
| 10 | Dead match arms, a poll loop that can run for ever, `LIVE_DOCS` grows | the arms stay with a comment (the service may get such a state); the poll loop stops after 10 s of `idle`; `LIVE_DOCS` is pruned at the destroy | text and the existing stages |

Timing limits of the close stages and the values measured: the destroy call must return in
under 1000 ms (measured 477 to 510 ms in the runs of rounds 3 and 4: it is the 500 ms wait,
because the worker does not acknowledge sooner), and the round must end as cancelled within
3500 ms after the destroy (measured 455 to 724 ms). Both limits are wide on purpose: a cancel
takes effect between two tracks, and a loaded host makes both values longer. The fault run
`wait` measures 4572 ms for the destroy.

## The result (the full run on the PR head)

`./check.sh all`: exit 0, `M1A CHECK: PASS`, 133 `ok` lines, 0 `FAIL` (89 of round 2, 36 of
round 3, 8 of round 4: the new stages, their fault runs and the reference lines they print). The log is
`out/final-check.log` (not committed). The values below are from that run.

| Stage | Result | Measured value | Fault switch that makes it fail, and the failure line |
| --- | --- | --- | --- |
| doc-id-hostile | ok | 5 bad ids (`../x`, `../../escape-rel`, an absolute path, empty, upper case) and 1 archive with bad track names: each restored into a fresh valid id, 0 files outside `documents/<id>/`, `doc_id_refused` and `restore_refused` rows | `DS_ARA_ID_FAULT=trust`: the folders `escape-rel` and `escape-abs` appear next to the cache |
| doc-same-id | ok | 2 ids, 2 folders, 2 projects on disk; the first keeps the stored id, the second gets `doc_id_taken`; both results byte for byte the reference | `DS_ARA_REGISTRY_FAULT=off`: 1 id, 1 folder |
| doc-restore-busy, doc-restore-busy-again | ok | the round of another document started 882 ms before the restore and was still running; the second archive holds 19 events (the first 19); it restores in a new process with every track byte for byte the reference | `DS_ARA_RESTORE_FAULT=late-events`: the second archive holds 0 events |
| doc-retry | ok | 1 `job_failed` row, 1 result round, 2 tracks for 2 sources, events byte for byte the reference, notes after the retry 5 and 3, rc 0 | `DS_ARA_RETRY_FAULT=duplicate`: 4 tracks (`name_2`) |
| doc-rerequest | ok | 1 round in all (the first), no round for the request of complete sources | `DS_ARA_ROUND_FAULT=always`: 2 rounds, the second with the reason `removal` |
| doc-errors | ok | the 2 good tracks (12 events) byte for byte the reference of the two together; the damaged track is in `errors` (`not a RIFF/WAVE file`), has no events and 0 notes; `partial` true; rc 0 (the ARA protocol has no error channel) | `DS_ARA_ERRORS_FAULT=swallow`: `errors` empty, `partial` false |
| doc-regions-change | ok | 2 rounds (`request`, `change`); the events of the last round are byte for byte the service reference of the new regions (6 events, the hit at 1.5 s is in); 6 notes after the change, 4 before | `DS_ARA_TOUCH_FAULT=ignore`: 1 round, events differ |
| doc-close | ok | destroy 510 ms (limit 1000); the round was running at the destroy; it ended as cancelled 649 ms after it (limit 3500); 0 job rows; 0 files written after the close; the worker ended at the unload; the document on disk has 5 tracks and a new analysis in its folder gives 14983 events | `=wait`: destroy 4572 ms; `=nocancel`: no cancelled end, the round runs on; `DS_ARA_UNLOAD_FAULT=skip`: no `worker_unloaded` with `ended: true` |
| doc-close-early | ok | cancel during the setup of a round (scenario `slow-setup`): the round ended 724 ms after the destroy with `cancelled before it started`, 0 job rows | (the `nocancel` run above) |
| doc-panic | ok | a panic in the worker: `worker_restarted`, the new request gives the reference result; a panic in a reader: `reader_panic`, the source is read again at the new request, reference result | `DS_ARA_GUARD_FAULT=off`: rc 2 for both, no result round |
| switches, switches-fault | ok | 24 variables listed; the source, `check.sh` and the README use no other; 22 plugin variables cleared even when all are set; `check.sh guard` refuses a set fault switch | an unlisted variable, a listed one nobody uses, a hand-edited README table: each reported |
| no-unwrap, no-unwrap-fault | ok | 0 unwrap or expect calls in the non-test code of 4 files | a sample with an unwrap and an expect: both found, a comment, `unwrap_or` and test code not |
| doc-source-names | ok | 4 hostile source names (`x/../../../../../escape-name/..`, `a/..`, an absolute path ending in `..`, empty): 0 files outside the cache folder, every WAV directly in `audio/` with a plain name, one source destroyed and its WAV deleted inside the folder | `DS_ARA_ID_FAULT=trust`: the folder `escape-name` appears next to the cache, folders are made in `audio/` |
| doc-region-move | ok | the second region moved in playback time only: 1 round in all, 4 notes before and after | `DS_ARA_TOUCH_FAULT=playback-time`: 2 rounds |
| doc-late-audio | ok | the samples access of the second source never comes (deadline 1500 ms in the stage): 1 round, the events of the first source alone byte for byte its reference (5 events), `late` names the second source, `partial` true | `DS_ARA_ROUND_FAULT=no-deadline`: host timeout, rc 2, 0 rounds |
| doc-two-fault | ok | now requires the own-folders line: `own folders False` | `DS_ARA_FOLDER_FAULT=shared` |
| the 89 stages of round 2 | ok | doc-service, doc-tracks, doc-rounds, doc-two, doc-archive, doc-destroy, doc-home, regions, identity, archive, threads, conformance and the rest all pass as before | (their own fault runs) |
| crate-fmt, -clippy, -test; workspace trio | ok | 31 unit tests, 0 warnings | |

New unit tests: both id and track-name validators, the registry, `find_archived` with two
equal names, `file_len` against the file on disk, a damaged track (errors, partial, the good
tracks equal the reference of the two), a change of the regions (new round, new segment, no
round when nothing changed), a track without audio (`skipped`, `partial`), the place of a
new track by creation index.

## The red proofs

The fault runs above are the failing-before proof of round 3: each switch brings one old
behaviour back in the fixed binary and the same check function refuses it, on the line the
switch breaks. The new stages were not run against the build of 91c580a. The red proofs of
rounds 1 and 2 (the new-only round, the long close, the first version) stay as they were:
`DS_ARA_ROUND_FAULT=new-only`, `DS_ARA_SERVICE_FAULT=per-job`, `DS_ARA_FOLDER_FAULT=shared`,
`DS_ARA_DESTROY_FAULT=keep`, `DS_ARA_HOME_FAULT=1`, `DS_ARA_RESTORE_FAULT=newid|empty|decode`,
`DS_ARA_SETTLE_FAULT=none`, `DS_ARA_TRACKS_FAULT=forget`.

## Open points

For the editor ticket (M1b2):

- After a restore the track sources point at a cache path without a file. The editor needs the
  audio: the plugin has no lazy re-read yet (the host must enable the samples access again).
  Until then a restored track is offline, is skipped by a round, and marks the round partial.
- The archive takes its events from the plugin's copy of the last round. Edits made in the
  editor must update that copy, or the store must read the events from the service.
- A later request adds tracks to an open project with `tracks_restore`; it builds the peak
  caches of the new tracks (a decode on the worker thread). A track with segments can be added
  only this way; `tracks_add` makes plain tracks. A small `crates/` entry for segments would be
  cleaner (not done: out of scope).
- The service has no event emitter and no lock client yet: the editor will need both, and it
  will have to show a round in progress and its cancel.
- A round analyses every track again; for a large session the cost grows with the number of
  tracks. The settle time bounds the number of rounds, not their length. Cancel works between
  tracks, not in the middle of one: a cancel takes as long as the track that runs.
- A round that is cancelled leaves the tracks of the round without events in the saved
  project until the next round.
- `analyze_start` with a list of names reads patterns (glob, ranges). Track names are made
  from file stems, so an exact name wins, but a stem such as `tom-1-3` is an edge case.

From the review of round 2 (not done):

- A restore changes the document id on the calling thread, and the worker sets the id of its
  service later. A reader that writes a WAV between the two puts it in the old random folder
  (the analysis still finds it, the folder is an orphan). A fix needs one shared id for the
  document and the service.
- The plugin never removes a document folder (rule in the README). A clean-up rule needs to
  know which folders an archive still names, which only the host knows. Candidates: remove
  the folder of a document that was never stored; remove a folder after a configured age.
- A worker that is still inside one long track at the unload stays behind (see "The worker
  thread and the unload of the library").
