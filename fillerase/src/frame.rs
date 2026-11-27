//! 1フレームの処理（AE に依存しないので、テストと計測ができる）。
//!
//! 入力のバイト列から必要な画素だけ読んで消す範囲を求め、出力にアルファだけ書き戻す。
//! 消す範囲（マスク）は入力の画素と設定から作ったキーでキャッシュする。
//! アニメの2コマ・3コマ打ちのように同じ絵が続くフレームや、Invert だけ変えたときは塗りつぶしを飛ばせる。

use std::sync::{Arc, Mutex};

use crate::engine::{self, Settings};
use crate::world_io::{compose, Changes, Layout, Rows, Source};

/// マスクのキャッシュ（新しい順）。複数フレームを同時にレンダリングするので Mutex で守る
static CACHE: Mutex<Vec<(u64, Arc<Vec<f32>>)>> = Mutex::new(Vec::new());
const CACHE_ENTRIES: usize = 8;
const CACHE_MAX_BYTES: usize = 256 << 20;

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

/// 入力の画素（各行の、使うバイトだけ）のハッシュ。行で分けて並列に
fn hash_pixels(src: &Source) -> u64 {
    let (w, h) = (src.w, src.rows.h);
    let n = w * src.layout.bytes_per_pixel();
    let per = crate::world_io::thread_rows(h);
    let parts: Vec<u64> = std::thread::scope(|sc| {
        let hs: Vec<_> = (0..h)
            .step_by(per.max(1))
            .map(|y0| {
                sc.spawn(move || {
                    let mut k = KeyHasher(0x2452_821e_638d_0137);
                    for y in y0..(y0 + per).min(h) {
                        let row = &src.buf[src.rows.start(y)..src.rows.start(y) + n];
                        let mut it = row.chunks_exact(8);
                        for c in &mut it {
                            k.add(u64::from_le_bytes(c.try_into().unwrap()));
                        }
                        let rem = it.remainder();
                        let mut last = [0u8; 8];
                        last[..rem.len()].copy_from_slice(rem);
                        k.add(u64::from_le_bytes(last) ^ ((rem.len() as u64) << 59));
                    }
                    k.0
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let mut k = KeyHasher(0xbe54_66cf_34e9_0c6c);
    for v in parts {
        k.add(v);
    }
    k.0
}

/// マスクを決める値（入力の画素・大きさ・並びと、Invert 以外の設定）から作るキー
fn mask_key(src: &Source, s: &Settings) -> u64 {
    let mut k = KeyHasher(0x243f_6a88_85a3_08d3);
    k.add(hash_pixels(src));
    k.add(src.w as u64);
    k.add(src.rows.h as u64);
    k.add(src.layout as u64);
    k.add(s.points.len() as u64);
    for &(x, y) in &s.points {
        k.add_f32(x);
        k.add_f32(y);
    }
    k.add_f32(s.tolerance);
    k.add_f32(s.softness);
    k.add_f32(s.choke);
    k.add(s.contiguous as u64 | (s.compare_alpha as u64) << 1);
    k.0
}

/// キャッシュにあればそれを、なければ求めて入れる
fn mask(src: &Source, s: &Settings) -> Arc<Vec<f32>> {
    let key = mask_key(src, s);
    if let Ok(mut cache) = CACHE.lock() {
        if let Some(pos) = cache.iter().position(|(k, _)| *k == key) {
            let hit = cache.remove(pos);
            let m = hit.1.clone();
            cache.insert(0, hit);
            return m;
        }
    }
    // 計算中はロックを持たない（他のフレームを止めない）
    let (w, h) = (src.w, src.rows.h);
    let m = Arc::new(crate::with_fast_source!(src, |p| engine::erase_mask_with(
        p, w, h, s
    )));
    if let Ok(mut cache) = CACHE.lock() {
        cache.retain(|(k, _)| *k != key);
        cache.insert(0, (key, m.clone()));
        let mut bytes = 0;
        let mut keep = 0;
        for (_, e) in cache.iter() {
            bytes += e.len() * 4;
            if keep >= CACHE_ENTRIES || (keep > 0 && bytes > CACHE_MAX_BYTES) {
                break;
            }
            keep += 1;
        }
        cache.truncate(keep);
    }
    m
}

/// キャッシュを空にする（計測用）
pub fn clear_cache() {
    if let Ok(mut c) = CACHE.lock() {
        c.clear();
    }
}

/// 1フレームを処理する。(dx, dy) は出力の原点から見た入力の原点のずれ
#[allow(clippy::too_many_arguments)]
pub fn render(
    input: &Source,
    out_buf: &mut [u8],
    out_rows: Rows,
    ow: usize,
    out_layout: Layout,
    dx: i32,
    dy: i32,
    s: &Settings,
) {
    let empty = input.w == 0 || input.rows.h == 0;
    // 点が1つも入力の中になければ、入力をそのまま写すだけ
    let m = if empty || engine::seeds(input, input.w, input.rows.h, s).is_empty() {
        None
    } else {
        Some(mask(input, s))
    };
    let changes = match &m {
        Some(m) => Changes::Erase {
            amount: m,
            invert: s.invert,
        },
        // Invert で点がないときは全部消す
        None if s.invert => {
            out_buf.fill(0);
            return;
        }
        None => Changes::List(&[]),
    };
    compose(input, out_buf, out_rows, ow, out_layout, dx, dy, changes);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world_io::{encode, read_world};

    fn world(
        w: usize,
        h: usize,
        layout: Layout,
        px: impl Fn(usize, usize) -> [f32; 4],
    ) -> (Vec<u8>, Rows) {
        let bpp = layout.bytes_per_pixel();
        let rows = Rows {
            stride: w * bpp + 4,
            flip: true,
            h,
        };
        let mut buf = vec![0u8; rows.stride * h];
        for y in 0..h {
            for x in 0..w {
                let s = rows.start(y) + x * bpp;
                encode(layout, px(x, y), &mut buf[s..s + bpp]);
            }
        }
        (buf, rows)
    }

    fn picture(x: usize, _y: usize) -> [f32; 4] {
        if x == 4 {
            [0.0, 0.0, 0.0, 1.0]
        } else {
            [0.2 + 0.01 * (x % 3) as f32, 0.6, 0.9, 1.0]
        }
    }

    /// float に直してから処理した結果と、バイト列のまま処理した結果が同じになる
    #[test]
    fn matches_float_path() {
        for layout in [
            Layout::Argb8,
            Layout::Argb16,
            Layout::ArgbF32,
            Layout::Bgra8,
            Layout::Bgra16,
        ] {
            for invert in [false, true] {
                let (w, h) = (9, 5);
                let (buf, rows) = world(w, h, layout, picture);
                let src = Source {
                    buf: &buf,
                    rows,
                    w,
                    layout,
                };
                let s = Settings {
                    points: vec![(1.0, 2.0)],
                    tolerance: 0.05,
                    softness: 0.1,
                    choke: 0.5,
                    invert,
                    ..Default::default()
                };
                let mut a = vec![0u8; buf.len()];
                render(&src, &mut a, rows, w, layout, 0, 0, &s);

                let f = read_world(&buf, rows, w, layout);
                let amount = engine::erase_amount(&f, w, h, &s);
                let want: Vec<[f32; 4]> = f
                    .iter()
                    .zip(&amount)
                    .map(|(p, m)| [p[0], p[1], p[2], p[3] * (1.0 - m.clamp(0.0, 1.0))])
                    .collect();
                let mut b = vec![0u8; buf.len()];
                let bpp = layout.bytes_per_pixel();
                for y in 0..h {
                    for x in 0..w {
                        let o = rows.start(y) + x * bpp;
                        encode(layout, want[y * w + x], &mut b[o..o + bpp]);
                    }
                }
                assert_eq!(
                    read_world(&a, rows, w, layout),
                    read_world(&b, rows, w, layout),
                    "{layout:?} invert={invert}"
                );
            }
        }
    }

    #[test]
    fn cached_result_is_same_and_invert_uses_cache() {
        let layout = Layout::Argb8;
        let (w, h) = (9, 5);
        let (buf, rows) = world(w, h, layout, picture);
        let src = Source {
            buf: &buf,
            rows,
            w,
            layout,
        };
        let s = Settings {
            points: vec![(0.0, 0.0)],
            ..Default::default()
        };
        let run = |s: &Settings| {
            let mut o = vec![0u8; buf.len()];
            render(&src, &mut o, rows, w, layout, 0, 0, s);
            o
        };
        let first = run(&s);
        let key = mask_key(&src, &s);
        assert!(CACHE.lock().unwrap().iter().any(|(k, _)| *k == key));
        assert_eq!(run(&s), first);
        // Invert だけ変えても同じキー（塗りつぶしをやり直さない）
        assert_eq!(
            mask_key(
                &src,
                &Settings {
                    invert: true,
                    ..s.clone()
                }
            ),
            key
        );
        // 画素が変わればキーも変わる
        let (buf2, _) = world(w, h, layout, |x, y| {
            if (x, y) == (8, 4) {
                [1.0; 4]
            } else {
                picture(x, y)
            }
        });
        let src2 = Source {
            buf: &buf2,
            rows,
            w,
            layout,
        };
        assert_ne!(mask_key(&src2, &s), key);
    }

    #[test]
    fn no_point_inside_copies_input_or_clears_when_inverted() {
        let layout = Layout::Argb8;
        let (w, h) = (6, 4);
        let (buf, rows) = world(w, h, layout, picture);
        let src = Source {
            buf: &buf,
            rows,
            w,
            layout,
        };
        let s = Settings {
            points: vec![(-5.0, 1.0)],
            ..Default::default()
        };
        let mut o = vec![9u8; buf.len()];
        render(&src, &mut o, rows, w, layout, 0, 0, &s);
        assert_eq!(
            read_world(&o, rows, w, layout),
            read_world(&buf, rows, w, layout)
        );
        render(
            &src,
            &mut o,
            rows,
            w,
            layout,
            0,
            0,
            &Settings { invert: true, ..s },
        );
        assert!(o.iter().all(|&b| b == 0));
    }
}
