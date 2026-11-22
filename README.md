# JK-Plugins

## About

Those plugins are AE Plugins written by Rust.

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
- まとめリリースには、Windows のインストーラー（`-windows-setup.exe`、Inno Setup。MediaCore に全部入れ、
  「インストールされているアプリ」からアンインストールできる）と zip（全プラグインの `.aex`）、
  macOS の pkg（MediaCore に全部入れる）、プラグインごとの `.aex` / `.plugin.zip` を置きます
- インストーラーも pkg も署名していません。Windows で「Windows によって PC が保護されました」と出たら
  「詳細情報」→「実行」、macOS ではシステム設定 → プライバシーとセキュリティ の「このまま開く」で進めます
- バージョンが新しいプラグインだけ、`<crate>-v<version>`（例：`tonegradient-v0.1.0`）の個別アーカイブも作ります
- プラグインのバージョンは各 `Cargo.toml` の `version` だけで管理します（build.rs が読んで AE のバージョンにします。
  AE の制限で minor と patch は 0〜15）
- 前回のアーカイブからプラグイン（とそれが使うワークスペース内の crate）が変わっているのに `version` が同じだと、
  上げ忘れとしてリリースが止まります

手元で計画だけ確かめるには：

```sh
python .github/scripts/release.py plan --out dist   # dist/notes.md と dist/plan.json ができる
# Windows のファイルとインストーラーを作るには Inno Setup 6 が要る
python .github/scripts/release.py package-windows --plan dist/plan.json --out dist
```
