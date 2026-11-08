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

The MCP server (the owner runs this once): `claude mcp add --scope user drumspike -- "<binary>" mcp`. The
`--scope user` registers the server for every folder. Check with `claude mcp list`. Start the agent session after
that (the MCP tools load at session start).

Two ways to reach the page helpers. Try the window first:

- **A Drum Spike window is open (the default).** Call the helpers of the window with the MCP tool `agent_call`
  (`fn` is the helper, `args` its arguments as a list), or from a terminal `"<binary>" agent_call <fn> [arg]...`.
  The user watches the edit right there; you need no browser and the user quits nothing. "The home is in use" then
  means: use the window. Do not pass another home.
- **No window is open** (`agent_call` answers "no Drum Spike window is open"): start the page with
  `"<binary>" bridge` and drive it with a browser tool (Claude in Chrome, or Playwright MCP). If the session has
  none, ask for one. Do not script a browser yourself. Open a visible browser window on the page by default
  (headless only if the user asks). `window.DSAgent` is there when the page has loaded.

Ask for the project path if the prompt has none. The user watches only; say so once at the start.

In the rest of this skill `DSAgent.x(...)` means the helper `x`: `agent_call` with `fn: "x"` in the window, a call in
the page with the bridge.

## Workflow

1. In the window, `DSAgent.project()` says which project is open; if it is not the one asked for, ask the user to open
   it there. With the bridge, open or import the session with the MCP tools (`project_open`, or `project_import`),
   then `analyze_start` and `analyze_status` until `done`, and open the same project in the page. Do not call
   `project_open` on a project the window holds: the app has it locked.
2. Turn Training Mode on: `DSAgent.setTraining(true)` (in the window: View > Training Mode).
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

- Never move or confirm a hit the user placed by hand. Read the project's training log (the `path` of
  `DSAgent.status()`'s `training_status`, `training_status {sid}`): a hit with a move, add or confirm record whose
  `by` is `person` or absent is hand-placed. Leave those as they are, and ask the user only when the log is empty
  or unreadable.
- Never delete a hit without `DSAgent.wide(track, index)` first. Look at the 20 ms view and decide on that.
- Never move a hit whose `judge.suggest` is `null` (`line_ok` false in a plan row; `why` says the reason, for example
  "departure far from the hit"). Leave it and list it.
- A `queue_doubt` row with `line_ok` true is a reason to look, not a block.
- A wobbly line stays doubtful. Do not talk yourself into a move.
- Never report sample data, file names, session names or song names outside this machine. Use "Session A", "Session B".
- One chunk per call. Do not plan a whole track in one evaluation; the page stays responsive and a failure costs one chunk.
- The helpers refuse while Training Mode is off. Turn it on; do not work around the refusal.
- Do not change detection settings, the keymap or the menu.
