Changes since 0.9.8:

### Finder
- Drum Spike project files (`.drumspike`) have their own document icon in Finder: a page with the Drum Spike mark. A double-click on a project file opens it in Drum Spike.

### Data safety
- A failed autosave is now shown on the status line: "Autosave failed: … Your edits are not backed up; save the project." Before, the error went only to the system log, so a full disk or a folder without write access could stop the backups without a word. The message comes once per failure; the next good autosave or a save clears it. The agent API project status carries the same error.

### Agent API
- A Pro Tools session folder can be the input: `tracks_scan` and `tracks_add` take the session folder and use its `Audio Files` folder.
- Tracks can be chosen by name pattern: exact names, globs (`t*`) and numbered ranges (`t1-t4`), in `tracks_add`, `analyze_start` and `export_all`. A pattern that matches no track stops with an error that names it.
- `export_all` takes a list of roles, instead of or next to track names. A role that no track has stops with an error.
- The agent API guide has a worked example: one script that imports a session folder, picks the tracks, sets the parameters per drum, analyses, and exports per role.
