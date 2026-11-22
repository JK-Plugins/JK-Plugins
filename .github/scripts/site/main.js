// 配布サイトの動き。site.py が index.html に埋め込む
// - 検索（名前・説明・フォルダー名で絞り込み、/ キー、?q= で共有）
// - 見出しをドットで描く（明朝・セリフ体を粗い升目に落として、ふちはディザ）。カーソルの近くは升目が粗くなる
// - カーソル（四角とドットの軌跡）、スクロールで出てくる・流れる・見出しが崩れる
(() => {
  const root = document.documentElement;
  const RM = matchMedia('(prefers-reduced-motion: reduce)').matches;
  const FINE = matchMedia('(hover: hover) and (pointer: fine)').matches;
  if (!RM) root.classList.add('fx');
  const mouse = { x: -1e4, y: -1e4, on: false };
  let dirty = true;

  /* ---------- 検索 ---------- */
  const q = document.getElementById('q'), empty = document.getElementById('empty'), count = document.getElementById('count');
  const items = [...document.querySelectorAll('[data-q]')], cols = [...document.querySelectorAll('.col')];
  const norm = s => s.normalize('NFKC').toLowerCase();
  items.forEach(el => { el._q = norm(el.dataset.q); });
  const runSearch = () => {
    const terms = norm(q.value).split(/\s+/).filter(Boolean);
    let total = 0;
    for (const c of cols) {
      let n = 0;
      for (const el of c.querySelectorAll('[data-q]')) {
        const ok = terms.every(t => el._q.includes(t));
        el.hidden = !ok;
        if (ok) { n++; el.classList.add('in'); }
      }
      c.hidden = terms.length > 0 && n === 0;
      const j = document.querySelector('.jump a[href="#' + c.id + '"] span');
      if (j) j.textContent = n;
      total += n;
    }
    count.textContent = String(total).padStart(3, '0');
    empty.hidden = total > 0;
    const u = new URL(location.href);
    if (q.value) u.searchParams.set('q', q.value); else u.searchParams.delete('q');
    history.replaceState(null, '', u);
    dirty = true;
  };
  q.addEventListener('input', runSearch);
  q.addEventListener('keydown', ev => { if (ev.key === 'Escape') { q.value = ''; runSearch(); q.blur(); } });
  document.addEventListener('keydown', ev => {
    if (ev.key === '/' && document.activeElement !== q && !ev.ctrlKey && !ev.metaKey) { ev.preventDefault(); q.focus(); }
  });
  const init = new URLSearchParams(location.search).get('q');
  if (init) { q.value = init; runSearch(); }

  /* ---------- ドットで描く見出し ---------- */
  const BAYER = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5].map(v => (v + 0.5) / 16);
  const accent = () => getComputedStyle(root).getPropertyValue('--accent').trim();
  class Pixel {
    constructor(el) {
      this.el = el;
      this.txt = el.textContent.trim();
      this.cv = document.createElement('canvas');
      this.cv.setAttribute('aria-hidden', 'true');
      this.ctx = this.cv.getContext('2d');
      this.sm = document.createElement('canvas');
      this.sc = this.sm.getContext('2d', { willReadFrequently: true });
      this.k = RM ? 1 : 7;          // 升目の倍率（出てくるときは粗いところから）
      this.scrollK = 1;             // スクロールで粗くする分（ヒーローだけ）
      this.visible = false;
      this.revealed = RM;
      el.appendChild(this.cv);
      el.classList.add('px-on');
      this.layout();
    }
    layout() {
      const cs = getComputedStyle(this.el);
      this.color = cs.color;
      this.accent = accent();
      const w = this.el.clientWidth;
      this.font = s => `${cs.fontStyle} ${cs.fontWeight} ${s}px ${cs.fontFamily}`;
      const sc = this.sc;
      sc.font = this.font(100);
      const m = sc.measureText(this.txt);
      const perPx = m.width / 100;
      let fs = parseFloat(cs.fontSize);
      if (this.el.hasAttribute('data-fit')) fs = (w / perPx) * 0.995; else fs = Math.min(fs, (w / perPx) * 0.995);
      this.fs = fs;
      const asc = (m.actualBoundingBoxAscent || 72) / 100 * fs, desc = (m.actualBoundingBoxDescent || 22) / 100 * fs;
      const pad = Math.ceil(fs * 0.06);
      this.W = w;
      this.H = Math.ceil(asc + desc + pad * 2);
      this.base = asc + pad;
      this.tw = perPx * fs;
      this.x0 = cs.textAlign === 'center' ? (w - this.tw) / 2 : cs.textAlign === 'right' || cs.textAlign === 'end' ? w - this.tw : 0;
      this.cell = Math.max(3, Math.round(fs / 30));
      const dpr = Math.min(2, devicePixelRatio || 1);
      this.dpr = dpr;
      this.cv.width = Math.round(w * dpr);
      this.cv.height = Math.round(this.H * dpr);
      this.cv.style.height = this.H + 'px';
      this.key = '';
      this.draw();
    }
    mask(c) {
      const cols = Math.ceil(this.W / c), rows = Math.ceil(this.H / c), key = c + ':' + cols + ':' + rows;
      if (key === this.key) return;
      this.key = key; this.cols = cols; this.rows = rows;
      const sm = this.sm, sc = this.sc;
      sm.width = cols; sm.height = rows;
      sc.clearRect(0, 0, cols, rows);
      sc.font = this.font(this.fs / c);
      sc.textBaseline = 'alphabetic';
      sc.fillStyle = '#000';
      sc.fillText(this.txt, this.x0 / c, this.base / c);
      const d = sc.getImageData(0, 0, cols, rows).data, a = new Float32Array(cols * rows);
      for (let i = 0; i < a.length; i++) a[i] = d[i * 4 + 3] / 255;
      this.a = a;
    }
    draw() {
      const c = Math.max(2, Math.round(this.cell * this.k * this.scrollK));
      this.mask(c);
      const { ctx, cols, rows, a } = this;
      ctx.setTransform(this.dpr, 0, 0, this.dpr, 0, 0);
      ctx.clearRect(0, 0, this.W, this.H);
      const r = this.el.getBoundingClientRect();
      const mx = mouse.x - r.left, my = mouse.y - r.top, R = Math.max(60, this.fs * 0.55), R2 = R * R;
      const near = mouse.on && mx > -R && mx < this.W + R && my > -R && my < this.H + R;
      const gap = c >= 6 ? 1 : 0, s = c - gap;
      const lens = [];
      ctx.fillStyle = this.color;
      for (let y = 0; y < rows; y++) {
        for (let x = 0; x < cols; x++) {
          let v = a[y * cols + x], hot = false;
          if (near) {
            const dx = x * c + c / 2 - mx, dy = y * c + c / 2 - my, d2 = dx * dx + dy * dy;
            if (d2 < R2) {
              // カーソルの近くは 2〜4 倍の升目でまとめて読む（レンズのように粗くなる）
              const m = d2 < R2 * 0.2 ? 4 : d2 < R2 * 0.55 ? 3 : 2;
              const sx = Math.min(cols - 1, Math.floor(x / m) * m + (m >> 1)), sy = Math.min(rows - 1, Math.floor(y / m) * m + (m >> 1));
              v = a[sy * cols + sx];
              hot = true;
            }
          }
          if (v > BAYER[(y & 3) * 4 + (x & 3)]) {
            if (hot) lens.push(x, y); else ctx.fillRect(x * c, y * c, s, s);
          }
        }
      }
      if (lens.length) {
        ctx.fillStyle = this.accent;
        for (let i = 0; i < lens.length; i += 2) ctx.fillRect(lens[i] * c, lens[i + 1] * c, s, s);
      }
      this.wasNear = near;
    }
    reveal() {
      if (this.revealed) return;
      this.revealed = true;
      const t0 = performance.now(), steps = [7, 5, 4, 3, 2, 1.5, 1];
      const tick = now => {
        const i = Math.max(0, Math.min(steps.length - 1, Math.floor((now - t0) / 90)));
        this.k = steps[i];
        this.draw();
        if (i < steps.length - 1) requestAnimationFrame(tick);
      };
      requestAnimationFrame(tick);
    }
  }

  let pixels = [];
  const pxEls = [...document.querySelectorAll('.px')];
  const fontsReady = Promise.race([
    Promise.all(pxEls.map(el => {
      const cs = getComputedStyle(el);
      return document.fonts.load(`${cs.fontStyle} ${cs.fontWeight} 64px ${cs.fontFamily}`, el.textContent);
    })),
    new Promise(r => setTimeout(r, 2500)),
  ]);
  fontsReady.then(() => {
    pixels = pxEls.map(el => new Pixel(el));
    const io = new IntersectionObserver(es => es.forEach(en => {
      const p = pixels.find(p => p.el === en.target);
      p.visible = en.isIntersecting;
      if (en.isIntersecting) p.reveal();
    }), { threshold: 0.15 });
    pixels.forEach(p => io.observe(p.el));
    dirty = true;
  });
  let rt;
  addEventListener('resize', () => { clearTimeout(rt); rt = setTimeout(() => pixels.forEach(p => p.layout()), 120); });
  matchMedia('(prefers-color-scheme: dark)').addEventListener('change', () => pixels.forEach(p => p.layout()));

  /* ---------- 出てくる ---------- */
  if (!RM) {
    const rv = [...document.querySelectorAll('.rv')];
    const io = new IntersectionObserver(es => {
      let i = 0;
      for (const en of es) {
        if (!en.isIntersecting) continue;
        en.target.style.setProperty('--d', (i++ % 6) * 0.05 + 's');
        en.target.classList.add('in');
        io.unobserve(en.target);
      }
    }, { rootMargin: '0px 0px -8% 0px' });
    rv.forEach(el => io.observe(el));
  }

  /* ---------- カーソル ---------- */
  let cur, label, trail, tctx, cells = new Map();
  const TC = 10;
  if (FINE && !RM) {
    root.classList.add('fx-cursor');
    cur = document.createElement('div'); cur.className = 'cur'; cur.setAttribute('aria-hidden', 'true');
    label = document.createElement('b'); cur.appendChild(label);
    trail = document.createElement('canvas'); trail.className = 'trail'; trail.setAttribute('aria-hidden', 'true');
    document.body.append(trail, cur);
    tctx = trail.getContext('2d');
    const size = () => { const d = Math.min(2, devicePixelRatio || 1); trail.width = innerWidth * d; trail.height = innerHeight * d; tctx.setTransform(d, 0, 0, d, 0, 0); };
    size(); addEventListener('resize', size);
    let lx = null, ly = null;
    addEventListener('pointermove', ev => {
      mouse.x = ev.clientX; mouse.y = ev.clientY; mouse.on = true; dirty = true;
      // 通ったところの升目に印をつける（あとで消えていく）
      if (lx !== null) {
        const n = Math.ceil(Math.hypot(ev.clientX - lx, ev.clientY - ly) / TC);
        for (let i = 1; i <= n; i++) {
          const x = Math.floor((lx + (ev.clientX - lx) * i / n) / TC), y = Math.floor((ly + (ev.clientY - ly) * i / n) / TC);
          cells.set(x + ',' + y, 1);
        }
      }
      lx = ev.clientX; ly = ev.clientY;
    }, { passive: true });
    document.addEventListener('pointerleave', () => { mouse.on = false; cur.style.opacity = 0; dirty = true; });
    document.addEventListener('pointerenter', () => { cur.style.opacity = 1; });
    document.addEventListener('pointerover', ev => {
      const t = ev.target.closest('a, button, label, input');
      cur.classList.toggle('text', !!t && t.tagName === 'INPUT');
      cur.classList.toggle('hov', !!t && t.tagName !== 'INPUT');
      label.textContent = t ? (t.dataset.cur || '') : '';
    });
  } else {
    addEventListener('pointermove', ev => { mouse.x = ev.clientX; mouse.y = ev.clientY; mouse.on = ev.pointerType === 'mouse'; dirty = true; }, { passive: true });
  }

  /* ---------- スクロール ---------- */
  const track = document.querySelector('.mq-track'), readout = document.querySelector('.readout b');
  const hero = document.querySelector('.hero');
  let mqX = 0, vel = 0, lastY = scrollY, dir = 1;
  addEventListener('scroll', () => { dirty = true; }, { passive: true });

  let cx = -100, cy = -100, prev = performance.now();
  const frame = now => {
    const dt = Math.min(50, now - prev); prev = now;
    // スクロールの速さ
    const y = scrollY, dy = y - lastY; lastY = y;
    if (dy) dir = dy > 0 ? 1 : -1;
    vel = vel * 0.9 + dy * 0.1;
    // 流れるプラグイン名：ふだんはゆっくり、スクロールすると速く、向きも変わる
    if (track && !RM) {
      const half = track.scrollWidth / 2;
      mqX -= (0.04 * dt) * dir + vel * 0.6;
      if (half) mqX = ((mqX % half) - half) % half;
      track.style.transform = `translate3d(${mqX}px,0,0)`;
    }
    const max = document.documentElement.scrollHeight - innerHeight;
    if (readout) readout.textContent = String(Math.round(max > 0 ? y / max * 100 : 0)).padStart(3, '0');
    // ヒーローの見出しはスクロールで升目が粗くなる
    if (hero && pixels[0] && !RM) {
      const k = 1 + Math.min(1, Math.max(0, y / hero.offsetHeight)) * 3;
      if (Math.abs(k - pixels[0].scrollK) > 0.02) { pixels[0].scrollK = k; dirty = true; }
    }
    if (dirty) {
      for (const p of pixels) if (p.visible && p.revealed && p.k === 1) {
        const r = p.el.getBoundingClientRect(), R = Math.max(60, p.fs * 0.55);
        const near = mouse.on && mouse.x > r.left - R && mouse.x < r.right + R && mouse.y > r.top - R && mouse.y < r.bottom + R;
        if (near || p.wasNear || p === pixels[0]) p.draw();
      }
      dirty = false;
    }
    if (cur) {
      cx += (mouse.x - cx) * 0.35; cy += (mouse.y - cy) * 0.35;
      cur.style.transform = `translate3d(${cx}px,${cy}px,0)`;
      tctx.clearRect(0, 0, innerWidth, innerHeight);
      if (cells.size) {
        tctx.fillStyle = accent();
        for (const [k, life] of cells) {
          const nl = life - dt / 420;
          if (nl <= 0) { cells.delete(k); continue; }
          cells.set(k, nl);
          const [x, yy] = k.split(',');
          tctx.globalAlpha = nl * 0.85;
          const s = Math.max(2, Math.round(TC * nl));
          tctx.fillRect(x * TC + (TC - s) / 2, yy * TC + (TC - s) / 2, s, s);
        }
        tctx.globalAlpha = 1;
      }
    }
    requestAnimationFrame(frame);
  };
  requestAnimationFrame(frame);
})();
