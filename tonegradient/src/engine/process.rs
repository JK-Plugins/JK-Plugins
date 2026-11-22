//! 色グループ内の境界グラデーション本体。
//!
//! 1. ラベル付け：各ピクセルがグループ内の何番目の色か（グループ外は NONE）
//! 2. 色ごとの距離変換：その色の領域までの距離を、領域の周り（届く範囲）だけ求める
//! 3. 符号付き距離 s_k（領域の内側で負）から重み w_k = clamp(0.5 - s_k / W) を作り、正規化
//!    - 2色の境界では従来どおり直線的なグラデーション
//!    - 3色以上が接する点でも、どの重みも位置に対して連続なので継ぎ目が出ない
//! 4. しきい値マップ t に対して、番号の大きい色から重みを積み上げ、t が入った区間の色を選ぶ
//!    （2色なら「t < 上側の色の重み」で上側の色。3色以上では網点が入れ子になる）

use super::curve::CurveLut;
use super::edt::distance_sq_by;
use super::texture::{fbm, Texture, TextureKind};

/// 1ピクセルが混ぜる「近くの他の色」の数の上限（Settings::max_neighbors で更に絞れる）
pub const MAX_NEIGHBORS: usize = 6;

pub const NONE: u8 = u8::MAX;
pub const MAX_COLORS: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Curve {
    Linear,
    Smooth,
    /// グラフで編集したカーブ（Settings::curve_points）
    Custom,
}

/// テクスチャの使い方
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextureMode {
    /// しきい値：テクスチャの値と色の割合を比べて色を選ぶ（網点のように2色に分かれる）
    Threshold,
    /// マップ：テクスチャの濃淡で混ざり具合を連続的にずらす（2値にならない）
    Map,
}

/// 中間色をテクスチャのどこに置くか
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MidPosition {
    /// いちばん上の色のすぐ外側（網点のふちに輪のように出る）
    Rim,
    /// いちばん内側（網点の中心に、中間色の網点として出る）
    Core,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputMode {
    /// 色を置き換えた結果
    Result,
    /// 確認用：テクスチャ前のグラデーション（色番号の重み付き平均）をグレーで表示
    Gradient,
}

/// 色の混ぜ方（グラデーションの補間）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interpolation {
    /// 受け取った RGB の値をそのまま混ぜる（従来どおり）
    Classic,
    /// sRGB をリニア（光の強さ）に直して混ぜる
    Linear,
    /// Oklab（人の見え方に近い色空間）で混ぜる
    Perceptual,
}

fn srgb_to_linear(v: f32) -> f32 {
    let a = v.abs();
    let l = if a <= 0.04045 {
        a / 12.92
    } else {
        ((a + 0.055) / 1.055).powf(2.4)
    };
    l.copysign(v)
}

fn linear_to_srgb(v: f32) -> f32 {
    let a = v.abs();
    let e = if a <= 0.0031308 {
        a * 12.92
    } else {
        1.055 * a.powf(1.0 / 2.4) - 0.055
    };
    e.copysign(v)
}

impl Interpolation {
    /// RGB（sRGB の値）を混ぜるための色空間に移す
    pub fn encode(self, rgb: [f32; 3]) -> [f32; 3] {
        match self {
            Interpolation::Classic => rgb,
            Interpolation::Linear => rgb.map(srgb_to_linear),
            Interpolation::Perceptual => {
                let [r, g, b] = rgb.map(srgb_to_linear);
                let l = 0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b;
                let m = 0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b;
                let s = 0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b;
                let (l, m, s) = (l.cbrt(), m.cbrt(), s.cbrt());
                [
                    0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
                    1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
                    0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s,
                ]
            }
        }
    }

    /// `encode` の逆（混ぜた結果を RGB に戻す）
    pub fn decode(self, v: [f32; 3]) -> [f32; 3] {
        match self {
            Interpolation::Classic => v,
            Interpolation::Linear => v.map(linear_to_srgb),
            Interpolation::Perceptual => {
                let [ll, a, b] = v;
                let l = ll + 0.3963377774 * a + 0.2158037573 * b;
                let m = ll - 0.1055613458 * a - 0.0638541728 * b;
                let s = ll - 0.0894841775 * a - 1.2914855480 * b;
                let (l, m, s) = (l * l * l, m * m * m, s * s * s);
                [
                    4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
                    -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
                    -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s,
                ]
                .map(linear_to_srgb)
            }
        }
    }
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

#[derive(Clone)]
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
    /// カーブの強さ（重みのべき乗。1 = そのまま、>1 で境界付近が急に、<1 でなだらかに）。
    /// Custom のときは使わない
    pub curve_power: f32,
    /// Curve = Custom のときの点（x, y は 0..1。両端 (0,0)・(1,1) は含めない）
    pub curve_points: Vec<[f32; 2]>,
    /// 1ピクセルが混ぜる近くの色の数（1..MAX_NEIGHBORS）
    pub max_neighbors: usize,
    /// テクスチャの効き（0 = なめらか、1 = 完全に2値化）
    pub amount: f32,
    pub texture_mode: TextureMode,
    /// 網点や粒のふちをぼかす幅（px）。0 でくっきり。Threshold のときだけ使う。
    /// どの値でも色の面積率は重みと一致し、重み 0・1 のところは変わらない
    pub softness: f32,
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
    /// 中間色の帯の太さ（0..1）。1 で混ざっているところ全体、小さいほど境界の真ん中に寄る
    pub mid_width: f32,
    pub mid_position: MidPosition,
    /// 色の混ぜ方
    pub interpolation: Interpolation,
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
            curve_points: Vec::new(),
            max_neighbors: 4,
            amount: 1.0,
            texture_mode: TextureMode::Threshold,
            softness: 0.0,
            aa_samples: 1,
            roughness: 0.0,
            rough_scale: 32.0,
            rough_seed: 0,
            mid_color: [1.0, 1.0, 1.0],
            mid_amount: 0.0,
            mid_width: 0.5,
            mid_position: MidPosition::Core,
            interpolation: Interpolation::Classic,
            output: OutputMode::Result,
            origin: (0, 0),
        }
    }
}

