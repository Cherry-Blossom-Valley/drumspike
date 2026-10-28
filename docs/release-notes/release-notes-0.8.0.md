Changes since 0.7.0:

### Analysis
- On a kick close mic, the analysis now also finds quiet kicks that the model misses. It does not add a
  quiet hit that is only the bleed of a tom or the snare. On the DRT velocity ramp it finds 30 of 32 kicks
  instead of 7. The Inspector shows "Kick: N quiet hits added". A project analysed before keeps its
  hits until you Re-analyze the kick track.
- On a snare close mic, the analysis now also finds quiet snare hits that the model files under another
  drum. A found hit 5 dB or more under the median level of the snare hits stays out (most of those are
  kick bleed). On the three real sessions it adds 0 to 3 hits per session, each a real stroke; the kick
  and tom hits do not change. It needs a
  model analysis (not the DSP fallback). The Inspector shows "Snare: N quiet hits added". A project
  analysed before keeps its hits until you Re-analyze the snare track.

### Saving
- A save never overwrites a project file that changed on disk since you opened or saved it (for
  example from another app or another computer). The status line says so and offers Reload from disk
  or Save anyway; the autosave keeps a backup and does not write over the file.
- Close and Quit ask the same question for a project with unsaved changes whose file changed on disk:
  Save anyway, Reload from disk, close or quit without saving, or Cancel. Nothing is written before
  you choose.

### Status line
- An error on the status line stays for its hold time; a background message (for example from the
  histograms or the analysis) no longer replaces it at once.

### Performance
- After you close a project, the memory of its lane and meter canvases is given back, also when a
  last draw was still queued at the close.
