# 14. Automate it (agent API)

Every app operation is a command. A script or an AI agent can open a project, analyse it,
set the parameters, edit hits, build the tempo map and export, without the window. This
chapter is the overview; the full reference with every command and its arguments is the
[agent API](../agent-api.md) document.

## The two front ends

- The command-line tool `drumspike`. One command is one process: it opens the project, runs
  the command, saves when the command changed the project and closes. `drumspike help` lists
  the commands, `drumspike help <command>` prints one in full.
- The MCP server: `drumspike mcp` speaks JSON-RPC over stdin and stdout, one tool per
  command. To use it from Claude Code:

```bash
claude mcp add drumspike -- drumspike mcp
```

Then ask: "Open ~/Music/Sessions/Song, analyse it, fix the kick, save, and export all."

## What the commands cover

- Projects: open, import a session folder, new, save, save as. `project_open` with
  `read_only: true` reads a project while the app has it open, without the write lock.
- Analysis: `analyze_start` answers at once; poll `analyze_status` until `state` is `done`.
- Parameters: `track_params_set` for the detection, `track_velocity_set` for the velocity
  floor, ceiling and loudness cut. It recomputes every velocity, so call it before
  single-hit edits.
- Hits: `events_get` lists them, `events_edit` adds, moves, deletes and sets velocities.
  An added or moved hit must lie within the track.
- Followers and layers: `track_set_follow` makes a track follow another, `track_layer_set`
  sets the one-shot layer of a track, `track_sampler_set` keeps a sampler calibration.
- Tempo and meter: `tempo_map_get`, `tempo_map_set`, `tempo_map_import_midi`. The detector
  is `tempo_map_detect`: it changes nothing, you pass its `suggestion` to `tempo_map_set`.
- Export: `export_all` writes the MIDI and spike files, per track or as one multi-track
  file with the tempo map; poll `job_get` until `state` is `done`.
- Markers: `markers_get` and `markers_set` read and write the review markers.

## One command, one example

```bash
drumspike analyze_start --project "~/Song"      # open, analyse (progress on stderr), save, close
```

Several steps in one process: `drumspike batch` reads one command per line from stdin, and
`drumspike run script.json` runs a list of steps and stops at the first error. Nothing is
saved unless a step is `project_save`.

## What the agent cannot do

Play back (the transport needs the window and a sound device), draw the waveform tiles, and
undo. A process that opens a project for writing holds a lock; a second writer gets an
error that names the holder. Close the project without saving to undo, or use
`project_save_as` as a snapshot.

Next: the reference [agent API](../agent-api.md) has every command with its arguments and
result, and the [guide README](README.md) lists all chapters.