/// 処理結果。入力から変わったピクセルだけを、インデックス（y*w+x）の昇順で持つ。
/// 変わらないピクセルは入力をそのまま使う（全画面ぶんのコピーを持たない）。
pub struct Output {
    pub changed: Vec<(u32, [f32; 4])>,
}

/// 確認・テスト用に全ピクセルへ展開したもの
pub struct Dense {
    pub rgba: Vec<[f32; 4]>,
    pub changed: Vec<bool>,
}

impl Output {
    pub fn to_dense(&self, src: &[[f32; 4]]) -> Dense {
        let mut rgba = src.to_vec();
        let mut changed = vec![false; src.len()];
        for &(i, p) in &self.changed {
            rgba[i as usize] = p;
            changed[i as usize] = true;
        }
        Dense { rgba, changed }
    }
}

pub(crate) fn label_pixels(src: &[[f32; 4]], s: &Settings) -> Vec<u8> {
    let alpha_min = s.alpha_threshold.clamp(0.0, 1.0) - 0.5 / 255.0;
    let mut labels = vec![NONE; src.len()];
    // 画素ごとに独立なので、まとまった範囲ごとにスレッドで分ける
    let threads = worker_threads(src.len() / 4096);
    let per = src.len().div_ceil(threads).max(1);
    super::par::scope(|sc| {
        for (lchunk, schunk) in labels.chunks_mut(per).zip(src.chunks(per)) {
            sc.spawn(move || {
                let mut last: Option<([f32; 4], u8)> = None;
                for (l, p) in lchunk.iter_mut().zip(schunk) {
                    if p[3] < alpha_min || p[3] <= 0.0 {
                        continue;
                    }
                    if let Some((lp, ll)) = last {
                        if lp == *p {
                            *l = ll;
                            continue;
                        }
                    }
                    let mut best = NONE;
                    let mut best_d = f32::INFINITY;
                    for (k, c) in s.colors.iter().enumerate() {
                        let rgb = c.rgb;
                        let tol = s.tolerance + c.tolerance + 0.5 / 255.0;
                        let d = (p[0] - rgb[0])
                            .abs()
                            .max((p[1] - rgb[1]).abs())
                            .max((p[2] - rgb[2]).abs());
                        if d <= tol && d < best_d {
                            best_d = d;
                            best = k as u8;
                        }
                    }
                    *l = best;
                    last = Some((*p, best));
                }
            });
        }
    });
    labels
}

/// 使うスレッド数（仕事の量 `work` を超えない）
fn worker_threads(work: usize) -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(work.max(1))
        .max(1)
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

/// 「しきい値 t < 累積の重み a」の境目を幅 e（しきい値の単位）でぼかしたもの（0..1）。
/// e は「ぼかしたい px × その場所でのテクスチャの変化の速さ」なので、網点や粒のふちが画面上で
/// e に応じた px だけなめらかになる（上から重ねるのではなく、境目そのものがぼける）。
/// - 幅は 2·min(a, 1−a) までに抑える：ぼかしの窓が [0, 1] に収まるので、t が一様なら平均が a
///   （色の割合が重みと一致する）。a = 0 / 1 では幅 0 なのでグラデーションの端も変わらない
/// - a について単調増加（隣り合う差が負にならない）
#[inline]
fn soft_step(a: f32, t: f32, e: f32) -> f32 {
    let e = e.min(2.0 * a.min(1.0 - a));
    if e <= 1e-6 {
        return if t < a { 1.0 } else { 0.0 };
    }
    ((a - t) / e + 0.5).clamp(0.0, 1.0)
}

/// Map モードで、累積の重み a をテクスチャの値 t でずらしたもの（0..1）。
/// a + k·(0.5 − t)·2a(1−a)：t が小さい（暗い）ほど上の色、大きい（明るい）ほど下の色に寄る。
/// - a = 0 / 1 では動かない（グラデーションの端が変わらない）
/// - t が一様なら平均は a（色の割合が重みと一致する）
/// - k ≤ 1 なら [0, 1] からはみ出さない
#[inline]
fn map_step(a: f32, t: f32, k: f32) -> f32 {
    (a + k * (0.5 - t) * 2.0 * a * (1.0 - a)).clamp(0.0, 1.0)
}

/// 並び順（反転できる）。網点の積み上げ・Offset・確認表示に使う
pub(crate) fn ranks(s: &Settings) -> Vec<f32> {
    let nc = s.colors.len();
    (0..nc)
        .map(|k| {
            if s.reverse {
                (nc - 1 - k) as f32
            } else {
                k as f32
            }
        })
        .collect()
}

/// 色ごとの幅（Width × Spread）
pub(crate) fn widths(s: &Settings) -> Vec<f32> {
    s.colors
        .iter()
        .map(|c| (s.width * c.spread.max(0.0)).max(0.0))
        .collect()
}

/// 色ごとのバイアス（符号付き距離に足す。小さいほどその色が広がる）。
/// どのピクセルから見ても同じ値なので、重みは位置に対して連続になる。
/// 2色の境界は (bias_b - bias_a) / 2 だけ b 側へ進む → Grow は 2倍して引く
pub(crate) fn biases(s: &Settings, rank: &[f32]) -> Vec<f32> {
    let nc = s.colors.len();
    (0..nc)
        .map(|k| 2.0 * s.offset * (rank[k] - (nc as f32 - 1.0) * 0.5) - 2.0 * s.colors[k].grow)
        .collect()
}

