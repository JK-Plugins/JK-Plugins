//! 塗りつぶし消去のコア処理（AE に依存しないのでテストできる）。
//!
//! 指定した点の色を基準に、似た色でつながっている範囲（塗りつぶしツールと同じ範囲）を求め、
//! その範囲のアルファを下げて消す。

/// 指定できる点の最大数
pub const MAX_POINTS: usize = 99;

#[derive(Clone, Debug)]
pub struct Settings {
    /// 消す起点（ワールドのピクセル座標）
    pub points: Vec<(f32, f32)>,
    /// 起点の色との差がこれ以下なら同じ色とみなす（0..1、RGBA の各チャンネルの差の最大）
    pub tolerance: f32,
    /// 許容量を超えてからこの幅（0..1）で、だんだん消え方を弱くする（0 でくっきり）
    pub softness: f32,
    /// 消す範囲のふちを動かすピクセル数。プラスで広げ（線画のふちの中間色まで消す）、マイナスで縮める
    pub choke: f32,
    /// true：起点とつながっている範囲だけ。false：画像全体の似た色を全部
    pub contiguous: bool,
    /// アルファも比べる（透明なところと不透明なところを別の色とみなす）
    pub compare_alpha: bool,
    /// 消す範囲を反転する（範囲だけを残す）
    pub invert: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            points: Vec::new(),
            tolerance: 0.1,
            softness: 0.0,
            choke: 0.0,
            contiguous: true,
            compare_alpha: true,
            invert: false,
        }
    }
}

/// 2つの色の差（チャンネルごとの差の最大）
#[inline]
fn distance(a: &[f32; 4], b: &[f32; 4], compare_alpha: bool) -> f32 {
    let d = (a[0] - b[0])
        .abs()
        .max((a[1] - b[1]).abs())
        .max((a[2] - b[2]).abs());
    if compare_alpha {
        d.max((a[3] - b[3]).abs())
    } else {
        d
    }
}

/// 色の差から消す量（1 = 全部消す、0 = 残す）
#[inline]
fn amount(d: f32, tol: f32, soft: f32) -> f32 {
    if d <= tol {
        1.0
    } else if soft > 0.0 && d < tol + soft {
        1.0 - (d - tol) / soft
    } else {
        0.0
    }
}

/// 画素の読み出し。入力のバイト列から必要な画素だけ読めるようにする（全体を float にしない）
pub trait Pixels: Sync {
    /// (x, y) の画素 [r, g, b, a]（0..1）
    fn px(&self, x: usize, y: usize) -> [f32; 4];
}

/// float の配列（行優先、幅 w）
pub struct Image<'a> {
    pub data: &'a [[f32; 4]],
    pub w: usize,
}

impl Pixels for Image<'_> {
    #[inline]
    fn px(&self, x: usize, y: usize) -> [f32; 4] {
        self.data[y * self.w + x]
    }
}

/// 画像の中にある点（ピクセル番号と、その色）。Contiguous でないときは同じ色の点を1つにまとめる
pub fn seeds<P: Pixels + ?Sized>(
    src: &P,
    w: usize,
    h: usize,
    s: &Settings,
) -> Vec<(usize, [f32; 4])> {
    let mut out: Vec<(usize, [f32; 4])> = Vec::new();
    for &(px, py) in &s.points {
        let (x, y) = (px.floor(), py.floor());
        if !(x >= 0.0 && y >= 0.0 && (x as usize) < w && (y as usize) < h) {
            // 画像の外の点は使わない
            continue;
        }
        let (x, y) = (x as usize, y as usize);
        let c = src.px(x, y);
        if !s.contiguous && out.iter().any(|o| o.1 == c) {
            continue;
        }
        out.push((y * w + x, c));
        // 塗りつぶしの作業用の配列は点の番号を 1 バイトで持つ（0 は「まだ」）
        if out.len() == 254 {
            break;
        }
    }
    out
}

/// 消す量のマスク（w×h、0..1）を求める。Invert はまだ掛けない
pub fn erase_mask(src: &[[f32; 4]], w: usize, h: usize, s: &Settings) -> Vec<f32> {
    erase_mask_with(&Image { data: src, w }, w, h, s)
}

