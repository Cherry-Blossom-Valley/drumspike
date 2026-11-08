# Screen recording: the agent fine-tune feature (guide chapter 15)

The recorded tutorial of the agent fine-tune feature, as the user sees it (#381, the re-take
of the 2026-11-08 part-agent take). Narration for `tools/media/tts.py` (one `##` per scene; `>`
lines are the scenario steps and captions for `tools/media/record.py`, not spoken). Target
length: about 100 s; it follows the recording standard (`docs/product/recording-standard.md`:
zoom to the action, a visible pointer, keys and menu items on screen, the result holds, the
0.5 % floor). The scenario is `tools/media/scenarios/part-agent.json`, on the demo session
(`seed: "demo"`, the s top close mic). The render runs with the built-in detector (the DSP
path, as the shipped app runs: no external detector), like the other recordings on the current
main.

The story, per the owner's review of 2026-11-08: prompt first, then the agent at work through
its `drumspike-finetune` skill, and one visible move of a hit by two samples. Training Mode is
not a topic of the tutorial: no scene, caption or close-up names it; the agent's skill
switches it on as its own step 2, and the narration says so once. The set up is one short
scene. The terminal pane types the real agent run of `tools/media/fixtures/agent-run/`
(ticket #380): the user's prompt from `prompt.txt`, the skill load and the first tool calls
from `transcript.txt`. No agent text is invented; when #380 lands, the fixture line ranges of
the two terminal scenes are pinned to the real files.

The hits the scenes act on are the demo session's own events, checked against `DSAgent.plan`
on the recording page before the take (probe of 2026-11-08): no s top event carries a non-zero
suggest, so the move is the k in close mic's hit at 3.750167 s, whose plan row suggests -2 (the
stored hit sits two samples after the last line sample; the move applies the suggestion as
given). The move scene zooms to 10 ms (px_per_sample well over 2) and holds 2 s on each side of
the move; the before and after pictures share the window, and the frame diff must change
pixels only in the two marker columns. The doubt is the s top hit at 7.020938 s, whose lead
spread 0.44 is over the 0.03 limit and whose plan row's why is "no clear line before the hit"
(the caption names it). The look (4.375 s) and the confirm (5.625 s) stay on the s top close
mic. The Settings scene rewrites the two texts the app prints with host paths (the install
target and the `claude mcp add` line's binary) into their neutral forms with `show_text`,
immediately after the app writes them.

## ra-prompt

> Caption: you ask the agent to fine-tune the song. A terminal pane types the user's prompt from the fixture, line by line.

You write one prompt and hand the song to the agent: fine-tune the kick and the snare, move
the clear hits onto the line, leave the doubtful ones for me, and tell me the counts when you
are done. The agent works through the Drum Spike page, the same lanes you work in.

## ra-skill

> Caption: the agent loads the drumspike-finetune skill and works hit by hit. The terminal pane types the run's first tool calls from the fixture; no close-up of any setting.

The agent loads the drumspike-finetune skill and follows it step by step. The skill has it
prepare the session first, the way you would; that part of the run you never see on screen.

## ra-setup

> Caption: the set up: install the skill, run the one mcp line. The Settings sheet shows the Agent fine-tune skill row: the Install click turns the status to installed, and the Copy click answers copied on the status line.

Once, you set it up. In Settings, the Agent fine-tune skill row: press Install, and the skill
is copied next to your Claude skills. Under it is the one line that tells Claude Code how to
reach Drum Spike; press Copy, paste it into a terminal, run it once. That is the whole set up.

## ra-look

> Caption: the agent looks at every hit before it touches it. The agent's pointer glides to the hit; the lane jumps to sample zoom, 0.9 milliseconds before and 0.6 after the hit.

Now the agent at work, on the page you just installed it for. Its first step on every hit is a
look: the lane jumps to sample zoom, and the samples come out as numbers, shares of the local
peak.

## ra-move

> Caption: a clear hit steps two samples onto the last sample of its line. The kick lane holds one zoom from before the move to after: the hit stalk steps two samples onto the last sample of the line, and the status line names the edit.

Where the line before the stroke is clear and the hit is off it, the agent moves the hit by
whole samples: here, two samples onto the last line sample. The zoom stays on the hit the
whole time, so you can see the stalk step. The move runs through the same nudge your keyboard
uses, so it is one normal, undoable edit, and the status line names it.

## ra-confirm

> Caption: a hit the agent finds right it confirms. The status line answers: hit confirmed, logged as reviewed, right where it is.

A hit the agent has looked at and found right it confirms: Hits > Confirm Hit. The status line
says the hit is logged as reviewed, right where it is.

## ra-doubt

> Caption: the doubtful hit stays for you: no clear line before the hit (the plan row's why). The wobbly hit's window: the lead is not a straight line, the spread is over the limit, and the hit does not move.

And here the judgment stops. This lead is not a straight line: its spread over the first 12
samples is over the limit. The agent leaves the hit alone and lists it for you, with the
reason.

## ra-save

> Caption: the project is saved. The status bar answers project saved.

When it is done, the agent saves the project and reports the counts: seen, moved, confirmed,
left for you. Every edit it made is exactly the edit you could have made yourself, one undo
step at a time.
