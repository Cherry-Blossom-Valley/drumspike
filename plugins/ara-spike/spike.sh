#!/bin/zsh
# The ARA spike S1 (#504). The documented build command is
#
#     ./spike.sh all
#
# which fetches the third-party parts into .deps (or the folder that DS_ARA_DEPS names),
# builds the ARA test host, the CLAP plugin and the wrapped VST3 for the architecture of
# this machine, and runs the whole check: ./check_s1.sh (the gate of the ticket) and the
# seed check of the README. It is not part of the Cargo workspace and not part of a release.
#
#     ./spike.sh deps    fetch ARA SDK 2.3.0 (with its submodules), clap-wrapper 0.17.0,
#                        pluginval 1.0.4 (pinned SHA-256) and clap-validator 0.4.1 into .deps,
#                        and apply the patches of patches/
#     ./spike.sh build   build ARATestHost, DrumSpikeAraSpike.clap and DrumSpikeAraSpike.vst3
#                        (the plugin crate builds alone into its own ./target)
#     ./spike.sh check   ./check_s1.sh, then the seed check (check.py: four sources, render,
#                        EditorView, PropertyUpdates, Archiving)
#     ./spike.sh all     deps, build, check
#
# Heavy: run this under the host lock.
set -u
cd "$(dirname "$0")" || exit 2
DEPS=${DS_ARA_DEPS:-$PWD/.deps}
OUT=$PWD/out
ARCH=$(uname -m)                                   # the build is native to this machine
CLT=$(xcode-select -p)                             # the command line tools, no Xcode needed
SDK=$(/usr/bin/xcrun -sdk macosx --show-sdk-path)
SDK=${SDK:A}                                       # the versioned SDK, not the MacOSX.sdk symlink
# SHA-256 of https://github.com/Tracktion/pluginval/releases/download/v1.0.4/pluginval_macOS.zip
PLUGINVAL_SHA256=3c4c533bda0c5059eea3ddaea752d757ee2025041f0f47e6bcb0e87f6082b29f
mkdir -p $OUT
rc=0

run_guarded() {  # run_guarded <logfile> <seconds> <cmd...> : one command with a wall-clock guard
  local log=$1 limit=$2; shift 2
  "$@" > $log 2>&1 & local pid=$! n=0
  while kill -0 $pid 2>/dev/null; do /bin/sleep 1; n=$((n+1)); [[ $n -ge $limit ]] && { kill $pid; echo "FAIL: timeout after ${limit}s ($log)"; return 9; }; done
  wait $pid; return $?
}

deps() {
  mkdir -p $DEPS || return 1
  if [[ ! -d $DEPS/ARA_SDK ]]; then
    git clone --depth 1 --branch releases/2.3.0 --recursive https://github.com/Celemony/ARA_SDK $DEPS/ARA_SDK || return 1
  fi
  local ex=$DEPS/ARA_SDK/ARA_Examples
  if git -C $ex apply --check $PWD/patches/ara-examples-2.3.0.patch 2>/dev/null; then
    git -C $ex apply $PWD/patches/ara-examples-2.3.0.patch || return 1
  elif ! grep -q 'spike:' $ex/CMakeLists.txt; then
    echo "ARA_Examples: the patch is neither applicable nor applied"; return 1
  fi
  if [[ ! -d $DEPS/clap-wrapper ]]; then
    git clone --depth 1 --branch v0.17.0 https://github.com/free-audio/clap-wrapper $DEPS/clap-wrapper || return 1
  fi
  if git -C $DEPS/clap-wrapper apply --check $PWD/patches/clap-wrapper-0.17.0.patch 2>/dev/null; then
    git -C $DEPS/clap-wrapper apply $PWD/patches/clap-wrapper-0.17.0.patch || return 1
  elif ! grep -q 'spike: the ARA test host passes nullptr' $DEPS/clap-wrapper/src/wrapasvst3.cpp; then
    echo "clap-wrapper: the patch is neither applicable nor applied"; return 1
  fi
  if [[ ! -x $DEPS/pluginval.app/Contents/MacOS/pluginval ]]; then
    curl -sL -o $DEPS/pluginval.zip https://github.com/Tracktion/pluginval/releases/download/v1.0.4/pluginval_macOS.zip || return 1
    local got
    got=$(shasum -a 256 $DEPS/pluginval.zip | cut -d' ' -f1) || return 1
    [[ $got == $PLUGINVAL_SHA256 ]] || { echo "pluginval.zip: sha256 $got is not $PLUGINVAL_SHA256"; return 1; }
    (cd $DEPS && unzip -oq pluginval.zip) || return 1
    xattr -dr com.apple.quarantine $DEPS/pluginval.app 2>/dev/null
  fi
  if [[ ! -d $DEPS/clap-validator-src ]]; then
    git clone --depth 1 --branch 0.4.1 https://github.com/free-audio/clap-validator $DEPS/clap-validator-src || return 1
  fi
  echo "deps: ok in $DEPS"
}

