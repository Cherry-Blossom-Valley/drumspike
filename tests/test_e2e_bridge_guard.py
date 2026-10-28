"""#495: the e2e_app guard. The session fixture of tests/e2e_app/conftest.py must refuse a stale
ds-bridge (a binary older than a source misled two gates on 2026-10-28) and start the bridge with
this worktree's static/. Stale means: a file that Cargo lists in the dep-info next to the binary
(<target>/debug/ds-bridge.d, the build graph the binary was really built from) is newer than it.
Without a dep-info file the fallback is every *.rs under crates/core, crates/app and
crates/devtools — not apps/desktop/src-tauri/src and not Cargo.lock: a change outside the build
graph of ds-bridge never relinks it, so a refusal from it could never be cleared (CI sets
DRUMSPIKE_REQUIRE_APP=1, e2e would stay red until the next core/app change). Unit test over a
temp tree: no bridge, no browser, no Rust build."""
import importlib.util
import os
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
E2E = HERE / "e2e_app"
T0 = 2_000_000_000   # the planted build time of the fake binary (local time in the refusal line)


def _conftest():
    """The e2e_app conftest as a plain module (loaded under its own name, its fixtures stay dormant)."""
    for p in (str(HERE), str(E2E)):   # conftest imports helpers (tests/) and app_support (e2e_app/)
        if p not in sys.path:
            sys.path.insert(0, p)
    spec = importlib.util.spec_from_file_location("e2e_app_conftest_guard", E2E / "conftest.py")
    assert spec is not None and spec.loader is not None
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def _planted(root):
    """A repo-shaped temp tree: the fake target/debug/ds-bridge built at T0, a source 100 s later."""
    src = root / "crates" / "core" / "src"
    src.mkdir(parents=True)
    (src / "lib.rs").write_text("// planted\n")
    os.utime(src / "lib.rs", (T0 + 100, T0 + 100))
    exe = root / "target" / "debug" / "ds-bridge"
    exe.parent.mkdir(parents=True)
    exe.write_bytes(b"\xcf\xfa\xed\xfe fake mach-o")
    os.utime(exe, (T0, T0))
    return exe


def _planted_fresh(root):
    """The same tree, but the binary touched newer than the planted source (T0+200)."""
    exe = _planted(root)
    os.utime(exe, (T0 + 200, T0 + 200))
    return exe


def _dep_info(exe, listed):
    """The dep-info next to the fake binary, the way Cargo writes it: one line 'target: dep ...'."""
    dep = Path(str(exe) + ".d")
    dep.write_text(f"{exe}: {listed}\n")
    os.utime(dep, (T0 + 200, T0 + 200))
    return dep


def test_stale_bridge_is_refused(tmp_path):
    """A binary older than a planted .rs file: the one-line refusal names built time and source."""
    mod = _conftest()
    root = tmp_path / "repo"
    root.mkdir()
    exe = _planted(root)
    msg = mod.stale_source(exe, root)
    assert msg is not None
    assert msg.startswith("stale ds-bridge (built ")
    assert "crates/core/src/lib.rs" in msg
    assert msg.endswith(": run cargo build -p drumspike-devtools --bin ds-bridge")
    assert "\n" not in msg


def test_fresh_bridge_passes(tmp_path):
    """The same tree with the binary touched newer than the source: no refusal."""
    mod = _conftest()
    root = tmp_path / "repo"
    root.mkdir()
    exe = _planted(root)
    os.utime(exe, (T0 + 200, T0 + 200))
    assert mod.stale_source(exe, root) is None


def test_dep_info_listed_source_is_refused(tmp_path):
    """A source the dep-info lists is newer than the binary: refused, the line names it."""
    mod = _conftest()
    root = tmp_path / "repo"
    root.mkdir()
    exe = _planted_fresh(root)
    lib = root / "crates" / "core" / "src" / "lib.rs"
    os.utime(lib, (T0 + 300, T0 + 300))   # newer than the binary again
    _dep_info(exe, str(lib))
    msg = mod.stale_source(exe, root)
    assert msg is not None
    assert "crates/core/src/lib.rs" in msg


def test_dep_info_ignores_sources_outside_the_build_graph(tmp_path):
    """A newer apps/desktop/src-tauri/src/lib.rs and a newer Cargo.lock do NOT refuse a binary
    whose dep-info does not list them: they are not in the build graph of ds-bridge, so
    'cargo build -p drumspike-devtools --bin ds-bridge' is a no-op and a refusal from them could
    never be cleared. Red on 3377001, green after the dep-info fix."""
    mod = _conftest()
    root = tmp_path / "repo"
    root.mkdir()
    exe = _planted_fresh(root)
    _dep_info(exe, str(root / "crates" / "core" / "src" / "lib.rs"))   # listed, and older: fine
    tauri = root / "apps" / "desktop" / "src-tauri" / "src"
    tauri.mkdir(parents=True)
    (tauri / "lib.rs").write_text("// release fix\n")
    os.utime(tauri / "lib.rs", (T0 + 300, T0 + 300))   # newer than the binary, NOT listed
    (root / "Cargo.lock").write_text("")
    os.utime(root / "Cargo.lock", (T0 + 300, T0 + 300))   # newer than the binary, NOT listed
    assert mod.stale_source(exe, root) is None


def test_cargo_lock_alone_is_not_a_source(tmp_path):
    """No dep-info file: the fallback is the three crates ds-bridge is built from. A newer
    Cargo.lock alone does not refuse (a lock change outside their graphs never relinks it)."""
    mod = _conftest()
    root = tmp_path / "repo"
    root.mkdir()
    exe = _planted_fresh(root)   # the planted lib.rs (T0+100) stays older than the binary
    (root / "Cargo.lock").write_text("")
    os.utime(root / "Cargo.lock", (T0 + 300, T0 + 300))
    assert mod.stale_source(exe, root) is None


def test_start_command_serves_this_worktree(tmp_path):
    """The bridge starts with --static <repo>/static (this worktree's page, not the build worktree's)."""
    mod = _conftest()
    cmd = mod.start_command(tmp_path / "ds-bridge")
    assert cmd[0] == str(tmp_path / "ds-bridge")
    assert "--static" in cmd
    assert cmd[cmd.index("--static") + 1] == str(mod.ROOT / "static")
