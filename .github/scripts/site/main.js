// 配布サイトの動き。site.py が index.html に埋め込む
// - 検索（名前・説明・フォルダー名で絞り込み、/ キー、?q= で共有）
// - ポスターがスクロールで下から出てくる。カーソルの方へほんの少し傾く
// - 表紙の文字は、スクロールで少し遅れて動く
// - 背景：灰色のドットの網がゆっくり波打つ。カーソルの近くは粒が少し大きくなる
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

  /* ---------- 背景：ドットの網 ---------- */
  const bg = document.querySelector('.bgfx');
  const pointer = { x: -1e4, y: -1e4 };
  if (bg) {
    const g = bg.getContext('2d');
    let W = 0, H = 0, dpr = 1;
    const STEP = 22;
    const size = () => {
      dpr = Math.min(2, devicePixelRatio || 1); W = innerWidth; H = innerHeight;
      bg.width = W * dpr; bg.height = H * dpr; g.setTransform(dpr, 0, 0, dpr, 0, 0);
    };
    size(); addEventListener('resize', size);
    const dark = matchMedia('(prefers-color-scheme: dark)');
    let last = 0, visible = true;
    document.addEventListener('visibilitychange', () => { visible = !document.hidden; if (visible) requestAnimationFrame(draw); });
    function draw(now) {
      if (!visible) return;
      if (!RM) requestAnimationFrame(draw);
      if (now - last < 33) return;          // 30fps で十分
      last = now;
      const t = RM ? 0 : now / 1000, sy = scrollY * 0.15;
      g.clearRect(0, 0, W, H);
      g.globalAlpha = dark.matches ? 0.16 : 0.12;
      g.fillStyle = dark.matches ? '#f1f0ec' : '#111';
      const oy = -(sy % STEP);
      for (let y = oy; y < H + STEP; y += STEP) {
        const wy = y + sy;
        for (let x = STEP / 2; x < W; x += STEP) {
          // ゆっくり流れる 2 つの波で粒の大きさを変える
          let r = 0.5 + 0.5 * Math.sin(x * 0.011 + t * 0.35) * Math.cos(wy * 0.009 - t * 0.25);
          r = 0.6 + r * r * 2.2;
          const dx = x - pointer.x, dy = y - pointer.y, d = Math.hypot(dx, dy);
          if (d < 160) r += (1 - d / 160) * 2.4;
          g.beginPath(); g.arc(x, y, Math.min(r, STEP * 0.45), 0, 6.283); g.fill();
        }
      }
      g.globalAlpha = 1;
    }
    requestAnimationFrame(draw);
    addEventListener('pointermove', ev => { pointer.x = ev.clientX; pointer.y = ev.clientY; }, { passive: true });
    if (RM) addEventListener('scroll', () => requestAnimationFrame(draw), { passive: true });
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
  let ticking = false;
  const onScroll = () => {
    ticking = false;
    if (!hero) return;
    const y = Math.min(scrollY, hero.offsetHeight * 1.2);
    if (h1) h1.style.transform = `translate3d(0, ${y * 0.18}px, 0)`;
  };
  addEventListener('scroll', () => { if (!ticking) { ticking = true; requestAnimationFrame(onScroll); } }, { passive: true });

  if (!FINE) return;

  /* ---------- ポスターがカーソルの方へ傾く ---------- */
  document.querySelectorAll('.poster').forEach(el => {
    el.addEventListener('pointermove', ev => {
      const r = el.getBoundingClientRect(), x = (ev.clientX - r.left) / r.width, y = (ev.clientY - r.top) / r.height;
      const k = el.classList.contains('feature') ? 1 : 2.5;
      el.classList.add('tilting');
      el.style.transform = `perspective(900px) rotateX(${(0.5 - y) * k}deg) rotateY(${(x - 0.5) * k}deg) translateY(-2px)`;
    });
    el.addEventListener('pointerleave', () => { el.classList.remove('tilting'); el.style.transform = ''; });
  });

})();
