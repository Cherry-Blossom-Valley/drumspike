# Explainer 13: Tempo, bars and beats

Chapter 13 of `docs/product/feature-path.md`. Narration for `tools/media/tts.py`: one `##` heading
per scene; lines that start with `>` are visual notes and are not spoken. Target length: about
2 minutes. The picture is the real output of the app on a generated session (kick and snare
tracks): the hits (`events_get`) and the answer of Detect Tempo (`tempo_map_detect`: the tempo, the
other tempos the beat could be, the confidence, the position of bar 1) are read from the service at
render time, and the grid is drawn from that map. The narration calls it "a song", "your drums",
"your session".

Facts used (checked against the code at release 0.9.33, not against a commit; the sources are
`crates/core/src/tempo.rs`, `crates/core/src/tempo_detect.rs`, `static/tempomap.js` (the Detect
Tempo dialog and the grid), `static/keymap.js` (the commands and keys) and `docs/guide/13-tempo.md`):
- Hits alone have no bars: the bars and beats come from a tempo and meter map, which the project
  stores (`DOC_KEY` of `tempo.rs`) and the MIDI exports carry: `docs/guide/13-tempo.md`.
- Track > Detect Tempo… (`detect_tempo`) suggests the tempo, bar 1, the meter and the tempo steps
  from the hits of the kick and snare tracks, which are analysed first: `static/keymap.js`.
- The detection needs at least `MIN_HITS` = 8 kick hits and 8 snare hits, else it warns that half
  or double the tempo is as likely: `crates/app/src/tempo.rs`, `crates/core/src/tempo_detect.rs`.
- The dialog writes the tempo in whole BPM with its confidence, and lists up to 3 other whole BPM
  numbers the beat could be (`The beat could also be:`): `static/tempomap.js` (`tmDetectDialog`).
- The dialog answers with three buttons: `Use` the detected tempo, `Half time:` the half of it, shown
  when that half is above 40 BPM, and `Use own tempo` for the number you type: `static/tempomap.js`,
  `static/index.html`. Half or double the tempo is the usual fault of a detection.
- When the confidence of bar 1 is low, the dialog says so (`The bar 1 confidence is low`) and offers the
  other candidate, two beats later in 4/4: `static/tempomap.js`.
- Track > Set Bar 1 at Playhead (`set_bar1`) moves bar 1 to the playhead or the selected hit; Track >
  Set Bar 1 at Audio Start (`set_bar1_audio`) puts it at 0 s, the DAW convention; the tempo, the steps
  and the meter stay; each is one undo step: `static/keymap.js`.
- A tempo step is a place where the song changes speed; a step needs at least `DEFAULT_MIN_BARS` = 4
  bars and a change of at least `DEFAULT_MIN_STEP_PCT` = 2 percent to be proposed:
  `crates/core/src/tempo_detect.rs`. View > Tempo Ruler (`ruler_tempo`): double-click adds a step:
  `static/keymap.js`.
- File > Import Tempo Map… (`import_tempo_map`) takes the tempo and time-signature events of a MIDI
  file from the DAW as the map; one undo step: `static/keymap.js`.
- `G` (`grid`) is View > Show Bar and Beat Grid: light bar and beat lines under the hit lines; a beat
  wider than 24 px also gets its 8th line, and above 48 px its 16th lines. There is no snap, the grid is
  a guide: `static/keymap.js`, `static/tempomap.js` (`drawGridLines`).
- View > Bars|Beats Ruler (`ruler_beats`) shows the bar numbers and beat ticks: `static/keymap.js`.
- The MIDI exports carry the map, so the DAW shows the same bars: `docs/guide/13-tempo.md`.

## s01-problem

> Kick and snare hits of one song as stalks on two lanes, in time, with no bars and no beats.

Your drums are recorded and every hit is found. But a song is counted in bars and beats, and hits alone have none. Without a tempo, your DAW cannot line the drums up on its grid.

## s02-detect

> The Detect Tempo dialog, as the app writes it: the start tempo in whole BPM with its confidence, the other tempos the beat could be, and the three buttons Use, Half time, Use own tempo.

Choose Detect Tempo. Drum Spike reads the kick and snare hits and proposes a tempo, in whole beats per minute, with the other tempos the beat could be. Half time is the usual mistake, so the dialog offers it with one click. Or type your own.

## s03-bar1

> The Bars|Beats ruler over the lanes with the bar 1 marker. The two commands Set Bar 1 at Playhead and Set Bar 1 at Audio Start; the marker moves.

Then say where the first bar is. Put bar one at the playhead, where the song really starts. Or at the start of the audio, as your DAW has it.

## s04-steps

> The Tempo ruler: a step where the song changes speed. Below it the command File > Import Tempo Map.

Does the song change speed? Add a tempo step where it does. Or, if your DAW already knows the tempo, import its map from a MIDI file instead.

## s05-grid

> The same hits with bar lines and beat lines under them, the bars numbered; the key G; the words "a guide, no snap".

Press G and the bars and beats appear under your hits. It is a guide. Nothing snaps, and no hit moves.

## s06-result

> The hits on their bars and beats; then the word Export and "the same bars in your DAW".

Export the MIDI, and the file carries the tempo map. Open it in your DAW, and the bars are the same bars. Your drums sit on the beat, where you counted them.
