#!/bin/zsh
# The commands of the webview spike (S2, #505). The documented build command is
#
#     ./spike.sh build
#
# which builds the CLAP plugin (both macOS targets, joined with lipo) with the editor page
# inside the bundle, the wrapped VST3, and the test host.
#     ./spike.sh deps    fetch the third-party parts into .deps (clap-wrapper 0.17.0,
#                        pluginval not needed for the S2 gate, clap-validator for the report)
#                        and both macOS Rust targets
#     ./spike.sh check   fmt, clippy and the tests of the spike crate
#     ./spike.sh gate    run check_s2.sh (the gate of the ticket; needs build)
#     ./spike.sh all     deps, build, check
#
# Build dirs: the plugin builds into its own ./target (wry and its objc2 crates are heavy and
# must not touch the workspace target). Heavy: run this under the host lock.
set -u
cd "$(dirname "$0")" || exit 2
ROOT=$(pwd)/../..
DEPS=$(pwd)/.deps
BUNDLE=$DEPS/DrumSpikeWebviewSpike.clap
rc=0

deps() {
  mkdir -p $DEPS || return 1
  rustup target add x86_64-apple-darwin aarch64-apple-darwin || return 1
  if [[ ! -d $DEPS/clap-wrapper ]]; then
    git clone --depth 1 --branch v0.17.0 https://github.com/free-audio/clap-wrapper $DEPS/clap-wrapper || return 1
  fi
  # the VST3 SDK the wrapper fetches itself (CLAP_WRAPPER_DOWNLOAD_DEPENDENCIES): v3.8.0,
  # MIT since 3.8.0 - the licence of everything the wrapper brings is in the report.
  echo "deps: ok in $DEPS"
}

build() {
  # the plugin for both macOS targets, then the universal binary inside the .clap bundle
  CARGO_TARGET_DIR=$(pwd)/target cargo build --release --target x86_64-apple-darwin --manifest-path Cargo.toml || return 1
  CARGO_TARGET_DIR=$(pwd)/target cargo build --release --target aarch64-apple-darwin --manifest-path Cargo.toml || return 1
  mkdir -p $BUNDLE/Contents/MacOS $BUNDLE/Contents/Resources
  cp Info.plist $BUNDLE/Contents/Info.plist
  lipo -create target/x86_64-apple-darwin/release/libds_webview_spike.dylib \
             target/aarch64-apple-darwin/release/libds_webview_spike.dylib \
       -output $BUNDLE/Contents/MacOS/DrumSpikeWebviewSpike || return 1
  # the editor page: a copy of static/ inside the bundle (the page files are read, not changed)
  rm -rf $BUNDLE/Contents/Resources/page
  ditto $ROOT/static $BUNDLE/Contents/Resources/page || return 1
  # the test host runs natively on this machine (its own target dir, no plugin deps shared)
  CARGO_TARGET_DIR=$(pwd)/target cargo build --release --manifest-path Cargo.toml --bin webview-spike-host || return 1
  # the wrapped VST3 from clap-wrapper (universal as well); it finds the CLAP through CLAP_PATH at run time
  if [[ ! -d $DEPS/build-wrapper ]] || [[ ! -d $DEPS/build-wrapper/DrumSpikeWebviewSpike.vst3 ]]; then
    cmake -S $DEPS/clap-wrapper -B $DEPS/build-wrapper \
      -DCMAKE_BUILD_TYPE=Release -DCMAKE_OSX_ARCHITECTURES="x86_64;arm64" \
      -DCMAKE_OSX_DEPLOYMENT_TARGET=12.0 \
      -DCLAP_WRAPPER_DOWNLOAD_DEPENDENCIES=TRUE -DCLAP_WRAPPER_OUTPUT_NAME=DrumSpikeWebviewSpike || return 1
    cmake --build $DEPS/build-wrapper --config Release || return 1
  fi
  [[ -d $DEPS/build-wrapper/DrumSpikeWebviewSpike.vst3 ]] || { echo "no .vst3 in $DEPS/build-wrapper"; ls $DEPS/build-wrapper; return 1; }
  echo "build: ok ($BUNDLE, $DEPS/build-wrapper/DrumSpikeWebviewSpike.vst3, target/release/webview-spike-host)"
}

check() {
  echo "=== the crate of the spike: fmt, clippy, tests"
  CARGO_TARGET_DIR=$(pwd)/target cargo fmt --manifest-path Cargo.toml --all -- --check || return 1
  CARGO_TARGET_DIR=$(pwd)/target cargo clippy --manifest-path Cargo.toml --all-targets -- -D warnings || return 1
  CARGO_TARGET_DIR=$(pwd)/target cargo test --manifest-path Cargo.toml || return 1
  echo "=== the standalone rule of the ticket (gate item 6)"
  if (cd $ROOT && cargo metadata --no-deps --format-version 1) | grep -q ds-webview-spike; then
    echo "FAIL standalone: cargo metadata --no-deps at the repo root names the spike crate"; return 1
  fi
  if (cd $ROOT && cargo tree -p drumspike-desktop -e normal --format-version 1 2>/dev/null || cargo tree -p drumspike-desktop 2>/dev/null) | grep -q ds-webview-spike; then
    echo "FAIL standalone: cargo tree -p drumspike-desktop names the spike crate"; return 1
  fi
  echo "ok   standalone: the spike crate stays out of the workspace and of drumspike-desktop"
}

case ${1:-all} in
  deps) deps || rc=1 ;;
  build) build || rc=1 ;;
  check) check || rc=1 ;;
  gate) check_s2=1; /bin/zsh ./check_s2.sh || rc=1 ;;
  all) deps || rc=1; [[ $rc = 0 ]] && build || rc=1; [[ $rc = 0 ]] && check || rc=1 ;;
  *) echo "usage: spike.sh deps|build|check|gate|all"; rc=64 ;;
esac
exit $rc
