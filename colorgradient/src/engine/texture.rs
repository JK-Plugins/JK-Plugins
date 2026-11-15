//! しきい値マップ（テクスチャ）。
//!
//! どのテクスチャも座標 (x, y) から 0 以上 1 未満のしきい値 `t` を返す。
//! グラデーションの値 `v` に対して `t < v` なら上側の色になる。
//! `t` の分布がおおよそ一様なら、`v` の割合でその色が出る（網点の面積率と同じ考え方）。

use std::f32::consts::TAU;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextureKind {
    /// テクスチャなし（なめらかなグラデーション）
    Smooth,
    /// 網点（丸）
    Dot,
    /// 線スクリーン
    Line,
    /// 規則ディザ（Bayer 8×8）
    Bayer,
    /// ホワイトノイズ（砂目）
    Grain,
    /// バリューノイズ（ムラ・紙目）
    Noise,
    /// 任意レイヤーの輝度
    Layer,
}

/// レイヤーテクスチャ（輝度 0..1、行優先）。
#[derive(Clone, Copy)]
pub struct LayerTexture<'a> {
    pub width: usize,
    pub height: usize,
    pub luma: &'a [f32],
    /// テクスチャワールドの原点（描画座標系でのオフセット）
    pub origin: (i32, i32),
}

#[derive(Clone, Copy)]
pub struct TextureParams {
    pub kind: TextureKind,
    /// セルの大きさ（px）。網点・線の1周期、Bayerのタイル、ノイズの粒の大きさ。
    pub cell: f32,
    /// 角度（度）。網点・線・ノイズを回転する。
    pub angle_deg: f32,
    pub seed: u32,
    pub invert: bool,
}

impl Default for TextureParams {
    fn default() -> Self {
        Self {
            kind: TextureKind::Smooth,
            cell: 8.0,
            angle_deg: 45.0,
            seed: 0,
            invert: false,
        }
    }
}

const BAYER8: [u8; 64] = [
    0, 32, 8, 40, 2, 34, 10, 42, //
    48, 16, 56, 24, 50, 18, 58, 26, //
    12, 44, 4, 36, 14, 46, 6, 38, //
    60, 28, 52, 20, 62, 30, 54, 22, //
    3, 35, 11, 43, 1, 33, 9, 41, //
    51, 19, 59, 27, 49, 17, 57, 25, //
    15, 47, 7, 39, 13, 45, 5, 37, //
    63, 31, 55, 23, 61, 29, 53, 21,
];

#[inline]
pub fn hash2(x: i32, y: i32, seed: u32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343)
        ^ (y as u32).wrapping_mul(0xd816_3841)
        ^ seed.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    h
}

#[inline]
fn hash_f(x: i32, y: i32, seed: u32) -> f32 {
    (hash2(x, y, seed) >> 8) as f32 / (1u32 << 24) as f32
}

#[inline]
fn value_noise(x: f32, y: f32, seed: u32) -> f32 {
    let xi = x.floor();
    let yi = y.floor();
    let fx = x - xi;
    let fy = y - yi;
    let (xi, yi) = (xi as i32, yi as i32);
    let sx = fx * fx * (3.0 - 2.0 * fx);
    let sy = fy * fy * (3.0 - 2.0 * fy);
    let a = hash_f(xi, yi, seed);
    let b = hash_f(xi + 1, yi, seed);
    let c = hash_f(xi, yi + 1, seed);
    let d = hash_f(xi + 1, yi + 1, seed);
    let ab = a + (b - a) * sx;
    let cd = c + (d - c) * sx;
    ab + (cd - ab) * sy
}

/// 3オクターブのfBm。0..1 付近に分布を広げて返す。
#[inline]
pub fn fbm(x: f32, y: f32, seed: u32) -> f32 {
    let n = value_noise(x, y, seed) * 0.5714
        + value_noise(x * 2.03 + 17.1, y * 2.03 - 9.3, seed ^ 0x9e37) * 0.2857
        + value_noise(x * 4.11 - 3.7, y * 4.11 + 5.9, seed ^ 0x7f4a) * 0.1429;
    // 中央に寄った分布をおおよそ一様に引き伸ばす
    ((n - 0.5) * 2.2 + 0.5).clamp(0.0, 1.0)
}

pub struct Texture<'a> {
    p: TextureParams,
    layer: Option<LayerTexture<'a>>,
    cos_a: f32,
    sin_a: f32,
    inv_cell: f32,
}

