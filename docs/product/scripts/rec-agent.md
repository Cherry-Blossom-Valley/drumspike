# Screen recording: the agent fine-tune feature (guide chapter 15)

The second half of `explainer-15-agent.mp4`: the recorded tutorial of the agent fine-tune
feature (#370 page mode, #371 page helpers and Settings row, the skill). Narration for
`tools/media/tts.py` (one `##` per scene; `>` lines are the scenario steps and captions for
`tools/media/record.py`, not spoken). Target length: about 100 s; it follows the recording
standard (`docs/product/recording-standard.md`: zoom to the action, a visible pointer, keys
and menu items on screen, the result holds, the 0.5 % floor). The scenario is
`tools/media/scenarios/part-agent.json`, on the demo session (`seed: "demo"`, the s top
close mic). The render runs with the built-in detector (the DSP path, as the shipped app
runs: no external detector), like the other recordings on the current main.

The hits the agent works on are the demo session's snare hits, read from the generated
session's truth (`t / 48000`): the wobbly lead at 1.875 s (the doubtful one), 3.125 s (the
move), 4.375 s (the look), 5.625 s (the confirm). The move's sign and size are checked
against `DSAgent.plan('s top', ...)` on the recording page before the take: the recorded move
is the suggestion the judge makes for that hit, so the picture never shows a move the agent's
own rule would refuse. The Settings scenes rewrite the two texts the app prints with host
paths (the install target and the `claude mcp add` line's binary) into their neutral forms
with `show_text`, immediately after the app writes them.

## ra-mode

> Caption: View > Training Mode. The menu item runs, and the status bar's training count appears: training: N records.

Before the agent works, turn Training Mode on: View, Training Mode. From now on every hit edit and Learn mark is logged with its audio context.

## ra-install

> Caption: Settings: the Agent fine-tune skill row. The Settings sheet opens on the Agent fine-tune skill row; the Install click turns the status text from not installed to installed, with the file count.

In Settings, the Agent fine-tune skill row: press Install. The app copies the skill next to your Claude skills and writes its own location into it, so you need no link in /usr/local/bin.

## ra-mcp

> Caption: the claude mcp add line. The row shows the one line and its Copy button; the Copy click answers copied on the status line.

Under the button is the one line that tells Claude Code how to reach Drum Spike. Press Copy, paste it into a terminal, run it once. That is the whole setup.

## ra-look

> Caption: DSAgent.look. The agent's pointer glides to the hit; the lane jumps to sample zoom, 0.9 milliseconds before and 0.6 after the hit.

Now the agent at work, on the page you just installed it for. Its first step on every hit is a look: the lane jumps to sample zoom, and the samples come out as numbers, shares of the local peak.

## ra-move

> Caption: DSAgent.move. The hit's stalk steps one sample onto the last sample of the line; the status line names the edit.

If the hit is not on the last sample of the line before the stroke, the agent moves it there by whole samples. The move runs through the same nudge your keyboard uses, so it is one normal, undoable edit.

## ra-confirm

> Caption: DSAgent.confirm. The status line answers: hit confirmed, logged as reviewed, right where it is.

A hit the agent has looked at and found right it confirms: Hits > Confirm Hit. The status line says the hit is logged as reviewed, right where it is.

## ra-doubt

> Caption: the doubtful hit stays for you. The wobbly hit's window: the lead is not a line, the spread is over the limit, and the hit does not move.

And here the judgement stops. This lead is not a line: its spread over the first twelve samples is over the limit. The agent leaves the hit alone and lists it for you, with the reason.

## ra-save

> Caption: DSAgent.save. The status bar answers project saved, and the training count stands next to it.

When it is done, the agent saves the project. The training count in the status bar is the record of what it did, exactly as if you had made every edit yourself.
