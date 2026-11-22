//! しきい値マップ（テクスチャ）。
//!
//! どのテクスチャも座標 (x, y) から 0 以上 1 未満のしきい値 `t` を返す。
//! 重み（面積率）`w` に対して `t < w` の場所がその色になる。
//! 網点・線は形ごとに累積分布表（LUT）で均一化しているので、面積率が重みに一致する。

use std::f32::consts::TAU;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextureKind {
    /// テクスチャなし（なめらかなグラデーション）
    Smooth,
    /// 網点（形は DotShape）
    Dot,
    /// 線スクリーン
    Line,
    /// 規則ディザ（Bayer 8×8）
    Bayer,
    /// ホワイトノイズ（砂目）
    Grain,
    /// バリューノイズ（ムラ・紙目）
    Noise,
    /// 任意レイヤー
    Layer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DotShape {
    /// 丸点（50%で市松につながる一般的な網点）
    Round,
    /// 円（ユークリッド距離）
    Circle,
    /// 楕円（Aspect で縦横比）
    Ellipse,
    /// 角点
    Square,
    /// ひし形
    Diamond,
}

/// レイヤーテクスチャ（値 0..1、行優先）。どのチャンネルを使うかは呼び出し側で選ぶ。
#[derive(Clone, Copy)]
pub struct LayerTexture<'a> {
    pub width: usize,
    pub height: usize,
    pub value: &'a [f32],
    /// テクスチャワールドの原点（描画座標系でのオフセット）
    pub origin: (i32, i32),
}

#[derive(Clone, Copy)]
pub struct TextureParams {
    pub kind: TextureKind,
    /// 網点・線の1周期、ノイズの粒の大きさ（px）
    pub cell: f32,
    /// 角度（度）。Dot・Line・Noise を回転する（Bayer・Grain・Layer は回転しない）
    pub angle_deg: f32,
    pub seed: u32,
    pub invert: bool,
    /// 位置のずらし（px）
    pub offset: (f32, f32),
    pub dot_shape: DotShape,
    /// 楕円の縦横比
    pub aspect: f32,
    /// Bayer・Grain の1粒の大きさ（px）
    pub dither_size: f32,
    /// ノイズの重ね数（1..6）
    pub octaves: u32,
    /// ノイズのコントラスト
    pub noise_contrast: f32,
    /// Layer の拡大率（1 = 等倍）
    pub layer_scale: f32,
    /// Layer のレベル補正（黒・白、0..1）
    pub layer_black: f32,
    pub layer_white: f32,
}

