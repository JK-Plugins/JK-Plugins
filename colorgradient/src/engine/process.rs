//! 色グループ内の境界グラデーション本体。
//!
//! 1. ラベル付け：各ピクセルがグループ内の何番目の色か（グループ外は NONE）
//! 2. 色ごとの距離変換：その色の領域までの距離を、領域の周り（届く範囲）だけ求める
//! 3. 符号付き距離 s_k（領域の内側で負）から重み w_k = clamp(0.5 - s_k / W) を作り、正規化
//!    - 2色の境界では従来どおり直線的なグラデーション
//!    - 3色以上が接する点でも、どの重みも位置に対して連続なので継ぎ目が出ない
//! 4. しきい値マップ t に対して、番号の大きい色から重みを積み上げ、t が入った区間の色を選ぶ
//!    （2色なら「t < 上側の色の重み」で上側の色。3色以上では網点が入れ子になる）

use super::edt::nearest_seed;
use super::texture::{fbm, Texture, TextureKind};

/// 1ピクセルが考慮する、近くにある他の色の数の上限
const MAX_NEIGHBORS: usize = 4;

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
    /// （隣り合う番号の組で offset、番号が k 離れた組では k 倍）
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

#[derive(Clone, Copy)]
struct Near {
    n: u8,
    label: [u8; MAX_NEIGHBORS],
    dist: [f32; MAX_NEIGHBORS],
}

impl Near {
    const EMPTY: Near = Near {
        n: 0,
        label: [NONE; MAX_NEIGHBORS],
        dist: [f32::INFINITY; MAX_NEIGHBORS],
    };

    /// 近い順に MAX_NEIGHBORS 個まで保持
    fn insert(&mut self, label: u8, dist: f32) {
        let n = self.n as usize;
        if n < MAX_NEIGHBORS {
            self.label[n] = label;
            self.dist[n] = dist;
            self.n += 1;
            return;
        }
        let (mut worst, mut wd) = (0, self.dist[0]);
        for i in 1..MAX_NEIGHBORS {
            if self.dist[i] > wd {
                worst = i;
                wd = self.dist[i];
            }
        }
        if dist < wd {
            self.label[worst] = label;
            self.dist[worst] = dist;
        }
    }
}

#[inline]
fn smooth(w: f32) -> f32 {
    w * w * (3.0 - 2.0 * w)
}