impl<'a> Texture<'a> {
    pub fn new(p: TextureParams, layer: Option<LayerTexture<'a>>) -> Self {
        let a = p.angle_deg.to_radians();
        let mut p = p;
        if p.kind == TextureKind::Layer && layer.map_or(true, |l| l.width == 0 || l.height == 0) {
            p.kind = TextureKind::Smooth;
        }
        Self {
            p,
            layer,
            cos_a: a.cos(),
            sin_a: a.sin(),
            inv_cell: 1.0 / p.cell.max(0.5),
        }
    }

    pub fn kind(&self) -> TextureKind {
        self.p.kind
    }

    /// 1ピクセル内でサンプル位置をずらす意味があるか（アンチエイリアス対象）
    pub fn is_continuous(&self) -> bool {
        matches!(
            self.p.kind,
            TextureKind::Dot | TextureKind::Line | TextureKind::Noise
        )
    }

    #[inline]
    fn rot(&self, x: f32, y: f32) -> (f32, f32) {
        (
            (x * self.cos_a + y * self.sin_a) * self.inv_cell,
            (-x * self.sin_a + y * self.cos_a) * self.inv_cell,
        )
    }

    /// 座標 (x, y)（描画座標、ピクセル中心は整数+0.5）のしきい値 0..1。
    #[inline]
    pub fn threshold(&self, x: f32, y: f32) -> f32 {
        let t = match self.p.kind {
            TextureKind::Smooth => 0.5,
            TextureKind::Dot => {
                let (u, v) = self.rot(x, y);
                // 丸点のスポット関数。セル中心(整数座標)が最も早く塗られる
                let s = 0.5 + 0.25 * ((TAU * u).cos() + (TAU * v).cos());
                1.0 - s
            }
            TextureKind::Line => {
                let (_, v) = self.rot(x, y);
                0.5 - 0.5 * (TAU * v).cos()
            }
            TextureKind::Bayer => {
                let e = (self.p.cell / 8.0).max(1.0);
                let bx = (x / e).floor() as i32;
                let by = (y / e).floor() as i32;
                let i = (by.rem_euclid(8) * 8 + bx.rem_euclid(8)) as usize;
                (BAYER8[i] as f32 + 0.5) / 64.0
            }
            TextureKind::Grain => {
                let e = (self.p.cell / 8.0).max(1.0);
                hash_f((x / e).floor() as i32, (y / e).floor() as i32, self.p.seed)
            }
            TextureKind::Noise => {
                let (u, v) = self.rot(x, y);
                fbm(u, v, self.p.seed)
            }
            TextureKind::Layer => {
                let l = self.layer.unwrap();
                let lx = (x.floor() as i32 - l.origin.0).rem_euclid(l.width as i32) as usize;
                let ly = (y.floor() as i32 - l.origin.1).rem_euclid(l.height as i32) as usize;
                l.luma[ly * l.width + lx]
            }
        };
        let t = if self.p.invert { 1.0 - t } else { t };
        t.clamp(0.0, 0.99999)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// しきい値の分布がおおよそ一様（= 面積率がvに比例する）か
    fn coverage(kind: TextureKind, v: f32) -> f32 {
        let tex = Texture::new(
            TextureParams {
                kind,
                cell: 8.0,
                angle_deg: 15.0,
                seed: 3,
                invert: false,
            },
            None,
        );
        let mut on = 0;
        let n = 256;
        for y in 0..n {
            for x in 0..n {
                if tex.threshold(x as f32 + 0.5, y as f32 + 0.5) < v {
                    on += 1;
                }
            }
        }
        on as f32 / (n * n) as f32
    }

    #[test]
    fn bayer_is_uniform() {
        for v in [0.1, 0.25, 0.5, 0.75, 0.9] {
            assert!((coverage(TextureKind::Bayer, v) - v).abs() < 0.02);
        }
    }

    #[test]
    fn dot_and_line_are_monotonic() {
        for kind in [
            TextureKind::Dot,
            TextureKind::Line,
            TextureKind::Noise,
            TextureKind::Grain,
        ] {
            let mut prev = -1.0;
            for i in 0..=10 {
                let c = coverage(kind, i as f32 / 10.0);
                assert!(c >= prev, "{kind:?}");
                prev = c;
            }
            assert!(coverage(kind, 0.0) == 0.0);
            assert!(
                (coverage(kind, 0.5) - 0.5).abs() < 0.12,
                "{kind:?} {}",
                coverage(kind, 0.5)
            );
        }
    }
}
