# Explainer 1: Why one millisecond matters

Chapter 1 of `docs/product/feature-path.md`. Narration for `tools/media/tts.py`: one `##` heading
per scene; the text under it is spoken. Visual notes are in the `>` lines (not spoken). Target
length: about 2.5 minutes.

Facts used (check them in the video): 1 ms is 48 samples at 48 kHz; sound travels about 34 cm in
1 ms (343 m/s); a signal summed with a copy of itself delayed by 1 ms cancels at 500 Hz, 1500 Hz,
2500 Hz and so on (odd multiples of 1 / (2 x 1 ms)); with 0.1 ms the first notch is at 5 kHz.

## s01-hook

> A snare close-mic waveform; a sample hit appears on top of it, slightly late.

A drummer hits the snare. A trigger plays a sample together with the snare microphone. The sample is one millisecond late. That sounds like nothing. But you can hear it. This video shows why, and what Drum Spike does about it.

## s02-one-ms

> Zoom into 1 ms on a timeline: 48 sample dots at 48 kHz; then a microphone moving 34 cm away.

One millisecond is short. At 48 kilohertz, it is 48 samples. In one millisecond, sound travels about 34 centimetres. So a sample that is one millisecond late sounds like a second microphone, 34 centimetres further away.

## s03-sum

> Two copies of the same snare wave, one shifted; their sum drawn below; some parts add, some cancel.

When two signals play together, they add. If both are the same sound at the same time, the sum is simply louder. If one copy is late, the peaks of one meet the valleys of the other at some frequencies. There, the two copies cancel.

## s04-comb

> A spectrum of the sum: deep notches at 500, 1500, 2500 Hz, like the teeth of a comb; the snare's spectrum behind it.

With a delay of one millisecond, the first cancellation is at 500 hertz. The next ones are at 1500 and 2500 hertz, and so on. Engineers call this a comb filter, because the spectrum looks like a comb. These notches fall right where a snare has its body and its crack. The snare sounds thin and hollow.

## s05-shift

> The delay slider moves from 1 ms to 0.1 ms to 0; the notches move up and disappear.

A smaller delay moves the first notch up. At one tenth of a millisecond, it is at 5 kilohertz, still inside the snare's crack. Only at zero delay do the two copies add up cleanly. So the trigger must fire on the exact sample where the hit starts.

## s06-threshold

> A loud hit and a soft hit; a horizontal threshold line; the soft hit crosses it later; a bleed bump from the kick also crosses it.

Many tools find a hit with a level threshold. A loud hit crosses the threshold early. A soft hit rises more slowly and crosses it later. So the error changes with every hit, with the dynamics of the drummer. And a loud kick that leaks into the snare microphone can cross the threshold too. Then the trigger fires on bleed.

## s07-first-motion

> The detector marks "hit near here" on the snare track; a marker walks back sample by sample to the first motion of the attack; the same point for the loud and the soft hit.

Drum Spike works in two steps. First, it listens to each track for onsets: a sudden rise of energy in that drum's own frequency bands. Its gates and the other mics tell a snare hit from the bleed of a kick, so it finds the real hits. Then Drum Spike walks back in the waveform to the first motion of the attack. It puts the hit on that sample. A soft hit and a loud hit get the same point: the start of the attack.

## s08-result

> Per track: a MIDI lane and a spike track lined up under the close mic; the sample now on time; the comb notches gone.

For every drum track, you get a MIDI file and a spike file with one hit per real stroke, on the first sample of the attack. Your trigger fires on time. The next video shows how Drum Spike finds that first sample.