impl Default for TextureParams {
    fn default() -> Self {
        Self {
            kind: TextureKind::Smooth,
            cell: 8.0,
            angle_deg: 45.0,
            seed: 0,
            invert: false,
            offset: (0.0, 0.0),
            dot_shape: DotShape::Round,
            aspect: 2.0,
            dither_size: 1.0,
            octaves: 3,
            noise_contrast: 2.2,
            layer_scale: 1.0,
            layer_black: 0.0,
            layer_white: 1.0,
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

/// fBm。`octaves` 回重ね、`contrast` で中央に寄った分布を引き伸ばして 0..1 で返す。
#[inline]
pub fn fbm_n(x: f32, y: f32, seed: u32, octaves: u32, contrast: f32) -> f32 {
    let mut sum = 0.0;
    let mut amp = 1.0;
    let mut norm = 0.0;
    let (mut fx, mut fy) = (x, y);
    for o in 0..octaves.clamp(1, 6) {
        sum += value_noise(fx, fy, seed ^ (o.wrapping_mul(0x9e37_79b9))) * amp;
        norm += amp;
        amp *= 0.5;
        fx = fx * 2.03 + 17.1;
        fy = fy * 2.03 - 9.3;
    }
    ((sum / norm - 0.5) * contrast + 0.5).clamp(0.0, 1.0)
}

/// 境界の荒らし用（3オクターブ固定）
#[inline]
pub fn fbm(x: f32, y: f32, seed: u32) -> f32 {
    fbm_n(x, y, seed, 3, 2.2)
}

/// スポット関数の値 → 面積率 の累積分布表
struct Cdf {
    min: f32,
    inv_range: f32,
    table: Vec<f32>,
}

impl Cdf {
    const BINS: usize = 4096;
    const GRID: usize = 256;

    fn build(spot: impl Fn(f32, f32) -> f32) -> Self {
        let n = Self::GRID;
        let mut vals = Vec::with_capacity(n * n);
        for j in 0..n {
            for i in 0..n {
                let u = (i as f32 + 0.5) / n as f32 - 0.5;
                let v = (j as f32 + 0.5) / n as f32 - 0.5;
                vals.push(spot(u, v));
            }
        }
        let min = vals.iter().cloned().fold(f32::INFINITY, f32::min);
        let max = vals.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        let range = (max - min).max(1e-9);
        let mut hist = vec![0u32; Self::BINS];
        for &v in &vals {
            let b = (((v - min) / range) * Self::BINS as f32) as usize;
            hist[b.min(Self::BINS - 1)] += 1;
        }
        // table[b] = b番目の区間の始まりまでの累積割合（区間内は線形補間）
        let mut table = vec![0f32; Self::BINS + 1];
        let total = vals.len() as f32;
        let mut acc = 0u32;
        for b in 0..Self::BINS {
            table[b] = acc as f32 / total;
            acc += hist[b];
        }
        table[Self::BINS] = 1.0;
        Self {
            min,
            inv_range: 1.0 / range,
            table,
        }
    }

    #[inline]
    fn eval(&self, v: f32) -> f32 {
        let p = ((v - self.min) * self.inv_range).clamp(0.0, 1.0) * Self::BINS as f32;
        let b = (p as usize).min(Self::BINS - 1);
        let f = p - b as f32;
        self.table[b] + (self.table[b + 1] - self.table[b]) * f
    }
}

fn spot_fn(shape: DotShape, aspect: f32) -> impl Fn(f32, f32) -> f32 {
    move |u: f32, v: f32| match shape {
        DotShape::Round => -((TAU * u).cos() + (TAU * v).cos()),
        DotShape::Circle => u * u + v * v,
        DotShape::Ellipse => {
            let a = aspect.max(0.05);
            u * u / a + v * v * a
        }
        DotShape::Square => u.abs().max(v.abs()),
        DotShape::Diamond => u.abs() + v.abs(),
    }
}

/// GPU に渡すための中身（`Texture::gpu_view`）
pub(crate) struct TextureGpuView<'a> {
    pub p: TextureParams,
    pub cos_a: f32,
    pub sin_a: f32,
    pub inv_cell: f32,
    /// 網点・線の分布表（最小値、1/範囲、表）
    pub cdf: Option<(f32, f32, &'a [f32])>,
    pub layer: Option<LayerTexture<'a>>,
}

pub struct Texture<'a> {
    p: TextureParams,
    layer: Option<LayerTexture<'a>>,
    cos_a: f32,
    sin_a: f32,
    inv_cell: f32,
    cdf: Option<Cdf>,
}

impl<'a> Texture<'a> {
    pub fn new(p: TextureParams, layer: Option<LayerTexture<'a>>) -> Self {
        let a = p.angle_deg.to_radians();
        let mut p = p;
        if p.kind == TextureKind::Layer && layer.map_or(true, |l| l.width == 0 || l.height == 0) {
            p.kind = TextureKind::Smooth;
        }
        let cdf = match p.kind {
            TextureKind::Dot => Some(Cdf::build(spot_fn(p.dot_shape, p.aspect))),
            TextureKind::Line => Some(Cdf::build(|_, v| v.abs())),
            _ => None,
        };
        Self {
            p,
            layer,
            cos_a: a.cos(),
            sin_a: a.sin(),
            inv_cell: 1.0 / p.cell.max(0.5),
            cdf,
        }
    }

    pub fn kind(&self) -> TextureKind {
        self.p.kind
    }

    /// GPU に渡すための中身（回転・セル・分布表・レイヤー）
    pub(crate) fn gpu_view(&self) -> TextureGpuView<'_> {
        TextureGpuView {
            p: self.p,
            cos_a: self.cos_a,
            sin_a: self.sin_a,
            inv_cell: self.inv_cell,
            cdf: self
                .cdf
                .as_ref()
                .map(|c| (c.min, c.inv_range, c.table.as_slice())),
            layer: self.layer,
        }
    }

    /// 回転（ずらしを引いてから）。セル単位にはしない
    #[inline]
    fn rot(&self, x: f32, y: f32) -> (f32, f32) {
        let x = x - self.p.offset.0;
        let y = y - self.p.offset.1;
        (
            x * self.cos_a + y * self.sin_a,
            -x * self.sin_a + y * self.cos_a,
        )
    }

    /// 座標 (x, y)（描画座標、ピクセル中心は整数+0.5）のしきい値 0..1。
    #[inline]
    pub fn threshold(&self, x: f32, y: f32) -> f32 {
        let t = match self.p.kind {
            TextureKind::Smooth => 0.5,
            TextureKind::Dot | TextureKind::Line => {
                let (u, v) = self.rot(x, y);
                let (u, v) = (u * self.inv_cell, v * self.inv_cell);
                // セル内の位置 -0.5..0.5（セル中心から塗られる）
                let fu = u - u.round();
                let fv = v - v.round();
                let s = if self.p.kind == TextureKind::Line {
                    fv.abs()
                } else {
                    spot_fn(self.p.dot_shape, self.p.aspect)(fu, fv)
                };
                self.cdf.as_ref().unwrap().eval(s)
            }
            TextureKind::Bayer => {
                // 1粒が数px の格子を回すと、ピクセル格子とずれてモアレが出るので回転しない
                let (u, v) = (x - self.p.offset.0, y - self.p.offset.1);
                let e = self.p.dither_size.max(0.25);
                let bx = (u / e).floor() as i32;
                let by = (v / e).floor() as i32;
                let i = (by.rem_euclid(8) * 8 + bx.rem_euclid(8)) as usize;
                (BAYER8[i] as f32 + 0.5) / 64.0
            }
            TextureKind::Grain => {
                let (u, v) = (x - self.p.offset.0, y - self.p.offset.1);
                let e = self.p.dither_size.max(0.25);
                hash_f((u / e).floor() as i32, (v / e).floor() as i32, self.p.seed)
            }
            TextureKind::Noise => {
                let (u, v) = self.rot(x, y);
                fbm_n(
                    u * self.inv_cell,
                    v * self.inv_cell,
                    self.p.seed,
                    self.p.octaves,
                    self.p.noise_contrast,
                )
            }
            TextureKind::Layer => {
                let l = self.layer.unwrap();
                let sc = self.p.layer_scale.max(0.01);
                // バイリニア補間（タイル状に繰り返す）。拡大しても角ばらない
                let u = (x - self.p.offset.0) / sc - 0.5 - l.origin.0 as f32;
                let v = (y - self.p.offset.1) / sc - 0.5 - l.origin.1 as f32;
                let (u0, v0) = (u.floor(), v.floor());
                let (fu, fv) = (u - u0, v - v0);
                let (lw, lh) = (l.width as i32, l.height as i32);
                let at = |xi: i32, yi: i32| -> f32 {
                    let xi = xi.rem_euclid(lw) as usize;
                    let yi = yi.rem_euclid(lh) as usize;
                    l.value[yi * l.width + xi]
                };
                let (xi, yi) = (u0 as i32, v0 as i32);
                let top = at(xi, yi) + (at(xi + 1, yi) - at(xi, yi)) * fu;
                let bottom = at(xi, yi + 1) + (at(xi + 1, yi + 1) - at(xi, yi + 1)) * fu;
                let v = top + (bottom - top) * fv;
                let (b, w) = (self.p.layer_black, self.p.layer_white);
                if (w - b).abs() < 1e-6 {
                    if v >= w {
                        1.0
                    } else {
                        0.0
                    }
                } else {
                    (v - b) / (w - b)
                }
            }
        };
        let t = if self.p.invert { 1.0 - t } else { t };
        t.clamp(0.0, 0.99999)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn coverage(p: TextureParams, v: f32) -> f32 {
        let tex = Texture::new(p, None);
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

    fn params(kind: TextureKind) -> TextureParams {
        TextureParams {
            kind,
            cell: 8.0,
            angle_deg: 15.0,
            seed: 3,
            ..Default::default()
        }
    }

    #[test]
    fn bayer_is_uniform() {
        for v in [0.1, 0.25, 0.5, 0.75, 0.9] {
            assert!((coverage(params(TextureKind::Bayer), v) - v).abs() < 0.02);
        }
    }

    /// LUT で均一化しているので、どの形でも面積率 ≒ 重み
    #[test]
    fn dot_shapes_are_uniform() {
        for shape in [
            DotShape::Round,
            DotShape::Circle,
            DotShape::Ellipse,
            DotShape::Square,
            DotShape::Diamond,
        ] {
            let p = TextureParams {
                dot_shape: shape,
                cell: 16.0,
                ..params(TextureKind::Dot)
            };
            for v in [0.1, 0.3, 0.5, 0.7, 0.9] {
                let c = coverage(p, v);
                assert!((c - v).abs() < 0.04, "{shape:?} v={v} c={c}");
            }
        }
        for v in [0.2, 0.5, 0.8] {
            let c = coverage(
                TextureParams {
                    cell: 16.0,
                    ..params(TextureKind::Line)
                },
                v,
            );
            assert!((c - v).abs() < 0.04, "line v={v} c={c}");
        }
    }

    #[test]
    fn noise_and_grain_are_monotonic() {
        for kind in [TextureKind::Noise, TextureKind::Grain] {
            let mut prev = -1.0;
            for i in 0..=10 {
                let c = coverage(params(kind), i as f32 / 10.0);
                assert!(c >= prev, "{kind:?}");
                prev = c;
            }
            assert!(coverage(params(kind), 0.0) == 0.0);
            assert!((coverage(params(kind), 0.5) - 0.5).abs() < 0.12);
        }
    }

    #[test]
    fn offset_moves_pattern() {
        let a = Texture::new(params(TextureKind::Dot), None);
        let b = Texture::new(
            TextureParams {
                offset: (3.0, -2.0),
                ..params(TextureKind::Dot)
            },
            None,
        );
        for (x, y) in [(10.5f32, 4.5f32), (33.5, 71.5)] {
            assert!((a.threshold(x, y) - b.threshold(x + 3.0, y - 2.0)).abs() < 1e-4);
        }
    }

    #[test]
    fn layer_scale_and_levels() {
        let data = [0.25f32, 0.75, 0.5, 1.0];
        let l = LayerTexture {
            width: 2,
            height: 2,
            value: &data,
            origin: (0, 0),
        };
        let p = TextureParams {
            kind: TextureKind::Layer,
            layer_scale: 2.0,
            layer_black: 0.25,
            layer_white: 0.75,
            ..Default::default()
        };
        let t = Texture::new(p, Some(l));
        // 2倍拡大なのでテクセルの中心は (1,1)=data[0]、(3,1)=data[1]、(1,3)=data[2]
        assert!((t.threshold(1.0, 1.0) - 0.0).abs() < 1e-6);
        assert!((t.threshold(3.0, 1.0) - 0.99999).abs() < 1e-4);
        assert!((t.threshold(1.0, 3.0) - 0.5).abs() < 1e-6);
        // 中心どうしの間はバイリニア補間（0.25 と 0.75 の中間 = 0.5 → レベル補正で 0.5）
        assert!((t.threshold(2.0, 1.0) - 0.5).abs() < 1e-6);
        // タイル状に繰り返す（幅2テクセル = 4px）
        assert!((t.threshold(5.0, 1.0) - t.threshold(1.0, 1.0)).abs() < 1e-6);
    }

    /// Bayer・Grain は Angle で回転しない（細かい粒を回すとモアレが出るため）
    #[test]
    fn bayer_and_grain_ignore_angle() {
        for kind in [TextureKind::Bayer, TextureKind::Grain] {
            let a = Texture::new(
                TextureParams {
                    kind,
                    angle_deg: 0.0,
                    ..Default::default()
                },
                None,
            );
            let b = Texture::new(
                TextureParams {
                    kind,
                    angle_deg: 45.0,
                    ..Default::default()
                },
                None,
            );
            for i in 0..200 {
                let (x, y) = ((i % 20) as f32 + 0.5, (i / 20) as f32 + 0.5);
                assert_eq!(a.threshold(x, y), b.threshold(x, y));
            }
        }
    }
}
