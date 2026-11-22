"""配布サイト（GitHub Pages）を作る。

  python .github/scripts/site.py --out _site

- 載せるリポジトリは COLLECTIONS（JK-Plugins と fs-rs）
- プラグインの一覧（名前・バージョン・説明・説明ページ）は各リポジトリの README の「プラグイン一覧」の表から読む
  （README-ja.md があればそれ、無ければ README.md。リリースの計画でバージョンが Cargo.toml と合っていることを確かめている）
- ダウンロードのリンクは各リポジトリの最新のまとめリリース（gh release view）から作る。リリースがまだ無ければ「準備中」
- 画像はこのリポジトリのプラグインのフォルダーの docs/samples/00_overview.png があれば使う
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
    assets = {a["name"]: a["url"] for a in rel["assets"]}

    def find(suffix):
        return next((u for n, u in assets.items() if n.endswith(suffix)), None)

    return {
        "tag": rel["tagName"],
        "version": rel["tagName"].lstrip("v"),
        "date": rel["publishedAt"][:10],
        "url": rel["url"],
        "win_setup": find("-windows-setup.exe"),
        "win_zip": find("-windows.zip"),
        "mac_pkg": find("-macos.pkg"),
    }


def e(s):
    return html.escape(str(s), quote=True)


CSS = """
:root {
  --bg: #fafafa; --surface: #ffffff; --text: #0f0f12; --muted: #6e6e78; --line: rgba(15, 15, 18, .08);
  --accent: #e0457b; --accent2: #7c5cff; --accent-soft: rgba(224, 69, 123, .10);
  --btn: #0f0f12; --btn-text: #ffffff; --shadow: 0 1px 2px rgba(0,0,0,.04), 0 8px 24px rgba(0,0,0,.06);
  --glow1: rgba(224, 69, 123, .20); --glow2: rgba(124, 92, 255, .18);
}
@media (prefers-color-scheme: dark) {
  :root:not([data-theme="light"]) {
    --bg: #0b0b0e; --surface: #141418; --text: #f4f4f6; --muted: #9b9ba6; --line: rgba(255, 255, 255, .08);
    --accent: #ff6b9c; --accent2: #9b85ff; --accent-soft: rgba(255, 107, 156, .14);
    --btn: #f4f4f6; --btn-text: #0b0b0e; --shadow: 0 1px 2px rgba(0,0,0,.3), 0 12px 32px rgba(0,0,0,.35);
    --glow1: rgba(255, 107, 156, .16); --glow2: rgba(155, 133, 255, .14);
  }
}
* { box-sizing: border-box; }
html { -webkit-text-size-adjust: 100%; }
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
.wrap { max-width: 1080px; margin: 0 auto; padding: 0 20px; }
nav { display: flex; justify-content: space-between; align-items: center; padding: 20px 0; }
.logo { font-weight: 800; letter-spacing: -0.02em; }
nav a.gh { color: var(--muted); font-size: 14px; display: inline-flex; gap: 6px; align-items: center; }
nav a.gh:hover { color: var(--text); }
.hero { padding: 48px 0 8px; text-align: center; }
.hero h1 {
  margin: 0; font-size: clamp(40px, 8vw, 72px); line-height: 1; letter-spacing: -0.04em; font-weight: 800;
  background: linear-gradient(100deg, var(--text) 30%, var(--accent) 70%, var(--accent2));
  -webkit-background-clip: text; background-clip: text; color: transparent;
}
.hero p { margin: 14px 0 0; color: var(--muted); font-size: 17px; }
.col { margin: 56px 0 72px; }
.col-head { display: flex; flex-direction: column; align-items: center; text-align: center; gap: 22px;
  padding: 36px 20px 32px; border-radius: 28px; background: var(--surface); border: 1px solid var(--line); box-shadow: var(--shadow); }
.col-head h2 { margin: 0; font-size: 32px; line-height: 1.1; letter-spacing: -0.02em; font-weight: 700; }
.meta { color: var(--muted); font-size: 14px; margin-top: 6px; }
.chips { display: flex; flex-wrap: wrap; gap: 6px; margin-top: 12px; justify-content: center; }
.chip { font-size: 12px; padding: 3px 10px; border-radius: 999px; background: var(--surface); border: 1px solid var(--line);
  color: var(--muted); white-space: nowrap; font-variant-numeric: tabular-nums; }
.chip b { color: var(--text); font-weight: 600; margin-left: 6px; }
.get { display: flex; flex-direction: column; align-items: center; gap: 12px; }
.installers { display: flex; gap: 12px; flex-wrap: wrap; justify-content: center; }
.big { display: inline-flex; align-items: center; gap: 12px; padding: 14px 28px 14px 22px; border-radius: 18px;
  color: #fff; background: linear-gradient(120deg, var(--accent), var(--accent2)); text-align: left;
  box-shadow: 0 10px 30px var(--glow1); transition: transform .15s, box-shadow .15s; min-width: 220px; }
