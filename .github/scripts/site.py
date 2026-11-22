"""配布サイト（GitHub Pages）を作る。

  python .github/scripts/site.py --out _site

- 載せるリポジトリは COLLECTIONS（JK-Plugins と fs-rs）
- プラグインの一覧（名前・バージョン・説明・説明ページ）は各リポジトリの README の「プラグイン一覧」の表から読む
  （README-ja.md があればそれ、無ければ README.md。リリースの計画でバージョンが Cargo.toml と合っていることを確かめている）
- ダウンロードのリンクとファイルサイズは各リポジトリの最新のまとめリリース（gh release view）から作る。リリースがまだ無ければ「準備中」
- 画像の無いプラグインは詰めた一覧で並べる。上の検索欄で名前・説明から絞り込める
- 画像はこのリポジトリのプラグインのフォルダーの docs/samples/00_overview.png があれば使う
- web/<プラグインのフォルダー>/ があれば、そのプラグインをブラウザで試すページ（try/<フォルダー>/）も作る。
  web/<フォルダー>/site/ のファイルを置き、crate を wasm32-unknown-unknown でビルドした .wasm を添える
"""
import argparse
import base64
import os
import html
import json
import re
import shutil
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SELF = "JK-Plugins/JK-Plugins"
COLLECTIONS = [
    {"repo": "JK-Plugins/JK-Plugins", "title": "JK-Plugins", "lead": "オリジナル"},
    {"repo": "JK-Plugins/fs-rs", "title": "fs-rs",
     "lead": "F's Plugin（bryful 氏）の Rust 版"},
]
ROW = re.compile(
    r"^\|\s*\[(?P<name>[^\]]+)\]\((?P<dir>[^)]+)/README\.md\)\s*\|"
    r"\s*\[(?P<version>[^\]]+)\]\((?P<tag_url>[^)]+)\)\s*\|"
    r"\s*(?P<desc>.+?)\s*\|\s*$"
)


def gh(*args, check=True):
    return subprocess.run(["gh", *args], cwd=ROOT, check=check, capture_output=True, text=True, encoding="utf-8")


# 確認用：SITE_REFS="owner/repo=branch,..." で他のリポジトリの README をそのブランチから読む
REFS = dict(x.split("=", 1) for x in os.environ.get("SITE_REFS", "").split(",") if "=" in x)


def readme_text(repo):
    """README-ja.md があればそれ、無ければ README.md（このリポジトリは手元のファイル）"""
    ref = f"?ref={REFS[repo]}" if repo in REFS else ""
    for name in ["README-ja.md", "README.md"]:
        if repo == SELF:
            f = ROOT / name
            if f.exists():
                return f.read_text(encoding="utf-8")
        else:
            r = gh("api", f"repos/{repo}/contents/{name}{ref}", "-q", ".content", check=False)
            if r.returncode == 0 and r.stdout.strip():
                return base64.b64decode(r.stdout).decode("utf-8")
    raise SystemExit(f"{repo}: README が読めない")


def plugins_of(repo):
    out = []
    for line in readme_text(repo).splitlines():
        m = ROW.match(line.strip())
        if m:
            d = m.groupdict()
            d["archive_tag"] = d["tag_url"].rsplit("/", 1)[-1]
            d["stem"] = d["name"].replace(" ", "-")
            d["readme"] = f"https://github.com/{repo}/blob/main/{d['dir']}/README.md"
            d["download"] = f"https://github.com/{repo}/releases/download/{d['archive_tag']}"
            d["image"] = None
            d["try"] = None
            d["aex_size"] = d["zip_size"] = None
            d["date"] = release_date(repo, d["archive_tag"])
            out.append(d)
    if not out:
        # まだプラグイン一覧が無いリポジトリは載せない（サイト全体は作る）
        print(f"warning: {repo}: README にプラグイン一覧の行が見つからないので載せない")
    return out


def release_date(repo, tag):
    r = gh("release", "view", tag, "--repo", repo, "--json", "publishedAt", "-q", ".publishedAt", check=False)
    return r.stdout.strip()[:10] if r.returncode == 0 and r.stdout.strip() else None


