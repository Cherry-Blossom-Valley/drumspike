"""#373 explainer 15, "An agent fine-tunes the song" (chapter 15). Script: docs/product/scripts/explainer-15-agent.md.

The picture uses the real demo session (tools/media/demo_session.py): the WAV files and
truth.json are read from ~/.cache/drumspike/media/demo/ (generated on demand with the default
seed). The s top close mic carries the sample-zoom scenes; the hit, its lead line, its spread
and its departure are measured from the session at render time, like the agent's own judge
(static/agent.js) reads them. The rule numbers are the script's "Facts used". Render through
build.py.
"""
import numpy as np

from explainer_common import (
    CAPTION_SIZE, COBALT, DIM, DOWN, ORANGE, TEXT, UP, Arrow, Create, Dot, Explainer, FadeIn,
    FadeOut, FONT, Line, Rectangle, Text, VGroup, wave,
)
from explainer_10 import PLOT_W, SR, kit_hits, read_seg, truth  # the demo session's readers

# The agent's own window and rule (static/agent.js): 0.9 ms before and 0.6 ms after the hit,
# the line judged over the first 12 samples with a spread limit of 0.03 (shares of the local
# peak), the departure looked for up to 12 samples before and 6 after the hit.
WINDOW_MS = (0.9, 0.6)
LINE_LEAD = 12
LINE_MAX_SPREAD = 0.03
DEPART_BEFORE, DEPART_AFTER = 12, 6
DEPART_MIN = 0.03
HIT_Y = 1.2


def window_samples(mic, t_hit, n):
    """The hit's window as the agent sees it: n samples (0.9 ms before, 0.6 ms after = 72),
    normalized to the window's own local peak, and the index of the hit's own sample in it."""
    x = read_seg(mic, t_hit - int(WINDOW_MS[0] / 1000 * SR), n)
    peak = float(np.max(np.abs(x))) or 1.0
    shares = [float(v) / peak for v in x]
    return shares, int(WINDOW_MS[0] / 1000 * SR)


def judge(s, hit_at):
    """The agent's judge (static/agent.js `judge`): the lead line's level and spread, the
    departure, the suggestion (last line sample before the first motion) and the why."""
    n = min(LINE_LEAD, len(s))
    lead = sorted(s[:n])
    level = lead[n >> 1]
    spread = max(abs(s[i] - level) for i in range(n))
    out = {"level": level, "spread": spread, "first_off": None, "suggest": None, "why": None}
    if not spread <= LINE_MAX_SPREAD:
        out["why"] = "the lead is not a line"
        return out
    thr = max(3 * spread, DEPART_MIN)
    for i in range(n, len(s)):
        if abs(s[i] - level) > thr:
            out["first_off"] = i - hit_at
            if out["first_off"] < -DEPART_BEFORE or out["first_off"] > DEPART_AFTER:
                out["why"] = "departure far from the hit"
            else:
                out["suggest"] = i - 1 - hit_at
            break
    return out


def kit_snares():
    """The kit check's snare hits (one per drum, 0.625 s apart)."""
    return [t for drum, t in sorted(kit_hits().items(), key=lambda kv: kv[1]) if drum == "snare"]


def measure_stored(t_truth):
    """The hit's stored sample as the analysis leaves it: the last line sample before the
    stroke's first motion in the mic itself (#351), i.e. one sample before the measured
    departure (the truth hit time is the stroke in air; the mic hears it a few samples later)."""
    s, _ = window_samples("s top", t_truth, 72)
    n = min(LINE_LEAD, len(s))
    level = sorted(s[:n])[n >> 1]
    spread = max(abs(s[i] - level) for i in range(n))
    if not spread <= LINE_MAX_SPREAD:
        return None
    thr = max(3 * spread, DEPART_MIN)
    for i in range(n, len(s)):
        if abs(s[i] - level) > thr:
            return t_truth + i - 1 - int(WINDOW_MS[0] / 1000 * SR)
    return None


def pick_hits():
    """One hit already on its last line sample (the confirm story) and one wobbly hit (the
    doubt story), measured over the snare hits of the demo session."""
    clean = wobbly = None
    for h in truth()["hits"]:
        if h["drum"] != "snare":
            continue
        stored = measure_stored(h["t"])
        if stored is None:
            if wobbly is None:
                wobbly = h["t"]
            continue
        if clean is None:
            clean = stored
        if wobbly is not None and clean is not None:
            break
    if clean is None or wobbly is None:
        raise SystemExit("the demo session has no clean and no wobbly snare for the scenes")
    out = []
    for t in (clean, wobbly):
        s, hit_at = window_samples("s top", t, 72)
        out.append((t, s, hit_at, judge(s, hit_at)))
    return out[0], out[1]


