//! パラメータごとの効果を見比べるサンプル画像を書き出す。
//!
//! ```sh
//! cargo run --release --example samples -- out_dir
//! python3 tools/make_sheets.py out_dir docs/samples
//! ```
//!
//! `out_dir` に `<シート名>__<番号>__<ラベル>.ppm` を書き出す。

use std::io::Write;
use tonegradient::engine::*;

const W: usize = 360;
const H: usize = 270;

fn save(path: &std::path::Path, w: usize, h: usize, px: &[[f32; 4]]) {
    let mut f = std::fs::File::create(path).unwrap();
    write!(f, "P6\n{w} {h}\n255\n").unwrap();
    let buf: Vec<u8> = px
        .iter()
        .flat_map(|p| [p[0], p[1], p[2]].map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8))
        .collect();
    f.write_all(&buf).unwrap();
}

pub const HI: [f32; 3] = [1.0, 0.95, 0.88];
pub const BASE: [f32; 3] = [0.96, 0.80, 0.70];
pub const SHADOW: [f32; 3] = [0.80, 0.58, 0.55];
pub const SHADOW2: [f32; 3] = [0.62, 0.42, 0.48];
const BG: [f32; 3] = [0.55, 0.70, 0.85];
const LINE: [f32; 3] = [0.1, 0.08, 0.1];

/// 球っぽい塗り分け：ハイライト・ベース・影・2影＋線と背景。`aa` で塗りの境界にアンチエイリアス
pub fn scene(aa: bool) -> Vec<[f32; 4]> {
    let mut px = vec![[0.0; 4]; W * H];
    let (cx, cy, r) = (W as f32 * 0.5, H as f32 * 0.5, H as f32 * 0.42);
    let sub = if aa { 4 } else { 1 };
    for y in 0..H {
        for x in 0..W {
            let mut acc = [0.0f32; 3];
            for sy in 0..sub {
                for sx in 0..sub {
                    let fx = x as f32 + (sx as f32 + 0.5) / sub as f32;
                    let fy = y as f32 + (sy as f32 + 0.5) / sub as f32;
                    let d = ((fx - cx).powi(2) + (fy - cy).powi(2)).sqrt();
                    let c = if (d - r).abs() < 2.0 {
                        LINE
                    } else if d > r {
                        BG
                    } else if ((fx - cx + r * 0.35).powi(2) + (fy - cy + r * 0.4).powi(2)).sqrt()
                        < r * 0.22
                    {
                        HI
                    } else if ((fx - cx - r * 0.35).powi(2) + (fy - cy - r * 0.2).powi(2)).sqrt()
                        < r * 0.45
                    {
                        SHADOW2
                    } else if fx - cx + (fy - cy) * 0.4 > -r * 0.05 {
                        SHADOW
                    } else {
                        BASE
                    };
                    for i in 0..3 {
                        acc[i] += c[i];
                    }
                }
            }
            let n = (sub * sub) as f32;
            px[y * W + x] = [acc[0] / n, acc[1] / n, acc[2] / n, 1.0];
        }
    }
    px
}

/// 紙目っぽいテクスチャレイヤー（Layer 用）
fn paper(w: usize, h: usize) -> Vec<f32> {
    let mut v = vec![0.0; w * h];
    for y in 0..h {
        for x in 0..w {
            let fx = x as f32;
            let fy = y as f32;
            let weave = 0.5
                + 0.25 * (fx * 0.9).sin() * (fy * 0.35).cos()
                + 0.25 * (fy * 0.9 + fx * 0.2).sin();
            let h1 = (texture_hash(x as i32, y as i32) as f32) / u32::MAX as f32;
            v[y * w + x] = (weave * 0.7 + h1 * 0.3).clamp(0.0, 1.0);
        }
    }
    v
}

fn texture_hash(x: i32, y: i32) -> u32 {
    tonegradient::engine::texture::hash2(x, y, 77)
}

fn colors() -> Vec<GroupColor> {
    [HI, BASE, SHADOW, SHADOW2]
        .iter()
        .map(|&c| GroupColor::new(c))
        .collect()
}

fn base_settings() -> Settings {
    Settings {
        colors: colors(),
        width: 40.0,
        ..Default::default()
    }
}

fn dot() -> TextureParams {
    TextureParams {
        kind: TextureKind::Dot,
        cell: 8.0,
        angle_deg: 45.0,
        ..Default::default()
    }
}

fn smooth() -> TextureParams {
    TextureParams::default()
}

