# Releasing Drum Spike

A release is two `.dmg` files, one per architecture, both signed with a Developer ID and notarized
by Apple, plus `SHA256SUMS` and release notes on the GitHub release page. Only maintainers with
the signing Mac release; contributors and agents run `--dry-run` only.

| Architecture | Macs | File |
|---|---|---|
| `arm64` | Apple silicon (M1 and newer) | `target/aarch64-apple-darwin/release/bundle/dmg/Drum Spike_<version>_aarch64.dmg` |
| `x86_64` | Intel | `target/x86_64-apple-darwin/release/bundle/dmg/Drum Spike_<version>_x86_64.dmg` |

The Apple Silicon Mac builds both (the Intel app is a cross build). The app bundles no dynamic
libraries; `build-dmg.sh` checks that. The scripts handle two names only (the
signing identity and the notary profile); they never read a password, a certificate or a key.

## Release by tag (the normal way)

`.github/workflows/release.yml` runs on the self-hosted signing runner (label `arm64-mac`). The
key never leaves that Mac's keychain.

1. `ci` is green on the release commit. The version is set in `Cargo.toml` (workspace),
   `apps/desktop/src-tauri/tauri.conf.json`, `apps/desktop/package.json` and
   `apps/desktop/selftest/shell.json`, and `docs/release-notes/release-notes-X.Y.Z.md` exists.
   `tools/release/cut.py plan`, `notes` and `bump` prepare these steps.
2. Push the tag: `git tag vX.Y.Z && git push origin vX.Y.Z`.
3. Job `check` stops early when the tag differs from one of the four versions, the notes file is
   missing, or `ci` is not green on the commit (a docs-only commit has no `ci` run; the check then
   looks at its parents).
4. Job `build` tests that the signing key works (a locked keychain fails here with a clear
   message), runs `release.sh` for both architectures and uploads the two `.dmg` files and
   `SHA256SUMS`.
5. Job `intel-check` runs the Intel check (below) on the Intel runner (label `x86-mac`).
6. Job `publish` makes a DRAFT release (`release.sh --publish-only --draft`). A maintainer reads
   the draft and presses "Publish release"; deleting the draft publishes nothing.
   `tools/release/cut.py status vX.Y.Z` prints the job times and checks the assets against
   `SHA256SUMS`.

Dry run (everything except the publish), on any commit: Actions > release > Run workflow with
`dry_run`, or `gh workflow run release.yml -f dry_run=true`.

The signing runner needs: the login keychain unlocked, `gh` logged in as a maintainer (the draft is
made with that login), and the runner service in the login security session (see "Errors",
`errSecInternalComponent`). The first signature from the runner may ask once to let `codesign` use
the key: choose "Always Allow".

## Before the first manual run (once)

1. The keychain holds a "Developer ID Application" certificate with its private key:
   `security find-identity -v -p codesigning`.
2. The keychain holds a notary profile: `xcrun notarytool store-credentials <profile>` asks for the
   Apple ID, the team ID and an app-specific password. Type them into the tool, never into a file.
3. The Xcode command line tools (`xcrun --find notarytool`) and the Intel Rust target
   (`rustup target add x86_64-apple-darwin`) are installed.
4. `gh auth status` shows a maintainer login.

## Manual procedure (fallback when the runner is down)

```sh
export DRUMSPIKE_SIGN_IDENTITY="Developer ID Application: <name> (<team id>)"
export DRUMSPIKE_NOTARY_PROFILE=<profile>
apps/desktop/scripts/release.sh --dry-run            # prints every command, runs none
apps/desktop/scripts/release.sh                      # both architectures: build, sign, notarize, staple
```

Then run the Intel check and the checks below. Publish only when they pass:

```sh
apps/desktop/scripts/release.sh --publish-only --dry-run
apps/desktop/scripts/release.sh --publish-only
```

For each architecture the script: checks that the identity and the profile exist; builds and signs
the `.app` with the hardened runtime and a secure timestamp; sends a zip of the `.app` to Apple,
staples and validates the ticket; makes the `.dmg` of the stapled `.app` and signs it; sends the
`.dmg` to Apple, staples and validates it; mounts the `.dmg` and checks that the `.app` inside has
its ticket (so a copied app starts offline). `--arch arm64` or `--arch x86_64` limits a run to one
architecture.

`--publish-only` checks that `gh` is logged in, the work tree is clean and `HEAD` is pushed; stops
when the tag or the release exists (`--replace` deletes them first); copies the files with names
without spaces (`Drum-Spike_<version>_<arch>.dmg`); writes `SHA256SUMS` and the notes
(`DRUMSPIKE_RELEASE_NOTES=<file>` for your own text); and runs `gh release create`.
`DRUMSPIKE_GH_REPO=<owner>/<name>` selects another repository (default
`Cherry-Blossom-Valley/drumspike`).

## Check the Intel app

Under the hardened runtime, library validation needs a Team ID, which only the Developer ID build
has. On an Intel Mac:

```sh
v=X.Y.Z
mkdir -p ~/release-check && cd ~/release-check && rm -rf mnt app && mkdir mnt app
hdiutil attach -readonly -nobrowse -noautoopen -mountroot mnt "Drum Spike_${v}_x86_64.dmg"
ditto "mnt/Drum Spike/Drum Spike.app" "app/Drum Spike.app" && hdiutil detach "mnt/Drum Spike"
codesign -dvv "app/Drum Spike.app/Contents/MacOS/drumspike-desktop" 2>&1 | grep -E "Authority=Developer|TeamIdentifier"
"app/Drum Spike.app/Contents/MacOS/drumspike-desktop" --app-info
```

