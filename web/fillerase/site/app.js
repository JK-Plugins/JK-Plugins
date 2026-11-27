// JK Fill Erase をブラウザで試すページ。
// 計算はプラグインと同じ Rust のエンジンを WebAssembly にしたもの（fillerase_web.wasm）。
// パラメータの並びは web/fillerase/src/lib.rs の P_* と合わせる。
(function () {
  "use strict";

  var P = { tolerance: 0, softness: 1, choke: 2, contiguous: 3, compare_alpha: 4, invert: 5, view: 6 };
  var N_PARAMS = 7, MAX_POINTS = 99;
  // 0..100 の UI を 0..1 にするもの
  var PERCENT = { tolerance: 1, softness: 1 };

  var canvas = document.getElementById("canvas");
  var ctx = canvas.getContext("2d");
  var pic = document.getElementById("pic");
  var panel = document.getElementById("panel");
  var msEl = document.getElementById("ms");

  var wasm = null;
  // scale：元の画像の大きさ ÷ ページで扱う大きさ（Ae に渡す点の位置を元に戻す）
  var W = 0, H = 0, src = null, scale = 1, points = [];
  var bufs = null;

  // ---- サンプル画像：VHS テープのラベルのようなグラフィック ----
  // ピクセルごとに色を決める（アンチエイリアスなし。境界はくっきり）。
  // クリーム色の地に、斜めの虹の帯・横じまの切れ目が入った太陽・左上の短い横じま・下の紺の帯と細い虹の線
  function hexRgb(h) { return [parseInt(h.substr(1, 2), 16), parseInt(h.substr(3, 2), 16), parseInt(h.substr(5, 2), 16)]; }
  var CREAM = hexRgb("#f1e6cf"), INK = hexRgb("#151515"), NAVY = hexRgb("#1b1f4a");
  var RAINBOW = ["#f6b51d", "#f07a1a", "#e3204a", "#b51c7d", "#5b2a86"].map(hexRgb);

  function sampleScene(w, h) {
    var px = new Uint8ClampedArray(w * h * 4);
    var k = h / 360;
    // 斜めの帯：右上がりの線に垂直な座標 u で、帯の中の何本目かを決める
    var nx = 0.42, ny = 1, nl = Math.hypot(nx, ny);
    var u0 = 190 * k, bw = 15 * k;
    // 太陽
    var cx = w * 0.74, cy = h * 0.4, r = h * 0.24;
    // 太陽の下半分の切れ目（t は太陽の上端 0〜下端 1、厚みは下ほど太く）
    var gaps = [[0.56, 2], [0.66, 3], [0.75, 4], [0.83, 5], [0.9, 6], [0.96, 7]];
    var barTop = Math.round(h * 0.84);
    for (var y = 0; y < h; y++) {
      for (var x = 0; x < w; x++) {
        var c = CREAM;
        var band = Math.floor(((x * nx + y * ny) / nl - u0) / bw);
        if (band >= 0 && band < RAINBOW.length) c = RAINBOW[band];
        var dx = x + 0.5 - cx, dy = y + 0.5 - cy;
        if (dx * dx + dy * dy < r * r) {
          var t = (dy + r) / (2 * r), cut = false;
          for (var g = 0; g < gaps.length; g++) {
            var gy = cy - r + gaps[g][0] * 2 * r;
            if (y + 0.5 >= gy && y + 0.5 < gy + gaps[g][1] * k) cut = true;
          }
          if (!cut) c = RAINBOW[Math.min(3, Math.floor(t * 4))];
        }
        if (y >= barTop) {
          c = NAVY;
          var line = Math.floor((y - barTop - Math.round(10 * k)) / Math.max(1, Math.round(4 * k)));
          if (line >= 0 && line < RAINBOW.length) c = RAINBOW[line];
        }
        // 左上：短い横じま（下ほど短く、間に地の色のすき間）
        var sy = Math.floor((y - 26 * k) / (9 * k)), sr = (y - 26 * k) - sy * 9 * k;
        if (sy >= 0 && sy < 4 && sr < 6 * k && x >= 24 * k && x < (150 - sy * 26) * k) c = sy === 0 ? INK : RAINBOW[sy - 1];
        var i = (y * w + x) * 4;
        px[i] = c[0]; px[i + 1] = c[1]; px[i + 2] = c[2]; px[i + 3] = 255;
      }
    }
    return px;
  }

  function useSample() {
    W = 480; H = 360; scale = 1; src = sampleScene(W, H);
    points = [[12, 12]];
    setup();
  }

  function useImage(img) {
    var max = 720, s = Math.min(1, max / Math.max(img.naturalWidth, img.naturalHeight));
    W = Math.max(1, Math.round(img.naturalWidth * s)); H = Math.max(1, Math.round(img.naturalHeight * s));
    scale = img.naturalWidth / W;
    var c = document.createElement("canvas"); c.width = W; c.height = H;
    var cx = c.getContext("2d");
    cx.imageSmoothingEnabled = s < 1;
    cx.drawImage(img, 0, 0, W, H);
    src = new Uint8ClampedArray(cx.getImageData(0, 0, W, H).data);
    // プラグインの既定と同じく中央に1つ
    points = [[Math.floor(W / 2), Math.floor(H / 2)]];
    setup();
  }

  // ---- WebAssembly のメモリ ----
  function setup() {
    canvas.width = W; canvas.height = H;
    applyZoom();
    renderPoints();
    if (!wasm) return;
    var e = wasm.exports;
    if (bufs) {
      e.dealloc(bufs.src, bufs.n); e.dealloc(bufs.out, bufs.n);
    } else {
      bufs = { params: e.alloc(N_PARAMS * 4), points: e.alloc(MAX_POINTS * 2 * 4) };
    }
    bufs.n = W * H * 4;
    bufs.src = e.alloc(bufs.n); bufs.out = e.alloc(bufs.n);
    new Uint8Array(e.memory.buffer, bufs.src, bufs.n).set(src);
    updateOutputs();
    schedule();
  }

  // ---- パラメータ ----
  function val(name) {
    var el = panel.querySelector('[data-p="' + name + '"]');
    if (el.classList.contains("seg")) return +el.querySelector(".on").dataset.v;
    if (el.type === "checkbox") return el.checked ? 1 : 0;
    return +el.value;
  }
  function readParams() {
    var p = new Float32Array(N_PARAMS);
    Object.keys(P).forEach(function (name) {
      var v = val(name);
      if (PERCENT[name]) v /= 100;
      p[P[name]] = v;
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
    new Float32Array(e.memory.buffer, bufs.params, N_PARAMS).set(readParams());
    var pf = new Float32Array(e.memory.buffer, bufs.points, MAX_POINTS * 2);
    points.forEach(function (q, i) { pf[i * 2] = q[0]; pf[i * 2 + 1] = q[1]; });
    var t0 = performance.now();
    e.render(W, H, bufs.src, bufs.params, bufs.points, points.length, bufs.out);
    var dt = performance.now() - t0;
    var out = new Uint8ClampedArray(e.memory.buffer, bufs.out, W * H * 4);
    ctx.putImageData(new ImageData(new Uint8ClampedArray(out), W, H), 0, 0);
    msEl.textContent = W + "×" + H + " ・ " + dt.toFixed(0) + " ms";
  }

  function fmt(el) {
    var v = +el.value, name = el.dataset.p;
    if (PERCENT[name]) return v + "%";
    if (name === "choke") return (v > 0 ? "+" : "") + v + "px";
    return String(v);
  }
  function updateOutputs() {
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
        schedule();
      });
    } else {
      defaults.set(el, el.type === "checkbox" ? el.checked : el.value);
      el.addEventListener("input", function () { updateOutputs(); schedule(); });
    }
  });
  document.getElementById("reset").addEventListener("click", function () {
    defaults.forEach(function (v, el) {
      if (el.classList.contains("seg")) el.querySelectorAll("button").forEach(function (x) { x.classList.toggle("on", x.dataset.v === v); });
      else if (el.type === "checkbox") el.checked = v;
      else el.value = v;
    });
    updateOutputs(); schedule();
  });

  // ---- 点：クリックで足す・ドラッグで動かす・ダブルクリックで外す ----
  var countEl = document.getElementById("count");
  function renderPoints() {
    pic.querySelectorAll(".pt").forEach(function (m) { m.remove(); });
    points.forEach(function (q, i) {
      var m = document.createElement("div");
      m.className = "pt"; m.dataset.i = i;
      m.style.left = ((q[0] + 0.5) / W * 100) + "%";
      m.style.top = ((q[1] + 0.5) / H * 100) + "%";
      m.title = "Point " + (i + 1) + "（" + Math.round(q[0] * scale) + ", " + Math.round(q[1] * scale) + "）";
      m.textContent = i + 1;
      pic.append(m);
    });
    countEl.textContent = points.length;
  }
  function at(ev) {
    var r = canvas.getBoundingClientRect();
    return [
      Math.min(W - 1, Math.max(0, Math.floor((ev.clientX - r.left) / r.width * W))),
      Math.min(H - 1, Math.max(0, Math.floor((ev.clientY - r.top) / r.height * H)))
    ];
  }
  var drag = -1, lastDown = { i: -1, t: 0 };
  pic.addEventListener("pointerdown", function (ev) {
    if (ev.button !== 0) return;
    var m = ev.target.closest(".pt");
    if (m) {
      var i = +m.dataset.i, now = performance.now();
      // 同じ点を続けて2回押したら外す（プラグインと同じく、最低1つは残す）
      if (lastDown.i === i && now - lastDown.t < 400 && points.length > 1) {
        lastDown = { i: -1, t: 0 };
        points.splice(i, 1);
        renderPoints(); schedule();
        return;
      }
      lastDown = { i: i, t: now };
      drag = i;
    } else {
      if (points.length >= MAX_POINTS) { msEl.textContent = "点は 99 個までです"; return; }
      points.push(at(ev));
      drag = points.length - 1;
    }
    pic.setPointerCapture(ev.pointerId);
    renderPoints();
    pic.querySelectorAll(".pt")[drag].classList.add("drag");
    schedule();
  });
  pic.addEventListener("pointermove", function (ev) {
    if (drag < 0) return;
    points[drag] = at(ev);
    moved = true;
    var m = pic.querySelectorAll(".pt")[drag];
    m.style.left = ((points[drag][0] + 0.5) / W * 100) + "%";
    m.style.top = ((points[drag][1] + 0.5) / H * 100) + "%";
    schedule();
  });
  var moved = false;
  function endDrag() {
    if (drag < 0) return;
    drag = -1;
    if (moved) lastDown = { i: -1, t: 0 };
    moved = false;
    renderPoints();
  }
  pic.addEventListener("pointerup", endDrag);
  pic.addEventListener("pointercancel", endDrag);
  document.getElementById("onept").addEventListener("click", function () {
    points = points.slice(0, 1); renderPoints(); schedule();
  });
  var hidePts = document.getElementById("hidepts");
  hidePts.addEventListener("click", function () {
    var hide = pic.classList.toggle("hide");
    hidePts.textContent = hide ? "点を表示" : "点を隠す";
  });

  // ---- 表示 ----
  var viewer = document.getElementById("viewer"), zoom = 1;
  function applyZoom() {
    viewer.classList.toggle("zoomed", zoom > 1);
    canvas.style.width = zoom > 1 ? (W * zoom) + "px" : "";
  }
  document.getElementById("zoom").addEventListener("click", function (ev) {
    var b = ev.target.closest("button"); if (!b) return;
    this.querySelectorAll("button").forEach(function (x) { x.classList.toggle("on", x === b); });
    zoom = +b.dataset.v;
    applyZoom();
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

  // ---- After Effects へコピー ----
  // Ae のキーフレームデータ（キーフレームをコピーしたときのテキスト）の形で書く。
  // 番号はプラグインのパラメータの並び（fillerase/src/plugin.rs の params_setup の順）：
  // Point Count、Point 1〜99、Tolerance、Softness、Choke、Contiguous、Compare Alpha、Invert。
  var AE_EFFECT = "JK Fill Erase";
  function aeIndex(name) {
    var m = /^Point (\d+)$/.exec(name);
    if (m) return 1 + +m[1];
    return { "Point Count": 1, "Tolerance": 101, "Softness": 102, "Choke": 103,
      "Contiguous": 104, "Compare Alpha": 105, "Invert": 106 }[name];
  }
  function aeText() {
    var v = [["Point Count", points.length]];
    points.forEach(function (q, i) { v.push(["Point " + (i + 1), [(q[0] + 0.5) * scale, (q[1] + 0.5) * scale]]); });
    v.push(["Tolerance", val("tolerance")], ["Softness", val("softness")], ["Choke", val("choke") * scale],
      ["Contiguous", val("contiguous")], ["Compare Alpha", val("compare_alpha")], ["Invert", val("invert")]);
    var out = ["Adobe After Effects 8.0 Keyframe Data", "",
      "\tUnits Per Second\t30", "\tSource Width\t" + Math.round(W * scale), "\tSource Height\t" + Math.round(H * scale),
      "\tSource Pixel Aspect Ratio\t1", "\tComp Pixel Aspect Ratio\t1", ""];
    v.forEach(function (p) {
      out.push("Effects\t" + AE_EFFECT + " #1\t" + p[0] + " #" + aeIndex(p[0]));
      if (Array.isArray(p[1])) {
        out.push("\tFrame\tX pixels\tY pixels\t", "\t0\t" + p[1].map(function (n) { return +n.toFixed(3); }).join("\t") + "\t");
      } else {
        out.push("\tFrame\t\t", "\t0\t" + (+p[1].toFixed(4)) + "\t");
      }
      out.push("");
    });
    out.push("", "End of Keyframe Data", "");
    return out.join("\r\n");
  }

  var aeCopy = document.getElementById("aecopy"), aeArea = document.getElementById("aetext");
  aeCopy.addEventListener("click", function () {
    var text = aeText();
    var done = function () {
      aeCopy.textContent = "コピーしました ✓"; aeCopy.classList.add("done"); aeArea.hidden = true;
      setTimeout(function () { aeCopy.textContent = "Ae 用にコピー"; aeCopy.classList.remove("done"); }, 1600);
    };
    var fail = function () {
      // クリップボードに書けないとき：選んだ状態のテキストを出して、自分でコピーしてもらう
      aeArea.hidden = false; aeArea.value = text; aeArea.focus(); aeArea.select();
      aeCopy.textContent = "下のテキストをコピーしてください";
    };
    if (navigator.clipboard && navigator.clipboard.writeText) navigator.clipboard.writeText(text).then(done, fail);
    else fail();
  });

  // ---- 起動 ----
  useSample();
  fetch("fillerase_web.wasm")
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
