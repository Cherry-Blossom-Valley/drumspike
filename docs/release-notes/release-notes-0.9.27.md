# Drum Spike 0.9.27

Changes since 0.9.26:

### Agent fine-tune

- The agent now drives the Drum Spike window you have open, so you watch every look, nudge and confirm in the window itself. The window answers on a local endpoint (127.0.0.1) with a token; the MCP tool `agent_call` and `drumspike-desktop cli agent_call` reach it with one call. The skill prefers the window and falls back to the page mode with a browser tool (#388).
- The local endpoint is open whenever the app runs. It listens on 127.0.0.1 only and needs the token in `window-endpoint.json` in the app's data folder, which only your user can read.
- Agent set-up works the first time: the command in Settings registers the MCP server for every folder (`--scope user`), and the skill and the guide say what to do before the agent starts. The agent asks for the project path if the prompt has none (#384).
- The binary answers `--help`, `-h`, `help` and `--version` and exits. An unknown flag prints the usage and never opens a second window (#383).
- The judge agrees with the detector. A bump under the detector's band is not the stroke's first motion, measured against the hit's peak as the detector does (#394). Hits without a clear line are left and listed with reasons. The run ends with a per-track table and the export question. The placement measure tool and its report are in the repository (#385).

### Training Mode

- Every record says who made it, a person or an agent. `tools/training_export.py` takes your own records by default; `--by agent` and `--by all` take the others (#389).
- Guide chapter 16 and Explainer 16 state what Training Mode is for and then walk through the process from start to end (#382). The agent turns Training Mode on for its own edits, so that they are recorded as the agent's.

### Settings

- The Settings dialog is a preferences window: categories on the left, one category's rows on the right, sized to the screen like the Help dialog. Rows do not shift, the last category is remembered, and Esc and Done close it (#387). The six pitch labels are bound to their inputs (#393).

### Videos

- The agent fine-tune tutorial shows one real run: the prompt, the skill, a hit stepping by two samples at one zoom, and the per-track table (#380, #381).
