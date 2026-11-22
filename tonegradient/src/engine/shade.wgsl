// 色の合成（process.rs の shade と同じ計算）を GPU で行う。
// 1スレッド = 1ピクセル。出力の w：0 = 変更なし、1 = 変更（アルファは入力のまま）、2 = 変更（アルファ 1）

struct Params {
    w: u32, h: u32, nc: u32, aa: u32,
    use_tex: u32, kind: u32, map_mode: u32, curve: u32,
    output: u32, mid_core: u32, rough_seed: u32, tex_seed: u32,
    octaves: u32, dot_shape: u32, invert: u32, layer_w: u32,
    layer_h: u32, layer_ox: i32, layer_oy: i32, interp: u32,
    ox: f32, oy: f32, soft: f32, mid_exp: f32,
    rough_sc: f32, power: f32, amount: f32, mid_amount: f32,
    roughness: f32, cos_a: f32, sin_a: f32, inv_cell: f32,
    tex_offx: f32, tex_offy: f32, dither: f32, contrast: f32,
    layer_scale: f32, layer_black: f32, layer_white: f32, cdf_min: f32,
    cdf_inv_range: f32, aspect: f32, mid_r: f32, mid_g: f32,
    mid_b: f32, lx_a: f32, lx_b: f32, lx_c: f32,
    lx_d: f32, lx_e: f32, lx_f: f32, _p4: f32,
}

@group(0) @binding(0) var<storage, read> P: Params;
// ラベル（1画素1バイトを4つずつ詰めたもの）
@group(0) @binding(1) var<storage, read> labels: array<u32>;
// 近くの色（1画素 8 語、edt.wgsl で作る）：数・ラベル0..2、ラベル3..5、距離6つ
@group(0) @binding(2) var<storage, read> near: array<u32>;
// 色 1つ 8 個：r, g, b, 幅, バイアス, 並び順, 予備2つ
@group(0) @binding(3) var<storage, read> colors: array<f32>;
// 分布表（4097）、カーブの表（257）、レイヤーの値 の順
@group(0) @binding(4) var<storage, read> tables: array<f32>;
@group(0) @binding(5) var<storage, read_write> out_px: array<vec4<f32>>;

const NONE: u32 = 255u;

// 補間の色空間から sRGB の値に戻す（process.rs の Interpolation::decode と同じ）
fn linear_to_srgb(v: f32) -> f32 {
    let a = abs(v);
    var e = a * 12.92;
    if (a > 0.0031308) {
        e = 1.055 * pow(a, 1.0 / 2.4) - 0.055;
    }
    return select(e, -e, v < 0.0);
}

fn decode_interp(v: vec3<f32>) -> vec3<f32> {
    if (P.interp == 1u) {
        return vec3<f32>(linear_to_srgb(v.x), linear_to_srgb(v.y), linear_to_srgb(v.z));
    }
    if (P.interp == 2u) {
        let l = v.x + 0.3963377774 * v.y + 0.2158037573 * v.z;
        let m = v.x - 0.1055613458 * v.y - 0.0638541728 * v.z;
        let s = v.x - 0.0894841775 * v.y - 1.2914855480 * v.z;
        let l3 = l * l * l;
        let m3 = m * m * m;
        let s3 = s * s * s;
        let r = 4.0767416621 * l3 - 3.3077115913 * m3 + 0.2309699292 * s3;
        let g = -1.2684380046 * l3 + 2.6097574011 * m3 - 0.3413193965 * s3;
        let b = -0.0041960863 * l3 - 0.7034186147 * m3 + 1.7076147010 * s3;
        return vec3<f32>(linear_to_srgb(r), linear_to_srgb(g), linear_to_srgb(b));
    }
    return v;
}
const CDF_BINS: u32 = 4096u;
const LUT_OFF: u32 = 4097u;
const LUT_SIZE: u32 = 256u;
const LAYER_OFF: u32 = 4354u;
const TAU: f32 = 6.28318530717958647692;

