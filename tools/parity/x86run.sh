#!/bin/zsh
# The manual x86 check on x86-mac (Intel Mac, x86_64; docs/hosts.md). Installed there as
# ~/x86run.sh; this file is the source.
#
#   X86_REPO=~/src/<clone> X86_RUN=~/x86run-<job> ~/x86run.sh
#
# Resumable: every step writes $X86_RUN/logs/<step>.log, and a step with a <step>.ok
# marker is skipped on the next start. Delete the marker (or use a new X86_RUN) to rerun a
# step. Phase 1 steps: tests, clippy, the parity scripts, the suites in both modes, the
# bench and the golden regeneration. Phase 2 (P2-8) adds the detection steps p_resample to
# p_onsets, runs the Rust and Python suites with the DSP fallback, and the bench with the
# detection row. Main is model-free (#281 D16): the detector is the DSP detector.
# Phase 3 (P3-2a) adds p_analyze; P3-3a adds build_svc and p_service (the service commands,
# tools/parity). P3-12: the Python server is retired; every check compares the Rust side with
# the frozen references in tests/golden/frozen/ (x86_64 values where they differ from arm64), so
# there is no module build, no Rust mode suite and no golden regeneration any more.
export PATH=$HOME/.local/bin:$HOME/.cargo/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin
R=${X86_REPO:-$HOME/src/drumspike-x86}
W=${X86_RUN:-$HOME/x86run}
L=$W/logs
C=$W/corpus
REFS=${X86_REFS:-$HOME/detect-refs/out}          # run1 (madmom chain), run3 (CLI chain) of make_detect_refs.py
SESSIONS=${X86_SESSIONS:-$HOME/sessions}          # <Session>/<file>.wav, read only
SONG1=${X86_SONG1:-/Users/x86-mac/adtpipeline/song1}  # read only
mkdir -p $L $C
cd $R || exit 1
echo "repo $R at $(git rev-parse --short HEAD), logs $L"

step() {
  local name=$1; shift
  if [[ -f $L/$name.ok ]]; then echo "SKIP $name"; return 0; fi
  echo "START $name $(date +%T)"
  "$@" > $L/$name.log 2>&1
  local rc=$?
  if (( rc == 0 )); then
    touch $L/$name.ok; echo "OK $name $(date +%T)"
  else
    echo "FAIL $name $(date +%T) (exit $rc)"; tail -5 $L/$name.log
  fi
}

step sync         uv sync
step cargo_test   cargo test --workspace
step clippy       cargo clippy --all-targets -- -D warnings
step build_rel    cargo build --release -p drumspike-devtools
step build_svc    cargo build --release -p drumspike-devtools --bin service_tool
step build_bridge cargo build -p drumspike-devtools --bin ds-bridge
step corpus       uv run --project . python tools/parity/make_corpus.py $C
step p_decode     uv run --project . python tools/parity/check_parity.py $C
step p_refine     uv run --project . python tools/parity/check_refine.py $C --json $L/p_refine.json
step p_midi       uv run --project . python tools/parity/check_midi.py $C
step p_cache      target/release/cache_tool check $C
# Phase 2 detection parity (P2-8).
step p_resample   uv run --no-project --python 3.12 --with librosa==0.10.2.post1 --with soxr==1.1.0 \
                    --with 'numpy<2.1' --with soundfile --with numba==0.60.0 --with llvmlite==0.43.0 \
                    python tools/parity/check_resample.py $C --work $W/resample --json $L/p_resample.json
step p_features_cli    uv run --project . python tools/parity/check_features.py cli $REFS/run3 \
                         --work $W/features_cli --md $L/p_features_cli.md
step p_features_madmom uv run --project . python tools/parity/check_features.py madmom $REFS/run1 \
                         --sig-dir $REFS/run3 --work $W/features_madmom --md $L/p_features_madmom.md
step p_pick_cli   uv run --project . python tools/parity/check_pick.py cli $REFS/run3 --work $W/pick_cli --md $L/p_pick_cli.md
step p_pick_madmom uv run --project . python tools/parity/check_pick.py madmom $REFS/run1 --work $W/pick_madmom --md $L/p_pick_madmom.md
step p_onsets     uv run --project . python tools/parity/check_onsets.py corpus $C --work $W/onsets --json $L/p_onsets.json
# P3-2a: the analysis pipeline in the core against the frozen engine.analyze_track (Rust mode), corpus
# and the 38 real tracks, with the model and with the DSP fallback.
step p_analyze    uv run --project . python tools/parity/check_analyze.py $C --samples $SESSIONS \
                    --layout flat --song1 $SONG1 --json $L/p_analyze.json
# P3-3a: Service::analyze_start and export_midi (crates/app) against the frozen engine and pretty_midi.
step p_service    uv run --project . python tools/parity/check_service.py $C --json $L/p_service.json
step golden      uv run pytest -q --no-cov tests/test_golden.py tests/test_rust_parity.py
# The suite; the model tests run (DRUMSPIKE_REQUIRE_MODEL=1 fails them instead of a skip).
step suite        env DRUMSPIKE_AUDIO=null DRUMSPIKE_REQUIRE_APP=1 DRUMSPIKE_REQUIRE_CORE=1 DRUMSPIKE_REQUIRE_MODEL=1 uv run pytest -q --no-cov
# Bench: phase 1 rows and the detection row, with the model and with the DSP fallback.
step bench           ./target/release/bench --runs 3 --detect-runs 3
step bench_10min     ./target/release/bench --tracks 1 --seconds 600 --runs 3 --detect-runs 5
step bench_dsp       ./target/release/bench --runs 3 --detect-runs 3
step bench_dsp_10min ./target/release/bench --tracks 1 --seconds 600 --runs 3 --detect-runs 5
echo "DONE"
ls $L/*.log | while read f; do n=${f:t:r}; [[ -f $L/$n.ok ]] || echo "NOT OK: $n"; done
