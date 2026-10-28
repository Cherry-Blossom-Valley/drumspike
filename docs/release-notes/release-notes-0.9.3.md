Changes since 0.9.1:

Version 0.9.2 was never published (its draft was skipped); its changes are part of 0.9.3, so these notes cover everything since 0.9.1.

### Detection
- Quiet tracks calibrate their detection thresholds on their own, so soft and loud tracks both get clean hits without touching the Sensitivity slider. (#416)
- New bleed rule: a floor tom hit that coincides with a louder peak of the kick close mic is dropped as kick bleed. (#368)
- Detector plug-ins: a track can be detected by a plug-in instead of the built-in detector. A Detector select appears in the Inspector, Preferences lists the plug-ins with their licences, and the CLI and MCP accept a --detector flag. The built-in detector stays the default. (#385, #391, #392, #395)
- The Sensitivity slider re-picks the detection from the kept candidates of the track, and the Inspector histograms read the same candidates; the default setting reproduces the detection exactly. (#287)
- New detector: spectral onset detection with gates and cross-mic rules; every hit comes from the close mics of your session. (#327)
- Removed in this release: the roll fill, the kick rescue and the snare rescue. (#327)
- Cross-track bleed rule: a tom hit that coincides within 10 ms with a louder peak of a kick, snare or hi-hat close mic is dropped as bleed when it arrives later and is quieter in its own band. The rule compares the tom hit with the full candidate list of the source mic (every peak, kept or dropped), not only its confirmed hits. Sparse tracks get fewer false tom hits. (#290)
- Snare ghost-note gate: a soft snare hit below the level gate passes when its spectral shape matches the track and no kick or tom hit sits within 30 ms. (#292)

### Tracks
- When a file name gives no role, Add Tracks suggests one from the audio, for kick and tom; the other drums get no suggestion yet. (#397, #413)
- The Add Tracks summary recounts on every change, and a role you pick by hand counts as "N set by you". (#277)

### Learn and export
- Learn This and Forget This work with the built-in detector. Forget removes the marked hit and its near twins. (#333)
- Export dialog texts: the Audio Spike section reads "Audio (the body of each hit)", the count reads "1 hit", and the built-in choice reads "Built-in". (#414)

### Data safety
- No export overwrites a source file. The export checks the whole plan before it writes; the source audio, the one-shots and the sampler bounces of every open project are protected, and a folder link counts as its target. (#261)
- A failed save never shows saved. A refused write keeps your edit and offers Retry and Discard in the status bar; Save, Export, Re-analyze and Learn stop before their next step. A late save answer moves the dirty mark of its own project only. (#260)
- Re-analyze offers Keep for velocity-only and restored edits. (#263)
- Undo restores a hit exactly: the note, the velocity marks and the edited state come back as they were. (#264)
- A project open read-only refuses every changing command with a clear message, and the file stays as it is; a done job with an errored track is refused too. Save As turns the session into a writable copy. (#266, #271)
- Event edits are one write: two overlapping edits can no longer lose one, and the second gets a refusal you can retry. (#268)
- A duplicate analysis start is refused while one is queued or running, and a cancelled or replaced analysis commits nothing. (#269)
- A failed composite command changes nothing: a rejected velocity set or role change leaves the track record and the dirty flag as they were. (#272)
- Project files are written atomically: a failed write leaves the destination and the folder as they were and no temporary file behind, and overlapping exports to one destination write in a defined order. (#273)
- A failed backup rotation after a committed save is a warning: the save still succeeds. (#274)

### Playback, tempo and MIDI (since the first 0.9.3 draft)
- MIDI export: the note ticks follow the exact tempo you set, so a DAW at the same tempo (for example Pro Tools at 92 BPM) plays every note within half a sample of its hit. (#466)
- MIDI export: the ticks and the written tempo agree, so a 6-minute export at 118 BPM no longer drifts against the audio. (#434)
- Detect Tempo: bar 1 lands on a beat of the hits' own grid when the suggested tempo is half or double the played one, for example in a session with a kick track only. (#465)
- Playback: tracks no longer drift out of sync under CPU load; a starved track catches up after an underrun. (#430)
- Playback: no tile seam while the view follows the playhead: no dark gap column, and the velocity lane and its labels cover the whole lane. (#435)
- Transport: Space, Numpad0 and Option+Space reach the transport after a click on a button or a slider (only a text field keeps them, and the status line says so). While Stop Returns to Start is on, every stop puts the playhead back where the play started. (#436)
- Learn: a hit that a Remove Example sweep deleted leaves the page at once. (#431)
- Velocity lane: the resize handle sits on the lane's bottom edge, and the lane follows the hand. (#437)
- Other Mics Under Hit: each small lane is scaled to its own peak in the shown window, with the gain shown at the right (for example +22 dB), so a bleed-level mic reads instead of a flat line. (#438)
- Export: a file dragged out of the Export dialog is a real export in the project's Exports folder; it stays after the dialog closes and after quit. (#433)

### Fixes
- The confidence of a hit updates in detector time after a Hitpoint Shift. (#275)
- No panics or invalid values at the public boundaries: an impossible cache header is a cache miss, a start past the signal answers none, and a shift value out of range is refused. (#278)
- The Layer section wraps the "no sampler calibration" text instead of clipping it. (#277)
- Tracks named tom_1, tom_2, tom_3 (or Tom_1 ...) stay three different tom drums, so the tom bleed rules work for them. (#369)
- Audio Spike export: each snippet now covers the body of the hit, as the Export tooltip says, not only the attack click. (#379)

### Documentation
- User guide chapters 1-14, and the new "Get the best results" page. (#304, #306, #310, #322, #328, #384, #427)
- Explainers: the Audio Spike body-of-the-hit page, hit or bleed, and the explainer layout fixes. (#381, #380, #404)
- Explainer 03: How Drum Spike hears a hit. (#421)
- Guide chapter 11, Learn: teach by example. (#428)