def samples_window(s, hit_at, x0, x1, yc, half=0.55):
    """The window drawn as a wave (one value per sample), with the hit's own sample at its place."""
    return wave(s, x0, x1, yc, half, COBALT), x0 + (x1 - x0) * hit_at / max(1, len(s) - 1)


def sample_stalk(x, yc, color=ORANGE, half=0.75, width=3):
    return Line([x, yc - half, 0], [x, yc + half, 0], color=color, stroke_width=width)


def number_lane(s, hit_at, j, yc, tag):
    """The window as the agent sees it: one column per sample, the hit's sample marked, the
    judge's level line and the first departure labelled."""
    half = 1.0
    col_w = PLOT_W / len(s)
    cols = VGroup(*[Rectangle(width=col_w * 0.7, height=max(0.04, abs(v) * half),
                              color=COBALT if abs(v) > j["level"] + max(3 * j["spread"], DEPART_MIN)
                              else DIM, stroke_width=0, fill_color=COBALT if abs(v) > j["level"] + max(3 * j["spread"], DEPART_MIN) else DIM,
                              fill_opacity=0.85).move_to([-PLOT_W / 2 + col_w * (i + 0.5), yc - half / 2 + abs(v) * half / 2, 0])
                    for i, v in enumerate(s)])
    hx = -PLOT_W / 2 + PLOT_W * hit_at / (len(s) - 1)
    hit_mark = sample_stalk(hx, yc, ORANGE, half=half + 0.25)
    level = Line([-PLOT_W / 2, yc + j["level"] * half, 0], [-PLOT_W / 2 + PLOT_W * (LINE_LEAD + 0.5), yc + j["level"] * half, 0],
                 color=TEXT, stroke_width=2)
    parts = VGroup(cols, hit_mark, level, Text(tag, font=FONT, font_size=22, color=TEXT))
    parts[-1].to_edge(UP, buff=0.7)
    if j["first_off"] is not None:
        dx = -PLOT_W / 2 + PLOT_W * (hit_at + j["first_off"]) / (len(s) - 1)
        parts.add(sample_stalk(dx, yc, ORANGE, half=half + 0.25, width=2))
    return parts


