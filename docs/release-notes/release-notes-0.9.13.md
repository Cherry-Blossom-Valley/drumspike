Changes since 0.9.12:

### Analyze
- Analyze works again for a project whose track names carry numbers on both sides of a dash, for example "Tom 1-2" or "Kick 2-1". Such a name was read as a track range and the analysis stopped with "the pattern ... matches no track". The role of the track was never the cause. A track's own name now always names it; the name patterns for scripts (globs and ranges) work as before.

No other change in the app. The tests and the landing page changed only.
