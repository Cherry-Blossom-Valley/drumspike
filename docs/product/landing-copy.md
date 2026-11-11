# Landing page copy

Text of `site/index.html` (the byte-identical mirror of the Cbvsite source,
`public/tools/drum-spike/index.html`). This file states what the page says, section by
section, so the copy source and the page stay the same. Brought to the page in #467
(the 0.9.33 wording audit); the page's earlier source was ticket "Media 6". Keep it short;
the videos and the guide carry the detail. Names and keys as in the app. No claim without a
source in the app, the guide or a release note.

## Hero

- Title line: Every drum hit, on the **first sample**.
- Sub: Drum Spike finds every hit in your close-mic drum tracks, puts each one on the first
  sample of its attack, and exports MIDI and spike files that trigger on time.
- Badges: Free and open source; Apache-2.0 source code; macOS, Apple silicon and Intel.
- Buttons: "Download for Mac - Apple silicon" (M1 and later) and "Download for Mac - Intel"
  (x86-64), both to the latest release.
- Fine print: Latest release, signed and notarized by Apple, release notes and checksums.
- Which one: Apple menu > About This Mac: "Chip: Apple M..." is Apple silicon,
  "Processor: Intel" is Intel.
- Figure: a synthetic snare onset; a level threshold fires late, Drum Spike marks the first
  sample of the attack. (Owner 2026-10-23: the page states macOS only; the app enforces no
  minimum. There is no tour button; the product-tour video slot is a comment in the page,
  issue #11.)

## What Drum Spike does (three cards)

1. Finds the real hits. Drum Spike listens to each close mic for its own drum only. It checks
   which mic heard a hit first and how the hit sounds: a kick in the snare mic is bleed, not a
   snare hit.
2. Places them on the first sample. Each hit moves to the first motion of its attack. A soft
   ghost note and a loud backbeat get the same point.
3. Exports MIDI and spike files. Per track, on your session's time. Each MIDI note sits on the
   sample of its hit: one tick is one sample where the MIDI standard allows it. Spike type
   Audio writes a short piece of the track's own audio at each hit: the body of the hit, not a
   click.

## Why it matters

- Heading: One millisecond is audible.
- Text: A trigger that fires one millisecond late against the close mic cuts holes into the
  sound at 500 Hz, 1.5 kHz and 2.5 kHz, right where a snare has its body and crack.
  Level-based transient detection fires late on soft hits and early on loud ones, and it fires
  on bleed. Drum Spike puts every hit on the first motion of its attack, soft or loud.
- Facts: 48 samples in one millisecond at 48 kHz; 34 cm sound travels in one millisecond;
  500 Hz, first notch of a 1 ms delay.
- Video: Explainer 1, "Why one millisecond matters".

## How it works

- Heading: The spectrum finds the hits. The waveform gives the sample.
- Text: Drum Spike watches each close mic for a sudden rise of energy in its drum's own
  frequency bands, and keeps a hit only when it sounds like that drum. Drum Spike finds the
  steep edge of the stroke and goes back to the last quiet sample before the stroke's first
  motion. A polarity flip does not move the hit. The result is the start of the attack, not
  the moment a level is crossed.
- Videos: Explainer 3, "How Drum Spike hears a hit"; Explainer 4, "Finding the first sample".

## Learn it in four steps

Step 1 does the whole job. The other steps are there when you need them.

1. Get the hits (the whole job): Add your drum tracks, let Drum Spike find the hits, export
   MIDI and spike files. Guide: part 1.
2. Check and fix (when a hit is wrong): Listen, zoom in, and correct a hit by hand. The app
   shows the doubtful hits first. Guide: part 2.
3. Let the app do more (for difficult sessions): Bleed rules keep the other drums out, flams
   become two hits, and tempo, followers and layers run on their own. Guide: part 3.
4. Automate it (for power users): Run Drum Spike from scripts or AI agents with its command
   line and MCP server. Let an AI agent fine-tune a song hit by hit, at sample zoom: one click
   in Settings installs the skill. A hit moves only when two opinions agree, the agent's read
   of the stroke and how the track's clear hits start; when they differ, the agent leaves the
   hit and lists it for you. The session runs in Training Mode, which writes every move to a
   log so the placement can learn from your corrections. Guide: agent fine-tune; Guide: part 4.

Below the steps: the editor screenshot (one lane per close mic, every detected hit as a spike
above the waveform, the Inspector for the selected track) and the videos: Explainer 8, "From
waveform to velocity"; Explainer 5, "The body of the hit"; Explainer 15, "Let an agent
fine-tune a song"; the tutorial, "An agent fine-tunes a song"; Explainer 16, "Training Mode".

## Open source

- Heading: Built in the open. Read it, build it, improve it.
- Sub: Drum Spike is free. Its source code is on GitHub under the Apache License 2.0.
- Repo card: Cherry-Blossom-Valley / drumspike, Apache-2.0. A Rust core for the detection and
  the sample-exact placement, a Tauri desktop app, a headless CLI and an MCP server. Buttons:
  View the code, Releases, Report an issue, Developer guide. Build block:
  `apps/desktop/scripts/build-dmg.sh`, for Intel add `--arch x86_64`.
- Points:
  - Check how it works. The detection, the threshold and the walk back are in the code, with
    tests. No black box between your tracks and your triggers.
  - Your audio stays on your Mac. No account, no tracking. The audio and the project never
    leave your Mac; the analysis runs locally. A feedback report holds only what you wrote in
    the form, plus the app version and the OS, and it goes out only when you send it, after a
    preview.
  - Make it better. Report a session that trips it up, propose a fix, or add a kit preset.
    Pull requests are welcome.

## Licence (in the footer)

Drum Spike is free and open source (Apache-2.0). (Owner, 2026-10-24.) Support it through
GitHub Sponsors or Buy Me a Coffee. Both downloads are signed with a Developer ID and
notarized by Apple (the fine print under the download buttons).

## Footer

Source on GitHub, release notes, the guide (Drum Spike column); Tools: CBV Icon Studio, Drum
Spike; Legal: Imprint, Privacy Policy. "Made by Cherry Blossom Valley."
