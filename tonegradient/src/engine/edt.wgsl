// 距離変換と「近くの色」の登録を GPU で行う（process.rs の prepare と同じ結果）。
// 何色かまとめて（gid.y = まとめた中の番号）列パス → 行パス（Felzenszwalb & Huttenlocher）で
// 二乗距離を求め、merge で画素ごとに色の番号順に登録する
// （近い順に limit 個まで。CPU の Near::insert と同じ規則・同じ順番なので結果も同じ）。

struct E {
    w: u32,
    h: u32,
    nb: u32,
    limit: u32,
    reach2: f32,
    _a: f32,
    _b: f32,
    _c: f32,
    // まとめた色の番号
    ks: array<vec4<u32>, 2>,
}

@group(0) @binding(0) var<storage, read> P: E;
// ラベル（1画素1バイトを4つずつ詰めたもの）
@group(0) @binding(1) var<storage, read> labels: array<u32>;
// 列パスの結果（縦方向の二乗距離。届かないものは INF）
@group(0) @binding(2) var<storage, read_write> g: array<f32>;
// 行パスの作業用（行ごとに w 個・w+1 個）
@group(0) @binding(3) var<storage, read_write> vb: array<u32>;
@group(0) @binding(4) var<storage, read_write> zb: array<f32>;
// 行パスの結果（二乗距離。届かないものは INF）
@group(0) @binding(5) var<storage, read_write> dd_b: array<f32>;
// 近くの色（1画素 8 語：数・ラベル0..2、ラベル3..5、距離6つ）
@group(0) @binding(6) var<storage, read_write> near: array<u32>;

const NONE: u32 = 255u;
const INF: f32 = 1e30;

fn label_at(i: u32) -> u32 {
    return (labels[i >> 2u] >> ((i & 3u) * 8u)) & 0xffu;
}

fn color_of(b: u32) -> u32 {
    return P.ks[b / 4u][b % 4u];
}

@compute @workgroup_size(64)
fn col_pass(@builtin(global_invocation_id) gid: vec3<u32>) {
    let x = gid.x;
    let b = gid.y;
    if (x >= P.w || b >= P.nb) {
        return;
    }
    let w = P.w;
    let kc = color_of(b);
    let go = b * P.w * P.h;
    var last = -1;
    for (var y = 0u; y < P.h; y = y + 1u) {
        let i = y * w + x;
        if (label_at(i) == kc) {
            last = i32(y);
        }
        if (last >= 0) {
            let d = f32(i32(y) - last);
            g[go + i] = d * d;
        } else {
            g[go + i] = INF;
        }
    }
    last = -1;
    for (var yy = i32(P.h) - 1; yy >= 0; yy = yy - 1) {
        let i = u32(yy) * w + x;
        if (label_at(i) == kc) {
            last = yy;
        }
        var v = g[go + i];
        if (last >= 0) {
            let d = f32(last - yy);
            v = min(v, d * d);
        }
        // 届く距離より遠いものは、行パスでも届かない（d² ≥ 縦の d²）
        if (v > P.reach2) {
            v = INF;
        }
        g[go + i] = v;
    }
}

fn set_label(base: u32, j: u32, l: u32) {
    let b = j + 1u;
    let idx = base + b / 4u;
    let sh = (b % 4u) * 8u;
    near[idx] = (near[idx] & ~(0xffu << sh)) | (l << sh);
}

// Near::insert と同じ：limit 個までは追加、それ以上はいちばん遠いもの（同じなら先のもの）と入れ替え
fn insert(base: u32, l: u32, d: f32) {
    let n = near[base] & 0xffu;
    if (n < P.limit) {
        set_label(base, n, l);
        near[base + 2u + n] = bitcast<u32>(d);
        near[base] = (near[base] & ~0xffu) | (n + 1u);
        return;
    }
    var worst = 0u;
    var wd = bitcast<f32>(near[base + 2u]);
    for (var j = 1u; j < P.limit; j = j + 1u) {
        let dj = bitcast<f32>(near[base + 2u + j]);
        if (dj > wd) {
            worst = j;
            wd = dj;
        }
    }
    if (d < wd) {
        set_label(base, worst, l);
        near[base + 2u + worst] = bitcast<u32>(d);
    }
}

@compute @workgroup_size(64)
fn row_pass(@builtin(global_invocation_id) gid: vec3<u32>) {
    let y = gid.x;
    let b = gid.y;
    if (y >= P.h || b >= P.nb) {
        return;
    }
    let w = P.w;
    let ro = b * P.w * P.h + y * w;
    let vo = ro;
    let zo = (b * P.h + y) * (w + 1u);
    var k = -1;
    for (var q = 0u; q < w; q = q + 1u) {
        let fq = g[ro + q];
        if (fq >= INF) {
            continue;
        }
        loop {
            if (k < 0) {
                k = 0;
                vb[vo] = q;
                zb[zo] = -INF;
                zb[zo + 1u] = INF;
                break;
            }
            let p = vb[vo + u32(k)];
            let fp = g[ro + p];
            // 放物線の交点。q² − p² を (q−p)(q+p) に分けて、大きな数どうしの引き算を避ける
            let s = (fq - fp) / (2.0 * f32(q - p)) + f32(q + p) * 0.5;
            if (s <= zb[zo + u32(k)]) {
                k = k - 1;
                continue;
            }
            k = k + 1;
            vb[vo + u32(k)] = q;
            zb[zo + u32(k)] = s;
            zb[zo + u32(k) + 1u] = INF;
            break;
        }
    }
    if (k < 0) {
        for (var q = 0u; q < w; q = q + 1u) {
            dd_b[ro + q] = INF;
        }
        return;
    }
    var j = 0u;
    for (var q = 0u; q < w; q = q + 1u) {
        while (zb[zo + j + 1u] < f32(q)) {
            j = j + 1u;
        }
        let p = vb[vo + j];
        let dq = f32(q) - f32(p);
        dd_b[ro + q] = dq * dq + g[ro + p];
    }
}

// まとめた色を、番号の順に画素ごとに登録する
@compute @workgroup_size(16, 16)
fn merge(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= P.w || gid.y >= P.h) {
        return;
    }
    let i = gid.y * P.w + gid.x;
    if (label_at(i) == NONE) {
        return;
    }
    let n = P.w * P.h;
    for (var b = 0u; b < P.nb; b = b + 1u) {
        let dd = dd_b[b * n + i];
        if (dd <= P.reach2 && dd > 0.0) {
            insert(i * 8u, color_of(b), sqrt(dd));
        }
    }
}
