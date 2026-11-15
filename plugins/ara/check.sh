#!/bin/zsh
# The M1a gate of ticket #525 (DAW plugin: the plugin analyses the host's tracks with the
# Drum Spike service, ARA, no editor). One self-contained script:
#
#   ./check.sh deps    fetch the third-party parts into .deps (the ARA SDK 2.3.0 API
#                      headers for bindgen, clap-wrapper 0.17.0 + the patch of patches/,
#                      pluginval 1.0.4 with a pinned SHA-256, clap-validator 0.4.1) and
#                      both macOS Rust targets. DS_ARA_DEPS overrides the folder.
#   ./check.sh build   the universal CLAP (arm64 + x86_64, joined with lipo - the script
#                      fails when lipo -info does not name both), the self-contained
#                      wrapped VST3 (the CLAP is copied inside the bundle, the wrapper
#                      finds it there without CLAP_PATH), the test host ara-host and the
#                      workspace service_tool for the compare.
#   ./check.sh check   the gate: ./check.py all (fixture, reference, identity on the CLAP
#                      and on the plugin inside the VST3, regions, archive, notes,
#                      threads, files, pluginval, clap-validator, universal, standalone,
#                      crate trio, workspace trio).
#   ./check.sh all     deps, build, check
#
# The cache folder the plugin writes below is named in README.md
# (~/Library/Caches/DrumSpike/ARA); the check overrides it with DS_ARA_CACHE into out/
# and proves the plugin writes nothing else (check.py, stage files).
#
# Heavy: run this under the host lock.
set -u
cd "$(dirname "$0")" || exit 2
ROOT=$(pwd)/../..
DEPS=${DS_ARA_DEPS:-$PWD/.deps}
OUT=$PWD/out
CLT=$(xcode-select -p)                             # the command line tools, no Xcode needed
CALLER_TARGET=${CARGO_TARGET_DIR:-$ROOT/target}    # the caller's warm dir, if one is set
export CARGO_TARGET_DIR=$PWD/target                # the plugin builds alone in its own dir
WS_TARGET=${WS_CARGO_TARGET_DIR:-$CALLER_TARGET}   # the workspace keeps the caller's dir
TOOLS=${DS_SPIKE_TOOLS:-$WS_TARGET/release}
CLAP=$DEPS/DrumSpikeAra.clap
VST3=$DEPS/build-wrapper/DrumSpikeAra.vst3
# SHA-256 of https://github.com/Tracktion/pluginval/releases/download/v1.0.4/pluginval_macOS.zip
PLUGINVAL_SHA256=3c4c533bda0c5059eea3ddaea752d757ee2025041f0f47e6bcb0e87f6082b29f
rc=0

deps() {
  mkdir -p $DEPS || return 1
  rustup target add aarch64-apple-darwin x86_64-apple-darwin || return 1
  # the ARA 2.3.0 API headers for bindgen ($ARA_API_DIR/ARAInterface.h). ARA_API is a
  # submodule of the ARA_SDK repo (releases/2.3.0), not a folder of the main repo:
  # check out the pinned ARA_API after the clone; the examples and library submodules
  # are not needed here.
  if [[ ! -f $DEPS/ARA_SDK/ARA_API/ARAInterface.h ]]; then
    rm -rf "$DEPS/ARA_SDK"
    git clone --depth 1 --branch releases/2.3.0 https://github.com/Celemony/ARA_SDK "$DEPS/ARA_SDK" || return 1
    git -C "$DEPS/ARA_SDK" submodule update --init --depth 1 ARA_API || return 1
  fi
  if [[ ! -f $DEPS/ARA_SDK/ARA_API/ARAInterface.h ]]; then
    echo "ARA_SDK: no ARA_API/ARAInterface.h after the clone"; return 1
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
  if [[ ! -x $DEPS/clap-validator-src/target/release/clap-validator ]]; then
    # its own target folder: the exported CARGO_TARGET_DIR is the plugin's
    CARGO_TARGET_DIR=$DEPS/clap-validator-src/target cargo build --release --manifest-path $DEPS/clap-validator-src/Cargo.toml || return 1
  fi
  echo "deps: ok in $DEPS"
}

