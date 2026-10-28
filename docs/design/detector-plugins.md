# Design: detector plugins as external processes

Status: implemented. Owner decision 2026-10-25: the detector becomes pluggable,
with external process plugins. This document is the exact design; the implementation
tickets at the end follow from it.

## 1. The seam

The seam is step 1 of the analysis, `detect_events` in `crates/core/src/analyze.rs`
(the DSP detector runs in `crates/core/src/detect/onset.rs`). A detector returns the hits
of one track as `(time, class, confidence)` and, optionally, a candidate list. The
candidate list feeds the Sensitivity slider, the Inspector histograms and Learn, exactly
as the kept candidates of the built-in detector do today (`Detector::Candidates` in
`analyze.rs`, `repick_candidates` in `onset.rs`).

Everything after step 1 stays in Drum Spike and runs on any detector's hits:

- steps 2 to 6 of `analyze_flagged` in `analyze.rs`: the role filter (`role_classes`),
  the sort and min-gap dedup, the first-sample refinement (`refine_track`), the global
  shift and the velocities (`velocity_map` in `velocity.rs`);
- everything after: the tom bleed rules, the flam rule, tempo, and the exports.

So the sample-exact placement stays ours: a plugin proposes coarse hit times, Drum Spike
refines them to the attack, sets the velocities and writes the MIDI and spike files.

## 2. Inside the code

A Rust trait in `drumspike-core`, `DetectorBackend`:

```rust
pub struct DetectedHit { pub time: f64, pub class: u8, pub confidence: f32 }
pub struct CandidateList { pub hits: Vec<DetectedHit> }

pub trait DetectorBackend {
    fn detect(&self, x: &[f32], sr: u32, role: &str)
        -> Result<(Vec<DetectedHit>, Option<CandidateList>), DetectError>;
}
```

One hit type everywhere: `DetectedHit`, in the return value and in the candidate
list. The protocol's string `class` (section 4) is validated and mapped to the
internal class index (`u8`, the classes of `N_CLASSES` in `crates/core/src/detect`)
before a hit enters the pipeline; unknown classes are dropped by the validator.

Two implementations:

1. `DspBackend`, the built-in DSP detector. It is the first implementation and the
   default. It wraps `detect_onsets` and the candidate machinery that
   `Detector::Dsp` and `Detector::Candidates` use today.
2. `ExternalDetector`, the adapter for one installed plugin. It starts the plugin
   executable, speaks protocol 1 (section 4) and validates the answer (section 5).

`detect_events` keeps its signature and dispatches on the selected backend. The app side
(`crates/app/src/jobs.rs`) learns a `DetectorSlot::Plugin { id }` next to the existing
`Kind::Dsp`; `detector_record` writes the storage record of section 6.

## 3. Plugin folder and manifest

A plugin is one folder per plugin under
`~/Library/Application Support/Drum Spike/Detectors/<id>/`. Plugins are macOS only for
now. The folder holds the manifest and the executable. The user
installs a plugin by copying the folder there; the app never downloads a plugin.

`manifest.json`, one object:

| Field | Type | Rule |
|---|---|---|
| `id` | string | Lowercase letters, digits and `-`, at most 40 characters. Same as the folder name. |
| `name` | string | The name the UI shows, at most 60 characters. |
| `version` | string | Semver-like `major.minor.patch`. |
| `protocol` | integer | The protocol major version the executable speaks. Protocol 1 only in this design. |
| `executable` | string | Path relative to the folder. One file, executable bit or `.exe`. |
| `roles` | array of string | Subset of `kick`, `snare`, `tom`, `hat`, `cymbal`, `other`. The app greys the plugin out for other roles. |
| `sample_rates` | array of integer | The rates the plugin accepts, from 44100, 48000, 88200, 96000, 192000. The app passes the file's rate; if the plugin does not list it, the app resamples the track to the plugin's first listed rate (`crates/core/src/detect/resample.rs`), writes that WAV and passes that rate. Times are in seconds, so nothing else changes. A plugin may list any subset of the five rates. |
| `licence` | string | The licence name, for example `MIT`. |
| `licence_file` | string | Path relative to the folder of the full licence text. |
| `homepage` | string | URL, informational. |