.big:hover { transform: translateY(-2px); box-shadow: 0 14px 36px var(--glow1); }
.big svg { width: 26px; height: 26px; flex: none; }
.big b { display: block; font-size: 18px; line-height: 1.2; }
.big small { display: block; font-size: 12px; opacity: .85; }
.btn {
  display: inline-flex; align-items: center; justify-content: center; gap: 8px; padding: 11px 18px; border-radius: 999px;
  background: var(--btn); color: var(--btn-text); font-weight: 600; font-size: 14px; white-space: nowrap;
  transition: transform .15s, opacity .15s;
}
.btn svg { width: 15px; height: 15px; flex: none; }
.btn:hover { transform: translateY(-1px); opacity: .9; }
.more { color: var(--muted); font-size: 13px; padding: 0 6px; }
.more:hover { color: var(--text); }
.soon { color: var(--muted); font-size: 13px; font-weight: 600; padding: 4px 12px; border-radius: 999px;
  border: 1px dashed var(--line); }
.grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 280px), 340px)); justify-content: center; gap: 18px; margin-top: 24px; }
.tile {
  display: flex; flex-direction: column; background: var(--surface); border: 1px solid var(--line);
  border-radius: 20px; overflow: hidden; box-shadow: var(--shadow); transition: transform .2s, box-shadow .2s;
}
.tile:hover { transform: translateY(-3px); box-shadow: var(--shadow), 0 0 0 1px var(--accent-soft); }
.thumb { display: block; aspect-ratio: 16 / 10; overflow: hidden; border-bottom: 1px solid var(--line); }
.thumb img { width: 100%; height: 100%; object-fit: cover; display: block; transition: transform .4s; }
.tile:hover .thumb img { transform: scale(1.03); }
.body { padding: 18px 18px 18px; display: flex; flex-direction: column; gap: 8px; flex: 1; }
.body h3 { margin: 0; font-size: 17px; letter-spacing: -0.01em; display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
.ver { font-size: 12px; padding: 1px 9px; border-radius: 999px; background: var(--accent-soft); color: var(--accent);
  font-weight: 600; font-variant-numeric: tabular-nums; }
.vline { display: flex; align-items: center; gap: 8px; margin-top: -2px; }
.date { color: var(--muted); font-size: 12px; font-variant-numeric: tabular-nums; }
.body p { margin: 0; color: var(--muted); font-size: 14px; flex: 1; }
.files { display: grid; grid-template-columns: 1fr 1fr; gap: 8px; margin-top: 8px; }
.files .btn { padding: 9px 12px; font-size: 13px; background: transparent; color: var(--text);
  box-shadow: inset 0 0 0 1px var(--line); }
.files .btn small { font-size: 11px; font-weight: 600; color: var(--accent); background: var(--accent-soft); padding: 1px 7px; border-radius: 999px; }
.files .btn:hover { box-shadow: inset 0 0 0 1px var(--text); opacity: 1; }
.files .soon { grid-column: 1 / -1; text-align: center; }
footer { margin-top: 24px; padding: 28px 0 48px; border-top: 1px solid var(--line); color: var(--muted); font-size: 13px;
  display: flex; gap: 20px; flex-wrap: wrap; }
footer a:hover { color: var(--text); }
@media (max-width: 640px) {
  .hero { padding-top: 32px; }
  .col { margin: 40px 0 56px; }
  .installers { width: 100%; flex-direction: column; }
  .big { justify-content: center; }
}
"""

ICON_WIN = ('<svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">'
            '<path d="M3 5.5 10.5 4.5v7H3zM11.5 4.3 21 3v8.5h-9.5zM3 12.5h7.5v7L3 18.5zM11.5 12.5H21V21l-9.5-1.3z"/></svg>')
ICON_MAC = ('<svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">'
            '<path d="M16.4 12.6c0-2.4 2-3.5 2.1-3.6-1.1-1.7-2.9-1.9-3.5-1.9-1.5-.2-2.9.9-3.7.9-.8 0-1.9-.9-3.2-.8-1.6 0-3.1 1-4 2.4'
            '-1.7 3-.4 7.4 1.2 9.8.8 1.2 1.8 2.5 3 2.4 1.2 0 1.7-.8 3.2-.8s1.9.8 3.2.8c1.3 0 2.1-1.2 2.9-2.4.9-1.4 1.3-2.7 1.3-2.8'
            '-.1 0-2.5-1-2.5-4zM14 5.6c.7-.8 1.1-1.9 1-3-1 0-2.1.7-2.8 1.5-.6.7-1.2 1.8-1 2.9 1.1.1 2.1-.6 2.8-1.4z"/></svg>')
ICON_GH = ('<svg viewBox="0 0 24 24" width="16" height="16" fill="currentColor" aria-hidden="true"><path d="M12 2a10 10 0 0 0-3.2 19.5'
           'c.5.1.7-.2.7-.5v-1.7c-2.8.6-3.4-1.3-3.4-1.3-.5-1.2-1.1-1.5-1.1-1.5-.9-.6.1-.6.1-.6 1 .1 1.5 1 1.5 1 .9 1.5 2.3 1.1 2.9.8'
           '.1-.6.3-1.1.6-1.3-2.2-.3-4.6-1.1-4.6-5 0-1.1.4-2 1-2.7-.1-.3-.4-1.3.1-2.7 0 0 .8-.3 2.8 1a9.5 9.5 0 0 1 5 0c1.9-1.3 2.8-1 2.8-1'
           '.5 1.4.2 2.4.1 2.7.6.7 1 1.6 1 2.7 0 3.9-2.4 4.7-4.6 5 .4.3.7.9.7 1.9V21c0 .3.2.6.7.5A10 10 0 0 0 12 2z"/></svg>')


def collection_head(c):
    rel = c["release"]
    if rel is None:
        chips, get = "", '<span class="soon">準備中</span>'
    else:
        chips = (f'<div class="chips"><span class="chip">リリース<b>{e(rel["version"])}</b></span>'
                 f'<span class="chip">公開<b>{e(rel["date"])}</b></span></div>')
        win = rel["win_setup"] or rel["url"]
        mac = rel["mac_pkg"] or rel["url"]
        get = (f'<div class="installers">'
               f'<a class="big" href="{e(win)}">{ICON_WIN}<span><b>Windows</b><small>インストーラー .exe</small></span></a>'
               f'<a class="big" href="{e(mac)}">{ICON_MAC}<span><b>macOS</b><small>インストーラー .pkg</small></span></a>'
               f'</div><a class="more" href="{e(rel["url"])}" title="zip・過去の版">他のファイル</a>')
    return f"""<div class="col-head">
        <div><h2>{e(c["title"])}</h2><div class="meta">{e(c["lead"])}</div>{chips}</div>
        <div class="get">{get}</div>
      </div>"""


def plugin_card(p, released):
    thumb = (
        f'<a class="thumb" href="{e(p["readme"])}"><img src="{e(p["image"])}" alt="" loading="lazy"></a>'
        if p["image"]
        else ""
    )
    date = f'<span class="date">{e(p["date"])}</span>' if p["date"] else ""
    if released:
        files = f"""<div class="files">
              <a class="btn" href="{e(p["download"])}/{e(p["stem"])}.aex" title="{e(p["stem"])}.aex">{ICON_WIN}Windows<small>.aex</small></a>
              <a class="btn" href="{e(p["download"])}/{e(p["stem"])}.plugin.zip" title="{e(p["stem"])}.plugin.zip">{ICON_MAC}macOS<small>.plugin</small></a>
            </div>"""
    else:
        files = '<div class="files"><span class="soon">準備中</span></div>'
    return f"""
        <article class="tile">
          {thumb}
          <div class="body">
            <h3><a href="{e(p["readme"])}">{e(p["name"])}</a></h3>
            <div class="vline"><a class="ver" href="{e(p["tag_url"])}" title="この版のリリース">v{e(p["version"])}</a>{date}</div>
            <p>{e(p["desc"])}</p>
            {files}
          </div>
        </article>"""


def render(collections):
    cols = "".join(
        f'\n    <section class="col">\n      {collection_head(c)}\n      <div class="grid">'
        + "".join(plugin_card(p, c["release"] is not None) for p in c["plugins"])
        + "\n      </div>\n    </section>"
        for c in collections
    )
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
  <nav><span></span><a class="gh" href="https://github.com/{SELF}">{ICON_GH}GitHub</a></nav>
  <header class="hero"><h1>JK-Plugins</h1><p>After Effects プラグイン</p></header>
{cols}
  <footer>
    {"".join(f'<a href="https://github.com/{c["repo"]}">GitHub: {e(c["title"])}</a>' for c in collections)}<a href="https://github.com/{SELF}/issues">不具合の報告</a>
  </footer>
</div>
</body>
</html>
"""


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
        if c["repo"] == SELF:
            for p in c["plugins"]:
                img = ROOT / p["dir"] / "docs" / "samples" / "00_overview.png"
                if img.exists():
                    shutil.copy2(img, out / "img" / f"{p['dir']}.png")
                    p["image"] = f"img/{p['dir']}.png"
        collections.append(c)
    (out / "index.html").write_text(render(collections), encoding="utf-8")
    (out / ".nojekyll").write_text("", encoding="utf-8")
    for c in collections:
        rel = c["release"]["tag"] if c["release"] else "no release"
        print(f"{c['repo']}: {len(c['plugins'])} plug-ins, {rel}")


if __name__ == "__main__":
    main()