build() {
  # the test host of the ARA examples -> build-examples/bin/Debug/ARATestHost
  cmake -S $DEPS/ARA_SDK/ARA_Examples -B $DEPS/build-examples \
    -DARA_SETUP_DEBUGGING=OFF -DCMAKE_BUILD_TYPE=Debug \
    -DCMAKE_OSX_ARCHITECTURES=$ARCH -DCMAKE_OSX_SYSROOT=$SDK -DXCODE_VERSION=16.0 || return 1
  SDKROOT=$SDK XCODE_VERSION=16.0 cmake --build $DEPS/build-examples --config Debug --target ARATestHost --parallel || return 1
  [[ -x $DEPS/build-examples/bin/Debug/ARATestHost ]] || { echo "no ARATestHost in $DEPS/build-examples"; return 1; }
  # the plugin: bindgen on $ARA_API_DIR/ARAInterface.h, alone in its own target dir
  ARA_API_DIR=$DEPS/ARA_SDK/ARA_API LIBCLANG_PATH=$CLT/usr/lib CARGO_TARGET_DIR=$PWD/target \
    cargo build --release || return 1
  mkdir -p $DEPS/DrumSpikeAraSpike.clap/Contents/MacOS
  cp Info.plist $DEPS/DrumSpikeAraSpike.clap/Contents/Info.plist
  cp target/release/libds_ara_spike.dylib $DEPS/DrumSpikeAraSpike.clap/Contents/MacOS/DrumSpikeAraSpike || return 1
  # the wrapped VST3 from clap-wrapper; it finds the CLAP through CLAP_PATH at run time
  cmake -S $DEPS/clap-wrapper -B $DEPS/build-wrapper \
    -DCMAKE_BUILD_TYPE=Release -DCMAKE_OSX_ARCHITECTURES=$ARCH \
    -DCMAKE_OSX_DEPLOYMENT_TARGET=12.0 \
    -DCLAP_WRAPPER_DOWNLOAD_DEPENDENCIES=TRUE -DCLAP_WRAPPER_OUTPUT_NAME=DrumSpikeAraSpike || return 1
  cmake --build $DEPS/build-wrapper --config Release || return 1
  [[ -d $DEPS/build-wrapper/DrumSpikeAraSpike.vst3 ]] || { echo "no .vst3 in $DEPS/build-wrapper"; ls $DEPS/build-wrapper; return 1; }
  # clap-validator for the CLAP bundle (pluginval 1.0.4 has no CLAP support)
  CARGO_TARGET_DIR=$DEPS/clap-validator-src/target cargo build --release --manifest-path $DEPS/clap-validator-src/Cargo.toml || return 1
  echo "build: ok ($DEPS/DrumSpikeAraSpike.clap, $DEPS/build-wrapper/DrumSpikeAraSpike.vst3)"
}

seed_check() {  # the seed check of the README: four sources, render, three host tests, both paths
  local D=$DEPS O=$OUT F=$PWD/fixtures fail=0
  local H=$D/build-examples/bin/Debug/ARATestHost
  [[ -x $H ]] || { echo "FAIL: no test host at $H"; return 1; }
  export CLAP_PATH=$D
  /usr/bin/python3 -I check.py make || return 2
  host() {  # host <label> <args...> : run the test host with a 60 s guard
    local label=$1; shift
    run_guarded $O/run-$label.log 60 $H "$@"; local rc=$?
    [[ $rc -eq 0 ]] || echo "FAIL $label: host rc=$rc"
    return $rc
  }
  local kind B out dump
  for kind in clap vst3; do
    if [[ $kind = clap ]]; then B=$D/DrumSpikeAraSpike.clap; else B=$D/build-wrapper/DrumSpikeAraSpike.vst3; fi
    echo "=== $kind: ${B#$D/}"
    # 1. the plugin reads every sample of four sources and gives the hits back as notes
    out=$O/out-$kind.jsonl; rm -f $out
    DS_ARA_SPIKE_OUT=$out host read-$kind -$kind $B -file $F/kick_48k_mono.wav $F/snare_44k_stereo.wav $F/tom_96k_mono.wav $F/hat_48k_mono.wav -test ContentReading \
      && /usr/bin/python3 -I check.py verify $out $kind || fail=1
    # 2. the host renders two regions back to back through the plugin instance; the audio must be the spikes
    dump=$O/render-$kind.f32; rm -f $dump $dump.txt
    DS_ARA_SPIKE_OUT=$O/out-render-$kind.jsonl DS_ARA_SPIKE_RENDER=$dump host render-$kind -$kind $B -file $F/kick_48k_mono.wav $F/hat_48k_mono.wav -test PlaybackRendering \
      && /usr/bin/python3 -I check.py render $dump render-$kind kick_48k_mono.wav hat_48k_mono.wav || fail=1
    # 3. the other host tests that touch the plugin instance and the model
    local t
    for t in EditorView PropertyUpdates Archiving; do
      host $t-$kind -$kind $B -file $F/kick_48k_mono.wav -test $t && echo "ok   $kind $t: host exit 0" || fail=1
    done
  done
  [[ $fail -eq 0 ]] && echo "SPIKE: PASS (CLAP and wrapped VST3)" || echo "SPIKE: FAIL"
  [[ $fail -eq 0 ]]
}

check() {
  ./check_s1.sh || return 1
  seed_check || return 1
}

case ${1:-all} in
  deps) deps || rc=1 ;;
  build) build || rc=1 ;;
  check) check || rc=1 ;;
  all) deps || rc=1; [[ $rc = 0 ]] && build || rc=1; [[ $rc = 0 ]] && check || rc=1 ;;
  *) echo "usage: spike.sh deps|build|check|all"; rc=64 ;;
esac
exit $rc
