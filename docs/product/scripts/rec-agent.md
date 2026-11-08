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

The hits the scenes act on are the demo session's own events, checked against
`DSAgent.plan` on the recording page before the take (probe of 2026-11-08): no s top event
carries a non-zero suggest, so the move is the k in close mic's hit at 3.750167 s, whose plan
row suggests -2 (the stored hit sits two samples after the last line sample; the move applies
the suggestion as given). The doubt is the s top hit at 7.020938 s, whose lead spread 0.44 is
over the 0.03 limit and whose plan row's why is "no clear line before the hit" (the caption
names it). The look (4.375 s) and the confirm (5.625 s) stay on the s top close mic. The
Settings scenes rewrite the two texts the app prints with host
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

Under the button is the one line that tells Claude Code how to reach Drum Spike. Press Copy, paste it into a terminal, run it once. That is the whole set up.

## ra-look

> Caption: DSAgent.look. The agent's pointer glides to the hit; the lane jumps to sample zoom, 0.9 milliseconds before and 0.6 after the hit.

Now the agent at work, on the page you just installed it for. Its first step on every hit is a look: the lane jumps to sample zoom, and the samples come out as numbers, shares of the local peak.

## ra-move

> Caption: DSAgent.move. The kick lane's hit stalk steps two samples onto the last sample of the line; the status line names the edit.

If the hit is not on the last sample of the line before the stroke, the agent moves it there by whole samples. The move runs through the same nudge your keyboard uses, so it is one normal, undoable edit.

## ra-confirm

> Caption: DSAgent.confirm. The status line answers: hit confirmed, logged as reviewed, right where it is.

A hit the agent has looked at and found right it confirms: Hits > Confirm Hit. The status line says the hit is logged as reviewed, right where it is.

## ra-doubt

> Caption: the doubtful hit stays for you: no clear line before the hit (the plan row's why). The wobbly hit's window: the lead is not a straight line, the spread is over the limit, and the hit does not move.

And here the judgment stops. This lead is not a straight line: its spread over the first 12 samples is over the limit. The agent leaves the hit alone and lists it for you, with the reason.

## ra-save

> Caption: DSAgent.save. The status bar answers project saved, and the training count stands next to it.

When it is done, the agent saves the project. The training count in the status bar is the record of what it did, exactly as if you had made every edit yourself.
