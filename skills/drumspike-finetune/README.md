# drumspike-finetune

A skill for Claude Code that fine-tunes the hits of one song in Drum Spike, hit by hit, the way you would by hand.

## Install

The skill is part of the Drum Spike app. In Settings, the row "Agent fine-tune skill" has an Install button; it copies this
folder to `~/.claude/skills/drumspike-finetune/` and writes the path of the app into `SKILL.md`. The same from a terminal:
`"<path of the app binary>" cli agent_skill_install`. Then run the one line that the Settings row shows (and has a Copy
button for) once: `claude mcp add drumspike -- "<path of the app binary>" mcp`.

## Use

Ask Claude Code: "Fine-tune the drums of `~/Music/Sessions/Song` in Drum Spike." The agent starts the page mode
(`drumspike bridge`), opens the session, turns Training Mode on, looks at the hits in chunks of about 100, moves the clear
ones by whole samples, confirms the ones that are right, and leaves the doubtful ones for you.

Every move and confirm lands in the Training Mode log exactly as if you had made it. Undo works as always. At the end you get
counts and a list of the hits left for you. Nothing leaves your machine.
