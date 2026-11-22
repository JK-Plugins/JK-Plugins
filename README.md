# JK-Plugins

## About

Those plugins are AE Plugins written by Rust.

**ダウンロード：https://jk-plugins.github.io/JK-Plugins/**

## プラグイン一覧

| プラグイン | バージョン | 説明 |
|---|---|---|
| [JK Tone Gradient](tonegradient/README.md) | [0.1.3](https://github.com/JK-Plugins/JK-Plugins/releases/tag/tonegradient-v0.1.3) | 指定した色どうしの境界をグラデーションにして、網点・線・ディザ・ノイズなどの質感をつける |

プラグイン名を押すと、それぞれの説明（パラメータ・使い方）を見られます。
バージョンを押すと、その版のファイル（Windows：`.aex`、macOS：`.plugin.zip`）のリリースに飛びます。

## インストール

[最新のリリース](https://github.com/JK-Plugins/JK-Plugins/releases/latest) からダウンロードします。
全部のプラグインがまとめて入ります。After Effects は終了しておいてください。

### Windows

1. `JK-Plugins-<日付>-windows-setup.exe` を実行する
2. 「Windows によって PC が保護されました」と出たら「詳細情報」→「実行」を押す（署名していないため）
3. `C:\Program Files\Adobe\Common\Plug-ins\7.0\MediaCore\JK-Plugins` に入る

インストーラーを使わないときは、`JK-Plugins-<日付>-windows.zip` を解凍し、中の `JK-Plugins` フォルダーを
`C:\Program Files\Adobe\Common\Plug-ins\7.0\MediaCore\` にコピーします。

アンインストールは「設定 → アプリ → インストールされているアプリ」の **JK-Plugins (After Effects)** から。

### macOS

1. `JK-Plugins-<日付>-macos.pkg` を開く
2. 署名していないので最初は開けない。システム設定 → プライバシーとセキュリティ の「このまま開く」を押してから、もう一度開く
3. `/Library/Application Support/Adobe/Common/Plug-ins/7.0/MediaCore/JK-Plugins` に入る

アンインストールは、そのフォルダーの中の `.plugin` を消します。

### アップデート

新しいリリースのインストーラー（pkg）をそのまま実行すると上書きされます。

### プラグインごとのファイル

リリースには、プラグインごとのファイルも置いてあります（必要なものだけ入れたいとき・前の版に戻したいとき）。

- Windows：`<プラグイン名>.aex` を MediaCore の中の `JK-Plugins` フォルダーにコピーする
- macOS：`<プラグイン名>.plugin.zip` を解凍し、中の `.plugin` を同じ場所にコピーする

前の版のファイルは、プラグインごとのリリース（例：[`tonegradient-v0.1.0`](https://github.com/JK-Plugins/JK-Plugins/releases/tag/tonegradient-v0.1.0)）にあります。

## Prerequisites

### **LLVM**  

Download and install from  
https://github.com/llvm/llvm-project/releases

and you need to set `LIBCLANG_PATH` to the path of the `bin` directory in the LLVM installation.

For example, if you installed LLVM to `/path/to/llvm`, you would set:
```bash
# macOS
export LIBCLANG_PATH=/path/to/llvm/bin
# Windows (PowerShell)
setx LIBCLANG_PATH "C:\path\to\llvm\bin"
```

and, you need to set `PATH` to include the LLVM binaries:
```bash
# macOS
export PATH=/path/to/llvm/bin:$PATH
" # Windows (PowerShell)
setx PATH "C:\path\to\llvm\bin;$env:PATH"
# or, if you use LLVM Installer, you can set it optionally
```

### **Adobe After Effects SDK (May 2023)**  

Download from https://console.adobe.io/downloads/ae
Place the SDK folder anywhere you like, then set:
```bash
# macOS
export AESDK_ROOT=/path/to/AfterEffects_SDK

# Windows (PowerShell)
setx AESDK_ROOT "C:\path\to\AfterEffects_SDK"
```

### **Rust**

Install Rust using [rustup](https://rustup.rs/).  

### **cargo-jk**

We are implementing our original build tool.

Install it with:
```bash
cargo install --git https://github.com/JK-Plugins/cargo-jk
```

## Build

Run the following command to build the project:
```bash
cd tonegradient
cargo jk install
# this is the same as `cargo jk build && cargo jk mv [plugin_path]`
```

## Release

GitHub の Actions → **Release** → **Run workflow** を押すと、全プラグインの最新版をビルドしてリリースします。

- タグは今日の日付（日本時間）で自動で付きます：`vYYYY.MM.DD`（同じ日の2回目以降は `.2`, `.3` …）。
  手で `vYYYY.MM.DD` のタグを push しても同じように動きます
- まとめリリースには、Windows のインストーラー（`-windows-setup.exe`、Inno Setup。
  「インストールされているアプリ」からアンインストールできる）と zip（全プラグインの `.aex`）、
  macOS の pkg、プラグインごとの `.aex` / `.plugin.zip` を置きます
- インストーラーと pkg は、プラグインを MediaCore の中の `JK-Plugins` フォルダーにまとめて入れます
  （Windows：`C:\Program Files\Adobe\Common\Plug-ins\7.0\MediaCore\JK-Plugins`、
  macOS：`/Library/Application Support/Adobe/Common/Plug-ins/7.0/MediaCore/JK-Plugins`）。
  前の版が MediaCore の直下に入れたものは、同じプラグインが2つ読み込まれないよう消します
- インストーラーも pkg も署名していません。Windows で「Windows によって PC が保護されました」と出たら
  「詳細情報」→「実行」、macOS ではシステム設定 → プライバシーとセキュリティ の「このまま開く」で進めます
- バージョンが新しいプラグインだけ、`<crate>-v<version>`（例：`tonegradient-v0.1.0`）の個別アーカイブも作ります
- プラグインのバージョンは各 `Cargo.toml` の `version` だけで管理します（build.rs が読んで AE のバージョンにします。
  AE の制限で minor と patch は 0〜15）
- 前回のアーカイブからプラグイン（とそれが使うワークスペース内の crate）が変わっているのに `version` が同じだと、
  上げ忘れとしてリリースが止まります
- リリースのあと、配布サイト（GitHub Pages）も自動で作り直します（`.github/workflows/pages.yml`。
  中身は JK-Plugins と [fs-rs](https://github.com/JK-Plugins/fs-rs) の README のプラグイン一覧と最新のリリースから
  `.github/scripts/site.py` が作る。fs-rs のリリースは6時間ごとの作り直しで反映される）
- リポジトリごとの名前やインストーラーの ID は `.github/release.json`（fs-rs と同じ `release.py` を使う）
- `version` を上げたら、この README のプラグイン一覧のバージョン（`[版](…/releases/tag/<crate>-v<版>)`）も直します。
  合っていないとリリースが止まります。新しいプラグインを足したときは一覧に行を足します

手元で計画だけ確かめるには：

```sh
python .github/scripts/release.py plan --out dist   # dist/notes.md と dist/plan.json ができる
# Windows のファイルとインストーラーを作るには Inno Setup 6 が要る
python .github/scripts/release.py package-windows --plan dist/plan.json --out dist
```
