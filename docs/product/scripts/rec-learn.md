# Screen recording: Learn, teach by example (guide chapter 11)

The Learn chapter of the guide on the demo session. Narration for `tools/media/tts.py` (one
`##` per scene; `>` lines are the scenario steps and captions for `tools/media/record.py`, not
spoken). Target length: 60 to 90 s. Keys shown in the key overlay. The render runs model-free
(`DRUMSPIKE_MODEL=none`, the built-in detector, as the 0.9.3 app runs). The TTS comes from the
shared folder (`/Users/Shared/drumspike-media/tts-out/rec-learn`, the coordinator's mbp route). No
path is shown in any scene, so there is nothing to neutralize. The wording keeps the guide's
claims: Forget This removes the marked hit and its near copies; nothing is greyed out on a
drum (no such code).

## pl-ghost

> s top: a click on the ghost note's spike selects it (the Inspector follows). Shift+K, Hits > Forget This. Caption: Hits > Forget This

Chapter 10's rules clean the session on their own. Learn is for the hits that stay wrong. Here is a ghost note on the snare. Select it and press Shift+K for Forget This.

## pl-grey

> The marked hit and its near copies go grey at once: they stay in the lane, Tab skips them, and they leave the MIDI, the spikes and the click. The status line says what the step did. The first Forget This turns the Learn filter on. Caption: The marked hit and its near copies go grey

Forget This removes the marked hit and its near copies. They go grey: they stay in the lane, Tab skips them, and they leave the MIDI, the spikes and the click. The first Forget This also turns the Learn filter on.

## pl-add

> Drag a range in the wave lane of s top over a stretch the detection missed. K, Hits > Learn This: the hit is added at once, flagged as learned, and the example is stored. Caption: Hits > Learn This

The detection missed this one. Drag a range over the stretch, and press K. Learn This adds the hit at once, flagged as learned, and stores your example.

## pl-histogram

> Open the Inspector of s top: the Learn section shows the Similarity histogram with the Match and the Add line, and the counts line under it. Caption: The Learn section of the Inspector

The Inspector's Learn section shows the Similarity histogram. It holds the track's candidates against your examples. The Match line is the filter's strictness. The Add line is the rule that added the missed hit. The line under the histogram counts the hits and the current Match.

## pl-remove

> Option+K, Hits > Remove Example: the example goes, and the added hits of the example go with it. One undo step. Caption: Hits > Remove Example

Option+K is Remove Example. It takes the example back, and the added hits of the example go with it.

## pl-end

> Caption: Drum Spike 0.9.3

That is Learn. Forget This for the hits you do not want, Learn This for the hits the detection missed, and the track follows your examples.
