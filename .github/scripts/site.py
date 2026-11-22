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
    {"repo": "JK-Plugins/JK-Plugins", "title": "JK-Plugins", "lead": "オリジナルのプラグイン"},
    {"repo": "JK-Plugins/fs-rs", "title": "fs-rs",
     "lead": "F's Plugin（bryful 氏）を Rust に移したもの。macOS でも使える"},
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
            out.append(d)
    if not out:
        # まだプラグイン一覧が無いリポジトリは載せない（サイト全体は作る）
        print(f"warning: {repo}: README にプラグイン一覧の行が見つからないので載せない")
    return out


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
  --bg: #f7f6f3; --surface: #ffffff; --text: #1d1d1f; --muted: #6b6b70; --line: #e4e2dd;
  --accent: #e0457b; --accent-text: #ffffff; --accent-soft: #fde8ef; --code: #f0eee9;
}
@media (prefers-color-scheme: dark) {
  :root:not([data-theme="light"]) {
    --bg: #141416; --surface: #1d1d20; --text: #f2f2f3; --muted: #a0a0a8; --line: #2e2e33;
    --accent: #ff6b9c; --accent-text: #141416; --accent-soft: #3a1f2a; --code: #26262a;
  }
}
* { box-sizing: border-box; }
body {
  margin: 0; background: var(--bg); color: var(--text);
  font: 16px/1.7 system-ui, -apple-system, "Segoe UI", "Hiragino Sans", "Yu Gothic UI", "Noto Sans JP", sans-serif;
}
a { color: var(--accent); }
.wrap { max-width: 920px; margin: 0 auto; padding: 0 16px; }
header { padding: 56px 0 28px; }
header h1 { margin: 0; font-size: clamp(32px, 6vw, 48px); letter-spacing: -0.02em; }
header p { margin: 8px 0 0; color: var(--muted); }
section { margin: 0 0 40px; }
h2 { font-size: 20px; margin: 0 0 14px; }
.stack { display: grid; gap: 16px; }
.card { background: var(--surface); border: 1px solid var(--line); border-radius: 14px; padding: 22px; }
.release { display: flex; flex-wrap: wrap; gap: 4px 14px; align-items: baseline; margin-bottom: 4px; }
.release strong { font-size: 18px; }
.release span { color: var(--muted); font-size: 14px; }
.lead { margin: 0 0 14px; color: var(--muted); font-size: 14px; }
.buttons { display: grid; grid-template-columns: repeat(auto-fit, minmax(240px, 1fr)); gap: 12px; }
.btn {
  display: block; text-decoration: none; border-radius: 12px; padding: 14px 16px;
  border: 1px solid var(--line); color: var(--text); background: var(--bg);
}
.btn b { display: block; font-size: 16px; }
.btn small { color: var(--muted); }
.btn.primary { background: var(--accent); border-color: var(--accent); color: var(--accent-text); }
.btn.primary small { color: var(--accent-text); opacity: .85; }
.alt { margin: 14px 0 0; font-size: 14px; color: var(--muted); }
h3.group { font-size: 16px; margin: 26px 0 10px; color: var(--muted); font-weight: 600; }
h3.group:first-child { margin-top: 0; }
.plugin { display: grid; grid-template-columns: minmax(0, 260px) 1fr; gap: 20px; align-items: start; }
.plugin.noimg { grid-template-columns: 1fr; }
.plugin img { width: 100%; border-radius: 10px; border: 1px solid var(--line); display: block; }
.plugin h4 { margin: 0; font-size: 18px; }
.plugin h4 a { color: var(--text); text-decoration: none; }
.plugin h4 a:hover { text-decoration: underline; }
.ver { display: inline-block; margin-left: 8px; font-size: 13px; padding: 1px 8px; border-radius: 999px;
  background: var(--accent-soft); color: var(--accent); vertical-align: 2px; font-weight: 400; }
.plugin p { margin: 6px 0 10px; }
.links { display: flex; flex-wrap: wrap; gap: 6px 16px; font-size: 14px; }
ol { margin: 0; padding-left: 1.3em; }
li { margin: 4px 0; }
code { background: var(--code); padding: 1px 6px; border-radius: 6px; font-size: 0.9em; word-break: break-all;
  font-family: ui-monospace, "SF Mono", Consolas, "Cascadia Mono", Menlo, monospace; }
