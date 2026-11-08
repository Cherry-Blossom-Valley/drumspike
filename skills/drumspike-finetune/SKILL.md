---
name: drumspike-finetune
description: Fine-tune the drums of one song in Drum Spike. Use when asked to fine-tune, place or review the hits of a session hit by hit, at sample zoom, with Training Mode on.
version: <version>
---

# Fine-tune the drums of one song in Drum Spike

You place the hits of one song the way a person does by hand: one hit at a time, in the app's page at sample zoom, with
Training Mode on. The page helpers (`window.DSAgent`) do the reading and the nudging; you judge.

The app installed this skill for the binary `<binary>`. Use that path as it stands; no symlink is needed.

## Before you start

In this order, before any session:

1. Quit the Drum Spike window. The bridge needs the app home; a second home is not a way around it.
2. The MCP server (the owner runs this once): `claude mcp add --scope user drumspike -- "<binary>" mcp`.
   The `--scope user` registers the server for every folder. Check with `claude mcp list`.
3. A browser tool the agent can drive, if the agent session has none: Claude in Chrome, or Playwright MCP.
4. Start the agent session (the MCP tools load at session start).

If the bridge refuses with "the home is in use by app", ask the user to quit the Drum Spike window. Do not pass
another home. If the session has no browser tool, ask for one. Do not script a browser yourself.

The workflow is visible: open a visible browser window on the page by default (headless only if the user asks). Ask
for the project path if the prompt has none. The user watches only; say so once at the start.

## Set up (once)

1. Start the page in the background: `"<binary>" bridge`. Open the address it prints in the browser you control.
   `window.DSAgent` is there when the page has loaded.

## Workflow

1. Open or import the session with the MCP tools (`project_open`, or `project_import`), then `analyze_start` and
   `analyze_status` until `done`. Open the same project in the page.
2. Turn Training Mode on: `setTraining(true)` in the page (in the window: View > Training Mode).
   Check `DSAgent.status()`.
3. Per track: read `DSAgent.hits(track)`. Note the count and the doubtful hits.
4. Work in chunks of about 100 hits, one chunk per call: `plan = await DSAgent.plan(track, from, to)`.
5. Step through the hits so the user can watch, one hit at a time: look at the hit at sample zoom
   (`DSAgent.look(track, index)`), nudge it by whole samples where the rule says so, confirm it
   (`DSAgent.confirm(track, index)`), next hit. Name each step in the status line and hold briefly after each move
   so a person can follow. Read the samples around the hit as shares of the local peak. The placement rule: the
   hit is the last line sample before the stroke's first motion of either sign. A pre-ripple, bump or dip that
   stands above the detector's departure band and leads into the stroke belongs to the stroke. A bump, dip or
   ripple under the detector's departure band is not the stroke's first motion: the hit stays at the foot of the
   main stroke. Only a departure the detector's rule recognises moves a hit.
6. One hit at a time is the default while the user watches. Only when the user asked not to watch, apply the clear
   moves of a chunk in one call: `DSAgent.apply(track, list)`.
7. Leave the hits without a suggestion (`line_ok` false) as they are and list them for the owner: track, index, time,
   the reason. Write the review list next to the project file, for example `<session>/agent/left-for-review.tsv`.
   Make no own safety copy of the session: the app's `Backups/` folder keeps the versions.
8. `DSAgent.save()`. A lock file that still names a stopped bridge does not block: the app or the next bridge takes
   it. Do not delete it and do not mention it.
9. After the save, report the per-track table before anything else: one row per track with the track's name, its
   hits, the moved, the confirmed, the hand-placed and untouched, the ones without a suggestion and the doubtful
   ones, then a total row.
10. Then ask the user one question: export now (MIDI and spike files through the MCP tools, `export_all` and
    `job_get` until `done`) or leave it.

## Guardrails

- Never move or confirm a hit the user placed by hand. `DSAgent.hits` returns no field that marks a hand-placed
  hit (its `edited` flag marks a hand velocity edit or a restore, not a hand move): ask the user which hits are
  theirs before you work a track, and leave those as they are.
- Never delete a hit without `DSAgent.wide(track, index)` first. Look at the 20 ms view and decide on that.
- Never move a hit whose `judge.suggest` is `null` (`line_ok` false in a plan row; `why` says the reason, for example
  "departure far from the hit"). Leave it and list it.
- A `queue_doubt` row with `line_ok` true is a reason to look, not a block.
- A wobbly line stays doubtful. Do not talk yourself into a move.
- Never report sample data, file names, session names or song names outside this machine. Use "Session A", "Session B".
- One chunk per call. Do not plan a whole track in one evaluation; the page stays responsive and a failure costs one chunk.
- The helpers refuse while Training Mode is off. Turn it on; do not work around the refusal.
- Do not change detection settings, the keymap or the menu.
