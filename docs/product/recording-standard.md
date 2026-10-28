# Recording standard

Every explainer recording must let a viewer follow each step on the real software and see how it
is done. The narration names what happens; the picture must show it. The recorder
(`tools/media/record.py`) enforces the rules below and FAILS a run that breaks one.

## The rules

- **Zoom to the action.** A `zoom_to` step sets the view to a preset span of 500 ms to 5 s around
  the time of the step and moves the track into the middle of the lanes, before the action. The
  whole 60 s view is for overviews only.
- **A visible pointer.** The overlay pointer glides (450 ms here, 300 to 600 ms allowed) to the
  click or drag target before the click, rings on the click, and draws a line for a drag. A click
  never happens off screen or without the pointer there.
- **Keys stay on screen.** The key overlay shows the pressed key for at least 1 s (1.6 s in the
  recorder), readable at 1080p.
- **The result holds.** After each action the result stays on screen for at least 1.5 s at the
  same zoom: the grey hit, the added hit with its flag, the histogram change.
- **The panel the narration explains is large.** The frame of such a scene is the panel itself
  (`area_sel` names it; the gate covers it). A 2x inset of the capture region is not built yet.
- **The action is visible.** Per action scene the recorder writes a before and an after PNG of the
  action area to `~/.cache/drumspike/media/rec/<name>/<scene>-{before,after}.png`. The gate FAILS
  when fewer than 0.5 % of the area's pixels changed.
- **The measured window.** In a zoomed scene the action area is the window the viewer looks at: the
  hit's x (the zoom centres it) plus or minus 300 capture pixels (300 px at the 2x capture, 150 CSS
  px each side), over the spike lane and the wave lane of the zoom's track, from the top of the spike
  lane to the bottom of the wave lane. The same window for every zoomed scene; the 0.5 % floor does not
  change. A scene without a zoom keeps its panel or its first track.
- **Drift.** A scenario may set `drift_limit_ms`; the Learn recording uses 10 ms.

## The scenario

A scene lists its steps in `tools/media/scenarios/<name>.json`:

- `{"do": "zoom_to", "track": "s_top", "t": 16.0938, "span": 5}` before the action on that track.
- `area_sel` on a scene names the element its before/after PNGs cover. Use it for a panel:
  `"area_sel": "#inspector"`. A zoomed scene without `area_sel` measures the window of the rule above;
  a scene with neither measures the row of its first track.
- A `hold` of 1.5 s or more after each action step.

## The check

Each run prints the share of changed pixels per action scene, for example
`pl-add: 14.20 % of the action area changed`. The coordinator looks at the PNGs and the MP4 before
the merge; the PR comment names the frames, not the files.