.os { display: grid; grid-template-columns: repeat(auto-fit, minmax(260px, 1fr)); gap: 16px; }
.os h3 { margin: 0 0 8px; font-size: 16px; }
footer { padding: 24px 0 48px; color: var(--muted); font-size: 14px; border-top: 1px solid var(--line); }
footer a { color: var(--muted); }
@media (max-width: 640px) { .plugin { grid-template-columns: 1fr; } }
"""

# いま使っている OS のボタンを先頭・強調にする
SCRIPT = """
(function () {
  var mac = /Mac/i.test(navigator.platform || navigator.userAgent);
  document.querySelectorAll(".buttons").forEach(function (box) {
    var first = box.querySelector(mac ? "[data-os=mac]" : "[data-os=win]");
    var other = box.querySelector(mac ? "[data-os=win]" : "[data-os=mac]");
    if (first && other) {
      box.insertBefore(first, box.firstChild);
      first.classList.add("primary");
      other.classList.remove("primary");
    }
  });
})();
"""


def download_card(c):
    repo, rel = c["repo"], c["release"]
    gh_url = f"https://github.com/{repo}"
    if rel is None:
        return f"""
      <div class="card">
        <div class="release"><strong>{e(c["title"])}</strong><span>準備中</span></div>
        <p class="lead">{e(c["lead"])}</p>
        <p class="alt">まだリリースがありません。<a href="{gh_url}">GitHub</a></p>
      </div>"""
    win = rel["win_setup"] or rel["url"]
    mac = rel["mac_pkg"] or rel["url"]
    zip_link = f'<a href="{e(rel["win_zip"])}">Windows の zip</a> ・ ' if rel["win_zip"] else ""
    return f"""
      <div class="card">
        <div class="release"><strong>{e(c["title"])} {e(rel["version"])}</strong><span>{e(rel["date"])} 公開 ・ {len(c["plugins"])} 個のプラグイン</span></div>
        <p class="lead">{e(c["lead"])}</p>
        <div class="buttons">
          <a class="btn primary" data-os="win" href="{e(win)}"><b>Windows</b><small>インストーラー（.exe）</small></a>
          <a class="btn" data-os="mac" href="{e(mac)}"><b>macOS</b><small>インストーラー（.pkg）</small></a>
        </div>
        <p class="alt">{zip_link}<a href="{e(rel["url"])}">このリリースのすべてのファイル</a> ・ <a href="{gh_url}/releases">前の版</a></p>
      </div>"""


def plugin_card(p):
    img = (
        f'<a href="{e(p["readme"])}"><img src="{e(p["image"])}" alt="{e(p["name"])} のサンプル" loading="lazy"></a>'
        if p["image"]
        else ""
    )
    return f"""
      <article class="card plugin{'' if p['image'] else ' noimg'}">
        {img}
        <div>
          <h4><a href="{e(p["readme"])}">{e(p["name"])}</a><span class="ver">{e(p["version"])}</span></h4>
          <p>{e(p["desc"])}</p>
          <div class="links">
            <a href="{e(p["readme"])}">説明・パラメータ</a>
            <a href="{e(p["download"])}/{e(p["stem"])}.aex">Windows 単体（.aex）</a>
            <a href="{e(p["download"])}/{e(p["stem"])}.plugin.zip">macOS 単体（.plugin.zip）</a>
            <a href="{e(p["tag_url"])}">この版のリリース</a>
          </div>
        </div>
      </article>"""


def render(collections):
    downloads = "".join(download_card(c) for c in collections)
    groups = "".join(
        f'\n      <h3 class="group">{e(c["title"])}</h3>\n      <div class="stack">'
        + "".join(plugin_card(p) for p in c["plugins"])
        + "\n      </div>"
        for c in collections
    )
    names = " / ".join(
        f"「{c['title']} (After Effects)」" for c in collections
    )
    return f"""<!doctype html>
<html lang="ja">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>JK-Plugins</title>
<meta name="description" content="After Effects 用プラグイン JK-Plugins・fs-rs のダウンロード">
<style>{CSS}</style>
</head>
<body>
<div class="wrap">
  <header>
    <h1>JK-Plugins</h1>
    <p>Rust で作った After Effects 用プラグイン集。インストーラーでまとめて入ります。</p>
  </header>

  <section>
    <h2>ダウンロード</h2>
    <div class="stack">{downloads}
    </div>
  </section>

  <section>
    <h2>プラグイン</h2>{groups}
  </section>

  <section>
    <h2>インストール</h2>
    <div class="os">
      <div class="card">
        <h3>Windows</h3>
        <ol>
          <li>After Effects を終了して、インストーラーを実行</li>
          <li>「Windows によって PC が保護されました」と出たら「詳細情報」→「実行」（署名していないため）</li>
          <li><code>MediaCore\\JK-Plugins</code> に入ります</li>
        </ol>
        <p class="alt">アンインストール：設定 → アプリ → インストールされているアプリ → {e(names)}</p>
      </div>
      <div class="card">
        <h3>macOS</h3>
        <ol>
          <li>After Effects を終了して、pkg を開く</li>
          <li>開けないときは システム設定 → プライバシーとセキュリティ の「このまま開く」</li>
          <li><code>MediaCore/JK-Plugins</code> に入ります</li>
        </ol>
        <p class="alt">アンインストール：<code>/Library/Application Support/Adobe/Common/Plug-ins/7.0/MediaCore/JK-Plugins</code> の中の .plugin を削除</p>
      </div>
    </div>
    <p class="alt">アップデートは、新しいインストーラーをそのまま実行すれば上書きされます。それぞれのインストーラーは別々に入れたり消したりできます。</p>
  </section>

  <footer>
    {" ・ ".join(f'<a href="https://github.com/{c["repo"]}">{e(c["title"])} (GitHub)</a>' for c in collections)} ・ <a href="https://github.com/{SELF}/issues">不具合の報告</a>
  </footer>
</div>
<script>{SCRIPT}</script>
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
