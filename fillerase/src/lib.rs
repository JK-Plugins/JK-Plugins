//! JK Fill Erase
//!
//! 指定した点から、塗りつぶしツールと同じように似た色でつながっている範囲を求めて消す。
//!
//! - `engine`：AEに依存しないコア処理（Linuxでもテスト可）
//! - `plugin`：After Effects 向けの組み込み（Windows / macOS のみ）

pub mod engine;
/// 1フレームの処理（キャッシュつき）。AE に依存しないのでテストできる
pub mod frame;
/// ワールド（画像バッファ）の読み書き。AE に依存しないのでテストできる
pub mod world_io;

#[cfg(any(windows, target_os = "macos"))]
mod plugin;
