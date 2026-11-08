# How this run was made

One real run of Claude Code on the demo session, driving a real Drum Spike window through the
skill and the `agent_call` tool. Nothing in `transcript.txt` was written by hand.

The run used main at a45f1c07 and a Mac with a window server. `<A>` is an empty scratch folder and `<H>` is the folder `home` inside it.

1. Build the binary and put it in an ad-hoc signed app bundle, so that macOS sends the app its
   open-document event. The bundle needs an `Info.plist` with `CFBundleDocumentTypes` for
   `.drumspike`.

       cargo build --release -p drumspike-desktop --bin drumspike-desktop
       codesign -s - --force <bundle>
       lsregister -f <bundle>

2. Make the project from two demo stems in `<A>/stems` (`k in.wav` and `s top.wav`, the demo session of
   `tools/media/record.py`, seed `demo`). Roles are kick and snare.

       uv run --no-project python tools/media/fixtures/agent-run/prep_demo.py <A> <binary>

3. Open the window on the scratch home, wait for `<H>/window-endpoint.json`, then open the
   project in that window.

       open -n --env DRUMSPIKE_HOME=<H> --env DRUMSPIKE_PROJECTS=<H>/projects -a <bundle>
       open -a <bundle> <the project document>

4. Install the skill into a scratch working folder.

       DRUMSPIKE_SKILL_SOURCE=skills/drumspike-finetune <binary> cli agent_skill_install \
         --target <A>/work/.claude/skills/drumspike-finetune

5. Write `<A>/mcp.json` with one server, `drumspike`: command `<binary>`, args `["mcp"]`, env
   `DRUMSPIKE_HOME=<H>` and `DRUMSPIKE_PROJECTS=<H>/projects`.

6. Run the agent from `<A>/work`. `prompt.txt` is the prompt, unchanged.

       claude -p "$(cat prompt.txt)" --model sonnet --setting-sources project \
         --mcp-config <A>/mcp.json --strict-mcp-config \
         --allowedTools "Skill,Write,mcp__drumspike__agent_call,Bash(sleep *)" \
         --output-format stream-json --verbose < /dev/null > <A>/out.jsonl

7. Build the fixture files.

       uv run --no-project python tools/media/fixtures/agent-run/build_fixture.py \
         <A>/out.jsonl <A> tools/media/fixtures/agent-run

The run took 60 s. No tool call was denied.

## What was changed in the transcript

- 4 path strings were replaced by `<home>`.
- 55 tool names were shortened: the prefix `mcp__drumspike__` is dropped.
- Long lines of the agent's text are wrapped at 94 characters. Table lines are not wrapped.
- 0 lines were removed.
- `move` lines carry the from and to positions of the tool's answer. The other answers (the
  samples of a look, the hit lists) are not shown.

`actions.jsonl` holds the 12 moves and 18 confirms from the tool answers.