struct Case {
    sheet: &'static str,
    label: String,
    s: Settings,
    t: TextureParams,
    aa_scene: bool,
    use_layer: bool,
    /// 拡大表示する範囲 (x, y, w, h)
    zoom: Option<(usize, usize, usize, usize)>,
}

fn case(sheet: &'static str, label: &str, s: Settings, t: TextureParams) -> Case {
    Case {
        sheet,
        label: label.into(),
        s,
        t,
        aa_scene: false,
        use_layer: false,
        zoom: None,
    }
}

fn cases() -> Vec<Case> {
    let mut v = Vec::new();
    let b = base_settings;

    // 入力と確認表示
    v.push(case(
        "00_overview",
        "入力（4色＋線）",
        Settings {
            colors: Vec::new(),
            ..b()
        },
        smooth(),
    ));
    v.push(case(
        "00_overview",
        "Output: Gradient (check)",
        Settings {
            output: OutputMode::Gradient,
            ..b()
        },
        smooth(),
    ));
    v.push(case("00_overview", "Texture: Smooth", b(), smooth()));
    v.push(case("00_overview", "Texture: Dot", b(), dot()));
    v.push(case(
        "00_overview",
        "Dot + Softness 2px",
        Settings {
            softness: 2.0,
            ..b()
        },
        TextureParams {
            kind: TextureKind::Dot,
            cell: 10.0,
            ..Default::default()
        },
    ));

    // Width
    for wd in [10.0, 40.0, 80.0] {
        v.push(case(
            "01_width",
            &format!("Width {wd}"),
            Settings { width: wd, ..b() },
            smooth(),
        ));
    }

    // Curve
    v.push(case("02_curve", "Curve: Linear", b(), smooth()));
    v.push(case(
        "02_curve",
        "Curve: Smooth",
        Settings {
            curve: Curve::Smooth,
            ..b()
        },
        smooth(),
    ));
    v.push(case(
        "02_curve",
        "Curve Power 0.5",
        Settings {
            curve_power: 0.5,
            ..b()
        },
        smooth(),
    ));
    v.push(case(
        "02_curve",
        "Curve Power 3",
        Settings {
            curve_power: 3.0,
            ..b()
        },
        smooth(),
    ));

    // Spread（色ごと）
    for sp in [1.0, 0.3, 3.0] {
        let mut s = b();
        s.colors[2].spread = sp; // 影
        v.push(case("03_spread", &format!("影の Spread {sp}"), s, dot()));
    }

    // Grow（色ごと）
    for g in [0.0, 12.0, -12.0] {
        let mut s = b();
        s.colors[2].grow = g;
        v.push(case("04_grow", &format!("影の Grow {g:+}"), s, smooth()));
    }

    // Offset / Reverse
    v.push(case("05_offset_reverse", "Offset 0", b(), dot()));
    v.push(case(
        "05_offset_reverse",
        "Offset +8",
        Settings { offset: 8.0, ..b() },
        dot(),
    ));
    v.push(case(
        "05_offset_reverse",
        "Offset -8",
        Settings {
            offset: -8.0,
            ..b()
        },
        dot(),
    ));
    v.push(case(
        "05_offset_reverse",
        "Reverse Order",
        Settings {
            reverse: true,
            ..b()
        },
        dot(),
    ));

    // Mid Color（3つ目の色としてグラデーションに入る。テクスチャも同じようにかかる）
    let mid = |amount: f32, width: f32, pos: MidPosition| Settings {
        mid_color: [1.0, 0.55, 0.45],
        mid_amount: amount,
        mid_width: width,
        mid_position: pos,
        ..b()
    };
    v.push(case(
        "07_mid_color",
        "Mid Amount 0%",
        mid(0.0, 0.5, MidPosition::Core),
        smooth(),
    ));
    v.push(case(
        "07_mid_color",
        "Mid 100% / Width 50%",
        mid(1.0, 0.5, MidPosition::Core),
        smooth(),
    ));
    v.push(case(
        "07_mid_color",
        "Mid 100% / Width 100%",
        mid(1.0, 1.0, MidPosition::Core),
        smooth(),
    ));
    let mut c = case(
        "07_mid_color",
        "Dot (Map) / Mid 60%",
        // Mid Color は Map（か Smooth）のときだけ使う
        Settings {
            texture_mode: TextureMode::Map,
            ..mid(0.6, 0.5, MidPosition::Core)
        },
        dot(),
    );
    c.zoom = Some((120, 60, 120, 120));
    v.push(c);

    // Texture 種類
    for (name, kind) in [
        ("Smooth", TextureKind::Smooth),
        ("Dot", TextureKind::Dot),
        ("Line", TextureKind::Line),
        ("Bayer", TextureKind::Bayer),
        ("Grain", TextureKind::Grain),
        ("Noise", TextureKind::Noise),
    ] {
        let t = TextureParams {
            kind,
            cell: if kind == TextureKind::Noise { 6.0 } else { 8.0 },
            dither_size: 2.0,
            ..dot()
        };
        v.push(case("08_texture", name, b(), t));
    }
    let mut c = case(
        "08_texture",
        "Layer（紙目）",
        b(),
        TextureParams {
            kind: TextureKind::Layer,
            ..dot()
        },
    );
    c.use_layer = true;
    v.push(c);

    // Amount
    for a in [0.0, 0.5, 1.0] {
        v.push(case(
            "09_amount",
            &format!("Amount {}%", (a * 100.0) as i32),
            Settings { amount: a, ..b() },
            dot(),
        ));
    }

    // Dot Shape
    let z = Some((180, 40, 120, 120));
    for (name, shape) in [
        ("Round", DotShape::Round),
        ("Circle", DotShape::Circle),
        ("Ellipse (Aspect 2)", DotShape::Ellipse),
        ("Square", DotShape::Square),
        ("Diamond", DotShape::Diamond),
    ] {
        let mut c = case(
            "10_dot_shape",
            name,
            b(),
            TextureParams {
                dot_shape: shape,
                cell: 12.0,
                angle_deg: 0.0,
                ..dot()
            },
        );
        c.zoom = z;
        v.push(c);
    }

    // Cell Size / Angle / Offset
    for (lab, cell, ang, off) in [
        ("Cell 6 / Angle 45", 6.0, 45.0, (0.0, 0.0)),
        ("Cell 14 / Angle 45", 14.0, 45.0, (0.0, 0.0)),
        ("Cell 14 / Angle 15", 14.0, 15.0, (0.0, 0.0)),
        ("Cell 14 / Offset X 7", 14.0, 45.0, (7.0, 0.0)),
    ] {
        let mut c = case(
            "11_cell_angle_offset",
            lab,
            b(),
            TextureParams {
                cell,
                angle_deg: ang,
                offset: off,
                ..dot()
            },
        );
        c.zoom = z;
        v.push(c);
    }

    // Texture Size（Bayer の1粒は Size ÷ 8 px）
    for d in [1.0, 2.0, 4.0] {
        let mut c = case(
            "12_dither_size",
            &format!("Bayer / Texture Size {}", d * 8.0),
            b(),
            TextureParams {
                kind: TextureKind::Bayer,
                dither_size: d,
                angle_deg: 0.0,
                ..dot()
            },
        );
        c.zoom = z;
        v.push(c);
    }

    // Noise Detail
    for (lab, oct, con) in [
        ("Detail 1", 1, 2.2),
        ("Detail 3", 3, 2.2),
        ("Detail 6", 6, 2.2),
    ] {
        v.push(case(
            "13_noise",
            lab,
            b(),
            TextureParams {
                kind: TextureKind::Noise,
                cell: 6.0,
                octaves: oct,
                noise_contrast: con,
                ..dot()
            },
        ));
    }

    // Layer Scale / Levels / Invert
    for (lab, sc, bl, wh, inv) in [
        ("Texture Size 8", 1.0, 0.0, 1.0, false),
        ("Texture Size 24", 3.0, 0.0, 1.0, false),
        ("Invert", 1.0, 0.0, 1.0, true),
    ] {
        let mut c = case(
            "14_layer",
            lab,
            b(),
            TextureParams {
                kind: TextureKind::Layer,
                layer_scale: sc,
                layer_black: bl,
                layer_white: wh,
                invert: inv,
                ..dot()
            },
        );
        c.use_layer = true;
        c.zoom = Some((150, 40, 150, 150));
        v.push(c);
    }

    // Antialias
    let az = Some((196, 86, 48, 48));
    for (lab, n) in [("Antialias Off", 1u32), ("2x2", 2), ("4x4", 4), ("8x8", 8)] {
        let mut c = case(
            "15_antialias",
            lab,
            Settings {
                aa_samples: n,
                ..b()
            },
            TextureParams {
                cell: 10.0,
                ..dot()
            },
        );
        c.zoom = az;
        v.push(c);
    }

    // Softness（0% で網点のようにくっきり、上げるほどなめらかなグラデーション＋テクスチャのムラ）
    let noise = TextureParams {
        kind: TextureKind::Noise,
        cell: 3.0,
        octaves: 4,
        ..Default::default()
    };
    for sf in [0.0f32, 1.5, 4.0] {
        let mut c = case(
            "18_softness",
            &format!("Noise / Softness {sf}px"),
            Settings {
                softness: sf,
                ..b()
            },
            noise,
        );
        c.zoom = Some((120, 60, 120, 120));
        v.push(c);
    }
    let mut c = case(
        "18_softness",
        "Dot / Softness 3px",
        Settings {
            softness: 3.0,
            ..b()
        },
        TextureParams {
            kind: TextureKind::Dot,
            cell: 12.0,
            ..Default::default()
        },
    );
    c.zoom = Some((120, 60, 120, 120));
    v.push(c);

    // Texture Mode：Threshold（2値）と Map（濃淡で連続的に混ぜる）
    for (lab, kind, mode) in [
        (
            "Noise / Threshold",
            TextureKind::Noise,
            TextureMode::Threshold,
        ),
        ("Noise / Map", TextureKind::Noise, TextureMode::Map),
        (
            "Layer / Threshold",
            TextureKind::Layer,
            TextureMode::Threshold,
        ),
        ("Layer / Map", TextureKind::Layer, TextureMode::Map),
    ] {
        let mut c = case(
            "20_texture_mode",
            lab,
            Settings {
                texture_mode: mode,
                ..b()
            },
            TextureParams {
                kind,
                cell: 8.0,
                octaves: 4,
                ..dot()
            },
        );
        c.use_layer = kind == TextureKind::Layer;
        c.zoom = Some((120, 60, 120, 120));
        v.push(c);
    }

    // Roughness
    for (lab, r, sc) in [
        ("Roughness 0", 0.0, 20.0),
        ("Amount 8 / Scale 20", 8.0, 20.0),
        ("Amount 8 / Scale 60", 8.0, 60.0),
        ("Amount 20 / Scale 20", 20.0, 20.0),
    ] {
        v.push(case(
            "16_roughness",
            lab,
            Settings {
                roughness: r,
                rough_scale: sc,
                width: 16.0,
                ..b()
            },
            smooth(),
        ));
    }

    // Tolerance / Alpha Threshold（アンチエイリアスの入った素材）
    let tz = Some((40, 20, 140, 140));
    for (lab, tol) in [
        ("AA素材 / Tolerance 0", 0.0),
        ("Tolerance 40", 40.0 / 255.0),
    ] {
        let mut c = case(
            "17_tolerance",
            lab,
            Settings {
                tolerance: tol,
                width: 24.0,
                ..b()
            },
            smooth(),
        );
        c.aa_scene = true;
        c.zoom = tz;
        v.push(c);
    }
    {
        let mut s = Settings { width: 24.0, ..b() };
        s.colors[0].tolerance = 60.0 / 255.0;
        let mut c = case("17_tolerance", "ハイライトだけ Tolerance 60", s, smooth());
        c.aa_scene = true;
        c.zoom = tz;
        v.push(c);
    }
    v
}

