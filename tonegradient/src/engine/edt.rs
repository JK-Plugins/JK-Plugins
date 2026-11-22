//! 最近傍シード探索付きのユークリッド距離変換（Felzenszwalb & Huttenlocher）。
//!
//! 2パス（列 → 行）で、各ピクセルから最も近いシードの位置と二乗距離を求める。
//! 計算量は O(W×H)。列・行ごとに独立なのでスレッド分割する。

pub const NO_SEED: u32 = u32::MAX;
const INF: f64 = 1e30;

/// 1次元の距離変換。`f[q]` が INF の位置は放物線を追加しない。
/// `d` に二乗距離、`arg` があれば最小を与える位置を書く。
fn dt1d(f: &[f64], d: &mut [f64], mut arg: Option<&mut [u32]>, v: &mut [usize], z: &mut [f64]) {
    let n = f.len();
    let mut k: isize = -1;
    for q in 0..n {
        if f[q] >= INF {
            continue;
        }
        let fq = f[q] + (q * q) as f64;
        loop {
            if k < 0 {
                k = 0;
                v[0] = q;
                z[0] = -INF;
                z[1] = INF;
                break;
            }
            let p = v[k as usize];
            let s = (fq - (f[p] + (p * p) as f64)) / (2.0 * (q as f64 - p as f64));
            if s <= z[k as usize] {
                k -= 1;
                continue;
            }
            k += 1;
            v[k as usize] = q;
            z[k as usize] = s;
            z[k as usize + 1] = INF;
            break;
        }
    }
    if k < 0 {
        d.fill(INF);
        if let Some(arg) = arg {
            arg.fill(NO_SEED);
        }
        return;
    }
    let mut k = 0usize;
    for q in 0..n {
        while z[k + 1] < q as f64 {
            k += 1;
        }
        let p = v[k];
        let dq = q as f64 - p as f64;
        d[q] = dq * dq + f[p];
        if let Some(arg) = arg.as_deref_mut() {
            arg[q] = p as u32;
        }
    }
}

/// `seed[y*w+x]` が true のピクセルをシードとして、各ピクセルから最も近いシードまでの
/// 二乗距離だけを返す（シードが無ければ INFINITY）。位置を追わないぶん `nearest_seed` より軽い。
pub fn distance_sq(w: usize, h: usize, seed: &[bool]) -> Vec<f32> {
    assert_eq!(seed.len(), w * h);
    distance_sq_by(w, h, |x, y| seed[y * w + x])
}

/// `is_seed(x, y)` が true のピクセルをシードとする `distance_sq`。
/// シードの配列を別に作らずに済む（呼び出し側のラベルを直接読む）。
pub fn distance_sq_by(
    w: usize,
    h: usize,
    is_seed: impl Fn(usize, usize) -> bool + Sync,
) -> Vec<f32> {
    if w == 0 || h == 0 {
        return Vec::new();
    }
    let is_seed = &is_seed;

    // パス1：列ごと（列優先で格納）
    let mut col_d = vec![0f64; w * h]; // [x*h + y]
    {
        let threads = thread_count(w);
        let cols_per = w.div_ceil(threads);
        super::par::scope(|sc| {
            for (ci, dchunk) in col_d.chunks_mut(cols_per * h).enumerate() {
                sc.spawn(move || {
                    let mut f = vec![0f64; h];
                    let mut v = vec![0usize; h];
                    let mut z = vec![0f64; h + 1];
                    let ncols = dchunk.len() / h;
                    for c in 0..ncols {
                        let x = ci * cols_per + c;
                        for y in 0..h {
                            f[y] = if is_seed(x, y) { 0.0 } else { INF };
                        }
                        dt1d(&f, &mut dchunk[c * h..(c + 1) * h], None, &mut v, &mut z);
                    }
                });
            }
        });
    }

    // パス2：行ごと
    let mut out_d2 = vec![0f32; w * h];
    {
        let threads = thread_count(h);
        let rows_per = h.div_ceil(threads);
        let col_d = &col_d;
        super::par::scope(|sc| {
            for (ri, dchunk) in out_d2.chunks_mut(rows_per * w).enumerate() {
                sc.spawn(move || {
                    let mut f = vec![0f64; w];
                    let mut d = vec![0f64; w];
                    let mut v = vec![0usize; w];
                    let mut z = vec![0f64; w + 1];
                    let nrows = dchunk.len() / w;
                    for r in 0..nrows {
                        let y = ri * rows_per + r;
                        for x in 0..w {
                            f[x] = col_d[x * h + y];
                        }
                        dt1d(&f, &mut d, None, &mut v, &mut z);
                        for x in 0..w {
                            dchunk[r * w + x] = if d[x] >= INF {
                                f32::INFINITY
                            } else {
                                d[x] as f32
                            };
                        }
                    }
                });
            }
        });
    }
    out_d2
}

