#!/usr/bin/env bash
# Run a pytest Gate N times while a release build loads the host.
#
#   bash tools/ci/loaded_repeat.sh N -- <pytest args>
#
# A background loop rebuilds drumspike-core in release mode (own target dir, the
# package cleaned before each build, so each build does real work). It never
# touches a source file: the e2e stale-ds-bridge check reads source mtimes. Before
# each run the script checks that the loop is alive. Run it under one
# `crewboard lock run` so the heavy work holds the host lock for the whole job.
# Set LOADED_REPEAT_LOG_DIR to keep the run logs (default: a temp dir).
set -u

usage() {
  echo "usage: bash tools/ci/loaded_repeat.sh N -- <pytest args>" >&2
  exit 64
}

[ $# -ge 3 ] || usage
N=$1
case "$N" in
  '' | *[!0-9]* | 0) usage ;;
esac
[ "$2" = "--" ] || usage
shift 2

cd "$(dirname "$0")/../.." || exit 1

LOG_DIR=${LOADED_REPEAT_LOG_DIR:-$(mktemp -d)}
mkdir -p "$LOG_DIR" || exit 1
LOAD_LOG="$LOG_DIR/load.log"
LOAD_TARGET=$(mktemp -d) || exit 1

load() {
  while :; do
    CARGO_TARGET_DIR="$LOAD_TARGET" cargo clean -p drumspike-core --release >>"$LOAD_LOG" 2>&1
    CARGO_TARGET_DIR="$LOAD_TARGET" cargo build -p drumspike-core --release >>"$LOAD_LOG" 2>&1
  done
}

# With job control the loop gets its own process group, so one kill stops
# the loop and the cargo build it runs.
set -m
load &
LOAD_PID=$!
set +m

cleanup() {
  kill -- -"$LOAD_PID" 2>/dev/null
  wait "$LOAD_PID" 2>/dev/null
  rm -rf "$LOAD_TARGET"
}
trap cleanup EXIT
trap 'exit 130' INT TERM

pass=0
for i in $(seq 1 "$N"); do
  if ! kill -0 "$LOAD_PID" 2>/dev/null; then
    echo "loaded_repeat: FAIL load process $LOAD_PID is dead before run $i (log $LOAD_LOG)"
    echo "loaded_repeat: $pass of $N passed under load"
    exit 1
  fi
  echo "load: running pid $LOAD_PID"
  RUN_LOG="$LOG_DIR/run-$i.log"
  env DRUMSPIKE_AUDIO=null DRUMSPIKE_REQUIRE_APP=1 DRUMSPIKE_REQUIRE_CORE=1 \
    uv run pytest -q "$@" >"$RUN_LOG" 2>&1
  rc=$?
  count=$(grep -E ' in [0-9.]+s' "$RUN_LOG" | tail -n 1)
  echo "run $i: rc=$rc $count log=$RUN_LOG"
  if [ "$rc" -ne 0 ]; then
    echo "loaded_repeat: FAIL at run $i of $N (log $RUN_LOG)"
    echo "loaded_repeat: $pass of $N passed under load"
    exit 1
  fi
  pass=$((pass + 1))
done

echo "loaded_repeat: $pass of $N passed under load"
[ "$pass" -eq "$N" ]
