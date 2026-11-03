# Drum Spike 0.9.16

Changes since 0.9.15:

### Detection

- On a session whose tracks were time-aligned (Auto-Align 2 or the like), snare and kick bleed on the tom close mics is marked as bleed again. The alignment removes the bleed's arrival delay, so the rule now measures per mic pair whether the session is aligned and then judges by level; once one pair of a tom is aligned, every pair of that tom is (#296, #309).
- A tom stroke played together with the snare or the kick stays: a hit louder than any bleed the pair produces on the session's own clear cases is the tom's own stroke (#309).
