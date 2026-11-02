# Drum Spike 0.9.14

Changes since 0.9.13:

### Analyze

- Hit placement: a hit sits on the last sample before the stroke's own rise, where the waveform visibly leaves the line (#293, #294). Before, a hit could sit on the bleed or the slow drift in front of a snare stroke, up to 2 ms early, and a kick hit a few samples off its rise. The rule was set against hand-placed kick and snare hits of a real session: the snare's mean distance to the hand-placed position goes from 5 samples to about 1, the kick's from 0.6 to 0.5 (at 48 kHz). Projects analysed before keep their hits until you run Analyze again.

### App

- Splash screen: the first paint is reported by a timer when the window starts hidden behind another one, so the splash closes after 5 s instead of 15 s (#203).
