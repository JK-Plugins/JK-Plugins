//! JK Tone Gradient
//!
//! 指定した色グループ（最大32色）の中で、色と色の境界をグラデーションにし、
//! しきい値マップ（網点・線・ディザ・ノイズ・任意レイヤー）で質感をつける。
//!
//! - `engine`：AEに依存しないコア処理（Linuxでもテスト可）
//! - `plugin`：After Effects 向けの組み込み（Windows / macOS のみ）

pub mod engine;

#[cfg(any(windows, target_os = "macos"))]
mod plugin;
