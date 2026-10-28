#!/bin/sh
# P3-4b (arm64-mac only): the Finder path of the debug .app. Cold start: `open -a <app> <document>` starts the
# app and the project opens; warm: `open -a <app> <folder>` opens a second project in the running app.
# The proof is the Open Recent file of an own home folder (DRUMSPIKE_HOME), which only the page's open writes.
# Needs the debug bundle (G3) and the real session folder (see self-test.sh). Exit 0 = both opens happened.
set -eu
root=$(cd "$(dirname "$0")/../../.." && pwd)
cd "$root"
session=${DRUMSPIKE_SELFTEST_SESSION:-"$HOME/.cache/drumspike/sessions/Session A"}
app=${DRUMSPIKE_SELFTEST_APP:-"$root/target/aarch64-apple-darwin/debug/bundle/macos/Drum Spike.app"}
bin="$app/Contents/MacOS/drumspike-desktop"
[ -x "$bin" ] || { echo "no debug app at $app (run G3 first)" >&2; exit 2; }
work=$(mktemp -d)
# copied audio: a referenced file in ~/Downloads makes macOS ask for access when the app starts from the Finder
a=$(uv run python apps/desktop/selftest/make_project.py --out "$work/a" --name "Cold Song" --from "$session" --tracks 2 --mode copied)
b=$(uv run python apps/desktop/selftest/make_project.py --out "$work/b" --name "Warm Song" --from "$session" --tracks 2 --mode copied)
mkdir -p "$work/home"
stop() { pkill -f "$bin" 2>/dev/null || true; }
trap stop EXIT
stop
waitfor() {   # waitfor <text> <seconds>
  i=0
  while [ "$i" -lt "$2" ]; do
    grep -q "$1" "$work/home/recent.json" 2>/dev/null && return 0
    sleep 1; i=$((i + 1))
  done
  return 1
}
start=$(date +%s)
open -a "$app" --env DRUMSPIKE_HOME="$work/home" --env DRUMSPIKE_OPEN_LOG="$work/open.log" "$a/Cold Song.drumspike"
waitfor "Cold Song" 30 || { echo "FAIL cold start: the project did not open (log: $(cat "$work/open.log" 2>/dev/null))" >&2; exit 1; }
echo "cold start, open -a <app> <document>: project opened after $(( $(date +%s) - start )) s"
open -a "$app" "$b"
waitfor "Warm Song" 20 || { echo "FAIL warm: the folder did not open" >&2; exit 1; }
echo "warm, open -a <app> <folder>: second project opened"
echo "paths the shell received:"; sed 's/^/  /' "$work/open.log"
