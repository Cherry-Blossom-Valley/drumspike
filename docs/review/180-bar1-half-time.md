# #180: Detect Tempo bar 1 under a half-time snare

Probe: `crates/core/tests/tempo_detect_bar1_probe.rs` (ignored; `DRUMSPIKE_TEMPO_SAMPLES` set).
Reference bar 1 is the session's tempo map, 0 s. Offset = `bar1_s` / beat of the detected map.
Hits are the DSP detector's. Sessions are named A, B, C only.

## Step 1: counted lines (before, rule 2)

| Session | backbeat period | BPM | bar 1 (s) | offset (beats) | class |
|---|---|---|---|---|---|
| A | 4 beats (half time) | 134 | 0.4443 | 0.99 | one beat off |
| B | 2 beats | 126 | 0.9555 | 2.01 | two beats off (#317, separate cause) |
| C | 2 beats | 126 | 0.0021 | 0.00 | on the reference |

Snare hits on the beat, by beat class mod 4 against the suggested bar 1 (class 0 = suggested beat 1):

| Session | kick | snare |
|---|---|---|
| A (before) | 94 21 69 59 | 8 117 21 45 |
| B | 85 9 80 12 | 24 115 21 117 |
| C | 20 6 88 5 | 2 114 15 105 |

Split by backbeat period: period 2 beats (B, C): 1 on the reference, 0 one beat off, 1 two beats
off, 0 other. Period 4 beats (A): 0 on, 1 one beat off, 0 two beats off, 0 other.

ENST: the repo holds no ENST item with a reference bar 1 for Detect Tempo (the ENST gates score
hits, not a tempo map), so no ENST bar-1 count exists, before or after. The synthetic trains of
`tests/tempo_detect.rs` (known bar 1) stand in; they are unchanged (see below).

## The rule

`bars` in `crates/core/src/tempo_detect.rs`, 4/4 branch: "The snare picks the pair of beats: its
parity is the backbeat, the other is 1 and 3." Bar 1 is chosen by the kick votes between the two
beats of the pair that the snare does NOT sit on. Under a half-time snare (one snare per bar, on
beat 3) the snare sits on beat 1 and 3 of the bar, so the pair that is searched is beats 2 and 4:
bar 1 cannot be found, and the pick is one beat late (Session A: snare class 1 holds 117, its
partner class 3 holds 45).

## Step 2: the fix (rule 3, `DRUMSPIKE_TEMPO_RULE`)

When the busier beat of the snare's pair holds at least 2 times the snares of its partner and at
least 8 (`HALF_TIME_RATIO`, `HALF_TIME_MIN_SNARES`), the snare is on beat 3: bar 1 is two beats
from it, the alternative is the snare beat. Rule 2 (`DRUMSPIKE_TEMPO_RULE=2`) pins the old
behaviour. Session A's ratio is 2.6 (117 against 45); B and C are 1.0 and 1.1.

## After (rule 3)

| Session | bar 1 (s) | offset (beats) |
|---|---|---|
| A | 0.0000 | 0.00 |
| B | 0.9555 | 2.01 (unchanged, #317) |
| C | 0.0021 | 0.00 (unchanged) |

Tests: `cargo test -p drumspike-core --test tempo_detect --test tempo_detect_real` with the
sessions: 23 + 2 passed, 1 ignored (#317). New `bar_1_of_a_half_time_snare_on_beat_3_is_on_the_bar_line`
fails with `DRUMSPIKE_TEMPO_RULE=2` (bar 1 1.447 s, want 1.0 s) and passes with rule 3. The Session A
bar-1 bound in `tempo_detect_real.rs` is tightened from 0.5 s to 0.02 s.
