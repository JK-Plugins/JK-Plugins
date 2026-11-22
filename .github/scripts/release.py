"""JK-Plugins のリリース用スクリプト（GitHub Actions から呼ぶ。手元でも dry run できる）。

サブコマンド
  plan [--tag vYYYY.MM.DD[.N]] [--out DIR]
      リリースの計画を作る：まとめリリースのタグ、プラグインの一覧とバージョン、
      個別アーカイブを作るプラグイン、リリースノート。バージョンの上げ忘れがあれば失敗する。
  package-windows --plan DIR/plan.json --out DIR
      各プラグインをビルドして .aex と、まとめの zip を作る。
  package-macos --plan DIR/plan.json --out DIR
      各プラグインを Intel + Apple Silicon でビルドして .plugin を組み立て、
      ad-hoc 署名し、個別の .plugin.zip と、まとめの .pkg を作る。

方針（README の「リリース」も参照）
- プラグインのバージョンは各 Cargo.toml の version だけで管理する
- まとめリリースは日付のタグ（vYYYY.MM.DD、同じ日の2回目以降は .2, .3 …）
- バージョンが新しいプラグインだけ、<crate>-v<version> の個別アーカイブを作る
- 前回のアーカイブからプラグイン（とそれが使うワークスペース内の crate）が変わっているのに
  バージョンが同じなら、上げ忘れとして失敗する
"""
import argparse
import datetime
import json
import os
import plistlib
import re
import shutil
import subprocess
import sys
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
BUNDLE_TAG = re.compile(r"^v(\d{4})\.(\d{2})\.(\d{2})(?:\.(\d+))?$")
MEDIACORE_MAC = "/Library/Application Support/Adobe/Common/Plug-ins/7.0/MediaCore"


def run(*args, cwd=ROOT, check=True, capture=True):
    r = subprocess.run(args, cwd=cwd, check=False, text=True, capture_output=capture)
    if check and r.returncode != 0:
        raise SystemExit(f"command failed ({r.returncode}): {' '.join(map(str, args))}\n{r.stdout}\n{r.stderr}")
    return r.stdout if capture else ""


def git(*args, check=True):
    return run("git", *args, check=check).strip()


def tag_exists(tag):
    return bool(git("tag", "--list", tag))


# ---------------------------------------------------------------- プラグインの一覧


def plugins():
    """ワークスペースの中で [package.metadata.jk_plugin] を持つ crate"""
    meta = json.loads(run("cargo", "metadata", "--format-version", "1", "--no-deps"))
    members = set(meta["workspace_members"])
    by_dir = {}
    for pkg in meta["packages"]:
        by_dir[Path(pkg["manifest_path"]).parent.resolve()] = pkg["name"]
    out = []
    for pkg in meta["packages"]:
        if pkg["id"] not in members:
            continue
        jk = (pkg.get("metadata") or {}).get("jk_plugin")
        if not jk:
            continue
        crate_dir = Path(pkg["manifest_path"]).parent.resolve()
        # ワークスペース内の path 依存（libs など）も「このプラグインの中身」として扱う
        deps = [crate_dir]
        for d in pkg["dependencies"]:
            if d.get("path"):
                deps.append(Path(d["path"]).resolve())
        out.append(
            {
                "crate": pkg["name"],
                "lib": pkg["name"].replace("-", "_"),
                "version": pkg["version"],
                "plugin_name": jk["plugin_name"],
                "identifier": jk["identifier"],
                "dir": crate_dir.relative_to(ROOT).as_posix(),
                "watch": sorted({p.relative_to(ROOT).as_posix() for p in deps if ROOT in p.parents or p == ROOT}),
            }
        )
    out.sort(key=lambda p: p["crate"])
    return out


def asset_stem(plugin):
    """ダウンロード用のファイル名（GitHub は空白を . に変えるのでハイフンにする）"""
    return plugin["plugin_name"].replace(" ", "-")


# ---------------------------------------------------------------- plan


def next_bundle_tag(today=None):
    today = today or datetime.datetime.now(datetime.timezone(datetime.timedelta(hours=9))).date()
    base = f"v{today:%Y.%m.%d}"
    if not tag_exists(base):
        return base
    n = 2
    while tag_exists(f"{base}.{n}"):
        n += 1
    return f"{base}.{n}"


def bundle_tags():
    tags = [t for t in git("tag", "--list", "v*").splitlines() if BUNDLE_TAG.match(t)]

    def key(t):
        m = BUNDLE_TAG.match(t)
        return tuple(int(g or 1) for g in m.groups())

    return sorted(tags, key=key)


def version_at(ref, plugin):
    """ref の時点の Cargo.toml の version（無ければ None）"""
    r = subprocess.run(
        ["git", "show", f"{ref}:{plugin['dir']}/Cargo.toml"], cwd=ROOT, text=True, capture_output=True
    )
    if r.returncode != 0:
        return None
    in_package = False
    for line in r.stdout.splitlines():
        s = line.strip()
        if s.startswith("["):
            in_package = s == "[package]"
        elif in_package:
            m = re.match(r'version\s*=\s*"([^"]+)"', s)
            if m:
                return m.group(1)
    return None