/// [`erase_mask`] の、画素の読み方を選べる版
pub fn erase_mask_with<P: Pixels + ?Sized>(src: &P, w: usize, h: usize, s: &Settings) -> Vec<f32> {
    let mut mask = fill_mask(src, w, h, s);
    choke(&mut mask, w, h, s.choke);
    mask
}

/// Invert まで掛けた、各ピクセルの消す量
pub fn erase_amount(src: &[[f32; 4]], w: usize, h: usize, s: &Settings) -> Vec<f32> {
    let mut m = erase_mask(src, w, h, s);
    if s.invert {
        for v in &mut m {
            *v = 1.0 - *v;
        }
    }
    m
}

thread_local! {
    /// 塗りつぶしの作業用の配列（調べたか・最初に塗った点）。フレームごとに確保し直さない
    static WORK: std::cell::RefCell<(Vec<u8>, Vec<u8>)> = const { std::cell::RefCell::new((Vec::new(), Vec::new())) };
}

/// 点ごとの塗りつぶしの範囲を合わせたマスク（Choke の前）
pub fn fill_mask<P: Pixels + ?Sized>(src: &P, w: usize, h: usize, s: &Settings) -> Vec<f32> {
    let mut mask = vec![0f32; w * h];
    if w == 0 || h == 0 {
        return mask;
    }
    let tol = s.tolerance.max(0.0);
    let soft = s.softness.max(0.0);
    let seeds = seeds(src, w, h, s);
    if seeds.is_empty() {
        return mask;
    }

    if !s.contiguous {
        // 画像全体の似た色：ピクセルごとに独立なので、行で分けて並列に
        let rows = crate::world_io::thread_rows(h);
        crate::par::scope(|sc| {
            for (ci, mc) in mask.chunks_mut(rows * w).enumerate() {
                let seeds = &seeds;
                sc.spawn(move || {
                    for (r, line) in mc.chunks_mut(w).enumerate() {
                        let y = ci * rows + r;
                        for (x, m) in line.iter_mut().enumerate() {
                            let p = src.px(x, y);
                            let mut best = 0f32;
                            for (_, c) in seeds {
                                best =
                                    best.max(amount(distance(&p, c, s.compare_alpha), tol, soft));
                                if best >= 1.0 {
                                    break;
                                }
                            }
                            *m = best;
                        }
                    }
                });
            }
        });
        return mask;
    }

    WORK.with_borrow_mut(|(visited, owner)| {
        // 調べたかどうかを点ごとの番号（1..=99）で持つ（点ごとに配列を作り直さない）
        visited.clear();
        visited.resize(w * h, 0);
        // そのピクセルを最初に塗った点の番号 + 1（0 = まだ）
        owner.clear();
        owner.resize(w * h, 0);
        fill_contiguous(src, w, h, s, tol, soft, &seeds, &mut mask, visited, owner);
    });
    mask
}

