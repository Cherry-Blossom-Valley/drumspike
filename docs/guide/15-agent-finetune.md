# 15. Let an agent fine-tune a song

Drum Spike can hand the last, slow pass over a song to an AI agent such as Claude Code. The agent
does what you do by hand: it goes to a hit, zooms in to single samples, reads the waveform around
the hit and, if the hit is not on the last sample before the stroke starts, moves it by whole
samples. Hits it is not sure about it leaves alone and lists for you. You stay the judge: every
move is a normal edit that you can undo.

## Before you start

In this order:

1. Keep the Drum Spike window open with the song in it. The agent works in that window and you
   watch it there: you quit nothing and the agent needs no browser. Only when no window is open
   does the agent start the page mode (`drumspike bridge`) in a browser.
2. Open Settings and find the row "Agent fine-tune skill". Press Install. The app copies the
   skill to `~/.claude/skills/drumspike-finetune` and writes its own location into it, so you
   need no link in `/usr/local/bin`. After an update of Drum Spike the row says "older copy"
   and the button says Update.
3. Under the button is one line that starts with `claude mcp add --scope user drumspike`.
   Press Copy, paste it into a terminal and run it. The `--scope user` registers the server
   for every folder, not only the one you run it from. This tells Claude Code how to reach
   Drum Spike. You do this once.
4. Then start the agent session (the MCP tools load at session start). A browser tool (Claude
   in Chrome, or Playwright MCP) is needed only when no window is open.

The app's binary answers on its own, without a window: `drumspike-desktop --help` (also `-h` or
`help`) prints the modes it has, and `--version` prints the version. An agent can read them
before it starts; an unknown flag never opens a second Drum Spike window.

## Start a session

Ask your agent, in your own words:

```
Fine-tune the drums of ~/Music/Sessions/Song in Drum Spike.
```

The agent works on the song that is open in the window, turns on Training Mode (View > Training
Mode), and steps through the hits so you can watch, one hit at a time (it plans in chunks of
about 100 hits). For each hit it looks at a window of about a millisecond around the hit, at
sample zoom, names each step in the status line and holds briefly after each move. After the
save it reports a per-track table (hits, moved, confirmed, hand-placed and untouched, no
suggestion, doubtful, and a total row) and asks you once whether to export now or leave it.

## What the agent does and does not do

- It moves a hit only when the line before the stroke is clean, and then to the last sample of
  that line, as in chapter 4.
- It confirms a hit it has looked at and found right (Hits > Confirm Hit), so the Training Mode
  log says "reviewed" for that hit.
- It never deletes a hit without first looking at a 20 ms view of it.
- It leaves a wobbly line, a hit with a noisy lead-in and every other doubtful hit as it is, and
  lists them at the end: the track, the hit number, the time and the reason.
- It reports counts only: how many hits it saw, moved, confirmed and left for you.

Because Training Mode is on, every move and confirm of the agent is written to the training log
exactly as if you had made it, with the view and the precision, and the log says who made each
edit: you or the agent. You can look at the result in
the window when the agent is done: undo a move you do not like, or check the doubtful hits with
the review queue from chapter 9. After the save the agent asks you once whether to export now
(MIDI and spike files) or leave it.

Next: the [agent API](../agent-api.md) lists the commands and the page helpers that the agent
uses, and the [guide README](README.md) lists all chapters.
