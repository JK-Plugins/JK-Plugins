"""配布サイト（GitHub Pages）を作る。

  python .github/scripts/site.py --out _site

- 載せるリポジトリは COLLECTIONS（JK-Plugins と fs-rs）
- プラグインの一覧（名前・バージョン・説明・説明ページ）は各リポジトリの README の「プラグイン一覧」の表から読む
  （README-ja.md があればそれ、無ければ README.md。リリースの計画でバージョンが Cargo.toml と合っていることを確かめている）
- ダウンロードのリンクとファイルサイズは各リポジトリの最新のまとめリリース（gh release view）から作る。リリースがまだ無ければ「準備中」
- 画像の無いプラグインは詰めた一覧で並べる。上の検索欄で名前・説明から絞り込める
- 見た目と動きは .github/scripts/site/ の style.css・main.js（index.html に埋め込む）
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

ASSETS = Path(__file__).resolve().parent / "site"
FONTS = ("https://fonts.googleapis.com/css2?family=DotGothic16&family=Instrument+Serif:ital@0;1"
         "&family=Shippori+Mincho+B1:wght@500;600;800&display=swap")


def px(tag, text, cls="", attrs=""):
    """ドットで描く見出し（JS が canvas に置き換える）"""
    return f'<{tag} class="px {cls}"{attrs}><span class="px-txt">{e(text)}</span></{tag}>'


def pretty_name(name):
    """JK Alpha Fix Fs → 頭の JK と末尾の Fs を薄く"""
    parts = name.split(" ")
    out = [e(x) for x in parts]
    if len(parts) > 2 and parts[0] == "JK":
        out[0] = f'<span class="pre">{out[0]}</span>'
    if len(parts) > 2 and parts[-1] == "Fs":
        out[-1] = f'<span class="suf">{out[-1]}</span>'
    return " ".join(out)


def files_html(p, released):
    """個別ファイルのボタン（Windows・macOS で同じ形）"""
    if not released:
        return '<div class="files"><span class="soon mono">準備中</span></div>'

    def b(ext, size, icon, os_name):
        f = f'{p["stem"]}{ext}'
        return (f'<a class="fbtn" href="{e(p["download"])}/{e(f)}" title="{e(f)}" data-cur="{e(fmt_size(size) or "DL")}">'
                f'{icon}<span>{os_name}</span><span class="size">{fmt_size(size)}</span>{ICON_DL}</a>')

    return ('<div class="files">' + b(".aex", p["aex_size"], ICON_WIN, "Windows")
            + b(".plugin.zip", p["zip_size"], ICON_MAC, "macOS") + '</div>')


def collection_head(c, i, n):
    rel = c["release"]
    facts = f'<div><dt>PLUGINS</dt><dd>{len(c["plugins"])}</dd></div>'
    if rel is None:
        get = '<span class="soon mono">準備中</span>'
    else:
        facts += (f'<div><dt>RELEASE</dt><dd>{e(rel["version"])}</dd></div>'
                  f'<div><dt>DATE</dt><dd>{e(rel["date"])}</dd></div>')

        def big(asset, icon, os_name, ext):
            if asset is None:
                return (f'<a class="big" href="{e(rel["url"])}" data-cur="OPEN">{icon}<span><b>{os_name}</b>'
                        f'<small>リリースのページへ</small></span></a>')
            url, size = asset
            sz = f" · {fmt_size(size)}" if size else ""
            return (f'<a class="big" href="{e(url)}" data-cur="DOWNLOAD">{icon}<span><b>{os_name}</b>'
                    f'<small>INSTALLER {ext}{sz}</small></span>{ICON_DL}</a>')

        get = ('<div class="installers">' + big(rel["win_setup"], ICON_WIN, "Windows", ".exe")
               + big(rel["mac_pkg"], ICON_MAC, "macOS", ".pkg") + '</div>'
               + f'<a class="more mono" href="{e(rel["url"])}" data-cur="RELEASE">zip・過去の版 →</a>')
    return f"""<header class="col-head">
        <div class="col-idx mono"><span>{i:02d} / {n:02d}</span><span>COLLECTION</span></div>
        <div>{px("h2", c["title"])}<p class="lead">{e(c["lead"])}</p><dl class="facts mono">{facts}</dl></div>
        <div class="get">{get}</div>
      </header>"""


def search_key(p):
    return " ".join([p["name"], p["desc"], p["dir"]])


def plugin_card(p, released, no):
    """画像（またはブラウザで試すページ）があるプラグイン：大きいカード"""
    href = p["try"] or p["readme"]
    thumb = (f'<a class="thumb" href="{e(href)}" data-cur="{"TRY" if p["try"] else "READ"}">'
             f'<img src="{e(p["image"])}" alt="" loading="lazy"></a>' if p["image"] else "")
    date = f' · {e(p["date"])}' if p["date"] else ""
    try_btn = (f'<a class="try" href="{e(p["try"])}" data-cur="TRY"><span>ブラウザで試す</span>{ICON_PLAY}</a>'
               if p["try"] else "")
    return f"""
        <article class="tile rv" data-q="{e(search_key(p))}">
          {thumb}
          <div class="body">
            <div class="top"><span class="no">{no:03d}</span><a class="ver" href="{e(p["tag_url"])}" title="この版のリリース">v{e(p["version"])}{date}</a></div>
            <h3><a href="{e(p["readme"])}" data-cur="README">{pretty_name(p["name"])}</a></h3>
            <p>{e(p["desc"])}</p>
            {try_btn}
            {files_html(p, released)}
          </div>
        </article>"""


def plugin_item(p, released, no):
    """画像の無いプラグイン：一覧の 1 マス"""
    return f"""
        <article class="item rv" data-q="{e(search_key(p))}">
          <div class="top"><span class="no">{no:03d}</span><a class="ver" href="{e(p["tag_url"])}" title="この版のリリース">v{e(p["version"])}</a></div>
          <h3><a href="{e(p["readme"])}" title="説明・パラメータ" data-cur="README">{pretty_name(p["name"])}</a></h3>
          <p>{e(p["desc"])}</p>
          {files_html(p, released)}
        </article>"""


def collection(c, i, n, start):
    released = c["release"] is not None
    big = [p for p in c["plugins"] if p["image"] or p["try"]]
    small = [p for p in c["plugins"] if not (p["image"] or p["try"])]
    body, no = "", start
    if big:
        body += '\n      <div class="grid">'
        for p in big:
            no += 1
            body += plugin_card(p, released, no)
        body += "\n      </div>"
    if small:
        body += '\n      <div class="list">'
        for p in small:
            no += 1
            body += plugin_item(p, released, no)
        body += "\n      </div>"
    return f'\n    <section class="col" id="{e(c["id"])}">\n      {collection_head(c, i, n)}{body}\n    </section>'


def render(collections):
    css = (ASSETS / "style.css").read_text(encoding="utf-8")
    js = (ASSETS / "main.js").read_text(encoding="utf-8")
    for c in collections:
        c["id"] = c["title"].lower()
    total = sum(len(c["plugins"]) for c in collections)
    n = len(collections)
    cols, start = "", 0
    for i, c in enumerate(collections, 1):
        cols += collection(c, i, n, start)
        start += len(c["plugins"])
    jump = "".join(f'<a href="#{e(c["id"])}" data-cur="JUMP">{e(c["title"])}<span>{len(c["plugins"])}</span></a>'
                   for c in collections)
    names = "".join(f'<span>{e(p["name"])}</span><i>■</i>' for c in collections for p in c["plugins"])
    return f"""<!doctype html>
