# Drum Spike 0.9.26

Changes since 0.9.25:

### Agent fine-tune

- An AI agent can fine-tune the hits of a song the way a person does: hit by hit at sample zoom, with Training Mode on. The page carries helpers for it (`window.DSAgent`: list the hits, look at one at sample zoom or in the 20 ms view, nudge it by whole samples, confirm it, plan a chunk, apply the clear moves, save). A move and a confirm go through the same code paths as the editor's own commands, so the agent's records equal a person's. The judge suggests a move only for a clear line with its departure near the hit; a doubtful hit (no clear line, or a departure far from the hit) stays for the person, and the doubt of the review queue is reported apart from the verdict of the judge. Helpers that write refuse while Training Mode is off, and a page in the background never hangs on a frame (#371).
- Settings > Agent fine-tune skill installs the skill `drumspike-finetune` for Claude Code with one click (status text, Install or Update, the `claude mcp add` line with Copy); the command line does the same (`agent_skill_install`, `agent_skill_status`). The skill ships inside the app bundle and the install bakes the app's binary path and version into it. The guide has a new page, Agent fine-tune (#371).
- `drumspike bridge` is the page mode of the shipped app: it serves the app's own page and the invoke API on localhost (127.0.0.1 only), prints one URL line, and runs until it is stopped; `--home` and `--projects` override the app's folders, `--port 0` picks a free port. The window and the bridge do not share one home at the same time: both hold a lock on the home folder, a second bridge or the window on the same home is refused with the holder's name. Training Mode in the page mode turns on with `setTraining(true)` in the page (#370).

### Fixes

- The about test expects the four bundle resources, the skill folder included (#375).
