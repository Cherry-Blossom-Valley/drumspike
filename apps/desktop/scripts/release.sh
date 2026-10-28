#!/bin/sh
# P3-8b, P3-10, P3-11 (arm64-mac, run by the OWNER): signed and notarized "Drum Spike" .dmg files, one per
# architecture, and optionally the GitHub Release.
#   DRUMSPIKE_SIGN_IDENTITY="Developer ID Application: Wolfgang Frank (2NZX5Q2ESG)" \
#   DRUMSPIKE_NOTARY_PROFILE=drumspike-notary \
#   apps/desktop/scripts/release.sh [--dry-run] [--arch arm64|x86_64|both] [--publish | --publish-only] [--draft] [--replace]
# --arch: default both (arm64 = Apple silicon, x86_64 = Intel; build-dmg.sh --arch).
# Without both variables: builds the ad hoc .dmg files and prints "unsigned: B1/B2 missing".
# With both: checks them (read-only queries), then per architecture builds and signs the .app with the
# hardened runtime, notarizes and staples the .app, makes the .dmg of the stapled .app, notarizes and staples
# the .dmg, validates both (X-5, P3-9b: an .app copied out of the .dmg starts offline too). --dry-run prints every command and runs none of the build,
# signing, notary, keychain or gh commands. The script never reads or prints a credential: it handles
# only the two names.
# P3-10: --publish runs after the notarization of all architectures: tag v<version> (tauri.conf.json) and a
# GitHub Release through the owner's gh login with the .dmg files, SHA256SUMS and short notes.
# --publish-only does the same with .dmg files that an earlier run notarized (it checks the tickets).
# The script stops when the tag or the release exists; --replace deletes the old release and tag first.
# --draft (P4-41, with --publish or --publish-only): the release is created as a draft, visible only to collaborators;
#   the owner publishes it in GitHub. A tag that already exists on origin at HEAD (the tag that started the
#   release workflow) is then not an error; a tag at another commit still is.
# Agents run only --dry-run.
set -eu

usage() {
  echo "usage: release.sh [--dry-run] [--arch arm64|x86_64|both] [--publish | --publish-only] [--draft] [--replace]" >&2
  exit 64
}

dry=0
arch=both
publish=0
build=1
replace=0
draft=0
exists=0
while [ $# -gt 0 ]; do
  case "$1" in
    --dry-run) dry=1 ;;
    --arch) [ $# -ge 2 ] || usage; arch=$2; shift ;;
    --publish) publish=1 ;;
    --publish-only) publish=1; build=0 ;;
    --replace) replace=1 ;;
    --draft) draft=1 ;;
    *) usage ;;
  esac
  shift
done
case "$arch" in
  both) archs="arm64 x86_64" ;;
  arm64|x86_64) archs=$arch ;;
  *) usage ;;
esac
[ "$draft" = 0 ] || [ "$publish" = 1 ] || { echo "--draft needs --publish or --publish-only" >&2; exit 64; }
[ "$replace" = 0 ] || [ "$publish" = 1 ] || { echo "--replace needs --publish or --publish-only" >&2; exit 64; }

root=$(cd "$(dirname "$0")/../../.." && pwd)
cd "$root"
ident=${DRUMSPIKE_SIGN_IDENTITY:-}
profile=${DRUMSPIKE_NOTARY_PROFILE:-}
repo=${DRUMSPIKE_GH_REPO:-Cherry-Blossom-Valley/drumspike}
builder=apps/desktop/scripts/build-dmg.sh
version=$(sed -n 's/^ *"version": *"\([^"]*\)".*/\1/p' apps/desktop/src-tauri/tauri.conf.json | head -n 1)
tag="v$version"
log=${DRUMSPIKE_RELEASE_LOG:-$root/target/release-notary.log}
out="$root/target/release-$version"