const BAYER8 = array<u32, 64>(
    0u, 32u, 8u, 40u, 2u, 34u, 10u, 42u,
    48u, 16u, 56u, 24u, 50u, 18u, 58u, 26u,
    12u, 44u, 4u, 36u, 14u, 46u, 6u, 38u,
    60u, 28u, 52u, 20u, 62u, 30u, 54u, 22u,
    3u, 35u, 11u, 43u, 1u, 33u, 9u, 41u,
    51u, 19u, 59u, 27u, 49u, 17u, 57u, 25u,
    15u, 47u, 7u, 39u, 13u, 45u, 5u, 37u,
    63u, 31u, 55u, 23u, 61u, 29u, 53u, 21u,
);

fn hash2(x: i32, y: i32, seed: u32) -> u32 {
    var h = (bitcast<u32>(x) * 0x8da6b343u) ^ (bitcast<u32>(y) * 0xd8163841u) ^ (seed * 0xcb1ab31fu);
    h = h ^ (h >> 13u);
    h = h * 0x5bd1e995u;
    h = h ^ (h >> 15u);
    return h;
}

fn hash_f(x: i32, y: i32, seed: u32) -> f32 {
    return f32(hash2(x, y, seed) >> 8u) / 16777216.0;
}

fn value_noise(x: f32, y: f32, seed: u32) -> f32 {
    let xf = floor(x);
    let yf = floor(y);
    let fx = x - xf;
    let fy = y - yf;
    let xi = i32(xf);
    let yi = i32(yf);
    let sx = fx * fx * (3.0 - 2.0 * fx);
    let sy = fy * fy * (3.0 - 2.0 * fy);
    let a = hash_f(xi, yi, seed);
    let b = hash_f(xi + 1, yi, seed);
    let c = hash_f(xi, yi + 1, seed);
    let d = hash_f(xi + 1, yi + 1, seed);
    let ab = a + (b - a) * sx;
    let cd = c + (d - c) * sx;
    return ab + (cd - ab) * sy;
}

fn fbm_n(x: f32, y: f32, seed: u32, octaves: u32, contrast: f32) -> f32 {
    var sum = 0.0;
    var amp = 1.0;
    var norm = 0.0;
    var fx = x;
    var fy = y;
    let n = clamp(octaves, 1u, 6u);
    for (var o = 0u; o < n; o = o + 1u) {
        sum = sum + value_noise(fx, fy, seed ^ (o * 0x9e3779b9u)) * amp;
        norm = norm + amp;
        amp = amp * 0.5;
        fx = fx * 2.03 + 17.1;
        fy = fy * 2.03 - 9.3;
    }
    return clamp((sum / norm - 0.5) * contrast + 0.5, 0.0, 1.0);
}

// Rust の rem_euclid
fn rem_e(a: i32, n: i32) -> i32 {
    let r = a % n;
    return select(r, r + n, r < 0);
}

// Rust の f32::round（0.5 は 0 から遠い側）
fn round_away(v: f32) -> f32 {
    return sign(v) * floor(abs(v) + 0.5);
}

fn spot(u: f32, v: f32) -> f32 {
    switch P.dot_shape {
        case 0u: { return -(cos(TAU * u) + cos(TAU * v)); }
        case 1u: { return u * u + v * v; }
        case 2u: {
            let a = max(P.aspect, 0.05);
            return u * u / a + v * v * a;
        }
        case 3u: { return max(abs(u), abs(v)); }
        default: { return abs(u) + abs(v); }
    }
}

fn cdf_eval(v: f32) -> f32 {
    let p = clamp((v - P.cdf_min) * P.cdf_inv_range, 0.0, 1.0) * f32(CDF_BINS);
    let b = min(u32(p), CDF_BINS - 1u);
    let f = p - f32(b);
    return tables[b] + (tables[b + 1u] - tables[b]) * f;
}