<html lang="ja">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>JK-Plugins</title>
<meta name="description" content="After Effects 用プラグイン JK-Plugins・fs-rs のダウンロード">
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link href="{FONTS}" rel="stylesheet">
<style>{css}</style>
</head>
<body>
<div class="wrap">
  <nav class="top mono"><span class="logo"><i></i>JK—PLUGINS</span><a href="https://github.com/{SELF}" data-cur="GITHUB">GITHUB ↗</a></nav>
  <header class="hero">
    <div class="hero-meta mono"><span>AFTER EFFECTS PLUG-INS</span><span>WRITTEN IN RUST</span><span>WINDOWS / MACOS</span><span>FREE</span></div>
    {px("h1", "JK-Plugins", attrs=" data-fit")}
    <div class="hero-foot">
      <p class="lead">Rust で書いた After Effects プラグイン。<br>Windows・macOS 対応、無料。</p>
      <div class="stats mono"><div><b>{total}</b><span>PLUGINS</span></div><div><b>{n:02d}</b><span>COLLECTIONS</span></div></div>
    </div>
  </header>
  <div class="marquee" aria-hidden="true"><div class="mq-track">{names}{names}</div></div>
  <div class="bar"><div class="bar-in">
    <label class="search">{ICON_SEARCH}<input id="q" type="search" placeholder="SEARCH — 名前・説明で探す" autocomplete="off" aria-label="プラグインを探す"><kbd>/</kbd></label>
    <span class="count mono"><b id="count">{total:03d}</b> / {total:03d}</span>
    <div class="jump mono">{jump}</div>
  </div></div>
{cols}
  <p class="empty mono" id="empty" hidden>NO RESULTS — 見つかりませんでした</p>
  <footer class="mono">
    {px("div", "JK-Plugins")}
    {"".join(f'<a href="https://github.com/{c["repo"]}">GITHUB: {e(c["title"])} ↗</a>' for c in collections)}<a href="https://github.com/{SELF}/issues">不具合の報告 ↗</a>
  </footer>
</div>
<div class="readout" aria-hidden="true">SCROLL <b>000</b>%</div>
<script>{js}</script>
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
