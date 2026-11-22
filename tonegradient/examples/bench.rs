//! 処理時間の目安を測る（cargo run --release --example bench）
use std::time::Instant;
use tonegradient::engine::*;

fn main() {
    let width: f32 = std::env::args()
        .nth(1)
        .and_then(|v| v.parse().ok())
        .unwrap_or(60.0);
    let (w, h) = (1920usize, 1080usize);
    let cols = [
        [1.0, 1.0, 1.0],
        [0.75, 0.75, 0.75],
        [0.5, 0.5, 0.5],
        [0.25, 0.25, 0.25],
    ];
    let src: Vec<[f32; 4]> = (0..w * h)
        .map(|i| {
            let (x, y) = ((i % w) as f32, (i / w) as f32);
            let v = ((x / 97.0).sin() + (y / 61.0).cos() + 2.0) / 4.0;
            let c = cols[((v * 4.0) as usize).min(3)];
            [c[0], c[1], c[2], 1.0]
        })
        .collect();
    let base = || Settings {
        colors: cols.iter().map(|&c| GroupColor::new(c)).collect(),
        width,
        ..Default::default()
    };
    let t = Instant::now();
    let key = prepare_key(w, h, &src, &base());
    println!(
        "prepare_key {:>7.1} ms ({key:x})",
        t.elapsed().as_secs_f64() * 1e3
    );
    let t = Instant::now();
    let p = prepare(w, h, &src, &base());
    println!(
        "prepare     {:>7.1} ms ({} MB)",
        t.elapsed().as_secs_f64() * 1e3,
        p.memory_bytes() >> 20
    );
    let cases: Vec<(&str, Settings, TextureParams)> = vec![
        ("smooth", base(), TextureParams::default()),
        (
            "dot",
            base(),
            TextureParams {
                kind: TextureKind::Dot,
                ..Default::default()
            },
        ),
        (
            "dot aa4",
            Settings {
                aa_samples: 4,
                ..base()
            },
            TextureParams {
                kind: TextureKind::Dot,
                ..Default::default()
            },
        ),
        (
            "noise",
            base(),
            TextureParams {
                kind: TextureKind::Noise,
                cell: 3.0,
                octaves: 4,
                ..Default::default()
            },
        ),
        (
            "noise aa4",
            Settings {
                aa_samples: 4,
                ..base()
            },
            TextureParams {
                kind: TextureKind::Noise,
                cell: 3.0,
                octaves: 4,
                ..Default::default()
            },
        ),
        (
            "noise soft",
            Settings {
                softness: 0.7,
                ..base()
            },
            TextureParams {
                kind: TextureKind::Noise,
                cell: 3.0,
                octaves: 4,
                ..Default::default()
            },
        ),
        (
            "dot rough",
            Settings {
                roughness: 8.0,
                ..base()
            },
            TextureParams {
                kind: TextureKind::Dot,
                ..Default::default()
            },
        ),
        (
            "dot mid",
            Settings {
                mid_amount: 0.5,
                ..base()
            },
            TextureParams {
                kind: TextureKind::Dot,
                ..Default::default()
            },
        ),
    ];
    for (ci, (name, s, tp)) in cases.into_iter().enumerate() {
        // ケースごとにキーを変えて、毎回1回目に準備の結果を送る時間を測る
        let key = key ^ (ci as u64 + 1);
        let tex = Texture::new(tp, None);
        let _ = shade(&p, &src, &s, &tex);
        let t = Instant::now();
        let n = 5;
        let mut changed = 0;
        for _ in 0..n {
            changed = shade(&p, &src, &s, &tex).changed.len();
        }
        let cpu_ms = t.elapsed().as_secs_f64() * 1e3 / n as f64;
        // GPU：1回目は距離変換から（入力が変わったフレーム）、2回目以降は使い回す
        let gpu = gpu::render_timed(w, h, &src, key, &s, &tex).map(|(_, t)| t);
        let gpu_avg = (0..n)
            .map(|_| gpu::render_timed(w, h, &src, key, &s, &tex).map(|(_, t)| t))
            .collect::<Result<Vec<_>, _>>();
        match (gpu, gpu_avg) {
            (Ok(first), Ok(v)) => {
                let run = v.iter().map(|t| t.run).sum::<f64>() / n as f64;
                let unpack = v.iter().map(|t| t.unpack).sum::<f64>() / n as f64;
                println!(
                    "shade {:<11} CPU {:>6.1} ms | GPU {:>5.1} ms (計算+転送 {run:.1} + 展開 {unpack:.1}) | 入力が変わったとき GPU 全体 {:.1} ms（ラベル {:.1}）(changed {changed})",
                    name,
                    cpu_ms,
                    run + unpack,
                    first.labels + first.run + first.unpack,
                    first.labels
                );
            }
            (Err(e), _) | (_, Err(e)) => {
                println!("shade {:<11} CPU {:>6.1} ms | GPU なし: {e}", name, cpu_ms);
            }
        }
    }
}