# The triple and the file suffix of an architecture (the names of build-dmg.sh).
triple() { if [ "$1" = x86_64 ]; then echo x86_64-apple-darwin; else echo aarch64-apple-darwin; fi; }
suffix() { if [ "$1" = x86_64 ]; then echo x86_64; else echo aarch64; fi; }
bundle_of() {
  if [ -n "${DRUMSPIKE_BUNDLE_DIR:-}" ]; then echo "$DRUMSPIKE_BUNDLE_DIR"; else echo "$root/target/$(triple "$1")/release/bundle"; fi
}
app_of() { echo "$(bundle_of "$1")/macos/Drum Spike.app"; }
dmg_of() { echo "$(bundle_of "$1")/dmg/Drum Spike_${version}_$(suffix "$1").dmg"; }
# The asset name on GitHub: no spaces (GitHub would turn them into dots).
asset_of() { echo "Drum-Spike_${version}_$(suffix "$1").dmg"; }

# The zip of the .app that goes to Apple (notarytool takes a zip, a .dmg or a .pkg, not a folder).
zip_of() { echo "$(bundle_of "$1")/notary/Drum Spike.app.zip"; }
# The notary log of one file: <log without .log>-<arch>-<app|dmg>.log
log_of() { echo "${log%.log}-$1-$2.log"; }

# Run a command, or only print it with --dry-run.
step() {
  if [ "$dry" = 1 ]; then
    printf 'DRY-RUN would run:'
    for _arg in "$@"; do printf " '%s'" "$_arg"; done
    printf '\n'
  else
    "$@"
  fi
}

if [ -z "$ident" ] || [ -z "$profile" ]; then
  if [ "$publish" = 1 ]; then
    echo "--publish needs a notarized .dmg: set DRUMSPIKE_SIGN_IDENTITY and DRUMSPIKE_NOTARY_PROFILE (docs/RELEASING.md)" >&2
    exit 1
  fi
  for a in $archs; do
    echo "== no signing: build the ad hoc .dmg ($a)"
    step env -u APPLE_SIGNING_IDENTITY -u APPLE_CERTIFICATE -u APPLE_CERTIFICATE_PASSWORD \
      -u APPLE_ID -u APPLE_PASSWORD -u APPLE_TEAM_ID -u APPLE_API_KEY -u APPLE_API_ISSUER -u APPLE_API_KEY_PATH \
      "$builder" --arch "$a"
  done
  echo "unsigned: B1/B2 missing"
  echo "Set DRUMSPIKE_SIGN_IDENTITY and DRUMSPIKE_NOTARY_PROFILE for a notarized release (docs/RELEASING.md)."
  exit 0
fi

echo "== identity: $ident"
echo "== notary profile: $profile"
echo "== architectures: $archs"
if [ "$publish" = 1 ]; then
  echo "== publish: $tag to github.com/$repo$([ "$build" = 0 ] && echo ' (existing .dmg files)')"
fi

echo "== checks (read-only)"
if [ "$dry" = 1 ]; then
  echo "DRY-RUN would run: security find-identity -v -p codesigning   (look for the identity name)"
  echo "DRY-RUN would run: xcrun notarytool history --keychain-profile $profile   (the tool reads its own profile; nothing is printed)"
  echo "DRY-RUN no keychain command was run"
else
  command -v xcrun >/dev/null || { echo "xcrun not found: install the Xcode command line tools" >&2; exit 1; }
  if ! security find-identity -v -p codesigning | grep -F -q "\"$ident\""; then
    echo "identity not found in the keychain (or not valid): $ident" >&2
    echo "Check with: security find-identity -v -p codesigning" >&2
    exit 1
  fi
  # Ask notarytool itself: it finds its own keychain item (the item name is Apple's detail and
  # differs between versions) and signs in to Apple. Its output is not shown; the secret is never read.
  if ! xcrun notarytool history --keychain-profile "$profile" >/dev/null 2>&1; then
    echo "notary profile not usable: $profile (not in the keychain, or Apple refused it)" >&2
    echo "Check with: xcrun notarytool history --keychain-profile $profile" >&2
    echo "Make it once with: xcrun notarytool store-credentials $profile" >&2
    exit 1
  fi
  echo "identity and profile exist"