def latest_release(repo):
    r = gh("release", "view", "--repo", repo, "--json", "tagName,publishedAt,url,assets", check=False)
    if r.returncode != 0:
        return None
    rel = json.loads(r.stdout)
    assets = {a["name"]: (a["url"], a.get("size")) for a in rel["assets"]}

    def find(suffix):
        return next((v for n, v in assets.items() if n.endswith(suffix)), None)

    return {
        "tag": rel["tagName"],
        "version": rel["tagName"].lstrip("v"),
        "date": rel["publishedAt"][:10],
        "url": rel["url"],
        "win_setup": find("-windows-setup.exe"),
        "win_zip": find("-windows.zip"),
        "mac_pkg": find("-macos.pkg"),
        # プラグインごとのファイル（<名前>.aex・<名前>.plugin.zip）のサイズ
        "sizes": {n: v[1] for n, v in assets.items()},
    }


def e(s):
    return html.escape(str(s), quote=True)


CSS = """
:root {
  --bg: #fafafa; --surface: #ffffff; --text: #0f0f12; --muted: #6e6e78; --line: rgba(15, 15, 18, .08);
  --line2: rgba(15, 15, 18, .14); --accent: #e0457b; --accent2: #7c5cff; --accent-soft: rgba(224, 69, 123, .10);
  --shadow: 0 1px 2px rgba(0,0,0,.04), 0 8px 24px rgba(0,0,0,.06);
  --glow1: rgba(224, 69, 123, .20); --glow2: rgba(124, 92, 255, .18); --bar: rgba(250, 250, 250, .82);
}
@media (prefers-color-scheme: dark) {
  :root:not([data-theme="light"]) {
    --bg: #0b0b0e; --surface: #141418; --text: #f4f4f6; --muted: #9b9ba6; --line: rgba(255, 255, 255, .08);
    --line2: rgba(255, 255, 255, .16); --accent: #ff6b9c; --accent2: #9b85ff; --accent-soft: rgba(255, 107, 156, .14);
    --shadow: 0 1px 2px rgba(0,0,0,.3), 0 12px 32px rgba(0,0,0,.35);
    --glow1: rgba(255, 107, 156, .16); --glow2: rgba(155, 133, 255, .14); --bar: rgba(11, 11, 14, .82);
  }
}
* { box-sizing: border-box; }
html { -webkit-text-size-adjust: 100%; scroll-behavior: smooth; scroll-padding-top: 84px; }
body {
  margin: 0; color: var(--text); min-height: 100vh;
  font: 15px/1.6 "Inter", system-ui, -apple-system, "Segoe UI", "Hiragino Sans", "Noto Sans JP", "Yu Gothic UI", sans-serif;
  background:
    radial-gradient(900px 420px at 12% -8%, var(--glow1), transparent 70%),
    radial-gradient(800px 380px at 92% -4%, var(--glow2), transparent 70%),
    var(--bg);
  background-repeat: no-repeat;
}
a { color: inherit; text-decoration: none; }
code { font: 12.5px/1.5 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; background: var(--accent-soft);
  padding: 1px 6px; border-radius: 6px; word-break: break-all; }
.wrap { max-width: 1120px; margin: 0 auto; padding: 0 20px; }
nav.top { display: flex; justify-content: space-between; align-items: center; padding: 20px 0; font-size: 14px; }
.logo { font-weight: 800; letter-spacing: -0.02em; font-size: 16px; }
.links { display: flex; gap: 18px; align-items: center; }
.links a { color: var(--muted); display: inline-flex; gap: 6px; align-items: center; }
.links a:hover { color: var(--text); }

.hero { padding: 40px 0 28px; text-align: center; }
.hero h1 {
  margin: 0; font-size: clamp(40px, 8vw, 72px); line-height: 1; letter-spacing: -0.04em; font-weight: 800;
  background: linear-gradient(100deg, var(--text) 30%, var(--accent) 70%, var(--accent2));
  -webkit-background-clip: text; background-clip: text; color: transparent;
}
.hero p { margin: 14px 0 0; color: var(--muted); font-size: 17px; }
.stats { display: flex; justify-content: center; gap: 8px; flex-wrap: wrap; margin-top: 18px; }
.stat { font-size: 13px; padding: 4px 12px; border-radius: 999px; background: var(--surface); border: 1px solid var(--line);
  color: var(--muted); font-variant-numeric: tabular-nums; }
.stat b { color: var(--text); font-weight: 700; margin-right: 4px; }

/* 検索バー（上に貼りつく） */
.bar { position: sticky; top: 0; z-index: 10; margin: 0 -20px; padding: 12px 20px;
  background: var(--bar); -webkit-backdrop-filter: blur(14px) saturate(1.4); backdrop-filter: blur(14px) saturate(1.4);
  border-bottom: 1px solid transparent; transition: border-color .2s; }
.bar.stuck { border-bottom-color: var(--line); }
.bar-in { display: flex; gap: 10px; align-items: center; flex-wrap: wrap; }
.search { position: relative; flex: 1 1 320px; }
.search svg { position: absolute; left: 14px; top: 50%; width: 17px; height: 17px; transform: translateY(-50%); color: var(--muted); pointer-events: none; }
.search input { width: 100%; font: inherit; color: inherit; padding: 10px 44px 10px 40px; border-radius: 12px; border: 0;
  background: var(--surface); box-shadow: inset 0 0 0 1px var(--line2), var(--shadow); outline: none; }
.search input:focus { box-shadow: inset 0 0 0 2px var(--accent), var(--shadow); }
.search input::-webkit-search-cancel-button { display: none; }
.search kbd { position: absolute; right: 12px; top: 50%; transform: translateY(-50%); font: 600 11px/1 ui-monospace, monospace;
  color: var(--muted); border: 1px solid var(--line2); border-radius: 5px; padding: 3px 6px; pointer-events: none; }
.search input:focus + kbd, .search input:not(:placeholder-shown) + kbd { display: none; }
.jump { display: flex; gap: 6px; flex-wrap: wrap; }
.jump a { font-size: 13px; font-weight: 600; padding: 7px 12px; border-radius: 999px; background: var(--surface);
  box-shadow: inset 0 0 0 1px var(--line); white-space: nowrap; }
.jump a:hover { box-shadow: inset 0 0 0 1px var(--text); }
.jump a span { color: var(--muted); font-weight: 400; margin-left: 4px; font-variant-numeric: tabular-nums; }
.empty { text-align: center; color: var(--muted); padding: 56px 0; }
.empty[hidden] { display: none; }

/* コレクション */
.col { margin: 40px 0 64px; }
.col[hidden] { display: none; }
.col-head { display: grid; grid-template-columns: minmax(0, 1fr) auto; align-items: center; gap: 20px 32px;
  padding: 28px 32px; border-radius: 24px; background: var(--surface); border: 1px solid var(--line); box-shadow: var(--shadow); }
.col-head h2 { margin: 0; font-size: 30px; line-height: 1.1; letter-spacing: -0.02em; font-weight: 800; }
.meta { color: var(--muted); font-size: 14px; margin-top: 6px; }
.chips { display: flex; flex-wrap: wrap; gap: 6px; margin-top: 12px; }
.chip { font-size: 12px; padding: 3px 10px; border-radius: 999px; border: 1px solid var(--line);
  color: var(--muted); white-space: nowrap; font-variant-numeric: tabular-nums; }
.chip b { color: var(--text); font-weight: 600; margin-left: 6px; }
.get { display: flex; flex-direction: column; align-items: flex-end; gap: 8px; }
.installers { display: flex; gap: 10px; flex-wrap: wrap; }
.big { display: inline-flex; align-items: center; gap: 12px; padding: 12px 24px 12px 18px; border-radius: 16px;
  color: #fff; background: linear-gradient(120deg, var(--accent), var(--accent2)); text-align: left;
  box-shadow: 0 10px 30px var(--glow1); transition: transform .15s, box-shadow .15s; min-width: 200px; }
.big:hover { transform: translateY(-2px); box-shadow: 0 14px 36px var(--glow1); }
.big svg { width: 24px; height: 24px; flex: none; }
.big b { display: block; font-size: 17px; line-height: 1.2; }
.big small { display: block; font-size: 12px; opacity: .85; font-variant-numeric: tabular-nums; }
.big > span { flex: 1; }
.big svg.dl { width: 18px; height: 18px; opacity: .9; margin-left: 4px; }
.more { color: var(--muted); font-size: 13px; }
.more:hover { color: var(--text); }
.soon { color: var(--muted); font-size: 13px; font-weight: 600; padding: 4px 12px; border-radius: 999px;
  border: 1px dashed var(--line2); white-space: nowrap; }
.ver { font-size: 12px; padding: 1px 9px; border-radius: 999px; background: var(--accent-soft); color: var(--accent);
  font-weight: 600; font-variant-numeric: tabular-nums; white-space: nowrap; }

/* 画像つきのカード */
.grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(min(100%, 320px), 1fr)); gap: 18px; margin-top: 20px; }
.tile {
  display: flex; flex-direction: column; background: var(--surface); border: 1px solid var(--line);
  border-radius: 20px; overflow: hidden; box-shadow: var(--shadow); transition: transform .2s, box-shadow .2s;
}
.tile:hover { transform: translateY(-3px); box-shadow: var(--shadow), 0 0 0 1px var(--accent-soft); }
.thumb { display: block; aspect-ratio: 16 / 10; overflow: hidden; border-bottom: 1px solid var(--line); background: #fff; }
.thumb img { width: 100%; height: 100%; object-fit: cover; object-position: top left; display: block; transition: transform .4s; }
.tile:hover .thumb img { transform: scale(1.03); }
.body { padding: 18px; display: flex; flex-direction: column; gap: 8px; flex: 1; }
.body h3 { margin: 0; font-size: 18px; letter-spacing: -0.01em; }
.vline { display: flex; align-items: center; gap: 8px; }
.date { color: var(--muted); font-size: 12px; font-variant-numeric: tabular-nums; }
.body p { margin: 0; color: var(--muted); font-size: 14px; flex: 1; }
.doc { font-size: 13px; color: var(--muted); }
.doc:hover { color: var(--text); }
.actions { display: flex; flex-wrap: wrap; gap: 8px; margin-top: 8px; align-items: center; }
.try { display: inline-flex; align-items: center; justify-content: center; gap: 8px; padding: 9px 16px; flex: 1 1 100%;
  border-radius: 999px; font-weight: 600; font-size: 14px; color: #fff;
  background: linear-gradient(120deg, var(--accent), var(--accent2)); box-shadow: 0 6px 18px var(--glow1); transition: transform .15s; }
.try:hover { transform: translateY(-1px); }
.try svg { width: 14px; height: 14px; }
.btn { display: inline-flex; align-items: center; justify-content: center; gap: 7px; padding: 8px 12px; border-radius: 999px;
  font-weight: 600; font-size: 13px; white-space: nowrap; box-shadow: inset 0 0 0 1px var(--line2); flex: 1; }
.btn:hover { box-shadow: inset 0 0 0 1px var(--text); }
.btn svg { width: 14px; height: 14px; flex: none; }
.btn small { font-size: 11px; font-weight: 600; color: var(--accent); }
.size { font-size: 11px; font-weight: 500; color: var(--muted); font-variant-numeric: tabular-nums; }
.btn svg.dl, .mini svg.dl { width: 13px; height: 13px; color: var(--accent); }
.mini:hover .size, .mini:hover svg.dl { color: inherit; }

/* 画像の無いプラグイン：詰めた一覧 */
.list { display: grid; grid-template-columns: repeat(auto-fill, minmax(min(100%, 340px), 1fr)); gap: 10px; margin-top: 20px; }
.item { display: flex; flex-direction: column; gap: 4px; padding: 14px 16px 12px; border-radius: 16px; background: var(--surface);
  border: 1px solid var(--line); transition: border-color .15s, box-shadow .15s; }
.item:hover { border-color: var(--line2); box-shadow: var(--shadow); }
.item[hidden], .tile[hidden] { display: none; }
.item h3 { margin: 0; font-size: 15px; font-weight: 700; letter-spacing: -0.01em; display: flex; align-items: baseline;
  gap: 8px; justify-content: space-between; }
.item h3 a:hover { color: var(--accent); }
.item p { margin: 0; color: var(--muted); font-size: 13px; line-height: 1.55; flex: 1; }
.item .row { display: flex; align-items: center; gap: 6px; margin-top: 8px; }
.item .doc { margin-right: auto; font-size: 12px; white-space: nowrap; }
.nw { white-space: nowrap; }
.mini { display: inline-flex; align-items: center; gap: 5px; padding: 4px 10px; border-radius: 999px; font-size: 12px;
  font-weight: 600; box-shadow: inset 0 0 0 1px var(--line2); white-space: nowrap; }
.mini:hover { background: var(--text); color: var(--bg); box-shadow: none; }
.mini svg { width: 12px; height: 12px; }
mark { background: var(--accent-soft); color: var(--accent); border-radius: 3px; padding: 0 1px; }

/* インストール */
.install { margin: 24px 0 64px; }
.install > h2 { font-size: 28px; letter-spacing: -0.02em; font-weight: 800; margin: 0 0 6px; }
.install > p { color: var(--muted); margin: 0 0 20px; }
.steps { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 420px), 1fr)); gap: 18px; }
.step { background: var(--surface); border: 1px solid var(--line); border-radius: 20px; padding: 22px 24px; box-shadow: var(--shadow); }
.step h3 { margin: 0 0 10px; display: flex; gap: 10px; align-items: center; font-size: 18px; }
.step h3 svg { width: 20px; height: 20px; }
.step ol { margin: 0; padding-left: 1.3em; display: flex; flex-direction: column; gap: 8px; font-size: 14px; }
.step li::marker { color: var(--accent); font-weight: 700; }
.step .sub { margin: 14px 0 0; padding-top: 12px; border-top: 1px solid var(--line); font-size: 13px; color: var(--muted); }
details { margin-top: 18px; background: var(--surface); border: 1px solid var(--line); border-radius: 16px; padding: 14px 20px; font-size: 14px; }
details summary { cursor: pointer; font-weight: 600; }
details ul { margin: 10px 0 0; padding-left: 1.2em; color: var(--muted); display: flex; flex-direction: column; gap: 6px; }

footer { margin-top: 24px; padding: 28px 0 48px; border-top: 1px solid var(--line); color: var(--muted); font-size: 13px;
  display: flex; gap: 20px; flex-wrap: wrap; }
footer a:hover { color: var(--text); }
@media (max-width: 900px) {
  .col-head { grid-template-columns: 1fr; }
  .get { align-items: flex-start; }
}
@media (max-width: 720px) {
  .hero { padding: 24px 0 20px; }
  .col { margin: 28px 0 48px; }
  .col-head { padding: 22px 20px; }
  .get { align-items: stretch; width: 100%; }
  .installers { flex-direction: column; }
  .big { justify-content: center; }
  .more { text-align: center; }
  .search kbd { display: none; }
}
"""

