//! README と配布サイトの作例の画像を作る。
//!
//! cargo run --release -p fillerase --example samples [out_dir]
//! python3 fillerase/tools/make_sheets.py <out_dir> fillerase/docs/samples
//!
//! `out_dir` に `<シート名>__<番号>__<ラベル>.ppm` を書き出す。
//! 入力はブラウザのデモ（web/fillerase/site/app.js）と同じ VHS のラベル風の絵（アンチエイリアスなし）。
//! 消えたところは市松模様、点の位置は白い輪で示す。
use fillerase::engine::*;

const W: usize = 360;
const H: usize = 270;

fn hex(h: u32) -> [f32; 4] {
    [
        ((h >> 16) & 255) as f32 / 255.0,
        ((h >> 8) & 255) as f32 / 255.0,
        (h & 255) as f32 / 255.0,
        1.0,
    ]
}

/// ブラウザのデモの sampleScene と同じ絵
fn scene() -> Vec<[f32; 4]> {
    let cream = hex(0xf1e6cf);
    let navy = hex(0x1b1f4a);
    let rainbow = [0xf6b51d, 0xf07a1a, 0xe3204a, 0xb51c7d, 0x5b2a86].map(hex);
    let (w, h) = (W as f32, H as f32);
    let k = h / 360.0;
    let (nx, ny) = (0.42f32, 1.0f32);
    let nl = nx.hypot(ny);
    let (u0, bw) = (190.0 * k, 18.0 * k);
    let (cx, cy, r) = (w * 0.74, h * 0.4, h * 0.24);
    let bar_top = (h * 0.82).round() as usize;
    let line_h = ((6.0 * k).round() as usize).max(1);
    let mut v = vec![cream; W * H];
    for y in 0..H {
        for x in 0..W {
            let mut c = cream;
            let band = (((x as f32 * nx + y as f32 * ny) / nl - u0) / bw).floor();
            if band >= 0.0 && (band as usize) < rainbow.len() {
                c = rainbow[band as usize];
            }
            let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
            if dx * dx + dy * dy < r * r {
                c = rainbow[(((dy + r) / (2.0 * r) * 4.0).floor() as usize).min(3)];
            }
            if y >= bar_top {
                let line = (y - bar_top) / line_h;
                c = if line < rainbow.len() { rainbow[line] } else { navy };
            }
            v[y * W + x] = c;
        }
    }
    v
}

/// 結果を市松模様の上に重ね、点の位置に輪を描いて RGB にする
fn show(src: &[[f32; 4]], s: &Settings) -> Vec<[f32; 3]> {
    let amount = erase_amount(src, W, H, s);
    let mut out: Vec<[f32; 3]> = (0..W * H)
        .map(|i| {
            let (x, y) = (i % W, i / W);
            let bg = if (x / 10 + y / 10) % 2 == 0 { 0.93 } else { 0.8 };
            let a = src[i][3] * (1.0 - amount[i].clamp(0.0, 1.0));
            [0, 1, 2].map(|k| src[i][k] * a + bg * (1.0 - a))
        })
        .collect();
    for &(px, py) in &s.points {
        for y in 0..H {
            for x in 0..W {
                let d = ((x as f32 + 0.5 - px - 0.5).powi(2) + (y as f32 + 0.5 - py - 0.5).powi(2)).sqrt();
                if (5.0..7.0).contains(&d) {
                    out[y * W + x] = [1.0; 3];
                } else if (4.0..5.0).contains(&d) || (7.0..8.0).contains(&d) {
                    out[y * W + x] = [0.08; 3];
                }
            }
        }
    }
    out
}

fn save(path: &std::path::Path, px: &[[f32; 3]]) {
    let mut b = format!("P6\n{W} {H}\n255\n").into_bytes();
    for p in px {
        b.extend(p.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8));
    }
    std::fs::write(path, b).unwrap();
}

fn main() {
    let out = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "samples_out".into()),
    );
    std::fs::create_dir_all(&out).unwrap();
    let src = scene();
    let bg = (12.0, 12.0);
    // 太陽の黄色（帯の黄色とつながっている）
    let yellow = (W as f32 * 0.74, H as f32 * 0.4 - H as f32 * 0.2);
    let base = Settings::default();
    let tiles: Vec<(&str, Option<Settings>)> = vec![
        ("入力（7色・アンチエイリアスなし）", None),
        (
            "背景の点 1つ",
            Some(Settings {
                points: vec![bg],
                ..base.clone()
            }),
        ),
        (
            "太陽の黄色：つながった黄色だけ",
            Some(Settings {
                points: vec![yellow],
                ..base.clone()
            }),
        ),
        (
            "Contiguous オフ：黄色を全部",
            Some(Settings {
                points: vec![yellow],
                contiguous: false,
                ..base.clone()
            }),
        ),
        (
            "点 3つ",
            Some(Settings {
                points: vec![bg, (300.0, 250.0), (60.0, 222.0)],
                ..base.clone()
            }),
        ),
        (
            "背景 + Invert（範囲だけ残す）",
            Some(Settings {
                points: vec![bg],
                invert: true,
                ..base.clone()
            }),
        ),
    ];
    for (i, (label, s)) in tiles.iter().enumerate() {
        let px = match s {
            Some(s) => show(&src, s),
            None => src.iter().map(|p| [p[0], p[1], p[2]]).collect(),
        };
        let path = out.join(format!("00_overview__{i:02}__{label}.ppm"));
        save(&path, &px);
        println!("{}", path.display());
    }
}
