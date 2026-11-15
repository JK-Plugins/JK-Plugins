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

/// 1ピクセルが混ぜる「近くの他の色」の数の上限（Settings::max_neighbors で更に絞れる）
pub const MAX_NEIGHBORS: usize = 6;

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
    /// 確認用：テクスチャ前のグラデーション（色番号の重み付き平均）をグレーで表示
    Gradient,
}

/// グループの1色
#[derive(Clone, Copy, Debug)]
pub struct GroupColor {
    /// RGB 0..1
    pub rgb: [f32; 3],
    /// この色だけの許容差（全体の許容差に足す、0..1）
    pub tolerance: f32,
    /// にじむ幅の倍率。大きいほど隣の色の領域へ深く入り込む
    pub spread: f32,
    /// 領域を広げる量（px）。＋で隣の色との境界が相手側へ進む
    pub grow: f32,
}

impl GroupColor {
    pub fn new(rgb: [f32; 3]) -> Self {
        Self {
            rgb,
            tolerance: 0.0,
            spread: 1.0,
            grow: 0.0,
        }
    }
}

pub struct Settings {
    /// グループの色（有効な色だけ、スロット順）
    pub colors: Vec<GroupColor>,
    /// 色の許容差（各チャンネルの最大差、0..1）。各色の tolerance と足して使う
    pub tolerance: f32,
    /// これ以上のアルファのピクセルだけを対象にする（0..1）
    pub alpha_threshold: f32,
    /// グラデーション全体の幅（px）。境界の両側に半分ずつ
    pub width: f32,
    /// 境界の位置ずらし（px）。＋で番号の大きい色側へ境界が進む
    /// （隣り合う番号の組で offset、番号が k 離れた組では k 倍）
    pub offset: f32,
    /// 色の並び順を逆にする（網点の積み上げ順・Offset の向きが反転）
    pub reverse: bool,
    pub curve: Curve,
    /// カーブの強さ（重みのべき乗。1 = そのまま、>1 で境界付近が急に、<1 でなだらかに）
    pub curve_power: f32,
    /// 1ピクセルが混ぜる近くの色の数（1..MAX_NEIGHBORS）
    pub max_neighbors: usize,
    /// テクスチャの効き（0 = なめらか、1 = 完全に2値化）
    pub amount: f32,
    /// サブピクセルAAの分割数（1 = なし、2 = 2×2、4 = 4×4 …）
    pub aa_samples: u32,
    /// 境界の荒らし（px）
    pub roughness: f32,
    /// 荒らしノイズの大きさ（px）
    pub rough_scale: f32,
    pub rough_seed: u32,
    /// 色が混ざっているところに乗せる色と、その強さ（0..1）
    pub mid_color: [f32; 3],
    pub mid_amount: f32,
    pub output: OutputMode,
    /// 描画座標の原点（テクスチャが領域の切り出しで動かないようにする）
    pub origin: (i32, i32),
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            colors: Vec::new(),
            tolerance: 0.0,
            alpha_threshold: 1.0,
            width: 20.0,
            offset: 0.0,
            reverse: false,
            curve: Curve::Linear,
            curve_power: 1.0,
            max_neighbors: 4,
            amount: 1.0,
            aa_samples: 4,
            roughness: 0.0,
            rough_scale: 32.0,
            rough_seed: 0,
            mid_color: [1.0, 1.0, 1.0],
            mid_amount: 0.0,
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
    let alpha_min = s.alpha_threshold.clamp(0.0, 1.0) - 0.5 / 255.0;
    let mut labels = vec![NONE; src.len()];
    let mut last: Option<([f32; 4], u8)> = None;
    for (i, p) in src.iter().enumerate() {
        if p[3] < alpha_min || p[3] <= 0.0 {
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
            let c = c.rgb;
            let tol = s.tolerance + s.colors[k].tolerance + 0.5 / 255.0;
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
    fn insert(&mut self, label: u8, dist: f32, limit: usize) {
        let n = self.n as usize;
        if n < limit {
            self.label[n] = label;
            self.dist[n] = dist;
            self.n += 1;
            return;
        }
        let (mut worst, mut wd) = (0, self.dist[0]);
        for i in 1..limit {
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
    let limit = s.max_neighbors.clamp(1, MAX_NEIGHBORS);

    let labels = label_pixels(src, s);

    // 並び順（反転できる）。網点の積み上げ・Offset・確認表示に使う
    let rank: Vec<f32> = (0..nc)
        .map(|k| {
            if s.reverse {
                (nc - 1 - k) as f32
            } else {
                k as f32
            }
        })
        .collect();
    // 色ごとの幅
    let widths: Vec<f32> = s
        .colors
        .iter()
        .map(|c| (s.width * c.spread.max(0.0)).max(0.0))
        .collect();
    // 色ごとのバイアス（符号付き距離に足す。小さいほどその色が広がる）。
    // どのピクセルから見ても同じ値なので、重みは位置に対して連続になる。
    // 2色の境界は (bias_b - bias_a) / 2 だけ b 側へ進む → Grow は 2倍して引く
    let bias: Vec<f32> = (0..nc)
        .map(|k| 2.0 * s.offset * (rank[k] - (nc as f32 - 1.0) * 0.5) - 2.0 * s.colors[k].grow)
        .collect();
    let bmax = bias.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let bmin = bias.iter().cloned().fold(f32::INFINITY, f32::min);
    let wmax = widths.iter().cloned().fold(0.0f32, f32::max);
    // これより遠い色は重みに効かない
    let reach = wmax * 0.5 + 0.5 + (bmax - bmin) + 2.0 * s.roughness.abs() + 1.5;
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
                    near[i].insert(k as u8, dd.sqrt(), limit);
                }
            }
        }
    }

