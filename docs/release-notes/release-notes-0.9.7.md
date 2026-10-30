Changes since 0.9.6:

### Detection
- The placement fix of 0.9.6 moved some hits too far ahead, mostly on snare. A hit now moves at most 3 samples earlier than its 0.9.5 placement. On real recordings no hit moves later, and the detected hits are the same as before. Projects analysed before keep their hits until Re-analyze.

### General
- The async runtime of the app is updated to a version with a security fix (tokio 1.53.3, RUSTSEC-2026-0386).
