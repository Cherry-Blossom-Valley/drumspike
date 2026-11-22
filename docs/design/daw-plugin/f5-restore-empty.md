# F5 (#572): a stored or restored plugin document with no tracks and no events

Status: fixed in the plugin. The stages were not at fault.

## What was seen

The stages `doc-archive`, `doc-same-id`, `doc-restore-busy`, `doc-restore-busy-again` and `doc-id-hostile`
failed now and then with "0 events, 0 tracks" or "the second archive holds 0 events (the first 0)". About one
stage run in five failed on main. One full gate run failed in all five stages at once, because the five stages
read the one archive that `archive_store()` writes once per process.

The words of the report say that the FIRST store was already empty. So the restore was not the first defect.

## Cause

A round of the plugin made its result visible in two steps, in this order (`run_job` in `src/lib.rs`):

1. For each source: the notes, the track name and the flag `has_events` are set. From this moment the host
   sees the source as complete: `isAudioSourceContentAnalysisIncomplete` answers false, and the host may store.
2. Then the settings are read from the service (a call that takes time) and only then the events of the
   document (`doc.events`) are set.

`storeObjectsToArchive` writes `doc.events` and the notes of the sources. A store in the window between step 1
and step 2 wrote an archive with ALL notes and NO events. The restore of such an archive has no events to
put into the project, so the document had no tracks and no events. The test host polls every 20 ms and stores
at once when the poll sees the sources complete. That is why the window was hit in about one run in five.

This is a defect of the plugin, not of the stages. The stages share one archive (`archive_store()`, once per process), so one bad store fails several stages at once. That
archive was empty because of the plugin, not because of the cache folder. The store run starts with a fresh cache folder (`host_run` without `keep_cache`), so no old project
was opened.

## Proof

1. Natural failures on main, evidence from a failed run (stage run alone, 10 repeats of the four stage groups,
   3 stage runs failed: `doc-archive`, `doc-restore-busy`, `doc-id-hostile`). The archive of a failed run holds
   the notes of all four sources (5, 3, 7 and 4 notes, all with the track name) and `"events": {}`. A good run
   holds the same notes and 19 events in 4 tracks. Notes without events cannot come from a round that
   finished: the two lines of the round (notes and events) were out of step.
2. A window that fails every time. On the code of main, a pause of 600 ms was put between step 1 and step 2
   (a temporary switch, not committed). `doc-archive` then failed 3 times in 3 runs, and each archive held 19 notes and 0
   events.
3. The committed fault run: `DS_ARA_ROUND_FAULT=events-late` brings back the old order (events after the notes,
   no lock between) and `publish-delay` widens the window. The stage `doc-store-window-fault` shows the archive
   with 19 notes and 0 events, and the check refuses it.

## Fix

A round now publishes in one step. The locks that a store takes (events, then settings, the same order as the
store) are held from the first source to the assignment of the events and the settings. A store sees the document
before the round or after it. The read of the settings (a service call) is made before the locks are taken, and
it names the tracks of the round itself, because their names are not yet set at that time. The store waits for a
publication that is running; a publication copies notes and takes microseconds, so the host's thread does not
wait for a round.

A store before the first round has ended: decision, no wait. The store writes the last complete state. For a new
document that state has the sources, no notes and no events: there is nothing else. The store does not block
for the round (a round runs for seconds or minutes). Good data is not replaced by it, because a document that had
a restore or an earlier round keeps its events in the document, and the store writes them.

The restore of such an early archive had a second defect. It marked every archived source as complete, so a
source with no result stayed without hits for ever. A source that the archive holds with no track name, no note
and no events is now left without a result, and the next round with its audio analyses it (the report row
`restore_no_result`).

A restore never shows an empty document while the archive holds events: the events of the archive are set on the
calling thread (this was already so, proved by `doc-restore-busy`).

## Stages and fault runs

| Stage | Proves | Fault run |
|---|---|---|
| `doc-store-window` | store in the middle of a publication: 19 notes and 19 events, byte for byte the reference | `doc-store-window-fault` (`events-late`): 19 notes, 0 events |
| `doc-store-window` (part `doc-store-early`) | store before the first round has ended: no notes, no events, the call takes under 1 s while the round needs 1.5 s more; the late archive of the same process holds every event; the early archive restored in a new process is analysed again and gives the reference events and notes | `doc-store-early-fault` (`DS_ARA_RESTORE_FAULT=result`): no round, no note |
| `archive_store()` | refuses an archive with no event and names the cause | - |

## Limits

- The test switches `publish-delay` and `events-late` exist only in the test build (`test-switches`); the
  delivery build reads no switch.
- An archive that an older build wrote in the window (all notes, no events) is not repaired by the restore. It
  holds no hit; only a new analysis brings them back. The plugin is not released yet (assumption of the author, not checked against any session file).
- Events in an archive whose source the host does not give again (the source is gone or has another name and
  persistent ID) are not placed. This is a separate case and was not changed.
- The stage times a store in the window with a delay of the plugin. Real rounds publish in microseconds, so the
  real window is small, but the host cannot be made to avoid it.
