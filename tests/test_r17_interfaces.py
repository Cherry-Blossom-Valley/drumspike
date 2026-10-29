"""R17 step 7 (#55): the interface check the ticket's Gate names — no module reaches
into another module's state except through the plan's interfaces (this check prints 0).

A module's state is its top-level let/var bindings (saveTimers, _editSeq, _gen, ...);
everything else it exposes (S, PH, the functions, the hook objects) is the plan's public
surface (docs/review/R17-plan.md: "the global surface the tests and shell.js use stays on
window"). This check counts bare references from any static/ page script to another
module's mutable bindings — `canvas._gen` and friends (property accesses) are not state
reaches. The step-7 commits routed every reach through persist.js's hasPendingWrite /
pendingWriteTracks / dropQueuedWrite / editSeq and session.js's loadGen.
"""
import os
import re

HERE = os.path.dirname(os.path.abspath(__file__))
STATIC = os.path.join(HERE, "..", "static")

MODULES = ["state.js", "persist.js", "edit.js", "transport.js", "render.js", "session.js"]
LET_VAR = re.compile(r"^(?:let|var) (\w+)", re.M)


def _violations():
    scripts = sorted(f for f in os.listdir(STATIC) if f.endswith(".js"))
    text = {f: open(os.path.join(STATIC, f)).read() for f in scripts}
    owned = {m: set(LET_VAR.findall(text[m])) for m in MODULES}
    found = []
    for m in MODULES:
        for name in sorted(owned[m]):
            for other in scripts:
                if other == m:
                    continue
                for i, line in enumerate(text[other].split("\n"), 1):
                    if re.search(rf"(?<![\w.]){name}\b", line) and not line.lstrip().startswith("//"):
                        found.append(f"{other}:{i} -> {m}.{name}: {line.strip()[:100]}")
    return found


def test_no_module_reaches_into_another_modules_state():
    found = _violations()
    assert not found, (
        f"{len(found)} bare state references across module borders "
        f"(route them through the owning module's interface):\n" + "\n".join(found)
    )
