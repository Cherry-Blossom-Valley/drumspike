# Get the best results

The chapters tell you how each step works. This page collects the settings
and the order of work that give the cleanest results: what to load, what to
name it, and what to check before you export.

## Which tracks to load

Load every close mic you have. These are the tracks you trigger, and each one
is a bleed source for the others:

- kick (in, out, sub), snare (top, bottom), every tom, and the hi-hat. The
  tom bleed rule and the cross-track rule judge every tom hit against the
  kick, snare, hi-hat and the other tom mics. The snare ghost-note gate
  checks its soft hits against the kick and tom hits. A missing source mic
  means more false hits on the other tracks.
- Overheads, ride and crash are optional, but they help the rules: the flam
  rule reads every non-snare track as a bleed source, and cymbal hits vote
  for bar 1 in the tempo detection. The overheads' own hits are less reliable
  than the close mics.
- Room mics usually stay out. Their role is other, and Add Tracks does not
  tick them.

Keep out of the project:

- Sample or trigger tracks. A name with `samp`, `smpl`, `smp`, `ssd` or
  `trig` is a sample or trigger track to Drum Spike: it never acts as a
  source or a target of the bleed rules.
- Earlier Drum Spike exports. A file named `_spike`, `_audio_spike` or
  `_layer` is the app's own output; the scan offers it as no drum track.
- Files at another sample rate. Drum Spike reads 44.1, 48 and 96 kHz, and a
  rule leaves a source mic at another rate out. Convert the file in your DAW.

## Names

Give each track one drum and one position: `k in`, `k out`, `s top`, `s bot`,
`t1`. Or pick a kit preset in the Add Tracks dialog and let it name the
roles. The tom bleed rule reads the drum from the track name. When a name
does not tell the drum, the rule is off, and the analysis says why. Two mics
of one drum, `t1 top` and `t1 bot`, never count as bleed of each other.

## Analyze all tracks together

Run **Hits > Analyze All** over the whole session, not one track at a time.
The cross-track rules read the hits and the candidate lists of the other
tracks, and those exist only after the tracks are analyzed. Analyze all
tracks, so every source mic has its hits when the rules run.

## Check the roles before the analysis

Check the role of every track before you analyze (chapter 2). A wrong role
gives wrong rules: the flam rule runs only on snare tracks, and the bleed
rules read only kick, snare, tom and hi-hat close mics. To change a role, use
the Inspector (chapter 2); the change analyzes the track again.

## After the analysis

Work in this order:

1. Check the doubtful hits first (chapter 9): press `D` and step through the
   queue, lowest confidence first.
2. Set the loudness cut per track in the Inspector (chapter 8). It leaves the
   quiet bleed out of the exports.
3. A doubtful tom hit: press `O` for **View > Other Mics Under Hit**
   (chapter 10). The panel shows the other mics at the hit time, so you see
   whether the hit is bleed.

## Fix by hand last

Your hand edits survive a new analysis: an added, moved or restored hit keeps
its place, and the rules never mark it again. So you can fix hits at any
time. One order matters: a hand velocity wins over the floor and the
ceiling, and Recompute Velocities keeps it. So set the floor and the ceiling
first, then set velocities by hand (chapter 8).

## Tempo

Detect the tempo after the kick and snare tracks have hits (chapter 13):
**Track > Detect Tempo…** reads the kick and snare hits, and a named
cymbal track votes for bar 1. Check bar 1 and the suggested tempo steps
before you export the MIDI.

Next: start with [chapter 1](01-why-sample-exact.md), or go back to the
[guide README](README.md).
