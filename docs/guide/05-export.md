# 5. Export

Export writes the hits as MIDI and spike files for your DAW. The files carry the time of the
session, so they line up in Pro Tools. After this chapter the main job is done: tracks in, hits
found, files out.

## Steps

1. **File > Export…** (`Cmd+E`). The Export dialog opens.
2. Choose per track a MIDI file and a spike WAV, or one multi-track MIDI file for all tracks.
3. Choose the spike type:
   - Synthetic: a short click.
   - Audio: the hit's own audio from the onset over its body. Drum Spike low-passes the track
     below 800 Hz and follows the first three clear swings of that low tone (a swing counts at
     25 % of the largest of the first three). The snippet ends on a zero crossing, at most
     60 ms. A kick gives about 15 to 25 ms, a snare about 5 to 10 ms.
   - Both: a synthetic spike and an audio spike.
4. Pick the destination folder and export.
5. **File > Export Again** (`Option+Cmd+E`) repeats the last export with the same settings. This
   is handy after an edit: export again and the DAW gets the new files. The spike files are
   written in blocks, so an export of a long track does not hold the whole file in memory.

![The Export dialog with the per-track MIDI and spike WAV files and the destination folder.](img/05-export.png)

![A kick hit of the demo session: the raw waveform in cobalt, the low tone below 800 Hz in orange, and the snippet range shaded up to the zero crossing.](img/05-audio-spike.png)

this demo kick: 8.9 ms (the length the Audio Spike rule measures on this hit). Real kicks
usually land at 15 to 25 ms, snares at 5 to 10 ms.

The Audio Spike in numbers: Drum Spike copies the hit's own audio from the onset over its
body. The low tone below 800 Hz drives the length: the first three clear swings of that tone,
cut at the zero crossing, at most 60 ms. A kick lands around 15 to 25 ms, a snare around 5 to
10 ms. The click spike stays available: it is the synthetic option in the same dialog.

The MIDI file carries every hit at its sample. The export picks the ticks per quarter note
from the track's sample rate and the tempo: one tick is one sample where the standard's 15
bits allow it, and the answer states the bound (`worst_error_samples`, with the chosen
`ticks_per_quarter`) where they do not. The spike file is sample exact at every tempo and
rate. A DAW that takes its tempo from the file's tempo event instead of its own session
tempo can drift by about 1.9 samples in five minutes; set the session tempo and import the
notes without the tempo.

Next: [find your way](06-find-your-way.md), the first chapter of part 2.
