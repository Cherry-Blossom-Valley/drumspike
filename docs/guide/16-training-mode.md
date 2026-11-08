# 16. Training Mode

Training Mode exists so that Drum Spike can learn to place hits the way you do. With it
on, every move, add and delete of a hit, every Confirm Hit and every Learn This or Forget
This mark is written to a training log as a record, with the audio around the hit. The log
is never part of the project, and it never changes a hit on its own: with the mode off, the
app behaves exactly as without it.

Your corrections are used twice. While the mode is on, the review queue of chapter 9 also
doubts a hit's placement: no steep edge at the stored position, a line before it that is noisy
or sloping, or onset estimates that disagree by 4 samples or more. And the records are the
truth the placement is measured against: when a later version of the placement rule is scored
against your rows, the hits you put right have to stay right. An agent (chapter 15) writes the
same kind of record as you.

## Turn it on

1. Press **View > Training Mode**. The check stays set across restarts; it is a setting of the
   app, not of the project.
2. While it is on, the status line counts the records of the open project, for example
   `training: 12 records`.

## Work as usual

Nothing about editing changes. The review queue (**Hits > Next Doubtful Hit**, `D`) walks you
to the doubtful hits, including the placement doubts of the mode. You check a hit in the lane
at sample zoom (chapter 4) and nudge it onto the last sample of its line by whole samples
(**Edit > Nudge Hit Earlier** and **Edit > Nudge Hit Later**, `Left` and `Right`). A hit that
the detector missed you add on the playhead (**Edit > Add Hit at Playhead**, `H`), a false hit
you delete (press Delete or Backspace for **Edit > Delete Hit**), a hit you have checked and
found right you mark reviewed (**Hits > Confirm Hit**). A Learn example works too
(**Hits > Learn This**, `K`; **Hits > Forget This**, `Shift+K`). Each of these writes one
record. A failed log write never fails the edit.

## The log

The records land in one file per project in the app's data folder
(`~/Library/Application Support/com.cbv.drumspike/training/` on macOS), named by the project's
id (the first 12 hex digits built from the project's creation time), so a rename keeps the
file. A record says what you did, the old and the new position, the rule versions in force,
the hit's confidence, the line measures of the hit and the samples around it: 40 ms before
and 10 ms after. A record also says who made it: `by` is person or agent. It can be read
without the session.

## The export

    uv run python tools/training_export.py <training log>.jsonl

prints the counts per role and op and writes the truth list (`training_truth.json`): every
record with its track, its positions, its precision and its line measures. The printout names
no project and no path, so you can paste it into a ticket. The truth file carries the track
names, so keep it with the logs. By default the export takes only your own records
(`--by person`); `--by agent` takes the agent's, `--by all` takes every record.

The records stay on your machine until you send them: the log is written when you edit, and
nothing is sent by itself.

Next: the [guide README](README.md) lists all chapters.
