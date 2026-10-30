Changes since 0.9.7:

### Tempo
- The Tempo and Meter rulers show the tempo and meter in force again. In 0.9.7 the value went missing while the lanes followed playback, and it stayed missing after a stop.
- A MIDI export of a project without a tempo now asks first. The notice names the three ways to set the tempo (Track > Detect Tempo, File > Import Tempo Map, the Tempo ruler) and offers Set Tempo…, Export anyway and Cancel. Before, the export used 120 BPM without a word, so the notes landed late in a DAW session with another tempo. The notice covers the Export dialog, File > Export Again and the Drag MIDI tile; the agent API reply carries the same warning.
