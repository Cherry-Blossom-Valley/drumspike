# LUNA: what the first run showed

Ticket #552, umbrella #503 (the DAW plugin program), part of M1 (#510). The plugin lives in
`plugins/ara/`. This note keeps what we know about LUNA from the first run of the plugin there,
on 2026-11-20. It has facts only. The results of the spikes are in `s1-ara-audio.md` and
`s2-page-in-plugin.md`.

## Facts

- LUNA reads VST3 plugins only from the system VST3 folder. It does not read the user folder.
- LUNA does not scan a VST3 that is new to it by itself. It lists the plugin as new. It scans a
  known plugin again when the date of the bundle changed.
- LUNA called the state of a new instance and logged an error, because the plugin had no
  `clap.state` yet. The plugin has it since #545.

## Open points

- How a normal install gets the first scan. The installer of M2 must solve this.
- Whether LUNA sends a transport to a plugin that is used through ARA only.
- What LUNA sends as the selection.