// 端は端の値を伸ばす（繰り返さない）
fn layer_at(xi: i32, yi: i32) -> f32 {
    let x = clamp(xi, 0, i32(P.layer_w) - 1);
    let y = clamp(yi, 0, i32(P.layer_h) - 1);
    return tables[LAYER_OFF + u32(y) * P.layer_w + u32(x)];
}

// Layer テクスチャの画素の座標。テクスチャのレイヤーの位置・スケール・回転に合わせる（texture.rs の layer_uv）
fn layer_uv(x: f32, y: f32) -> vec2<f32> {
    return vec2<f32>(P.lx_a * x + P.lx_b * y + P.lx_c, P.lx_d * x + P.lx_e * y + P.lx_f);
}

// この位置にテクスチャがあるか（Layer は画像の範囲の中だけ。texture.rs の covers）
fn tex_covers(x: f32, y: f32) -> bool {
    if (P.kind != 6u) {
        return true;
    }
    let uv = layer_uv(x, y);
    return uv.x >= -0.5 && uv.y >= -0.5 && uv.x < f32(P.layer_w) - 0.5 && uv.y < f32(P.layer_h) - 0.5;
}

// テクスチャのしきい値 0..1（texture.rs の threshold と同じ）
fn tex_t(x: f32, y: f32) -> f32 {
    var t = 0.5;
    let k = P.kind;
    if (k == 1u || k == 2u || k == 5u) {
        let dx = x - P.tex_offx;
        let dy = y - P.tex_offy;
        let u = (dx * P.cos_a + dy * P.sin_a) * P.inv_cell;
        let v = (-dx * P.sin_a + dy * P.cos_a) * P.inv_cell;
        if (k == 5u) {
            t = fbm_n(u, v, P.tex_seed, P.octaves, P.contrast);
        } else {
            let fu = u - round_away(u);
            let fv = v - round_away(v);
            var s: f32;
            if (k == 2u) {
                s = abs(fv);
            } else {
                s = spot(fu, fv);
            }
            t = cdf_eval(s);
        }
    } else if (k == 3u) {
        let e = max(P.dither, 0.25);
        let bx = i32(floor((x - P.tex_offx) / e));
        let by = i32(floor((y - P.tex_offy) / e));
        let i = rem_e(by, 8) * 8 + rem_e(bx, 8);
        t = (f32(BAYER8[i]) + 0.5) / 64.0;
    } else if (k == 4u) {
        let e = max(P.dither, 0.25);
        t = hash_f(i32(floor((x - P.tex_offx) / e)), i32(floor((y - P.tex_offy) / e)), P.tex_seed);
    } else if (k == 6u) {
        let uv = layer_uv(x, y);
        let u = uv.x;
        let v = uv.y;
        let u0 = floor(u);
        let v0 = floor(v);
        let fu = u - u0;
        let fv = v - v0;
        let xi = i32(u0);
        let yi = i32(v0);
        let top = layer_at(xi, yi) + (layer_at(xi + 1, yi) - layer_at(xi, yi)) * fu;
        let bottom = layer_at(xi, yi + 1) + (layer_at(xi + 1, yi + 1) - layer_at(xi, yi + 1)) * fu;
        let lv = top + (bottom - top) * fv;
        let b = P.layer_black;
        let wv = P.layer_white;
        if (abs(wv - b) < 1e-6) {
            t = select(0.0, 1.0, lv >= wv);
        } else {
            t = (lv - b) / (wv - b);
        }
    }
    if (P.invert != 0u) {
        t = 1.0 - t;
    }
    return clamp(t, 0.0, 0.99999);
}

fn lut_eval(t: f32) -> f32 {
    let f = clamp(t, 0.0, 1.0) * f32(LUT_SIZE);
    let i = min(u32(f), LUT_SIZE - 1u);
    let r = f - f32(i);
    return tables[LUT_OFF + i] + (tables[LUT_OFF + i + 1u] - tables[LUT_OFF + i]) * r;
}