def changed_since(ref, paths):
    r = subprocess.run(["git", "diff", "--quiet", ref, "HEAD", "--", *paths], cwd=ROOT)
    return r.returncode != 0


def cmd_plan(args):
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    tag = args.tag or next_bundle_tag()
    m = BUNDLE_TAG.match(tag)
    if not m:
        raise SystemExit(f"bundle tag must look like vYYYY.MM.DD or vYYYY.MM.DD.N: {tag}")
    bundle_version = tag[1:]
    previous = [t for t in bundle_tags() if t != tag]
    prev_tag = previous[-1] if previous else None

    plist = plugins()
    if not plist:
        raise SystemExit("no plug-ins with [package.metadata.jk_plugin] found")
    errors = []
    for p in plist:
        p["archive_tag"] = f"{p['crate']}-v{p['version']}"
        p["asset_stem"] = asset_stem(p)
        if tag_exists(p["archive_tag"]):
            p["new_version"] = False
            if changed_since(p["archive_tag"], p["watch"]):
                errors.append(
                    f"{p['crate']}: {', '.join(p['watch'])} changed since {p['archive_tag']} "
                    f"but version is still {p['version']}. Bump version in {p['dir']}/Cargo.toml."
                )
        else:
            p["new_version"] = True
        p["previous_version"] = version_at(prev_tag, p) if prev_tag else None
    if errors:
        raise SystemExit("version check failed:\n  " + "\n  ".join(errors))

    # リリースノート
    updated = [p for p in plist if p["previous_version"] != p["version"]]
    same = [p for p in plist if p["previous_version"] == p["version"]]
    lines = [f"# JK-Plugins {bundle_version}", ""]
    if updated:
        lines.append("## 更新")
        for p in updated:
            if p["previous_version"]:
                lines.append(f"- {p['plugin_name']} {p['previous_version']} → {p['version']}")
            else:
                lines.append(f"- {p['plugin_name']} {p['version']}（新規）")
        lines.append("")
    if same:
        lines.append("## 変更なし")
        for p in same:
            lines.append(f"- {p['plugin_name']} {p['version']}")
        lines.append("")
    lines += [
        "## インストール",
        "",
        f"- **Windows**: `JK-Plugins-{bundle_version}-windows.zip` を解凍し、中の `.aex` を "
        "`C:\\Program Files\\Adobe\\Common\\Plug-ins\\7.0\\MediaCore\\` にコピーする",
        f"- **macOS**: `JK-Plugins-{bundle_version}-macos.pkg` を開く。署名していないので最初は開けないため、"
        "システム設定 → プライバシーとセキュリティ の「このまま開く」を押してからもう一度開く",
        "",
        "プラグインごとのファイル（`.aex` / `.plugin.zip`）も同じリリースに置いてある。",
    ]
    if prev_tag:
        lines += ["", f"前回のまとめリリース：{prev_tag}"]
    notes = "\n".join(lines) + "\n"
    (out / "notes.md").write_text(notes, encoding="utf-8")

    plan = {
        "tag": tag,
        "bundle_version": bundle_version,
        "previous_tag": prev_tag,
        "sha": git("rev-parse", "HEAD"),
        "plugins": plist,
    }
    (out / "plan.json").write_text(json.dumps(plan, indent=2, ensure_ascii=False), encoding="utf-8")
    print(notes)
    print(json.dumps({k: v for k, v in plan.items() if k != "plugins"}, indent=2))
    for p in plist:
        print(f"{p['crate']} {p['version']} new_version={p['new_version']} watch={p['watch']}")
    gh_out = os.environ.get("GITHUB_OUTPUT")
    if gh_out:
        with open(gh_out, "a", encoding="utf-8") as f:
            f.write(f"tag={tag}\nbundle_version={bundle_version}\n")


# ---------------------------------------------------------------- Windows


def cmd_package_windows(args):
    plan = json.loads(Path(args.plan).read_text(encoding="utf-8"))
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    target = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
    bundle = out / f"JK-Plugins-{plan['bundle_version']}-windows.zip"
    with zipfile.ZipFile(bundle, "w", zipfile.ZIP_DEFLATED) as z:
        for p in plan["plugins"]:
            run("cargo", "build", "--release", "-p", p["crate"], capture=False)
            dll = target / "release" / f"{p['lib']}.dll"
            # まとめの zip の中は AE に入れるときの名前（cargo jk と同じ）
            z.write(dll, f"{p['plugin_name']}.aex")
            # 個別のファイル（GitHub は空白を . に変えるのでハイフンの名前）
            shutil.copy2(dll, out / f"{p['asset_stem']}.aex")
            print(f"built {p['plugin_name']}.aex")
    print(bundle)


