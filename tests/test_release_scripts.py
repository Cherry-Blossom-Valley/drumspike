"""X-5 (P3-9b): release.sh notarizes and staples the .app before the .dmg is made, then notarizes and
staples the .dmg, for each architecture. Agents run only --dry-run: the names below are not real (no
keychain item, no notary profile, no gh command runs). build-dmg.sh has the phases --app-only and
--dmg-only for this order."""
import os
import subprocess
import sys

import pytest

from helpers import ROOT

pytestmark = pytest.mark.skipif(sys.platform != "darwin", reason="the release scripts run on macOS only")

RELEASE = ROOT / "apps/desktop/scripts/release.sh"
BUILD_DMG = ROOT / "apps/desktop/scripts/build-dmg.sh"
FAKE = {"DRUMSPIKE_SIGN_IDENTITY": "Developer ID Application: Test Only (TEST000000)",
        "DRUMSPIKE_NOTARY_PROFILE": "test-only-profile"}


def dry_run(*args, bundle=None):
    env = dict(os.environ, **FAKE)
    env.pop("DRUMSPIKE_BUNDLE_DIR", None)
    if bundle:
        env["DRUMSPIKE_BUNDLE_DIR"] = str(bundle)
    r = subprocess.run([str(RELEASE), "--dry-run", *args], cwd=ROOT, env=env, capture_output=True, text=True, timeout=60)
    assert r.returncode == 0, r.stdout + r.stderr
    return r.stdout.splitlines()


def first(lines, *parts, after=-1):
    """The index of the first line after `after` that holds every part, else a failure with the lines."""
    for i, line in enumerate(lines):
        if i > after and all(p in line for p in parts):
            return i
    raise AssertionError(f"no line with {parts} after line {after}:\n" + "\n".join(lines))


@pytest.mark.parametrize("arch,triple,suffix", [("arm64", "aarch64-apple-darwin", "aarch64"),
                                                ("x86_64", "x86_64-apple-darwin", "x86_64")])
def test_the_app_is_notarized_and_stapled_before_the_dmg_is_made(arch, triple, suffix):
    lines = dry_run("--arch", arch)
    app = f"target/{triple}/release/bundle/macos/Drum Spike.app"
    dmg = f"target/{triple}/release/bundle/dmg/Drum Spike_"
    build_app = first(lines, "DRY-RUN would run:", "build-dmg.sh", "'--arch' '%s'" % arch, "'--app-only'")
    submit_app = first(lines, "DRY-RUN would run:", "'notarytool' 'submit'", "Drum Spike.app.zip", after=build_app)
    staple_app = first(lines, "DRY-RUN would run:", "'stapler' 'staple'", app, after=submit_app)
    validate_app = first(lines, "DRY-RUN would run:", "'stapler' 'validate'", app, after=staple_app)
    make_dmg = first(lines, "DRY-RUN would run:", "build-dmg.sh", "'--arch' '%s'" % arch, "'--dmg-only'", after=validate_app)
    submit_dmg = first(lines, "DRY-RUN would run:", "'notarytool' 'submit'", dmg, f"_{suffix}.dmg", after=make_dmg)
    staple_dmg = first(lines, "DRY-RUN would run:", "'stapler' 'staple'", f"_{suffix}.dmg", after=submit_dmg)
    first(lines, "DRY-RUN would run:", "'stapler' 'validate'", f"_{suffix}.dmg", after=staple_dmg)
    # the app inside the stapled .dmg has its own ticket (checked on a read-only mount)
    first(lines, "DRY-RUN would check", "inside the .dmg", "stapler validate", after=staple_dmg)
    # no .dmg is made before the app has its ticket, and no step builds the whole thing at once
    assert not any("build-dmg.sh" in x and "--app-only" not in x and "--dmg-only" not in x for x in lines), lines
    assert not any("'stapler' 'staple'" in x and ".dmg" in x for x in lines[:make_dmg])


def test_both_architectures_follow_the_order_and_nothing_runs():
    lines = dry_run()
    for triple in ("aarch64-apple-darwin", "x86_64-apple-darwin"):
        app = f"target/{triple}/release/bundle/macos/Drum Spike.app"
        staple_app = first(lines, "'stapler' 'staple'", app)
        first(lines, "build-dmg.sh", "'--dmg-only'", after=staple_app)
    assert lines[-1] == "DRY-RUN done. Nothing was built, signed or sent."
    assert not any(x.startswith("== ") and "signed and notarized" in x for x in lines)


def test_the_release_notes_name_no_model_licence():
    text = RELEASE.read_text(encoding="utf-8")
    assert "Drum Spike is free and open source (Apache-2.0)." in text
    lowered = text.lower()
    # the forbidden licence names, split so this checker file carries no model name itself
    for term in ("ad" + "tof", "cc by-nc-sa", "non-commercial", "on" + "nx"):
        assert term not in lowered, term


def test_build_dmg_knows_the_phases_and_the_dmg_phase_needs_the_app(tmp_path):
    r = subprocess.run([str(BUILD_DMG), "--bogus"], cwd=ROOT, capture_output=True, text=True, timeout=60)
    assert r.returncode == 64 and "--app-only" in r.stderr and "--dmg-only" in r.stderr, r.stderr
    env = dict(os.environ, DRUMSPIKE_BUNDLE_DIR=str(tmp_path))
    r = subprocess.run([str(BUILD_DMG), "--arch", "x86_64", "--dmg-only"], cwd=ROOT, env=env,
                       capture_output=True, text=True, timeout=120)
    assert r.returncode == 1 and "no app at" in r.stderr, r.stdout + r.stderr
    assert "release build" not in r.stdout and "ON" + "NX Runtime" not in r.stdout   # the dmg phase builds nothing
