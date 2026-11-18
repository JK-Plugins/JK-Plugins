//! AEに依存しないコア処理。単体でテストできる。
pub mod edt;
pub mod process;
pub mod texture;

pub use process::{
    process, Curve, GroupColor, Output, OutputMode, Settings, MAX_COLORS, MAX_NEIGHBORS, NONE,
};
pub use texture::{DotShape, LayerTexture, Texture, TextureKind, TextureParams};
