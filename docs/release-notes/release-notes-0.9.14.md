# Drum Spike 0.9.14

Changes since 0.9.13:

### Analyze

- Hit placement: a hit no longer sits on the bleed or the slow drift before its attack. The placement now finds the first steep rise after the walk's foot and puts the hit where the waveform visibly starts (#293, #294). Kick hits that sat 3 to 5 samples before the visible start, and snare hits that sat at the start of a drift, move onto it; no hit moves earlier. Projects analysed before keep their hits until you run Analyze again.

### App

- Splash screen: the first paint is reported by a timer when the window starts hidden behind another one, so the splash closes after 5 s instead of 15 s (#203).
