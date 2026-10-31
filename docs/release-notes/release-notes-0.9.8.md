Changes since 0.9.7:

### Tempo
- The Tempo and Meter rulers show the tempo and meter in force again. In 0.9.7 the value went missing while the lanes followed playback, and it stayed missing after a stop.
- A MIDI export of a project without a tempo now asks first. The notice names the three ways to set the tempo (Track > Detect Tempo, File > Import Tempo Map, the Tempo ruler) and offers Set Tempo…, Export anyway and Cancel. Before, the export used 120 BPM without a word, so the notes landed late in a DAW session with another tempo. The notice covers the Export dialog, File > Export Again and the Drag MIDI tile; the agent API reply carries the same warning.
- Detect Tempo picks the right octave on more half-time grooves. A song with the snare on every fourth beat and a kick on the 8ths was suggested at half its tempo; the detector now checks the finer grid before it settles on the slower tempo. The change was measured on three real sessions, the ENST and the STAR test sets: no result got worse.
- Detect Tempo places bar 1 on the downbeat under a half-time snare. Before, bar 1 could land one beat late on such a groove, so the grid and the MIDI export were shifted by a beat. Checked on three real sessions: one now starts on the right beat, the other two are unchanged.

### Counter
- The main counter also shows Bars|Beats (bar|beat|ticks, 960 ticks per beat, from the tempo map, like the Pro Tools main counter) and Timecode (HH:MM:SS:FF at 30 fps). View > Main Counter has the two new formats; Exchange Counters swaps Bars|Beats and Timecode the way it swaps seconds and samples.

### Editing
- Tab and Shift+Tab skip the deactivated hits by default. View > Tab Moves To (and the setting in the sidebar) switches to All Found Hits. A selected deactivated hit stays visibly deactivated: a dashed grey line, a grey marker with the selection ring, and "deactivated" in the counter and the info line.
- Undo and redo restore a deleted hit exactly as it was, with its stored identity. Before, an undone delete brought back an equal hit under a new identity.
- A failed layer render no longer blocks every later render with "the path is a folder". The leftover of the failed render is removed; any other folder at the target stays untouched and the message says what to do.

### Look
- The role drawing in a track header hides when the row is dragged too short for it, instead of overlapping the M/S buttons and the pan slider.

### Reliability
- Fewer places where a damaged or cut-off file could stop the app: the file writer, the waveform cache and the export path now answer with an error instead of a crash.
- The app window gets only the system permissions it uses (events, the open and save dialogs, the file drag), instead of the broad default set.
- More places in the desktop shell and the developer tools answer a bad input with an error message instead of a crash.
- The dependencies follow a written supply-chain policy (licences, sources, advisories), checked before each release.
