# S3 spike: the app service inside a CLAP plugin, wrapped as VST3

Ticket #506, umbrella #503 (the DAW plugin program). Throwaway spike:
`plugins/service-spike/`, a crate in its own folder, outside the workspace
members. No JUCE and no C++ of our own: the plugin is Rust on `clap-sys`; the
VST3 is `clap-wrapper` v0.17.0 wrapped around that CLAP (CLAP first, CPM).

## What the spike proves

- The app service runs inside a plugin as a normal crate. The plugin answers
  through the same `drumspike_app::call` dispatch; a call through the plugin's
  service answers the same JSON as a direct call (tests/boundary.rs).
- Same results as the desktop app: the event list the plugin writes for the
  synthetic fixture is byte for byte the one `service_tool analyze` prints
  (163 bytes, 5 events on 1 track; verified through the CLAP and through the
  wrapped VST3). The `service` stage states the event count and fails on 0 -
  two empty lists would also compare equal.
- A forced panic inside a plugin entry point is caught at the CLAP boundary:
  `init` answers false, `process` answers `CLAP_PROCESS_ERROR`, the host
  process lives (the test host survived 128 forced panics per run, exit 0).
- No analysis on the audio thread: the service job runs on its own thread;
  `process` only passes audio through. The test host processed blocks while the
  analysis ran: 0 overruns, worst block 80 µs of all 52266 blocks, each
  measured against its own 10667 µs budget (512 frames at 48 kHz). The check
  can fail: with `DS_SERVICE_SPIKE_FAULT=in-process` the analysis runs inside
  `process` and the host reports the overrun (1 block over budget, worst
  132309 µs, event file written).
- `clap-validator` 0.4.1 passes on the CLAP, `pluginval` strictness 5 passes on
  the wrapped VST3; both binaries are universal (x86_64 + arm64).

## Numbers

- One documented command builds both formats: `./spike.sh build` (the CLAP for
  both macOS targets, joined with lipo inside the `.clap` bundle; the wrapped
  VST3 from clap-wrapper). `./spike.sh check` runs every gate line of the
  ticket; `check.py all` prints one PASS/FAIL line per stage.
- Binary size: the CLAP binary is 8.5 MB (8,541,424 bytes, universal), the
  wrapped VST3 binary is 1.4 MB (1,363,608 bytes, universal). The VST3 is only
  the wrapper: the service code lives in the CLAP it loads at run time
  (`CLAP_PATH`), so the two files are not two copies of the service.
- Start time: `init` and `start_processing` answer immediately (no heavy work
  on the host thread - the job runs on its own thread). The service job
  answered the fixture analysis inside the plugin after 114 ms (CLAP host run)
  and 151 ms (pluginval run), measured by the plugin itself
  (`out/meta-clap.json`, `out/meta-vst3.json`): `analyze_projects` on the
  3 s 48 kHz fixture with one track and 163 bytes of event list.
- Reproducible fetches and builds: `clap-wrapper` pinned at v0.17.0,
  `clap-validator` pinned at tag 0.4.1 (built into its own `target` folder,
  where `deps` and `check.py` look for it), `pluginval` v1.0.4 with a pinned
  SHA-256 checked at download; `deps` installs both macOS Rust targets; the
  wrapper builds with `CMAKE_OSX_DEPLOYMENT_TARGET=12.0` (the current macOS
  SDK refuses the build without a deployment target).

## How it works

- `src/lib.rs`: the CLAP entry, the plugin, the guard. The service job starts
  in `init` on its own thread (the `DS_SERVICE_SPIKE_FAULT` switch runs it
  inside `process` on purpose - the overrun proof), writes the event list
  under a temp name and renames it (the rename is the completion signal; the
  meta file lands before the rename). Every entry point runs inside
  `catch_unwind`.
- `src/bin/host.rs`: the test host - dlopen, the `DS_SERVICE_SPIKE_*` test
  switch, audio blocks while the analysis runs, per-block budget check; the
  report carries the worst of all blocks.
- `check.py`: one stage per gate line (fixture, service reference, clap,
  fault, panic, vst3, validate, universal, standalone); `SPIKE: PASS` = all
  green.

## Open points

- The wrapped VST3 loads the CLAP at run time - and clap-wrapper can make the
  VST3 self-contained on macOS: the VST3 target takes
  `MACOS_EMBEDDED_CLAP_LOCATION`, stages the `.clap` into
  `<name>.vst3/Contents/PlugIns/`, and the wrapper searches its own bundle's
  `PlugIns` first (`cmake/shared_prologue.cmake` and
  `src/detail/clap/mac_helpers.mm` of v0.17.0). Verified: `pluginval` on a
  `.vst3` with the embedded `.clap` and no `CLAP_PATH` wrote the reference
  event list (134 ms, byte for byte). The embed is Apple-only in 0.17.0; on
  Windows and Linux the `.clap` still ships beside the wrapper. A product
  installs one `.vst3` bundle on macOS.
- The test switch is environment-based (`DS_SERVICE_SPIKE_*`): fine for a
  spike, not for a product.
- One analysis per process (a flag stops a second job): a real plugin ties the
  job to the project the host opens.
- The detector is DSP-only, so the models dir is unused in the plugin;
  `ServiceConfig::from_env(Path::new("/"))` stands in for a real config.
- pluginval ran at strictness 5, not higher. No ARA (S1), no user interface
  (S2).
- The spike crate pins its own `serde_json` (float_roundtrip,
  preserve_order) to keep the byte compare exact; a product build takes the
  app's lock instead.
