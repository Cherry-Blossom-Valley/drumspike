# Research: a native app UI and an optional ARA plugin (#123)

Owner request (2026-10-29): can a fully native app (no web technology) improve the Drum Spike
experience, and what would it take to make Drum Spike optionally an ARA plugin? This report
measures the web UI first (counted numbers, no estimates), compares the UI options, and maps
the ARA path. Sources are primary docs and SDK licences; the two deep-dive research notes this
report condenses are the agent's research notes (agent workspace, this Mac, 2026-10-30).

## 1. Sources

UI toolkits and licences:

| # | Source | URL |
|---|--------|-----|
| S1 | egui licence and AccessKit integration | https://github.com/emilk/egui , https://accesskit.dev/looking-back-looking-forward/ |
| S2 | iced licence, releases, accessibility status | https://github.com/iced-rs/iced/releases , https://github.com/plushie-ui/plushie-iced |
| S3 | Slint triple licence (GPL-3.0 / Royalty-free / Commercial) | https://github.com/slint-ui/slint/blob/master/LICENSE.md |
| S4 | Vizia licence and status | https://github.com/vizia/vizia |
| S5 | Qt LGPLv3 obligations and App Store conflict | https://www.qt.io/development/open-source-lgpl-obligations |
| S6 | JUCE licence (AGPLv3 or commercial) | https://github.com/juce-framework/JUCE/blob/master/LICENSE.md |
| S7 | UniFFI (Rust to Swift) | https://mozilla.github.io/uniffi-rs/ |
| S8 | Tauri transparent window / macos-private-api | https://v2.tauri.app/reference/javascript/api/namespacewebview/ |
| S9 | wgpu inside a Tauri macOS window (worked example) | https://github.com/clearlysid/tauri-wgpu-cam |
| S10 | Tauri layer/flicker issues | https://github.com/tauri-apps/tauri/issues/9220 , /issues/13415 |

ARA and plugin SDKs:

| # | Source | URL |
|---|--------|-----|
| S11 | ARA SDK (Apache-2.0 since 2021) | https://github.com/Celemony/ARA_SDK |
| S12 | ARA API headers (vtables, document model) | https://github.com/Celemony/ARA_API |
| S13 | Pro Tools ARA 2 supported plugins and conditions | https://kb.avid.com/pkb/articles/compatibility/Pro-Tools-ARA-2-Supported-Plugins |
| S14 | Avid Alliance Partner Program (AAX SDK access) | https://www.avid.com/alliance-partner-program/how-to-apply |
| S15 | PACE AAX code signing for Pro Tools | https://paceap.com/getting-started-with-aax-code-signing-for-pro-tools-plugins |
| S16 | VST 3.8.0: relicensed to MIT (2025-10-20) | https://steinbergmedia.github.io/vst3_dev_portal/pages/VST+3+Licensing/VST3+License.html |
| S17 | Apple AudioUnitSDK (free) | https://github.com/apple/AudioUnitSDK |
| S18 | CLAP (MIT); CLAP is an ARA full citizen (2024-07) | https://github.com/free-audio/clap , https://cleveraudio.org/news/july-2024/ |
| S19 | nice-plug (ISC, nih-plug successor), VST3+CLAP | https://crates.io/crates/nice-plug |
| S20 | ara-bridge (partial Rust ARA bindings, MIT) | https://crates.io/crates/ara-bridge |
| S21 | pluginval, auval | https://github.com/Tracktion/pluginval |
| S22 | macOS signing/notarization for audio plugins | https://melatonin.dev/blog/how-to-code-sign-and-notarize-macos-audio-plugins-in-ci/ |
| S23 | Logic Pro ARA 2 (Apple, Rosetta condition) | https://support.apple.com/guide/logicpro/support-for-ara-2-compatible-plug-ins-lgcp58ce340b/10.7/mac/11.0 |

Licence checks against Drum Spike's Apache-2.0 were made from the licence files at the URLs
above, retrieved 2026-10-30. VST3 3.8 MIT and nice-plug were re-verified directly this session.