fi
if [ "$publish" = 1 ]; then
  if [ "$dry" = 1 ]; then
    echo "DRY-RUN would run: gh auth status   (the owner's gh login)"
    echo "DRY-RUN would run: git fetch origin; git branch -r --contains HEAD   (stop when HEAD is not pushed)"
    echo "DRY-RUN would run: gh release view $tag --repo $repo   (stop when it exists, unless --replace)"
    echo "DRY-RUN would run: git ls-remote --tags origin refs/tags/$tag   (stop when it exists, unless --replace)"
    if [ -n "$(git status --porcelain)" ]; then
      echo "DRY-RUN note: the work tree is not clean; a real --publish stops here"
    fi
  else
    command -v gh >/dev/null || { echo "gh not found: install the GitHub CLI and run gh auth login" >&2; exit 1; }
    gh auth status >/dev/null 2>&1 || { echo "gh is not logged in: run gh auth login" >&2; exit 1; }
    [ -z "$(git status --porcelain)" ] || { echo "the work tree is not clean: commit or stash first" >&2; exit 1; }
    git fetch -q origin
    [ -n "$(git branch -r --contains HEAD)" ] || { echo "HEAD $(git rev-parse --short HEAD) is not pushed: push it first" >&2; exit 1; }
    if gh release view "$tag" --repo "$repo" >/dev/null 2>&1; then exists=1; fi
    if git ls-remote --exit-code --tags origin "refs/tags/$tag" >/dev/null 2>&1; then
      # --draft: the tag of the workflow run points at HEAD (annotated tags: the peeled line ^{})
      tagsha=$(git ls-remote --tags origin "refs/tags/$tag" "refs/tags/$tag^{}" | awk '{print $1}' | tail -n 1)
      if [ "$draft" = 1 ] && [ "$tagsha" = "$(git rev-parse HEAD)" ]; then
        echo "tag $tag exists on origin at HEAD: fine for a draft"
      else
        exists=1
      fi
    fi
    if [ "$exists" = 1 ] && [ "$replace" = 0 ]; then
      echo "release or tag $tag exists on github.com/$repo: stop (use --replace to delete and publish again)" >&2
      exit 1
    fi
  fi
fi

# Send one file to Apple and wait; stop with Apple's reason when it is not accepted.
notarize() {
  _file=$1
  _log=$2
  if [ "$dry" = 1 ]; then
    step xcrun notarytool submit "$_file" --keychain-profile "$profile" --wait
    return 0
  fi
  mkdir -p "$(dirname "$_log")"
  xcrun notarytool submit "$_file" --keychain-profile "$profile" --wait 2>&1 | tee "$_log"
  if ! grep -q 'status: Accepted' "$_log"; then
    id=$(sed -n 's/^ *id: *\([0-9a-f-]*\).*/\1/p' "$_log" | head -n 1)
    echo "notarization was not accepted ($_file). Apple's reason:" >&2
    if [ -n "$id" ]; then xcrun notarytool log "$id" --keychain-profile "$profile" >&2 || true; fi
    echo "See docs/RELEASING.md, section 'Errors'." >&2
    exit 1
  fi
}

