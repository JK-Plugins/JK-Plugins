use colorgradient::engine::*;
use std::io::Write;

fn save(path: &str, w: usize, h: usize, px: &[[f32;4]]) {
    let mut f = std::fs::File::create(path).unwrap();
    write!(f, "P6\n{w} {h}\n255\n").unwrap();
    let buf: Vec<u8> = px.iter().flat_map(|p| [p[0],p[1],p[2]].map(|c| (c.clamp(0.0,1.0)*255.0).round() as u8)).collect();
    f.write_all(&buf).unwrap();
}

fn scene(w: usize, h: usize) -> (Vec<[f32;4]>, Vec<[f32;3]>) {
    let base = [0.96, 0.80, 0.70]; let shadow = [0.80, 0.58, 0.55]; let shadow2 = [0.62, 0.42, 0.48]; let hi = [1.0, 0.95, 0.88];
    let bg = [0.55, 0.70, 0.85]; let line = [0.1, 0.08, 0.1];
    let mut px = vec![[0.0;4]; w*h];
    let (cx, cy, r) = (w as f32*0.5, h as f32*0.5, h as f32*0.42);
    for y in 0..h { for x in 0..w {
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        let d = ((fx-cx).powi(2)+(fy-cy).powi(2)).sqrt();
        let c = if (d - r).abs() < 2.0 { line }
            else if d > r { bg }
            else {
                let sx = fx - cx - r*0.35; let sy = fy - cy - r*0.2;
                if ((fx-cx+r*0.35).powi(2)+(fy-cy+r*0.4).powi(2)).sqrt() < r*0.22 { hi }
                else if (sx*sx+sy*sy).sqrt() < r*0.45 { shadow2 }
                else if fx - cx + (fy-cy)*0.4 > -r*0.05 { shadow } else { base }
            };
        px[y*w+x] = [c[0],c[1],c[2],1.0];
    }}
    (px, vec![hi, base, shadow, shadow2])
}

fn main() {
    let (w, h) = (480, 360);
    let (src, colors) = scene(w, h);
    save("out_src.ppm", w, h, &src);
    let variants: Vec<(&str, TextureKind, f32, f32, OutputMode)> = vec![
        ("gradient", TextureKind::Smooth, 0.0, 0.0, OutputMode::Gradient),
        ("smooth", TextureKind::Smooth, 0.0, 0.0, OutputMode::Result),
        ("dot", TextureKind::Dot, 8.0, 0.0, OutputMode::Result),
        ("line", TextureKind::Line, 6.0, 0.0, OutputMode::Result),
        ("bayer", TextureKind::Bayer, 16.0, 0.0, OutputMode::Result),
        ("grain", TextureKind::Grain, 8.0, 0.0, OutputMode::Result),
        ("noise_rough", TextureKind::Noise, 6.0, 6.0, OutputMode::Result),
        ("dot_rough", TextureKind::Dot, 6.0, 5.0, OutputMode::Result),
    ];
    for (name, kind, cell, rough, out) in variants {
        let s = Settings { colors: colors.clone(), width: 40.0, roughness: rough, rough_scale: 20.0, output: out, ..Default::default() };
        let tex = Texture::new(TextureParams { kind, cell, angle_deg: 45.0, seed: 1, invert: false }, None);
        let o = process(w, h, &src, &s, &tex);
        let img: Vec<[f32;4]> = o.rgba.iter().zip(&o.changed).zip(&src).map(|((p,c),s)| if *c {*p} else {*s}).collect();
        save(&format!("out_{name}.ppm"), w, h, &img);
    }
    // timing
    let (w, h) = (1920, 1080);
    let (src, colors) = scene(w, h);
    for (name, kind) in [("smooth", TextureKind::Smooth), ("dot+AA", TextureKind::Dot), ("bayer", TextureKind::Bayer)] {
        let s = Settings { colors: colors.clone(), width: 40.0, ..Default::default() };
        let tex = Texture::new(TextureParams { kind, cell: 8.0, ..Default::default() }, None);
        let t = std::time::Instant::now();
        let _ = process(w, h, &src, &s, &tex);
        println!("1920x1080 {name}: {:?}", t.elapsed());
    }
    println!("threads: {:?}", std::thread::available_parallelism());
}