    let use_tex = texture.kind() != TextureKind::Smooth && s.amount > 0.0;
    let aa = s.aa_samples.clamp(1, 16);
    let (ox, oy) = (s.origin.0 as f32, s.origin.1 as f32);
    let rough_sc = 1.0 / s.rough_scale.max(0.5);
    let power = s.curve_power.clamp(0.05, 20.0);

    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(h)
        .max(1);
    let rows_per = (h + threads - 1) / threads;
    let labels = &labels;
    let near = &near;
    let (rank, widths, bias) = (&rank, &widths, &bias);

    std::thread::scope(|sc| {
        for (ci, (rgba_chunk, ch_chunk)) in out
            .rgba
            .chunks_mut(rows_per * w)
            .zip(out.changed.chunks_mut(rows_per * w))
            .enumerate()
        {
            sc.spawn(move || {
                // 候補：自分 + 近くの色
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
                        // 色 k のバイアス（荒らしノイズ込み）
                        let bias_k = |k: u8| -> f32 {
                            let mut v = bias[k as usize];
                            if s.roughness != 0.0 {
                                let sd = s.rough_seed ^ (k as u32).wrapping_mul(0x9e37_79b9);
                                v += s.roughness
                                    * (2.0 * fbm(gx * rough_sc, gy * rough_sc, sd) - 1.0);
                            }
                            v
                        };
                        // 符号付き距離 sd（内側で負）→ 重み
                        let weight = |sd: f32, k: u8| -> f32 {
                            let wk = widths[k as usize];
                            let sd = sd + bias_k(k);
                            let v = if wk > 1e-4 {
                                (0.5 - sd / wk).clamp(0.0, 1.0)
                            } else if sd < 0.0 {
                                1.0
                            } else if sd == 0.0 {
                                0.5
                            } else {
                                0.0
                            };
                            let v = if s.curve == Curve::Smooth {
                                smooth(v)
                            } else {
                                v
                            };
                            if power != 1.0 {
                                v.powf(power)
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
                        cw[m] = weight(-(dmin - 0.5), a);
                        m += 1;
                        for j in 0..nb.n as usize {
                            let k = nb.label[j];
                            cl[m] = k;
                            cw[m] = weight(nb.dist[j] - 0.5, k);
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
                            let mut v = 0.0;
                            for j in 0..m {
                                v += cw[j] * rank[cl[j] as usize];
                            }
                            let v = v / (nc as f32 - 1.0);
                            rgba_chunk[r * w + x] = [v, v, v, 1.0];
                            ch_chunk[r * w + x] = true;
                            continue;
                        }

                        // 最終的な各色の割合
                        let mut frac = cw;
                        if use_tex {
                            // 並び順の大きい色から積み上げる
                            let mut order = [0usize; MAX_NEIGHBORS + 1];
                            for j in 0..m {
                                order[j] = j;
                            }
                            order[..m].sort_unstable_by(|&p, &q| {
                                rank[cl[q] as usize]
                                    .partial_cmp(&rank[cl[p] as usize])
                                    .unwrap()
                            });
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
                            if aa > 1 {
                                let step = 1.0 / aa as f32;
                                let add = step * step;
                                for sy in 0..aa {
                                    for sx in 0..aa {
                                        let px = gx + (sx as f32 + 0.5) * step - 0.5;
                                        let py = gy + (sy as f32 + 0.5) * step - 0.5;
                                        pick(texture.threshold(px, py), add);
                                    }
                                }
                            } else {
                                pick(texture.threshold(gx, gy), 1.0);
                            }
                            for j in 0..m {
                                frac[j] = cw[j] + (cnt[j] - cw[j]) * s.amount;
                            }
                        }

                        // 中間色：なめらかな重みで混ざっている度合い（2色の境界で1）
                        let mut mid = 0.0;
                        if s.mid_amount > 0.0 {
                            let sq: f32 = cw[..m].iter().map(|v| v * v).sum();
                            mid = (2.0 * (1.0 - sq)).min(1.0) * s.mid_amount.min(1.0);
                        }

                        // 自分の色のままなら入力をそのまま残す
                        if frac[0] >= 1.0 - 1e-6 && mid < 1e-6 {
                            continue;
                        }
                        let mut c = [0f32; 3];
                        for j in 0..m {
                            let col = s.colors[cl[j] as usize].rgb;
                            for ch in 0..3 {
                                c[ch] += col[ch] * frac[j];
                            }
                        }
                        for ch in 0..3 {
                            c[ch] += (s.mid_color[ch] - c[ch]) * mid;
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
            colors: vec![
                GroupColor::new([1.0, 0.0, 0.0]),
                GroupColor::new([0.0, 0.0, 1.0]),
            ],
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
            colors: vec![
                GroupColor::new([1.0, 0.0, 0.0]),
                GroupColor::new([0.0, 0.0, 1.0]),
            ],
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
            colors: vec![
                GroupColor::new([1.0, 0.0, 0.0]),
                GroupColor::new([0.0, 0.0, 1.0]),
            ],
            width: 160.0,
            aa_samples: 1,
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

    /// Grow・Offset・Spread を入れても3色の接合部が連続
    #[test]
    fn triple_junction_with_grow_is_continuous() {
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
        let mut colors: Vec<GroupColor> = cols.iter().map(|&c| GroupColor::new(c)).collect();
        colors[1].grow = 10.0;
        colors[2].spread = 2.0;
        let s = Settings {
            colors,
            width: 30.0,
            offset: 3.0,
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
        assert!(worst < 0.1, "max step {worst}");
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
            colors: cols.iter().map(|&c| GroupColor::new(c)).collect(),
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

    fn blue_row(o: &Output, w: usize) -> Vec<f32> {
        (0..w).map(|x| o.rgba[x][2]).collect()
    }

    fn rb() -> Vec<GroupColor> {
        vec![
            GroupColor::new([1.0, 0.0, 0.0]),
            GroupColor::new([0.0, 0.0, 1.0]),
        ]
    }

    #[test]
    fn grow_moves_boundary() {
        let (w, h) = (60, 2);
        let src = two_halves(w, h);
        let mut colors = rb();
        colors[0].grow = 5.0; // 赤が5px広がる
        let s = Settings {
            colors,
            width: 10.0,
            ..Default::default()
        };
        let tex = Texture::new(TextureParams::default(), None);
        let row = blue_row(&process(w, h, &src, &s, &tex), w);
        // 境界(青50%)が x=30 から x=35 へ
        assert!(row[34] < 0.5 && row[35] > 0.5, "{:?}", &row[28..40]);
        assert!(row[33] < 0.5 && row[36] > 0.5);
    }

    #[test]
    fn spread_widens_one_side() {
        let (w, h) = (80, 2);
        let src = two_halves(w, h);
        let mut colors = rb();
        colors[1].spread = 3.0; // 青が赤側へ深く入り込む
        let s = Settings {
            colors,
            width: 10.0,
            ..Default::default()
        };
        let tex = Texture::new(TextureParams::default(), None);
        let row = blue_row(&process(w, h, &src, &s, &tex), w);
        assert!(row[40 - 12] > 0.0, "赤側の奥まで青が入る");
        assert!(row[40 + 6] >= 1.0 - 1e-6, "青側は通常の幅で終わる");
    }

    #[test]
    fn reverse_swaps_stacking() {
        let (w, h) = (100, 100);
        let src = two_halves(w, h);
        let tex = Texture::new(
            TextureParams {
                kind: TextureKind::Dot,
                cell: 10.0,
                ..Default::default()
            },
            None,
        );
        let s0 = Settings {
            colors: rb(),
            width: 40.0,
            aa_samples: 1,
            ..Default::default()
        };
        let s1 = Settings {
            reverse: true,
            ..Settings {
                colors: rb(),
                width: 40.0,
                aa_samples: 1,
                ..Default::default()
            }
        };
        let a = process(w, h, &src, &s0, &tex);
        let b = process(w, h, &src, &s1, &tex);
        assert!(a.rgba != b.rgba);
    }

    #[test]
    fn alpha_and_per_color_tolerance() {
        let (w, h) = (20, 1);
        let mut src = two_halves(w, h);
        src[0] = [0.98, 0.0, 0.0, 1.0]; // 少しずれた赤
        src[1] = [1.0, 0.0, 0.0, 0.5]; // 半透明
        let mut colors = rb();
        colors[0].tolerance = 0.05;
        let tex = Texture::new(TextureParams::default(), None);
        let s = Settings {
            colors: colors.clone(),
            width: 2.0,
            ..Default::default()
        };
        let l = label_pixels(&src, &s);
        assert_eq!(l[0], 0);
        assert_eq!(l[1], NONE);
        let s = Settings {
            colors,
            alpha_threshold: 0.4,
            width: 2.0,
            ..Default::default()
        };
        assert_eq!(label_pixels(&src, &s)[1], 0);
        let _ = process(w, h, &src, &s, &tex);
    }

    #[test]
    fn mid_color_peaks_at_boundary() {
        let (w, h) = (40, 2);
        let src = two_halves(w, h);
        let s = Settings {
            colors: rb(),
            width: 10.0,
            mid_color: [0.0, 1.0, 0.0],
            mid_amount: 1.0,
            ..Default::default()
        };
        let tex = Texture::new(TextureParams::default(), None);
        let o = process(w, h, &src, &s, &tex);
        let g: Vec<f32> = (0..w).map(|x| o.rgba[x][1]).collect();
        assert!(g[19] > 0.9 && g[20] > 0.9, "{:?}", &g[14..26]);
        assert_eq!(g[10], 0.0);
    }
}
