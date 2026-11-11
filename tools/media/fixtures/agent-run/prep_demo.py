import json, os, subprocess, sys, time
A, BIN = sys.argv[1], sys.argv[2]
H = os.path.join(A, "home")
P = os.path.join(H, "projects")
env = dict(os.environ, DRUMSPIKE_HOME=H, DRUMSPIKE_PROJECTS=P)
p = subprocess.Popen([BIN, "cli", "batch"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, env=env)
def call(c, a, wait=None):
    m = {"command": c, "args": a}
    if wait is not None: m["wait"] = wait
    p.stdin.write(json.dumps(m) + "\n"); p.stdin.flush()
    while True:  # progress lines come first; the answer names the command
        d = json.loads(p.stdout.readline())
        assert isinstance(d, dict), type(d)
        if d.get("command") == c: break
    print(c, str(d)[:200]); return d
r = call("project_new", {"name": "Demo", "dir": P})
r = r.get("result", r); sid = r["sid"]
t = call("tracks_add", {"sid": sid, "paths": [f"{A}/stems/k in.wav", f"{A}/stems/s top.wav"], "roles": ["kick", "snare"]}, True)
a = call("analyze_start", {"sid": sid}, True)
for i in range(60):
    s = call("analyze_status", {"sid": sid}); s = s.get("result", s)
    if s.get("done") or s.get("status") == "done" or s.get("state") == "done": break
    time.sleep(2)
# One hit placed by hand before the run: Training Mode on, then a move through the edit path of the
# window (`events_edit` with by "person"), so the training log names it as the person's hit.
HAND_TRACK, HAND_INDEX, HAND_SHIFT_S = "k_in", 3, -0.0005
call("training_status", {"sid": sid, "on": True})
ev = call("events_get", {"sid": sid, "name": HAND_TRACK}); ev = ev.get("result", ev)
hits = ev["events"] if isinstance(ev, dict) else ev
t = hits[HAND_INDEX]["t"] + HAND_SHIFT_S
call("events_edit", {"sid": sid, "name": HAND_TRACK, "ops": [{"op": "move", "index": HAND_INDEX, "t": t}],
                     "training": True, "by": "person"})
print(call("project_save", {"sid": sid}))
print("folder", r["folder"])
p.stdin.close(); p.wait(); print("rc", p.returncode)