if [ "$build" = 1 ]; then
  for a in $archs; do
    app=$(app_of "$a")
    dmg=$(dmg_of "$a")
    zip=$(zip_of "$a")
    echo "== $a: build and sign the .app (hardened runtime, entitlements, secure timestamp)"
    # The Tauri build must not sign or notarize by itself: remove its signing variables.
    step env -u APPLE_SIGNING_IDENTITY -u APPLE_CERTIFICATE -u APPLE_CERTIFICATE_PASSWORD \
      -u APPLE_ID -u APPLE_PASSWORD -u APPLE_TEAM_ID -u APPLE_API_KEY -u APPLE_API_ISSUER -u APPLE_API_KEY_PATH \
      DS_CODESIGN_IDENTITY="$ident" DS_CODESIGN_TIMESTAMP=1 "$builder" --arch "$a" --app-only

    echo "== $a: check the signature of the .app"
    step codesign --verify --deep --strict --verbose=2 "$app"
    if [ "$dry" = 1 ]; then
      echo "DRY-RUN would check: Authority=Developer ID Application, TeamIdentifier=2NZX5Q2ESG, flags with 'runtime', a Timestamp line"
    else
      code="$app"
      info=$(codesign -dvv "$code" 2>&1)
        echo "$info" | grep -E 'Identifier|flags|Authority|TeamIdentifier|Timestamp' || true
        echo "$info" | grep -q 'Authority=Developer ID Application' || { echo "$code is not signed with a Developer ID" >&2; exit 1; }
        echo "$info" | grep -q 'flags=.*runtime' || { echo "hardened runtime flag missing: $code" >&2; exit 1; }
        echo "$info" | grep -q '^Timestamp=' || { echo "secure timestamp missing: $code" >&2; exit 1; }
    fi

    echo "== $a: notarize the .app (a zip of it; waits for Apple; log: $(log_of "$a" app))"
    if [ "$dry" = 0 ]; then mkdir -p "$(dirname "$zip")"; rm -f "$zip"; fi
    step ditto -c -k --keepParent "$app" "$zip"
    notarize "$zip" "$(log_of "$a" app)"
    echo "== $a: staple the .app and validate"
    step xcrun stapler staple "$app"
    step xcrun stapler validate "$app"
    step rm -f "$zip"

    echo "== $a: make the .dmg of the stapled .app and sign it"
    step env -u APPLE_SIGNING_IDENTITY -u APPLE_CERTIFICATE -u APPLE_CERTIFICATE_PASSWORD \
      -u APPLE_ID -u APPLE_PASSWORD -u APPLE_TEAM_ID -u APPLE_API_KEY -u APPLE_API_ISSUER -u APPLE_API_KEY_PATH \
      DS_CODESIGN_IDENTITY="$ident" DS_CODESIGN_TIMESTAMP=1 "$builder" --arch "$a" --dmg-only
    step codesign --verify --strict --verbose=2 "$dmg"

    echo "== $a: notarize the .dmg (waits for Apple; log: $(log_of "$a" dmg))"
    notarize "$dmg" "$(log_of "$a" dmg)"

    echo "== $a: staple and validate the .dmg"
    step xcrun stapler staple "$dmg"
    step xcrun stapler validate "$dmg"
    step spctl -a -t open --context context:primary-signature -v "$dmg"
    if [ "$dry" = 1 ]; then
      echo "DRY-RUN would check: the app inside the .dmg has its ticket (hdiutil attach -readonly, stapler validate, hdiutil detach)"
    else
      mnt=$(mktemp -d)
      hdiutil attach -quiet -readonly -nobrowse -noautoopen -mountpoint "$mnt" "$dmg"
      ok=0
      xcrun stapler validate "$mnt/Drum Spike.app" || ok=1
      hdiutil detach -quiet "$mnt" || true
      rmdir "$mnt" 2>/dev/null || true
      [ "$ok" = 0 ] || { echo "the app inside $dmg has no stapled ticket" >&2; exit 1; }
      echo "the app inside the .dmg has its ticket"
      shasum -a 256 "$dmg"
      echo "signed and notarized: $dmg"
    fi
  done
fi

if [ "$publish" = 0 ]; then
  if [ "$dry" = 1 ]; then
    echo "DRY-RUN done. Nothing was built, signed or sent."
  else
    echo "Next: check on a second Mac and the Intel app on x86-mac (docs/RELEASING.md), then --publish-only."
  fi
  exit 0
fi

echo "== publish $tag: assets"
if [ "$build" = 0 ]; then
  for a in $archs; do
    dmg=$(dmg_of "$a")
    if [ "$dry" = 0 ] && [ ! -f "$dmg" ]; then
      echo "no .dmg for $a: $dmg (run release.sh without --publish-only first)" >&2
      exit 1
    fi
    step xcrun stapler validate "$dmg"
    step spctl -a -t open --context context:primary-signature -v "$dmg"
  done
