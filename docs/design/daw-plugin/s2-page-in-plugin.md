# S2 spike: the editor page runs in a plugin window

Ticket #505, umbrella #503 (the DAW plugin program). Throwaway spike:
`plugins/webview-spike/`, a crate in its own folder, outside the workspace
members. No JUCE (owner decision). The editor is a `wry` web view (WKWebView)
as a child of the host's window: a Rust CLAP plugin with the `clap.gui`
extension (cocoa, child of the host's NSView), wrapped as VST3 by
`clap-wrapper` v0.17.0. The page is a copy of `static/` inside the plugin
bundle, served by a `dspage://` scheme handler from the bundle files - no
network, no local server. `static/` itself is read, not changed. The page gets
a stub transport at document start (`src/page_stub.js`) that answers every
`backend.js` command from one recorded fixture project (8 tracks, 5 minutes,
48 kHz, one fixed seed - synthetic audio only).

## Result

The gate is `plugins/webview-spike/check_s2.sh`; it exits 0 on the PR head.
`./spike.sh build` builds the CLAP (both macOS targets, lipo inside the
bundle), the wrapped VST3 and the test host; `./spike.sh check` runs fmt,
clippy, the crate tests and the standalone rule.

| Check | Result on the test host |
| --- | --- |
| Boot: the page ready in the editor | PASS - ready in far under 20 s (sid=fixture-s2, 8 tracks, 1000x640) |
| 50 open/close cycles | PASS - 50 opens, ready every time, no crash |
| Frame times, 8 s of scroll and zoom (8 tracks, 5 min) | PASS - 418 frames, p95 19 ms, max 208 ms (a zoom redraw), at 1000x640 |
| Keys: Space, Right, Cmd+Z, Tab, Esc | PASS - all five reached the page and the page ran them (play/stop, nudge, undo, next hit, deselect); the page's transport reached the host: play, locate, stop |
| Focus back after a click in the host | PASS on the input path - the key after the host's click did not reach the page, the key after the click into the editor did (and the page called play). document.hasFocus() stayed false the whole run (see findings) |
| Resize | PASS - the page followed 1000 -> 1400 -> 760 px wide |
| Fault run (a page that never sends ready) | PASS - the host reported the missing ready after 20 s, exit 2 |
| Standalone rule | PASS - `cargo metadata --no-deps` and `cargo tree -p drumspike-desktop` do not name the spike crate; the diff touches only `plugins/webview-spike/` and `docs/design/daw-plugin/` |

| Host | Keys that reach the page / that the host takes | Focus back after a click in the host | Resize |
| --- | --- | --- | --- |
| Test host of the script (x86_64 Mac, macOS 15.8.2, run from a shell) | all five reach the page; the host itself takes none (it has no key handling of its own) | yes - the input returns (the key after the click into the editor reaches the page again); document.hasFocus() cannot turn true on this host | yes, the page follows the window |
| Reaper | not on this host - not measured | | |
| LUNA (filled by the coordinator after the merge) | | | |

## Findings (what does not work out of the box)

- A host run from a shell cannot activate: a minimal AppKit probe answers
  `isActive=false` and the window answers `isKeyWindow=false` forever
  (verified with 3 lines of Swift). `document.hasFocus()` therefore stays false
  the whole run - that is a property of the shell session, not of the page.
  The focus evidence is the input path instead: the host takes the first
  responder (its click in the host), the key then misses the page; the click
  into the editor gives the input back and the key reaches the page again.
- `CGEventPostToPid` is dropped without the accessibility trust (zero events
  arrived). The test host sends real `NSEvent`s through `[NSApp sendEvent:]`
  instead - the same path a DAW host uses after the window server hands the
  event over. The page cannot tell the difference.
- `wry`'s `with_focused` is a no-op on macOS: nothing makes the WKWebView the
  first responder. The plugin must call `makeFirstResponder` on it itself;
  without that no key reaches the page.
- The zoom redraws are the expensive frames. Under a robot drive (a wheel
  event every 60 ms, every third a zoom - 3.3 zooms/s) the p95 was 45 ms at
  1000x640. Under the human-scale drive of the gate (scroll 10x/s, zoom 2x/s)
  the p95 is 19 ms; a single zoom redraw peaked at 208 ms. On this 2010s Intel
  bot the 30 fps budget holds for human-scale interaction, not for the robot
  drive.
- Two CLAP ABI traps cost the wake: `clap_version_t` is a 12-byte struct
  (major, minor, revision) - a host that declares it one `u32` reads the
  revision as the `init` pointer and crashes; the window-api strings cross the
  dylib boundary - compare them by content, not by pointer.

## Licences of the new dependencies

The plugin brings 76 crates into its own build (the workspace target is not
touched; `cargo tree -p drumspike-desktop` does not name any of them). Exact
licence strings from the crates themselves:

| Crate | Version | Licence |
| --- | --- | --- |
| wry | 0.57.0 | Apache-2.0 OR MIT |
| clap-sys | 0.5.0 | MIT/Apache-2.0 |
| libloading | 0.8.9 | ISC |
| dpi | 0.1.2 | Apache-2.0 AND MIT |
| objc2 | 0.6.5 | MIT |
| objc2-encode | 4.1.0 | MIT |
| objc2-foundation | 0.3.2 | MIT |
| objc2-app-kit, objc2-web-kit, objc2-cloud-kit, objc2-core-data, objc2-core-foundation, objc2-core-graphics, objc2-core-image, objc2-core-text, objc2-core-video, objc2-quartz-core, objc2-exception-helper | 0.3.2 / 0.1.1 | Zlib OR Apache-2.0 OR MIT |
| block2 | 0.6.2 | MIT |
| raw-window-handle | 0.6.2 | MIT OR Apache-2.0 OR Zlib |
| url | 2.5.8, idna 1.1.0, percent-encoding 2.3.2, form_urlencoded 1.2.2, icu_* and the zerovec/yoke/zerofrom/zerotrie/tinystr/litemap/writeable/potential_utf/utf8_iter/utf_types family | MIT OR Apache-2.0; the icu4x crates and their friends: Unicode-3.0 |
| http 1.5.0, cookie 0.18.2, bytes 1.12.1, itoa 1.0.19, smallvec 1.16.5, once_cell 1.21.4, bitflags 2.13.2, cfg-if 1.0.5, stable_deref_trait 1.2.1 | | MIT OR Apache-2.0 (bytes, block2, itoa: MIT) |
| time 0.3.55, deranged, time-core, time-macros, num-conv, powerfmt | 0.3.x | MIT OR Apache-2.0 |
| thiserror 2.0.21, thiserror-impl | 2.0.21 | MIT OR Apache-2.0 |
| dirs 7.0.0, dirs-sys 0.5.0 | | MIT OR Apache-2.0 (option-ext 0.2.0: MPL-2.0) |
| libc 0.2.191 | | MIT OR Apache-2.0 |
| build-only: proc-macro2, quote, syn 3.0.10, unicode-ident, synstructure, displaydoc, cc, shlex, find-msvc-tools, version_check | | MIT OR Apache-2.0 (synstructure, block2, objc2: MIT; unicode-ident adds Unicode-3.0) |

The wrapper brings, fetched by CPM at build time: CLAP SDK 1.2.10 (MIT),
VST3 SDK v3.8.0_build_66 with its submodules (MIT; the VST3 SDK is MIT only
from 3.8.0), clap-wrapper v0.17.0 itself (MIT). The wrapped VST3 loads
the CLAP at run time through `CLAP_PATH`; the wrapper is not a second copy of
the spike.

## Recommendation

The `wry` web view works in the plugin window. The editor page of Drum Spike
boots from the bundle files without network or server, follows the host
transport, draws the 8-track/5-minute fixture at p95 19 ms at 1000x640 on this
Intel host, takes its keys, gives the input back after a click in the host and
survived 50 open/close cycles. The three things the product must take from the
spike: the plugin sets the first responder itself (wry does not do it on
macOS), the host compares the window-api strings by content, and the page's
transport comes through `B.hostTransport` exactly as P2/P3 merged it - the
stub transport is the only throwaway part. The `static/` page needs no change
for a plugin window.