/// グラデーションが届く距離（px）。これより遠い色は重みに効かない。
/// 部分的に描画するときに入力を広げる量（SmartPreRender）にもこの値を使う。
///
/// 色 k の重みが正になるのは「k までの距離 − 0.5 + bias_k ± 荒らし < W_k / 2」のときなので、
/// 届く距離は max_k (W_k / 2 − bias_k) + 0.5 + 荒らし。
/// （全部の色を同じだけ Grow すると bias はみな負になり、そのぶん遠くまで届く）
pub fn reach(s: &Settings) -> f32 {
    if s.colors.is_empty() {
        return 0.0;
    }
    let bias = biases(s, &ranks(s));
    let far = widths(s)
        .iter()
        .zip(&bias)
        .map(|(w, b)| w * 0.5 - b)
        .fold(0.0f32, f32::max);
    far + 0.5 + 2.0 * s.roughness.abs() + 1.5
}

/// 近くの色が無いピクセルの `near_idx`
const NO_NEAR: u32 = u32::MAX;

/// 距離変換まで済ませた途中結果。テクスチャ・Amount・Curve・Mid Color などを変えても
/// 使い回せる（`prepare_key` が同じなら中身も同じ）。
pub struct Prepared {
    w: usize,
    h: usize,
    labels: Vec<u8>,
    /// ピクセルごとの `near` の番号（近くに他の色が無ければ NO_NEAR）。
    /// 境界の近くのピクセルだけ `near` を持つので、領域の内側はメモリを使わない。
    /// 番号は、そのピクセルの行が属するまとまり（`chunk_pixels` 画素ずつ）の中での番号
    near_idx: Vec<u32>,
    /// 行のまとまりごとの near（作るときにスレッドごとに持ったものをそのまま使う）
    near: Vec<Vec<Near>>,
    chunk_pixels: usize,
}

impl Prepared {
    /// おおよそのメモリ使用量（キャッシュの上限判定用）
    pub fn memory_bytes(&self) -> usize {
        self.labels.len()
            + self.near_idx.len() * 4
            + self.near.iter().map(|c| c.len()).sum::<usize>() * std::mem::size_of::<Near>()
    }

    #[inline]
    fn near_at(&self, i: usize) -> Option<&Near> {
        if self.near_idx.is_empty() {
            return None;
        }
        match self.near_idx[i] {
            NO_NEAR => None,
            j => Some(&self.near[i / self.chunk_pixels][j as usize]),
        }
    }
}

/// 速いハッシュ（キャッシュのキー用。暗号的な強さは要らない）
struct KeyHasher(u64);

