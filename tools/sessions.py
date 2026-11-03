"""The session tables of the real-session yardstick (`tools/fix_metric.py`) and the snare diagnosis
(`tools/snare_onset.py`).

`NEW_SESSIONS`: the P4-14 stage 1b sessions (moved from `tools/tom_bleed_new.py`): per session the folder under
`~/.cache/drumspike/samples`, the mic table (`drum_mics`: name -> (stem, role, label)) and the tom references (`toms_ref`).
The sessions of P4-6b are `tools/rescue_real.py` `SESSIONS`; `fix_metric.songs()` joins both.

`KICK_FILES` and `SNARE_FILES`: the reference files of the same kick and snare strokes of each yardstick song (moved from
`tools/kick_onset.py` and `tools/snare_onset.py`). `fix_metric.kick_files()` and `snare_files()` read them.
"""

TOMS4 = ("t1", "t2", "t3", "t4")


def session(dir_, mics, toms_ref):
    return {"dir": dir_, "excluded": set(), "mics": mics, "toms_ref": toms_ref}


def drum_mics(k, s, t, hh, oh):
    """The mic table of a session: kick, snare, the four toms, the hat and one or two overheads."""
    m = {"k in": (k, "kick", "k in"), "s top": (s, "snare", "s top")}
    m.update({name: (stem, "tom", name) for name, stem in zip(TOMS4, t)})
    m["hh"] = (hh, "hat", "hh")
    for i, stem in enumerate(oh):
        m[f"oh{i + 1}" if len(oh) > 1 else "oh"] = (stem, "cymbal", "oh")
    return m


NEW_SESSIONS = {
    "Session H": session("Session H", drum_mics("k in_01", "s top_01", ["t 1_01", "t 2_01", "t 3_01", "t 4_01"],
                                                  "hh_04", ["oh_03.L", "oh_03.R"]),
                       {"combined": ["t samp_01.L", "t samp_01.R"]}),
    "Session F": session("Session F",
                                drum_mics("k in_01", "s top_01", ["t 1_01", "t 2_01", "t 3_01", "t 4_01"], "hh_01",
                                          ["oh_01.L", "oh_01.R"]),
                                {"per_tom": {t: [f"{t} samp_01.L", f"{t} samp_01.R"] for t in TOMS4}}),
    "Session G": session("Session G",
                                        drum_mics("k in_01", "s top_01", ["t1HH_01", "t2_01", "t3_01", "t4Ri_01"],
                                                  "hh_10", ["oh_01.L", "oh_01.R"]),
                                        {"combined": ["t trigger_01.L", "t trigger_01.R"]}),
    "Session E": session("Session E",
                                drum_mics("k in-AtA2", "s top-AtA2", ["t1 hh-AtA2", "t2-AtA2", "t3-AtA2", "t4 ri-AtA2"],
                                          "hh-AtA2", ["oh-AtA2.L", "oh-AtA2.R"]),
                                {"combined": ["t sample sd3_03.L", "t sample sd3_03.R"]}),
    "Session D": session("Session D",
                              drum_mics("k in", "s top", ["t1 hh", "t2", "t3", "t4 ri"], "hh", ["oh HH", "oh Ri"]),
                              {"combined": ["t trigger.L", "t trigger.R"]}),
}

# the reference files of each song: `yard` is what the yardstick's `refs.json` uses (the union of its onsets within 20 ms,
# the earliest of a cluster), `files` all the files of the same strokes (a trigger track and sample layers)
KICK_FILES = {
    "Session A": {"yard": ["k sampl trig_01"], "files": ["k sampl trig_01", "k sampl savior_01", "k sampl sd3_01"]},
    "Session B": {"yard": ["k trigger_01"], "files": ["k trigger_01", "k samp savior_01", "k samp sd3_01"]},
    "Session C": {"yard": ["k samp 1_02"], "files": ["k samp 1_02", "k samp 2_02"]},
    "Session H": {"yard": ["k samp 1_01", "k samp 2_01", "k samp 3_01"], "files": ["k samp 1_01", "k samp 2_01", "k samp 3_01"]},
    "Session F": {"yard": ["k samp 1_01", "k samp 2_01", "k samp 3_01"],
                         "files": ["k samp 1_01", "k samp 2_01", "k samp 3_01"]},
    "Session G": {"yard": ["k triggerr_03"], "files": ["k triggerr_03", "k samp savior_03", "k samp sd3_03"]},
    "Session D": {"yard": ["k trigger"], "files": ["k trigger", "k samp saviour", "k samp sd3"]},
}

# the reference files of each song: `yard` is what the yardstick's `refs.json` uses (the union of the onsets of all of them within
# 20 ms, or one file), `files` the files of the same strokes of the session (a trigger track and sample layers), `extra`
# a file that is another sample (the hat leaves it out; the pair table lists it)
SNARE_FILES = {
    "Session A": {"yard": ["s sampl trig_01"], "files": ["s sampl trig_01", "s sampl savior_01", "s sampl sd3_01"]},
    "Session B": {"yard": ["s trigger_01"], "files": ["s trigger_01", "s samp savior_01", "s samp sd3_01"]},
    "Session C": {"yard": ["s samp 1_02"], "files": ["s samp 1_02", "s samp 2_02"], "extra": ["s samp rev_02"]},
    "Session H": {"yard": ["s samp 1_01", "s samp 2_01", "s samp 3_01"], "files": ["s samp 1_01", "s samp 2_01", "s samp 3_01"]},
    "Session F": {"yard": ["s samp 1_01", "s samp 2_01", "s samp 3_01"], "files": ["s samp 1_01", "s samp 2_01", "s samp 3_01"]},
    "Session G": {"yard": ["s trigger_03"], "files": ["s trigger_03", "s samp savior_03", "s samp sd3_03"]},
    "Session D": {"yard": ["s trigger"], "files": ["s trigger", "s samp saviour", "s samp sd3"]},
}
