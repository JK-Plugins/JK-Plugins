//! 処理時間の目安を測る（cargo run --release -p fillerase --example bench [幅 高さ]）
use std::time::Instant;

use fillerase::engine::*;
use fillerase::frame;
use fillerase::world_io::{compose, encode, read_world, Changes, Layout, Rows, Source};

/// セル画っぽい絵：白い背景に、黒い線で囲まれた色の塗りの円がたくさん
fn cel(w: usize, h: usize) -> Vec<[f32; 4]> {
    let cols = [
        [0.9, 0.6, 0.5, 1.0],
        [0.4, 0.6, 0.9, 1.0],
        [0.95, 0.9, 0.5, 1.0],
    ];
    let mut v = vec![[1.0, 1.0, 1.0, 1.0]; w * h];
    let n = 40;
    for k in 0..n {
        let cx = ((k * 7919) % 97) as f32 / 97.0 * w as f32;
        let cy = ((k * 104729) % 89) as f32 / 89.0 * h as f32;
        let r = (w.min(h) as f32) * (0.04 + 0.08 * ((k * 31) % 7) as f32 / 7.0);
        let c = cols[k % 3];
        let (x0, x1) = (
            (cx - r - 2.0).max(0.0) as usize,
            ((cx + r + 2.0) as usize).min(w),
        );
        let (y0, y1) = (
            (cy - r - 2.0).max(0.0) as usize,
            ((cy + r + 2.0) as usize).min(h),
        );
        for y in y0..y1 {
            for x in x0..x1 {
                let d = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
                if d < r - 1.0 {
                    v[y * w + x] = c;
                } else if d < r + 1.0 {
                    v[y * w + x] = [0.0, 0.0, 0.0, 1.0];
                }
            }
        }
    }
    v
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

fn main() {
    let args: Vec<usize> = std::env::args()
        .skip(1)
        .filter_map(|a| a.parse().ok())
        .collect();
    let (w, h) = (
        *args.first().unwrap_or(&1920),
        *args.get(1).unwrap_or(&1080),
    );
    println!("{w}×{h}");
    let src = cel(w, h);

    // 8bit のワールドを作る
    let layout = Layout::Argb8;
    let bpp = layout.bytes_per_pixel();
    let rows = Rows {
        stride: w * bpp,
        flip: false,
        h,
    };
    let mut buf = vec![0u8; w * h * bpp];
    for (p, b) in src.iter().zip(buf.chunks_exact_mut(bpp)) {
        encode(layout, *p, b);
    }

    let t = Instant::now();
    let _ = read_world(&buf, rows, w, layout);
    println!("read_world        {:>8.1} ms", ms(t));

    let one = vec![(1.0, 1.0)];
    let many: Vec<(f32, f32)> = (0..99)
        .map(|i| {
            (
                ((i * 37) % 100) as f32 / 100.0 * w as f32,
                ((i * 61) % 100) as f32 / 100.0 * h as f32,
            )
        })
        .collect();
    // 背景（白）の点：大きな範囲を塗る
    let bg = (0..w * h)
        .find(|&i| src[i] == [1.0, 1.0, 1.0, 1.0])
        .unwrap();
    let bg = vec![((bg % w) as f32, (bg / w) as f32)];
    let cases: Vec<(&str, Settings)> = vec![
        (
            "background",
            Settings {
                points: bg.clone(),
                ..Default::default()
            },
        ),
        (
            "background choke+10",
            Settings {
                points: bg.clone(),
                choke: 10.0,
                ..Default::default()
            },
        ),
        (
            "1 point",
            Settings {
                points: one.clone(),
                ..Default::default()
            },
        ),
        (
            "99 points",
            Settings {
                points: many.clone(),
                ..Default::default()
            },
        ),
        (
            "1 point choke +10",
            Settings {
                points: one.clone(),
                choke: 10.0,
                ..Default::default()
            },
        ),
        (
            "1 point choke -10",
            Settings {
                points: one.clone(),
                choke: -10.0,
                ..Default::default()
            },
        ),
        (
            "1 point softness",
            Settings {
                points: one.clone(),
                softness: 0.2,
                ..Default::default()
            },
        ),
        (
            "non-contiguous 1",
            Settings {
                points: one.clone(),
                contiguous: false,
                ..Default::default()
            },
        ),
        (
            "non-contiguous 99",
            Settings {
                points: many.clone(),
                contiguous: false,
                ..Default::default()
            },
        ),
    ];
    let src_w = Source {
        buf: &buf,
        rows,
        w,
        layout,
    };
    println!(
        "{:<20} {:>9} {:>9} {:>9}   erased",
        "", "old", "new", "held"
    );
    for (name, s) in &cases {
        let mut out = vec![0u8; w * h * bpp];
        // 前のやり方：全体を float にして、消す量を作り、書き戻す（2回目を測る）
        let old = || {
            let f = read_world(&buf, rows, w, layout);
            let amount = erase_amount(&f, w, h, s);
            let mut out = vec![0u8; w * h * bpp];
            compose(
                &src_w,
                &mut out,
                rows,
                w,
                layout,
                0,
                0,
                Changes::Erase {
                    amount: &amount,
                    invert: false,
                },
            );
            amount
        };
        let _ = old();
        let t = Instant::now();
        let amount = old();
        let t_old = ms(t);

        // 今のやり方：入力のバイト列から直接（キャッシュなし＝絵が変わったフレーム）
        frame::clear_cache();
        frame::render(&src_w, &mut out, rows, w, layout, 0, 0, s);
        frame::clear_cache();
        let t = Instant::now();
        frame::render(&src_w, &mut out, rows, w, layout, 0, 0, s);
        let t_new = ms(t);
        // 同じ絵が続くフレーム（キャッシュが効く）
        let t = Instant::now();
        frame::render(&src_w, &mut out, rows, w, layout, 0, 0, s);
        let t_held = ms(t);

        let erased = amount.iter().filter(|&&v| v > 0.0).count() as f64 / (w * h) as f64 * 100.0;
        println!("{name:<20} {t_old:>6.1} ms {t_new:>6.1} ms {t_held:>6.1} ms   {erased:>5.1}%");
    }
}
