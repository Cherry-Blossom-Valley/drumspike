Changes since 0.9.9:

### Tempo
- A typed tempo is always used. Before, a start tempo typed in the Tempo ruler could be refused without a clear message, and the old tempo stayed: when the downbeat fell on the very first hit, the new bar 1 landed a fraction of a millisecond before the audio starts. Bar 1 now stays at 0 s in that case.
- The Detect Tempo dialog offers three answers: the detected tempo, half time (when that is above 40 BPM; the detector can lock on double time), or your own tempo. Half time and your own tempo work like a start tempo typed in the Tempo ruler: bar 1 is fitted to the kick and snare hits again.

### Feedback
- A Feedback button in the toolbar sends a bug report, an improvement or other feedback to the Cherry Blossom Valley feedback gateway, which opens an issue in the Drum Spike repo. The form shows a preview of exactly what is sent, and it blocks a text that holds a file path.