fn bias_k(k: u32, gx: f32, gy: f32) -> f32 {
    var v = colors[k * 8u + 4u];
    if (P.roughness != 0.0) {
        let sd = P.rough_seed ^ (k * 0x9e3779b9u);
        v = v + P.roughness * (2.0 * fbm_n(gx * P.rough_sc, gy * P.rough_sc, sd, 3u, 2.2) - 1.0);
    }
    return v;
}

fn weight(sd_in: f32, k: u32, gx: f32, gy: f32) -> f32 {
    let wk = colors[k * 8u + 3u];
    let sd = sd_in + bias_k(k, gx, gy);
    var v: f32;
    if (wk > 1e-4) {
        v = clamp(0.5 - sd / wk, 0.0, 1.0);
    } else if (sd < 0.0) {
        v = 1.0;
    } else if (sd == 0.0) {
        v = 0.5;
    } else {
        v = 0.0;
    }
    if (P.curve == 1u) {
        v = v * v * (3.0 - 2.0 * v);
    } else if (P.curve == 2u) {
        v = lut_eval(v);
    }
    if (P.power != 1.0) {
        v = select(pow(v, P.power), 0.0, v <= 0.0);
    }
    return v;
}

fn soft_step(a: f32, t: f32, e_in: f32) -> f32 {
    let e = min(e_in, 2.0 * min(a, 1.0 - a));
    if (e <= 1e-6) {
        return select(0.0, 1.0, t < a);
    }
    return clamp((a - t) / e + 0.5, 0.0, 1.0);
}

fn map_step(a: f32, t: f32) -> f32 {
    return clamp(a + (0.5 - t) * 2.0 * a * (1.0 - a), 0.0, 1.0);
}

