# Drum Spike 0.9.17

Changes since 0.9.16:

### Detection

- Sessions that are not time-aligned are untouched again. The 0.9.16 rule for time-aligned sessions also fired on close mics of ordinary sessions, where a hi-hat or snare bleeds into the nearest tom mic almost without delay, and it changed tom hits there. A session now counts as time-aligned only when a source reads aligned on every tom it coincides with, decided once per session; on the public ENST-Drums set every one of the 210 items is identical to the build before the aligned-session rule (#313).