Unknown manifest fields are ignored. A manifest that misses a required field, or whose
executable does not exist, makes the plugin invisible in the UI and logs the reason.

## 4. Protocol 1

The app starts the plugin executable once per analysis job (not per track). For each
track it writes one JSON request line on the plugin's stdin and reads one JSON response
line from the plugin's stdout. The plugin's stderr goes to the analysis log.

Request, one line, UTF-8 JSON:

| Field | Type | Unit or rule |
|---|---|---|
| `protocol` | integer | `1`. |
| `wav` | string | Absolute path of the decoded mono WAV file for this track. The app wrote it; the plugin reads it. |
| `channel` | integer | 0-based index of the source channel the file holds. Always 0 in protocol 1 (the app decodes mono). |
| `role` | string | One of `kick`, `snare`, `tom`, `hat`, `cymbal`, `other`. |
| `sr` | integer | Hz. The sample rate of the WAV file: the file's own rate, or the resampled rate of section 3 when the app resampled. |
| `params` | object | Free-form detector parameters, for example `{"sensitivity": 0.5}`. The app passes the track's stored params of the plugin, or an empty object. |

Response, one line, UTF-8 JSON, either a hit set or an error:

| Field | Type | Unit or rule |
|---|---|---|
| `hits` | array of object | See the hit table. At most 50 hits per second of audio, at most 100000 hits. |
| `candidates` | optional array of object | The same shape as `hits`; the candidate list for the Sensitivity slider, the histograms and Learn. At most 400 candidates per second of audio, at most 1000000 candidates. May be omitted. |

Each object in `hits` and `candidates`:

| Field | Type | Unit or rule |
|---|---|---|
| `t` | number | Seconds from the start of the file. Finite, `0 <= t <= file length`. |
| `class` | string | One of `kick`, `snare`, `tom`, `hat`, `cymbal`, `other` (the classes of `N_CLASSES` in `crates/core/src/detect`). |
| `confidence` | number | 0.0 to 1.0. Optional; a missing confidence is 1.0. |

Or, instead of `hits`: `{"error": "<one readable sentence>"}`. The app shows that
sentence as the track's analysis error.

Version rule: the request names `protocol: 1`. If the plugin answers with a response
whose protocol major is not 1, or the manifest names a major other than 1, the app
refuses the plugin for that job with a clear message: "plugin <id> speaks protocol <n>,
this app speaks protocol 1". A different minor is compatible and runs.

## 5. Validation, limits, errors

The app validates every response before it uses it:

- every `t` is finite and inside the file length; out-of-range hits are dropped, and the
  count of dropped hits goes to the analysis log;
- the hits are sorted by time; if not, the app sorts them (the min-gap dedup needs the
  order);
- every `class` is a known class; unknown classes are dropped and logged;
- `confidence` is clamped to 0.0 to 1.0;
- at most 50 hits per second of audio (hits) and 400 candidates per second
  (candidates); over that, the app fails the track with a readable error instead of
  guessing which hits to keep.

Limits and timeouts:

| Item | Value |
|---|---|
| Timeout per track | 120 s default (a plugin param `timeout_s` can lower it, not raise it). On timeout the process is killed and the track fails. |
| Timeout per job | The sum of the per-track timeouts. |
| Hits per second | 50. |
| Candidates per second | 400. |
| Hits per track | 100000. |
| Candidates per track | 1000000. |
| Response line length | 64 MiB. |
| Manifest size | 1 MiB. |

Error cases, each a readable track error that never takes the app down:

| Case | Result |
|---|---|
| Executable missing or not runnable | Plugin not offered; if a project names it, see section 6. |
| Manifest invalid | Plugin invisible, reason in the log. |
| Protocol major mismatch | Refused with the message of section 4. |
| Non-zero exit, crash, no response line, more than one response line per request | Track fails with the stderr tail (at most 20 lines) in the error. |
| Timeout | Process killed, track fails. |
| Invalid JSON | Track fails. |
| Limit exceeded | Track fails. |

