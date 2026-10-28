# ADR 0001: detector plugins as external processes

Date: 2026-10-25. Status: accepted (owner decision 2026-10-25).

## Context

The detector is one fixed code path: step 1 of the analysis, `detect_events` in
`crates/core/src/analyze.rs`, runs the built-in DSP detector
(`crates/core/src/detect/onset.rs`). Users and future work want other detectors, for
example a detector that uses a trained network. A network must not ship with Drum Spike
(owner rule: the app ships only the built-in detector, and a model-based detector lives
in its own repository under its own licence).

## Decision

The detector becomes pluggable through external process plugins. The seam is step 1 of
the analysis: a detector returns `(time, class, confidence)` per hit and, optionally, a
candidate list. A Rust trait `DetectorBackend` in `drumspike-core` has the built-in DSP
detector as the first implementation and the default, and an `ExternalDetector` adapter
as the second. A plugin is a folder with `manifest.json` and an executable that speaks a
line-based JSON protocol on stdin and stdout (protocol 1). The app starts the plugin
once per analysis job, validates every response, and applies a per-track timeout, so a
crash or a bad answer fails that track and never the app. Steps 2 to 6 and everything
after stay in Drum Spike, so the sample-exact placement, the velocities and the exports
are ours for every detector. The full protocol, the limits and the storage format are in
`docs/design/detector-plugins.md`.

## Consequences

- Sample-exact placement stays in Drum Spike: a plugin proposes hits, the app refines
  them to the attack and writes the MIDI and spike files.
- The plugin boundary is also the licence boundary: a detector that needs a trained
  network is installed by the user, never bundled, and its licence text is shown before
  the first use and recorded in the project.
- A project that names a missing plugin still opens with its stored hits; a re-analyze
  asks for the plugin or the built-in detector.
- Every detector change crosses a process boundary and a JSON encoding: the built-in
  detector keeps its in-process path and stays the default for speed and parity.
- The example plugin for tests is a small Python script under `tests/fixtures/plugins/`
  that returns hits from a JSON file.

## Rejected options

1. In-process plugins as dynamic libraries (`dlopen`): a plugin crash would take the
   app down, and a C ABI across a licence boundary drags the third-party code into our
   process and its licence questions into the binary.
2. A long-running plugin server that the app talks to over a socket: more state, a
   second lifetime to manage, and a network-shaped surface for a local, one-shot job;
   the per-job process keeps the isolation with none of that.
