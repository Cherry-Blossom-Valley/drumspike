# Product video: the 90-second tour

Built by `tools/media/compose.py` from the part 1 recording and clips of explainers 1 and 4 (and
later parts). Narration for `tools/media/tts.py` (one `##` per scene; `>` lines say which clip runs).
Target: 80 to 95 s.

## pv-hook

> Explainer 1: the snare and the late sample (8.0-11.5 s), then the comb notches (62.0-65.35 s).

Your trigger is one millisecond late. You hear it: a thin, hollow snare.

## pv-what

> Title card "Drum Spike" (2.2 s), then recording part 1, the export-again scene (57.13-62.66 s): the editor with the full session.

Drum Spike finds every drum hit in your close-miked tracks and puts it on the first sample of its attack.

## pv-in

> Recording part 1, scene p1-add (6.20-25.49 s), sped up 2x.

Add your tracks. Drum Spike reads the roles from your file names, and its detector listens to each track for that drum only.

## pv-exact

> Explainer 4, scene s05-walk-back (73.0-77.46 s), then recording part 1, the 5 ms zoom of p1-zoom (41.65-45.68 s).

Then it walks back to the first motion of each attack. Soft and loud hits land on the same point.

## pv-fix

> Explainer 10, the marked hit and the other mics under it (155.5-157.7 s), then recording part 2, the velocity lane (79.0-81.2 s), then recording part-learn, scene pl-grey with the marked hit and its near copies grey (27.0-29.21 s).

Check the doubtful hits first, shape the velocities, and teach it by example.

## pv-out

> Recording part 1, scene p1-export (45.84-53.53 s).

Export MIDI and spike files on your session's time, and your trigger fires on time.

## pv-end

> Title card: "Drum Spike. Free and open source." and the download address (4.69 s).

Drum Spike. Every drum hit, on the first sample.
