Changes since 0.2.0:

### Hit times at the first motion of the hit
- Each hit now sits at the first motion of the drum, not part-way up its attack. On acoustic drums the hit time was 1 to 2 ms late before; it is now within a few samples of the start. Sharp, synthetic attacks were already exact and stay so.
- A hit can no longer come out 10 ms or more early when the detector's coarse time is far before the attack (before: up to 3.5 % of the hits in a busy session).
- Projects analysed with an earlier version keep their hit times. Re-analyze a track to get the new times; hand edits stay where you put them.
- Known limit: on a few snare and tom hits the new time starts on the bleed of another drum, 0.5 to 3 ms early.

### Fewer false tom hits
- Drum Spike now compares every tom hit with the other tom mics and with the snare and kick mics of the session. A hit that another drum's bleed explains is taken out of the output: it leaves the MIDI and spike files, Export All and the click.
- A second check compares tom hits with each other: when two tom mics have a hit within 12 ms and one of them is 6 dB or more quieter than the other and also quiet for its own mic, the quieter one is bleed. Loud strokes on two toms at once (unisons) stay.
- On seven test songs the two checks remove about nine tenths of the false tom hits (1318 to 135) and lose 5 real tom hits of about 830. Together with the new hit times and the snare roll fill, the hand fixes (hits to delete, add or move) against fair references fell from 3467 to 1068 on those songs.
- Removed hits stay visible in grey, like hits under the loudness cut. Restore brings a hit back, and it stays back through later re-picks.
- The rule needs at least two played tom mics. It is off, with the reason shown, when the tom tracks are edited to silence, and it never uses sample or trigger tracks. Projects analysed before keep their hits until Re-analyze.

### Soft snare rolls
- Soft strokes inside a snare roll or a run of ghost notes are now found: when quiet candidates form an even run of 8 or more strokes, they become hits. This applies to snare tracks only.
- On seven test songs this finds 57 more snare hits and adds no false hits. Projects analysed before keep their hits until Re-analyze.
- Known limit: the evidence from songs not used to set the rule is small (one roll). Tell us if a steady hi-hat pattern turns into snare hits.

### Tempo and meter map
- One tempo and meter map per project: start tempo, tempo changes, meter and meter changes, and the time of bar 1.
- Track > Detect Tempo suggests the tempo, bar 1, the meter and tempo changes from the kick and snare hits, with alternatives (for example half or double tempo). Accept is one undo step.
- Tempo, Meter and Bars|Beats rulers and a bar and beat grid; File > Import Tempo Map reads a MIDI file. The MIDI export writes the map into the tempo track.

### Range selection and Learn marks
- A drag in a waveform lane selects a time range; a click still moves the playhead.
- K marks the hits in the range as "this", Shift+K as "not this". The marks are saved with the project. They are the groundwork for a Learn filter in a later version and do not filter hits yet.

### Command line and AI agents
- Drum Spike has a command line (`drumspike cli`) and an MCP server (`drumspike mcp`) for scripts and AI agents. They use the same commands as the window, and their exports are byte-identical. See docs/agent-api.md.
- A project is locked while it is open, so two programs do not write it at the same time.


### Pro Tools key commands
- The default key preset follows the Avid Pro Tools Shortcuts Guide (2025.10): P and ; select the track above or below, L and ' step to the previous or next hit (Control+Cmd on all tracks), E zooms the selection, Option+A zooms to the whole project, comma and period nudge, Control+Return starts playback.
- Waveform zoom in, out and reset (Option+Cmd+] and [): the waveform height changes, the audio does not.
- Edit > Nudge Value: 1, 10 or 100 samples, 1 ms or 10 ms.
- Half-speed playback (Shift+Space), one octave down as in Pro Tools.
- A Cubase key preset is in Drum Spike > Settings > Key preset.

### Editing and the track view
- An Inspector for the selected track: name, colour, role, note and source file.
- An info line with a main counter in seconds and a sub counter in samples.
- A compact track header with meter, volume and pan, and View > Track Height.
- Soft tracks (for example a sub kick) can follow another track, and the sensitivity range is wider.

### Playback
- Playback uses less CPU: about 6 % of one core during play of 30 tracks, down from about 8.5 %.
- Smooth follow glides without redrawing every lane in every frame, and without a blank frame when it starts.
- The analysis of a session is faster (the two stages overlap).

### Sensitivity and velocity histograms
- The Inspector shows two histograms, as in Massey DRT: the detection strength of every candidate hit and the loudness of every hit. Hits that a setting filters out are grey, and the lane shows the same split while you drag.
- The largest gap in a histogram is marked and can be used as the start value with one click.
- A new threshold picks the hits again from the kept model output, with no new model run. The result is the same as a full Re-analyze.
- A minimum-loudness cut (off by default).

### Exports
- Hits outside the audio file are now counted and named in the export message. Before, they were skipped with no message.
- Two tracks whose names differ only in case or in Unicode spelling (for example "Kick" and "kick") no longer share export files.
- A follow track is derived again after a relink, when a project opens and before an export, so its exports always hold the current hits of its source.
- A rename that changes only the case of a track name no longer deletes the track's analysis folder.
