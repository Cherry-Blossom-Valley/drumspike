# drumspike-finetune

A skill for Claude Code that fine-tunes the hits of one song in Drum Spike, hit by hit, the way you would by hand.

## Before you start

In this order:

1. Keep the Drum Spike window open with the project in it. The agent works in that window and you watch it there;
   you quit nothing and the agent needs no browser. Only when no window is open does the agent start the page mode
   (`drumspike bridge`), and it then needs a browser tool.
2. Install the skill. It is part of the Drum Spike app: in Settings, the row "Agent fine-tune skill" has an Install
   button; it copies this folder to `~/.claude/skills/drumspike-finetune/` and writes the path of the app into
   `SKILL.md`. The same from a terminal: `"<path of the app binary>" cli agent_skill_install`.
3. Run the one line that the Settings row shows (and has a Copy button for) once:
   `claude mcp add --scope user drumspike -- "<path of the app binary>" mcp`. The `--scope user` registers the
   server for every folder, not only the one you run it from. Restart Claude Code so the tools load.
4. Then start the agent session (the tools load at session start). A browser tool (Claude in Chrome, or Playwright
   MCP) is needed only for the case that no window is open.

## Use

Ask Claude Code: "Fine-tune the drums of `~/Music/Sessions/Song` in Drum Spike." The agent calls the helpers of the
open window (`agent_call`), turns Training Mode on, and steps through the hits so you can watch, one
hit at a time: look at the hit at sample zoom, nudge it by whole samples where the rule says so, confirm it, next
hit. It names each step in the status line and holds briefly after each move (it plans in chunks of about 100, and
moves a whole chunk in one call only if you asked not to watch). It leaves the doubtful ones for you, reports a
per-track table after the save, and then asks you once whether to export now or leave it.

Every move and confirm lands in the Training Mode log exactly as if you had made it. Undo works as always. At the
end you get counts and a list of the hits left for you. Nothing leaves your machine.
