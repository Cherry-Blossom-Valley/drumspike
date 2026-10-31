"""#104: tests for the private-name text guard. Fixtures use invented words only
(ZUMQUUX, FLOOB-FRAME), never a real private name."""
import re
import subprocess
import sys
from pathlib import Path

TOOLS = Path(__file__).resolve().parents[1] / "tools"
sys.path.insert(0, str(TOOLS))

import check_private  # noqa: E402

REPO = Path(__file__).resolve().parents[1]
INVENTED = "ZUMQUUX"
INVENTED_PAT = r"[a-z]+-flooob-frame\.npz"


def run(argv, env_extra):
    import os
    env = {k: v for k, v in os.environ.items() if not k.startswith("DRUMSPIKE_PRIVATE_")}
    env.update(env_extra)
    return subprocess.run([sys.executable, str(TOOLS / "check_private.py"), *argv],
                          capture_output=True, text=True, env=env, cwd=REPO)


def test_planted_word_found(tmp_path):
    f = tmp_path / "draft.md"
    f.write_text(f"clean line\nthe take is {INVENTED} take 7\n", encoding="utf-8")
    r = run(["--text-file", str(f)], {"DRUMSPIKE_PRIVATE_WORDS": "quuxcorp,zumquux"})
    assert r.returncode == 1
    assert "draft.md:2" in r.stderr
    assert INVENTED.lower() in r.stderr.lower()


def test_planted_pattern_found(tmp_path):
    f = tmp_path / "body.md"
    f.write_text("dataset: some-flooob-frame.npz here\n", encoding="utf-8")
    r = run(["--text-file", str(f)],
            {"DRUMSPIKE_PRIVATE_WORDS": "quuxcorp", "DRUMSPIKE_PRIVATE_PATTERNS": INVENTED_PAT})
    assert r.returncode == 1
    assert "body.md:1" in r.stderr
    assert "flooob" in r.stderr


def test_unset_list_is_loud():
    r = run(["--diff", "origin/main"], {})
    assert r.returncode == 2
    assert "unset" in r.stderr.lower()


def test_allow_empty_passes_unset(tmp_path):
    """--allow-empty lets an unset DRUMSPIKE_PRIVATE_WORDS pass on a clean text; the
    base list still applies. The clean case is a plain file, not the self-diff:
    #108 adds the alias allowlist literals to the guard itself, so the branch diff
    is never base-word clean."""
    f = tmp_path / "ok.md"
    f.write_text("nothing private here\n", encoding="utf-8")
    r = run(["--text-file", str(f), "--allow-empty"], {})
    assert r.returncode == 0, r.stderr


def test_base_word_flagged_in_file(tmp_path):
    """A base-list word (/Users) in a plain file is flagged."""
    f = tmp_path / "x.txt"
    f.write_text("/Users/someone/x\n", encoding="utf-8")
    r = run([str(f)], {"DRUMSPIKE_PRIVATE_WORDS": "quuxcorp"})
    assert r.returncode == 1
    assert "/Users" in r.stderr


def test_clean_text_file_passes(tmp_path):
    f = tmp_path / "ok.md"
    f.write_text("Nothing private here, all invented words only.\n", encoding="utf-8")
    r = run([str(f)], {"DRUMSPIKE_PRIVATE_WORDS": "quuxcorp,zumquux"})
    assert r.returncode == 0


def test_diff_no_planted_word_leaks():
    """--diff origin/main with an invented word built at run time (its literal must
    not appear in this diff): no hit names it. The self-diff of this branch carries
    the guard's own alias allowlist literals (#108), so a plain rc=0 is not
    assertable here; the contract kept is that a planted word never leaks into the
    diff scan."""
    planted = "zorp" + "blat"
    r = run(["--diff", "origin/main"], {"DRUMSPIKE_PRIVATE_WORDS": planted,
                                        "DRUMSPIKE_PRIVATE_PATTERNS": INVENTED_PAT})
    assert planted not in r.stderr
    assert "flooob-frame" not in r.stderr


def test_shared_definition_with_check_paths():
    """One definition, not two copies: check_private's private_words is the function of
    the loaded check_paths module, not a re-defined tuple."""
    assert check_private._check_paths.__file__ == str(TOOLS / "media" / "check_paths.py")
    assert check_private.private_words is check_private._check_paths.private_words
    assert check_private.BANNED_BASE == check_paths_module().BANNED_BASE


def check_paths_module():
    import importlib.util
    spec = importlib.util.spec_from_file_location(
        "check_paths", TOOLS / "media" / "check_paths.py")
    assert spec is not None and spec.loader is not None
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def test_bad_pattern_regex_is_loud(tmp_path):
    f = tmp_path / "x.md"
    f.write_text("text\n", encoding="utf-8")
    r = run([str(f)], {"DRUMSPIKE_PRIVATE_WORDS": "quuxcorp",
                       "DRUMSPIKE_PRIVATE_PATTERNS": "(unclosed"})
    assert r.returncode == 2


# #108: agent and branch names with a host alias are allowed; the bare alias stays banned.
def test_agent_name_passes(tmp_path):
    f = tmp_path / "note.md"
    f.write_text("assigned to claude@arm64-mac for review\n", encoding="utf-8")
    r = run([str(f)], {"DRUMSPIKE_PRIVATE_WORDS": "quuxcorp"})
    assert r.returncode == 0, r.stderr