build() {
  # the workspace release binary (service_tool) for the byte compares
  CARGO_TARGET_DIR=$WS_TARGET cargo build --release --locked --manifest-path $ROOT/Cargo.toml -p drumspike-devtools --bin service_tool || return 1
  # the plugin for both macOS targets (bindgen reads $ARA_API_DIR), then the universal
  # binary inside the .clap bundle; the script fails unless lipo names both arches
  ARA_API_DIR=$DEPS/ARA_SDK/ARA_API LIBCLANG_PATH=$CLT/usr/lib cargo build --release --locked --target x86_64-apple-darwin || return 1
  ARA_API_DIR=$DEPS/ARA_SDK/ARA_API LIBCLANG_PATH=$CLT/usr/lib cargo build --release --locked --target aarch64-apple-darwin || return 1
  mkdir -p $CLAP/Contents/MacOS
  cp Info.plist $CLAP/Contents/Info.plist
  lipo -create target/x86_64-apple-darwin/release/libdrumspike_ara.dylib \
             target/aarch64-apple-darwin/release/libdrumspike_ara.dylib \
       -output $CLAP/Contents/MacOS/DrumSpikeAra || return 1
  local arches
  arches=$(lipo -info $CLAP/Contents/MacOS/DrumSpikeAra) || return 1
  [[ $arches == *arm64* && $arches == *x86_64* ]] || { echo "FAIL: lipo -info does not name arm64 and x86_64: $arches"; return 1; }
  # the test host runs natively on this machine
  ARA_API_DIR=$DEPS/ARA_SDK/ARA_API LIBCLANG_PATH=$CLT/usr/lib cargo build --release --locked --bin ara-host || return 1
  # the wrapped VST3 from clap-wrapper (universal as well) ...
  cmake -S $DEPS/clap-wrapper -B $DEPS/build-wrapper \
    -DCMAKE_BUILD_TYPE=Release -DCMAKE_OSX_ARCHITECTURES="x86_64;arm64" \
    -DCMAKE_OSX_DEPLOYMENT_TARGET=12.0 \
    -DCLAP_WRAPPER_DOWNLOAD_DEPENDENCIES=TRUE -DCLAP_WRAPPER_OUTPUT_NAME=DrumSpikeAra || return 1
  cmake --build $DEPS/build-wrapper --config Release || return 1
  [[ -d $DEPS/build-wrapper/DrumSpikeAra.vst3 ]] || { echo "no .vst3 in $DEPS/build-wrapper"; return 1; }
  # ... made self-contained: the CLAP goes inside the bundle, where the wrapper searches
  # first (Contents/PlugIns). No CLAP_PATH anywhere in the check: the embed is the proof.
  rm -rf $VST3/Contents/PlugIns
  mkdir -p $VST3/Contents/PlugIns
  cp -R $CLAP $VST3/Contents/PlugIns/DrumSpikeAra.clap || return 1
  echo "build: ok ($CLAP, $VST3 with the CLAP inside, $PWD/target/release/ara-host)"
}

check() {
  ARA_API_DIR=$DEPS/ARA_SDK/ARA_API LIBCLANG_PATH=$CLT/usr/lib /usr/bin/python3 -I check.py all || return 1
}

case ${1:-all} in
  deps) deps || rc=1 ;;
  build) build || rc=1 ;;
  check) check || rc=1 ;;
  all) deps || rc=1; [[ $rc = 0 ]] && build || rc=1; [[ $rc = 0 ]] && check || rc=1 ;;
  *) echo "usage: check.sh deps|build|check|all"; rc=64 ;;
esac
exit $rc