ICON_WIN = ('<svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">'
            '<path d="M3 5.5 10.5 4.5v7H3zM11.5 4.3 21 3v8.5h-9.5zM3 12.5h7.5v7L3 18.5zM11.5 12.5H21V21l-9.5-1.3z"/></svg>')
ICON_MAC = ('<svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">'
            '<path d="M16.4 12.6c0-2.4 2-3.5 2.1-3.6-1.1-1.7-2.9-1.9-3.5-1.9-1.5-.2-2.9.9-3.7.9-.8 0-1.9-.9-3.2-.8-1.6 0-3.1 1-4 2.4'
            '-1.7 3-.4 7.4 1.2 9.8.8 1.2 1.8 2.5 3 2.4 1.2 0 1.7-.8 3.2-.8s1.9.8 3.2.8c1.3 0 2.1-1.2 2.9-2.4.9-1.4 1.3-2.7 1.3-2.8'
            '-.1 0-2.5-1-2.5-4zM14 5.6c.7-.8 1.1-1.9 1-3-1 0-2.1.7-2.8 1.5-.6.7-1.2 1.8-1 2.9 1.1.1 2.1-.6 2.8-1.4z"/></svg>')
ICON_PLAY = '<svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M7 4.5v15l13-7.5z"/></svg>'
ICON_GH = ('<svg viewBox="0 0 16 16" width="16" height="16" fill="currentColor" aria-hidden="true"><path d="M8 0C3.58 0 0 '
           '3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23'
           '-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07'
           '-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27'
           '.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 '
           '3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z"/></svg>')
