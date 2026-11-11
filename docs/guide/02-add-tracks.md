# 2. Bring in a session

Drum Spike reads your existing audio. The files stay where they are. Drum Spike references them
and keeps the BWF time stamps, so your DAW session and the project stay in sync.

## Steps

1. **File > New Project…** (`Cmd+N`).
2. **File > Add Tracks…** (`Shift+Cmd+I`).
3. Pick WAV files, or pick a session folder such as a Pro Tools "Audio Files" folder.
4. Drum Spike gives each track a role: kick, snare, tom, hat, cymbal or other. The role comes
   from a kit preset, or from the file name. A file name like "k in", "s top" or "t1" says the
   role directly. When the name gives none, Add Tracks suggests a role from the audio for kick
   and tom files; the row is marked "from audio", and you can change the role before the add.
   Drum Spike never offers its own exports as drum tracks.
5. The buttons **Drums only** and **All** above the list check only the files with a drum
   role, or every file. The box **Copy the audio into the project** decides where the audio
   lives: with the box, the files are copied in and the project holds everything it needs and
   can be moved or sent; without it, the files stay where they are (referenced). The select
   picks a kit preset, and **Save as kit preset** stores the names, roles and notes of this
   list as a kit preset for the next session.
6. Check the roles. To change one, open the Inspector with **View > Show Inspector** (`Option+I`),
   section Track. The hint there says where the role came from. A change analyzes the track again.
   **Edit > Undo** (`Cmd+Z`) brings everything back.

## Start screen and recent projects

At launch, a splash shows for about 5 s; a click or any key ends it early. The start screen then
lists your recent projects; a click opens one. **Clear Recent** under the list, or the Clear Recent
entry at the end of the File menu's recent projects, empties the list. The projects themselves stay on disk.
**File > Open Most Recent** (`Shift+Cmd+O`) opens the first entry of the list.

A project is one `.drumspike` document. It has its own document icon in Finder; a double-click
on a project file opens it in Drum Spike.

## Which tracks to load

Load every close mic: the kick, snare, tom and hi-hat mics are the tracks
you trigger, and each one is a bleed source for the others. Overheads, ride
and crash help the rules as sources too. Room mics and sample tracks usually
stay out. See [Get the best results](best-results.md) for the full list and
the reasons.

The demo session has 13 tracks and 62.5 s of audio. It is the example for the rest of the guide.

![The Add Tracks dialog with a session folder selected; the 13 tracks are listed with their roles.](img/02-add-tracks.png)

![The Inspector, section Track, with the role and the hint that says where the role came from.](img/02-inspector-role.png)

Next: [detection](03-detection.md).