Pass: `Authority=Developer ID Application` and the team ID, `--app-info` exits 0 with `"ok":true`.

## What success looks like

```sh
spctl -a -t open --context context:primary-signature -v "<file>.dmg"   # accepted, source=Notarized Developer ID
xcrun stapler validate "<file>.dmg"                                    # The validate action worked!
shasum -a 256 -c SHA256SUMS                                            # every file: OK
```

## Check on a second Mac

Use a Mac without the build tools, the `aarch64` file on Apple silicon and the `x86_64` file on
Intel. Download the `.dmg` (a download sets the quarantine flag, so Gatekeeper checks it), drag
"Drum Spike" to Applications and start it: no "cannot be opened" dialog, Drum Spike > About Drum
Spike shows the version and the licence notice. Offline test: turn Wi-Fi off, copy the app from the `.dmg` again,
start it.

## Entitlements

The file is `apps/desktop/src-tauri/entitlements.plist`. It is empty on purpose. The app needs no
exception to the hardened runtime: audio is output only (no microphone key); no plug-ins are
loaded; no App Sandbox (the app opens
project folders anywhere). The comment in the file explains each key that was left
out. Add a key only when a test shows the need, and say why in the file.

## Errors

Read Apple's reason first: `xcrun notarytool log <submission-id> --keychain-profile drumspike-notary`.
The script prints it when a submission is not accepted. The submission ID is in
`target/release-notary-<arch>-app.log` (the `.app`) or `target/release-notary-<arch>-dmg.log`
(the `.dmg`); `DRUMSPIKE_RELEASE_LOG=<path>.log` moves them to `<path>-<arch>-<app|dmg>.log`.

| Message | Cause | Action |
|---|---|---|
| `errSecInternalComponent` from the runner's probe step (release.yml) | The runner service runs in its own security session (`SessionCreate` true in its plist) and cannot use the login keychain | Set `SessionCreate` to false in the runner's launchd plist, restart the runner service (`./svc.sh stop && ./svc.sh start`), run the workflow again. |
| `identity not found in the keychain` | Name differs from the certificate, or the certificate expired or has no private key | Run `security find-identity -v -p codesigning`. Copy the name exactly. Install the certificate again if it is missing. |
| `notary profile not usable` | Profile never made, made on another Mac or another user, or Apple refused the sign-in (run the check command it prints to see the reason) | Run `xcrun notarytool store-credentials drumspike-notary`. |
| `HTTP status code: 401` or "Invalid credentials" | App-specific password revoked, or Apple ID or team ID wrong | Make a new app-specific password at appleid.apple.com. Run `store-credentials` again. |
| "You must first sign the relevant contracts" (403) | New Apple developer agreement not accepted | Accept it at developer.apple.com (Account). Run the script again. |
| `The signature of the binary is invalid` or `is not signed with a valid Developer ID certificate` | Signed with the wrong certificate (for example "Apple Development") | Use the "Developer ID Application" certificate. |
| `The signature does not include a secure timestamp` | Signed without `--timestamp`, or the timestamp server was not reachable | Check the network. Run the script again; it always passes `--timestamp`. |
| `The executable does not have the hardened runtime enabled` | Signing step skipped or replaced | Run the script again. Do not sign by hand. |
| `The binary uses an SDK older than 10.9` or "the binary is not signed" for a nested file | A new nested binary in the bundle | Sign the nested file first, then the `.app`. Change `build-dmg.sh`. |
| Status `Invalid` for another reason | See the `issues` list in the log | Fix the cause. The dmg is rebuilt on each run. |
| `Package Invalid` after a long wait, or no answer | Apple service is slow | Run `xcrun notarytool history --keychain-profile drumspike-notary`, wait, and read the log by ID. |
| `stapler staple` says "Could not find base64 encoded ticket" or error 65 | Ticket not ready yet | Wait one minute and run `xcrun stapler staple` again on the same file (the `.app` or the `.dmg`). |
| `the app inside ... has no stapled ticket` | The `.dmg` was made from an `.app` without a ticket (an old `.dmg`, or `build-dmg.sh` run by hand without `--dmg-only`) | Run the script again. It staples the `.app` before it makes the `.dmg`. |
| `no app at ... (run build-dmg.sh --app-only first)` | `build-dmg.sh --dmg-only` found no `.app` | Run the script again; it runs `--app-only` first. |
| `Rust target missing: rustup target add x86_64-apple-darwin` | The Intel target is not installed | Run the command it prints. Never run `rustup default`. |
| `release or tag v<version> exists on github.com/...: stop` | The version was published before | Raise `"version"` in `tauri.conf.json`, commit, push, run again. Or, to publish the same version again, add `--replace`. |
| `gh is not logged in` | No `gh` login on the signing Mac | `gh auth login`. |
| `HEAD ... is not pushed` or `the work tree is not clean` | The release commit is not on GitHub | Commit, `git push`, run again. |
| `--publish needs a notarized .dmg` | The two variables are not set | Export them (section "Procedure"). |
| `spctl` says `rejected` and `source=Unnotarized Developer ID` | The `.dmg` was not stapled, or an old file is used | Run the script again and use the new file. |
| Second Mac still shows the dialog | The file came from the ad hoc build, or a different copy | Check `codesign -dvv` shows the Developer ID authority and `TeamIdentifier=2NZX5Q2ESG`. |

## Rules

- Never put a password, a certificate or a key in a file, a commit or a chat.
- The app has no updater.
- No dynamic library is committed or bundled; `build-dmg.sh` stops when the bundle would contain one.
- Tags and releases are made by maintainers only.
