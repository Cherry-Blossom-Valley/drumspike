# drumspike-finetune

A skill for Claude Code that fine-tunes the hits of one song in Drum Spike, hit by hit, the way you would by hand.

## Before you start

In this order:

1. Quit the Drum Spike window. The bridge needs the app home; a second home is not a way around it.
2. Install the skill. It is part of the Drum Spike app: in Settings, the row "Agent fine-tune skill" has an Install
   button; it copies this folder to `~/.claude/skills/drumspike-finetune/` and writes the path of the app into
   `SKILL.md`. The same from a terminal: `"<path of the app binary>" cli agent_skill_install`.
3. Run the one line that the Settings row shows (and has a Copy button for) once:
   `claude mcp add --scope user drumspike -- "<path of the app binary>" mcp`. The `--scope user` registers the
   server for every folder, not only the one you run it from. Restart Claude Code so the tools load.
4. A browser tool the agent can drive (Claude in Chrome, or Playwright MCP), if the agent session has none.
5. Then start the agent session (the tools load at session start).

## Use

Ask Claude Code: "Fine-tune the drums of `~/Music/Sessions/Song` in Drum Spike." The agent starts the page mode
(`drumspike bridge`), opens the session, turns Training Mode on, and steps through the hits so you can watch: look
at the hit at sample zoom, nudge it by whole samples where the rule says so, confirm it, next hit. It names each
step in the status line and holds briefly after each move. It works in chunks of about 100, leaves the doubtful
ones for you, and after the save asks you once whether to export now or leave it.

Every move and confirm lands in the Training Mode log exactly as if you had made it. Undo works as always. At the
end you get counts and a list of the hits left for you. Nothing leaves your machine.
