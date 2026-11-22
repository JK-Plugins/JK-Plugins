// 配布サイトの動き。site.py が index.html に埋め込む
// - 検索（名前・説明・フォルダー名で絞り込み、/ キー、?q= で共有）
// - ポスターがスクロールで下から出てくる。カーソルの方へほんの少し傾く
// - 表紙の文字は、スクロールで少し遅れて動き、カーソルの位置で少しずれる
// - 背景は無地。カーソルの位置に大きな柔らかい光が少し遅れてついてくる（カーソル自体はふつうの矢印）
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

  /* ---------- カーソルの位置：背景の光・表紙の文字 ---------- */
  const glow = document.querySelector('.glow');
  let mx = innerWidth / 2, my = innerHeight / 3, gx = mx, gy = my, moving = false;
  addEventListener('pointermove', ev => {
    mx = ev.clientX; my = ev.clientY;
    if (glow) glow.classList.add('on');
    if (!moving) { moving = true; requestAnimationFrame(follow); }
  }, { passive: true });
  document.addEventListener('pointerleave', () => { if (glow) glow.classList.remove('on'); });
  function follow() {
    gx += (mx - gx) * 0.12; gy += (my - gy) * 0.12;
    if (glow) { glow.style.setProperty('--gx', gx + 'px'); glow.style.setProperty('--gy', gy + 'px'); }
    // 表紙の文字は画面の真ん中からのずれに合わせて最大 8px ほど逆へ
    hx = (0.5 - gx / innerWidth) * 16; hy = (0.5 - gy / innerHeight) * 10;
    place();
    if (Math.abs(mx - gx) > 0.5 || Math.abs(my - gy) > 0.5) requestAnimationFrame(follow); else moving = false;
  }

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
