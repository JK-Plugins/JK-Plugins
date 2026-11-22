//! ワールド（AE / Premiere の画像バッファ）の読み書き。
//!
//! AE に依存しない形（バイト列＋行の並び）にしてあるので、単体でテストできる。
//! 行ごとに独立なので、どれもスレッドで分けて処理する。

/// 16bit チャンネルの最大値（PF_MAX_CHAN16）
pub const MAX_CHANNEL16: f32 = 32768.0;

/// ワールドのピクセルの並び。Premiere は BGRA、AE は ARGB
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Layout {
    Argb8,
    Argb16,
    ArgbF32,
    Bgra8,
    Bgra16,
    BgraF32,
}

impl Layout {
    /// 1ピクセルのバイト数
    pub fn bytes_per_pixel(self) -> usize {
        match self {
            Layout::Argb8 | Layout::Bgra8 => 4,
            Layout::Argb16 | Layout::Bgra16 => 8,
            Layout::ArgbF32 | Layout::BgraF32 => 16,
        }
    }

    fn is_bgra(self) -> bool {
        matches!(self, Layout::Bgra8 | Layout::Bgra16 | Layout::BgraF32)
    }
}

// Pixel8/16/F32 の構造体はメモリ上 alpha, red, green, blue の順。
// BGRA のワールドでは、その位置に B, G, R, A が入っている。
#[inline]
fn from_mem(layout: Layout, m: [f32; 4]) -> [f32; 4] {
    // m = メモリ順の4つの値 → [r, g, b, a]
    if layout.is_bgra() {
        [m[2], m[1], m[0], m[3]]
    } else {
        [m[1], m[2], m[3], m[0]]
    }
}

#[inline]
fn to_mem(layout: Layout, p: [f32; 4]) -> [f32; 4] {
    // [r, g, b, a] → メモリ順の4つの値
    if layout.is_bgra() {
        [p[2], p[1], p[0], p[3]]
    } else {
        [p[3], p[0], p[1], p[2]]
    }
}

/// バイト列から1ピクセルを読み、[r, g, b, a]（0..1、32bit はそのまま）にする
#[inline]
pub fn decode(layout: Layout, b: &[u8]) -> [f32; 4] {
    let m = match layout {
        Layout::Argb8 | Layout::Bgra8 => [b[0], b[1], b[2], b[3]].map(|v| v as f32 / 255.0),
        Layout::Argb16 | Layout::Bgra16 => [0, 1, 2, 3]
            .map(|i| u16::from_le_bytes([b[2 * i], b[2 * i + 1]]) as f32 / MAX_CHANNEL16),
        Layout::ArgbF32 | Layout::BgraF32 => [0, 1, 2, 3]
            .map(|i| f32::from_le_bytes([b[4 * i], b[4 * i + 1], b[4 * i + 2], b[4 * i + 3]])),
    };
    from_mem(layout, m)
}

/// [r, g, b, a] を1ピクセルぶんのバイト列に書く（8/16bit は 0..1 に丸める）
#[inline]
pub fn encode(layout: Layout, p: [f32; 4], out: &mut [u8]) {
    let m = to_mem(layout, p);
    match layout {
        Layout::Argb8 | Layout::Bgra8 => {
            for i in 0..4 {
                out[i] = (m[i].clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            }
        }
        Layout::Argb16 | Layout::Bgra16 => {
            for i in 0..4 {
                let v = (m[i].clamp(0.0, 1.0) * MAX_CHANNEL16 + 0.5) as u16;
                out[2 * i..2 * i + 2].copy_from_slice(&v.to_le_bytes());
            }
        }
        Layout::ArgbF32 | Layout::BgraF32 => {
            for i in 0..4 {
                out[4 * i..4 * i + 4].copy_from_slice(&m[i].to_le_bytes());
            }
        }
    }
}

/// バッファの中の行の並び（rowbytes が負のときは下の行から並んでいる）
#[derive(Clone, Copy, Debug)]
pub struct Rows {
    pub stride: usize,
    pub flip: bool,
    pub h: usize,
}

impl Rows {
    /// y 行目の先頭のバイト位置
    #[inline]
    pub fn start(&self, y: usize) -> usize {
        if self.flip {
            (self.h - 1 - y) * self.stride
        } else {
            y * self.stride
        }
    }
    /// メモリ上 r 番目の行が何行目か
    #[inline]
    pub fn y_of(&self, r: usize) -> usize {
        if self.flip {
            self.h - 1 - r
        } else {
            r
        }
    }
}

/// 1スレッドが受け持つ行数
pub fn thread_rows(h: usize) -> usize {
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(h.max(1));
    h.div_ceil(threads.max(1)).max(1)
}

/// ワールドを RGBA f32 の配列（行優先、w×h）に変換する
pub fn read_world(buf: &[u8], rows: Rows, w: usize, layout: Layout) -> Vec<[f32; 4]> {
    let h = rows.h;
    let mut v = vec![[0f32; 4]; w * h];
    if w == 0 || h == 0 {
        return v;
    }
    let bpp = layout.bytes_per_pixel();
    let rows_per = thread_rows(h);
    std::thread::scope(|sc| {
        for (ci, chunk) in v.chunks_mut(rows_per * w).enumerate() {
            sc.spawn(move || {
                for (r, row) in chunk.chunks_mut(w).enumerate() {
                    let start = rows.start(ci * rows_per + r);
                    let src = &buf[start..start + w * bpp];
                    for (px, b) in row.iter_mut().zip(src.chunks_exact(bpp)) {
                        *px = decode(layout, b);
                    }
                }
            });
        }
    });
    v
}

/// 入力ワールドの画像
pub struct Source<'a> {
    pub buf: &'a [u8],
    pub rows: Rows,
    pub w: usize,
    pub layout: Layout,
}

