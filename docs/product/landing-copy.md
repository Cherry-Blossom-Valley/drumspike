# Landing page copy

Text for `site/index.html` (ticket "Media 6"). Keep it short; the videos and the guide carry the
detail. Names and keys as in the app. No claim without a source in the app or the guide.

## Hero

- Title: Drum Spike
- Line: Every drum hit, on the first sample.
- Text: Drum Spike finds every hit in your close-mic drum tracks, puts each one on the first
  sample of its attack, and exports MIDI and spike files that trigger on time.
- Buttons: "Download for Apple silicon" and "Download for Intel" (latest release), "Watch the
  90-second tour" (product video).
- Small print under the buttons: macOS 26 Tahoe or later, Apple silicon and Intel. Free and open
  source (Apache-2.0). (Owner 2026-10-23: the page states macOS 26; the app enforces no minimum.)

## Why it matters (from explainer 1)

- Heading: One millisecond is audible.
- Text: A trigger that fires one millisecond late against the close mic cuts holes into the sound
  at 500 Hz, 1.5 kHz, 2.5 kHz, right where a snare has its body and crack. Level-based
  transient detection fires late on soft hits and early on loud ones, and it fires on bleed.
  Drum Spike puts every hit on the first motion of the attack, soft or loud.
- Link: "Why one millisecond matters" (explainer 1).

## The feature path (cards)

Part 1, "The main job" (after these, you are done):
1. Why sample-exact matters. One millisecond is audible.
2. Bring in a session. Roles from your file names, a kit preset or the audio.
3. The app finds the hits. Each track listens for its own drum only.
4. Sample-exact. Each hit moves to the first motion of its attack.
5. Export. MIDI and spike files per track, on your session's time.

Part 2, "Check and fix":
6. Find your way. Play, zoom to 5 ms, jump from hit to hit, Pro Tools or Cubase keys.
7. Edit hits. Add, delete and nudge by one sample; full undo history.
8. Velocity. How hard each hit was played, and a lane to edit it.
9. Doubtful hits first. The app shows you the hits it is least sure of.

Part 3, "Let the app do more":
10. Bleed and the hard cases. Tom bleed, missed kicks, flams and rolls, handled per session.
11. Learn. Mark one unwanted hit and every similar one goes; mark a missed one and similar ones come back.
12. Followers and layers. One track follows another; render a layer.
13. Tempo, bars and beats. A tempo map from your kick and snare.

Part 4, "For power users":
14. Automate it. A CLI and an MCP server for scripts and AI agents.

## Licence (near the download buttons and in the footer)

Drum Spike is free and open source (Apache-2.0). (Owner, 2026-10-24, changed the same day.) Both downloads are signed with a Developer ID and notarized by Apple.

## Footer

Source on GitHub, release notes, the guide, licence, "Made by Cherry Blossom Valley".
