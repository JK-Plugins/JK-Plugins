// JK Tone Gradient をブラウザで試すページ。
// 計算はプラグインと同じ Rust のエンジンを WebAssembly にしたもの（tonegradient_web.wasm）。
// パラメータの並びは web/tonegradient/src/lib.rs の P_* と合わせる。
(function () {
  "use strict";

  var P = {
    width: 0, offset: 1, reverse: 2, curve: 3, curve_power: 4, amount: 5, tex_mode: 6, softness: 7,
    roughness: 8, rough_scale: 9, mid_amount: 10, mid_color: 11, mid_width: 14,
    output: 16, tex_kind: 17, cell: 18, angle: 19, dot_shape: 20, aspect: 21, dither_size: 22,
    octaves: 23, noise_contrast: 24, seed: 25, invert: 26, tolerance: 27
  };
  var N_PARAMS = 28;
  // 0..100 の UI を 0..1 にするもの、0..255 の UI を 0..1 にするもの
  var PERCENT = { amount: 1, mid_amount: 1, mid_width: 1 };
  var BYTE = { tolerance: 1 };

  var canvas = document.getElementById("canvas");
  var ctx = canvas.getContext("2d");
  var panel = document.getElementById("panel");
  var msEl = document.getElementById("ms");

  var wasm = null;
  var W = 0, H = 0, src = null, colors = [], usingImage = false;
  var bufs = null;

  // ---- サンプル画像（プラグインの説明の画像と同じ球の塗り分け） ----
  var HI = [1.0, 0.95, 0.88], BASE = [0.96, 0.80, 0.70], SHADOW = [0.80, 0.58, 0.55], SHADOW2 = [0.62, 0.42, 0.48];
  var BG = [0.55, 0.70, 0.85], LINE = [0.1, 0.08, 0.1];
  function u8(c) { return c.map(function (v) { return Math.round(v * 255); }); }

  function sampleScene(w, h) {
    var px = new Uint8ClampedArray(w * h * 4);
    var cx = w * 0.5, cy = h * 0.5, r = h * 0.42;
    var cols = [u8(HI), u8(BASE), u8(SHADOW), u8(SHADOW2), u8(BG), u8(LINE)];
    for (var y = 0; y < h; y++) {
      for (var x = 0; x < w; x++) {
        var fx = x + 0.5, fy = y + 0.5;
        var d = Math.hypot(fx - cx, fy - cy), c;
        if (Math.abs(d - r) < 2.0) c = cols[5];
        else if (d > r) c = cols[4];
        else if (Math.hypot(fx - cx + r * 0.35, fy - cy + r * 0.4) < r * 0.22) c = cols[0];
        else if (Math.hypot(fx - cx - r * 0.35, fy - cy - r * 0.2) < r * 0.45) c = cols[3];
        else if (fx - cx + (fy - cy) * 0.4 > -r * 0.05) c = cols[2];
        else c = cols[1];
        var i = (y * w + x) * 4;
        px[i] = c[0]; px[i + 1] = c[1]; px[i + 2] = c[2]; px[i + 3] = 255;
      }
    }
    return px;
  }

  function useSample() {
    W = 480; H = 360; src = sampleScene(W, H); usingImage = false;
    colors = [HI, BASE, SHADOW, SHADOW2].map(function (c) { return { rgb: u8(c), on: true }; });
    setup();
  }

  // ---- 読み込んだ画像：よく使われている色を自動で選ぶ ----
  function useImage(img) {
    var max = 720, s = Math.min(1, max / Math.max(img.naturalWidth, img.naturalHeight));
    W = Math.max(1, Math.round(img.naturalWidth * s)); H = Math.max(1, Math.round(img.naturalHeight * s));
    var c = document.createElement("canvas"); c.width = W; c.height = H;
    var cx = c.getContext("2d");
    cx.imageSmoothingEnabled = s < 1;
    cx.drawImage(img, 0, 0, W, H);
    src = new Uint8ClampedArray(cx.getImageData(0, 0, W, H).data);
    usingImage = true;
    var count = new Map();
    for (var i = 0; i < src.length; i += 4) {
      if (src[i + 3] < 255) continue;
      var k = (src[i] << 16) | (src[i + 1] << 8) | src[i + 2];
      count.set(k, (count.get(k) || 0) + 1);
    }
    var min = W * H * 0.01;
    var top = Array.from(count.entries()).filter(function (e) { return e[1] >= min; })
      .sort(function (a, b) { return b[1] - a[1]; }).slice(0, 8)
      .map(function (e) { return [(e[0] >> 16) & 255, (e[0] >> 8) & 255, e[0] & 255]; });
    // 明るい順（ハイライト → 影）に並べる
    top.sort(function (a, b) { return lum(b) - lum(a); });
    colors = top.map(function (rgb) { return { rgb: rgb, on: true }; });
    if (usingImage && colors.length < 2) msEl.textContent = "同じ色の面が少ない画像です（ベタ塗りの画像向け）";
    setup();
  }
  function lum(c) { return 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]; }

  // ---- WebAssembly のメモリ ----
  function setup() {
    canvas.width = W; canvas.height = H;
    if (!wasm) return;
    var e = wasm.exports;
    if (bufs) {
      e.dealloc(bufs.src, bufs.n); e.dealloc(bufs.out, bufs.n);
    } else {
      bufs = { params: e.alloc(N_PARAMS * 4), colors: e.alloc(32 * 3 * 4) };
    }
    bufs.n = W * H * 4;
    bufs.src = e.alloc(bufs.n); bufs.out = e.alloc(bufs.n);
    new Uint8Array(e.memory.buffer, bufs.src, bufs.n).set(src);
    renderSwatches();
    updateVisibility();
    schedule();
  }

  // ---- パラメータ ----
  function readParams() {
    var p = new Float32Array(N_PARAMS);
    panel.querySelectorAll("[data-p]").forEach(function (el) {
      var name = el.dataset.p, i = P[name], v;
      if (el.classList.contains("seg")) v = +el.querySelector(".on").dataset.v;
      else if (el.type === "checkbox") v = el.checked ? 1 : 0;
      else if (el.type === "color") {
        var h = el.value;
        p[i] = parseInt(h.substr(1, 2), 16) / 255; p[i + 1] = parseInt(h.substr(3, 2), 16) / 255; p[i + 2] = parseInt(h.substr(5, 2), 16) / 255;
        return;
      } else v = +el.value;
      if (PERCENT[name]) v /= 100;
      if (BYTE[name]) v /= 255;
      p[i] = v;
    });
    return p;
  }

  var pending = false, showBefore = false;
  function schedule() {
    if (pending) return;
    pending = true;
    requestAnimationFrame(function () { pending = false; draw(); });
  }

  function draw() {
    if (!wasm || !src) return;
    var e = wasm.exports;
    if (showBefore) {
      ctx.putImageData(new ImageData(new Uint8ClampedArray(src), W, H), 0, 0);
      return;
    }
    var on = colors.filter(function (c) { return c.on; });
    new Float32Array(e.memory.buffer, bufs.params, N_PARAMS).set(readParams());
    var cf = new Float32Array(e.memory.buffer, bufs.colors, 32 * 3);
    on.forEach(function (c, i) { cf[i * 3] = c.rgb[0] / 255; cf[i * 3 + 1] = c.rgb[1] / 255; cf[i * 3 + 2] = c.rgb[2] / 255; });
    var t0 = performance.now();
    e.render(W, H, bufs.src, bufs.params, bufs.colors, on.length, bufs.out);
    var dt = performance.now() - t0;
    var out = new Uint8ClampedArray(e.memory.buffer, bufs.out, W * H * 4);
    ctx.putImageData(new ImageData(new Uint8ClampedArray(out), W, H), 0, 0);
    msEl.textContent = on.length < 2 ? "色を2つ以上オンにしてください" : W + "×" + H + " ・ " + dt.toFixed(0) + " ms";
  }

  function fmt(el) {
    var v = +el.value, name = el.dataset.p;
    if (PERCENT[name]) return v + "%";
    if (name === "angle") return v + "°";
    if (el.step && el.step.indexOf(".") >= 0) return v.toFixed(el.step.length > 3 ? 2 : 1);
    return String(v);
  }

  function updateVisibility() {
    var kind = panel.querySelector('[data-p="tex_kind"] .on').dataset.v;
    var mode = panel.querySelector('[data-p="tex_mode"] .on').dataset.v;
    var shape = panel.querySelector('[data-p="dot_shape"]').value;
    panel.querySelectorAll(".row").forEach(function (row) {
      var ok = true;
      if (row.dataset.tex) ok = ok && row.dataset.tex.split(" ").indexOf(kind) >= 0;
      if (row.dataset.mode) ok = ok && row.dataset.mode === mode;
      if (row.dataset.shape) ok = ok && row.dataset.shape === shape;
      if (row.dataset.show === "img") ok = ok && usingImage;
      row.hidden = !ok;
    });
    // Mid Color は2値にならないとき（Smooth か Map）だけ
    var midOn = kind === "0" || mode === "1";
    var mg = document.getElementById("midgroup");
    mg.classList.toggle("disabled", !midOn);
    mg.querySelectorAll("input").forEach(function (el) { el.disabled = !midOn; });
    document.getElementById("midnote").hidden = midOn;
    panel.querySelectorAll("input[type=range]").forEach(function (el) {
      var o = el.parentNode.querySelector("output"); if (o) o.textContent = fmt(el);
    });
  }

  var defaults = new Map();
  panel.querySelectorAll("[data-p]").forEach(function (el) {
    if (el.classList.contains("seg")) {
      defaults.set(el, el.querySelector(".on").dataset.v);
      el.addEventListener("click", function (ev) {
        var b = ev.target.closest("button"); if (!b) return;
        el.querySelectorAll("button").forEach(function (x) { x.classList.toggle("on", x === b); });
        updateVisibility(); schedule();
      });
    } else {
      defaults.set(el, el.type === "checkbox" ? el.checked : el.value);
      el.addEventListener("input", function () { updateVisibility(); schedule(); });
    }
  });
  document.getElementById("reset").addEventListener("click", function () {
    defaults.forEach(function (v, el) {
      if (el.classList.contains("seg")) el.querySelectorAll("button").forEach(function (x) { x.classList.toggle("on", x.dataset.v === v); });
      else if (el.type === "checkbox") el.checked = v;
      else el.value = v;
    });
    updateVisibility(); schedule();
  });

  // ---- 色の並び ----
  var swatches = document.getElementById("swatches");
  function hex(c) { return "#" + c.map(function (v) { return ("0" + v.toString(16)).slice(-2); }).join(""); }
  function renderSwatches() {
    swatches.innerHTML = "";
    var n = 0;
    colors.forEach(function (c, i) {
      var item = document.createElement("span");
      item.className = "sw-item" + (c.on ? "" : " off");
      var left = document.createElement("button"); left.className = "arrow"; left.textContent = "‹"; left.title = "前へ";
      var chip = document.createElement("button"); chip.className = "chip"; chip.style.background = hex(c.rgb);
      chip.title = hex(c.rgb) + (c.on ? "（オン）" : "（オフ）");
      if (c.on) { n++; chip.innerHTML = "<b>" + n + "</b>"; }
      var right = document.createElement("button"); right.className = "arrow"; right.textContent = "›"; right.title = "後ろへ";
      left.disabled = i === 0; right.disabled = i === colors.length - 1;
      left.onclick = function () { move(i, -1); };
      right.onclick = function () { move(i, 1); };
      chip.onclick = function () { c.on = !c.on; renderSwatches(); schedule(); };
      item.append(left, chip, right);
      swatches.append(item);
    });
  }
  function move(i, d) {
    var j = i + d; if (j < 0 || j >= colors.length) return;
    var t = colors[i]; colors[i] = colors[j]; colors[j] = t;
    renderSwatches(); schedule();
  }

  // ---- 表示 ----
  var viewer = document.getElementById("viewer");
  document.getElementById("zoom").addEventListener("click", function (ev) {
    var b = ev.target.closest("button"); if (!b) return;
    this.querySelectorAll("button").forEach(function (x) { x.classList.toggle("on", x === b); });
    var z = +b.dataset.v;
    viewer.classList.toggle("zoomed", z > 1);
    canvas.style.width = z > 1 ? (W * z) + "px" : "";
  });
  var before = document.getElementById("before");
  function hold(v) { showBefore = v; before.classList.toggle("active", v); draw(); }
  before.addEventListener("pointerdown", function (e) { e.preventDefault(); hold(true); });
  ["pointerup", "pointerleave", "pointercancel"].forEach(function (t) { before.addEventListener(t, function () { if (showBefore) hold(false); }); });
  document.getElementById("file").addEventListener("change", function () {
    var f = this.files && this.files[0]; if (!f) return;
    var img = new Image();
    img.onload = function () { useImage(img); URL.revokeObjectURL(img.src); };
    img.src = URL.createObjectURL(f);
    this.value = "";
  });
  document.getElementById("sample").addEventListener("click", useSample);

  // ---- 起動 ----
  useSample();
  fetch("tonegradient_web.wasm")
    .then(function (r) { if (!r.ok) throw new Error(r.status); return r.arrayBuffer(); })
    .then(function (b) { return WebAssembly.instantiate(b, {}); })
    .then(function (m) {
      wasm = m.instance;
      document.getElementById("loading").hidden = true;
      setup();
    })
    .catch(function (err) {
      document.getElementById("loading").textContent = "読み込めませんでした（" + err + "）";
    });
})();
