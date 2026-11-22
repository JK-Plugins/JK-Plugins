// 配布サイトの動き。site.py が index.html に埋め込む
// - 検索（名前・説明・フォルダー名で絞り込み、/ キー、?q= で共有）
// - ポスターがスクロールで下から出てくる。カーソルの方へ少し傾いて、光が当たる（大きいのも小さいのも同じ）
// - 表紙の文字は、スクロールで少し遅れて動き、カーソルの位置で少しずれる
// - 背景：カーソルの通ったところに ASCII の文字の粒（@ # * + : .）が散り、重力で落ちながら薄くなって消える（カーソル自体はふつうの矢印）
// - 説明の改行を文節で（word-break: auto-phrase が無いブラウザは BudouX を読み込んで <wbr> を入れる）
(() => {
  const root = document.documentElement;
  const RM = matchMedia('(prefers-reduced-motion: reduce)').matches;
  const FINE = matchMedia('(hover: hover) and (pointer: fine)').matches;
  if (!RM) root.classList.add('fx');

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
      if (j) j.textContent = '(' + n + ')';
      total += n;
    }
    count.textContent = '(' + total + ')';
    empty.hidden = total > 0;
    const u = new URL(location.href);
    if (q.value) u.searchParams.set('q', q.value); else u.searchParams.delete('q');
    history.replaceState(null, '', u);
  };
  q.addEventListener('input', runSearch);
  q.addEventListener('keydown', ev => { if (ev.key === 'Escape') { q.value = ''; runSearch(); q.blur(); } });
  document.addEventListener('keydown', ev => {
    if (ev.key === '/' && document.activeElement !== q && !ev.ctrlKey && !ev.metaKey) { ev.preventDefault(); q.focus(); }
  });
  const init = new URLSearchParams(location.search).get('q');
  if (init) { q.value = init; runSearch(); }

  /* ---------- 文節で改行（Chrome 以外） ---------- */
  if (!(window.CSS && CSS.supports('word-break', 'auto-phrase'))) {
    import('https://cdn.jsdelivr.net/npm/budoux@0.10.3/+esm').then(m => {
      const parser = m.loadDefaultJapaneseParser();
      document.querySelectorAll('.cap').forEach(el => parser.applyToElement(el));
    }).catch(() => {});
  }

  if (RM) return;

  /* ---------- 出てくる（同じ行のものは少しずつずらす） ---------- */
  const io = new IntersectionObserver(es => {
    let i = 0;
    for (const en of es) {
      if (!en.isIntersecting) continue;
      en.target.style.setProperty('--d', (i++ % 4) * 0.07 + 's');
      en.target.classList.add('in');
      io.unobserve(en.target);
    }
  }, { rootMargin: '0px 0px -6% 0px' });
  document.querySelectorAll('.rv').forEach(el => io.observe(el));

  /* ---------- 表紙のパララックス ---------- */
  const hero = document.querySelector('.hero'), h1 = hero && hero.querySelector('h1');
  let ticking = false, hx = 0, hy = 0;
  const place = () => {
    if (!hero || !h1) return;
    const y = Math.min(scrollY, hero.offsetHeight * 1.2);
    h1.style.transform = `translate3d(${hx}px, ${y * 0.18 + hy}px, 0)`;
  };
  addEventListener('scroll', () => { if (!ticking) { ticking = true; requestAnimationFrame(() => { ticking = false; place(); }); } }, { passive: true });

  if (!FINE) return;

  /* ---------- カーソルの位置：表紙の文字が少しずれる ---------- */
  let mx = innerWidth / 2, my = innerHeight / 3, gx = mx, gy = my, moving = false;
  function follow() {
    gx += (mx - gx) * 0.12; gy += (my - gy) * 0.12;
    hx = (0.5 - gx / innerWidth) * 16; hy = (0.5 - gy / innerHeight) * 10;
    place();
    if (Math.abs(mx - gx) > 0.5 || Math.abs(my - gy) > 0.5) requestAnimationFrame(follow); else moving = false;
  }

  /* ---------- 背景：ASCII の粒の軌跡 ---------- */
  const cv = document.querySelector('.trail'), g = cv && cv.getContext('2d');
  const RAMP = '@#%*+=-:.';               // 古くなるほど右の（薄い）文字へ
  const parts = [], MAX = 420, FS = 14;
  let W = 0, H = 0, lx = null, ly = null, running = false, ink = '#111';
  const size = () => {
    const d = Math.min(2, devicePixelRatio || 1); W = innerWidth; H = innerHeight;
    cv.width = W * d; cv.height = H * d; g.setTransform(d, 0, 0, d, 0, 0);
    ink = matchMedia('(prefers-color-scheme: dark)').matches ? '#f1f0ec' : '#111';
  };
  if (cv) { size(); addEventListener('resize', size); matchMedia('(prefers-color-scheme: dark)').addEventListener('change', size); }
  function spawn(x, y, vx, vy) {
    if (parts.length >= MAX) parts.shift();
    // 升目にそろえて置く（文字の画面のように）
    parts.push({ x: Math.round(x / 9) * 9, y: Math.round(y / FS) * FS, vx: vx * 0.04 + (Math.random() - 0.5) * 0.6,
      vy: vy * 0.03 - 0.6 - Math.random() * 1.6, life: 1, decay: 0.006 + Math.random() * 0.01 });
  }
  function tick() {
    g.clearRect(0, 0, W, H);
    g.font = `${FS}px ui-monospace, SFMono-Regular, Menlo, Consolas, monospace`;
    g.textAlign = 'center'; g.textBaseline = 'middle'; g.fillStyle = ink;
    for (let i = parts.length - 1; i >= 0; i--) {
      const p = parts[i];
      p.life -= p.decay; if (p.life <= 0) { parts.splice(i, 1); continue; }
      // 重力：少し跳ねてから落ちていく
      p.vy += 0.09; p.vx *= 0.98; p.x += p.vx; p.y += p.vy;
      const ch = RAMP[Math.min(RAMP.length - 1, Math.floor((1 - p.life) * RAMP.length))];
      g.globalAlpha = Math.min(1, p.life * 1.3) * 0.7;
      g.fillText(ch, Math.round(p.x / 9) * 9, Math.round(p.y / FS) * FS);
    }
    g.globalAlpha = 1;
    if (parts.length) requestAnimationFrame(tick); else running = false;
  }

  addEventListener('pointermove', ev => {
    mx = ev.clientX; my = ev.clientY;
    if (!moving) { moving = true; requestAnimationFrame(follow); }
    if (!cv) return;
    if (lx !== null) {
      const dx = ev.clientX - lx, dy = ev.clientY - ly, n = Math.min(10, Math.ceil(Math.hypot(dx, dy) / 6));
      for (let i = 0; i < n; i++) spawn(lx + dx * (i + Math.random()) / n, ly + dy * (i + Math.random()) / n, dx, dy);
    }
    lx = ev.clientX; ly = ev.clientY;
    if (parts.length && !running) { running = true; requestAnimationFrame(tick); }
  }, { passive: true });
  document.addEventListener('pointerleave', () => { lx = ly = null; });

  /* ---------- ポスターがカーソルの方へ傾く ---------- */
  document.querySelectorAll('.poster').forEach(el => {
    el.addEventListener('pointermove', ev => {
      const r = el.getBoundingClientRect(), x = (ev.clientX - r.left) / r.width, y = (ev.clientY - r.top) / r.height;
      const k = 3.5;   // 大きいパネルも小さいパネルも同じ角度
      el.classList.add('tilting');
      el.style.transform = `perspective(900px) rotateX(${(0.5 - y) * k}deg) rotateY(${(x - 0.5) * k}deg) translateY(-3px)`;
      el.style.setProperty('--mx', x * 100 + '%');
      el.style.setProperty('--my', y * 100 + '%');
    });
    el.addEventListener('pointerleave', () => { el.classList.remove('tilting'); el.style.transform = ''; });
  });

})();