pub fn process(w: usize, h: usize, src: &[[f32; 4]], s: &Settings, texture: &Texture) -> Output {
    assert_eq!(src.len(), w * h);
    let n = w * h;
    let mut out = Output {
        rgba: src.to_vec(),
        changed: vec![false; n],
    };
    let nc = s.colors.len();
    if nc < 2 || n == 0 {
        return out;
    }

    let labels = label_pixels(src, s);

    let half = (s.width * 0.5).max(0.0);
    // ラベル k のバイアス。番号の大きい色ほど符号付き距離を大きく（＝狭く）する
    let bias = |k: u8| 2.0 * s.offset * (k as f32 - (nc as f32 - 1.0) * 0.5);
    let max_bias = s.offset.abs() * (nc as f32 - 1.0);
    // これより遠い色は重みに効かない
    let reach = half + 0.5 + 2.0 * max_bias + 2.0 * s.roughness.abs() + 1.0;
    let reach_i = reach.ceil() as i64;
    let reach2 = reach * reach;

    // 色ごとの領域の外接矩形
    let mut bbox = vec![(usize::MAX, usize::MAX, 0usize, 0usize); nc];
    let mut present = vec![false; nc];
    for y in 0..h {
        for x in 0..w {
            let l = labels[y * w + x];
            if l != NONE {
                let b = &mut bbox[l as usize];
                b.0 = b.0.min(x);
                b.1 = b.1.min(y);
                b.2 = b.2.max(x);
                b.3 = b.3.max(y);
                present[l as usize] = true;
            }
        }
    }
    if present.iter().filter(|&&p| p).count() < 2 {
        return out;
    }

    // 色ごとに、領域の周り reach の範囲だけ距離変換し、近くの色として登録
    let mut near = vec![Near::EMPTY; n];
    let mut seed = Vec::new();
    for k in 0..nc {
        if !present[k] {
            continue;
        }
        let (x0, y0, x1, y1) = bbox[k];
        let x0 = (x0 as i64 - reach_i).max(0) as usize;
        let y0 = (y0 as i64 - reach_i).max(0) as usize;
        let x1 = ((x1 as i64 + reach_i) as usize).min(w - 1);
        let y1 = ((y1 as i64 + reach_i) as usize).min(h - 1);
        let (bw, bh) = (x1 - x0 + 1, y1 - y0 + 1);
        seed.clear();
        seed.resize(bw * bh, false);
        for y in 0..bh {
            for x in 0..bw {
                seed[y * bw + x] = labels[(y + y0) * w + x + x0] == k as u8;
            }
        }
        let (_, d2) = nearest_seed(bw, bh, &seed);
        for y in 0..bh {
            for x in 0..bw {
                let dd = d2[y * bw + x];
                if dd > reach2 || dd == 0.0 {
                    continue;
                }
                let i = (y + y0) * w + x + x0;
                if labels[i] != NONE {
                    near[i].insert(k as u8, dd.sqrt());
                }
            }
        }
    }

    let inv_w = if s.width > 1e-4 { 1.0 / s.width } else { 0.0 };
    let use_tex = texture.kind() != TextureKind::Smooth && s.amount > 0.0;
    let aa = s.antialias && texture.is_continuous();
    let (ox, oy) = (s.origin.0 as f32, s.origin.1 as f32);
    let rough_sc = 1.0 / s.rough_scale.max(0.5);

    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(h)
        .max(1);
    let rows_per = (h + threads - 1) / threads;
    let labels = &labels;
    let near = &near;
    let bias = &bias;

    std::thread::scope(|sc| {
        for (ci, (rgba_chunk, ch_chunk)) in out
            .rgba
            .chunks_mut(rows_per * w)
            .zip(out.changed.chunks_mut(rows_per * w))
            .enumerate()
        {
            sc.spawn(move || {
                // 候補：自分 + 近くの色（最大 MAX_NEIGHBORS）
                let mut cl = [NONE; MAX_NEIGHBORS + 1];
                let mut cw = [0f32; MAX_NEIGHBORS + 1];
                let rows = rgba_chunk.len() / w;
                for r in 0..rows {
                    let y = ci * rows_per + r;
                    for x in 0..w {
                        let i = y * w + x;
                        let a = labels[i];
                        let nb = &near[i];
                        if a == NONE || nb.n == 0 {
                            continue;
                        }
                        let gx = x as f32 + 0.5 + ox;
                        let gy = y as f32 + 0.5 + oy;
                        let rough = |k: u8| -> f32 {
                            if s.roughness == 0.0 {
                                0.0
                            } else {
                                let sd = s.rough_seed ^ (k as u32).wrapping_mul(0x9e37_79b9);
                                s.roughness * (2.0 * fbm(gx * rough_sc, gy * rough_sc, sd) - 1.0)
                            }
                        };
                        let weight = |sd: f32| -> f32 {
                            let v = if inv_w > 0.0 {
                                (0.5 - sd * inv_w).clamp(0.0, 1.0)
                            } else if sd < 0.0 {
                                1.0
                            } else if sd == 0.0 {
                                0.5
                            } else {
                                0.0
                            };
                            if s.curve == Curve::Smooth {
                                smooth(v)
                            } else {
                                v
                            }
                        };

                        // 自分の色：内側なので符号付き距離は負（いちばん近い他の色の境界まで）
                        let mut dmin = f32::INFINITY;
                        for j in 0..nb.n as usize {
                            dmin = dmin.min(nb.dist[j]);
                        }
                        let mut m = 0;
                        cl[m] = a;
                        cw[m] = weight(-(dmin - 0.5) + bias(a) + rough(a));
                        m += 1;
                        for j in 0..nb.n as usize {
                            let k = nb.label[j];
                            cl[m] = k;
                            cw[m] = weight(nb.dist[j] - 0.5 + bias(k) + rough(k));
                            m += 1;
                        }
                        let sum: f32 = cw[..m].iter().sum();
                        if sum <= 0.0 {
                            continue;
                        }
                        for v in &mut cw[..m] {
                            *v /= sum;
                        }

                        if s.output == OutputMode::Gradient {
                            // 番号の重み付き平均（0..1）
                            let mut v = 0.0;
                            for j in 0..m {
                                v += cw[j] * cl[j] as f32;
                            }
                            let v = v / (nc as f32 - 1.0);
                            rgba_chunk[r * w + x] = [v, v, v, 1.0];
                            ch_chunk[r * w + x] = true;
                            continue;
                        }

                        // 最終的な各色の割合
                        let mut frac = cw;
                        if use_tex {
                            // 番号の大きい色から積み上げる
                            let mut order = [0usize; MAX_NEIGHBORS + 1];
                            for j in 0..m {
                                order[j] = j;
                            }
                            order[..m].sort_unstable_by(|&p, &q| cl[q].cmp(&cl[p]));
                            let mut cnt = [0f32; MAX_NEIGHBORS + 1];
                            let mut pick = |t: f32, add: f32| {
                                let mut acc = 0.0;
                                for &j in &order[..m] {
                                    acc += cw[j];
                                    if t < acc {
                                        cnt[j] += add;
                                        return;
                                    }
                                }
                                cnt[order[m - 1]] += add;
                            };
                            if aa {
                                for sy in 0..4 {
                                    for sx in 0..4 {
                                        let px = gx + (sx as f32 + 0.5) * 0.25 - 0.5;
                                        let py = gy + (sy as f32 + 0.5) * 0.25 - 0.5;
                                        pick(texture.threshold(px, py), 1.0 / 16.0);
                                    }
                                }
                            } else {
                                pick(texture.threshold(gx, gy), 1.0);
                            }
                            for j in 0..m {
                                frac[j] = cw[j] + (cnt[j] - cw[j]) * s.amount;
                            }
                        }

                        // 自分の色のままなら入力をそのまま残す
                        if frac[0] >= 1.0 - 1e-6 {
                            continue;
                        }
                        let mut c = [0f32; 3];
                        for j in 0..m {
                            let col = s.colors[cl[j] as usize];
                            for ch in 0..3 {
                                c[ch] += col[ch] * frac[j];
                            }
                        }
                        rgba_chunk[r * w + x] = [c[0], c[1], c[2], src[i][3]];
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

    /// 3色が1点で接するところでも、隣り合うピクセルの色が急に変わらない
    #[test]
    fn triple_junction_is_continuous() {
        let (w, h) = (120usize, 120usize);
        let cols = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let src: Vec<[f32; 4]> = (0..w * h)
            .map(|i| {
                let x = (i % w) as f32 + 0.5 - 60.0;
                let y = (i / w) as f32 + 0.5 - 60.0;
                let a = y.atan2(x).to_degrees().rem_euclid(360.0);
                let c = cols[if a < 120.0 {
                    0
                } else if a < 240.0 {
                    1
                } else {
                    2
                }];
                [c[0], c[1], c[2], 1.0]
            })
            .collect();
        let s = Settings {
            colors: cols.to_vec(),
            width: 30.0,
            ..Default::default()
        };
        let tex = Texture::new(TextureParams::default(), None);
        let o = process(w, h, &src, &s, &tex);
        let mut worst = 0f32;
        for y in 0..h {
            for x in 0..w - 1 {
                for (p, q) in [(y * w + x, y * w + x + 1), (x * w + y, (x + 1) * w + y)] {
                    let d = (0..3)
                        .map(|c| (o.rgba[p][c] - o.rgba[q][c]).abs())
                        .fold(0.0, f32::max);
                    worst = worst.max(d);
                }
            }
        }
        // 幅30pxの直線グラデなら1pxあたり約0.033。継ぎ目があると0.3以上になる
        assert!(worst < 0.08, "max step {worst}");
    }
}
