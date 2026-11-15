#!/bin/zsh
# The commands of the service spike (S3, #506). The documented build command is
#
#     ./spike.sh build
#
# which builds the CLAP plugin (both macOS targets, joined with lipo) and the wrapped VST3.
#     ./spike.sh deps    fetch the third-party parts into .deps (clap-wrapper 0.17.0, pluginval
#                        with a pinned SHA-256, clap-validator 0.4.1) and both macOS Rust targets
#     ./spike.sh check   run every gate line of the ticket on this head (the crate tests, the
#                        test host, the byte-for-byte compare, both validators, the universal
#                        and the standalone probes, and the workspace trio of gate line 6)
#     ./spike.sh all     deps, build, check
#
# Build dirs: the plugin builds into its own ./target; the workspace binaries come from
# ${CARGO_TARGET_DIR:-$ROOT/target}/release (DS_SPIKE_TOOLS overrides the folder). Heavy:
# run this under the host lock.
set -u
cd "$(dirname "$0")" || exit 2
ROOT=$(pwd)/../..
DEPS=$(pwd)/.deps
BUNDLE=$DEPS/DrumSpikeServiceSpike.clap
CALLER_TARGET=${CARGO_TARGET_DIR:-$ROOT/target}  # the caller's warm dir, if one is set
export CARGO_TARGET_DIR=$(pwd)/target            # the plugin builds alone in its own dir
WS_TARGET=${WS_CARGO_TARGET_DIR:-$CALLER_TARGET} # the workspace keeps the caller's dir
TOOLS=${DS_SPIKE_TOOLS:-$WS_TARGET/release}
# SHA-256 of https://github.com/Tracktion/pluginval/releases/download/v1.0.4/pluginval_macOS.zip
PLUGINVAL_SHA256=3c4c533bda0c5059eea3ddaea752d757ee2025041f0f47e6bcb0e87f6082b29f
rc=0

deps() {
  mkdir -p $DEPS
  rustup target add aarch64-apple-darwin x86_64-apple-darwin || return 1
  if [[ ! -d $DEPS/clap-wrapper ]]; then
    git clone --depth 1 --branch v0.17.0 https://github.com/free-audio/clap-wrapper $DEPS/clap-wrapper || return 1
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
    # its own target folder: the exported CARGO_TARGET_DIR is the plugin's, and check.py
    # looks for the validator in .deps/clap-validator-src/target/release
    CARGO_TARGET_DIR=$DEPS/clap-validator-src/target cargo build --release --manifest-path $DEPS/clap-validator-src/Cargo.toml || return 1
  fi
  echo "deps: ok in $DEPS"
}

build() {
  # the workspace release binaries (service_tool) for the compare
  CARGO_TARGET_DIR=$WS_TARGET cargo build --release --manifest-path $ROOT/Cargo.toml -p drumspike-devtools || return 1
  # the plugin for both macOS targets, then the universal binary inside the .clap bundle
  cargo build --release --target x86_64-apple-darwin || return 1
  cargo build --release --target aarch64-apple-darwin || return 1
  mkdir -p $BUNDLE/Contents/MacOS
  cp Info.plist $BUNDLE/Contents/Info.plist
  lipo -create target/x86_64-apple-darwin/release/libds_service_spike.dylib \
             target/aarch64-apple-darwin/release/libds_service_spike.dylib \
       -output $BUNDLE/Contents/MacOS/DrumSpikeServiceSpike || return 1
  # the test host runs natively on this machine
  cargo build --release || return 1
  # the wrapped VST3 from clap-wrapper (universal as well); it finds the CLAP through CLAP_PATH at run time
  cmake -S $DEPS/clap-wrapper -B $DEPS/build-wrapper \
    -DCMAKE_BUILD_TYPE=Release -DCMAKE_OSX_ARCHITECTURES="x86_64;arm64" \
    -DCMAKE_OSX_DEPLOYMENT_TARGET=12.0 \
    -DCLAP_WRAPPER_DOWNLOAD_DEPENDENCIES=TRUE -DCLAP_WRAPPER_OUTPUT_NAME=DrumSpikeServiceSpike || return 1
  cmake --build $DEPS/build-wrapper --config Release || return 1
  [[ -d $DEPS/build-wrapper/DrumSpikeServiceSpike.vst3 ]] || { echo "no .vst3 in $DEPS/build-wrapper"; ls $DEPS/build-wrapper; return 1; }
  echo "build: ok ($BUNDLE, $DEPS/build-wrapper/DrumSpikeServiceSpike.vst3)"
}

check() {
  echo "=== the crate of the spike: fmt, clippy, tests (gate item 2)"
  cargo fmt --all -- --check || return 1
  cargo clippy --all-targets -- -D warnings || return 1
  cargo test || return 1
  echo "=== the workspace: fmt, clippy, tests (gate item 6)"
  cargo fmt --all --manifest-path $ROOT/Cargo.toml -- --check || return 1
  CARGO_TARGET_DIR=$WS_TARGET cargo clippy --manifest-path $ROOT/Cargo.toml --all-targets -- -D warnings || return 1
  CARGO_TARGET_DIR=$WS_TARGET cargo test --manifest-path $ROOT/Cargo.toml --workspace --no-fail-fast || return 1
  echo "=== the gate lines 1 to 5 and 7: check.py all"
  /usr/bin/python3 -I check.py all || return 1
}

case ${1:-all} in
  deps) deps || rc=1 ;;
  build) build || rc=1 ;;
  check) check || rc=1 ;;
  all) deps || rc=1; [[ $rc = 0 ]] && build || rc=1; [[ $rc = 0 ]] && check || rc=1 ;;
  *) echo "usage: spike.sh deps|build|check|all"; rc=64 ;;
esac
exit $rc
