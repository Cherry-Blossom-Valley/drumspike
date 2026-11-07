# drumspike-finetune

A skill for Claude Code that fine-tunes the hits of one song in Drum Spike, hit by hit, the way you would by hand.

## Install

1. Unpack `drumspike-finetune-skill.zip` into `~/.claude/skills/` so that `~/.claude/skills/drumspike-finetune/SKILL.md` exists.
2. Link the app's command once (see `docs/agent-api.md`, "Set up") and add the server: `claude mcp add drumspike -- drumspike mcp`.

## Use

Start `drumspike bridge`, open the address it prints in a browser, then ask Claude Code: "Fine-tune the drums of
`~/Music/Sessions/Song` in Drum Spike." The agent opens the session, turns Training Mode on, looks at the hits in chunks of
about 100, moves the clear ones by whole samples, confirms the ones that are right, and leaves the doubtful ones for you.

Every move and confirm lands in the Training Mode log exactly as if you had made it. Undo works as always. At the end you get
counts and a list of the hits left for you. Nothing leaves your machine.
