# F6 (#582): a track is offline after the plugin is removed from one track and added on another

Status: fixed in the plugin. The standalone page and the app are not changed.

## What was seen

The user removed the plugin from one track of a DAW session and added it on another track. The page showed the new
track as offline: "analysis failed: track is offline (changed): relink it first" and "cannot draw this track: ...
segment 0 is offline (changed)". No hit could be edited. The text names an action (relink) that does not exist in a
DAW.

The plugin report of that run, in order: every audio source was destroyed and the document was closed; a new document
controller was made; the archive of the DAW was restored (first a project with no track, then one track); the audio
of the source was read again without an error and written to the cache; the round after it ended as `idle`.
The same source had the file number `002` in the cache before and `000` after, because the number is the creation index
of the source in the document, and the new document has fewer sources.

## Cause

Two facts, each read in the code and then proved by a stage.

1. The restore writes the project record of a track from a snapshot of the source whose audio is NOT read yet
   (`DocService::restore`, `wav_source`). It names the cache file where the reader will write it. The file does not exist
   at that time (the new document has another file number, and the removal of the old source deleted its files). So the
   record has a path and no size and no head hash (`build_source` needs the file to make them). The service decides
   the status of a source by size and head hash (`resolve_source`): a record with no size never matches, and a
   file that exists but does not match is `changed`. After the reader has written the file, the status of the track is
   `changed` for ever.
2. Nothing makes the record again. The read of a restored source with a result queues no round (rule of
   `no-request-restored`: the restored hits must not be analysed again just because the audio is read). The round that
   a later change starts (here: a change of the regions) compares the new snapshot with the old one; the old one has the same
   name, rate, length, generation and regions, so it is "the same" (`same_as` does not compare the file or the ready
   mark). The round ends as `idle` with the track still offline.

A restore into a cache that still holds the old file does not show it: `build_source` finds the file and the record
is right. That is why `restore-settings` keeps the cache of the store run (its comment names the "offline (changed)"
state it saw).

## Proof

- Stage `doc-readd`, parts `all` and `one`: the archive of a four-source session is restored in a new process with a
  fresh cache, the host reads the audio. The project document on disk is read independently of the plugin: the source
  record of each track has no size (or a size that is not the size of the cache file). The service status of
  each track is `changed`.
- Fault run `doc-readd-fault` (`DS_ARA_RESTORE_FAULT=no-repair`) brings back the old behaviour: no repair row, the record
  stays without identity, and the stage refuses it.
- The unit test `a_restored_track_is_pointed_at_the_new_cache_file_and_keeps_its_hits` shows `changed` with the file
  present, then `ok` after the repair, with the same hits.

## Fix rule

A restored source carries a mark `repair` until its audio is read. When the read is committed (`Msg::Ready`):

1. The archive now stores `content_hash`: the FNV-1a hash (`fnv64`, the hash the read already computes) of the samples
   that the plugin read when it stored. A source that was not read in that session carries the hash of the archive it
   was restored from. An archive without the field (an earlier plugin) loads as before; its audio counts as the same
   when the sample count is the same (the rule of the restore, R4).
2. Same hash, or no hash stored: `DocService::repoint` makes the record of the track again with the new file (only
   `segments` and `source`; the hits and every setting of the user stay), saves the project, tells the page to reload.
   No round runs. The report has a `track_repair` row with the status before and after.
3. Another hash: the restored hits belong to other audio. The source is marked stale (the rule of a changed sample
   count): the next round analyses it, keeps the hits that the user placed (the `keep` rule of the round), and
   the round points the record first (`analyse` calls `repoint` before it looks for changes). The report has a
   `restore_audio` row with `same: false`.
4. A failed repair (the service is busy with a job of the page) is tried again every 100 ms for as long as the other
   retries of the worker run. If the read fails, nothing is claimed: the mark stays, and the next good read repairs
   the track without a user action.
5. No page text is changed. The text of an offline track names "relink" (it comes from the app). A host-mode word
   mapping in `static/` was tried and taken out: the stage `standalone-diff` allows no file outside `plugins/ara/` and
   `docs/design/daw-plugin/`. After this fix the state that shows the text (an audio file the service cannot match)
   does not outlive a good read. A failed read leaves the old text until the next good read repairs the track; a
   DAW-worded text for that case is a separate change (it needs `static/`).

## Stages

| Stage | Proves | Fault run |
|---|---|---|
| `doc-readd` part `all` | four sources restored, audio read: every track online (record on disk matches the file), the stored hits byte for byte, no round | `doc-readd-fault` (`no-repair`) |
| part `one` | one source alone (the others are gone, its file number differs): the same | same |
| part `hand` | hits and settings that the user changed survive (a hit value, role, note) | - |
| part `nohash` | an archive without a content hash is the same audio | - |
| part `other` | another content hash: one round analyses the track, which ends online | - |

The #572 stages (`doc-store-window`, `doc-store-early`, `doc-restore-repair` and the restore stages) pass unchanged.

## Limits

- The case of a source that was removed earlier in the session and comes back is the same restore path: the archive
  matches a source by its persistent ID or name, not by its creation index, so a new index does not matter
  (the stage restores one source with another index than in the archive). The stage does not drive a host that destroys
  and adds the same source in one document.
- A round that analyses one stale source analyses every track whose audio is here, as every round does; the hits that
  the user placed are kept by the round, a hit that the user only deleted can come back in the tracks of that round
  (not changed by this fix).