def test_branch_name_passes(tmp_path):
    f = tmp_path / "note.md"
    f.write_text("branch: crew/hermes@arm64-mac/108-check-private-py\n", encoding="utf-8")
    r = run([str(f)], {"DRUMSPIKE_PRIVATE_WORDS": "quuxcorp"})
    assert r.returncode == 0, r.stderr


def test_bare_alias_flagged(tmp_path):
    f = tmp_path / "note.md"
    f.write_text("the run happened on arm64-mac\n", encoding="utf-8")
    r = run([str(f)], {"DRUMSPIKE_PRIVATE_WORDS": "quuxcorp"})
    assert r.returncode == 1
    assert "arm64-mac" in r.stderr


# #108 send-back: a banned login token plus an alias must still FAIL, in every form.
# Only the alias part is masked; the part before @ stays in the search text.
def test_login_token_with_alias_flagged(tmp_path):
    f = tmp_path / "note.md"
    f.write_text("ssh zumquux-77@arm64-mac first.\n", encoding="utf-8")
    r = run([str(f)], {"DRUMSPIKE_PRIVATE_WORDS": "zumquux-77"})
    assert r.returncode == 1
    assert "zumquux-77" in r.stderr


# #164 decision (narrow): an added diff line quoting the shared base list passes only when
# it carries the marker comment and ALL its hits are BANNED_BASE tokens. A host word on a
# marked line still fails; an unmarked base token still fails. The samples are built at run
# time from the shared constant and planted words, so this diff carries no literal token
# outside the marked definition lines.
def _diff_scan(tmp_path, lines, env_extra):
    import os
    repo = tmp_path / "repo"
    repo.mkdir()

    def git(*args):
        subprocess.run(["git", *args], cwd=repo, check=True, capture_output=True)

    git("init", "-q", "-b", "main")
    git("config", "user.email", "t@example.com")
    git("config", "user.name", "t")
    (repo / "f.txt").write_text("clean\n", encoding="utf-8")
    git("add", ".")
    git("commit", "-qm", "base")
    git("checkout", "-qb", "feature")
    (repo / "f.txt").write_text("clean\n" + "\n".join(lines) + "\n", encoding="utf-8")
    git("commit", "-am", "second")
    env = {k: v for k, v in os.environ.items() if not k.startswith("DRUMSPIKE_PRIVATE_")}
    env.update(env_extra)
    env.setdefault("DRUMSPIKE_PRIVATE_WORDS", "quuxcorp")
    return subprocess.run(
        [sys.executable, str(TOOLS / "check_private.py"), "--diff", "main"],
        capture_output=True, text=True, env=env, cwd=repo)


def test_marked_base_only_line_passes(tmp_path):
    line = f'const X = ["{check_private.BANNED_BASE[0]}"];  // {check_private.LIST_MARKER}'
    r = _diff_scan(tmp_path, [line], {})
    assert r.returncode == 0, r.stderr


def test_marked_line_with_host_word_fails(tmp_path):
    planted = "zorp" + "blat"
    line = (f'const X = ["{check_private.BANNED_BASE[0]}"];  // '
            f'{check_private.LIST_MARKER} planted {planted}')
    r = _diff_scan(tmp_path, [line], {"DRUMSPIKE_PRIVATE_WORDS": f"quuxcorp,{planted}"})
    assert r.returncode == 1, r.stderr
    assert planted in r.stderr


def test_marked_line_with_pattern_hit_fails(tmp_path):
    line = (f'const X = ["{check_private.BANNED_BASE[0]}"];  // '
            f'{check_private.LIST_MARKER} some-' + "flooob" + '-frame.npz')
    r = _diff_scan(tmp_path, [line], {"DRUMSPIKE_PRIVATE_PATTERNS": INVENTED_PAT})
    assert r.returncode == 1, r.stderr


def test_unmarked_base_token_fails(tmp_path):
    line = f'const X = ["{check_private.BANNED_BASE[0]}"];'
    r = _diff_scan(tmp_path, [line], {})
    assert r.returncode == 1, r.stderr
    assert check_private.BANNED_BASE[0] in r.stderr


def test_app_guards_share_the_base_list():
    """The Rust and JS guard constants and check_paths.BANNED_BASE are the one list, so
    they cannot drift."""
    rust = (REPO / "crates" / "app" / "src" / "feedback.rs").read_text(encoding="utf-8")
    m = re.search(r"pub const PRIVATE_WORDS: \[&str; (\d+)\] = \[(.*?)\];", rust, re.S)
    assert m is not None
    n = int(m.group(1))
    rust_items = re.findall(r'"([^"]*)"', m.group(2))
    assert len(rust_items) == n
    assert tuple(rust_items) == check_private.BANNED_BASE
    js = (REPO / "static" / "shell.js").read_text(encoding="utf-8")
    m2 = re.search(r"const FB_PRIVATE = \[(.*?)\];", js, re.S)
    assert m2 is not None
    assert tuple(re.findall(r'"([^"]*)"', m2.group(1))) == check_private.BANNED_BASE


def test_login_token_with_alias_in_branch_flagged(tmp_path):
    f = tmp_path / "note.md"
    f.write_text("branch: crew/hermes@arm64-mac/108-zumquux-77\n", encoding="utf-8")
    r = run([str(f)], {"DRUMSPIKE_PRIVATE_WORDS": "zumquux-77"})
    assert r.returncode == 1
    assert "zumquux-77" in r.stderr