impl KeyHasher {
    #[inline]
    fn add(&mut self, v: u64) {
        self.0 = (self.0.rotate_left(5) ^ v).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
    fn add_f32(&mut self, v: f32) {
        self.add(v.to_bits() as u64);
    }
}

/// `prepare` の結果を決める値だけから作るキー。
/// 入力の画素・色の判定に関わる設定・届く距離・混ぜる色の数が同じなら同じになる。
pub fn prepare_key(w: usize, h: usize, src: &[[f32; 4]], s: &Settings) -> u64 {
    // 画素はまとまりごとにスレッドでハッシュする
    let threads = worker_threads(src.len() / 65536);
    let per = src.len().div_ceil(threads).max(1);
    let parts: Vec<u64> = super::par::scope(|sc| {
        let hs: Vec<_> = src
            .chunks(per)
            .map(|chunk| {
                sc.spawn(move || {
                    let mut h = KeyHasher(0x1319_8a2e_0370_7344);
                    for p in chunk {
                        h.add((p[0].to_bits() as u64) << 32 | p[1].to_bits() as u64);
                        h.add((p[2].to_bits() as u64) << 32 | p[3].to_bits() as u64);
                    }
                    h.0
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let mut k = KeyHasher(0x0a2e_0370);
    for v in parts {
        k.add(v);
    }
    prepare_key_with(w, h, s, k.0)
}

/// 画素のバイト列（行ごと）のハッシュ。`prepare_key_with` に渡すと、画素を float にしなくても
/// キーが作れる（同じ画像は同じキーになる。`prepare_key` とは値が違う）
pub fn hash_rows(rows: &[&[u8]]) -> u64 {
    let threads = worker_threads(rows.iter().map(|r| r.len()).sum::<usize>() / (1 << 20));
    let per = rows.len().div_ceil(threads).max(1);
    let parts: Vec<u64> = super::par::scope(|sc| {
        let hs: Vec<_> = rows
            .chunks(per)
            .map(|chunk| {
                sc.spawn(move || {
                    let mut h = KeyHasher(0x2452_821e_638d_0137);
                    for row in chunk {
                        let mut it = row.chunks_exact(8);
                        for c in &mut it {
                            h.add(u64::from_le_bytes(c.try_into().unwrap()));
                        }
                        let mut last = [0u8; 8];
                        let rem = it.remainder();
                        last[..rem.len()].copy_from_slice(rem);
                        h.add(u64::from_le_bytes(last) ^ (rem.len() as u64) << 59);
                    }
                    h.0
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let mut k = KeyHasher(0xbe54_66cf_34e9_0c6c);
    k.add(rows.len() as u64);
    for v in parts {
        k.add(v);
    }
    k.0
}

/// 画素のハッシュ `pixels` と設定からキーを作る
pub fn prepare_key_with(w: usize, h: usize, s: &Settings, pixels: u64) -> u64 {
    let mut k = KeyHasher(0x243f_6a88_85a3_08d3);
    k.add(w as u64);
    k.add(h as u64);
    k.add(s.colors.len() as u64);
    for c in &s.colors {
        for v in c.rgb {
            k.add_f32(v);
        }
        k.add_f32(c.tolerance);
    }
    k.add_f32(s.tolerance);
    k.add_f32(s.alpha_threshold);
    k.add_f32(reach(s));
    k.add(s.max_neighbors.clamp(1, MAX_NEIGHBORS) as u64);
    k.add(pixels);
    k.0
}

/// ラベル付けと色ごとの距離変換（重い部分）
pub fn prepare(w: usize, h: usize, src: &[[f32; 4]], s: &Settings) -> Prepared {
    assert_eq!(src.len(), w * h);
    let n = w * h;
    let nc = s.colors.len();
    let mut p = Prepared {
        w,
        h,
        labels: Vec::new(),
        near_idx: Vec::new(),
        near: Vec::new(),
        chunk_pixels: 1,
    };
    if nc < 2 || n == 0 {
        return p;
    }
    let limit = s.max_neighbors.clamp(1, MAX_NEIGHBORS);
    let labels = label_pixels(src, s);

    let reach = reach(s);
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
        return p;
    }

    // 色ごとに、領域の周り reach の範囲だけ距離変換し、近くの色として登録。
    // 登録は行ごとに独立なので、行をスレッドに固定で割り当てる（スレッドごとに near を持ち、最後につなぐ）
    let threads = worker_threads(h);
    let rows_per = h.div_ceil(threads);
    let mut near_idx = vec![NO_NEAR; n];
    let mut chunk_near: Vec<Vec<Near>> = vec![Vec::new(); h.div_ceil(rows_per)];
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
        // シード（この色の画素）はラベルを直接読む（別の配列を作らない）
        let d2 = {
            let labels = &labels;
            let kk = k as u8;
            distance_sq_by(bw, bh, move |x, y| labels[(y + y0) * w + x + x0] == kk)
        };
        let (d2, labels) = (&d2, &labels);
        super::par::scope(|sc| {
            for (ci, (idx_chunk, near)) in near_idx
                .chunks_mut(rows_per * w)
                .zip(chunk_near.iter_mut())
                .enumerate()
            {
                let r0 = ci * rows_per;
                let r1 = (r0 + rows_per).min(h);
                // この色の範囲と重なる行だけ
                let ya = r0.max(y0);
                let yb = r1.min(y1 + 1);
                if ya >= yb {
                    continue;
                }
                sc.spawn(move || {
                    for y in ya..yb {
                        for x in x0..=x1 {
                            let dd = d2[(y - y0) * bw + (x - x0)];
                            if dd > reach2 || dd == 0.0 {
                                continue;
                            }
                            let i = y * w + x;
                            if labels[i] == NONE {
                                continue;
                            }
                            let li = i - r0 * w;
                            let j = match idx_chunk[li] {
                                NO_NEAR => {
                                    idx_chunk[li] = near.len() as u32;
                                    near.push(Near::EMPTY);
                                    near.len() - 1
                                }
                                j => j as usize,
                            };
                            near[j].insert(k as u8, dd.sqrt(), limit);
                        }
                    }
                });
            }
        });
    }
    p.labels = labels;
    p.near_idx = near_idx;
    p.near = chunk_near;
    p.chunk_pixels = rows_per * w;
    p
}

/// 重み・テクスチャ・色の合成（軽い部分）。`p` は同じ入力と設定で作った `prepare` の結果。
pub fn shade(p: &Prepared, src: &[[f32; 4]], s: &Settings, texture: &Texture) -> Output {
    let (w, h) = (p.w, p.h);
    let nc = s.colors.len();
    if p.labels.is_empty() || nc < 2 {
        return Output {
            changed: Vec::new(),
        };
    }
    assert_eq!(src.len(), w * h);

    let rank = ranks(s);
    let widths = widths(s);
    let bias = biases(s, &rank);

    let use_tex = texture.kind() != TextureKind::Smooth && s.amount > 0.0;
    // Softness：網点や粒のふちをぼかす幅（px）。0 でくっきり
    let soft = s.softness.max(0.0);
    // Mid Width 1 → 指数1（混ざっているところ全体）、0 → 指数4（真ん中だけ）
    let mid_exp = 1.0 + 3.0 * (1.0 - s.mid_width.clamp(0.0, 1.0));
    let aa = s.aa_samples.clamp(1, 16);
    let (ox, oy) = (s.origin.0 as f32, s.origin.1 as f32);
    let rough_sc = 1.0 / s.rough_scale.max(0.5);
    let power = if s.curve == Curve::Custom {
        1.0
    } else {
        s.curve_power.clamp(0.05, 20.0)
    };
    let custom = (s.curve == Curve::Custom).then(|| CurveLut::new(&s.curve_points));
    let custom = &custom;
    // 混ぜる色は補間の色空間に移しておき、混ぜた結果だけを戻す
    let interp = s.interpolation;
    let mixed: Vec<[f32; 3]> = s.colors.iter().map(|c| interp.encode(c.rgb)).collect();
    let mixed = &mixed;
    let mid_mixed = interp.encode(s.mid_color);

    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(h)
        .max(1);
    let rows_per = h.div_ceil(threads);
    let (rank, widths, bias) = (&rank, &widths, &bias);

    let parts: Vec<Vec<(u32, [f32; 4])>> = super::par::scope(|sc| {
        let handles: Vec<_> = (0..threads)
            .map(|ti| {
                sc.spawn(move || {
                    let mut changed = Vec::new();
                    // 候補：自分 + 近くの色
                    let mut cl = [NONE; MAX_NEIGHBORS + 1];
                    let mut cw = [0f32; MAX_NEIGHBORS + 1];
                    let y_end = ((ti + 1) * rows_per).min(h);
                    for y in ti * rows_per..y_end {
                        for x in 0..w {
                            let i = y * w + x;
                            let a = p.labels[i];
                            if a == NONE {
                                continue;
                            }
                            let gx = x as f32 + 0.5 + ox;
                            let gy = y as f32 + 0.5 + oy;
                            let Some(nb) = p.near_at(i) else {
                                continue;
                            };
                            if nb.n == 0 {
                                continue;
                            }
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
                                let v = match (s.curve, custom) {
                                    (Curve::Smooth, _) => smooth(v),
                                    (Curve::Custom, Some(lut)) => lut.eval(v),
                                    _ => v,
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
                                changed.push((i as u32, [v, v, v, 1.0]));
                                continue;
                            }

                            // 中間色：混ざっている度合い（2色の境界の真ん中で1）に Mid Amount を掛けた割合を、
                            // 3つ目の色としてグラデーションに入れる（上から重ねるのではなく、
                            // テクスチャ・Softness も同じようにかかる）。他の色はそのぶん減らす
                            let mut mid = 0.0;
                            if s.mid_amount > 0.0 {
                                let sq: f32 = cw[..m].iter().map(|v| v * v).sum();
                                let mix = (2.0 * (1.0 - sq)).clamp(0.0, 1.0);
                                // Mid Width が小さいほど真ん中に寄せる（端に向かってなだらかに減る）
                                mid = mix.powf(mid_exp) * s.mid_amount.min(1.0);
                            }
                            // 候補の末尾 m 番に中間色を置く（m+1 個）
                            let mut ew = [0f32; MAX_NEIGHBORS + 2];
                            for j in 0..m {
                                ew[j] = cw[j] * (1.0 - mid);
                            }
                            ew[m] = mid;
                            let me = if mid > 0.0 { m + 1 } else { m };

                            // 最終的な各色の割合
                            let mut frac = ew;
                            // Layer テクスチャは画像の範囲の中だけ（外側はなめらかなグラデーションのまま）
                            if use_tex && texture.covers(gx, gy) {
                                // 並び順の大きい色から積み上げる。中間色は Rim ならいちばん上の色のすぐ後
                                // （網点のふちに出る）、Core なら先頭（網点の中心に出る）
                                let mut order = [0usize; MAX_NEIGHBORS + 2];
                                for j in 0..m {
                                    order[j] = j;
                                }
                                order[..m].sort_unstable_by(|&p, &q| {
                                    rank[cl[q] as usize]
                                        .partial_cmp(&rank[cl[p] as usize])
                                        .unwrap()
                                });
                                if me > m {
                                    // Rim：いちばん上の色の直後／Core：先頭（しきい値の小さい側＝網点の中心）
                                    let at = if s.mid_position == MidPosition::Core {
                                        0
                                    } else {
                                        1
                                    };
                                    for k in (at..m).rev() {
                                        order[k + 1] = order[k];
                                    }
                                    order[at] = m;
                                }
                                let mut cnt = [0f32; MAX_NEIGHBORS + 2];
                                // ぼかす幅をしきい値の単位にする：その場所でのテクスチャの変化の速さ
                                // （1px あたり）× Softness（px）
                                let edge = if soft > 0.0 && s.texture_mode == TextureMode::Threshold
                                {
                                    let t0 = texture.threshold(gx, gy);
                                    let tx = texture.threshold(gx + 1.0, gy);
                                    let ty = texture.threshold(gx, gy + 1.0);
                                    ((tx - t0).abs() + (ty - t0).abs()) * soft
                                } else {
                                    0.0
                                };
                                let map = s.texture_mode == TextureMode::Map;
                                let mut pick = |t: f32, add: f32| {
                                    if map {
                                        // テクスチャをそのままマップとして使う：境目を連続的にずらす
                                        let mut acc = 0.0;
                                        let mut prev = 0.0;
                                        for (n, &j) in order[..me].iter().enumerate() {
                                            acc += ew[j];
                                            let cur = if n + 1 == me {
                                                1.0
                                            } else {
                                                map_step(acc, t, 1.0)
                                            };
                                            cnt[j] += (cur - prev).max(0.0) * add;
                                            prev = cur;
                                        }
                                    } else if edge <= 0.0 {
                                        let mut acc = 0.0;
                                        for &j in &order[..me] {
                                            acc += ew[j];
                                            if t < acc {
                                                cnt[j] += add;
                                                return;
                                            }
                                        }
                                        cnt[order[me - 1]] += add;
                                    } else {
                                        // 区間の境目をぼかす。累積 a ごとに soft_step(a, t) を求め、
                                        // 隣り合う差をその色の割合にする（合計は1）
                                        let mut acc = 0.0;
                                        let mut prev = 0.0;
                                        for (n, &j) in order[..me].iter().enumerate() {
                                            acc += ew[j];
                                            let cur = if n + 1 == me {
                                                1.0
                                            } else {
                                                soft_step(acc, t, edge)
                                            };
                                            cnt[j] += (cur - prev).max(0.0) * add;
                                            prev = cur;
                                        }
                                    }
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
                                for j in 0..me {
                                    frac[j] = ew[j] + (cnt[j] - ew[j]) * s.amount;
                                }
                            }

                            // 自分の色のままなら入力をそのまま残す
                            if frac[0] >= 1.0 - 1e-6 {
                                continue;
                            }
                            let mut c = [0f32; 3];
                            for j in 0..me {
                                let col = if j == m {
                                    mid_mixed
                                } else {
                                    mixed[cl[j] as usize]
                                };
                                for ch in 0..3 {
                                    c[ch] += col[ch] * frac[j];
                                }
                            }
                            let c = interp.decode(c);
                            changed.push((i as u32, [c[0], c[1], c[2], src[i][3]]));
                        }
                    }
                    changed
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    // スレッドは行の順に並んでいるので、つなげればインデックスの昇順になる
    Output {
        changed: parts.concat(),
    }
}

pub fn process(w: usize, h: usize, src: &[[f32; 4]], s: &Settings, texture: &Texture) -> Output {
    let p = prepare(w, h, src, s);
    shade(&p, src, s, texture)
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
        let o = process(w, h, &src, &s, &tex).to_dense(&src);
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
        let o = process(w, h, &src, &s, &tex).to_dense(&src);
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
        let o = process(w, h, &src, &s, &tex).to_dense(&src);
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
        let o = process(w, h, &src, &s, &tex).to_dense(&src);
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
        let o = process(w, h, &src, &s, &tex).to_dense(&src);
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

    fn blue_row(o: &Dense, w: usize) -> Vec<f32> {
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
        let row = blue_row(&process(w, h, &src, &s, &tex).to_dense(&src), w);
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
        let row = blue_row(&process(w, h, &src, &s, &tex).to_dense(&src), w);
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
        let a = process(w, h, &src, &s0, &tex).to_dense(&src);
        let b = process(w, h, &src, &s1, &tex).to_dense(&src);
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
        let _ = process(w, h, &src, &s, &tex).to_dense(&src);
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
        let o = process(w, h, &src, &s, &tex).to_dense(&src);
        let g: Vec<f32> = (0..w).map(|x| o.rgba[x][1]).collect();
        assert!(g[19] > 0.9 && g[20] > 0.9, "{:?}", &g[14..26]);
        assert_eq!(g[10], 0.0);
    }

    /// prepare を使い回して shade だけやり直しても、毎回 process した結果と同じ
    #[test]
    fn interpolation_round_trips() {
        for interp in [
            Interpolation::Classic,
            Interpolation::Linear,
            Interpolation::Perceptual,
        ] {
            for rgb in [[0.0, 0.0, 0.0], [1.0, 1.0, 1.0], [1.0, 0.0, 0.0], [0.2, 0.6, 0.9], [1.5, 0.3, 0.0]] {
                let back = interp.decode(interp.encode(rgb));
                for c in 0..3 {
                    assert!((back[c] - rgb[c]).abs() < 1e-4, "{interp:?} {rgb:?} -> {back:?}");
                }
            }
        }
    }

    #[test]
    fn interpolation_changes_the_middle() {
        // 赤と緑の真ん中：Classic はそのまま平均、Linear は明るく、Perceptual は Classic より明るい
        let (w, h) = (64, 4);
        let src: Vec<[f32; 4]> = (0..w * h)
            .map(|i| if i % w < w / 2 { [1.0, 0.0, 0.0, 1.0] } else { [0.0, 1.0, 0.0, 1.0] })
            .collect();
        let mid = |interp| {
            let s = Settings {
                colors: vec![GroupColor::new([1.0, 0.0, 0.0]), GroupColor::new([0.0, 1.0, 0.0])],
                width: 40.0,
                interpolation: interp,
                ..Default::default()
            };
            let tex = Texture::new(TextureParams::default(), None);
            let d = process(w, h, &src, &s, &tex).to_dense(&src);
            d.rgba[w + w / 2]
        };
        let c = mid(Interpolation::Classic);
        let l = mid(Interpolation::Linear);
        let p = mid(Interpolation::Perceptual);
        assert!(l[0] > c[0] + 0.1 && l[1] > c[1] + 0.1, "classic {c:?} linear {l:?}");
        assert!(p[0] + p[1] > c[0] + c[1] + 0.1, "classic {c:?} perceptual {p:?}");
    }

    #[test]
    fn shade_reuses_prepared() {
        let (w, h) = (64, 16);
        let src = two_halves(w, h);
        let mut s = Settings {
            colors: rb(),
            width: 20.0,
            ..Default::default()
        };
        let p = prepare(w, h, &src, &s);
        let key = prepare_key(w, h, &src, &s);
        for kind in [TextureKind::Smooth, TextureKind::Dot, TextureKind::Noise] {
            s.amount = if kind == TextureKind::Smooth {
                0.0
            } else {
                0.7
            };
            let tex = Texture::new(
                TextureParams {
                    kind,
                    ..Default::default()
                },
                None,
            );
            // テクスチャ・Amount はキーに入らない
            assert_eq!(prepare_key(w, h, &src, &s), key);
            let a = shade(&p, &src, &s, &tex).changed;
            let b = process(w, h, &src, &s, &tex).changed;
            assert_eq!(a, b);
        }
        // 入力や色の判定に関わる設定が変わればキーも変わる
        let mut src2 = src.clone();
        src2[3][0] = 0.5;
        assert_ne!(prepare_key(w, h, &src2, &s), key);
        s.width = 40.0;
        assert_ne!(prepare_key(w, h, &src, &s), key);
    }

    /// 変わったピクセルはインデックスの昇順・重複なし
    #[test]
    fn output_is_sorted() {
        let (w, h) = (50, 37);
        let src = two_halves(w, h);
        let s = Settings {
            colors: rb(),
            width: 30.0,
            ..Default::default()
        };
        let tex = Texture::new(TextureParams::default(), None);
        let o = process(w, h, &src, &s, &tex);
        assert!(!o.changed.is_empty());
        assert!(o.changed.windows(2).all(|p| p[0].0 < p[1].0));
    }

    /// 一部分だけ描画するとき、reach だけ広げた入力で処理すれば全体で処理したのと同じになる
    #[test]
    fn crop_padded_by_reach_matches_full() {
        let (w, h) = (160usize, 40usize);
        let src = two_halves(w, h);
        let mut colors = rb();
        colors[0].grow = 6.0;
        colors[1].spread = 2.0;
        let s = Settings {
            colors,
            width: 20.0,
            offset: 4.0,
            roughness: 3.0,
            ..Default::default()
        };
        let tex = Texture::new(TextureParams::default(), None);
        let full = process(w, h, &src, &s, &tex).to_dense(&src);

        // 境界(x=80)のすぐ右から 8px 幅の範囲を描画したい
        let (rx0, rx1) = (82usize, 90usize);
        let pad = reach(&s).ceil() as usize + 2;
        let cx0 = rx0.saturating_sub(pad);
        let cx1 = (rx1 + pad).min(w);
        let cw = cx1 - cx0;
        let crop: Vec<[f32; 4]> = (0..h)
            .flat_map(|y| src[y * w + cx0..y * w + cx1].to_vec())
            .collect();
        let cs = Settings {
            origin: (cx0 as i32, 0),
            ..s
        };
        let part = process(cw, h, &crop, &cs, &tex).to_dense(&crop);
        for y in 0..h {
            for x in rx0..rx1 {
                assert_eq!(
                    full.rgba[y * w + x],
                    part.rgba[y * cw + x - cx0],
                    "x={x} y={y}"
                );
            }
        }
    }

    /// 全部の色を同じだけ Grow しても、届く範囲の端で段差が出ない
    #[test]
    fn equal_grow_has_no_seam() {
        let (w, h) = (200, 2);
        let src = two_halves(w, h);
        let mut colors = rb();
        colors[0].grow = 10.0;
        colors[1].grow = 10.0;
        let s = Settings {
            colors,
            width: 20.0,
            ..Default::default()
        };
        let tex = Texture::new(TextureParams::default(), None);
        let row = blue_row(&process(w, h, &src, &s, &tex).to_dense(&src), w);
        let worst = row
            .windows(2)
            .map(|p| (p[1] - p[0]).abs())
            .fold(0.0f32, f32::max);
        assert!(worst < 0.06, "max step {worst} {:?}", &row[40..80]);
    }

    /// Softness を上げても、色の割合（面積率）は重みのまま。重み 0・1 のところも変わらない
    #[test]
    fn softness_keeps_coverage_and_ends() {
        for soft in [0.0f32, 0.3, 1.0, 5.0] {
            for a in [0.0f32, 0.05, 0.3, 0.5, 0.8, 1.0] {
                let n = 20000;
                let mean: f32 = (0..n)
                    .map(|i| soft_step(a, (i as f32 + 0.5) / n as f32, soft))
                    .sum::<f32>()
                    / n as f32;
                assert!((mean - a).abs() < 1e-3, "soft={soft} a={a} mean={mean}");
            }
            // a について単調
            for t in [0.1f32, 0.45, 0.9] {
                let mut prev = 0.0;
                for k in 0..=100 {
                    let v = soft_step(k as f32 / 100.0, t, soft);
                    assert!(v + 1e-6 >= prev);
                    prev = v;
                }
            }
        }
    }

    /// Softness を上げると、くっきりした2値ではなく中間の色が増える
    #[test]
    fn softness_blends_texture() {
        let (w, h) = (200, 60);
        let src = two_halves(w, h);
        let tex = Texture::new(
            TextureParams {
                kind: TextureKind::Noise,
                cell: 6.0,
                ..Default::default()
            },
            None,
        );
        let mid_count = |softness: f32| {
            let s = Settings {
                colors: rb(),
                width: 120.0,
                softness,
                aa_samples: 1,
                ..Default::default()
            };
            let o = process(w, h, &src, &s, &tex).to_dense(&src);
            o.rgba.iter().filter(|p| p[2] > 0.1 && p[2] < 0.9).count()
        };
        let hard = mid_count(0.0);
        let soft = mid_count(3.0);
        assert_eq!(hard, 0, "AA なし・Softness 0 なら2値");
        assert!(soft > 1000, "{soft}");
    }

    /// Mid Width を小さくすると中間色の帯が細くなり、真ん中の量は変わらない
    #[test]
    fn mid_width_narrows_band() {
        let (w, h) = (80, 2);
        let src = two_halves(w, h);
        let band = |mid_width: f32| {
            let s = Settings {
                colors: rb(),
                width: 40.0,
                mid_color: [0.0, 1.0, 0.0],
                mid_amount: 1.0,
                mid_width,
                ..Default::default()
            };
            let tex = Texture::new(TextureParams::default(), None);
            let o = process(w, h, &src, &s, &tex).to_dense(&src);
            let g: Vec<f32> = (0..w).map(|x| o.rgba[x][1]).collect();
            (g.iter().filter(|&&v| v > 0.5).count(), g[39].max(g[40]))
        };
        let (wide, peak_w) = band(1.0);
        let (narrow, peak_n) = band(0.0);
        assert!(narrow < wide, "{narrow} {wide}");
        assert!(peak_w > 0.9 && peak_n > 0.9);
    }

    /// Core では中間色が網点の中心側、Rim ではふち側に出る
    #[test]
    fn mid_position_moves_mid_in_dots() {
        let (w, h) = (120, 120);
        let src = two_halves(w, h);
        let tex = Texture::new(
            TextureParams {
                kind: TextureKind::Dot,
                cell: 12.0,
                ..Default::default()
            },
            None,
        );
        let run = |mid_position| {
            let s = Settings {
                colors: rb(),
                width: 200.0,
                aa_samples: 1,
                mid_color: [0.0, 1.0, 0.0],
                mid_amount: 0.5,
                mid_position,
                ..Default::default()
            };
            process(w, h, &src, &s, &tex).to_dense(&src)
        };
        let rim = run(MidPosition::Rim);
        let core = run(MidPosition::Core);
        // 色の割合は同じ（中間色の画素数がほぼ同じ）で、置き場所だけが違う
        let count = |o: &Dense| o.rgba.iter().filter(|p| p[1] > 0.5).count() as f32;
        assert!((count(&rim) - count(&core)).abs() / count(&rim) < 0.1);
        assert!(rim.rgba != core.rgba);
    }

    /// いろいろな大きさ・設定で落ちず、出力が範囲内・昇順・有限の値になる
    #[test]
    fn random_inputs_do_not_panic() {
        let mut r = 0x1234_5678u32;
        let mut rnd = move || {
            r ^= r << 13;
            r ^= r >> 17;
            r ^= r << 5;
            r
        };
        let palette = [
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
        ];
        for case in 0..120 {
            let w = 1 + (rnd() % 23) as usize;
            let h = 1 + (rnd() % 17) as usize;
            let src: Vec<[f32; 4]> = (0..w * h)
                .map(|_| {
                    let c = palette[(rnd() % 5).min(3) as usize];
                    let a = if rnd() % 7 == 0 { 0.0 } else { 1.0 };
                    [c[0], c[1], c[2], a]
                })
                .collect();
            let nc = (rnd() % 5) as usize;
            let mut colors: Vec<GroupColor> =
                palette[..nc].iter().map(|&c| GroupColor::new(c)).collect();
            for c in &mut colors {
                c.grow = (rnd() % 11) as f32 - 5.0;
                c.spread = (rnd() % 4) as f32 * 0.5;
            }
            let s = Settings {
                colors,
                width: (rnd() % 30) as f32,
                offset: (rnd() % 7) as f32 - 3.0,
                reverse: rnd() % 2 == 0,
                max_neighbors: 1 + (rnd() % 6) as usize,
                softness: (rnd() % 3) as f32 * 0.5,
                aa_samples: [1, 2, 4][(rnd() % 3) as usize],
                roughness: (rnd() % 3) as f32,
                mid_amount: (rnd() % 3) as f32 * 0.5,
                mid_width: (rnd() % 3) as f32 * 0.5,
                mid_position: if rnd() % 2 == 0 {
                    MidPosition::Core
                } else {
                    MidPosition::Rim
                },
                output: if rnd() % 5 == 0 {
                    OutputMode::Gradient
                } else {
                    OutputMode::Result
                },
                ..Default::default()
            };
            let kind = [
                TextureKind::Smooth,
                TextureKind::Dot,
                TextureKind::Line,
                TextureKind::Bayer,
                TextureKind::Grain,
                TextureKind::Noise,
            ][(rnd() % 6) as usize];
            let tex = Texture::new(
                TextureParams {
                    kind,
                    cell: 1.0 + (rnd() % 9) as f32,
                    ..Default::default()
                },
                None,
            );
            let o = process(w, h, &src, &s, &tex);
            assert!(
                o.changed.windows(2).all(|p| p[0].0 < p[1].0),
                "case {case}: not sorted"
            );
            for &(i, p) in &o.changed {
                assert!((i as usize) < w * h, "case {case}: index out of range");
                assert!(p.iter().all(|v| v.is_finite()), "case {case}: {p:?}");
            }
        }
    }

    /// Custom カーブ：点が無ければ Linear と同じ、点を入れると重みが変わり、両端は変わらない
    #[test]
    fn custom_curve_maps_weights() {
        let (w, h) = (40, 2);
        let src = two_halves(w, h);
        let tex = Texture::new(TextureParams::default(), None);
        let base = Settings {
            colors: rb(),
            width: 20.0,
            ..Default::default()
        };
        let linear = process(w, h, &src, &base, &tex).to_dense(&src);
        let same = process(
            w,
            h,
            &src,
            &Settings {
                curve: Curve::Custom,
                ..base.clone()
            },
            &tex,
        )
        .to_dense(&src);
        assert_eq!(linear.rgba, same.rgba);
        let bent = process(
            w,
            h,
            &src,
            &Settings {
                curve: Curve::Custom,
                curve_points: vec![[0.5, 0.9]],
                ..base.clone()
            },
            &tex,
        )
        .to_dense(&src);
        // 曲げたぶん真ん中付近は変わり、グラデーションの外（端）は同じ
        assert!(linear.rgba[18] != bent.rgba[18]);
        assert_eq!(linear.rgba[2], bent.rgba[2]);
        assert_eq!(linear.rgba[37], bent.rgba[37]);
    }

    /// Softness は境目をぼかす：網点の中心と外側は変わらず、ふちだけが中間の色になる
    #[test]
    fn softness_blurs_only_edges() {
        let (w, h) = (120, 120);
        let src = two_halves(w, h);
        let tex = Texture::new(
            TextureParams {
                kind: TextureKind::Dot,
                cell: 16.0,
                ..Default::default()
            },
            None,
        );
        let run = |softness: f32| {
            let s = Settings {
                colors: rb(),
                width: 400.0,
                softness,
                aa_samples: 1,
                ..Default::default()
            };
            process(w, h, &src, &s, &tex).to_dense(&src)
        };
        let hard = run(0.0);
        let soft = run(2.0);
        let mut same = 0;
        let mut mid = 0;
        for i in 0..w * h {
            let (a, b) = (hard.rgba[i][2], soft.rgba[i][2]);
            if (a - b).abs() < 1e-6 {
                same += 1;
            }
            if b > 0.05 && b < 0.95 {
                mid += 1;
            }
        }
        // 大半（網点の中と外）はそのまま、ふちに中間の色が出る
        assert!(same > w * h / 2, "{same}");
        assert!(mid > w * h / 20, "{mid}");
    }

    /// Map モード：テクスチャの濃淡がそのまま連続的に反映され（2値にならない）、
    /// 平均の色の割合は重みのまま、両端は変わらない
    #[test]
    fn map_mode_is_continuous() {
        for a in [0.0f32, 0.1, 0.5, 0.9, 1.0] {
            let n = 10000;
            let mean: f32 = (0..n)
                .map(|i| map_step(a, (i as f32 + 0.5) / n as f32, 1.0))
                .sum::<f32>()
                / n as f32;
            assert!((mean - a).abs() < 1e-4, "a={a} mean={mean}");
        }
        assert_eq!(map_step(0.0, 0.1, 1.0), 0.0);
        assert_eq!(map_step(1.0, 0.9, 1.0), 1.0);

        let (w, h) = (200, 60);
        let src = two_halves(w, h);
        let tex = Texture::new(
            TextureParams {
                kind: TextureKind::Noise,
                cell: 8.0,
                ..Default::default()
            },
            None,
        );
        let s = Settings {
            colors: rb(),
            width: 160.0,
            texture_mode: TextureMode::Map,
            aa_samples: 1,
            ..Default::default()
        };
        let o = process(w, h, &src, &s, &tex).to_dense(&src);
        // 2値ではなく、中間の色がたくさん出る
        let mid = o.rgba.iter().filter(|p| p[2] > 0.1 && p[2] < 0.9).count();
        assert!(mid > w * h / 4, "{mid}");
        // テクスチャの濃淡に応じて値がばらつく（なめらかなグラデーションとは違う）
        let smooth = process(
            w,
            h,
            &src,
            &Settings {
                amount: 0.0,
                ..s.clone()
            },
            &tex,
        )
        .to_dense(&src);
        assert!(o.rgba != smooth.rgba);
    }
}
