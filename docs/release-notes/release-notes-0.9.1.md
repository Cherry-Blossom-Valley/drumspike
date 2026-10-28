Changes since 0.9.0:

### Editing
- Track role in the Inspector: set or change the role (kick, snare, tom, hat, cymbal, other) and the track is re-analyzed. Undo brings the role, the note and the hits back; the hint says where the role comes from and warns when no name rule matched. The lane colour follows the role. (PR #227)
- Velocity lane is a usable velocity editor: lane size, velocity scaling, hover values and hints. (PR #220)
- Play-and-return for repeated listening: Option+Space plays from the playhead, the next Space (or Option+Space again) stops and puts the playhead back where this play started; Option+Shift+Space is the same at half speed. Keys for the common convenience commands: Add Hit at Playhead (H) and Show Bar and Beat Grid (G) in the default (Pro Tools) preset; Velocity Lane (V), Export Selected Tracks… (Shift+Cmd+E) and Undo History… (Option+Cmd+Z) in both presets. (PR #223)

### Export
- Audio spikes: `<track>_audio_spike.wav` holds, at each exported hit, the track's own audio from the hit start for about 1.5 cycles of the waveform there (2-30 ms, cut at the next hit), silence everywhere else, with the length, sample rate and BEXT time of the source. The source file is only read; a snippet export never overwrites it. (PR #214)
- The Export dialog offers the spike type: Synthetic (the blip WAV), Audio (1.5 cycles of the track audio) or Both; the choice is kept per project and File > Export Again repeats it. The export summary makes its file noun singular for one file. (PR #221)
- Opening the Export dialog derives a stale follower's hits again (the drag-tile render runs the same derivation as the export) and the lanes show the new hits at once. (PR #214)

### Tracks
- Add Tracks suggests the role from the audio when the name gives none: the drum model runs on the file and the class with the most confident onsets wins (kick, snare, tom, hat, cymbal; below the confidence floor the file stays "other"). The row is marked "from audio" and the role select is preset to the suggestion; you can change it before the add. A role from a kit preset or from the name scheme is never overridden. (PR #232)
- A session scan no longer offers Drum Spike's own exports (synthetic spike, audio spike, layer render, also Pro Tools' numbered copies such as `s top_02_spike_01`) as drum tracks: they get no role, neither from the name nor from the audio. (PR #231)

### Layer
- Layer onset view is no longer black and unexplained: it renders and carries a label. (PR #213)
- The Layer section loads, renders and saves through the file dialog with sensible defaults. (PR #230)

### Inspector
- The Learn histogram labels (Match, Add) are drawn above the bars and never clipped on Retina screens. (PR #219)

### Keys self-test
- The keys self-test runs every preset of DSKeys.PRESETS and prints the totals per preset (Pro Tools default); menu-path keys that the NSMenu does not dispatch are marked 'harness: not dispatched' (owner hand-check). (PR #215)

### Fixes
- Every menu and page key works in the desktop app: View > Track Height > Taller and Shorter (Cmd+Down / Cmd+Up in the menu, Option+Down / Option+Up on the page) change the track height again, and the Shift+Cmd menu keys fire. (PR #215)
- An audition ends in the engine at its window end, every frame delivered. (PR #202)
- The audition e2e reads the page's own record of the audition state (20/20 under the gate load). (PR #203)
- The Export Again e2e waits for the export-finished event, not a stale message. (PR #229, internal)
