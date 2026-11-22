//! AEに依存しないコア処理。単体でテストできる。
pub mod curve;
pub mod edt;
#[cfg(any(windows, target_os = "macos"))]
pub mod gpu;
pub mod process;
pub mod texture;

pub use process::{
    prepare, prepare_key, process, reach, shade, Curve, Dense, GroupColor, MidPosition, Output,
    OutputMode, Prepared, Settings, TextureMode, MAX_COLORS, MAX_NEIGHBORS, NONE,
};
pub use texture::{DotShape, LayerTexture, Texture, TextureKind, TextureParams};