ICON_DL = ('<svg class="dl" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" '
           'stroke-linejoin="round" aria-hidden="true"><path d="M12 4v11m-5-5 5 5 5-5M5 20h14"/></svg>')


def fmt_size(n):
    """バイト数を 180 KB・4.5 MB のように"""
    if not n:
        return ""
    if n < 1024 * 1024:
        return f"{round(n / 1024)} KB"
    return f"{n / 1024 / 1024:.1f} MB"


def size_tag(n):
    return f'<span class="size">{fmt_size(n)}</span>' if n else ""


ICON_SEARCH = ('<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" '
               'aria-hidden="true"><circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/></svg>')

# 検索：空白で区切った語がすべて名前・説明・フォルダー名に含まれるものだけ残す（全角・半角や大文字・小文字は区別しない）
JS = """
(() => {
  const q = document.getElementById('q'), bar = document.querySelector('.bar'), empty = document.getElementById('empty');
  const items = [...document.querySelectorAll('[data-q]')], cols = [...document.querySelectorAll('.col')];
  const norm = s => s.normalize('NFKC').toLowerCase();
  items.forEach(el => { el._q = norm(el.dataset.q); });
  const run = () => {
    const terms = norm(q.value).split(/\\s+/).filter(Boolean);
    let total = 0;
    for (const c of cols) {
      let n = 0;
      for (const el of c.querySelectorAll('[data-q]')) {
        const ok = terms.every(t => el._q.includes(t));
        el.hidden = !ok; if (ok) n++;
      }
      c.hidden = terms.length > 0 && n === 0;
      const j = document.querySelector('.jump a[href="#' + c.id + '"] span');
      if (j) j.textContent = n;
      total += n;
    }
    empty.hidden = total > 0;
    const u = new URL(location.href);
    if (q.value) u.searchParams.set('q', q.value); else u.searchParams.delete('q');
    history.replaceState(null, '', u);
  };
  q.addEventListener('input', run);
  q.addEventListener('keydown', ev => { if (ev.key === 'Escape') { q.value = ''; run(); q.blur(); } });
  document.addEventListener('keydown', ev => {
    if (ev.key === '/' && document.activeElement !== q && !ev.ctrlKey && !ev.metaKey) { ev.preventDefault(); q.focus(); }
  });
  const init = new URLSearchParams(location.search).get('q');
  if (init) { q.value = init; run(); }
  new IntersectionObserver(([e]) => bar.classList.toggle('stuck', e.intersectionRatio < 1), { threshold: [1] })
    .observe(document.getElementById('sentinel'));
})();
"""


