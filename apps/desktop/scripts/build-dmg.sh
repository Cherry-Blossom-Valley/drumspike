#!/bin/sh
# P3-8a (arm64-mac only): a release "Drum Spike.app" and an installable .dmg, both signed ad hoc.
#   apps/desktop/scripts/build-dmg.sh [--arch arm64|x86_64] [--app-only | --dmg-only]      (default arm64, both phases)
# P3-11: --arch x86_64 cross-builds the Intel app (target x86_64-apple-darwin).
# #281 D16: the app is model-free (the DSP detector is the only path); no model files and no
# dynamic inference library are bundled on either architecture.
# Steps: check THIRD_PARTY.md is current, build the release .app (tauri build, bundle "app"),
# sign it ad hoc with the hardened runtime, verify the signature, compare every resource with the repo
# file (SHA-256), make the .dmg with hdiutil (no Finder scripting), sign the .dmg ad hoc, print the sizes.
# Ad hoc only: the Developer ID signing and the notarization are card P3-8b (release.sh, run by the owner).
# Output: target/<triple>/release/bundle/macos/Drum Spike.app and .../bundle/dmg/Drum Spike_<version>_<arch>.dmg
# with <triple>_<arch> = aarch64-apple-darwin_aarch64 or x86_64-apple-darwin_x86_64 (never committed). Environment: DRUMSPIKE_BUNDLE_DIR overrides the output folder.
# release.sh (P3-8b) calls this script with DS_CODESIGN_IDENTITY set to the Developer ID name and
# DS_CODESIGN_TIMESTAMP=1 (secure timestamp, needed for notarization). Without them: ad hoc ("-").
# X-5 (P3-9b): two phases, so that release.sh can notarize and staple the .app before the .dmg is made:
#   --app-only  the .app: build, sign, verify, compare the resources (no .dmg)
#   --dmg-only  the .dmg of the .app that is there (stapled by release.sh): make, sign, verify; builds nothing
# Without a phase flag the script does both (the ad hoc build, CI).
set -eu
usage() { echo "usage: build-dmg.sh [--arch arm64|x86_64] [--app-only | --dmg-only]" >&2; exit 64; }
arch=arm64
phase=all
while [ $# -gt 0 ]; do
  case "$1" in
    --arch) [ $# -ge 2 ] || usage; arch=$2; shift ;;
    --app-only) [ "$phase" = all ] || usage; phase=app ;;
    --dmg-only) [ "$phase" = all ] || usage; phase=dmg ;;
    *) usage ;;
  esac
  shift
done
case "$arch" in
  arm64|aarch64) target=aarch64-apple-darwin; suffix=aarch64; lipo_arch=arm64 ;;
  x86_64|x64|intel) target=x86_64-apple-darwin; suffix=x86_64; lipo_arch=x86_64 ;;
  *) echo "unknown architecture: $arch (arm64 or x86_64)" >&2; exit 64 ;;
esac
ident=${DS_CODESIGN_IDENTITY:--}
stamp=--timestamp=none
[ "${DS_CODESIGN_TIMESTAMP:-0}" = 1 ] && stamp=--timestamp
entitlements=apps/desktop/src-tauri/entitlements.plist
root=$(cd "$(dirname "$0")/../../.." && pwd)
cd "$root"
bundle=${DRUMSPIKE_BUNDLE_DIR:-$root/target/$target/release/bundle}
app="$bundle/macos/Drum Spike.app"
version=$(uv run --no-project --python 3.12 python -c 'import json,sys; print(json.load(open(sys.argv[1]))["version"])' apps/desktop/src-tauri/tauri.conf.json)
dmg="$bundle/dmg/Drum Spike_${version}_${suffix}.dmg"

if [ "$phase" != dmg ]; then
if [ "$target" = x86_64-apple-darwin ]; then
  rustup target list --installed | grep -qx x86_64-apple-darwin ||
    { echo "Rust target missing: rustup target add x86_64-apple-darwin" >&2; exit 1; }
fi

echo "== THIRD_PARTY.md is current"
uv run --no-project --python 3.12 apps/desktop/scripts/gen_third_party.py --check

echo "== release build"
[ -d apps/desktop/node_modules ] || (cd apps/desktop && npm ci)
(cd apps/desktop && npx tauri build --target "$target" --bundles app --ci)
[ -d "$app" ] || { echo "no app at $app" >&2; exit 1; }
exe="$app/Contents/MacOS/drumspike-desktop"
[ "$(lipo -archs "$exe")" = "$lipo_arch" ] || { echo "executable is $(lipo -archs "$exe"), want $lipo_arch" >&2; exit 1; }
# model-free (#281 D16): no dylibs are bundled; the executable must not link one
[ ! -e "$app/Contents/Frameworks" ] || { echo "unexpected Contents/Frameworks in the app" >&2; exit 1; }

echo "== signature (identity: $([ "$ident" = - ] && echo 'ad hoc' || echo 'Developer ID')), hardened runtime, entitlements"
codesign --force --deep --options runtime "$stamp" --entitlements "$entitlements" --sign "$ident" "$app"
codesign --verify --deep --strict --verbose=2 "$app"
codesign -dv "$app" 2>&1 | grep -E 'Identifier|flags|Signature' || true
if [ "$ident" != - ] && codesign -d --entitlements - "$app" 2>/dev/null | grep -q disable-library-validation; then
  echo "a Developer ID app must not have disable-library-validation" >&2
  exit 1
fi

echo "== resources equal the repository files"
res="$app/Contents/Resources"
a=$(shasum -a 256 apps/desktop/THIRD_PARTY.md | cut -d' ' -f1)
b=$(shasum -a 256 "$res/THIRD_PARTY.md" | cut -d' ' -f1)
[ "$a" = "$b" ] || { echo "MISMATCH THIRD_PARTY.md" >&2; exit 1; }
echo "$a  THIRD_PARTY.md"
fi   # phase app

if [ "$phase" = app ]; then
  du -sk "$app" | awk '{printf ".app  %.1f MB (%d KB on disk)\n", $1/1024, $1}'
  echo "app: $app"
  exit 0
fi

if [ "$phase" = dmg ]; then
  echo "== the .app for the .dmg (no build)"
  [ -d "$app" ] || { echo "no app at $app (run build-dmg.sh --app-only first)" >&2; exit 1; }
  codesign --verify --deep --strict --verbose=2 "$app"
  if xcrun stapler validate "$app" >/dev/null 2>&1; then echo "the .app has a stapled ticket"; else echo "the .app has no stapled ticket"; fi
fi

echo "== dmg"
mkdir -p "$(dirname "$dmg")"
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
ditto "$app" "$stage/Drum Spike.app"   # ditto keeps the stapled ticket, the signature and the attributes
ln -s /Applications "$stage/Applications"
rm -f "$dmg"
hdiutil create -quiet -volname "Drum Spike" -srcfolder "$stage" -fs HFS+ -format UDZO -ov "$dmg"
codesign --force "$stamp" --sign "$ident" "$dmg"
codesign --verify --strict "$dmg"
hdiutil verify -quiet "$dmg"

echo "== sizes"
du -sk "$app" | awk '{printf ".app  %.1f MB (%d KB on disk)\n", $1/1024, $1}'
stat -f %z "$dmg" | awk '{printf ".dmg  %.1f MB (%d bytes)\n", $1/1048576, $1}'
shasum -a 256 "$dmg"
echo "app: $app"
echo "dmg: $dmg"
