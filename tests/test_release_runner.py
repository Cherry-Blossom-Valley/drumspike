"""#536: the signing runner of release.yml comes from the repository variable DRUMSPIKE_RELEASE_RUNNER.

The owner route keeps .github/workflows out of the agent PRs: the change to release.yml travels
as docs/patches/536-workflow.patch and the owner applies it at the merge. When the patch is
present and release.yml does not yet name the variable, the test applies the patch to a copy and
checks that. After the merge (release.yml carries the change) it checks release.yml directly.
"""

import shutil
import subprocess
import tempfile
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parent.parent
WORKFLOW = ROOT / ".github" / "workflows" / "release.yml"
PATCH = ROOT / "docs" / "patches" / "536-workflow.patch"
CI = ROOT / ".github" / "workflows" / "ci.yml"
SIGNING_JOBS = ("check", "build", "publish")
INTEL_RUNS_ON = ["self-hosted", "x86-mac"]
VARIABLE = "DRUMSPIKE_RELEASE_RUNNER"
DEFAULT_LABEL = "arm64-mac"


def workflow_text() -> str:
    """The effective release.yml: patched copy when the owner patch is pending."""
    text = WORKFLOW.read_text()
    if VARIABLE in text:
        return text
    assert PATCH.exists(), (
        f"{WORKFLOW} does not name {VARIABLE} and {PATCH} is missing: "
        "the runner change is neither applied nor shipped as the owner patch"
    )
    with tempfile.TemporaryDirectory() as tmp:
        repo = Path(tmp)
        (repo / ".github" / "workflows").mkdir(parents=True)
        shutil.copy2(WORKFLOW, repo / ".github" / "workflows" / "release.yml")
        shutil.copy2(CI, repo / ".github" / "workflows" / "ci.yml")
        shutil.copy2(PATCH, repo / "change.patch")
        subprocess.run(["git", "init", "-q"], cwd=repo, check=True)
        subprocess.run(["git", "apply", "--check", "change.patch"], cwd=repo, check=True)
        subprocess.run(["git", "apply", "change.patch"], cwd=repo, check=True)
        patched = (repo / ".github" / "workflows" / "release.yml").read_text()
    for line in PATCH.read_text().splitlines():
        if line.startswith("diff --git"):
            assert "release.yml" in line, f"the owner patch touches a file it must not: {line}"
            assert "ci.yml" not in line, "the owner patch must not touch ci.yml"
    return patched


def jobs_runs_on(text: str) -> dict:
    """Top-level jobs of the workflow with their runs-on value (a minimal reader, no yaml dep)."""
    lines = text.splitlines()
    jobs: dict = {}
    current = None
    for line in lines:
        if line.startswith("jobs:"):
            current = "scan"
            continue
        if current is None:
            continue
        if line and not line.startswith(" "):
            current = None
            continue
        job = line[2:].split(":")[0].strip()
        if line.startswith("  ") and not line.startswith("    ") and line.rstrip().endswith(":"):
            if job in {"true", "false", "null"} or job.startswith("${{"):
                current = None
            else:
                current = job
                jobs.setdefault(job, None)
            continue
        if current in jobs and line.startswith("    runs-on:"):
            jobs[current] = line.split(":", 1)[1].strip()
    return jobs


def test_the_signing_jobs_take_their_runner_from_the_variable():
    jobs = jobs_runs_on(workflow_text())
    checked = 0
    for name in SIGNING_JOBS:
        assert name in jobs, f"job {name} not found in release.yml"
        runs_on = jobs[name]
        assert runs_on is not None, f"job {name} has no runs-on"
        # not a fixed signing label: the expression must read the repository variable
        assert VARIABLE in runs_on, (
            f"job {name} pins its runner instead of reading {VARIABLE}: {runs_on}"
        )
        assert "[" not in runs_on.split(VARIABLE)[0], (
            f"job {name} names a fixed signing label before the variable: {runs_on}"
        )
        # the default is the label of today
        assert DEFAULT_LABEL in runs_on, f"job {name} does not default to {DEFAULT_LABEL}: {runs_on}"
        checked += 1
    print(f"signing jobs checked: {checked} ({', '.join(SIGNING_JOBS)})")
    assert checked > 0


def test_intel_check_is_unchanged():
    jobs = jobs_runs_on(workflow_text())
    assert "intel-check" in jobs, "job intel-check not found in release.yml"
    assert jobs["intel-check"] == "[self-hosted, x86-mac]", (
        f"intel-check changed: {jobs['intel-check']}"
    )
    # the variable must not leak into the Intel job
    assert VARIABLE not in (jobs["intel-check"] or "")


def test_ci_yml_is_untouched():
    text = CI.read_text()
    assert VARIABLE not in text, "ci.yml must stay out of the runner change"


def test_messages_name_the_runner_of_the_run_not_a_fixed_label():
    text = workflow_text()
    # the signing-key error names the label of the run, not a hardcoded host
    assert "Unlock the login keychain on arm64-mac" not in text
    key_error = [ln for ln in text.splitlines() if "signing key is not usable" in ln]
    assert key_error, "the signing-key error message is missing"
    assert VARIABLE in key_error[0] or "DRUMSPIKE_RELEASE_RUNNER_LABEL" in key_error[0], (
        f"the signing-key error does not name the runner of the run: {key_error[0]}"
    )
    # no step message names the signing host as a fixed label
    fixed = [ln for ln in text.splitlines() if "::error::" in ln and "arm64-mac" in ln]
    assert not fixed, f"fixed signing label in a step message: {fixed}"


if __name__ == "__main__":
    raise SystemExit(pytest.main([__file__, "-q"]))
