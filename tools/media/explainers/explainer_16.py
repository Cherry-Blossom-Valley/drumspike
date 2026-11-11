"""#382 explainer 16, "Training Mode" (chapter 16). Script: docs/product/scripts/explainer-16-training-mode.md.

The picture uses the real demo session (tools/media/demo_session.py) for the work scene: the
s top close mic at sample zoom, the hit and its last line sample measured at render time the way
the agent's judge (static/agent.js) reads them, through explainer_15's readers. The log, export
and measure scenes draw real shapes: the record of the recorded tutorial's own move (part-agent
ra-move: k in 180008 -> 180006), Session F's real aggregates (522 seen, 124 moved, 314 confirmed)
and the field names of tools/training_export.py; the record's frame names the action in the
user's words (the log field is op). The rule numbers
are the script's "Facts used". Render through build.py.
"""
from explainer_common import (
    DIM, ORANGE, TEXT, UP, Arrow, Create, Explainer, FadeIn, FONT, Line, Rectangle, Text, VGroup,
)
from explainer_15 import pick_hits, samples_window

LANE_X0, LANE_X1 = -7.0, -1.0
LANE_Y = 1.9


def edit_rows():
    """The edits that write a record (one per row), the key of each after it (static/keymap.js)."""
    return (
        "Hits > Next Doubtful Hit  (D)",
        "Edit > Nudge Hit Earlier  (Left)",
        "Edit > Nudge Hit Later  (Right)",
        "Edit > Add Hit at Playhead  (H)",
        "Edit > Delete Hit",
        "Hits > Confirm Hit",
    )


