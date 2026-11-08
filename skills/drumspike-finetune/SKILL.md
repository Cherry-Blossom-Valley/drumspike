---
name: drumspike-finetune
description: Fine-tune the drums of one song in Drum Spike. Use when asked to fine-tune, place or review the hits of a session hit by hit, at sample zoom, with Training Mode on.
version: <version>
---

# Fine-tune the drums of one song in Drum Spike

You place the hits of one song the way a person does by hand: one hit at a time, in the app's page at sample zoom, with
Training Mode on. The page helpers (`window.DSAgent`) do the reading and the nudging; you judge.

The app installed this skill for the binary `<binary>`. Use that path as it stands; no symlink is needed.

## Set up (once)

1. The MCP server (the owner runs this once): `claude mcp add drumspike -- "<binary>" mcp`. Check with `claude mcp list`.
2. Start the page in the background: `"<binary>" bridge`. Open the address it prints in the browser you control.
   `window.DSAgent` is there when the page has loaded.

## Workflow

1. Open or import the session with the MCP tools (`project_open`, or `project_import`), then `analyze_start` and
   `analyze_status` until `done`. Open the same project in the page.
2. Turn Training Mode on: `setTraining(true)` in the page (in the window: View > Training Mode).
   Check `DSAgent.status()`.
3. Per track: read `DSAgent.hits(track)`. Note the count and the doubtful hits.
4. Work in chunks of about 100 hits, one chunk per call: `plan = await DSAgent.plan(track, from, to)`.
5. Look at every suggested move and every doubtful row with `DSAgent.look(track, index)`. Read the samples around the
   hit as shares of the local peak. The placement rule: the hit is the last line sample before the stroke's first
   motion of either sign. A pre-ripple, bump or dip that leads into the stroke belongs to the stroke.
6. Apply the clear moves with `DSAgent.apply(track, list)` (or one `DSAgent.move`). Confirm a reviewed hit that is already
   right with `DSAgent.confirm(track, index)`.
7. Leave the hits without a suggestion (`line_ok` false) as they are and list them for the owner: track, index, time, the reason.
8. `DSAgent.save()`, then export with the MCP tools (`export_all`, `job_get` until `done`).
9. Report aggregates only: hits seen, moved, confirmed, left doubtful, the spread of the moves in samples.

## Guardrails

- Never delete a hit without `DSAgent.wide(track, index)` first. Look at the 20 ms view and decide on that.
- Never move a hit whose `judge.suggest` is `null` (`line_ok` false in a plan row; `why` says the reason, for example
  "departure far from the hit"). Leave it and list it.
- A `queue_doubt` row with `line_ok` true is a reason to look, not a block.
- A wobbly line stays doubtful. Do not talk yourself into a move.
- Never report sample data, file names, session names or song names outside this machine. Use "Session A", "Session B".
- One chunk per call. Do not plan a whole track in one evaluation; the page stays responsive and a failure costs one chunk.
- The helpers refuse while Training Mode is off. Turn it on; do not work around the refusal.
- Do not change detection settings, the keymap or the menu.
