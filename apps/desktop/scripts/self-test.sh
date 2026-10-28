#!/bin/sh
# G5 (P3-7a, arm64-mac only): run a self-test scenario in the built debug .app and check the report.
#   apps/desktop/scripts/self-test.sh slice        (a name in apps/desktop/selftest/, or the path of a .json file)
# Needs the debug bundle (G3: npx tauri build --debug --target aarch64-apple-darwin --bundles app) and
# the real session folder. Environment:
#   DRUMSPIKE_SELFTEST_SESSION  session folder with "Audio Files/" (default: Session A in ~/.cache/drumspike/sessions)
#   DRUMSPIKE_SELFTEST_WAVS     folder of WAV files (make_session.py output) used instead of the session: scenario `render` (30 lanes)
#   DRUMSPIKE_SELFTEST_TRACKS   number of tracks of the project (default: the scenario's expect.tracks, 10)
#   DRUMSPIKE_SELFTEST_DIR      work folder for the project and the report (default: a new temp folder)
#   DRUMSPIKE_SELFTEST_SYNTH    a make_session.py folder (session.json, t01.wav ...): the project is its tracks with their hits
#                               as events (scenario play: the 30-track 96 kHz session)
#   DRUMSPIKE_SELFTEST_AUDIBLE  1 lets the app play with the master gain 1.0; without it the app starts with
#                               DRUMSPIKE_MASTER_GAIN=0 (silent: the full mix and the clicks run, the output is 0)
#   DRUMSPIKE_SELFTEST_APP      the .app (default: target/aarch64-apple-darwin/debug/bundle/macos/Drum Spike.app, the G3 output)
# A scenario with a "drop" field (tracks, P3-4c) starts with no project: make_project.py --drop copies 10 drum tracks of the
# session into $work/drop, the app gets that folder as a window drop, and the script checks their SHA-256 afterwards.
# A scenario with an "open_doc" field (shell, P3-4b) also gets the project document in
# DRUMSPIKE_SELFTEST_OPEN_DOC; this script then runs `open -a <app> <document>` against the running app,
# so that the real Finder path (Launch Services, RunEvent::Opened) is tested.
# Prints the numbers; exit 0 = the scenario passed and every exported MIDI file equals pretty_midi's.
set -eu
scenario=${1:?usage: self-test.sh <scenario>}
root=$(cd "$(dirname "$0")/../../.." && pwd)
cd "$root"
session=${DRUMSPIKE_SELFTEST_SESSION:-"$HOME/.cache/drumspike/sessions/Session A"}
work=${DRUMSPIKE_SELFTEST_DIR:-$(mktemp -d)}
# The app keeps recent.json and settings.json in DRUMSPIKE_HOME (else in the owner's Application Support folder).
# A self-test run gets its own home under the work folder, so no run changes the owner's settings (two agent runs
# wrote track_height and compared the key preset against the owner's file on 2026-10-12).
export DRUMSPIKE_HOME=${DRUMSPIKE_HOME:-$work/home}
mkdir -p "$DRUMSPIKE_HOME"
app=${DRUMSPIKE_SELFTEST_APP:-"$root/target/aarch64-apple-darwin/debug/bundle/macos/Drum Spike.app"}
bin="$app/Contents/MacOS/drumspike-desktop"
[ -x "$bin" ] || { echo "no debug app at $app (run G3 first)" >&2; exit 2; }
# P3-17: Launch Services gives `open -a` to an instance that already runs. A left-over instance of this app
# (for example one that a late `open -a` started after an earlier run ended) gets the document instead of the
# self-test, and the step finder_open waits in vain. Stop before the run and name the PIDs.
others=$(pgrep -f "$bin" || true)
if [ -n "$others" ]; then
  echo "another instance of this app runs (PID $(echo $others | tr '\n' ' ')); quit it first" >&2
  exit 2
fi
mkdir -p "$work"
case "$scenario" in */*|*.json) scenario_file=$scenario ;; *) scenario_file="$root/apps/desktop/selftest/$scenario.json" ;; esac
scenario_tracks=$(uv run python -c 'import json,sys; print(json.load(open(sys.argv[1])).get("expect", {}).get("tracks", 10))' "$scenario_file")
tracks=${DRUMSPIKE_SELFTEST_TRACKS:-$scenario_tracks}
drop=""
if grep -q '"drop"' "$scenario_file"; then
  # scenario tracks (P3-4c): no project; a folder of 10 copies of the real drum tracks is dropped on the empty window
  drop=$(uv run python apps/desktop/selftest/make_project.py --out "$work/dropped" --from "$session" --drop "$work/drop")
  project=""
  export DRUMSPIKE_PROJECTS="$work/projects"   # the project the drop makes goes to the work folder, not to Documents
  ( cd "$drop" && shasum -a 256 -- *.wav ) > "$work/drop.sha256"
elif [ -n "${DRUMSPIKE_SELFTEST_SYNTH:-}" ]; then
  project=$(uv run python apps/desktop/selftest/make_project.py --out "$work/project" --name "Self Test" \
    --synthetic "$DRUMSPIKE_SELFTEST_SYNTH")
elif [ -n "${DRUMSPIKE_SELFTEST_WAVS:-}" ]; then
  project=$(uv run python apps/desktop/selftest/make_project.py --out "$work/project" --name "Self Test" \
    --wavs "$DRUMSPIKE_SELFTEST_WAVS" --tracks "$tracks")
else
  project=$(uv run python apps/desktop/selftest/make_project.py --out "$work/project" --name "Self Test" \
    --from "$session" --tracks "$tracks")
fi
# P3-5b: a device run is silent unless the caller asks otherwise (master gain after the mix; the system volume is never touched)
if [ "${DRUMSPIKE_SELFTEST_AUDIBLE:-0}" != "1" ]; then export DRUMSPIKE_MASTER_GAIN=0; fi
report="$work/$(basename "$scenario" .json).report.json"
rm -f "$report"
export DRUMSPIKE_SELFTEST_DIR="$work" DRUMSPIKE_SELFTEST_PROJECT="$project" DRUMSPIKE_SELFTEST_OPEN_DOC="" DRUMSPIKE_SELFTEST_DROP="$drop"
mkdir -p "$work/new" "$work/copy"
if grep -q '"open_doc"' "$scenario_file"; then
  # the shell scenario: the project document goes to the running app through Launch Services
  export DRUMSPIKE_SELFTEST_OPEN_DOC="$project/Self Test.drumspike"
  "$bin" --self-test "$scenario" --out "$report" &
  pid=$!
  sleep 6
  open -a "$app" "$DRUMSPIKE_SELFTEST_OPEN_DOC" || echo "open -a failed" >&2
  wait "$pid" || status=$?
else
  "$bin" --self-test "$scenario" --out "$report" || status=$?
fi
[ -f "$report" ] || { echo "the app wrote no report (exit ${status:-0})" >&2; exit 2; }
echo "report: $report"
uv run python apps/desktop/selftest/check_report.py "$report" --project "$project"
if [ -n "$drop" ]; then
  # the sources of a referenced import keep their SHA-256
  ( cd "$drop" && shasum -a 256 -c "$work/drop.sha256" ) >/dev/null && echo "  source SHA-256 unchanged for $(wc -l < "$work/drop.sha256" | tr -d ' ') files" || { echo "  a source file changed" >&2; exit 1; }
fi
