# App icon source

`drumspike-icon.svg` is the source of the app icon (owner choice 2026-10-11, concept "Transient"):
an orange spike stands exactly on a sample point at the onset of a blue decaying drum waveform.
Colours are the app colours (`--accent` #4da3ff, `--orange` #ff9f43).

Make the icon files again after a change of the SVG (run in `apps/desktop`):

```bash
npx tauri icon src-tauri/icons/source/drumspike-icon.svg -o /tmp/ds-icons
```

Then copy `32x32.png`, `64x64.png`, `128x128.png`, `128x128@2x.png`, `icon.png` and `icon.icns`
from that folder into `src-tauri/icons/`. The other platform files are not used.
