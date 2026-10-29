# Screen recording, part 1: tracks in, hits on the attack, files out

Chapters 2 to 5 of `docs/product/feature-path.md`, on the demo session. Narration for
`tools/media/tts.py` (one `##` per scene; `>` lines are the scenario steps and captions for
`tools/media/record.py`, not spoken). Target length: 75 to 90 s. Keys shown in the key overlay.

## p1-new

> Cmd+N, name "Demo", choose a folder. Caption: File > New Project…

Start with a new project. Press Command N and give it a name.

## p1-add

> Shift+Cmd+I, pick the demo session's Audio Files folder; the Add Tracks list shows thirteen rows with roles from the names (k in = kick, s top = snare, t1 = tom ...). Caption: File > Add Tracks…

Now add the tracks. Press Shift Command I and pick the session's audio folder. Drum Spike reads the names and gives every track its role. Kick in is a kick, s top is a snare, t1 is a tom. Check the list and press Add.

## p1-analyze

> The analysis runs; the lanes fill with hits track by track; the progress shows in the info line.

The analysis starts at once. For each track, the detector listens for that track's drum only. The hits appear as spikes under each waveform.

## p1-role

> In the Add Tracks list (before Add), the row "new take" shows "from audio" with the role kick. After the analysis: Option+I opens the Inspector, select "new take"; section Track shows the role and its hint "suggested from the audio". Change it to "tom", then press Cmd+Z: the role and the hits come back.

One file has a name that says nothing: new take. Drum Spike listened to it and suggests kick. The Inspector shows where every role came from. Change a role if it is wrong; the track is analyzed again. Undo brings everything back.

## p1-zoom

> Click the wave lane just before a hit (the playhead), then Tab selects the hit; press 6 (5 ms zoom); the spike sits on the first motion of the attack; the sample counter in the info line.

Zoom in to five milliseconds. Every hit sits on the first sample of its attack, not on a level threshold.

## p1-export

> Cmd+E opens the Export dialog: per-track MIDI and spike WAV, spike type Synthetic; press Export; the summary lists the files.

Now export. Press Command E. Choose MIDI and spike files per track, and the spike type. Press Export.

## p1-again

> Option+Cmd+E; the summary shows the same files again.

After the next edit, Option Command E exports again with the same settings.

## p1-end

> Caption: "Next: check and fix (part 2)".

That is the whole job: tracks in, hits on the attack, files out. Part two shows how to check and fix the hits.