/// 出力ワールドを作る：入力の重なる部分をそのまま写し（入力の外は透明）、
/// `changed`（入力のピクセル番号の昇順）のピクセルだけ上書きする。
/// (dx, dy) は出力の原点から見た入力の原点のずれ（出力の (x, y) が入力の (x+dx, y+dy)）。
#[allow(clippy::too_many_arguments)]
/// 入力から変わったピクセル
#[derive(Clone, Copy)]
pub enum Changes<'a> {
    /// （入力の番号 y*w+x、色）を番号の昇順に
    List(&'a [(u32, [f32; 4])]),
    /// 入力と同じ大きさの全画素。w：0 = 変更なし、1 = 変更（アルファは入力のまま）、
    /// 2 = 変更（アルファ 1）。GPU から読み戻したものをそのまま使う
    Dense(&'a [[f32; 4]]),
}

#[allow(clippy::too_many_arguments)]
pub fn compose(
    input: &Source,
    out_buf: &mut [u8],
    out_rows: Rows,
    ow: usize,
    out_layout: Layout,
    dx: i32,
    dy: i32,
    changes: Changes,
) {
    let (iw, ih) = (input.w, input.rows.h);
    let oh = out_rows.h;
    if ow == 0 || oh == 0 {
        return;
    }
    let in_bpp = input.layout.bytes_per_pixel();
    let out_bpp = out_layout.bytes_per_pixel();
    // 出力の x のうち、入力と重なる範囲
    let xa = (-(dx as i64)).clamp(0, ow as i64) as usize;
    let xb = (iw as i64 - dx as i64).clamp(0, ow as i64) as usize;
    let stride = out_rows.stride;
    let rows_per = thread_rows(oh);
    std::thread::scope(|sc| {
        for (ci, chunk) in out_buf.chunks_mut(stride * rows_per).enumerate() {
            sc.spawn(move || {
                for (r, row) in chunk.chunks_mut(stride).enumerate() {
                    let y = out_rows.y_of(ci * rows_per + r);
                    let row = &mut row[..ow * out_bpp];
                    row.fill(0);
                    let sy = y as i64 + dy as i64;
                    if sy < 0 || sy >= ih as i64 || xa >= xb {
                        continue;
                    }
                    let sy = sy as usize;
                    let in_start = input.rows.start(sy);
                    let sxa = (xa as i64 + dx as i64) as usize;
                    if input.layout == out_layout {
                        let n = (xb - xa) * out_bpp;
                        row[xa * out_bpp..xa * out_bpp + n].copy_from_slice(
                            &input.buf[in_start + sxa * in_bpp..in_start + sxa * in_bpp + n],
                        );
                    } else {
                        for x in xa..xb {
                            let sx = (x as i64 + dx as i64) as usize;
                            let p = decode(
                                input.layout,
                                &input.buf[in_start + sx * in_bpp..in_start + (sx + 1) * in_bpp],
                            );
                            encode(out_layout, p, &mut row[x * out_bpp..(x + 1) * out_bpp]);
                        }
                    }
                    match changes {
                        Changes::List(changed) => {
                            // この入力行で変わったピクセル（changed は番号の昇順）
                            let lo = changed.partition_point(|c| (c.0 as usize) < sy * iw);
                            let hi = changed.partition_point(|c| (c.0 as usize) < (sy + 1) * iw);
                            for &(i, p) in &changed[lo..hi] {
                                let x = (i as usize - sy * iw) as i64 - dx as i64;
                                if x >= 0 && (x as usize) < ow {
                                    let x = x as usize;
                                    encode(out_layout, p, &mut row[x * out_bpp..(x + 1) * out_bpp]);
                                }
                            }
                        }
                        Changes::Dense(px) => {
                            let line = &px[sy * iw..(sy + 1) * iw];
                            for x in xa..xb {
                                let sx = (x as i64 + dx as i64) as usize;
                                let o = line[sx];
                                if o[3] == 0.0 {
                                    continue;
                                }
                                let alpha = if o[3] >= 1.5 {
                                    1.0
                                } else {
                                    decode(
                                        input.layout,
                                        &input.buf
                                            [in_start + sx * in_bpp..in_start + (sx + 1) * in_bpp],
                                    )[3]
                                };
                                encode(
                                    out_layout,
                                    [o[0], o[1], o[2], alpha],
                                    &mut row[x * out_bpp..(x + 1) * out_bpp],
                                );
                            }
                        }
                    }
                }
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// テスト用のワールド（行の末尾に余白あり、flip なら下の行から並ぶ）
    fn make_world(
        w: usize,
        h: usize,
        layout: Layout,
        flip: bool,
        px: impl Fn(usize, usize) -> [f32; 4],
    ) -> (Vec<u8>, Rows) {
        let bpp = layout.bytes_per_pixel();
        let stride = w * bpp + 8;
        let rows = Rows { stride, flip, h };
        let mut buf = vec![0xAAu8; stride * h];
        for y in 0..h {
            for x in 0..w {
                let s = rows.start(y) + x * bpp;
                encode(layout, px(x, y), &mut buf[s..s + bpp]);
            }
        }
        (buf, rows)
    }

    fn pattern(x: usize, y: usize) -> [f32; 4] {
        [
            (x % 7) as f32 / 6.0,
            (y % 5) as f32 / 4.0,
            ((x + y) % 3) as f32 / 2.0,
            1.0,
        ]
    }

    #[test]
    fn encode_decode_roundtrip_all_layouts() {
        let p = [0.2, 0.4, 0.6, 0.8];
        for layout in [
            Layout::Argb8,
            Layout::Argb16,
            Layout::ArgbF32,
            Layout::Bgra8,
            Layout::Bgra16,
            Layout::BgraF32,
        ] {
            let mut b = vec![0u8; layout.bytes_per_pixel()];
            encode(layout, p, &mut b);
            let q = decode(layout, &b);
            for i in 0..4 {
                assert!((p[i] - q[i]).abs() < 1.0 / 255.0, "{layout:?} {q:?}");
            }
        }
        // ARGB8 のメモリ順は alpha, red, green, blue
        let mut b = [0u8; 4];
        encode(Layout::Argb8, [1.0, 0.0, 0.0, 0.5], &mut b);
        assert_eq!(b, [128, 255, 0, 0]);
        // BGRA8 は blue, green, red, alpha
        encode(Layout::Bgra8, [1.0, 0.0, 0.0, 0.5], &mut b);
        assert_eq!(b, [0, 0, 255, 128]);
    }

    #[test]
    fn read_world_handles_padding_and_flip() {
        for flip in [false, true] {
            let (w, h) = (9, 6);
            let (buf, rows) = make_world(w, h, Layout::Argb8, flip, pattern);
            let v = read_world(&buf, rows, w, Layout::Argb8);
            for y in 0..h {
                for x in 0..w {
                    let e = pattern(x, y);
                    let g = v[y * w + x];
                    for i in 0..4 {
                        assert!((e[i] - g[i]).abs() < 1.0 / 255.0, "flip={flip} {x},{y}");
                    }
                }
            }
        }
    }

    /// 出力が入力とずれていても（dx, dy）、重なる部分は入力、外は透明、変わったピクセルは上書き
    #[test]
    fn compose_with_offsets_and_flip() {
        let (iw, ih) = (8, 5);
        for (in_flip, out_flip) in [(false, false), (true, false), (false, true), (true, true)] {
            for (dx, dy) in [(0, 0), (2, 1), (-3, -2), (10, 0), (0, -7)] {
                let (ow, oh) = (7, 6);
                let (in_buf, in_rows) = make_world(iw, ih, Layout::Argb16, in_flip, pattern);
                let input = Source {
                    buf: &in_buf,
                    rows: in_rows,
                    w: iw,
                    layout: Layout::Argb16,
                };
                let stride = ow * 8 + 16;
                let out_rows = Rows {
                    stride,
                    flip: out_flip,
                    h: oh,
                };
                let mut out_buf = vec![0x55u8; stride * oh];
                // 入力の (3, 2) と (7, 4) を変える
                let changed = vec![
                    (2 * iw as u32 + 3, [1.0, 0.0, 0.0, 1.0]),
                    (4 * iw as u32 + 7, [0.0, 1.0, 0.0, 1.0]),
                ];
                compose(
                    &input,
                    &mut out_buf,
                    out_rows,
                    ow,
                    Layout::Argb16,
                    dx,
                    dy,
                    Changes::List(&changed),
                );
                let out = read_world(&out_buf, out_rows, ow, Layout::Argb16);
                for y in 0..oh {
                    for x in 0..ow {
                        let sx = x as i32 + dx;
                        let sy = y as i32 + dy;
                        let inside = sx >= 0 && sy >= 0 && (sx as usize) < iw && (sy as usize) < ih;
                        let expect = if !inside {
                            [0.0; 4]
                        } else if (sx, sy) == (3, 2) {
                            [1.0, 0.0, 0.0, 1.0]
                        } else if (sx, sy) == (7, 4) {
                            [0.0, 1.0, 0.0, 1.0]
                        } else {
                            pattern(sx as usize, sy as usize)
                        };
                        let got = out[y * ow + x];
                        for i in 0..4 {
                            assert!(
                                (expect[i] - got[i]).abs() < 1e-3,
                                "flip {in_flip}/{out_flip} d=({dx},{dy}) at {x},{y}: {got:?} vs {expect:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    /// 入力と出力で並びが違っても（例：8bit → 32bit）色が保たれる
    #[test]
    fn compose_converts_layouts() {
        let (w, h) = (4, 3);
        let (in_buf, in_rows) = make_world(w, h, Layout::Bgra8, false, pattern);
        let input = Source {
            buf: &in_buf,
            rows: in_rows,
            w,
            layout: Layout::Bgra8,
        };
        let out_rows = Rows {
            stride: w * 16,
            flip: false,
            h,
        };
        let mut out_buf = vec![0u8; w * 16 * h];
        compose(
            &input,
            &mut out_buf,
            out_rows,
            w,
            Layout::ArgbF32,
            0,
            0,
            Changes::List(&[]),
        );
        let out = read_world(&out_buf, out_rows, w, Layout::ArgbF32);
        for y in 0..h {
            for x in 0..w {
                let e = pattern(x, y);
                for i in 0..4 {
                    assert!((e[i] - out[y * w + x][i]).abs() < 1.0 / 255.0);
                }
            }
        }
    }

    /// 入力が空でも落ちず、出力は透明になる
    #[test]
    fn compose_with_empty_input() {
        let input = Source {
            buf: &[],
            rows: Rows {
                stride: 0,
                flip: false,
                h: 0,
            },
            w: 0,
            layout: Layout::Argb8,
        };
        let out_rows = Rows {
            stride: 3 * 4,
            flip: false,
            h: 2,
        };
        let mut out = vec![7u8; 3 * 4 * 2];
        compose(
            &input,
            &mut out,
            out_rows,
            3,
            Layout::Argb8,
            0,
            0,
            Changes::List(&[]),
        );
        assert!(out.iter().all(|&b| b == 0));
    }

    /// 全画素の形（Dense）でも、変わった画素の並び（List）と同じ出力になる
    #[test]
    fn compose_dense_matches_list() {
        let (iw, ih) = (8, 5);
        for layout in [
            Layout::Argb8,
            Layout::Argb16,
            Layout::ArgbF32,
            Layout::Bgra8,
        ] {
            for (dx, dy) in [(0, 0), (2, 1), (-3, -2)] {
                let (in_buf, in_rows) = make_world(iw, ih, layout, false, pattern);
                let input = Source {
                    buf: &in_buf,
                    rows: in_rows,
                    w: iw,
                    layout,
                };
                let src = read_world(&in_buf, in_rows, iw, layout);
                let mut dense = vec![[0f32; 4]; iw * ih];
                let mut list = Vec::new();
                for (i, flag) in [(2 * iw + 3, 2.0f32), (4 * iw + 7, 1.0), (iw + 1, 1.0)] {
                    let c = [0.25, 0.5, 0.75];
                    dense[i] = [c[0], c[1], c[2], flag];
                    let a = if flag >= 1.5 { 1.0 } else { src[i][3] };
                    list.push((i as u32, [c[0], c[1], c[2], a]));
                }
                list.sort_by_key(|c| c.0);
                let (ow, oh) = (7, 6);
                let stride = ow * layout.bytes_per_pixel() + 4;
                let rows = Rows {
                    stride,
                    flip: false,
                    h: oh,
                };
                let mut a = vec![0x33u8; stride * oh];
                let mut b = vec![0x77u8; stride * oh];
                compose(
                    &input,
                    &mut a,
                    rows,
                    ow,
                    layout,
                    dx,
                    dy,
                    Changes::List(&list),
                );
                compose(
                    &input,
                    &mut b,
                    rows,
                    ow,
                    layout,
                    dx,
                    dy,
                    Changes::Dense(&dense),
                );
                assert_eq!(
                    read_world(&a, rows, ow, layout),
                    read_world(&b, rows, ow, layout),
                    "{dx},{dy}"
                );
            }
        }
    }
}