## 2. Measured web UI baseline (counted, this host, this session)

Environment: this Mac (macOS 27.0.1, arm64), ds-bridge debug build of this tree (built 36.1 s,
`cargo build -p drumspike-devtools --bin ds-bridge`, rc=0), Chrome driven over CDP, the
licence-clean demo session (tools/media/demo_session.py, 13 tracks, 64.3 s at 48 kHz,
3,086,408 samples per track) as a 13-track project. Two caveats are counted, not estimated
away: (a) this browser renders GL through SwiftShader (software) and its rAF cadence is
throttled to ~10 Hz (idle frame p50 107.5 ms, p95 149.2 ms, n=60) — so wall-clock frame
times here measure the harness display, not the app; (b) the bridge speaks HTTP, so IPC
numbers are upper bounds for the Tauri IPC path (binary payloads either way).

| Metric | Counted value |
|--------|---------------|
| Full synchronous redraw (redrawAll, 13 tracks, JS main thread) | p50 0.10 ms, p95 0.30 ms (n=30 per zoom level; identical at 30 s / 5 s / 500 ms / 50 ms / 5 ms view) |
| GL lane engine, internal draw timer (SwiftShader, worst of session) | max 0.9 ms per frame; JS-side orchestration dominates (the wave lanes are GPU draws) |
| rAF cadence in this harness (idle, no interaction) | p50 107.5 ms ≈ 9.3 fps — harness-bound; on a real 60 Hz display the app's ~0.1 ms redraw leaves >99% of the frame budget |
| IPC: peaks_tile, finest level (256-sample buckets, 12,056 bins, 141 KB) | p50 1.8 ms, max 3.3 ms (n=10) |
| IPC: samples_window, 62 s asked | server clamps to 262,144 samples (1,048,584 bytes): p50 2.4 ms, max 3.0 ms (n=10) |
| IPC: all 13 tracks × all peak levels (initial paint path) | 55 ms total |
| Hit edit input latency (events_edit velocity round trip + drawHits) | IPC p50 2.8 ms (max 20.1 ms, n=20) + draw 0.10 ms — well under the 16 ms frame budget |
| JS heap after load + full analysis | 11.6 MB used / 13.9 MB total |
| Bridge process RSS (debug, after analyzing 13 tracks) | 56 MB before analysis, 274 MB after (debug build, detector included) |
| Start time (warm bridge, page reload: navigationStart to S.loaded>0) | ~901 ms; DOMContentLoaded 24.8 ms |
| Bundle size | static/ 824 KB (unminified, no bundler); target/release/drumspike-desktop 16,412,720 B (15.7 MiB, macOS arm64); debug ds-bridge 18,847,608 B |

Reading: the hot paths are healthy. A full redraw costs ~0.1 ms of JS and ≤0.9 ms of GPU
even in software rendering; IPC payloads are binary and 1–3 ms per call over HTTP. The web
UI is not the bottleneck — any move to a native UI buys feel and platform integration, not
frame time.

## 3. Decision table: UI options

Effort S/M/L/XL against the current Tauri 2 shell; every option keeps the Rust core
(crates/core, crates/app) and names its binding layer.

