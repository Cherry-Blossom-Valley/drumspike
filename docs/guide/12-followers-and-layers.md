# 12. Follower tracks and layers

Two ways to keep a track right without a second detection: a follower takes the hits of
another track, and a layer plays your own one-shot sample at every hit.

A follower is for a sub mic. A kick sub mic hears the same hits as the kick in, but much
softer and duller, so its own detection is not reliable. So the sub takes the hits of
the kick in, sample-exact, with velocities from its own signal.

## Steps

1. When you add tracks, **File > Add Tracks…** (`Shift+Cmd+I`) marks a sub mic row with a
   follow check. Leave it on and the new track follows its drum from the start.
2. To set it later, select the track and pick Track > Follow Hits Of > the drum track.
   The check moves to that track. **Track > Own Detection** switches the track back to its
   own detection.
3. A follower is read-only: its hits are derived, and an edit on it is refused. Edit the hits
   of the source track instead. The follower takes the change at once, and the next analysis
   of the source refreshes it too. The velocity comes from the follower's own signal, so the
   softness of the sub mic stays.
4. The track row and the Inspector show `· follows <name>`, so you always see which track
   leads. A track follows one other track, a source cannot follow another track, and if the
   source is removed the follower detects on its own again.
5. For a layer, select the track and open the Inspector. The Layer section takes one WAV or
   AIFF one-shot: drop it on the zone or press **Choose…**. Drum Spike plays the one-shot
   at every hit of the track, with its onset on the hit.
6. Shape the layer with the controls of the section: **Play** on or off during playback, the
   **Level** in dB, the **Polarity** (normal, inverted, or auto), and the **Velocity curve**
   (dB, linear, or fixed). **Align** places each hit's one-shot where its attack matches the
   close mic best; **Measure** shows the match of every hit.
7. The Onset view shows the first 200 ms of the one-shot. Drag the line to set the moment
   the one-shot sounds on each hit; **Reset onset** goes back to the measured one.
8. Press **Render Layer** to save the one-shot at every hit as `Exports/<track>_layer.wav`,
   with the rate, length and session position of the track.
9. If your sampler is late, use the sampler calibration at the bottom of the section: save
   the **Calibration MIDI…**, import it in your DAW at the session start, bounce the kit
   piece, and load the bounce with **Load bounce...**. The MIDI exports then put each note
   on the aligned attack minus the offset of your sampler.

The layer and the sampler calibration also work from the agent API (chapter 14).

Next: [tempo, bars and beats](13-tempo.md).
