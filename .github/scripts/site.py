"""配布サイト（GitHub Pages）を作る。

  python .github/scripts/site.py --out _site

- プラグインの一覧（名前・バージョン・説明・説明ページ）は README.md の「プラグイン一覧」の表から読む
  （リリースの計画でバージョンが Cargo.toml と合っていることを確かめている）
- ダウンロードのリンクは最新のまとめリリース（gh release view）から作る
- 画像はプラグインのフォルダーの docs/samples/00_overview.png があれば使う
"""
import argparse
import html
import json
import re
import shutil
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
REPO = "JK-Plugins/JK-Plugins"
GITHUB = f"https://github.com/{REPO}"
ROW = re.compile(
    r"^\|\s*\[(?P<name>[^\]]+)\]\((?P<dir>[^)]+)/README\.md\)\s*\|"
    r"\s*\[(?P<version>[^\]]+)\]\((?P<tag_url>[^)]+)\)\s*\|"
    r"\s*(?P<desc>.+?)\s*\|\s*$"
)


def gh_json(*args):
    out = subprocess.run(["gh", *args], cwd=ROOT, check=True, capture_output=True, text=True, encoding="utf-8")
    return json.loads(out.stdout)


def plugins_from_readme():
    text = (ROOT / "README.md").read_text(encoding="utf-8")
    out = []
    for line in text.splitlines():
        m = ROW.match(line.strip())
        if m:
            d = m.groupdict()
            d["archive_tag"] = d["tag_url"].rsplit("/", 1)[-1]
            d["stem"] = d["name"].replace(" ", "-")
            out.append(d)
    if not out:
        raise SystemExit("README.md にプラグイン一覧の行が見つからない")
    return out


def latest_release():
    rel = gh_json("release", "view", "--repo", REPO, "--json", "tagName,name,publishedAt,url,assets")
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
.card { background: var(--surface); border: 1px solid var(--line); border-radius: 14px; padding: 22px; }
.release { display: flex; flex-wrap: wrap; gap: 6px 16px; align-items: baseline; margin-bottom: 16px; }
.release strong { font-size: 18px; }
.release span { color: var(--muted); font-size: 14px; }
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
.plugins { display: grid; gap: 16px; }
.plugin { display: grid; grid-template-columns: minmax(0, 260px) 1fr; gap: 20px; align-items: start; }
.plugin img { width: 100%; border-radius: 10px; border: 1px solid var(--line); display: block; }
.plugin h3 { margin: 0; font-size: 19px; }
.plugin h3 a { color: var(--text); text-decoration: none; }
.plugin h3 a:hover { text-decoration: underline; }
.ver { display: inline-block; margin-left: 8px; font-size: 13px; padding: 1px 8px; border-radius: 999px;
  background: var(--accent-soft); color: var(--accent); vertical-align: 2px; }
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
  var box = document.getElementById("buttons");
  if (!box) return;
  var first = box.querySelector(mac ? "[data-os=mac]" : "[data-os=win]");
  var other = box.querySelector(mac ? "[data-os=win]" : "[data-os=mac]");
  if (first && other) {
    box.insertBefore(first, box.firstChild);
    first.classList.add("primary");
    other.classList.remove("primary");
  }
})();
"""


def render(plugins, rel):
    def plugin_card(p):
        readme = f"{GITHUB}/blob/main/{p['dir']}/README.md"
        img = f'<a href="{e(readme)}"><img src="img/{e(p["dir"])}.png" alt="{e(p["name"])} のサンプル" loading="lazy"></a>' if p.get("image") else ""
        tag = f"{GITHUB}/releases/download/{p['archive_tag']}"
        return f"""
      <article class="card plugin">
        {img}
        <div>
          <h3><a href="{e(readme)}">{e(p["name"])}</a><span class="ver">{e(p["version"])}</span></h3>
          <p>{e(p["desc"])}</p>
          <div class="links">
            <a href="{e(readme)}">説明・パラメータ</a>
            <a href="{e(tag)}/{e(p["stem"])}.aex">Windows 単体（.aex）</a>
            <a href="{e(tag)}/{e(p["stem"])}.plugin.zip">macOS 単体（.plugin.zip）</a>
            <a href="{e(p["tag_url"])}">この版のリリース</a>
          </div>
        </div>
      </article>"""

    cards = "".join(plugin_card(p) for p in plugins)
    win = rel["win_setup"] or rel["url"]
    mac = rel["mac_pkg"] or rel["url"]
    zip_link = f'<a href="{e(rel["win_zip"])}">Windows の zip</a>' if rel["win_zip"] else ""
    return f"""<!doctype html>
<html lang="ja">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>JK-Plugins</title>
<meta name="description" content="After Effects 用プラグイン JK-Plugins のダウンロード">
<style>{CSS}</style>
</head>
<body>
<div class="wrap">
  <header>
    <h1>JK-Plugins</h1>
    <p>Rust で作った After Effects 用プラグイン集。インストーラーで全部まとめて入ります。</p>
  </header>

  <section>
    <h2>ダウンロード</h2>
    <div class="card">
      <div class="release"><strong>最新版 {e(rel["version"])}</strong><span>{e(rel["date"])} 公開</span></div>
      <div class="buttons" id="buttons">
        <a class="btn primary" data-os="win" href="{e(win)}"><b>Windows</b><small>インストーラー（.exe）</small></a>
        <a class="btn" data-os="mac" href="{e(mac)}"><b>macOS</b><small>インストーラー（.pkg）</small></a>
      </div>
      <p class="alt">{zip_link}{" ・ " if zip_link else ""}<a href="{e(rel["url"])}">このリリースのすべてのファイル</a> ・ <a href="{GITHUB}/releases">前の版</a></p>
    </div>
  </section>

  <section>
    <h2>プラグイン</h2>
    <div class="plugins">{cards}
    </div>
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
        <p class="alt">アンインストール：設定 → アプリ → インストールされているアプリ → JK-Plugins (After Effects)</p>
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
    <p class="alt">アップデートは、新しいインストーラーをそのまま実行すれば上書きされます。</p>
  </section>

  <footer>
    <a href="{GITHUB}">GitHub</a> ・ <a href="{GITHUB}/releases">リリース一覧</a> ・ <a href="{GITHUB}/issues">不具合の報告</a>
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
    plugins = plugins_from_readme()
    for p in plugins:
        img = ROOT / p["dir"] / "docs" / "samples" / "00_overview.png"
        if img.exists():
            shutil.copy2(img, out / "img" / f"{p['dir']}.png")
            p["image"] = True
    rel = latest_release()
    (out / "index.html").write_text(render(plugins, rel), encoding="utf-8")
    (out / ".nojekyll").write_text("", encoding="utf-8")
    print(f"{out / 'index.html'}: {len(plugins)} plug-ins, release {rel['tag']}")


if __name__ == "__main__":
    main()