fn main() {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "samples_out".into());
    let dir = std::path::Path::new(&dir);
    std::fs::create_dir_all(dir).unwrap();
    let plain = scene(false);
    let aa = scene(true);
    let pap = paper(W, H);
    let mut counter = std::collections::HashMap::new();
    for c in cases() {
        let src = if c.aa_scene { &aa } else { &plain };
        let layer = if c.use_layer {
            Some(LayerTexture {
                width: W,
                height: H,
                value: &pap,
                origin: (0, 0),
                // 紙の画像をレイヤーと同じ位置に置く（Texture Size 8 で 1倍）
                xform: {
                    let k = 1.0 / c.t.layer_scale.max(0.01);
                    [k, 0.0, -0.5, 0.0, k, -0.5]
                },
            })
        } else {
            None
        };
        let tex = Texture::new(c.t, layer);
        let img: Vec<[f32; 4]> = process(W, H, src, &c.s, &tex).to_dense(src).rgba;
        let (img, w, h) = match c.zoom {
            Some((x0, y0, zw, zh)) => {
                let mut z = Vec::with_capacity(zw * zh);
                for y in y0..y0 + zh {
                    for x in x0..x0 + zw {
                        z.push(img[y * W + x]);
                    }
                }
                (z, zw, zh)
            }
            None => (img, W, H),
        };
        let n = counter.entry(c.sheet).or_insert(0);
        *n += 1;
        let name = format!(
            "{}__{:02}__{}.ppm",
            c.sheet,
            n,
            c.label.replace('/', "／").replace(':', "：")
        );
        save(&dir.join(name), w, h, &img);
    }
}
