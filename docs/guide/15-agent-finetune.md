# 15. Let an agent fine-tune a song

Drum Spike can hand the last, slow pass over a song to an AI agent such as Claude Code. The agent
does what you do by hand: it goes to a hit, zooms in to single samples, reads the waveform around
the hit and, if the hit is not on the last sample before the stroke starts, moves it by whole
samples. Hits it is not sure about it leaves alone and lists for you. You stay the judge: every
move is a normal edit that you can undo.

## Set it up (once)

1. Open Settings and find the row "Agent fine-tune skill". Press Install. The app copies the
   skill to `~/.claude/skills/drumspike-finetune` and writes its own location into it, so you
   need no link in `/usr/local/bin`. After an update of Drum Spike the row says "older copy"
   and the button says Update.
2. Under the button is one line that starts with `claude mcp add drumspike`. Press Copy, paste
   it into a terminal and run it. This tells Claude Code how to reach Drum Spike. You do this
   once.

## Start a session

Ask your agent, in your own words:

```
Fine-tune the drums of ~/Music/Sessions/Song in Drum Spike.
```

The agent opens the session, analyses it if needed, turns on Training Mode (View > Training
Mode), and works through the tracks in chunks of about 100 hits. For each hit it looks at a
window of about a millisecond around the hit, at sample zoom.

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
exactly as if you had made it, with the view and the precision. You can look at the result in
the window when the agent is done: undo a move you do not like, or check the doubtful hits with
the review queue from chapter 9.

Next: the [agent API](../agent-api.md) lists the commands and the page helpers that the agent
uses, and the [guide README](README.md) lists all chapters.