def collection_head(c):
    rel = c["release"]
    if rel is None:
        chips, get = "", '<span class="soon">準備中</span>'
    else:
        chips = (f'<div class="chips"><span class="chip">プラグイン<b>{len(c["plugins"])}</b></span>'
                 f'<span class="chip">リリース<b>{e(rel["version"])}</b></span>'
                 f'<span class="chip">公開<b>{e(rel["date"])}</b></span></div>')

        def big(asset, icon, os_name, ext):
            if asset is None:
                return f'<a class="big" href="{e(rel["url"])}">{icon}<span><b>{os_name}</b><small>リリースのページへ</small></span></a>'
            url, size = asset
            sz = f' <span class="nw">· {fmt_size(size)}</span>' if size else ""
            return (f'<a class="big" href="{e(url)}">{icon}<span><b>{os_name}</b>'
                    f'<small>インストーラー {ext}{sz}</small></span>{ICON_DL}</a>')

        get = (f'<div class="installers">'
               + big(rel["win_setup"], ICON_WIN, "Windows", ".exe")
               + big(rel["mac_pkg"], ICON_MAC, "macOS", ".pkg")
               + f'</div><a class="more" href="{e(rel["url"])}">zip・過去の版 →</a>')
    return f"""<div class="col-head">
        <div><h2>{e(c["title"])}</h2><div class="meta">{e(c["lead"])}</div>{chips}</div>
        <div class="get">{get}</div>
      </div>"""


