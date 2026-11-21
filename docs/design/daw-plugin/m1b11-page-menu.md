# M1b11: a menu in the plugin page, the hit lane, the line geometry

Ticket #559, umbrella #503 (the DAW plugin program), part of M1 (#510). The owner ran the build of
#554 in a real DAW and found three things. This note names the cause of each and the proof.

## 1. The menu of the page

A plugin window has no menu bar. The settings and the view options were only in the macOS menu, so
in the plugin nothing could be reached.

In host mode the page adds one button to the top bar (`#hostMenuBtn`, a trigram) and a panel
(`static/hostmenu.js`). The panel lists the commands of the app menu in the groups of the app
menu (Drum Spike, Edit, View, Transport, Hits, Track, Help), with their key commands.

- **One table.** The panel is made from `DSKeys.COMMANDS` (`static/keymap.js`), the table that the
  macOS menu is compiled from. Two lists take commands out: `HOST_OWNED` and the menu ids of
  `HOST_ABSENT` (`static/state.js`, #539). A new command appears in the panel with no second list.
  `DSHostMenu.audit()` names every command of the table that is in neither the panel nor a list,
  one in both, a name in a list that the table does not have, a command with no handler in
  `static/shell.js` and a check command with no check state. `tests/e2e_app/test_host_menu.py`
  fails on each (a planted command and a stale name are the negative cases).
- **Same handler.** An item calls `DSShell.menu({id})`, so the menu item, its key and its button do
  the same thing. The check marks and the greyed items use the state that the page reports to the
  macOS menu (`DSShell.menuState()`, the rules of `menu/sync.rs`).
- **Dynamic entries.** The own zoom presets and Follow Hits Of (the other tracks) are made from
  the project, as in the macOS menu. Open Recent and the Window menu are the host's.
- **Settings.** Settings is an item of the first group. It shows only the categories of host mode:
  the audio categories are hidden by `HOST_ABSENT` (#539). A last open category that is hidden falls
  back to a shown one.
- **Keyboard.** The button opens with Enter, Space or Down. In the panel: Up, Down, Home, End move;
  Right or Enter opens a menu or runs an item; Left goes back; Escape goes back and then closes and
  returns the focus to the button. No panel key reaches the key table of the page (it would nudge
  a hit or start the play).
- **Low window.** Below 520 px (#539) the panel keeps inside the window and scrolls. One list at a
  time (the menus, then the items of one menu) keeps it short.
- **Standalone.** With the mode off `hostMenuInit()` returns at once: the page has no button and no
  panel. `test_standalone_page_has_no_menu_button_and_no_panel` checks it.

- **Focus.** A command closes the panel with the focus on the menu button, so a dialog (Settings)
  gives the focus back to the button after Escape. A click on a heading or the padding of the panel
  keeps the focus in the panel (the panel has a tabindex), so Escape still closes it and no key reaches
  the page.
- **State of an open panel.** The marks and the greyed items are read when the panel opens and again
  every 250 ms while it is open (a key, or the host, can change a setting under it). The page sends
  no event to a panel; a short timer is the cheap way, and it runs only while the panel is open.
- **Commands checked for host mode.** Half-Speed Play is absent (`HOST_ABSENT`, group audio): a play
  request to the host carries no speed. Stop and Return and Stop Returns to Start act through the host:
  the page stops the host and locates it to the start of the play. They stay. The tempo commands that
  stay listed (Set Bar 1 at Playhead, Set Bar 1 at Audio Start, Set Marker at Selection, the ruler
  toggles) are kept; the tempo of the host replaces them in a later part (#560).
- **Help.** The external links of the manual use `open_url`, which the plugin implements (it opens the
  system browser); `open_link` of the About dialog is the absent one. A refusal now shows the address
  on the status line instead of nothing.
- **Settings, Detector plug-ins.** The category lists the installed detector plug-ins and their
  licences (`plugins_list`, which the plugin serves). The analysis of the plugin runs the same service,
  so a plug-in applies there too; the category stays. I did not run a plug-in inside the plugin.

Not done: keys that the DAW takes before the page (the host may keep Cmd+comma). The menu is the
way that does not depend on a key.

## 2. The hit lane after a new hit

What the owner saw: after a hit made with the pointer the whole hit lane of the track was one
colour (magenta, then yellow), the hits were dark lines, and the lane looked selected. The other
tracks had a black hit lane.

**Cause.** The tracks of the plugin project have no `color`. The lane code writes
`ctx.fillStyle = color + "0d"` (`wave.js`, `drawSpike`). With no colour that is the text
`"undefined0d"`, which is no colour, so the 2D context keeps the fill style it had. A canvas
context keeps its state between frames. The fill style at the start is black (the black hit lanes),
and after a draw it is the last `fillStyle` that the draw set: the marker of a hand-added hit
(`MANUAL_COLOR`, magenta) or of the selected hit (yellow). The next frame fills the lane with that
colour, and the halos of the hit lines (black, 3 px) are the dark lines. The hit lines drawn with
the track colour have the same problem (`strokeStyle = undefined` keeps the last value).

**Evidence.** In the test host editor (a real web view, the double click as `NSEvent`s) the
stage `editor-hit` reads the pixels of the lanes. The fault run (`DS_ARA_EDITOR_FAULT=no-color`:
the tracks have no colour and the repair is off) gives a spike lane that is 97 percent opaque, one
colour on 93 percent, for a track of the role "other" and for a snare. Before the repair the tracks
of the plugin project report `color` as `undefined` (read in the editor). The e2e test
`test_a_new_hit_does_not_colour_the_lane_of_a_track_without_a_colour` reproduces the same lane in
Chromium, in the app and in host mode.

Possible directions of the ticket that were tested and are not the cause: a range selection with a
bad end (`LN.sel` stays null after the new hit), a pointer-up that the page does not get (the web
view delivers the whole double click, down, up, click, dblclick), the lane colour of the learn or
training state, and the WebGL lane path (the wave lane carries only the lines).

**Repair.** `trackColor(name)` is the colour of a track for drawing (its own, else its role's); every draw
site of `app.js`, `wave.js`, `velocity.js` and `review.js` calls it, so a lane never builds "undefined0d".
`trackColors()` (`static/app.js`, called by `loadProject` in `session.js`) gives a track
without a colour of 6 hex digits the colour of its role (`ROLE_COLORS`, the page copy of `role_color`
in `crates/app/src/roles.rs`). A role change takes the colour of the new role as long as it was only
the default. The same defect is possible in the standalone app (a track record without a colour,
for example in a project written by an older or another writer), so the repair is not behind the
host-mode switch. A track that has a colour keeps it.

Not done: why the service of the plugin project writes no colour. The page repair is enough for
the page and covers every project, and the service is outside the scope of this ticket.

## 3. The lines cut after the zoom and the track height

What the owner saw: after the zoom and the track height changed, the grid lines and the hit lines
of a track spanned a middle band of the lane; the waveform used the full lane.

**Cause.** A row can have its own wave height (the Zoom Toggle and the drag of a row,
`setRowHeights`). The page writes the new size of the canvas store at once and draws at once. The
row is laid out later, so the draw used the old measured height of the row (the lines spanned
120 px of a 180 px lane, two thirds). The observer then measured the new height, but the key that
decides whether to size and draw again (`sizeKey` in `render.js`) had only the global height, so
nothing drew the lines again until some other change did. The GL wave is drawn from the layout, so
it was right: the lines and the wave disagreed.

**Evidence.** Reproduced in Chromium in the app and in host mode (`test_lane_geometry.py`: after
`setRowHeights({track: 1.5})` the lines ended at 119 of 180 px). In the test host editor the stage
`editor-geometry` (the host resizes the window below and above 520 px, thirteen steps of zoom, track
height and row height) fails in its fault run with "the lines span 0 to 55 of 84 px (67 percent)".

**Repair.** `sizeKey` includes the measured wave height of each row. The same defect is in the
standalone app; the repair is there too. No expected value of an existing page test changed.

## What the tests do not prove

- The panel in the real web view with real single clicks and real keys. The test host hands a double
  click to the page but not a single click, so `editor-menu` uses DOM mouse events and key events.
  The Chromium tests use real Playwright clicks and keys, which is not the web view of a DAW.
- The service of the plugin still writes no colour into its tracks. The page covers it at load, at a
  role change and at every draw site (`trackColor` in `static/app.js`); the service is a later ticket.

## The stages

`plugins/ara/check.py`: `editor-hit`, `editor-geometry`, `editor-menu`, each with a `-fault` run that
fails for the named reason and a run with no report that fails with "no usable run". The test host
(`ara-host editor`) got the modes `hit`, `menu`, `geometry`: the page asks the host for what only a
host can do (a window of another size, the double click of the pointer as `NSEvent`s).

Limit of the test host: it hands a double click to the web view, but a single press and release
at a new place does not arrive in the page (the first pair of a series is dropped, a single one is
not delivered). The menu stage therefore uses DOM mouse events for the clicks and the keyboard
events for the keys; the hit stage uses the real double click.

The screen of the machine must be unlocked: a hidden page draws no frame and the stages say so.