fi
sums=""
for a in $archs; do
  dmg=$(dmg_of "$a")
  if [ -f "$dmg" ]; then
    h=$(shasum -a 256 "$dmg" | cut -d' ' -f1)
  else
    h="(made by the build above)"
  fi
  sums="$sums$h  $(asset_of "$a")
"
done

notes_text() {
  echo "Drum Spike $version for macOS."
  echo
  echo "Drum Spike finds the hits in your close-mic drum tracks and writes MIDI and spike files for them."
  echo
  echo "## Downloads"
  echo
  for a in $archs; do
    if [ "$a" = x86_64 ]; then
      echo "- \`$(asset_of "$a")\`: Intel Macs (x86_64)."
    else
      echo "- \`$(asset_of "$a")\`: Apple silicon Macs (M1 and newer)."
    fi
  done
  echo
  echo "The files are signed with a Developer ID and notarized by Apple. Check a download with \`shasum -a 256 -c SHA256SUMS\`."
  echo
  echo "## What changed"
  echo
  if [ -n "${DRUMSPIKE_RELEASE_NOTES:-}" ]; then
    cat "$DRUMSPIKE_RELEASE_NOTES"
  else
    prev=$(git describe --tags --abbrev=0 HEAD 2>/dev/null || true)
    if [ -z "$prev" ]; then
      echo "First release of the desktop app: projects, track import, DSP drum detection, the editor, playback, MIDI and spike export."
    else
      echo "Changes since $prev:"
      echo
      git log --format='- %s' "$prev..HEAD" | head -n 30
    fi
  fi
  echo
  echo "Drum Spike is free and open source (Apache-2.0)."
}

if [ "$dry" = 1 ]; then
  echo "DRY-RUN tag: $tag (repository $repo, commit $(git rev-parse --short HEAD))"
  echo "DRY-RUN assets and SHA-256 (file SHA256SUMS):"
  printf '%s' "$sums" | sed 's/^/  /'
  echo "  SHA256SUMS"
  echo "DRY-RUN notes:"
  notes_text | sed 's/^/  | /'
else
  rm -rf "$out"
  mkdir -p "$out"
  for a in $archs; do cp "$(dmg_of "$a")" "$out/$(asset_of "$a")"; done
  (cd "$out" && for a in $archs; do shasum -a 256 "$(asset_of "$a")"; done > SHA256SUMS)
  notes_text > "$out/notes.md"
  echo "assets in $out:"
  cat "$out/SHA256SUMS"
fi

# the asset paths as the positional parameters (the argument parsing is done)
set --
for a in $archs; do set -- "$@" "$out/$(asset_of "$a")"; done
if [ "$replace" = 1 ]; then
  if [ "$dry" = 1 ]; then
    echo "DRY-RUN would run (only when the release or tag exists): gh release delete $tag --repo $repo --yes --cleanup-tag"
  elif [ "$exists" = 1 ]; then
    gh release delete "$tag" --repo "$repo" --yes --cleanup-tag
    git push -q origin ":refs/tags/$tag" 2>/dev/null || true
  fi
fi
draftflag=""
[ "$draft" = 0 ] || draftflag="--draft"
# shellcheck disable=SC2086
step gh release create "$tag" "$@" "$out/SHA256SUMS" --repo "$repo" --target "$(git rev-parse HEAD)" \
  --title "Drum Spike $version" --notes-file "$out/notes.md" $draftflag
if [ "$dry" = 1 ]; then
  echo "DRY-RUN done. Nothing was built, signed, sent or published."
else
  gh release view "$tag" --repo "$repo" --json url,tagName,assets --jq '.url, (.assets[] | .name)'
  if [ "$draft" = 1 ]; then
    echo "draft release created: $(gh release view "$tag" --repo "$repo" --json url --jq .url), publish it in GitHub"
  else
    echo "published: $tag (the repository is private: only collaborators see the page)"
  fi
fi