def search_key(p):
    return " ".join([p["name"], p["desc"], p["dir"]])


def plugin_card(p, released):
    """画像（またはブラウザで試すページ）があるプラグイン：大きいカード"""
    href = p["try"] or p["readme"]
    thumb = (f'<a class="thumb" href="{e(href)}"><img src="{e(p["image"])}" alt="" loading="lazy"></a>'
             if p["image"] else "")
    date = f'<span class="date">{e(p["date"])}</span>' if p["date"] else ""
    if released:
        files = (f'<a class="btn" href="{e(p["download"])}/{e(p["stem"])}.aex" title="{e(p["stem"])}.aex">'
                 f'{ICON_WIN}Windows<small>.aex</small>{size_tag(p["aex_size"])}{ICON_DL}</a>'
                 f'<a class="btn" href="{e(p["download"])}/{e(p["stem"])}.plugin.zip" title="{e(p["stem"])}.plugin.zip">'
                 f'{ICON_MAC}macOS<small>.plugin</small>{size_tag(p["zip_size"])}{ICON_DL}</a>')
    else:
        files = '<span class="soon">準備中</span>'
    try_btn = f'<a class="try" href="{e(p["try"])}">{ICON_PLAY}ブラウザで試す</a>' if p["try"] else ""
    return f"""
        <article class="tile" data-q="{e(search_key(p))}">
          {thumb}
          <div class="body">
            <h3><a href="{e(href)}">{e(p["name"])}</a></h3>
            <div class="vline"><a class="ver" href="{e(p["tag_url"])}" title="この版のリリース">v{e(p["version"])}</a>{date}</div>
            <p>{e(p["desc"])}</p>
            <a class="doc" href="{e(p["readme"])}">説明・パラメータ →</a>
            <div class="actions">{try_btn}{files}</div>
          </div>
        </article>"""