class Explainer15(Explainer):
    script_ids = (
        "s01-hand", "s02-agent", "s03-look", "s04-judge", "s05-training", "s06-loop", "s07-result",
    )

    def s01_hand(self):
        """What the owner does by hand: sample zoom, the last line sample, the nudge, Confirm Hit."""
        clean, _ = pick_hits()
        t, s, hit_at, j = clean
        pic, hx = samples_window(s, hit_at, -PLOT_W / 2, PLOT_W / 2, HIT_Y)
        label = Text("s top, sample zoom (0.9 ms before, 0.6 ms after the hit)", font=FONT,
                     font_size=22, color=TEXT).to_edge(UP, buff=0.7)
        self.t_play(Create(pic), FadeIn(label), t=3.0)
        self.say("Zoom to a hit until one sample is a pixel: a flat line before the stroke, "
                 "then the stroke's first motion.", t=2.0)
        # the hit's own sample, and the last line sample the rule wants it on
        hit_dot = Dot([hx, HIT_Y, 0], radius=0.09, color=ORANGE)
        last_x = -PLOT_W / 2 + PLOT_W * (hit_at + j["suggest"]) / (len(s) - 1)
        last_mark = sample_stalk(last_x, HIT_Y, TEXT, half=0.9, width=2)
        last_tag = Text("the last sample of the line", font=FONT, font_size=22, color=TEXT)
        last_tag.next_to([last_x, HIT_Y - 1.4, 0], DOWN, buff=0.3)
        self.t_play(FadeIn(hit_dot), Create(last_mark), FadeIn(last_tag), t=2.5)
        self.say("The hit belongs on the last sample of that line.", t=2.0)
        if j["suggest"]:
            nudge = Arrow([hx, HIT_Y + 0.2, 0], [last_x, HIT_Y + 0.2, 0], color=ORANGE,
                          stroke_width=3, max_tip_length_to_length_ratio=0.25)
            nudge_tag = Text(f"the nudge: {j['suggest']} samples", font=FONT, font_size=22,
                             color=ORANGE)
            nudge_tag.move_to([(hx + last_x) / 2, HIT_Y + 0.75, 0])
            self.t_play(Create(nudge), FadeIn(nudge_tag), t=2.0)
        else:
            nudge_tag = Text("this hit is already on the last line sample", font=FONT,
                             font_size=22, color=ORANGE).move_to([0.0, HIT_Y + 0.75, 0])
            self.t_play(FadeIn(nudge_tag), t=2.0)
        self.say("The nudge moves it there by whole samples.", t=2.0)
        confirm = Text("Hits > Confirm Hit: reviewed, right where it is", font=FONT,
                       font_size=CAPTION_SIZE, color=TEXT).to_edge(DOWN, buff=1.2)
        self.t_play(FadeIn(confirm), t=2.0)
        self.say("Hits Confirm Hit marks it reviewed.", t=2.0)

    def s02_agent(self):
        """The agent does the same three steps through the app's own page and the same code paths."""
        clean, _ = pick_hits()
        t, s, hit_at, j = clean
        pic, hx = samples_window(s, hit_at, -PLOT_W / 2, PLOT_W / 2, HIT_Y)
        self.t_play(Create(pic), t=2.0)
        steps = VGroup(*[Text(f"DSAgent.{name}", font=FONT, font_size=24, color=TEXT)
                         .move_to([0, 2.6 - 0.75 * i, 0])
                         for i, name in enumerate(("look", "move", "confirm"))])
        self.t_play(FadeIn(steps[0]), t=1.5)
        self.say("An agent does the same three steps through the app's own page: look, move, confirm.",
                 t=2.0)
        self.t_play(FadeIn(steps[1]), FadeIn(steps[2]), t=1.5)
        same = Text("the same code paths as your keyboard: nudgeSelected, Hits > Confirm Hit",
                    font=FONT, font_size=22, color=TEXT).to_edge(DOWN, buff=1.2)
        self.t_play(FadeIn(same), t=2.5)
        self.say("Every move and confirm runs through the code your keyboard uses.", t=2.0)
        eq = Text("the record of an agent is byte for byte the record of a person", font=FONT,
                  font_size=22, color=ORANGE).to_edge(DOWN, buff=1.2)
        # #459: one line at a time at the bottom edge — split the fade so both are never visible.
        self.t_play(FadeOut(same), t=1.0)
        self.t_play(FadeIn(eq), t=1.0)
        self.say("So the record of an agent is byte for byte the record of a person.", t=2.0)

    def s03_look(self):
        """What the agent sees: the lane picture and the samples as numbers, with the judge's marks."""
        clean, _ = pick_hits()
        t, s, hit_at, j = clean
        pic, _ = samples_window(s, hit_at, -7.0, -1.0, 1.4)
        tag = Text("the lane picture", font=FONT, font_size=20, color=TEXT).next_to(pic, UP, buff=0.3)
        self.t_play(Create(pic), FadeIn(tag), t=2.5)
        # the same window as numbers, on the right: shares of the local peak, hit_at marked
        nums, _ = samples_window(s, hit_at, 1.2, 6.8, 1.4, half=0.55)
        ntag = Text("the same samples as numbers", font=FONT, font_size=20, color=TEXT).next_to(nums, UP, buff=0.3)
        hx = 1.2 + (6.8 - 1.2) * hit_at / (len(s) - 1)
        hit_mark = sample_stalk(hx, 1.4, ORANGE, half=0.8)
        hit_tag = Text("hit_at", font=FONT, font_size=20, color=ORANGE).next_to([hx, 0.55, 0], DOWN, buff=0.25)
        self.t_play(Create(nums), FadeIn(ntag), Create(hit_mark), FadeIn(hit_tag), t=2.5)
        self.say("The window is 0.9 milliseconds before and 0.6 after the hit; the samples are "
                 "shares of the local peak, and hit_at points at the hit's own sample.", t=2.0)
        info = Text(f"the judge: level {j['level']:.2f}, spread {j['spread']:.3f}, "
                    f"first departure {j['first_off']:+d} samples", font=FONT, font_size=22,
                    color=TEXT).to_edge(DOWN, buff=1.2)
        self.t_play(FadeIn(info), t=2.5)
        self.say("From these the judge reads the line's level and its spread, and where the "
                 "stroke departs.", t=2.0)

    def s04_judge(self):
        """Where the judgement stops: a wobbly lead gets no suggestion, the hit stays for the person."""
        _, wobbly = pick_hits()
        t, s, hit_at, j = wobbly
        parts = number_lane(s, hit_at, j, HIT_Y,
                            f"a wobbly lead: the spread {j['spread']:.3f} is over the 0.03 limit")
        self.t_play(Create(parts[0]), FadeIn(parts[-1]), t=3.0)
        self.say("The judgement stops where the line is not a line.", t=2.0)
        rule = Text("no suggestion: the spread over the first 12 samples is judged against "
                    "0.03 of the local peak", font=FONT, font_size=22, color=TEXT).to_edge(DOWN, buff=1.2)
        self.t_play(FadeIn(rule), t=2.5)
        self.say("A wobbly lead, a spread over 0.03 of the local peak over the first 12 samples, "
                 "gets no suggestion.", t=2.0)
        doubt = Text("doubt: the hit lands on the doubtful list with its reason, and stays for you",
                     font=FONT, font_size=22, color=ORANGE).to_edge(DOWN, buff=1.2)
        # #459: split the fade so the two bottom lines are never visible at once.
        self.t_play(FadeOut(rule), t=1.0)
        self.t_play(FadeIn(doubt), t=1.0)
        self.say("The hit lands on the doubtful list with its reason, and stays for you.", t=2.0)

    def s05_training(self):
        """The training log line of the move: op, from, to, precision, placement, audio window."""
        clean, _ = pick_hits()
        t, s, hit_at, j = clean
        lines = [
            "op: move   track: s_top",
            f"from: {t}   to: {t + j['suggest']}   precision: \"sample\"",
            "placement: edge, line_noise,",
            "            line_slope, onset_spread",
            "audio: 40 ms before, 10 ms after",
        ]
        panel = Rectangle(width=9.6, height=2.6, color=DIM, stroke_width=1).move_to([0, 0.6, 0])
        text = VGroup(*[Text(ln, font=FONT, font_size=22, color=TEXT).move_to([0, 1.5 - 0.5 * i, 0])
                        for i, ln in enumerate(lines)])
        title = Text("<home>/training/<project id>.jsonl, one line per affected hit", font=FONT,
                     font_size=22, color=TEXT).to_edge(UP, buff=0.7)
        self.t_play(Create(panel), FadeIn(title), t=2.0)
        self.say("With Training Mode on, every move and confirm writes one line to the project's "
                 "training log.", t=2.0)
        self.t_play(FadeIn(text), t=2.5)
        self.say("The op, the position, the precision, and the placement measures of the hit.",
                 t=2.0)
        self.say("The record carries the samples around it, forty milliseconds before and ten "
                 "after, so it can be read without the session.", t=2.0)

    def s06_loop(self):
        """The loop: the agent's and the person's records in one log, the truth list out of it."""
        boxes = [("the agent's moves", -4.4), ("your corrections", 0.0), ("the truth list", 4.4)]
        bxs = VGroup(*[Rectangle(width=3.2, height=1.0, color=TEXT, stroke_width=1)
                       .move_to([x, 0.8, 0]) for _, x in boxes])
        labels = VGroup(*[Text(name, font=FONT, font_size=22, color=TEXT).move_to(bxs[i])
                          for i, (name, _) in enumerate(boxes)])
        self.t_play(Create(bxs), FadeIn(labels), t=2.5)
        self.say("That log is the loop: the agent's records and your corrections land in the "
                 "same file.", t=2.0)
        arr1 = Arrow([bxs[1].get_center()[0] - 1.6, 0.8, 0], [bxs[0].get_center()[0] + 1.7, 0.8, 0],
                     color=ORANGE, stroke_width=3, max_tip_length_to_length_ratio=0.2)
        arr2 = Arrow([bxs[1].get_center()[0] + 1.6, 0.8, 0], [bxs[2].get_center()[0] - 1.7, 0.8, 0],
                     color=ORANGE, stroke_width=3, max_tip_length_to_length_ratio=0.2)
        self.t_play(Create(arr1), Create(arr2), t=2.0)
        export = Text("tools/training_export.py prints the truth list the placement is measured "
                      "against", font=FONT, font_size=22, color=TEXT).to_edge(DOWN, buff=1.2)
        self.t_play(FadeIn(export), t=2.5)
        self.say("The export turns them into the truth list the placement is measured against.",
                 t=2.0)
        self.say("The more you correct, the better the measures say what a right hit looks like.",
                 t=2.0)

    def s07_result(self):
        """Counts only; you stay the judge; the doubtful wait in the review queue."""
        counts = VGroup(*[Text(ln, font=FONT, font_size=26, color=TEXT).move_to([0, 1.8 - 0.7 * i, 0])
                          for i, ln in enumerate(["saw: 412", "moved: 39", "confirmed: 121", "left for you: 14"])])
        self.t_play(FadeIn(counts), t=2.5)
        self.say("The agent reports counts only: how many hits it saw, moved, confirmed and "
                 "left for you.", t=2.0)
        judge = Text("you stay the judge: every move is a normal edit you can undo", font=FONT,
                     font_size=22, color=TEXT).to_edge(DOWN, buff=1.2)
        self.t_play(FadeIn(judge), t=2.0)
        self.say("You stay the judge: every move is a normal edit you can undo.", t=2.0)
        queue = Text("Hits > Next Doubtful Hit: the doubtful wait in the review queue", font=FONT,
                     font_size=22, color=ORANGE).to_edge(DOWN, buff=1.2)
        # #459: split the fade so the two bottom lines are never visible at once.
        self.t_play(FadeOut(judge), t=1.0)
        self.t_play(FadeIn(queue), t=1.0)
        self.say("And the doubtful hits wait in the review queue.", t=2.0)