#[allow(clippy::too_many_arguments)]
fn fill_contiguous<P: Pixels + ?Sized>(
    src: &P,
    w: usize,
    h: usize,
    s: &Settings,
    tol: f32,
    soft: f32,
    seeds: &[(usize, [f32; 4])],
    mask: &mut [f32],
    visited: &mut [u8],
    owner: &mut [u8],
) {
    let mut stack: Vec<(usize, usize, usize)> = Vec::new();
    for (n, &(seed_i, seed)) in seeds.iter().enumerate() {
        let o = owner[seed_i];
        if o != 0 && seeds[o as usize - 1].1 == seed {
            // 同じ色の点が塗った範囲の中にある → 範囲は同じなので塗り直さない
            continue;
        }
        // 点は最大 99 個なので 1 バイトに収まる
        let gen = (n + 1) as u8;
        let amt =
            |x: usize, y: usize| amount(distance(&src.px(x, y), &seed, s.compare_alpha), tol, soft);
        let mut set = |i: usize, a: f32, mask: &mut [f32]| {
            if a > mask[i] {
                mask[i] = a;
            }
            if owner[i] == 0 {
                owner[i] = gen;
            }
        };
        // 区間（スパン）で塗る：横につながった区間をまとめて塗って積み、上下の行で塗れる区間を探す。
        // 調べる前に印をつけるので、どのピクセルも色を比べるのは1回だけ。
        // 4近傍（斜めは通さないので、線のすき間から漏れにくい）
        // 塗れる (x, y) から左右に広げて塗り、区間の両端を返す
        let mut run =
            |x: usize, y: usize, a: f32, mask: &mut [f32], visited: &mut [u8]| -> (usize, usize) {
                let row = y * w;
                set(row + x, a, mask);
                let mut l = x;
                while l > 0 {
                    let j = row + l - 1;
                    if visited[j] == gen {
                        break;
                    }
                    visited[j] = gen;
                    let a = amt(l - 1, y);
                    if a <= 0.0 {
                        break;
                    }
                    set(j, a, mask);
                    l -= 1;
                }
                let mut r = x;
                while r + 1 < w {
                    let j = row + r + 1;
                    if visited[j] == gen {
                        break;
                    }
                    visited[j] = gen;
                    let a = amt(r + 1, y);
                    if a <= 0.0 {
                        break;
                    }
                    set(j, a, mask);
                    r += 1;
                }
                (l, r)
            };
        let (sx, sy) = (seed_i % w, seed_i / w);
        visited[seed_i] = gen;
        let a = amt(sx, sy);
        if a <= 0.0 {
            continue;
        }
        stack.clear();
        let (l, r) = run(sx, sy, a, mask, visited);
        stack.push((sy, l, r));
        while let Some((y, l, r)) = stack.pop() {
            for ny in [y.wrapping_sub(1), y + 1] {
                if ny >= h {
                    continue;
                }
                let mut x = l;
                while x <= r {
                    let j = ny * w + x;
                    if visited[j] == gen {
                        x += 1;
                        continue;
                    }
                    visited[j] = gen;
                    let a = amt(x, ny);
                    if a <= 0.0 {
                        x += 1;
                        continue;
                    }
                    let (nl, nr) = run(x, ny, a, mask, visited);
                    stack.push((ny, nl, nr));
                    // nr + 1 は塗れないか調べ済み
                    x = nr + 2;
                }
            }
        }
    }
}

/// 消す範囲のふちを動かす。プラス：全部消すピクセルから choke 以内も消す。
/// マイナス：残すピクセル（消す量 0）から |choke| 以内は残す。どちらも外側 1px でなめらかにする
pub fn choke(mask: &mut [f32], w: usize, h: usize, choke: f32) {
    if choke > 0.0 {
        let d = distance_to(w, h, |i| mask[i] >= 1.0);
        for (m, d) in mask.iter_mut().zip(d) {
            *m = m.max((choke + 1.0 - d).clamp(0.0, 1.0));
        }
    } else if choke < 0.0 {
        let r = -choke;
        let d = distance_to(w, h, |i| mask[i] <= 0.0);
        for (m, d) in mask.iter_mut().zip(d) {
            *m = m.min(1.0 - (r + 1.0 - d).clamp(0.0, 1.0));
        }
    }
}