/// 壁を越えない距離変換。`passable` な画素だけを伝って届くシードのうち、
/// いちばん近いもの（直線距離）の二乗距離を返す（届かないものは INFINITY）。
///
/// 最近傍シードの位置を隣から受け渡すラスター走査（8SSEDT と同じ近傍）を、変わらなくなるまで繰り返す。
/// 開けたところでは普通の距離変換とほぼ同じになり、壁の向こうへは回り込める道があるときだけ届く。
/// 斜めの移動は、間の縦・横の画素の少なくとも一方が `passable` のときだけ（1px の斜めの線をすり抜けない）。
/// シードは `passable` であること。
pub fn distance_sq_constrained(
    w: usize,
    h: usize,
    is_seed: impl Fn(usize, usize) -> bool + Sync,
    passable: impl Fn(usize, usize) -> bool + Sync,
) -> Vec<f32> {
    let n = w * h;
    if n == 0 {
        return Vec::new();
    }
    let stripes = thread_count(h / 24).max(1);
    let rows_per = h.div_ceil(stripes);
    // 0 = 壁、1 = 通れる（距離を求める）、2 = シード
    let mut kind = vec![0u8; n];
    // シードの位置（x, y）。届いていなければ NOSEED
    let mut seed = vec![(NOSEED, 0i32); n];
    // 行ごとに、距離を求める画素がある範囲（無ければ start > end）
    let mut span = vec![(usize::MAX, 0usize); h];
    {
        let (is_seed, passable) = (&is_seed, &passable);
        super::par::scope(|sc| {
            for (i, ((kc, sc_), spc)) in kind
                .chunks_mut(rows_per * w)
                .zip(seed.chunks_mut(rows_per * w))
                .zip(span.chunks_mut(rows_per))
                .enumerate()
            {
                sc.spawn(move || {
                    let r0 = i * rows_per;
                    for (ry, sp) in spc.iter_mut().enumerate() {
                        let y = r0 + ry;
                        for x in 0..w {
                            let li = ry * w + x;
                            if is_seed(x, y) {
                                kc[li] = 2;
                                sc_[li] = (x as i32, y as i32);
                            } else if passable(x, y) {
                                kc[li] = 1;
                                sp.0 = sp.0.min(x);
                                sp.1 = sp.1.max(x);
                            }
                        }
                    }
                });
            }
        });
    }
    // 行を帯に分けてスレッドで並べて走査する。帯の外の行（上下1行）は、その回の初めの値を読む。
    // 帯をまたぐ受け渡しは回を重ねて伝わるので、変わらなくなるまで繰り返す
    let kind = &kind;
    let span = &span;
    for _ in 0..1024 {
        // 帯の境目の行の写し（帯 i の上の行・下の行）
        let edge = |y: Option<usize>| -> Option<Vec<(i32, i32)>> {
            y.map(|y| seed[y * w..(y + 1) * w].to_vec())
        };
        let edges: Vec<(Option<Vec<(i32, i32)>>, Option<Vec<(i32, i32)>>)> = (0..h
            .div_ceil(rows_per))
            .map(|i| {
                let r0 = i * rows_per;
                let r1 = (r0 + rows_per).min(h);
                (edge(r0.checked_sub(1)), edge((r1 < h).then_some(r1)))
            })
            .collect();
        let changed: bool = super::par::scope(|sc| {
            let hs: Vec<_> = seed
                .chunks_mut(rows_per * w)
                .zip(edges.iter())
                .enumerate()
                .map(|(i, (sd, (up, down)))| {
                    sc.spawn(move || {
                        let r0 = i * rows_per;
                        let r1 = r0 + sd.len() / w;
                        let mut st = Stripe {
                            w,
                            h,
                            r0,
                            r1,
                            kind,
                            seed: sd,
                            up: up.as_deref(),
                            down: down.as_deref(),
                        };
                        st.sweep(span)
                    })
                })
                .collect();
            hs.into_iter().fold(false, |c, h| h.join().unwrap() | c)
        });
        if !changed {
            break;
        }
    }
    // 二乗距離にする（帯ごとにスレッドで）
    let mut out = vec![f32::INFINITY; n];
    let seed = &seed;
    super::par::scope(|sc| {
        for (i, oc) in out.chunks_mut(rows_per * w).enumerate() {
            sc.spawn(move || {
                let base = i * rows_per * w;
                for (li, o) in oc.iter_mut().enumerate() {
                    let (sx, sy) = seed[base + li];
                    if sx != NOSEED {
                        let (x, y) = (((base + li) % w) as i64, ((base + li) / w) as i64);
                        let (ex, ey) = (x - sx as i64, y - sy as i64);
                        *o = (ex * ex + ey * ey) as f32;
                    }
                }
            });
        }
    });
    out
}