def plugin_item(p, released):
    """画像の無いプラグイン：詰めた一覧の 1 行"""
    if released:
        files = (f'<a class="mini" href="{e(p["download"])}/{e(p["stem"])}.aex" title="Windows：{e(p["stem"])}.aex">'
                 f'{ICON_WIN}Win{size_tag(p["aex_size"])}{ICON_DL}</a>'
                 f'<a class="mini" href="{e(p["download"])}/{e(p["stem"])}.plugin.zip" title="macOS：{e(p["stem"])}.plugin.zip">'
                 f'{ICON_MAC}Mac{size_tag(p["zip_size"])}{ICON_DL}</a>')
    else:
        files = '<span class="soon">準備中</span>'
    return f"""
        <article class="item" data-q="{e(search_key(p))}">
          <h3><a href="{e(p["readme"])}">{e(p["name"])}</a><a class="ver" href="{e(p["tag_url"])}" title="この版のリリース">v{e(p["version"])}</a></h3>
          <p>{e(p["desc"])}</p>
          <div class="row"><a class="doc" href="{e(p["readme"])}" title="説明・パラメータ">説明 →</a>{files}</div>
        </article>"""


def collection(c):
    released = c["release"] is not None
    big = [p for p in c["plugins"] if p["image"] or p["try"]]
    small = [p for p in c["plugins"] if not (p["image"] or p["try"])]
    body = ""
    if big:
        body += '\n      <div class="grid">' + "".join(plugin_card(p, released) for p in big) + "\n      </div>"
    if small:
        body += '\n      <div class="list">' + "".join(plugin_item(p, released) for p in small) + "\n      </div>"
    return f'\n    <section class="col" id="{e(c["id"])}">\n      {collection_head(c)}{body}\n    </section>'


INSTALL = f"""
  <section class="install" id="install">
    <h2>インストール</h2>
    <p>インストーラーで、そのコレクションのプラグインがまとめて入ります。先に After Effects を終了しておいてください。</p>
    <div class="steps">
      <div class="step">
        <h3>{ICON_WIN}Windows</h3>
        <ol>
          <li><code>…-windows-setup.exe</code> を実行する</li>
          <li>「Windows によって PC が保護されました」と出たら「詳細情報」→「実行」を押す（署名していないため）</li>
          <li><code>C:\\Program Files\\Adobe\\Common\\Plug-ins\\7.0\\MediaCore\\</code> の中に入る</li>
        </ol>
        <p class="sub">アンインストールは「設定 → アプリ → インストールされているアプリ」から。</p>
      </div>
      <div class="step">
        <h3>{ICON_MAC}macOS</h3>
        <ol>
          <li><code>…-macos.pkg</code> を開く</li>
          <li>最初は開けないので、システム設定 → プライバシーとセキュリティ の「このまま開く」を押してから、もう一度開く</li>
          <li><code>/Library/Application Support/Adobe/Common/Plug-ins/7.0/MediaCore/</code> の中に入る</li>
        </ol>
        <p class="sub">アンインストールは、そのフォルダーの中の <code>.plugin</code> を消します。</p>
      </div>
    </div>
    <details>
      <summary>アップデート・プラグインを 1 つだけ入れるとき</summary>
      <ul>
        <li>アップデート：新しいリリースのインストーラーをそのまま実行すると上書きされます。</li>
        <li>1 つだけ入れる・前の版に戻す：各プラグインの Win / Mac ボタンのファイルを使います。Windows は <code>.aex</code> を、macOS は <code>.plugin.zip</code> を解凍した <code>.plugin</code> を、上の MediaCore の中のフォルダーにコピーします。</li>
      </ul>
    </details>
  </section>"""


def render(collections):
    for c in collections:
        c["id"] = c["title"].lower()
    total = sum(len(c["plugins"]) for c in collections)
    cols = "".join(collection(c) for c in collections)
    jump = "".join(f'<a href="#{e(c["id"])}">{e(c["title"])}<span>{len(c["plugins"])}</span></a>' for c in collections)
    return f"""<!doctype html>
<html lang="ja">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>JK-Plugins</title>
<meta name="description" content="After Effects 用プラグイン JK-Plugins・fs-rs のダウンロード">
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link href="https://fonts.googleapis.com/css2?family=Inter:wght@400;600;700;800&display=swap" rel="stylesheet">
<style>{CSS}</style>
</head>
<body>
<div class="wrap">
  <nav class="top"><span class="logo">JK-Plugins</span><span class="links"><a href="#install">インストール</a><a href="https://github.com/{SELF}">{ICON_GH}GitHub</a></span></nav>
  <header class="hero">
    <h1>JK-Plugins</h1>
    <p>Rust で書いた After Effects プラグイン。Windows・macOS 対応、無料。</p>
    <div class="stats"><span class="stat"><b>{total}</b>プラグイン</span><span class="stat"><b>{len(collections)}</b>コレクション</span></div>
  </header>
  <div id="sentinel"></div>
  <div class="bar"><div class="bar-in">
    <label class="search">{ICON_SEARCH}<input id="q" type="search" placeholder="プラグインを探す（名前・説明）" autocomplete="off" aria-label="プラグインを探す"><kbd>/</kbd></label>
    <div class="jump">{jump}<a href="#install">インストール</a></div>
  </div></div>
{cols}
  <p class="empty" id="empty" hidden>見つかりませんでした</p>
{INSTALL}
  <footer>
    {"".join(f'<a href="https://github.com/{c["repo"]}">GitHub: {e(c["title"])}</a>' for c in collections)}<a href="https://github.com/{SELF}/issues">不具合の報告</a>
  </footer>
</div>
<script>{JS}</script>
</body>
</html>
"""


