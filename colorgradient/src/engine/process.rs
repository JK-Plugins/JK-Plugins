//! 色グループ内の境界グラデーション本体。
//!
//! 1. ラベル付け：各ピクセルがグループ内の何番目の色か（グループ外は NONE）
//! 2. 境界シード：4近傍に別ラベルを持つピクセル。相手のラベルを記録
//! 3. 距離変換1回：全ピクセルについて最近傍シードを求める（色数に依存しない）
//! 4. 境界をまたいで連続する値 v（下側の色=0 → 上側の色=1）を作る
//! 5. しきい値マップで v を2値化（＋任意でサブピクセルAA）し、2色を混ぜる

use super::edt::{nearest_seed, NO_SEED};
use super::texture::{fbm, Texture, TextureKind};

pub const NONE: u8 = u8::MAX;
pub const MAX_COLORS: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Curve {
    Linear,
    Smooth,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputMode {
    /// 色を置き換えた結果
    Result,
    /// 確認用：テクスチャ前のグラデーション値 v をグレーで表示
    Gradient,
}

pub struct Settings {
    /// グループの色（有効な色だけ、スロット順）。RGB 0..1
    pub colors: Vec<[f32; 3]>,
    /// 色の許容差（各チャンネルの最大差、0..1）
    pub tolerance: f32,
    /// グラデーション全体の幅（px）。境界の両側に半分ずつ
    pub width: f32,
    /// 境界の位置ずらし（px）。＋で番号の大きい色側へ境界が進む
    pub offset: f32,
    pub curve: Curve,
    /// テクスチャの効き（0 = なめらか、1 = 完全に2値化）
    pub amount: f32,
    /// 境界の荒らし（px）
    pub roughness: f32,
    /// 荒らしノイズの大きさ（px）
    pub rough_scale: f32,
    pub rough_seed: u32,
    /// サブピクセル 4×4 サンプルのアンチエイリアス
    pub antialias: bool,
    pub output: OutputMode,
    /// 描画座標の原点（テクスチャが領域の切り出しで動かないようにする）
    pub origin: (i32, i32),
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            colors: Vec::new(),
            tolerance: 0.0,
            width: 20.0,
            offset: 0.0,
            curve: Curve::Linear,
            amount: 1.0,
            roughness: 0.0,
            rough_scale: 32.0,
            rough_seed: 0,
            antialias: true,
            output: OutputMode::Result,
            origin: (0, 0),
        }
    }
}

/// 処理結果。`changed[i]` が false のピクセルは入力をそのまま使う。
pub struct Output {
    pub rgba: Vec<[f32; 4]>,
    pub changed: Vec<bool>,
}

fn label_pixels(src: &[[f32; 4]], s: &Settings) -> Vec<u8> {
    let tol = s.tolerance + 0.5 / 255.0;
    let mut labels = vec![NONE; src.len()];
    let mut last: Option<([f32; 4], u8)> = None;
    for (i, p) in src.iter().enumerate() {
        if p[3] < 0.999 {
            continue;
        }
        if let Some((lp, ll)) = last {
            if lp == *p {
                labels[i] = ll;
                continue;
            }
        }
        let mut best = NONE;
        let mut best_d = f32::INFINITY;
        for (k, c) in s.colors.iter().enumerate() {
            let d = (p[0] - c[0])
                .abs()
                .max((p[1] - c[1]).abs())
                .max((p[2] - c[2]).abs());
            if d <= tol && d < best_d {
                best_d = d;
                best = k as u8;
            }
        }
        labels[i] = best;
        last = Some((*p, best));
    }
    labels
}

