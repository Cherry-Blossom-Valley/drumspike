/* WebGL2 waveform lanes (P3-6a).
   One WebGL2 canvas (#lanesGL, created by app.js) sits under the lane list and draws every visible wave
   lane at its place with ONE draw call per lane (instanced quads, one instance per pixel column). The
   2D wave canvases above it are transparent and keep the event lines, the selection and the hit markers;
   the ruler, the overview, the spike lanes and the playhead stay Canvas 2D. The GL canvas is composited
   by the browser: no copy into a 2D canvas (that copy cost 9 ms per frame in WKWebView, measured).
   Data: the peak tiles (min, max, RMS) as an RGBA32F texture per track and level; below the sample
   level (view of at most 262144 samples) the PCM window of the track as an R32F texture.
   The look is the one of drawWave in app.js: translucent min/max envelope (colour + 0x55), solid RMS
   body, centre line, and in the sample window the line (and dots when zoomed in far).
   Without WebGL2 `create` returns null and app.js keeps its Canvas 2D path.
   P3-6b: the engine can run in a worker (tiles-worker.js, OffscreenCanvas): `remote` is the page side, `host`
   the worker side. The page then holds no peak arrays and no sample windows: tiles arrive as raw bytes
   (`addTiles`, uploaded and dropped), sample windows as transferred buffers (`setWindow`, a small LRU of
   textures). */