| Option | Licence (and Apache-2.0 fit) | Platform reach | Accessibility | Waveform fit | Gain | Effort | Risk |
|--------|------------------------------|----------------|---------------|--------------|------|--------|------|
| Middle path: keep web UI, native waveform overlay (wgpu → Metal under a transparent WKWebView) | Existing stack, Apache-2.0 unchanged | macOS now (macos-private-api), Windows later (WebView2) | Web ARIA unchanged; overlay needs its own | Best of both: wgpu compute decimation, WebGL fallback stays | 60 fps pan/zoom headroom with no UI rewrite; keeps e2e suite | S–M | Medium: layer ordering, flicker (tauri#9220), private API blocks the Mac App Store; reversible |
| egui (Rust, immediate mode) | MIT OR Apache-2.0 — pick Apache-2.0 | macOS/Windows/Linux + WASM | Good: AccessKit (NSAccessibility/UIA/AT-SPI) | Good: custom PaintCallback over egui-wgpu | One language, no FFI, cross-platform | M | Pre-1.0 churn; immediate-mode rewrite of a dense editor |
| SwiftUI / AppKit + Metal | Apple SDKs (proprietary, not a library licence); our code stays Apache-2.0 | macOS only | Best (native) | Best (Metal) | Native feel, menus, keys, drag-and-drop | XL | No Windows path without a second UI; binding via UniFFI (S7) or cbindgen C ABI |
| iced (Rust, Elm-style) | MIT | macOS/Windows/Linux + WASM | Gap: AccessKit not upstream (0.14) | Good (iced-wgpu) | Clean architecture, Rust-native | M–L | Accessibility is a ship blocker today |
| Slint | GPL-3.0-only OR Royalty-free (attribution) OR Commercial | macOS/Windows/Linux + embedded | Excellent (AccessKit built in) | Good (Skia/wgpu) | Mature declarative UI | L | Licence trap: crates.io default is GPL-3.0-only; legal review mandatory |
| Vizia | MIT | macOS/Windows/Linux | Partial (feature flag, docs.rs builds failing) | Good (Skia, audio-plugin focus) | Audio orientation | M–L | Pre-1.0, tiny ecosystem |
| Qt (Widgets/QML) | LGPL-3.0-only or Commercial (~$3.5k/dev/yr) | All desktop | Excellent | Excellent (used by DAWs) | Pro-audio precedent | XL | LGPLv3 vs App Store (re-link rule), C++/Rust friction via cxx/C ABI |
| JUCE | AGPLv3 or Commercial ($3.5k/seat) | All desktop + iOS/Android | Good | Best-in-class audio | Plugin formats out of the box | XL | AGPLv3 viral; C++ lock-in; overkill for the standalone app |

## 4. ARA 2: what it gives, who supports it

ARA is an extension of the plugin formats (VST3, AU, AAX, CLAP): the plugin gets random
access to the host's audio regions (faster than real time), tempo/chord/region metadata, undo
integration, and persistence inside the host project (`storeObjectsToArchive`). For Drum
Spike that means: analyze the DAW's clips in place, show spikes in the host window, export
MIDI/spikes without any file round trip. Analysis-only scope (read regions, produce markers)
is fully covered by ARA 2; no audio modification is needed (S11, S12).

Host and format matrix (Pro Tools first — the owner's main host):

| DAW | ARA 2 | Format(s) | Conditions |
|-----|-------|-----------|------------|
| Pro Tools 2022.9+ | ✅ | **AAX only** (separate AAX-ARA plugin) | Avid developer approval, Pro Tools Developer build for unsigned testing, PACE/Eden signing + iLok for release (S13–S15). ✅ checked |
| Cubase / Nuendo 10+ | ✅ | VST3 | Extensions menu; Elements excluded |
| Studio One 4+ | ✅ | VST3 | Event FX workflow |
| REAPER 5.97+ | ✅ | VST3, CLAP (6.71+) | Best-documented ARA support; no signing gate — the natural dev/test host |
| Logic Pro 10.4+ | ✅ | AU | Apple Silicon: ARA plugins run out-of-process; ARA in-process needs Rosetta 2 (S23) |
| Digital Performer 11.2+ | ✅ | VST3, AU, AAX | |
| Bitwig 5+ | ✅ | VST3, CLAP | |
| Ableton Live 12.x | ❌ | — | Not implemented |
| FL Studio | ❌ | — | "Planned for the future" only |

SDK licences against Apache-2.0:

| SDK | Licence | Fit |
|-----|---------|-----|
| ARA SDK | Apache-2.0 (S11) | ✅ clean |
| VST3 SDK | MIT since 3.8.0 (2025-10; previously GPLv3 or proprietary) (S16) | ✅ clean on 3.8+ |
| CLAP | MIT (S18) | ✅ clean |
| AU SDK | free, Apple (S17) | ✅ clean; auval required |
| AAX SDK | proprietary, NDA, PACE/Eden signing (S14, S15) | ⚠️ our code stays Apache-2.0, but the build/release pipeline is closed and gated on Avid approval |

Frameworks and bindings: JUCE has ARA support but its AGPLv3/commercial split makes it the
wrong default for an Apache-2.0 project — usable only with a commercial licence or if the
JUCE-based wrapper is a separate closed component. In Rust, nice-plug (ISC, the maintained
nih-plug successor) covers VST3+CLAP but has no ARA; ara-bridge implements only 25 of 55
vtable entries. The pragmatic path is a thin C++ layer over the ARA_Library (Apache-2.0)
that calls the Rust core through a C ABI (`cbindgen`-generated header, `catch_unwind` at the
boundary, no panics, allocations or locks on the audio thread).

Architecture changes for the plugin shape: the core becomes a `cdylib` with a C ABI inside
the host process; the document model maps ARA `AudioSource`/`PlaybackRegion` to Drum Spike's
tracks/regions, hits persist through the ARA archive (versioned schema); the plugin UI lives
in the host's lower zone (this is where part 1's UI decision reappears — the same Rust UI or
web view must fit a host-provided parent window); export writes MIDI/spikes from inside the
DAW. Distribution: hardened-runtime codesign + notarization (macOS), pluginval level 5 +
auval in CI, AAX additionally gated on PACE signing.

## 5. Recommended path and first spike

Recommendation: do not rewrite the UI. The measured baseline shows the web UI has an order of
magnitude of frame-budget headroom; a native rewrite buys platform feel at effort XL and
splits the codebase. Take the middle path (effort S–M): keep the Tauri 2 shell and the web
UI, and move only the hot waveform drawing to a native wgpu surface (Metal on macOS) behind
the transparent-webview pattern (S8, S9), keeping lanes-gl.js as the fallback. For ARA, start
with the open formats: Rust core as `cdylib` + C ABI, a thin C++ ARA_Library wrapper shipped
as VST3 (MIT SDK) and CLAP (MIT), developed against REAPER (no signing gate), with Pro Tools
AAX-ARA as a second phase after Avid developer approval — its conditions (NDA SDK, PACE
signing) make it the wrong first target even though it is the owner's main host.

First spike (can fail): a minimal ARA 2 VST3 plugin — C++ ARA_Library document controller
over a `drumspike_core` cdylib exposing `detect_hits(buffer, sr) -> hits` through a C ABI —
that (1) lists the region names of the open session in its plugin UI and (2) draws one
waveform of the first region from host audio via `ARAAudioAccessController`, in REAPER.

Pass criterion: in REAPER, the plugin lists the correct region names of the demo session and
renders a waveform that visually matches the region's peaks; the hit list for one region
matches the desktop app's analysis of the same WAV (same events, tolerance ±10 ms); pluginval
level 5 passes. Fail criterion: host refuses the ARA document controller, the vtable wiring
cannot be completed without forking ARA_Library, or the core cannot be called from the
document-controller thread without panics — any of these sends the report back to the ARA
question with evidence instead of a product.

## 6. What the user would gain, and the cost

Gain of a full native app (argued, per the table in §3): native menus/keys/drag-and-drop,
system audio device handling, VoiceOver-grade accessibility on macOS, and headroom for
60 fps pan/zoom of very dense projects. Measured cost of the status quo: none of the hot
paths are close to a budget (§2) — the strongest argument for a rewrite is feel, not speed.
Cost of a rewrite: a second UI (or a replaced one), migration of the whole static/ feature
set, a reworked e2e suite (pytest e2e_app drives the web page), and — for the ARA plugin —
the same UI living inside a host window. The middle path buys the only measured-relevant
upgrade (drawing headroom) at S–M effort and keeps everything else.
