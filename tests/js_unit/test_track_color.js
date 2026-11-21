/* js_unit: the colour of a track for drawing (#559). state.js holds trackColor and trackColors, so session.js, tracks.js, wave.js,
   velocity.js, review.js and app.js share one rule: a valid colour is kept, a missing one is the colour of the role, an unknown
   role is the colour of "other". A lane never builds "undefined0d". */
"use strict";

const assert = require("node:assert/strict");

exports.tests = {
  async "trackColor keeps a valid colour, gives the role colour for a missing one and the colour of other for an unknown role"(env) {
    env.load("state.js");
    env.get("S").tracks = {
      a: { role: "kick", color: "#123456" }, b: { role: "snare" }, c: { role: "tom", color: undefined },
      d: { role: "hat", color: "undefined" }, e: { role: "no-such-role" }, f: { color: "#12" },
    };
    const col = env.get("trackColor");
    assert.equal(col("a"), "#123456");
    assert.equal(col("b"), "#e0a13c");
    assert.equal(col("c"), "#57b8e0");
    assert.equal(col("d"), "#9be055");
    assert.equal(col("e"), "#8a93a6");
    assert.equal(col("f"), "#8a93a6");
    assert.equal(col("missing"), "#8a93a6");   // no such track: still a colour
    assert.equal(env.get("ROLE_COLORS.cymbal"), "#c9a0e0");
    assert.equal(env.get("ROLE_COLORS.kick"), "#e05555");
  },

  async "trackColors fills a missing colour once, marks it as derived and leaves a valid colour alone"(env) {
    env.load("state.js");
    const tracks = { a: { role: "kick", color: "#abcdef" }, b: { role: "cymbal" }, c: {} };
    env.get("trackColors")(tracks);
    assert.equal(tracks.a.color, "#abcdef");
    assert.equal(tracks.a.colorFromRole, undefined);
    assert.equal(tracks.b.color, "#c9a0e0");
    assert.equal(tracks.b.colorFromRole, true);
    assert.equal(tracks.c.color, "#8a93a6");
    env.get("trackColors")(null);   // nothing: no throw
  },
};
