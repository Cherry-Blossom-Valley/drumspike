Changes since 0.9.3:

Drum Spike now lives at [Cherry-Blossom-Valley/drumspike](https://github.com/Cherry-Blossom-Valley/drumspike) as an open-source project. This is the first release built from the new repository.

### Tempo
- Detect Tempo suggests and shows whole-number tempos only.
- Detect Tempo: on a session with a kick track only, the suggested tempo is no longer half or double the played one.
- Typing a start tempo on the Tempo ruler moves bar 1 onto the beat the hits support, as one undo step with the tempo.
- New: Track > Set Bar 1 at Audio Start puts bar 1 at 0 s, the session start in the DAW.
- The bar and beat grid shows 8th lines when a beat is wider than 24 px and 16th lines above 48 px.

### Playback
- The playhead moves without visual jumps: small clock corrections are absorbed smoothly; it jumps only on a seek, play, pause, speed change or a correction above 100 ms.
- The playhead clock follows the audio device's own playback time, so the playhead keeps a steady pace under CPU load, and the app sends its position 25 times a second.

### Start
- The splash shows for 5 s from its first frame; a click or any key ends it early.
- Clear Recent, on the start screen and in File > Open Recent, empties the list of recent projects.

### Project
- New public repository with a contributor guide, a code of conduct, a security policy and issue templates.
- The licence terms of the code, the third-party parts and the media are in docs/licensing.md.
