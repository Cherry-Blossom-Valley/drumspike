# LUNA: what the first run showed

Ticket #552, umbrella #503 (the DAW plugin program), part of M1 (#510). The plugin lives in
`plugins/ara/`. This note keeps what we know about LUNA from the first run of the plugin there,
on 2026-11-20. It has facts only. The results of the spikes are in `s1-ara-audio.md` and
`s2-page-in-plugin.md`.

## Facts

- LUNA reads VST3 plugins only from the system VST3 folder. It does not read the user folder.
- LUNA does not scan a VST3 that is new to it by itself. It lists the plugin as new. It scans a
  known plugin again when the date of the bundle changed.
- LUNA gives an audio source a name of 32 hex characters. The name of the DAW track is in the
  properties of the region sequence, and the name of the clip in the properties of the playback
  region. The plugin shows them since #558 (`m1b10-track-names.md`).
- LUNA logs `VST3_PluginInstance <plugin> ::loadPreset failed` (level ERROR) each time the user
  chooses the plugin for a track. See "loadPreset failed" below. It is not fixed.

## loadPreset failed (#558)

What we measured:

- The first run (2026-11-20) logged the message three times. The run with the build of #554
  (2026-11-21, with `clap.state` since #545) logged it once, right after the user chose the plugin.
  So `clap.state` did not remove it. The LUNA logs of that Mac name no other plugin with this message
  (20 logs); they also hold no `loadPreset success` line for any plugin, so the logs give no
  comparison with another ARA plugin.
- The message comes from the class of LUNA that wraps a VST3 instance. The strings of LUNA's engine
  library put it next to the preset code of LUNA (`Preset_LoadPreset`, `ReadEffectPresetState`). The
  engine library has no symbols, so the VST3 calls behind it are not known. Nothing of this was
  confirmed by a debugger or by the source of LUNA.
- The message comes before the page opens, in the same second as the ARA calls of the instance
  (bind, restore of the archive). Those calls worked: the report of the run has the restore, the analysis
  rounds with the result `done`, the hits and the page. We saw no missing function after the message.
- The VST3 path of the plugin is the wrapper of clap-wrapper 0.17.0. It maps `setState` and
  `getState` to `clap.state` (a load that answers false is the only way the wrapper answers false
  there). The wrapper makes a program list and a preset selector only when the plugin has
  `clap.preset-load`; the plugin has none. The plugin has no parameter, either.
- The plugin answers true for its own blob, an empty stream, a newer version and garbage (stage
  `state`, on the CLAP). No tool in `.deps` calls `getState` and `setState` on the wrapped VST3
  alone, so the VST3 side of that answer is not measured.

What we think, and what we do not know. The message is LUNA's own preset path for an instance that
has no stored preset of LUNA. It does not touch the ARA part, and nothing of the plugin was lost
after it. This is a conclusion from the points above, not a proof. We do not know whether LUNA
calls `getState` or `setState` of the plugin at all, and we do not know whether the missing
parameters or the missing program list make the call fail.

What #558 adds to find out: the report has a row `state_call` for every call of the host to
`clap.state` (kind, bytes, answer), and the stage `state` checks the rows. The next run in LUNA
says if the state is called when the message comes. If LUNA calls it and the row shows a true
answer, the message is not about the state; the next candidates are a parameter or a program
list, which the wrapper offers only for a plugin that has them.

## Open points

- How a normal install gets the first scan. The installer of M2 must solve this.
- Whether LUNA sends a transport to a plugin that is used through ARA only.
- What LUNA sends as the selection.
