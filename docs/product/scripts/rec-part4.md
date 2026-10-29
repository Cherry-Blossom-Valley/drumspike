# Screen recording, part 4: what is new in 0.9.3

The new features of 0.9.3 on the demo session. Narration for `tools/media/tts.py` (one `##`
per scene; `>` lines are the scenario steps and captions for `tools/media/record.py`, not
spoken). Target length: 60 to 90 s. Keys shown in the key overlay. The recording home has the
example plugin installed (`tests/fixtures/plugins/json_hits.py`, the scenario sets
`"plugins": true`), so the Detector select lists it and the licence dialog shows the neutral
`~/Library/...` path. Explainer 05 covers the Audio Spike rule; this recording shows the
workflow only.

## p4-export

> Select the kick. Track > Export Selected Tracks… (Shift+Cmd+E); MIDI off, Spikes on, type Audio; Export. Caption: Track > Export Selected Tracks…
> The summary names the audio spike file. Spike type Synthetic, Export again: the click spike.

The export has a new spike type, and it is called Audio. An audio spike holds the body of each hit, the wave around the attack. The click spike keeps only the attack itself. Export once for each: the summary names the file it wrote. Here the two of them are, side by side.

## p4-import

> File > Add Tracks… on the export folder: the two spike WAVs, checked, Add. Caption: File > Add Tracks…
> Select the kick, Tab, 5 (50 ms zoom), then select the audio spike track: its lane sits next to the click spike.

The exports can come back in. Add Tracks from the export folder: the two spike WAVs of the kick, checked. The 5 ms preset at a hit shows the spike against the wave.

## p4-detector

> Inspector of the kick: the Detector select lists Built-in and JSON Hits. Pick the plug-in: the licence dialog shows the executable path in the neutral home form, the licence name and the full text. Accept and use. Caption: The Detector select of the Inspector
> Re-analyze: the plug-in proposes its own hits, Drum Spike refines each one to the attack.

The Inspector lists Built-in and your plug-ins. The licence shows before the first use. On a re-analyze the plug-in proposes the hits, and Drum Spike refines each one to the attack.

## p4-sens

> Inspector of the snare: the Sensitivity slider. Click it (the thumb takes the click), then Shift+Up twice; every move re-picks the detection from the kept candidates. Caption: The Sensitivity slider

The Sensitivity slider re-picks the detection from the kept candidates. Raise it, and quieter hits join at once.

## p4-keep

> Snare: Tab selects a hit; the info line: type a velocity over the old one, Return. Re-analyze: the dialog offers Keep. Keep edited hits. Caption: Re-analyze offers Keep

Your edits survive a re-analyze. Set a velocity by hand. Press Tab to select a hit, type the velocity over the old one in the info line, and press Return. Then run the re-analyze. The dialog offers Keep edited hits, and the hand velocity stays.

## p4-end

> Caption: Drum Spike 0.9.3

That is what is new. The audio spike, the detector plug-ins, the sensitivity re-pick, and Keep for your edits.
