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
  (`fn` is the helper, `args` its arguments as a list), or from a terminal `"<binary>" cli agent_call <fn> [arg]...` (on the app binary the word `cli` comes first).
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
   so a person can follow. A hit counts as looked at only after `DSAgent.look` on that hit: a row of `plan`
   or `audit` is a listing, never a look, and never a move by a number it carries. Read the samples around the
   hit as shares of the local peak. The placement rule: the
   hit is the last line sample before the stroke's first motion of either sign. A pre-ripple, bump or dip that
   stands above the detector's departure band and leads into the stroke belongs to the stroke. A bump, dip or
   ripple under the detector's departure band is not the stroke's first motion: the hit stays at the foot of the
   main stroke. Only a departure the detector's rule recognises moves a hit.
   `null` or `stands` is not a verdict: when the judge answers `null` or `stands`, look at the hit yourself
   (`DSAgent.look`) and decide. Move the hit when the verdict is `late`, by `judge.foot_off` samples (a whole
   number; negative is earlier). A hit whose foot lies more than 12 samples before or 6 after it has no
   suggestion (`suggest` is `null`): such a hit is a look and a listing, never a move by that number. A late hit
   can carry a `null` `suggest` when its foot is far from the hit:
   look at those yourself too, and if you do not move one, list it with the reason. Leave a hit only when no
   quiet stretch exists before it (`no_quiet_line`) - and list it as "no quiet stretch", never as "not looked
   at". Do not
   invent thresholds: use the verdict and the documented bands. After the moves, re-read every moved hit
   (`DSAgent.look`) and check the verdict against the new position.
6. One hit at a time is the default while the user watches. Only when the user asked not to watch, apply the clear
   moves of a chunk in one call: `DSAgent.apply(track, list)`.
7. List the hits you left: track, index, time and the verdict's name. Write the review list next to the project
   file, for example `<session>/agent/left-for-review.tsv`. Make no own safety copy of the session: the app's
   `Backups/` folder keeps the versions (`DSAgent.backups()` names the backup that holds the state before this
   session; a backup is named with the time of the save that replaced it, and holds the state before that save).
8. Before `DSAgent.save()`, run `DSAgent.audit(track)` on every track (it looks at every hit of the track, so give
   it a longer `timeout_ms`, for example 600000; the default is 60 s). The remaining late hits must be zero; if
   one stays, list it one by one with the reason, and say so in the report.
9. `DSAgent.save()`. A lock file that still names a stopped bridge does not block: the app or the next bridge takes
   it. Do not delete it and do not mention it.
10. After the save, report the per-track table before anything else: one row per track with the track's name, its
    hits, the moved, the confirmed, the hand-placed, "looked at, right" and "no quiet stretch" (the two parts of
    not moved), and "not looked at" - that column must be zero, and a hit counts as looked at only after
    `DSAgent.look` on that hit - then a total row.
11. Then ask the user one question: export now (MIDI and spike files through the MCP tools, `export_all` and
    `job_get` until `done`) or leave it.

## Guardrails

- Never move or confirm a hit the user placed by hand. Name those hits with `DSAgent.handPlaced(track)` on
  every track before you work on it (the training log is far too large to read: the helper answers one row
  `{index, id, t, sample, ops}` per hit that carries a person's move, add or confirm record). Leave those
  hits as they are, and ask the user only when the helper answers an empty list on every track and
  `DSAgent.status()` shows no records.
- Never delete a hit without `DSAgent.wide(track, index)` first. Look at the 20 ms view and decide on that.
- A `queue_doubt` row is a reason to look, not a block.
- Never report sample data, file names, session names or song names outside this machine. Use "Session A", "Session B".
- One chunk per call. Do not plan a whole track in one evaluation; the page stays responsive and a failure costs one chunk.
- The helpers refuse while Training Mode is off. Turn it on; do not work around the refusal.
- Do not change detection settings, the keymap or the menu.
