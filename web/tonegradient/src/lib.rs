//! JK Tone Gradient をブラウザで試すための WebAssembly。
//!
//! プラグインと同じエンジン（`tonegradient::engine`）をそのまま使う。
//! JavaScript からは `alloc` で確保したメモリに RGBA（8bit）とパラメータを書いて `render` を呼ぶ。
//!
//! パラメータ（f32 の配列、`P_*` の位置）と色（RGB 0..1 を `ncolors` 個）は site/tonegradient.js と合わせる。

use tonegradient::engine::*;

const P_WIDTH: usize = 0;
const P_OFFSET: usize = 1;
const P_REVERSE: usize = 2;
const P_CURVE: usize = 3;
const P_CURVE_POWER: usize = 4;
const P_AMOUNT: usize = 5;
const P_TEX_MODE: usize = 6;
const P_SOFTNESS: usize = 7;
const P_ROUGHNESS: usize = 8;
const P_ROUGH_SCALE: usize = 9;
const P_MID_AMOUNT: usize = 10;
const P_MID_R: usize = 11;
const P_MID_WIDTH: usize = 14;
const P_INTERP: usize = 15;
const P_OUTPUT: usize = 16;
const P_TEX_KIND: usize = 17;
const P_CELL: usize = 18;
const P_ANGLE: usize = 19;
const P_DOT_SHAPE: usize = 20;
const P_ASPECT: usize = 21;
const P_DITHER_SIZE: usize = 22;
const P_OCTAVES: usize = 23;
const P_NOISE_CONTRAST: usize = 24;
const P_SEED: usize = 25;
const P_INVERT: usize = 26;
const P_TOLERANCE: usize = 27;
pub const N_PARAMS: usize = 28;

/// JavaScript 用のメモリ確保（`len` バイト）
#[no_mangle]
pub extern "C" fn alloc(len: usize) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(len.max(1));
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

/// `alloc` で確保したメモリを返す
///
/// # Safety
/// `ptr` と `len` は `alloc` が返したものと同じであること
#[no_mangle]
pub unsafe extern "C" fn dealloc(ptr: *mut u8, len: usize) {
    drop(Vec::from_raw_parts(ptr, 0, len.max(1)));
}

fn flag(v: f32) -> bool {
    v >= 0.5
}

fn settings(p: &[f32], colors: &[f32]) -> Settings {
    // プラグインと同じく、Mid Color はテクスチャなし（Smooth）か Map のときだけ
    let mid_on = p[P_TEX_KIND] as u32 == 0 || flag(p[P_TEX_MODE]);
    Settings {
        colors: colors
            .chunks_exact(3)
            .map(|c| GroupColor::new([c[0], c[1], c[2]]))
            .collect(),
        tolerance: p[P_TOLERANCE].clamp(0.0, 1.0),
        width: p[P_WIDTH].max(0.0),
        offset: p[P_OFFSET],
        reverse: flag(p[P_REVERSE]),
        curve: if flag(p[P_CURVE]) { Curve::Smooth } else { Curve::Linear },
        curve_power: p[P_CURVE_POWER].max(0.01),
        amount: p[P_AMOUNT].clamp(0.0, 1.0),
        texture_mode: if flag(p[P_TEX_MODE]) { TextureMode::Map } else { TextureMode::Threshold },
        softness: p[P_SOFTNESS].max(0.0),
        roughness: p[P_ROUGHNESS].max(0.0),
        rough_scale: p[P_ROUGH_SCALE].max(1.0),
        mid_amount: if mid_on { p[P_MID_AMOUNT].clamp(0.0, 1.0) } else { 0.0 },
        mid_color: [p[P_MID_R], p[P_MID_R + 1], p[P_MID_R + 2]],
        mid_width: p[P_MID_WIDTH].clamp(0.0, 1.0),
        interpolation: match p[P_INTERP] as u32 {
            1 => Interpolation::Linear,
            2 => Interpolation::Perceptual,
            _ => Interpolation::Classic,
        },
        output: if flag(p[P_OUTPUT]) { OutputMode::Gradient } else { OutputMode::Result },
        ..Default::default()
    }
}

