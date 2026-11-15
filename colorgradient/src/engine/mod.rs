//! AEに依存しないコア処理。単体でテストできる。
pub mod edt;
pub mod process;
pub mod texture;

pub use process::{process, Curve, Output, OutputMode, Settings, MAX_COLORS, NONE};
pub use texture::{LayerTexture, Texture, TextureKind, TextureParams};
