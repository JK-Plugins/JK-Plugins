// 配布サイトの動き。site.py が index.html に埋め込む
// - 検索（名前・説明・フォルダー名で絞り込み、/ キー、?q= で共有）
// - ポスターがスクロールで下から出てくる。カーソルの方へ少し傾いて、光が当たる
// - 表紙の文字と背景の大きな文字は、スクロールで違う速さで動く
// - カーソルは丸。リンクの上では大きくなって「README」「↓ 180 KB」などを出す
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
  const hero = document.querySelector('.hero'), h1 = hero && hero.querySelector('h1'), ghost = hero && hero.querySelector('.ghost');
  let ticking = false;
  const onScroll = () => {
    ticking = false;
    if (!hero) return;
    const y = Math.min(scrollY, hero.offsetHeight * 1.2);
    if (h1) h1.style.transform = `translate3d(0, ${y * 0.18}px, 0)`;
    if (ghost) ghost.style.transform = `translate3d(${-y * 0.12}px, ${-y * 0.22}px, 0)`;
  };
  addEventListener('scroll', () => { if (!ticking) { ticking = true; requestAnimationFrame(onScroll); } }, { passive: true });

  if (!FINE) return;

  /* ---------- ポスターがカーソルの方へ傾く ---------- */
  document.querySelectorAll('.poster').forEach(el => {
    el.addEventListener('pointermove', ev => {
      const r = el.getBoundingClientRect(), x = (ev.clientX - r.left) / r.width, y = (ev.clientY - r.top) / r.height;
      const k = el.classList.contains('feature') ? 3 : 6;
      el.classList.add('tilting');
      el.style.transform = `perspective(900px) rotateX(${(0.5 - y) * k}deg) rotateY(${(x - 0.5) * k}deg) translateY(-4px)`;
      el.style.setProperty('--mx', x * 100 + '%');
      el.style.setProperty('--my', y * 100 + '%');
    });
    el.addEventListener('pointerleave', () => { el.classList.remove('tilting'); el.style.transform = ''; });
  });

  /* ---------- カーソル ---------- */
  root.classList.add('fx-cursor');
  const cur = document.createElement('div'); cur.className = 'cur'; cur.setAttribute('aria-hidden', 'true');
  const label = document.createElement('b'); cur.appendChild(label);
  document.body.appendChild(cur);
  let mx = -100, my = -100, cx = -100, cy = -100;
  addEventListener('pointermove', ev => { mx = ev.clientX; my = ev.clientY; }, { passive: true });
  document.addEventListener('pointerleave', () => { cur.style.opacity = 0; });
  document.addEventListener('pointerenter', () => { cur.style.opacity = 1; });
  document.addEventListener('pointerover', ev => {
    const t = ev.target.closest('a, button, label');
    const text = t && t.dataset.cur;
    cur.classList.toggle('hov', !!text);
    label.textContent = text || '';
  });
  const frame = () => {
    cx += (mx - cx) * 0.3; cy += (my - cy) * 0.3;
    cur.style.transform = `translate3d(${cx}px, ${cy}px, 0)`;
    requestAnimationFrame(frame);
  };
  requestAnimationFrame(frame);
})();
