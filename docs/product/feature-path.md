# Drum Spike: the feature path

This is the spine of all user documentation and marketing material (guide pages, screen
recordings, explainer videos, product video, landing page). It breaks the product into 14
chapters. Each chapter is small, has one idea, and builds on the chapters before it. A reader
who stops after chapter 5 can already do the main job; the later chapters make it faster and
cleaner.

Who it is for: mixing engineers and drummers who replace or layer drum sounds with a trigger
plug-in or a sample replacer, and who need the trigger points on the exact sample.

The one-sentence pitch: Drum Spike finds every drum hit in your close-mic tracks, puts each hit
on the first sample of its attack, and gives you MIDI and spike files that trigger on time.

Rules for every asset:
- Use the names exactly as the app shows them (menu titles and keys below are from
  `static/keymap.js`; keys are the default Pro Tools preset unless a chapter says otherwise).
- Show the demo session (see `media-plan.md`), never an owner or client session.
- State the licence where users download: "Drum Spike is free and open source (Apache-2.0)."
  (owner, 2026-10-24, changed the same day). Name no model licence.

## Part 1: the main job

### 1. The problem: why "sample-exact" matters
- Idea: a trigger that fires 1 ms late against the close mic smears the sound (comb filtering,
  phase), and a missed or doubled hit is audible. Transient detectors in DAWs work on loudness and
  fire on the wrong sample or on bleed.
- What Drum Spike gives you: one MIDI note and one spike per real hit, on the first sample of the
  attack, per drum track.
- Builds on: nothing. Asset: 3b1b explainer "Why one millisecond matters".

### 2. Bring in a session (Add Tracks)
- File > New Project… (Cmd+N), File > Add Tracks… (Shift+Cmd+I): pick WAV files or a session
  folder (Pro Tools "Audio Files"). Files stay where they are (referenced); BWF time stamps are kept.
- Each track gets a role: kick, snare, tom, hat, cymbal or other. The role comes from a kit preset,
  from the file name ("k in", "s top", "t1"), or from the audio when the name gives none
  ("from audio"). Drum Spike's own exports are never offered as drum tracks.
- Change a role later in the Inspector (Option+I), section Track: the hint says where the role
  came from; a change analyzes the track again; Undo brings everything back.
- Builds on: 1.

### 3. Detection: the app finds the hits
- Analysis runs per track when tracks are added (Hits > Analyze Selected Track, Analyze All). The
  detector listens for the track's role only: spectral onset detection finds a sudden rise of
  energy in the drum's own bands, then gates keep the real hits (level, shape against the track's
  own loud hits, time-domain attack). A snare mic yields snare hits, not the hat that leaks into it.
- Cross-mic rules decide the rest: a hit is loudest and earliest on its own mic, and the overhead
  denies hi-hat and cymbal bleed.
- The Sensitivity setting re-picks the hits from the kept candidate list, with no new analysis.
- Builds on: 2 (the role picks what the detector listens for).
- Asset: 3b1b explainer "How Drum Spike hears a hit".

### 4. Sample-exact: each hit moves to the attack
- The detector says "a hit is near here". Drum Spike then walks to the first motion of the attack
  in the waveform and puts the hit on that sample. Zoom to 5 ms (key 6) to see it.
- This is the core idea of the product. Asset: 3b1b explainer "Finding the first sample".
- Builds on: 3.

### 5. Export: MIDI and spikes for your DAW
- File > Export… (Cmd+E): per track a MIDI file and a spike WAV, or one multi-track MIDI file.
  Spike type: Synthetic (a short click), Audio (the hit's own audio over its body: the low tone
  below 800 Hz, its first three clear swings, cut at the zero crossing, at most 60 ms) or Both.
  The files carry the session's time, so they line up in Pro Tools.
- File > Export Again (Option+Cmd+E) repeats the last export with the same settings.
- Asset: 3b1b explainer "The body of the hit".
- After this chapter the main job is done: tracks in, hits found, files out.
- Builds on: 2-4.

## Part 2: check and fix

### 6. Find your way: play, zoom, move between hits
- Play/Stop (Space), Half-Speed Play (Shift+Space), play-and-return (Option+Space), zoom presets
  (keys 1 to 6), Zoom In/Out (T, R), track height (Option+Up/Down), Next/Previous Hit (Tab, L),
  Next Hit on All Tracks (N), the Overview (Option+O), markers.