class Explainer16(Explainer):
    script_ids = (
        "s01-why", "s02-on", "s03-work", "s04-log", "s05-export", "s06-measure", "s07-loop",
    )

    def s01_why(self):
        """The purpose first, in plain words: what the mode is for, and what it does not do."""
        names = [("your corrections", -5.1), ("the record", -1.7), ("the truth list", 1.7),
                 ("a better placement", 5.1)]
        boxes = VGroup(*[Rectangle(width=3.1, height=0.9, color=TEXT, stroke_width=1)
                         .move_to([x, 0.9, 0]) for _, x in names])
        labels = VGroup(*[Text(name, font=FONT, font_size=20, color=TEXT).move_to(boxes[i])
                          for i, (name, _) in enumerate(names)])
        self.t_play(Create(boxes), FadeIn(labels), t=2.5)
        self.say("Training Mode exists so that Drum Spike can learn to place hits the way you "
                 "do.", t=2.0)
        arrows = VGroup(*[Arrow([names[i][1] + 1.65, 0.9, 0], [names[i + 1][1] - 1.65, 0.9, 0],
                                color=ORANGE, stroke_width=3, max_tip_length_to_length_ratio=0.25)
                          for i in range(3)])
        self.t_play(Create(arrows), t=2.0)
        self.say("It writes down every hit you correct, with the sound around it, and those "
                 "corrections become the truth the placement is measured against.", t=2.0)
        off = Text("it never changes a hit on its own; off, the app is exactly as without it",
                   font=FONT, font_size=22, color=ORANGE).move_to([0, -0.9, 0])
        self.t_play(FadeIn(off), t=2.0)
        self.say("It never changes a hit on its own, and when it is off the app behaves "
                 "exactly as without it.", t=2.0)

    def s02_on(self):
        """The switch: the View menu with Training Mode checked, the status count beside it."""
        panel = Rectangle(width=3.4, height=1.1, color=TEXT, stroke_width=1).move_to([-4.4, 0.9, 0])
        tag = Text("the View menu", font=FONT, font_size=20, color=DIM).next_to(panel, UP, buff=0.25)
        row = Text("Training Mode", font=FONT, font_size=22, color=TEXT).move_to([-4.7, 0.9, 0])
        self.t_play(Create(panel), FadeIn(tag), FadeIn(row), t=2.0)
        self.say("Turn it on in the View menu: Training Mode.", t=2.0)
        # the menu's check, drawn as two strokes, so no glyph fallback is needed
        check = VGroup(
            Line([-3.5, 0.86, 0], [-3.32, 0.64, 0], color=ORANGE, stroke_width=3),
            Line([-3.32, 0.64, 0], [-3.0, 1.2, 0], color=ORANGE, stroke_width=3),
        )
        self.t_play(Create(check), t=1.5)
        status = Text("training: 0 records", font=FONT, font_size=24, color=TEXT).move_to([3.6, 0.9, 0])
        self.t_play(FadeIn(status), t=2.0)
        self.say("The switch belongs to the app, not to the project, and it survives a restart.",
                 t=2.0)
        self.say("While it is on, the status line counts the records of the open project.", t=2.0)

    def s03_work(self):
        """Work as usual: the review queue, the nudge at sample zoom, the edits that write a record."""
        t, s, hit_at, j = pick_hits()
        pic, hx = samples_window(s, hit_at, LANE_X0, LANE_X1, LANE_Y)
        tag = Text("s top, sample zoom", font=FONT, font_size=20, color=TEXT).next_to(pic, UP, buff=0.25)
        self.t_play(Create(pic), FadeIn(tag), t=2.5)
        self.say("Then work as you always do.", t=2.0)
        rows = VGroup(*[Text(ln, font=FONT, font_size=20, color=TEXT).move_to([3.7, 2.3 - 0.78 * i, 0])
                        for i, ln in enumerate(edit_rows())])
        self.t_play(FadeIn(rows), t=2.0)
        self.say("The review queue walks you to the doubtful hits; at sample zoom you nudge a "
                 "hit by whole samples onto its last line sample.", t=2.0)
        if j["suggest"]:
            last_x = LANE_X0 + (LANE_X1 - LANE_X0) * (hit_at + j["suggest"]) / (len(s) - 1)
            nudge = Arrow([hx, LANE_Y + 0.25, 0], [last_x, LANE_Y + 0.25, 0], color=ORANGE,
                          stroke_width=3, max_tip_length_to_length_ratio=0.25)
            nudge_tag = Text(f"the nudge: {j['suggest']} samples", font=FONT, font_size=20,
                             color=ORANGE).move_to([(hx + last_x) / 2, LANE_Y - 1.3, 0])
            self.t_play(Create(nudge), FadeIn(nudge_tag), t=2.0)
        else:
            nudge_tag = Text("this hit is already on the last line sample", font=FONT,
                             font_size=20, color=ORANGE).move_to([LANE_X0 / 2, LANE_Y - 1.3, 0])
            self.t_play(FadeIn(nudge_tag), t=2.0)
        self.say("Add a hit, delete one, confirm one as reviewed: every one of these writes one "
                 "record.", t=2.0)
        self.say("An agent writes the same kind of record; it says person or agent.", t=2.0)

    def s04_log(self):
        """The log: the file, and one record of it (the recorded tutorial's own move)."""
        lines = [
            "action: move   track: k_in   by: \"person\"",
            "from: 180008   to: 180006   precision: \"sample\"",
            "audio: 40 ms before, 10 ms after",
        ]
        panel = Rectangle(width=10.6, height=3.0, color=DIM, stroke_width=1).move_to([0, 0.5, 0])
        title = Text("<home>/training/<project id>.jsonl, one line per affected hit", font=FONT,
                     font_size=22, color=TEXT).to_edge(UP, buff=0.7)
        self.t_play(Create(panel), FadeIn(title), t=2.0)
        self.say("One file per project, in the app's data folder, named by the project's id: a "
                 "rename keeps the file.", t=2.0)
        text = VGroup(*[Text(ln, font=FONT, font_size=22, color=TEXT).move_to([0, 1.05 - 0.55 * i, 0])
                        for i, ln in enumerate(lines)])
        self.t_play(FadeIn(text), t=2.5)
        self.say("A record says what you did, the old and the new position, and the sound "
                 "around the hit.", t=2.0)

    def s05_export(self):
        """The export: one script from the source checkout, the printout and the truth list."""
        lines = [
            "$ uv run python tools/training_export.py <training log>.jsonl",
            "move: 124 records   confirm: 314 records   per role and op",
            "truth list: 438 entries -> training_truth.json",
            "only your own records (--by person)",
        ]
        panel = Rectangle(width=11.2, height=2.2, color=DIM, stroke_width=1).move_to([0, 1.1, 0])
        text = VGroup(*[Text(ln, font=FONT, font_size=20, color=TEXT).move_to([0, 1.85 - 0.5 * i, 0])
                        for i, ln in enumerate(lines)])
        self.t_play(Create(panel), FadeIn(text), t=2.5)
        self.say("From the source checkout, one script turns the logs into the truth list.",
                 t=2.0)
        note = Text("the printout names no project and no path", font=FONT, font_size=22,
                    color=ORANGE).move_to([0, -0.9, 0])
        self.t_play(FadeIn(note), t=2.0)
        self.say("The printout names no project and no path, so it is safe to paste; the truth "
                 "file carries track names, so keep it with the logs.", t=2.0)

    def s06_measure(self):
        """What the truth list does: the new placement version scored against your rows."""
        row = Text("k_in   180008 -> 180006   move   precision: sample", font=FONT, font_size=22,
                   color=TEXT).move_to([0, 2.2, 0])
        self.t_play(FadeIn(row), t=2.0)
        self.say("The truth list is the yardstick: when the placement rule changes, the new "
                 "version is scored against your rows, next to the old one.", t=2.0)
        names = [("the version before", -3.2), ("the version after", 3.2)]
        boxes = VGroup(*[Rectangle(width=4.4, height=1.0, color=TEXT, stroke_width=1)
                         .move_to([x, 0.3, 0]) for _, x in names])
        labels = VGroup(*[Text(name, font=FONT, font_size=22, color=TEXT).move_to(boxes[i])
                          for i, (name, _) in enumerate(names)])
        arrs = VGroup(*[Arrow([0, 1.7, 0], [x, 0.95, 0], color=ORANGE, stroke_width=3,
                              max_tip_length_to_length_ratio=0.25) for _, x in names])
        self.t_play(Create(boxes), FadeIn(labels), Create(arrs), t=2.0)
        note = Text("a clear stroke weighs more than a wobbly one", font=FONT, font_size=22,
                    color=ORANGE).move_to([0, -1.3, 0])
        self.t_play(FadeIn(note), t=2.0)
        self.say("A clear stroke weighs more than a wobbly one: every record carries the line "
                 "measures.", t=2.0)
        self.say("The next version must place your hits as you did.", t=2.0)

    def s07_loop(self):
        """The loop back, and what stays home: the records leave only when you send them."""
        names = [("your edits", -5.25), ("the log", -1.75), ("the truth list", 1.75),
                 ("the next release", 5.25)]
        boxes = VGroup(*[Rectangle(width=3.1, height=0.9, color=TEXT, stroke_width=1)
                         .move_to([x, 0.9, 0]) for _, x in names])
        labels = VGroup(*[Text(name, font=FONT, font_size=20, color=TEXT).move_to(boxes[i])
                          for i, (name, _) in enumerate(names)])
        self.t_play(Create(boxes), FadeIn(labels), t=2.5)
        self.say("That closes the loop: the hits you fix are what the next placement rule gets "
                 "measured against.", t=2.0)
        arrows = VGroup(*[Arrow([names[i][1] + 1.65, 0.9, 0], [names[i + 1][1] - 1.65, 0.9, 0],
                                color=ORANGE, stroke_width=3, max_tip_length_to_length_ratio=0.25)
                          for i in range(3)])
        back = Arrow([names[3][1] - 0.3, 0.25, 0], [names[0][1] + 0.3, 0.25, 0], color=TEXT,
                     stroke_width=2, max_tip_length_to_length_ratio=0.1)
        self.t_play(Create(arrows), Create(back), t=2.0)
        note = Text("the records stay on your machine until you send them", font=FONT,
                    font_size=22, color=ORANGE).move_to([0, -1.1, 0])
        self.t_play(FadeIn(note), t=2.0)
        self.say("Nothing leaves your machine by itself; the log stays in your data folder, and "
                 "you decide what to send.", t=2.0)