# ---------------------------------------------------------------- macOS


def build_bundle(p, target, out):
    """Intel + Apple Silicon のユニバーサルな .plugin を組み立てて ad-hoc 署名する"""
    triples = ["x86_64-apple-darwin", "aarch64-apple-darwin"]
    for t in triples:
        run("cargo", "build", "--release", "-p", p["crate"], "--target", t, capture=False)
    name = p["plugin_name"]
    bundle = out / f"{name}.plugin"
    if bundle.exists():
        shutil.rmtree(bundle)
    (bundle / "Contents" / "MacOS").mkdir(parents=True)
    (bundle / "Contents" / "Resources").mkdir(parents=True)
    run(
        "lipo",
        "-create",
        *[str(target / t / "release" / f"lib{p['lib']}.dylib") for t in triples],
        "-output",
        str(bundle / "Contents" / "MacOS" / name),
    )
    shutil.copy2(target / triples[0] / "release" / f"{p['crate']}.rsrc", bundle / "Contents" / "Resources" / f"{name}.rsrc")
    (bundle / "Contents" / "PkgInfo").write_text("eFKTFXTC", encoding="ascii")
    with open(bundle / "Contents" / "Info.plist", "wb") as f:
        plistlib.dump(
            {
                "CFBundlePackageType": "eFKT",
                "CFBundleSignature": "FXTC",
                "CFBundleIdentifier": p["identifier"],
                "CFBundleExecutable": name,
                "CFBundleName": name,
                "CFBundleShortVersionString": p["version"],
                "CFBundleVersion": p["version"],
            },
            f,
        )
    # Apple Silicon は署名が無いと読み込めないので ad-hoc 署名だけしておく
    run("codesign", "--force", "--deep", "--sign", "-", str(bundle))
    return bundle


def cmd_package_macos(args):
    plan = json.loads(Path(args.plan).read_text(encoding="utf-8"))
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    target = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
    run("rustup", "target", "add", "x86_64-apple-darwin", "aarch64-apple-darwin", capture=False)
    root = out / "pkgroot"
    if root.exists():
        shutil.rmtree(root)
    root.mkdir()
    for p in plan["plugins"]:
        bundle = build_bundle(p, target, root)
        # 個別のファイル：中は AE に入れるときの名前の .plugin（ditto でバンドルの構造を保つ）
        run(
            "ditto",
            "-c",
            "-k",
            "--sequesterRsrc",
            "--keepParent",
            str(bundle),
            str(out / f"{p['asset_stem']}.plugin.zip"),
        )
        print(f"built {bundle.name}")

    # まとめの pkg：MediaCore に全部入れ、インストール後に ad-hoc 署名し直す（ntsc-rs と同じ）
    scripts = out / "scripts"
    if scripts.exists():
        shutil.rmtree(scripts)
    scripts.mkdir()
    names = " ".join(f'"{p["plugin_name"]}.plugin"' for p in plan["plugins"])
    post = scripts / "postinstall"
    post.write_text(
        "#!/bin/sh\n"
        "# 署名していないプラグインを Apple Silicon で読み込めるよう、インストール先で ad-hoc 署名する\n"
        f'DEST="${{2:-${{DSTROOT:-{MEDIACORE_MAC}}}}}"\n'
        f"for p in {names}; do\n"
        '  if [ -d "$DEST/$p" ]; then /usr/bin/codesign --force --deep --sign - "$DEST/$p"; fi\n'
        "done\n"
        "exit 0\n",
        encoding="utf-8",
    )
    post.chmod(0o755)
    comp = out / "component.plist"
    run("pkgbuild", "--analyze", "--root", str(root), str(comp))
    with open(comp, "rb") as f:
        components = plistlib.load(f)
    for c in components:
        # 別の場所にある同じ ID のバンドルを探してそこへ入れる動きを止める
        c["BundleIsRelocatable"] = False
    with open(comp, "wb") as f:
        plistlib.dump(components, f)
    pkg = out / f"JK-Plugins-{plan['bundle_version']}-macos.pkg"
    run(
        "pkgbuild",
        "--root",
        str(root),
        "--component-plist",
        str(comp),
        "--install-location",
        MEDIACORE_MAC,
        "--scripts",
        str(scripts),
        "--identifier",
        "com.jk-plugins.bundle",
        "--version",
        plan["bundle_version"],
        "--min-os-version",
        "11.0",
        str(pkg),
    )
    print(pkg)


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("plan")
    p.add_argument("--tag")
    p.add_argument("--out", default="dist")
    p.set_defaults(func=cmd_plan)
    for name, func in [("package-windows", cmd_package_windows), ("package-macos", cmd_package_macos)]:
        q = sub.add_parser(name)
        q.add_argument("--plan", required=True)
        q.add_argument("--out", default="dist")
        q.set_defaults(func=func)
    args = ap.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