(function () {
  "use strict";
  const TW = 4096;                       // texture width in texels; index i is at (i % TW, i / TW)
  const BG = [0x10 / 255, 0x13 / 255, 0x18 / 255];   // #101318, the lane background of drawWave
  const PAGE = [0x14 / 255, 0x16 / 255, 0x1a / 255]; // --bg of style.css: a muted lane fades towards it

  const VS = `#version 300 es
precision highp float; precision highp int; precision highp sampler2D;
uniform sampler2D u_tex;
uniform vec2 u_atlas;      // canvas size in device pixels
uniform vec4 u_lane;       // lane width and height in CSS px, device scale x and y
uniform vec2 u_org;        // top left of the lane in device pixels
uniform float u_scale, u_mid;
uniform int u_mode;        // 0 peak tiles, 1 sample columns, 2 sample line
uniform int u_cols, u_cl;  // columns; index of the centre-line instance
uniform float u_bin0, u_binPerCol;   // mode 0: first bin position of column 0, bins per column
uniform int u_n, u_hasRms;
uniform float u_pos0, u_posPerCol;   // mode 1: first sample position (texture index) of column 0, samples per column
uniform int u_cnt;                   // mode 1: samples in the texture window used
uniform int u_base;                  // mode 2: texture index of the first drawn sample
uniform float u_ppx;                 // mode 2: pixels per sample
uniform float u_x0;                  // mode 2: x of texture sample 0 in CSS px
uniform int u_nseg;
uniform vec3 u_color;
uniform vec3 u_page;
uniform float u_fill;      // P4-26: 1 = the min/max shape solid (Pro Tools look, spans of 500 ms and more), 0 = translucent with the RMS body
uniform float u_fade;      // 1, or the opacity of a muted lane (mixed towards the page colour)
out vec4 v_color;
out vec2 v_uv;
out float v_dot;
vec4 tex(int i) { return texelFetch(u_tex, ivec2(i % ${TW}, i / ${TW}), 0); }
vec2 clip(vec2 p) {
  vec2 d = u_org + vec2(p.x * u_lane.z, p.y * u_lane.w);
  return vec2(d.x / u_atlas.x * 2.0 - 1.0, 1.0 - d.y / u_atlas.y * 2.0);
}
void main() {
  int inst = gl_InstanceID;
  vec2 c = vec2(float(gl_VertexID & 1), float(gl_VertexID >> 1));
  vec4 rect = vec4(0.0);         // x, y, w, h in CSS px
  vec4 col = vec4(0.0);
  float dot = 0.0;
  vec2 uv = vec2(0.0);
  vec2 pos;
  if (inst == u_cl) {
    rect = vec4(0.0, u_mid - 0.5, u_lane.x, 1.0);
    col = vec4(1.0, 1.0, 1.0, u_mode == 0 ? 0.12 : 0.15);
    pos = rect.xy + c * rect.zw;
  } else if (u_mode == 0) {
    int cc = inst >> 1;
    bool rms = (inst & 1) == 1;
    int b0 = max(0, int(floor(u_bin0 + float(cc) * u_binPerCol)));
    int b1 = min(u_n, int(ceil(u_bin0 + float(cc + 1) * u_binPerCol)));
    if (b1 <= b0) { b0 = min(u_n - 1, b0); b1 = b0 + 1; }
    int stride = max(1, (b1 - b0 + 255) / 256);
    float mn = 1.0, mx = -1.0, rv = 0.0;
    for (int k = 0; k < 256; k++) {
      int b = b0 + k * stride;
      if (b >= b1) break;
      vec4 t = tex(b);
      mn = min(mn, t.x); mx = max(mx, t.y); rv = max(rv, t.z);
    }
    if (!rms) {
      float y0 = u_mid - mx * u_scale, y1 = u_mid - mn * u_scale;
      rect = vec4(float(cc), y0, 1.0, max(1.0, y1 - y0));
      col = vec4(u_color, u_fill > 0.5 ? 1.0 : 85.0 / 255.0);
    } else {
      float hh = rv * u_scale;
      if (u_fill < 0.5 && u_hasRms == 1 && hh > 0.3) rect = vec4(float(cc), u_mid - hh, 1.0, hh * 2.0);
      col = vec4(u_color, 1.0);
    }
    pos = rect.xy + c * rect.zw;
  } else if (u_mode == 1) {
    int a = max(0, int(floor(u_pos0 + float(inst) * u_posPerCol)) - 1);   // the sample before: its line enters this column
    int b = min(u_cnt, int(ceil(u_pos0 + float(inst + 1) * u_posPerCol)) + 1);
    int stride = max(1, (b - a + 255) / 256);
    float mn = 1e9, mx = -1e9;
    for (int k = 0; k < 256; k++) {
      int i = a + k * stride;
      if (i >= b) break;
      float v = tex(i).x;
      mn = min(mn, v); mx = max(mx, v);
    }
    if (mx >= mn) {
      float y0 = u_mid - mx * u_scale - 0.5, y1 = u_mid - mn * u_scale + 0.5;
      rect = vec4(float(inst), y0, 1.0, max(1.0, y1 - y0));
    }
    col = vec4(u_color, 1.0);
    pos = rect.xy + c * rect.zw;
  } else {
    if (inst < u_nseg) {
      float v0 = tex(u_base + inst).x, v1 = tex(u_base + inst + 1).x;
      vec2 p0 = vec2(u_x0 + float(inst) * u_ppx, u_mid - v0 * u_scale);
      vec2 p1 = vec2(u_x0 + float(inst + 1) * u_ppx, u_mid - v1 * u_scale);
      vec2 d = p1 - p0; float len = max(length(d), 1e-4);
      vec2 dn = d / len;
      vec2 nn = vec2(-dn.y, dn.x) * 0.5;
      vec2 base = mix(p0 - dn * 0.5, p1 + dn * 0.5, c.x);
      pos = base + nn * (c.y * 2.0 - 1.0);
      col = vec4(u_color, 1.0);
    } else {
      int i = inst - u_nseg;
      float v = tex(u_base + i).x;
      vec2 p = vec2(u_x0 + float(i) * u_ppx, u_mid - v * u_scale);
      uv = c * 2.0 - 1.0;
      pos = p + uv * 1.4;
      col = vec4(u_color, 1.0);
      dot = 1.0;
    }
  }
  col.rgb = mix(u_page, col.rgb, u_fade);
  v_color = col; v_uv = uv; v_dot = dot;
  gl_Position = vec4(clip(pos), 0.0, 1.0);
}`;
  const FS = `#version 300 es
precision highp float;
in vec4 v_color; in vec2 v_uv; in float v_dot;
out vec4 o;
void main() {
  if (v_dot > 0.5 && dot(v_uv, v_uv) > 1.0) discard;
  o = v_color;
}`;

  function parseColor(c) {
    const m = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})/i.exec(c || "");
    return m ? [parseInt(m[1], 16) / 255, parseInt(m[2], 16) / 255, parseInt(m[3], 16) / 255] : [0.54, 0.58, 0.65];
  }

  function create(opts) {
    opts = opts || {};
    const canvas = opts.canvas || document.createElement("canvas");
    canvas.width = 16; canvas.height = 16;
    let gl = null;
    try {
      gl = canvas.getContext("webgl2", { antialias: false, alpha: true, premultipliedAlpha: true, depth: false, stencil: false,
                                         preserveDrawingBuffer: false, powerPreference: "high-performance" });
    } catch (e) { gl = null; }
    if (!gl) return null;
    const sh = (type, src) => {
      const s = gl.createShader(type); gl.shaderSource(s, src); gl.compileShader(s);
      if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s));
      return s;
    };
    let prog;
    try {
      prog = gl.createProgram();
      gl.attachShader(prog, sh(gl.VERTEX_SHADER, VS)); gl.attachShader(prog, sh(gl.FRAGMENT_SHADER, FS));
      gl.linkProgram(prog);
      if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(prog));
    } catch (e) { return null; }
    gl.useProgram(prog);
    const U = {};
    for (const n of ["u_tex", "u_atlas", "u_lane", "u_org", "u_page", "u_fade", "u_fill", "u_scale", "u_mid", "u_mode", "u_cols", "u_cl", "u_bin0",
                     "u_binPerCol", "u_n", "u_hasRms", "u_pos0", "u_posPerCol", "u_cnt", "u_base", "u_ppx", "u_x0", "u_nseg", "u_color"]) U[n] = gl.getUniformLocation(prog, n);
    gl.uniform1i(U.u_tex, 0);
    gl.bindVertexArray(gl.createVertexArray());
    gl.enable(gl.BLEND);
    gl.blendFuncSeparate(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA, gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
    gl.enable(gl.SCISSOR_TEST);
    gl.activeTexture(gl.TEXTURE0);

    const eng = { canvas, lost: false, stats: { draws: 0, lanes: 0, modes: [0, 0, 0], uploads: 0, last_ms: 0, max_ms: 0, posted: 0, drawn: 0 },
                  maxTexture: gl.getParameter(gl.MAX_TEXTURE_SIZE) };
    const peakTex = new Map();     // key -> { peaks, levels: { L: tex }, owned }; owned: raw tiles, the CPU views go once uploaded
    const wins = new Map();        // key -> { start, n, tex, ref }: sample windows, least recently used first
    const MAX_WINDOWS = opts.maxWindows || 16;

    canvas.addEventListener("webglcontextlost", ev => {
      ev.preventDefault(); eng.lost = true;
      if (opts.onLost) opts.onLost();
    });

    function uploadTex(floats, w, h, internal, fmt) {
      const t = gl.createTexture();
      gl.bindTexture(gl.TEXTURE_2D, t);
      gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
      gl.texImage2D(gl.TEXTURE_2D, 0, internal, w, h, 0, fmt, gl.FLOAT, floats);
      eng.stats.uploads++;
      return t;
    }
    /* one scratch array for the texel layouts of all uploads (a new 2 to 4 MB array for each of 90 levels was the
       garbage that kept the footprint of the process high) */
    let scratchBuf = new Float32Array(0);
    function scratch(len, zero) {
      if (scratchBuf.length < len) scratchBuf = new Float32Array(len);
      const v = scratchBuf.subarray(0, len);
      if (zero) v.fill(0);
      return v;
    }
    /* the tile texture of one level: texel b = (min, max, rms, 0) */
    function levelTex(key, peaks, level) {
      let e = peakTex.get(key);
      if (!e || e.peaks !== peaks) {
        if (e) for (const t of Object.values(e.levels)) gl.deleteTexture(t);
        e = { peaks, levels: {}, owned: false }; peakTex.set(key, e);
      }
      if (e.levels[level]) return e.levels[level];
      const L = peaks.levels[level], n = L.n, h = Math.max(1, Math.ceil(n / TW));
      const f = scratch(TW * h * 4, true), hasRms = Array.isArray(L.rms) || ArrayBuffer.isView(L.rms);
      for (let b = 0; b < n; b++) {
        f[b * 4] = L.min[b]; f[b * 4 + 1] = L.max[b]; f[b * 4 + 2] = hasRms ? L.rms[b] : 0;
      }
      L.hasRms = hasRms;
      const tex = (e.levels[level] = uploadTex(f, TW, h, gl.RGBA32F, gl.RGBA));
      if (e.owned) { L.min = L.max = L.rms = null; }   // raw tiles: the texture is the only copy now
      return tex;
    }
    /* Raw tiles (P3-6b): `raw` = { L: ArrayBuffer of a peaks_tile answer: u32 level, start, n, then f32 min[n], max[n], rms[n] }.
       The levels are uploaded one per task in the background; the views over the bytes go when they are in a texture. */
    eng.addTiles = (key, raw) => {
      const old = peakTex.get(key);
      if (old) for (const t of Object.values(old.levels)) gl.deleteTexture(t);
      const peaks = { levels: {} };
      for (const lv of Object.keys(raw)) {
        const buf = raw[lv], n = new Uint32Array(buf, 0, 3)[2], at = k => new Float32Array(buf, 12 + 4 * k * n, n);
        peaks.levels[lv] = { n, min: at(0), max: at(1), rms: at(2) };
      }
      peakTex.set(key, { peaks, levels: {}, owned: true });
      const q = Object.keys(peaks.levels).map(Number);
      const step = () => {
        if (eng.lost) return;
        const lv = q.shift();
        if (lv === undefined) { if (opts.onUploaded) opts.onUploaded(); return; }
        const e = peakTex.get(key);
        if (e && e.peaks === peaks && !e.levels[lv]) levelTex(key, peaks, lv);
        setTimeout(step, 0);
      };
      setTimeout(step, 0);
    };
    /* A window of samples of one track (Float32Array) as a texture. Least recently used windows go first;
       `ref` is the caller's array (the server path hands the same array in every draw). */
    function putWindow(key, start, data, ref) {
      const old = wins.get(key);
      if (old) { gl.deleteTexture(old.tex); wins.delete(key); }
      const n = data.length, h = Math.max(1, Math.ceil(n / TW)), f = scratch(TW * h, false);
      f.set(data); f.fill(0, n);
      wins.set(key, { start, n, tex: uploadTex(f, TW, h, gl.R32F, gl.RED), ref });
      while (wins.size > MAX_WINDOWS) {
        const [k, w] = wins.entries().next().value;
        gl.deleteTexture(w.tex); wins.delete(k);
        if (opts.onEvict) opts.onEvict(k);
      }
    }
    eng.setWindow = (key, start, data) => putWindow(key, start, data, null);
    eng.hasTiles = key => { const e = peakTex.get(key); return !!(e && e.owned); };
    function useWindow(key) {      // the window of a track, now the most recently used
      const w = wins.get(key);
      if (w) { wins.delete(key); wins.set(key, w); }
      return w;
    }
    /* drop the textures of tracks that are no longer lanes */
    eng.prune = keys => {
      const keep = new Set(keys);
      for (const [k, e] of peakTex) if (!keep.has(k)) { for (const t of Object.values(e.levels)) gl.deleteTexture(t); peakTex.delete(k); }
      for (const [k, w] of wins) if (!keep.has(k)) { gl.deleteTexture(w.tex); wins.delete(k); }
    };
    eng.dropWindows = () => { for (const w of wins.values()) gl.deleteTexture(w.tex); wins.clear(); };
    eng.textures = () => ({ peaks: [...peakTex.values()].reduce((a, e) => a + Object.keys(e.levels).length, 0), samples: wins.size });
    /* upload every level of the given tracks in the background, one level per task */
    eng.warm = items => {
      const q = [];
      for (const it of items) for (const lv of Object.keys(it.peaks.levels)) q.push([it.key, it.peaks, Number(lv)]);
      const step = () => {
        if (eng.lost) return;
        const j = q.shift();
        if (!j) return;
        const e = peakTex.get(j[0]);
        if (!(e && e.peaks === j[1] && e.levels[j[2]])) levelTex(j[0], j[1], j[2]);
        setTimeout(step, 0);
      };
      setTimeout(step, 0);
    };

    function pickLevel(p, spp) {            // the same rule as pickLevel in app.js
      const levels = Object.keys(p.levels).map(Number).sort((a, b) => a - b);
      let best = null;
      for (const L of levels) if (L <= spp) best = L;
      return best;
    }

    /* Size of the canvas in device pixels (a change clears it; the caller draws again). */
    eng.size = (w, h) => {
      if (canvas.width !== w) canvas.width = w;
      if (canvas.height !== h) canvas.height = h;
    };
    /* lanes: [{ key, x, y, dw, dh, W, H, fade, color, peaks, amp, span0, spanS, samples }]
       x, y, dw, dh: the lane box in device pixels on this canvas; W, H: the lane size in CSS px; span0 and
       spanS in samples; peaks = the arrays of the page (server) or absent (the tiles added by `addTiles`);
       samples = { start, data } (a Float32Array window of the page, server) or { start, end } (the window
       added by `setWindow`) or null. Clears the canvas, then draws the lanes in order. */
    eng.draw = lanes => {
      if (eng.lost) return false;
      const t0 = performance.now();
      gl.viewport(0, 0, canvas.width, canvas.height);
      gl.uniform2f(U.u_atlas, canvas.width, canvas.height);
      gl.uniform3f(U.u_page, PAGE[0], PAGE[1], PAGE[2]);
      gl.disable(gl.SCISSOR_TEST);
      gl.clearColor(0, 0, 0, 0);
      gl.clear(gl.COLOR_BUFFER_BIT);
      gl.enable(gl.SCISSOR_TEST);
      for (const l of lanes) {
        gl.scissor(l.x, canvas.height - (l.y + l.dh), l.dw, l.dh);
        const f = l.fade;
        gl.clearColor(PAGE[0] + (BG[0] - PAGE[0]) * f, PAGE[1] + (BG[1] - PAGE[1]) * f, PAGE[2] + (BG[2] - PAGE[2]) * f, 1);
        gl.clear(gl.COLOR_BUFFER_BIT);
        drawLane(l);
      }
      eng.stats.draws++; eng.stats.posted++; eng.stats.drawn++;
      eng.stats.lanes += lanes.length;
      eng.stats.last_ms = performance.now() - t0;
      if (eng.stats.last_ms > eng.stats.max_ms) eng.stats.max_ms = eng.stats.last_ms;
      return true;
    };

    /* The pixels of one lane as the user sees the GL part of it: { width, height, data: RGBA bytes, top row first }.
       Draws the lane alone at the corner (the caller draws the canvas again afterwards), reads it back in the same
       task (the browser clears a WebGL canvas after it shows it). */
    eng.readLane = l => {
      const lane = Object.assign({}, l, { x: 0, y: 0 });
      if (!eng.draw([lane])) return null;
      const w = l.dw, h = l.dh, px = new Uint8Array(w * h * 4), out = new Uint8ClampedArray(w * h * 4);
      gl.readPixels(0, canvas.height - h, w, h, gl.RGBA, gl.UNSIGNED_BYTE, px);
      for (let r = 0; r < h; r++) out.set(px.subarray((h - 1 - r) * w * 4, (h - r) * w * 4), r * w * 4);
      return { width: w, height: h, data: out };
    };
    eng.close = () => {};

    function drawLane(l) {
      const e = l.peaks ? null : peakTex.get(l.key), p = l.peaks || (e && e.peaks), W = l.W, scale = (l.H * 0.45) / (l.amp || 1) * (l.gain || 1);   // P3-20a: gain = the amplitude zoom of the page (display only); the lane box clips
      if (!p) return;       // the tiles of this track are not here yet: the lane stays empty
      const col = parseColor(l.color);
      gl.uniform4f(U.u_lane, W, l.H, l.dw / W, l.dh / l.H);
      gl.uniform2f(U.u_org, l.x, l.y);
      gl.uniform1f(U.u_fade, l.fade);
      gl.uniform1f(U.u_fill, l.fill ? 1 : 0);
      gl.uniform1f(U.u_scale, scale);
      gl.uniform1f(U.u_mid, l.H / 2);
      gl.uniform3f(U.u_color, col[0], col[1], col[2]);
      let c = l.samples, w = null;
      if (c && c.data) {      // the page hands the array in (server): one texture per track, replaced when the array changes
        w = wins.get(l.key);
        if (!w || w.ref !== c.data || w.start !== c.start) { putWindow(l.key, c.start, c.data, c.data); }
        w = useWindow(l.key);
      } else if (c) {
        w = useWindow(l.key);
        if (!w || w.start !== c.start) w = null;
      }
      if (!w) c = null;
      let count;
      if (c) {
        const i0 = Math.max(c.start, Math.floor(l.span0)), i1 = Math.min(c.start + w.n, Math.ceil(l.span0 + l.spanS));
        const spp = l.spanS / W;
        gl.bindTexture(gl.TEXTURE_2D, w.tex);
        gl.uniform1i(U.u_cnt, Math.max(0, i1 - c.start));
        if (spp > 2) {      // dense: one bar per column, from the lowest to the highest sample in it
          gl.uniform1i(U.u_mode, 1);
          gl.uniform1i(U.u_cols, W);
          gl.uniform1f(U.u_pos0, l.span0 - c.start);
          gl.uniform1f(U.u_posPerCol, spp);
          gl.uniform1i(U.u_cl, W);
          count = W + 1;
          eng.stats.modes[1]++;
        } else {            // sparse: the line from sample to sample, dots when the samples are > 3 px apart
          const nseg = Math.max(0, i1 - i0 - 1), pxPer = W / l.spanS, dots = pxPer > 3 ? Math.max(0, i1 - i0) : 0;
          gl.uniform1i(U.u_mode, 2);
          gl.uniform1f(U.u_ppx, pxPer);
          gl.uniform1f(U.u_x0, (i0 - l.span0) * pxPer);
          gl.uniform1i(U.u_nseg, nseg);
          gl.uniform1i(U.u_base, i0 - c.start);
          gl.uniform1i(U.u_cl, nseg + dots);
          count = nseg + dots + 1;
          eng.stats.modes[2]++;
        }
      } else {
        const level = pickLevel(p, l.spanS / W) || 256, L = p.levels[level];
        gl.bindTexture(gl.TEXTURE_2D, levelTex(l.key, p, level));
        gl.uniform1i(U.u_mode, 0);
        gl.uniform1i(U.u_cols, W);
        gl.uniform1i(U.u_cl, 2 * W);
        gl.uniform1f(U.u_bin0, l.span0 / level);
        gl.uniform1f(U.u_binPerCol, l.spanS / W / level);
        gl.uniform1i(U.u_n, L.n);
        gl.uniform1i(U.u_hasRms, L.hasRms ? 1 : 0);
        count = 2 * W + 1;
        eng.stats.modes[0]++;
      }
      gl.drawArraysInstanced(gl.TRIANGLE_STRIP, 0, 4, count);
    }
    /* P4-1b: let the drawing buffer go (the page hides the canvas under a strip): a 1 x 1 canvas, cleared, so the shown
       frame is 1 x 1 too; the next draw sizes it again */
    eng.release = () => {
      if (eng.lost || (canvas.width === 1 && canvas.height === 1)) return;
      eng.size(1, 1);
      gl.disable(gl.SCISSOR_TEST);
      gl.clearColor(0, 0, 0, 0);
      gl.clear(gl.COLOR_BUFFER_BIT);
      gl.enable(gl.SCISSOR_TEST);
    };
    /* test hook: lose the context like a GPU reset */
    eng.loseContext = () => {
      const x = gl.getExtension("WEBGL_lose_context");
      if (x) x.loseContext();
      return !!x;
    };
    eng.info = () => {
      const dbg = gl.getExtension("WEBGL_debug_renderer_info");
      return { renderer: dbg ? gl.getParameter(dbg.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER),
               max_texture: eng.maxTexture, canvas: [canvas.width, canvas.height] };
    };
    return eng;
  }

  /* ---- the worker side (tiles-worker.js): messages in, messages out ---- */
  const MAX_IN_FLIGHT = 2;
  const LANE_KEYS = ["key", "x", "y", "dw", "dh", "W", "H", "fade", "fill", "color", "amp", "gain", "span0", "spanS"];
  function host(post) {
    let eng = null, shown = null;   // shown: the last draw ({ w, h, lanes }), what the canvas (#lanesGL) shows
    const note = () => post({ op: "textures", textures: eng.textures() });
    return function handle(m) {
      if (m.op === "init") {
        eng = create({ canvas: m.canvas, onLost: () => post({ op: "lost" }), onEvict: key => post({ op: "evicted", key }),
                       onUploaded: () => note(), maxWindows: m.maxWindows });
        post(eng ? { op: "ready", ok: true, info: eng.info(), max_texture: eng.maxTexture } : { op: "ready", ok: false });
        return;
      }
      if (!eng) return;
      if (m.op === "tiles") { eng.addTiles(m.key, m.raw); }
      else if (m.op === "window") { eng.setWindow(m.key, m.start, m.data); post({ op: "textures", textures: eng.textures() }); }
      else if (m.op === "prune") { eng.prune(m.keys); note(); }
      else if (m.op === "drop_windows") { eng.dropWindows(); note(); }
      else if (m.op === "draw") {
        let ok = false, error = "";
        try { eng.size(m.w, m.h); ok = eng.draw(m.lanes); } catch (e) { error = String(e && e.message || e); }
        if (ok) shown = { w: m.w, h: m.h, lanes: m.lanes };
        post(ok ? { op: "drawn", seq: m.seq, stats: eng.stats } : { op: "failed", error: error || "the context is lost" });
      } else if (m.op === "read") {
        let r = null;
        try { eng.size(m.w, m.h); r = eng.readLane(m.lane); } catch (e) { r = null; }
        post({ op: "read", id: m.id, width: r && r.width, height: r && r.height, data: r && r.data.buffer }, r ? [r.data.buffer] : []);
      } else if (m.op === "strip") {
        /* P4-1: a strip of the smooth follow, in parts of one view each (m.parts: the lanes of each part). Each part is
           drawn on the canvas and taken off it as an ImageBitmap, which goes to the page: the page shows the bitmaps
           and the 2D lanes in one frame, so the wave and the hit lines move together. The canvas itself (#lanesGL)
           is hidden while a strip shows; the page draws it again when the strip ends.
           P4-1b: transferToImageBitmap leaves the canvas with a cleared buffer, and the end of this task shows it on
           #lanesGL. The page asks for the first strip while #lanesGL is still visible (the entry frame), so the lanes
           went blank until the next draw (measured in Chromium: blank until then, 0 of 788 wave pixels). So the
           worker draws the last shown lanes again before the task ends (m.keep: the page shows the canvas now): the
           canvas never shows the cleared buffer. Under a strip (the canvas is hidden) it lets the buffer go instead. */
        let bitmaps = null, error = "";
        try {
          if (typeof eng.canvas.transferToImageBitmap !== "function") throw new Error("no transferToImageBitmap");
          eng.size(m.w, m.h);
          bitmaps = m.parts.map(lanes => {
            if (!eng.draw(lanes)) throw new Error("the context is lost");
            return eng.canvas.transferToImageBitmap();
          });
        } catch (e) { error = String(e && e.message || e); if (bitmaps) bitmaps.forEach(b => b.close()); bitmaps = null; }
        try { if (!m.keep) eng.release(); else if (shown && !eng.lost) { eng.size(shown.w, shown.h); eng.draw(shown.lanes); } } catch (e) { /* the next draw shows the lanes */ }
        post(bitmaps ? { op: "strip", id: m.id, bitmaps } : { op: "strip", id: m.id, error }, bitmaps || []);
      } else if (m.op === "stall") {   // test hook: a worker that is behind (busy for m.ms)
        const end = performance.now() + m.ms;
        while (performance.now() < end) { /* busy */ }
      } else if (m.op === "release") { eng.release(); }
      else if (m.op === "lose") { post({ op: "lose", id: m.id, ok: eng.loseContext() }); }
      else if (m.op === "info") { post({ op: "info", id: m.id, info: eng.info(), stats: eng.stats, textures: eng.textures() }); }
    };
  }

  /* ---- the page side: the same calls as `create`, the work happens in a worker ---- */
  function remote(opts) {
    opts = opts || {};
    if (typeof Worker === "undefined" || typeof OffscreenCanvas === "undefined") return null;
    const canvas = document.createElement("canvas");
    if (!canvas.transferControlToOffscreen) return null;
    let worker;
    try {
      worker = new Worker(opts.url || "tiles-worker.js");
      const off = canvas.transferControlToOffscreen();
      worker.postMessage({ op: "init", canvas: off, maxWindows: opts.maxWindows }, [off]);
    } catch (e) { return null; }
    const eng = { canvas, lost: false, remote: true, ready: false, failed: false, size_: [16, 16], seq: 0, maxTexture: 0, info_: null, tex: { peaks: 0, samples: 0 },
                  stats: { draws: 0, lanes: 0, modes: [0, 0, 0], uploads: 0, last_ms: 0, max_ms: 0, posted: 0, drawn: 0 } };
    const waits = new Map();
    let ids = 0;
    const ask = m => new Promise(res => { if (eng.lost) { res(null); return; } m.id = ++ids; waits.set(m.id, res); worker.postMessage(m); });
    const fail = why => {
      if (eng.lost) return;
      eng.lost = true; eng.failed = true; eng.why = why;
      for (const res of waits.values()) res(null);
      waits.clear();
      if (opts.onLost) opts.onLost(why);
    };
    worker.onerror = ev => fail(ev && ev.message || "worker error");
    worker.onmessage = ev => {
      const m = ev.data;
      if (m.op === "ready") {
        if (!m.ok) { fail("no WebGL2 in the worker"); return; }
        eng.ready = true; eng.info_ = m.info; eng.maxTexture = m.max_texture;
        if (opts.onReady) opts.onReady();
      } else if (m.op === "drawn") {
        const st = eng.stats;
        st.drawn = Math.max(st.drawn, m.seq);         // the draw of request m.seq is done (older requests it replaced count as done)
        for (const k of ["draws", "lanes", "modes", "uploads", "last_ms", "max_ms"]) st[k] = m.stats[k];
        inFlight--; sendPending();
        if (opts.onDrawn) opts.onDrawn();
      } else if (m.op === "textures") { eng.tex = m.textures; }
      else if (m.op === "evicted") { if (opts.onEvict) opts.onEvict(m.key); }
      else if (m.op === "lost" || m.op === "failed") { fail(m.error || "context lost"); }
      else if (waits.has(m.id)) { const res = waits.get(m.id); waits.delete(m.id); res(m); }
    };
    const plain = l => {
      const o = {}; for (const k of LANE_KEYS) o[k] = l[k];
      o.samples = l.samples ? { start: l.samples.start, end: l.samples.end } : null;
      return o;
    };
    eng.size = (w, h) => { eng.size_ = [w, h]; };
    /* At most MAX_IN_FLIGHT draws wait in the worker. A request that comes while they do replaces the one that waits
       in the page, so a burst of redraws (a resize, a loop) costs one draw and a few surfaces, and the worker always
       draws the newest lanes. stats.posted counts the requests, stats.drawn is the newest request that was drawn. */
    let inFlight = 0, pending = null;
    function sendPending() {
      if (!pending || inFlight >= MAX_IN_FLIGHT || eng.lost) return;
      worker.postMessage(pending); pending = null; inFlight++;
    }
    eng.draw = lanes => {
      if (eng.lost) return false;
      eng.stats.posted++;
      pending = { op: "draw", seq: ++eng.seq, w: eng.size_[0], h: eng.size_[1], lanes: lanes.map(plain) };
      sendPending();
      return true;
    };
    const keys = new Set();
    eng.addTiles = (key, raw) => {
      keys.add(key);
      worker.postMessage({ op: "tiles", key, raw }, Object.values(raw));   // the buffers move to the worker: the page keeps nothing
    };
    eng.hasTiles = key => keys.has(key);
    eng.setWindow = (key, start, data) => { held.add(key); worker.postMessage({ op: "window", key, start, data }, [data.buffer]); };
    const held = new Set();       // keys with a window in the worker
    eng.prune = keep => {         // only when something goes: the page calls this before every draw
      const gone = [...keys, ...held].some(k => !keep.includes(k));
      for (const k of [...keys]) if (!keep.includes(k)) keys.delete(k);
      for (const k of [...held]) if (!keep.includes(k)) held.delete(k);
      if (gone) worker.postMessage({ op: "prune", keys: keep.slice() });
    };
    eng.dropWindows = () => { held.clear(); worker.postMessage({ op: "drop_windows" }); };
    eng.textures = () => eng.tex;
    eng.info = () => eng.info_ || { renderer: "", max_texture: 0, canvas: eng.size_ };
    /* the lane as pixels: { width, height, data } or null */
    eng.readLane = async l => {
      if (eng.lost) return null;
      const m = await ask({ op: "read", w: eng.size_[0], h: eng.size_[1], lane: plain(Object.assign({}, l, { x: 0, y: 0 })) });
      return m && m.data ? { width: m.width, height: m.height, data: new Uint8ClampedArray(m.data) } : null;
    };
    eng.loseContext = async () => { const m = await ask({ op: "lose" }); return !!(m && m.ok); };
    eng.release = () => { if (!eng.lost) worker.postMessage({ op: "release" }); };   // P4-1b: see create
    /* P4-1: the lanes of each part ([[lane, ...], ...]) as ImageBitmaps of the canvas size: { bitmaps } or { error }.
       P4-1b: keep: the canvas shows, so the worker draws the last lanes again after the parts (else it lets the buffer go). */
    eng.strip = async (parts, keep) => {
      const m = await ask({ op: "strip", w: eng.size_[0], h: eng.size_[1], keep: !!keep, parts: parts.map(ls => ls.map(plain)) });
      return m ? (m.bitmaps ? { bitmaps: m.bitmaps } : { error: m.error }) : { error: "the engine is gone" };
    };
    eng.stall = ms => { if (!eng.lost) worker.postMessage({ op: "stall", ms }); };   // test hook
    eng.query = async () => { const m = await ask({ op: "info" }); return m && { info: m.info, stats: m.stats, textures: m.textures }; };
    eng.close = () => { worker.terminate(); eng.lost = true; for (const res of waits.values()) res(null); waits.clear(); };
    return eng;
  }

  (typeof window !== "undefined" ? window : self).LanesGL = { create, host, remote };
})();
