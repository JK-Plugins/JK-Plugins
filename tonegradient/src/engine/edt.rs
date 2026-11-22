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
        std::thread::scope(|sc| {
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
        std::thread::scope(|sc| {
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
        std::thread::scope(|sc| {
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
        std::thread::scope(|sc| {
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