fn near_label(base: u32, j: u32) -> u32 {
    let b = j + 1u;
    let word = near[base + b / 4u];
    return (word >> ((b % 4u) * 8u)) & 0xffu;
}

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= P.w || gid.y >= P.h) {
        return;
    }
    let i = gid.y * P.w + gid.x;
    out_px[i] = vec4<f32>(0.0);
    let a = (labels[i >> 2u] >> ((i & 3u) * 8u)) & 0xffu;
    if (a == NONE) {
        return;
    }
    let base = i * 8u;
    let n = near[base] & 0xffu;
    if (n == 0u) {
        return;
    }
    let gx = f32(gid.x) + 0.5 + P.ox;
    let gy = f32(gid.y) + 0.5 + P.oy;

    // 候補：自分 + 近くの色
    var cl: array<u32, 8>;
    var cw: array<f32, 8>;
    var dmin = 3.4e38;
    for (var j = 0u; j < n; j = j + 1u) {
        dmin = min(dmin, bitcast<f32>(near[base + 2u + j]));
    }
    cl[0] = a;
    cw[0] = weight(-(dmin - 0.5), a, gx, gy);
    var m = 1u;
    for (var j = 0u; j < n; j = j + 1u) {
        let k = near_label(base, j);
        cl[m] = k;
        cw[m] = weight(bitcast<f32>(near[base + 2u + j]) - 0.5, k, gx, gy);
        m = m + 1u;
    }
    var sum = 0.0;
    for (var j = 0u; j < m; j = j + 1u) {
        sum = sum + cw[j];
    }
    if (sum <= 0.0) {
        return;
    }
    for (var j = 0u; j < m; j = j + 1u) {
        cw[j] = cw[j] / sum;
    }

    if (P.output == 1u) {
        var v = 0.0;
        for (var j = 0u; j < m; j = j + 1u) {
            v = v + cw[j] * colors[cl[j] * 8u + 5u];
        }
        v = v / (f32(P.nc) - 1.0);
        out_px[i] = vec4<f32>(v, v, v, 2.0);
        return;
    }

    // 中間色
    var mid = 0.0;
    if (P.mid_amount > 0.0) {
        var sq = 0.0;
        for (var j = 0u; j < m; j = j + 1u) {
            sq = sq + cw[j] * cw[j];
        }
        let mix = clamp(2.0 * (1.0 - sq), 0.0, 1.0);
        if (mix > 0.0) {
            mid = pow(mix, P.mid_exp) * min(P.mid_amount, 1.0);
        }
    }
    var ew: array<f32, 8>;
    for (var j = 0u; j < m; j = j + 1u) {
        ew[j] = cw[j] * (1.0 - mid);
    }
    ew[m] = mid;
    let me = select(m, m + 1u, mid > 0.0);

    var frac = ew;
    // Layer テクスチャは画像の範囲の中だけ（外側はなめらかなグラデーションのまま）
    if (P.use_tex != 0u && tex_covers(gx, gy)) {
        // 並び順の大きい色から積み上げる（挿入ソート）
        var order: array<u32, 8>;
        for (var j = 0u; j < m; j = j + 1u) {
            let r = colors[cl[j] * 8u + 5u];
            var q = j;
            while (q > 0u && colors[cl[order[q - 1u]] * 8u + 5u] < r) {
                order[q] = order[q - 1u];
                q = q - 1u;
            }
            order[q] = j;
        }
        if (me > m) {
            let at = select(1u, 0u, P.mid_core != 0u);
            var k = m;
            while (k > at) {
                order[k] = order[k - 1u];
                k = k - 1u;
            }
            order[at] = m;
        }
        var cnt: array<f32, 8>;
        var edge = 0.0;
        if (P.soft > 0.0 && P.map_mode == 0u) {
            let t0 = tex_t(gx, gy);
            let tx = tex_t(gx + 1.0, gy);
            let ty = tex_t(gx, gy + 1.0);
            edge = (abs(tx - t0) + abs(ty - t0)) * P.soft;
        }
        let aa = P.aa;
        let step = 1.0 / f32(aa);
        var add = 1.0;
        if (aa > 1u) {
            add = step * step;
        }
        for (var sy = 0u; sy < aa; sy = sy + 1u) {
            for (var sx = 0u; sx < aa; sx = sx + 1u) {
                var t: f32;
                if (aa > 1u) {
                    t = tex_t(gx + (f32(sx) + 0.5) * step - 0.5, gy + (f32(sy) + 0.5) * step - 0.5);
                } else {
                    t = tex_t(gx, gy);
                }
                if (P.map_mode != 0u || edge > 0.0) {
                    var acc = 0.0;
                    var prev = 0.0;
                    for (var q = 0u; q < me; q = q + 1u) {
                        let j = order[q];
                        acc = acc + ew[j];
                        var cur = 1.0;
                        if (q + 1u != me) {
                            if (P.map_mode != 0u) {
                                cur = map_step(acc, t);
                            } else {
                                cur = soft_step(acc, t, edge);
                            }
                        }
                        cnt[j] = cnt[j] + max(cur - prev, 0.0) * add;
                        prev = cur;
                    }
                } else {
                    var acc = 0.0;
                    var hit = order[me - 1u];
                    for (var q = 0u; q < me; q = q + 1u) {
                        let j = order[q];
                        acc = acc + ew[j];
                        if (t < acc) {
                            hit = j;
                            break;
                        }
                    }
                    cnt[hit] = cnt[hit] + add;
                }
            }
        }
        for (var j = 0u; j < me; j = j + 1u) {
            frac[j] = ew[j] + (cnt[j] - ew[j]) * P.amount;
        }
    }

    // 自分の色のままなら入力をそのまま残す
    if (frac[0] >= 1.0 - 1e-6) {
        return;
    }
    var c = vec3<f32>(0.0);
    for (var j = 0u; j < me; j = j + 1u) {
        var col: vec3<f32>;
        if (j == m) {
            col = vec3<f32>(P.mid_r, P.mid_g, P.mid_b);
        } else {
            let o = cl[j] * 8u;
            col = vec3<f32>(colors[o], colors[o + 1u], colors[o + 2u]);
        }
        c = c + col * frac[j];
    }
    out_px[i] = vec4<f32>(decode_interp(c), 1.0);
}
