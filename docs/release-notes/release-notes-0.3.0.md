Changes since 0.2.0:

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