fn texture_params(p: &[f32]) -> TextureParams {
    let kind = match p[P_TEX_KIND] as u32 {
        1 => TextureKind::Dot,
        2 => TextureKind::Line,
        3 => TextureKind::Bayer,
        4 => TextureKind::Grain,
        5 => TextureKind::Noise,
        _ => TextureKind::Smooth,
    };
    let dot_shape = match p[P_DOT_SHAPE] as u32 {
        1 => DotShape::Circle,
        2 => DotShape::Ellipse,
        3 => DotShape::Square,
        4 => DotShape::Diamond,
        _ => DotShape::Round,
    };
    TextureParams {
        kind,
        cell: p[P_CELL].max(1.0),
        angle_deg: p[P_ANGLE],
        seed: p[P_SEED].max(0.0) as u32,
        invert: flag(p[P_INVERT]),
        dot_shape,
        aspect: p[P_ASPECT].max(0.1),
        dither_size: p[P_DITHER_SIZE].max(1.0),
        octaves: (p[P_OCTAVES] as u32).clamp(1, 6),
        noise_contrast: p[P_NOISE_CONTRAST].max(0.0),
        ..Default::default()
    }
}

/// `src`（RGBA 8bit、w×h）にエフェクトをかけて `out`（RGBA 8bit）に書く。
///
/// # Safety
/// 各ポインタは `alloc` で確保した十分な大きさのメモリであること
/// （src・out は w×h×4 バイト、params は N_PARAMS 個の f32、colors は ncolors×3 個の f32）
#[no_mangle]
pub unsafe extern "C" fn render(
    w: usize,
    h: usize,
    src: *const u8,
    params: *const f32,
    colors: *const f32,
    ncolors: usize,
    out: *mut u8,
) {
    let n = w * h;
    let src = std::slice::from_raw_parts(src, n * 4);
    let out = std::slice::from_raw_parts_mut(out, n * 4);
    let p = std::slice::from_raw_parts(params, N_PARAMS);
    let colors = std::slice::from_raw_parts(colors, ncolors.min(MAX_COLORS) * 3);

    let px: Vec<[f32; 4]> = src
        .chunks_exact(4)
        .map(|c| [c[0], c[1], c[2], c[3]].map(|v| v as f32 / 255.0))
        .collect();
    out.copy_from_slice(src);
    let s = settings(p, colors);
    if s.colors.len() < 2 {
        return;
    }
    let tex = Texture::new(texture_params(p), None);
    for (i, c) in process(w, h, &px, &s, &tex).changed {
        let o = &mut out[i as usize * 4..i as usize * 4 + 4];
        for k in 0..4 {
            o[k] = (c[k].clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_two_colors() {
        let (w, h) = (16usize, 8usize);
        let mut src = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let c = if x < w / 2 { [255, 0, 0, 255] } else { [0, 0, 255, 255] };
                src[(y * w + x) * 4..][..4].copy_from_slice(&c);
            }
        }
        let mut p = vec![0f32; N_PARAMS];
        p[P_WIDTH] = 8.0;
        p[P_CURVE_POWER] = 1.0;
        p[P_AMOUNT] = 1.0;
        p[P_CELL] = 8.0;
        p[P_ROUGH_SCALE] = 32.0;
        p[P_MID_WIDTH] = 0.5;
        let colors = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0];
        let mut out = vec![0u8; src.len()];
        unsafe { render(w, h, src.as_ptr(), p.as_ptr(), colors.as_ptr(), 2, out.as_mut_ptr()) };
        // 境界の近くは2色の間の色になる
        let mid = &out[(4 * w + w / 2) * 4..][..4];
        assert!(mid[0] > 0 && mid[2] > 0, "{mid:?}");
        // 端はそのまま
        assert_eq!(&out[..4], &[255, 0, 0, 255]);
    }
}
