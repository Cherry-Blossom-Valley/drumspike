# Product video: the 90-second tour

Built by `tools/media/compose.py` from the part 1 recording and clips of explainers 1 and 4 (and
later parts). Narration for `tools/media/tts.py` (one `##` per scene; `>` lines say which clip runs).
Target: 80 to 95 s.

## pv-hook

> Explainer 1: the snare and the late sample; the comb notches appear.

Your trigger is one millisecond late. You hear it: a thin, hollow snare.

## pv-what

> Title card "Drum Spike", then the editor with a full session.

Drum Spike finds every drum hit in your close-miked tracks and puts it on the first sample of its attack.

## pv-in

> Recording part 1, scenes p1-add and p1-analyze, sped up 2x.

Add your tracks. Drum Spike reads the roles from your file names, and its detector listens to each track for that drum only.

## pv-exact

> Explainer 4, scene s05-walk-back, then the 5 ms zoom from the recording.

Then it walks back to the first motion of each attack. Soft and loud hits land on the same point.

## pv-fix

> Short cuts (2-3 s each) from later recordings: doubtful hits (D), the velocity lane, Forget This.

Check the doubtful hits first, shape the velocities, and teach it by example.

## pv-out

> Recording part 1, scene p1-export; the files in Finder and a DAW session with the spikes lined up.

Export MIDI and spike files on your session's time, and your trigger fires on time.

## pv-end

> Title card: "Drum Spike. Free and open source." and the download address.

Drum Spike. Every drum hit, on the first sample.
