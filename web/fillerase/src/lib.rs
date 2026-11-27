//! JK Fill Erase をブラウザで試すための WebAssembly。
//!
//! プラグインと同じエンジン（`fillerase::engine`）をそのまま使う。
//! JavaScript からは `alloc` で確保したメモリに RGBA（8bit）・パラメータ・点を書いて `render` を呼ぶ。
//!
//! パラメータ（f32 の配列、`P_*` の位置）は site/app.js と合わせる。点は (x, y) を `npoints` 組。

use fillerase::engine::*;

const P_TOLERANCE: usize = 0;
const P_SOFTNESS: usize = 1;
const P_CHOKE: usize = 2;
const P_CONTIGUOUS: usize = 3;
const P_COMPARE_ALPHA: usize = 4;
const P_INVERT: usize = 5;
/// 0：結果（消したところは透明）、1：消える範囲を色で重ねて見せる
const P_VIEW: usize = 6;
pub const N_PARAMS: usize = 7;

/// 消える範囲を見せるときの色（配布サイトのアクセントの赤）
const MARK: [f32; 3] = [227.0, 32.0, 74.0];

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

fn settings(p: &[f32], points: &[f32]) -> Settings {
    Settings {
        points: points
            .chunks_exact(2)
            .take(MAX_POINTS)
            .map(|c| (c[0], c[1]))
            .collect(),
        tolerance: p[P_TOLERANCE].clamp(0.0, 1.0),
        softness: p[P_SOFTNESS].clamp(0.0, 1.0),
        choke: p[P_CHOKE],
        contiguous: flag(p[P_CONTIGUOUS]),
        compare_alpha: flag(p[P_COMPARE_ALPHA]),
        invert: flag(p[P_INVERT]),
    }
}

/// `src`（RGBA 8bit、w×h）にエフェクトをかけて `out`（RGBA 8bit）に書く。
///
/// # Safety
/// 各ポインタは `alloc` で確保した十分な大きさのメモリであること
/// （src・out は w×h×4 バイト、params は N_PARAMS 個の f32、points は npoints×2 個の f32）
#[no_mangle]
pub unsafe extern "C" fn render(
    w: usize,
    h: usize,
    src: *const u8,
    params: *const f32,
    points: *const f32,
    npoints: usize,
    out: *mut u8,
) {
    let n = w * h;
    let src = std::slice::from_raw_parts(src, n * 4);
    let out = std::slice::from_raw_parts_mut(out, n * 4);
    let p = std::slice::from_raw_parts(params, N_PARAMS);
    let points = std::slice::from_raw_parts(points, npoints.min(MAX_POINTS) * 2);

    let px: Vec<[f32; 4]> = src
        .chunks_exact(4)
        .map(|c| [c[0], c[1], c[2], c[3]].map(|v| v as f32 / 255.0))
        .collect();
    let s = settings(p, points);
    // 点が画像の中に1つもなければ、プラグインと同じく何も消さない（Invert なら全部消す）
    let amount = if seeds(&Image { data: &px, w }, w, h, &s).is_empty() {
        vec![if s.invert { 1.0 } else { 0.0 }; n]
    } else {
        erase_amount(&px, w, h, &s)
    };
    let view = p[P_VIEW] as u32;
    for ((o, c), m) in out.chunks_exact_mut(4).zip(src.chunks_exact(4)).zip(&amount) {
        let m = m.clamp(0.0, 1.0);
        if view == 1 {
            for k in 0..3 {
                o[k] = (c[k] as f32 + (MARK[k] - c[k] as f32) * m * 0.7).round() as u8;
            }
            o[3] = c[3];
        } else {
            o[..3].copy_from_slice(&c[..3]);
            o[3] = (c[3] as f32 * (1.0 - m)).round() as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn erases_connected_side() {
        // 左が白、x=4 が黒い線、右も白
        let (w, h) = (9usize, 3usize);
        let mut src = vec![255u8; w * h * 4];
        for y in 0..h {
            src[(y * w + 4) * 4..][..3].copy_from_slice(&[0, 0, 0]);
        }
        let mut p = vec![0f32; N_PARAMS];
        p[P_TOLERANCE] = 0.1;
        p[P_CONTIGUOUS] = 1.0;
        p[P_COMPARE_ALPHA] = 1.0;
        let pts = [1.0f32, 1.0];
        let mut out = vec![0u8; src.len()];
        unsafe { render(w, h, src.as_ptr(), p.as_ptr(), pts.as_ptr(), 1, out.as_mut_ptr()) };
        let alpha = |x: usize| out[(w + x) * 4 + 3];
        assert_eq!(alpha(0), 0);
        assert_eq!(alpha(4), 255);
        assert_eq!(alpha(8), 255);
    }
}