const NOSEED: i32 = i32::MIN;

/// `distance_sq_constrained` の1本の帯（行 r0..r1）
struct Stripe<'a> {
    w: usize,
    h: usize,
    r0: usize,
    r1: usize,
    kind: &'a [u8],
    seed: &'a mut [(i32, i32)],
    /// 帯のすぐ上・すぐ下の行のシード（その回の初めの値）
    up: Option<&'a [(i32, i32)]>,
    down: Option<&'a [(i32, i32)]>,
}

impl Stripe<'_> {
    #[inline(always)]
    fn seed_at(&self, x: usize, y: usize) -> (i32, i32) {
        if y < self.r0 {
            self.up.map_or((NOSEED, 0), |r| r[x])
        } else if y >= self.r1 {
            self.down.map_or((NOSEED, 0), |r| r[x])
        } else {
            self.seed[(y - self.r0) * self.w + x]
        }
    }

    /// (x, y) の画素に、隣 (x+dx, y+dy) の持つシードを受け取れるなら受け取る
    #[inline(always)]
    fn relax(&mut self, x: usize, y: usize, dx: isize, dy: isize) -> bool {
        let (w, h) = (self.w, self.h);
        let nx = x as isize + dx;
        let ny = y as isize + dy;
        if nx < 0 || ny < 0 || nx >= w as isize || ny >= h as isize {
            return false;
        }
        let (nx, ny) = (nx as usize, ny as usize);
        let (sx, sy) = self.seed_at(nx, ny);
        if sx == NOSEED {
            return false;
        }
        // 斜めは、間の縦・横の少なくとも一方が通れるときだけ（1px の斜めの線をすり抜けない）
        if dx != 0 && dy != 0 && self.kind[y * w + nx] == 0 && self.kind[ny * w + x] == 0 {
            return false;
        }
        let i = (y - self.r0) * w + x;
        let sq = |sx: i32, sy: i32| {
            let (ex, ey) = ((x as i32 - sx) as i64, (y as i32 - sy) as i64);
            ex * ex + ey * ey
        };
        let (cx, cy) = self.seed[i];
        if cx == NOSEED || sq(sx, sy) < sq(cx, cy) {
            self.seed[i] = (sx, sy);
            true
        } else {
            false
        }
    }

    /// 前向き・後ろ向きの走査を1回ずつ（8SSEDT と同じ近傍）
    fn sweep(&mut self, span: &[(usize, usize)]) -> bool {
        let w = self.w;
        let mut changed = false;
        for y in self.r0..self.r1 {
            let (a, b) = span[y];
            if a > b {
                continue;
            }
            for x in a..=b {
                if self.kind[y * w + x] != 1 {
                    continue;
                }
                for (dx, dy) in [(-1, 0), (-1, -1), (0, -1), (1, -1)] {
                    changed |= self.relax(x, y, dx, dy);
                }
            }
            for x in (a..=b).rev() {
                if self.kind[y * w + x] == 1 {
                    changed |= self.relax(x, y, 1, 0);
                }
            }
        }
        for y in (self.r0..self.r1).rev() {
            let (a, b) = span[y];
            if a > b {
                continue;
            }
            for x in (a..=b).rev() {
                if self.kind[y * w + x] != 1 {
                    continue;
                }
                for (dx, dy) in [(1, 0), (1, 1), (0, 1), (-1, 1)] {
                    changed |= self.relax(x, y, dx, dy);
                }
            }
            for x in a..=b {
                if self.kind[y * w + x] == 1 {
                    changed |= self.relax(x, y, -1, 0);
                }
            }
        }
        changed
    }
}