A plugin crash or a bad answer fails that track only. The job continues with the other
tracks; the app never crashes. No network access is promised or needed; the plugin gets
the WAV path, not a URL.

Security: the plugin executable runs with the user's rights, so the licence dialog of
section 8 names the executable path together with the licence before the first run, and
the user accepts that exact path. The app never removes the macOS quarantine attribute
from a plugin; a Gatekeeper-blocked plugin fails with a readable error that says how the
user can allow it. The app is not sandboxed (no App Sandbox in `entitlements.plist`), so
spawning a user-installed executable works.

Learn works on plugin hits exactly as on built-in hits: the shape score (B8) and the
ghost gate features come from the audio, computed app-side at the candidate times. A
plugin never computes or returns them.

Before the first use of a plugin the app shows its executable path, its licence name and
the text of `licence_file`, and the user accepts. The acceptance, the licence name and a
hash of the licence text are recorded in the project (section 6).

## 6. Project format

The detector of a track keeps its current record fields: `detector` (the opaque detector
string), `chain` and `refine`. A plugin track adds one optional field:

- `plugin`: `{"id": "<id>", "version": "<manifest version>", "licence_sha256": "<hex>"}`.

The SHA-256 is the hash of the licence text the user accepted (section 5). Project
format 1 stays loadable: an old app opens a new project and ignores the extra `plugin`
field, and a project without the field is a built-in-detector project.

A project that names a missing plugin (folder gone, manifest invalid) opens and shows
the stored hits. A re-analyze of that track asks: install the plugin again, or analyze
with the built-in detector.

## 7. UI and agent API

- Inspector, Detection section (`buildParamsPanel` in `static/app.js`): a Detector
  select above the Sensitivity row. First entry `Built-in`, then one entry per installed
  plugin that supports the track's role and rate. Changing it applies on the next
  re-analyze, like the other detection settings. A follower track has no Detector
  select (it has no own detection).
- Preferences: the installed plugins, each with id, name, version, roles, and the
  licence name and text; the Accept-before-first-use dialog of section 5.
- CLI and MCP: `--detector <id>` on the analyze commands; `builtin` or a missing flag
  selects the built-in detector.
- Example plugin, tests only: `tests/fixtures/plugins/json_hits.py`, a small Python
  script that reads hit times from a JSON file next to it and prints a protocol 1
  response. No model, no network. It is never bundled with the app.

## 8. Licence boundary

Drum Spike ships only the built-in detector. A detector plugin that uses a trained
network lives in its own repository, is installed by the user by hand, is never bundled
with Drum Spike, and is not published without the rights holders' clearance. The
licence text of every installed plugin is shown before the first use and recorded in the
project, per section 5 and 6.

## 9. Implementation tickets

Each with its Gate line:

1. Core: `DetectorBackend` trait, `DspBackend`, the protocol 1 types and the response
   validator, and the `ExternalDetector` adapter, with the example plugin in
   `tests/fixtures/plugins/` and unit tests for the validator and the adapter against
   the example plugin.
   Gate: `cargo test -p drumspike-core` passes; the validator tests cover every error
   case of section 5; a golden-file test shows the default analysis output is
   byte-identical before and after the trait refactor (the frozen references in
   `tests/golden/`).
2. App: plugin discovery (the folder scan, the manifest reader), the Inspector Detector
   select, the Preferences page with the licence dialog, the project format of section 6
   (read and write), and the missing-plugin path.
   Gate: `cargo test -p drumspike-app` passes; a browser test opens a project with a
   missing plugin and re-analyzes with the built-in detector.
3. CLI and MCP: the `--detector <id>` flag through `crates/app/src/cli.rs` and
   `mcp.rs`.
   Gate: the CLI runs an analysis with the example plugin against a fixture WAV; the
   MCP schema names the flag.
4. Docs: the user guide page (install a plugin, the licence dialog, the Detector
   select) and the `docs/agent-api.md` entry.
   Gate: `uv run pytest tests/test_docs_wording.py` passes.

Ticket 1 is in `crates/core` and can start at once; tickets 2 and 3 touch
`crates/app` and the page; ticket 4 is docs.