- Key presets: Pro Tools (default) or Cubase, in Settings (Cmd+,). Help > Keyboard Shortcuts.
- Builds on: 5 (a session with hits to look at).

### 7. Edit hits
- Add Hit at Playhead (H), Delete Hit, Nudge Earlier/Later with a nudge value of 1 sample to
  10 ms, Audition Hit. Undo (Cmd+Z), Redo, Undo History… (Option+Cmd+Z).
- Builds on: 6.

### 8. Velocity: how hard each hit was played
- Every hit gets a velocity from its level. View > Velocity Lane (V) shows and edits them: drag a
  stalk, draw a line over a range (Option-drag), resize the lane. Floor, ceiling and the loudness
  cut in the Inspector shape the curve; Edit > Recompute Velocities.
- Asset: 3b1b explainer "From waveform to velocity".
- Builds on: 7.

### 9. Doubtful hits first (review queue)
- Hits > Next Doubtful Hit (D) and Previous (Shift+D) step through the hits the app is least sure
  of, lowest first; the info line says why a hit is doubtful.
- Builds on: 6-7.

## Part 3: let the app do more

### 10. Bleed and the hard cases (automatic)
- Close mics hear the other drums. Drum Spike cleans this up per session after the analysis:
  a tom hit that another close mic explains is marked and left out (a kick, snare or hi-hat
  close mic hears it louder and earlier, or another drum's tom mic hears it within 12 ms and
  6 dB or more quieter); a quiet snare hit stays only when its spectral shape matches the
  snare and no kick or tom hit lies within 30 ms; the second stroke of a flam on a snare
  close mic becomes a hit of its own (the Inspector's Split flams check, on by default).
- A marked hit shows grey and leaves MIDI, spikes, Export All, the click, the tempo detection
  and a follower. An edit on it (a nudge) brings it back.
- View > Other Mics Under Hit (O) shows what the other mics heard at the selected hit.
- Asset: 3b1b explainer "Is it a hit or is it bleed?".
- Builds on: 3-4, 9.

### 11. Learn: teach by example
- Hits > Forget This: mark one unwanted hit; every similar hit on the track goes too. Learn This:
  mark a missed hit; similar missed hits come back. Remove Example, Learn Filter, Forget Examples
  of Track. The Inspector histogram shows Match and Add.
- Builds on: 7, 9, 10.

### 12. Follower tracks and layers
- A track can follow the detection of another track (Track > Own Detection switches back), for
  example a snare bottom mic that follows the snare top. The Layer section loads, renders and saves
  a layer through the file dialog.
- Builds on: 3, 5.

### 13. Tempo, bars and beats
- Track > Detect Tempo… suggests a tempo map from the kick and snare hits; Set Bar 1 at Playhead;
  File > Import Tempo Map…; View > Show Bar and Beat Grid (G), the Tempo, Meter and Bars|Beats
  rulers, Metronome Click.
- Builds on: 3, 6.

## Part 4: for power users

### 14. Automate it (agent API)
- Every app operation is a command (`docs/agent-api.md`): a CLI and an MCP server let scripts and
  AI agents open projects, analyze, edit and export.
- Builds on: all.

## Asset map

| Chapter | Guide page | Screen recording | 3b1b explainer |
|---|---|---|---|
| 1 | yes | - | Why one millisecond matters |
| 2 | yes | yes (one recording, 2-3) | - |
| 3 | yes | yes (one recording, 2-3) | How Drum Spike hears a hit |
| 4 | yes | yes | Finding the first sample |
| 5 | yes | yes | The body of the hit |
| 6-7 | yes | yes (one recording) | - |
| 8 | yes | yes | From waveform to velocity |
| 9 | yes | yes | - |
| 10 | yes | yes | Is it a hit or is it bleed? |
| 11 | yes | yes | - |
| 12-13 | yes | yes (one recording) | - |
| 14 | yes | - | - |

Part 4 of the recordings shows what is new in 0.9.3 on the demo session (the Audio spike
export, the detector plug-ins, the Sensitivity re-pick and Keep for edits):
`docs/product/scripts/rec-part4.md`. It covers the workflow of chapters 3, 3b, 5 and 7 on the
current release, so the recording set shows the product as it ships.

The product video (about 90 s) and the landing page are made from these pieces.