fn thread_count(work: usize) -> usize {
    let n = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    n.min(work.max(1)).max(1)
}

/// `seed[y*w+x]` が true のピクセルをシードとして、各ピクセルの最近傍シードの
/// インデックス（y*w+x、無ければ NO_SEED）と二乗距離を返す。
pub fn nearest_seed(w: usize, h: usize, seed: &[bool]) -> (Vec<u32>, Vec<f32>) {
    assert_eq!(seed.len(), w * h);
    if w == 0 || h == 0 {
        return (Vec::new(), Vec::new());
    }

    // パス1：列ごと（列優先で格納）
    let mut col_d = vec![0f64; w * h]; // [x*h + y]
    let mut col_y = vec![0u32; w * h];
    {
        let threads = thread_count(w);
        let cols_per = (w + threads - 1) / threads;
        super::par::scope(|sc| {
            for (ci, (dchunk, ychunk)) in col_d
                .chunks_mut(cols_per * h)
                .zip(col_y.chunks_mut(cols_per * h))
                .enumerate()
            {
                sc.spawn(move || {
                    let mut f = vec![0f64; h];
                    let mut v = vec![0usize; h];
                    let mut z = vec![0f64; h + 1];
                    let ncols = dchunk.len() / h;
                    for c in 0..ncols {
                        let x = ci * cols_per + c;
                        for y in 0..h {
                            f[y] = if seed[y * w + x] { 0.0 } else { INF };
                        }
                        let d = &mut dchunk[c * h..(c + 1) * h];
                        let a = &mut ychunk[c * h..(c + 1) * h];
                        dt1d(&f, d, Some(a), &mut v, &mut z);
                    }
                });
            }
        });
    }

    // パス2：行ごと
    let mut out_idx = vec![NO_SEED; w * h];
    let mut out_d2 = vec![0f32; w * h];
    {
        let threads = thread_count(h);
        let rows_per = (h + threads - 1) / threads;
        let col_d = &col_d;
        let col_y = &col_y;
        super::par::scope(|sc| {
            for (ri, (ichunk, dchunk)) in out_idx
                .chunks_mut(rows_per * w)
                .zip(out_d2.chunks_mut(rows_per * w))
                .enumerate()
            {
                sc.spawn(move || {
                    let mut f = vec![0f64; w];
                    let mut d = vec![0f64; w];
                    let mut a = vec![0u32; w];
                    let mut v = vec![0usize; w];
                    let mut z = vec![0f64; w + 1];
                    let nrows = ichunk.len() / w;
                    for r in 0..nrows {
                        let y = ri * rows_per + r;
                        for x in 0..w {
                            f[x] = col_d[x * h + y];
                        }
                        dt1d(&f, &mut d, Some(&mut a), &mut v, &mut z);
                        for x in 0..w {
                            let sx = a[x];
                            if sx == NO_SEED || d[x] >= INF {
                                ichunk[r * w + x] = NO_SEED;
                                dchunk[r * w + x] = f32::INFINITY;
                            } else {
                                let sy = col_y[sx as usize * h + y];
                                ichunk[r * w + x] = sy * w as u32 + sx;
                                dchunk[r * w + x] = d[x] as f32;
                            }
                        }
                    }
                });
            }
        });
    }
    (out_idx, out_d2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constrained_matches_plain_without_walls() {
        let (w, h) = (41usize, 29usize);
        let mut seed = vec![false; w * h];
        let mut s = 777u32;
        for i in 0..w * h {
            s = s.wrapping_mul(1103515245).wrapping_add(12345);
            seed[i] = (s >> 16) % 53 == 0;
        }
        let a = distance_sq(w, h, &seed);
        let b = distance_sq_constrained(w, h, |x, y| seed[y * w + x], |_, _| true);
        let mut worst = 0f32;
        for i in 0..w * h {
            worst = worst.max((a[i].sqrt() - b[i].sqrt()).abs());
        }
        // 位置を受け渡す方法なのでわずかな誤差はある
        assert!(worst < 0.5, "worst {worst}");
    }

    #[test]
    fn constrained_stops_at_walls() {
        // 左端の列がシード。x = 10 に縦の壁（y = 0 のところだけすき間）
        let (w, h) = (20usize, 15usize);
        let wall = |x: usize, y: usize| x == 10 && y != 0;
        let d = distance_sq_constrained(w, h, |x, _| x == 0, |x, y| !wall(x, y));
        // 壁のすぐ右は、すき間を回り込んだ分だけ遠い（直線距離 11 ではなく、すき間の先のシードまで）
        assert!(d[14 * w + 11].is_finite());
        assert!(
            d[14 * w + 11].sqrt() > 11.0 + 1.0,
            "{}",
            d[14 * w + 11].sqrt()
        );
        // 壁の上は届かない
        assert!(d[5 * w + 10].is_infinite());
        // 斜めの1px の線はすり抜けない
        let (w, h) = (12usize, 12usize);
        let diag = |x: usize, y: usize| x == y;
        let d = distance_sq_constrained(w, h, |x, y| x == 0 && y > 0, |x, y| !diag(x, y));
        for y in 0..h {
            for x in 0..w {
                if x > y {
                    assert!(d[y * w + x].is_infinite(), "leak at {x},{y}");
                }
            }
        }
    }

    #[test]
    fn matches_brute_force() {
        let (w, h) = (37usize, 23usize);
        let mut seed = vec![false; w * h];
        let mut s = 12345u32;
        for i in 0..w * h {
            s = s.wrapping_mul(1103515245).wrapping_add(12345);
            seed[i] = (s >> 16) % 29 == 0;
        }
        let (idx, d2) = nearest_seed(w, h, &seed);
        for y in 0..h {
            for x in 0..w {
                let mut best = f32::INFINITY;
                for sy in 0..h {
                    for sx in 0..w {
                        if seed[sy * w + sx] {
                            let dx = x as f32 - sx as f32;
                            let dy = y as f32 - sy as f32;
                            best = best.min(dx * dx + dy * dy);
                        }
                    }
                }
                let i = y * w + x;
                assert_eq!(d2[i], best, "at {x},{y}");
                let si = idx[i] as usize;
                let dx = x as f32 - (si % w) as f32;
                let dy = y as f32 - (si / w) as f32;
                assert_eq!(dx * dx + dy * dy, best);
            }
        }
    }

    #[test]
    fn distance_only_matches_nearest_seed() {
        let (w, h) = (41usize, 19usize);
        let mut seed = vec![false; w * h];
        let mut s = 777u32;
        for i in 0..w * h {
            s = s.wrapping_mul(1103515245).wrapping_add(12345);
            seed[i] = (s >> 16) % 31 == 0;
        }
        let (_, d2) = nearest_seed(w, h, &seed);
        assert_eq!(distance_sq(w, h, &seed), d2);
        assert!(distance_sq(5, 4, &[false; 20])
            .iter()
            .all(|d| d.is_infinite()));
    }

    #[test]
    fn no_seeds() {
        let (idx, _) = nearest_seed(4, 3, &[false; 12]);
        assert!(idx.iter().all(|&i| i == NO_SEED));
    }
}