pub fn process(w: usize, h: usize, src: &[[f32; 4]], s: &Settings, texture: &Texture) -> Output {
    assert_eq!(src.len(), w * h);
    let n = w * h;
    let mut out = Output {
        rgba: src.to_vec(),
        changed: vec![false; n],
    };
    if s.colors.len() < 2 || n == 0 {
        return out;
    }

    let labels = label_pixels(src, s);

    // 境界シード：4近傍に別のグループ色があるピクセル
    let mut seed = vec![false; n];
    let mut other = vec![NONE; n];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let a = labels[i];
            if a == NONE {
                continue;
            }
            let mut b = NONE;
            if x > 0 && labels[i - 1] != NONE && labels[i - 1] != a {
                b = labels[i - 1];
            } else if x + 1 < w && labels[i + 1] != NONE && labels[i + 1] != a {
                b = labels[i + 1];
            } else if y > 0 && labels[i - w] != NONE && labels[i - w] != a {
                b = labels[i - w];
            } else if y + 1 < h && labels[i + w] != NONE && labels[i + w] != a {
                b = labels[i + w];
            }
            if b != NONE {
                seed[i] = true;
                other[i] = b;
            }
        }
    }
    if !seed.iter().any(|&b| b) {
        return out;
    }

    let (near, d2) = nearest_seed(w, h, &seed);

    let half = (s.width * 0.5).max(0.0);
    let reach = half + s.offset.abs() + s.roughness.abs() + 1.0;
    let reach2 = reach * reach;
    let inv_w = if s.width > 1e-4 { 1.0 / s.width } else { 0.0 };
    let use_tex = texture.kind() != TextureKind::Smooth && s.amount > 0.0;
    let aa = s.antialias && texture.is_continuous();
    let (ox, oy) = (s.origin.0 as f32, s.origin.1 as f32);

    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(h)
        .max(1);
    let rows_per = (h + threads - 1) / threads;
    let labels = &labels;
    let other = &other;
    let near = &near;
    let d2 = &d2;

    std::thread::scope(|sc| {
        for (ci, (rgba_chunk, ch_chunk)) in out
            .rgba
            .chunks_mut(rows_per * w)
            .zip(out.changed.chunks_mut(rows_per * w))
            .enumerate()
        {
            sc.spawn(move || {
                let rows = rgba_chunk.len() / w;
                for r in 0..rows {
                    let y = ci * rows_per + r;
                    for x in 0..w {
                        let i = y * w + x;
                        let a = labels[i];
                        if a == NONE || near[i] == NO_SEED || d2[i] > reach2 {
                            continue;
                        }
                        let si = near[i] as usize;
                        let (sl, so) = (labels[si], other[si]);
                        let dist = d2[i].sqrt();
                        // 境界（シードとその隣のピクセルの中間）までの距離
                        let (b, d) = if a == sl {
                            (so, dist + 0.5)
                        } else if a == so {
                            (sl, (dist - 0.5).max(0.0))
                        } else {
                            continue; // この色の境界ではない（線などを挟んだ先）
                        };
                        let (lo, hi) = if a < b { (a, b) } else { (b, a) };
                        // 上側の色(hi)に向かって＋の符号付き距離
                        let mut xpos = if a == hi { d } else { -d };
                        let gx = x as f32 + 0.5 + ox;
                        let gy = y as f32 + 0.5 + oy;
                        if s.roughness != 0.0 {
                            let sc = 1.0 / s.rough_scale.max(0.5);
                            xpos += s.roughness * (2.0 * fbm(gx * sc, gy * sc, s.rough_seed) - 1.0);
                        }
                        xpos -= s.offset;
                        let mut v = if inv_w > 0.0 {
                            (0.5 + xpos * inv_w).clamp(0.0, 1.0)
                        } else if xpos >= 0.0 {
                            1.0
                        } else {
                            0.0
                        };
                        if s.curve == Curve::Smooth {
                            v = v * v * (3.0 - 2.0 * v);
                        }

                        if s.output == OutputMode::Gradient {
                            rgba_chunk[r * w + x] = [v, v, v, 1.0];
                            ch_chunk[r * w + x] = true;
                            continue;
                        }

                        let cov = if !use_tex {
                            v
                        } else {
                            let hard = if aa {
                                let mut on = 0u32;
                                for sy in 0..4 {
                                    for sx in 0..4 {
                                        let px = gx + (sx as f32 + 0.5) * 0.25 - 0.5;
                                        let py = gy + (sy as f32 + 0.5) * 0.25 - 0.5;
                                        if texture.threshold(px, py) < v {
                                            on += 1;
                                        }
                                    }
                                }
                                on as f32 / 16.0
                            } else if texture.threshold(gx, gy) < v {
                                1.0
                            } else {
                                0.0
                            };
                            v + (hard - v) * s.amount
                        };

                        // 自分の色のままなら入力をそのまま残す
                        let own = if a == hi { 1.0 } else { 0.0 };
                        if (cov - own).abs() < 1e-6 {
                            continue;
                        }
                        let c0 = s.colors[lo as usize];
                        let c1 = s.colors[hi as usize];
                        let alpha = src[i][3];
                        rgba_chunk[r * w + x] = [
                            c0[0] + (c1[0] - c0[0]) * cov,
                            c0[1] + (c1[1] - c0[1]) * cov,
                            c0[2] + (c1[2] - c0[2]) * cov,
                            alpha,
                        ];
                        ch_chunk[r * w + x] = true;
                    }
                }
            });
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::super::texture::TextureParams;
    use super::*;

    fn two_halves(w: usize, h: usize) -> Vec<[f32; 4]> {
        (0..w * h)
            .map(|i| {
                if i % w < w / 2 {
                    [1.0, 0.0, 0.0, 1.0]
                } else {
                    [0.0, 0.0, 1.0, 1.0]
                }
            })
            .collect()
    }

    #[test]
    fn smooth_gradient_is_symmetric_and_continuous() {
        let (w, h) = (40, 4);
        let src = two_halves(w, h);
        let s = Settings {
            colors: vec![[1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            width: 10.0,
            ..Default::default()
        };
        let tex = Texture::new(TextureParams::default(), None);
        let o = process(w, h, &src, &s, &tex);
        let row: Vec<f32> = (0..w).map(|x| o.rgba[x][2]).collect(); // 青の量 = v
                                                                    // 境界は x=20 の左端。ピクセル中心で v = 0.5 + (x+0.5-20)/10
        for x in 0..w {
            let expect = (0.5 + (x as f32 + 0.5 - 20.0) / 10.0).clamp(0.0, 1.0);
            assert!(
                (row[x] - expect).abs() < 1e-5,
                "x={x} {} {}",
                row[x],
                expect
            );
        }
        // 遠いピクセルは変更なし
        assert!(!o.changed[0] && !o.changed[w - 1]);
    }

    #[test]
    fn ignores_non_group_colors() {
        let (w, h) = (20, 3);
        let mut src = two_halves(w, h);
        src[0] = [0.0, 1.0, 0.0, 1.0];
        let s = Settings {
            colors: vec![[1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            width: 4.0,
            ..Default::default()
        };
        let tex = Texture::new(TextureParams::default(), None);
        let o = process(w, h, &src, &s, &tex);
        assert_eq!(o.rgba[0], [0.0, 1.0, 0.0, 1.0]);
        assert!(!o.changed[0]);
    }

    #[test]
    fn halftone_coverage_follows_gradient() {
        let (w, h) = (200, 200);
        let src = two_halves(w, h);
        let s = Settings {
            colors: vec![[1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            width: 160.0,
            antialias: false,
            ..Default::default()
        };
        let tex = Texture::new(
            TextureParams {
                kind: TextureKind::Bayer,
                cell: 8.0,
                ..Default::default()
            },
            None,
        );
        let o = process(w, h, &src, &s, &tex);
        // 境界(x=100)付近の16列は約50%
        let mut on = 0;
        let mut total = 0;
        for y in 0..h {
            for x in 92..108 {
                total += 1;
                if o.rgba[y * w + x][2] > 0.5 {
                    on += 1;
                }
            }
        }
        let r = on as f32 / total as f32;
        assert!((r - 0.5).abs() < 0.05, "{r}");
    }
}