def build_wasm(crate_dir):
    """web/<dir> の crate を wasm にビルドして .wasm のパスを返す（できなければ None）"""
    if shutil.which("cargo") is None:
        print(f"warning: cargo が無いので {crate_dir} の wasm を作れない")
        return None
    r = subprocess.run(["cargo", "build", "--release", "--target", "wasm32-unknown-unknown"],
                       cwd=crate_dir, encoding="utf-8", errors="replace")
    if r.returncode != 0:
        print(f"warning: {crate_dir} の wasm のビルドに失敗")
        return None
    target = Path(os.environ.get("CARGO_TARGET_DIR") or crate_dir / "target")
    found = sorted((target / "wasm32-unknown-unknown" / "release").glob("*.wasm"))
    return found[0] if found else None


def build_try_page(p, out):
    """ブラウザで試すページを out/try/<dir>/ に作る"""
    crate_dir = ROOT / "web" / p["dir"]
    site_dir = crate_dir / "site"
    if not (site_dir / "index.html").exists():
        return False
    wasm = build_wasm(crate_dir)
    if wasm is None:
        return False
    dest = out / "try" / p["dir"]
    shutil.copytree(site_dir, dest)
    shutil.copy2(wasm, dest / wasm.name)
    page = dest / "index.html"
    html_text = page.read_text(encoding="utf-8")
    for k, v in {
        "{{README}}": e(p["readme"]),
        "{{VERSION}}": e(p["version"]),
        "{{AEX}}": e(f'{p["download"]}/{p["stem"]}.aex'),
        "{{PLUGIN}}": e(f'{p["download"]}/{p["stem"]}.plugin.zip'),
        "{{ICON_WIN}}": ICON_WIN,
        "{{ICON_MAC}}": ICON_MAC,
        "{{ICON_DL}}": ICON_DL,
        "{{AEX_SIZE}}": size_tag(p["aex_size"]),
        "{{PLUGIN_SIZE}}": size_tag(p["zip_size"]),
    }.items():
        html_text = html_text.replace(k, v)
    page.write_text(html_text, encoding="utf-8")
    return True


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default="_site")
    args = ap.parse_args()
    out = Path(args.out)
    if out.exists():
        shutil.rmtree(out)
    (out / "img").mkdir(parents=True)
    collections = []
    for c in COLLECTIONS:
        c = dict(c)
        c["plugins"] = plugins_of(c["repo"])
        if not c["plugins"]:
            continue
        c["release"] = latest_release(c["repo"])
        if c["release"]:
            for p in c["plugins"]:
                p["aex_size"] = c["release"]["sizes"].get(f'{p["stem"]}.aex')
                p["zip_size"] = c["release"]["sizes"].get(f'{p["stem"]}.plugin.zip')
        if c["repo"] == SELF:
            for p in c["plugins"]:
                img = ROOT / p["dir"] / "docs" / "samples" / "00_overview.png"
                if img.exists():
                    shutil.copy2(img, out / "img" / f"{p['dir']}.png")
                    p["image"] = f"img/{p['dir']}.png"
                if build_try_page(p, out):
                    p["try"] = f"try/{p['dir']}/"
        collections.append(c)
    (out / "index.html").write_text(render(collections), encoding="utf-8")
    (out / ".nojekyll").write_text("", encoding="utf-8")
    for c in collections:
        rel = c["release"]["tag"] if c["release"] else "no release"
        print(f"{c['repo']}: {len(c['plugins'])} plug-ins, {rel}")


if __name__ == "__main__":
    main()