/// 各ピクセルから、`is_source` が true の一番近いピクセルまでの距離（ユークリッド距離）。
/// source が1つもなければすべて無限大。縦はそのまま数え、横は Felzenszwalb の方法で求める。
/// どちらも列・行ごとに独立なので並列にする
pub fn distance_to(w: usize, h: usize, is_source: impl Fn(usize) -> bool + Sync) -> Vec<f32> {
    const INF: f32 = 1e20;
    if w == 0 || h == 0 {
        return Vec::new();
    }
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    // 縦：画像を縦長の帯に分け、帯ごとに上から・下から1行ずつ見て、同じ列の一番近い source までの
    // 距離の2乗を求める（帯の中は行優先に並べるので、メモリを順に読み書きする）
    let band_w = w.div_ceil(threads).max(1);
    let bands: Vec<(usize, Vec<f32>)> = crate::par::scope(|sc| {
        let hs: Vec<_> = (0..w)
            .step_by(band_w)
            .map(|x0| {
                let is_source = &is_source;
                sc.spawn(move || {
                    let bw = band_w.min(w - x0);
                    // 上から：一番近い source までの行数（なければ大きな値）
                    let mut up = vec![u32::MAX; bw * h];
                    for y in 0..h {
                        for k in 0..bw {
                            let v = if is_source(y * w + x0 + k) {
                                0
                            } else if y > 0 {
                                up[(y - 1) * bw + k].saturating_add(1)
                            } else {
                                u32::MAX
                            };
                            up[y * bw + k] = v;
                        }
                    }
                    // 下から合わせ、距離の2乗にする
                    let mut g = vec![INF; bw * h];
                    let mut down = vec![u32::MAX; bw];
                    for y in (0..h).rev() {
                        for k in 0..bw {
                            let u = up[y * bw + k];
                            down[k] = if u == 0 { 0 } else { down[k].saturating_add(1) };
                            let d = u.min(down[k]);
                            if d != u32::MAX {
                                g[y * bw + k] = (d as f32) * (d as f32);
                            }
                        }
                    }
                    (bw, g)
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    // 横：行ごとに min_x' (g + (x − x')²)
    let mut d = vec![0f32; w * h];
    let rows = h.div_ceil(threads).max(1);
    crate::par::scope(|sc| {
        for (ri, chunk) in d.chunks_mut(rows * w).enumerate() {
            let bands = &bands;
            sc.spawn(move || {
                let mut f = vec![0f32; w];
                let mut v = vec![0usize; w];
                let mut z = vec![0f32; w + 1];
                for (k, row) in chunk.chunks_mut(w).enumerate() {
                    let y = ri * rows + k;
                    let mut x = 0;
                    for (bw, g) in bands {
                        f[x..x + bw].copy_from_slice(&g[y * bw..(y + 1) * bw]);
                        x += bw;
                    }
                    edt_1d(&f, row, &mut v, &mut z);
                    for o in row.iter_mut() {
                        *o = if *o >= INF { f32::INFINITY } else { o.sqrt() };
                    }
                }
            });
        }
    });
    d
}

/// 1次元の距離の2乗の変換：out[q] = min_p (f[p] + (q − p)²)
fn edt_1d(f: &[f32], out: &mut [f32], v: &mut [usize], z: &mut [f32]) {
    let n = f.len();
    if n == 0 {
        return;
    }
    let mut k = 0;
    v[0] = 0;
    z[0] = f32::NEG_INFINITY;
    z[1] = f32::INFINITY;
    for q in 1..n {
        let mut sx;
        loop {
            let p = v[k];
            sx =
                ((f[q] + (q * q) as f32) - (f[p] + (p * p) as f32)) / (2.0 * (q as f32 - p as f32));
            if sx <= z[k] && k > 0 {
                k -= 1;
            } else {
                break;
            }
        }
        if sx <= z[k] {
            v[0] = q;
            z[1] = f32::INFINITY;
            continue;
        }
        k += 1;
        v[k] = q;
        z[k] = sx;
        z[k + 1] = f32::INFINITY;
    }
    let mut k = 0;
    for (q, o) in out.iter_mut().enumerate() {
        while z[k + 1] < q as f32 {
            k += 1;
        }
        let p = v[k];
        let dq = q as f32 - p as f32;
        *o = f[p] + dq * dq;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
    const BLACK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

    /// 7×5 の白い画像の真ん中に縦の黒い線（x = 3）
    fn split_image() -> (Vec<[f32; 4]>, usize, usize) {
        let (w, h) = (7, 5);
        let v = (0..w * h)
            .map(|i| if i % w == 3 { BLACK } else { WHITE })
            .collect();
        (v, w, h)
    }

    fn settings(points: &[(f32, f32)]) -> Settings {
        Settings {
            points: points.to_vec(),
            ..Default::default()
        }
    }

    #[test]
    fn fills_only_connected_side() {
        let (src, w, h) = split_image();
        let m = erase_mask(&src, w, h, &settings(&[(1.5, 2.5)]));
        for y in 0..h {
            for x in 0..w {
                let want = if x < 3 { 1.0 } else { 0.0 };
                assert_eq!(m[y * w + x], want, "({x}, {y})");
            }
        }
    }

    #[test]
    fn non_contiguous_fills_all_similar() {
        let (src, w, h) = split_image();
        let mut s = settings(&[(1.0, 1.0)]);
        s.contiguous = false;
        let m = erase_mask(&src, w, h, &s);
        assert!((0..w * h).all(|i| m[i] == if i % w == 3 { 0.0 } else { 1.0 }));
    }

    #[test]
    fn diagonal_gap_does_not_leak() {
        // 斜めの黒い線（x == y）で分かれた画像
        let (w, h) = (5, 5);
        let src: Vec<_> = (0..w * h)
            .map(|i| if i % w == i / w { BLACK } else { WHITE })
            .collect();
        let m = erase_mask(&src, w, h, &settings(&[(4.0, 0.0)]));
        assert_eq!(m[4], 1.0);
        assert_eq!(m[4 * w], 0.0);
    }

    #[test]
    fn tolerance_and_softness() {
        let src = vec![
            [0.5, 0.5, 0.5, 1.0],
            [0.55, 0.5, 0.5, 1.0],
            [0.65, 0.5, 0.5, 1.0],
            [0.9, 0.5, 0.5, 1.0],
        ];
        let mut s = settings(&[(0.0, 0.0)]);
        s.tolerance = 0.06;
        s.softness = 0.1;
        let m = erase_mask(&src, 4, 1, &s);
        assert_eq!(m[0], 1.0);
        assert_eq!(m[1], 1.0);
        // 差 0.15 は許容量 0.06 を 0.09 超えている → 1 − 0.09 / 0.1
        assert!((m[2] - 0.1).abs() < 1e-4, "{}", m[2]);
        assert_eq!(m[3], 0.0);
    }

    #[test]
    fn point_outside_is_ignored() {
        let (src, w, h) = split_image();
        let m = erase_mask(&src, w, h, &settings(&[(-1.0, 2.0), (100.0, 2.0)]));
        assert!(m.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn choke_plus_eats_the_line() {
        let (src, w, h) = split_image();
        let mut s = settings(&[(0.0, 0.0)]);
        s.choke = 1.0;
        let m = erase_mask(&src, w, h, &s);
        assert_eq!(m[2 * w + 3], 1.0, "line is erased");
        assert!(m[2 * w + 4] < 1.0, "right side mostly remains");
    }

    #[test]
    fn invert_flips_amount() {
        let (src, w, h) = split_image();
        let s = settings(&[(0.0, 0.0)]);
        let a = erase_amount(&src, w, h, &s);
        let b = erase_amount(&src, w, h, &Settings { invert: true, ..s });
        assert!(a.iter().zip(&b).all(|(a, b)| a + b == 1.0));
        assert!((0..w * h).all(|i| a[i] == if i % w < 3 { 1.0 } else { 0.0 }));
    }

    #[test]
    fn point_in_other_colors_region_is_not_skipped() {
        // x = 0..3 が灰色 0.5、3..6 が 0.55、6 が 0.62、7 が黒、8..10 が 0.55（黒で分かれた別の範囲）。
        // 点1（x = 9、0.55）は右の範囲だけ。点2（0.5）は 0.55 の帯まで塗る。
        // 点3（x = 4、0.55）は点2が塗った中にあり、0.55 の点はもう使ったが、塗ったのは別の色の点なので
        // 飛ばしてはいけない（飛ばすと 0.62 の列が残る）
        let (w, h) = (10, 3);
        let g1 = [0.5, 0.5, 0.5, 1.0];
        let g2 = [0.55, 0.55, 0.55, 1.0];
        let g3 = [0.62, 0.62, 0.62, 1.0];
        let src: Vec<_> = (0..w * h)
            .map(|i| match i % w {
                0..=2 => g1,
                3..=5 => g2,
                6 => g3,
                7 => BLACK,
                _ => g2,
            })
            .collect();
        let mut s = settings(&[(9.0, 0.0), (0.0, 0.0), (4.0, 1.0)]);
        s.tolerance = 0.1;
        let m = erase_mask(&src, w, h, &s);
        assert!((0..h).all(|y| m[y * w + 6] == 1.0));
        assert!((0..h).all(|y| m[y * w + 7] == 0.0));
    }

    #[test]
    fn scanline_fill_matches_simple_flood() {
        // 迷路のような絵で、スキャンライン法と素朴な4近傍の塗りつぶしが同じになる
        let (w, h) = (31, 23);
        let src: Vec<_> = (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                let wall = (x % 4 == 2 && (y + x / 4) % 7 != 0) || (y % 5 == 3 && (x + y) % 9 == 0);
                if wall {
                    BLACK
                } else {
                    WHITE
                }
            })
            .collect();
        for start in [(0.0, 0.0), (15.0, 11.0), (30.0, 22.0)] {
            let m = erase_mask(&src, w, h, &settings(&[start]));
            let seed = src[start.1 as usize * w + start.0 as usize];
            let mut want = vec![0f32; w * h];
            let mut q = vec![start.1 as usize * w + start.0 as usize];
            while let Some(i) = q.pop() {
                if want[i] == 1.0 || src[i] != seed {
                    continue;
                }
                want[i] = 1.0;
                let (x, y) = (i % w, i / w);
                if x > 0 {
                    q.push(i - 1);
                }
                if x + 1 < w {
                    q.push(i + 1);
                }
                if y > 0 {
                    q.push(i - w);
                }
                if y + 1 < h {
                    q.push(i + w);
                }
            }
            assert_eq!(m, want, "{start:?}");
        }
    }

    #[test]
    fn choke_minus_shrinks_region() {
        let (src, w, h) = split_image();
        let mut s = settings(&[(0.0, 0.0)]);
        s.choke = -1.0;
        let m = erase_mask(&src, w, h, &s);
        // 1px 縮む：線のとなり（x = 2）だけ残る
        assert_eq!(m[2 * w + 2], 0.0);
        assert_eq!(m[2 * w + 1], 1.0);
        assert_eq!(m[2 * w], 1.0);

        // 1.5px：x = 1 は半分だけ消える
        s.choke = -1.5;
        let m = erase_mask(&src, w, h, &s);
        assert_eq!(m[2 * w + 2], 0.0);
        assert!((m[2 * w + 1] - 0.5).abs() < 1e-5, "{}", m[2 * w + 1]);
    }

    #[test]
    fn choke_minus_without_kept_pixels_changes_nothing() {
        let src = vec![WHITE; 9];
        let mut s = settings(&[(1.0, 1.0)]);
        s.choke = -3.0;
        assert!(erase_mask(&src, 3, 3, &s).iter().all(|&v| v == 1.0));
    }

    #[test]
    fn distance_transform_matches_brute_force() {
        let (w, h) = (13, 9);
        let src = |i: usize| (i * 7919) % 11 == 0;
        let d = distance_to(w, h, src);
        for i in 0..w * h {
            let (x, y) = ((i % w) as f32, (i / w) as f32);
            let want = (0..w * h)
                .filter(|&j| src(j))
                .map(|j| (((j % w) as f32 - x).powi(2) + ((j / w) as f32 - y).powi(2)).sqrt())
                .fold(f32::INFINITY, f32::min);
            assert!((d[i] - want).abs() < 1e-3, "{i}: {} vs {want}", d[i]);
        }
    }

    #[test]
    fn many_points_union() {
        // 縦線を2本（x = 2, 5）引いた 8×3 の画像の、左と右の2か所から消す
        let (w, h) = (8, 3);
        let src: Vec<_> = (0..w * h)
            .map(|i| {
                if i % w == 2 || i % w == 5 {
                    BLACK
                } else {
                    WHITE
                }
            })
            .collect();
        let m = erase_mask(&src, w, h, &settings(&[(0.0, 0.0), (7.0, 2.0), (0.0, 1.0)]));
        for i in 0..w * h {
            let want = if matches!(i % w, 0 | 1 | 6 | 7) {
                1.0
            } else {
                0.0
            };
            assert_eq!(m[i], want, "x = {}", i % w);
        }
    }
}
