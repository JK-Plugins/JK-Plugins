//! 距離変換（`prepare`）と色の合成（`shade`）を GPU（wgpu のコンピュートシェーダー）で行う。
//!
//! - CPU でするのはラベル付けだけ（1画素1バイトを送る）。距離変換・近くの色の登録・合成は GPU
//! - 距離変換の結果は GPU に置いたまま `prepare_key` で使い回す（テクスチャ・Amount などだけを
//!   変えたときは合成だけ）
//! - GPU が使えない・エラーになったときは `Err` を返す（呼び出し側で CPU に戻す）

use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use super::curve::CurveLut;
use super::process::{
    biases, label_pixels, ranks, reach, widths, Curve, MidPosition, Output, OutputMode, Settings,
    TextureMode, MAX_NEIGHBORS, NONE,
};
use super::texture::{DotShape, Texture, TextureKind};

const SHADE: &str = include_str!("shade.wgsl");
const EDT: &str = include_str!("edt.wgsl");
const LAYER_OFF: usize = 4354;
/// 近くの色 1画素ぶんの大きさ（u32 8 語）
const NEAR_BYTES: u64 = 32;

/// GPU の処理時間の内訳（ミリ秒）
#[derive(Clone, Copy, Debug, Default)]
pub struct Timing {
    /// ラベル付け（CPU）。キャッシュが効いたときは 0
    pub labels: f64,
    /// 送って計算し、結果を受け取るまで（距離変換をしたときはそれも含む）
    pub run: f64,
    /// 受け取った結果を CPU 側の形にする時間
    pub unpack: f64,
    /// 距離変換をしたか（キャッシュが効かなかった）
    pub prepared: bool,
}

/// GPU に置いた距離変換の結果
struct PreparedBufs {
    key: u64,
    w: usize,
    h: usize,
    /// 2色以上が無く、合成するものが無い
    empty: bool,
    labels: wgpu::Buffer,
    near: wgpu::Buffer,
}

struct Scratch {
    w: usize,
    h: usize,
    batch: usize,
    g: wgpu::Buffer,
    vb: wgpu::Buffer,
    zb: wgpu::Buffer,
    dd: wgpu::Buffer,
}

/// 距離変換の作業用に使ってよい大きさの目安
const SCRATCH_BUDGET: u64 = 256 << 20;
/// 一度にまとめる色の数の上限（edt.wgsl の ks の数）
const MAX_BATCH: usize = 8;

struct Pipelines {
    shade: wgpu::ComputePipeline,
    shade_layout: wgpu::BindGroupLayout,
    col: wgpu::ComputePipeline,
    row: wgpu::ComputePipeline,
    merge: wgpu::ComputePipeline,
    edt_layout: wgpu::BindGroupLayout,
}

struct Ctx {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pl: Pipelines,
    max_binding: u64,
    prepared: Option<PreparedBufs>,
    /// 距離変換の作業用（幅・高さ・まとめる色の数が同じなら使い回す）
    scratch: Option<Scratch>,
    /// 出力と読み戻し用（大きさが変わったら作り直す）
    out: Option<(u64, wgpu::Buffer, wgpu::Buffer)>,
    adapter_name: String,
}

static CTX: OnceLock<Option<Mutex<Ctx>>> = OnceLock::new();

fn ctx() -> Result<&'static Mutex<Ctx>, String> {
    CTX.get_or_init(|| match std::panic::catch_unwind(init) {
        Ok(Ok(c)) => Some(Mutex::new(c)),
        _ => None,
    })
    .as_ref()
    .ok_or_else(|| "GPU が使えません".to_string())
}

/// GPU が使えるか（初めて呼んだときに初期化する）
pub fn available() -> bool {
    ctx().is_ok()
}

/// 使っている GPU の名前
pub fn adapter_name() -> Option<String> {
    ctx().ok().map(|c| c.lock().unwrap().adapter_name.clone())
}

fn layout(device: &wgpu::Device, label: &str, read_only: &[bool]) -> wgpu::BindGroupLayout {
    let entries: Vec<_> = read_only
        .iter()
        .enumerate()
        .map(|(i, &ro)| wgpu::BindGroupLayoutEntry {
            binding: i as u32,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: ro },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        })
        .collect();
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(label),
        entries: &entries,
    })
}

fn pipeline(
    device: &wgpu::Device,
    module: &wgpu::ShaderModule,
    layout: &wgpu::BindGroupLayout,
    entry: &str,
) -> wgpu::ComputePipeline {
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(entry),
        bind_group_layouts: &[layout],
        push_constant_ranges: &[],
    });
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(entry),
        layout: Some(&pl),
        module,
        entry_point: Some(entry),
        compilation_options: Default::default(),
        cache: None,
    })
}

fn init() -> Result<Ctx, String> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::PRIMARY,
        ..Default::default()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        compatible_surface: None,
    }))
    .ok_or("GPU が見つかりません")?;
    let al = adapter.limits();
    let limits = wgpu::Limits {
        max_storage_buffer_binding_size: al.max_storage_buffer_binding_size,
        max_buffer_size: al.max_buffer_size,
        ..wgpu::Limits::default()
    };
    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("tonegradient"),
            required_features: wgpu::Features::empty(),
            required_limits: limits,
            memory_hints: wgpu::MemoryHints::Performance,
        },
        None,
    ))
    .map_err(|e| e.to_string())?;
    // エラーで止まらないようにする（エラーはスコープで受け取る）
    device.on_uncaptured_error(Box::new(|_e| {}));

    device.push_error_scope(wgpu::ErrorFilter::Validation);
    let shade_m = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("shade"),
        source: wgpu::ShaderSource::Wgsl(SHADE.into()),
    });
    let edt_m = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("edt"),
        source: wgpu::ShaderSource::Wgsl(EDT.into()),
    });
    let shade_layout = layout(&device, "shade", &[true, true, true, true, true, false]);
    let edt_layout = layout(
        &device,
        "edt",
        &[true, true, false, false, false, false, false],
    );
    let pl = Pipelines {
        shade: pipeline(&device, &shade_m, &shade_layout, "main"),
        col: pipeline(&device, &edt_m, &edt_layout, "col_pass"),
        row: pipeline(&device, &edt_m, &edt_layout, "row_pass"),
        merge: pipeline(&device, &edt_m, &edt_layout, "merge"),
        shade_layout,
        edt_layout,
    };
    if let Some(e) = pollster::block_on(device.pop_error_scope()) {
        return Err(e.to_string());
    }
    Ok(Ctx {
        device,
        queue,
        pl,
        max_binding: al.max_storage_buffer_binding_size as u64,
        prepared: None,
        scratch: None,
        out: None,
        adapter_name: adapter.get_info().name,
    })
}

fn words_bytes(v: &[u32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn floats_bytes(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

impl Ctx {
    /// 中身を書いたバッファ（バインドする大きさは 0 にできず、4 バイト単位）
    fn buffer(&self, label: &str, bytes: &[u8]) -> Result<wgpu::Buffer, String> {
        let size = (bytes.len().max(16) as u64).div_ceil(4) * 4;
        if size > self.max_binding {
            return Err(format!("{label} が GPU の上限より大きい"));
        }
        let b = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: true,
        });
        {
            let mut view = b.slice(..).get_mapped_range_mut();
            view[..bytes.len()].copy_from_slice(bytes);
            view[bytes.len()..].fill(0);
        }
        b.unmap();
        Ok(b)
    }

    /// GPU だけで使う作業用のバッファ（書き込みもできる）
    fn scratch_buf(&self, label: &str, size: u64) -> Result<wgpu::Buffer, String> {
        let size = (size.max(16)).div_ceil(4) * 4;
        if size > self.max_binding {
            return Err(format!("{label} が GPU の上限より大きい"));
        }
        Ok(self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }))
    }

    /// ラベルを送り、何色かずつまとめて距離変換し、近くの色を登録するコマンドを積む
    #[allow(clippy::too_many_arguments)]
    fn encode_prepare(
        &mut self,
        enc: &mut wgpu::CommandEncoder,
        key: u64,
        w: usize,
        h: usize,
        src: &[[f32; 4]],
        s: &Settings,
        timing: &mut Timing,
    ) -> Result<(), String> {
        let old = self.prepared.take();
        let t = Instant::now();
        let nc = s.colors.len();
        let mut labels = label_pixels(src, s);
        let mut present = vec![false; nc];
        for &l in &labels {
            if l != NONE {
                present[l as usize] = true;
            }
        }
        timing.labels = t.elapsed().as_secs_f64() * 1e3;
        let n = (w * h) as u64;
        // 同じ大きさなら前のバッファを使い回す
        let (labels_b, near) = match old {
            Some(o) if o.w == w && o.h == h => (o.labels, o.near),
            _ => (
                self.scratch_buf("labels", n.div_ceil(4) * 4)?,
                self.scratch_buf("near", n * NEAR_BYTES)?,
            ),
        };
        while labels.len() % 4 != 0 {
            labels.push(NONE);
        }
        self.queue.write_buffer(&labels_b, 0, &labels);
        let ks: Vec<u32> = (0..nc).filter(|&k| present[k]).map(|k| k as u32).collect();
        let empty = ks.len() < 2;
        if !empty {
            enc.clear_buffer(&near, 0, None);
            // 1色ぶんの作業用は 4 本（g・v・z・dd）。目安の大きさに収まるだけまとめる
            let per_color = n * 4 * 4 + h as u64 * 4;
            let batch = ((SCRATCH_BUDGET / per_color) as usize)
                .clamp(1, MAX_BATCH)
                .min(ks.len());
            if !self
                .scratch
                .as_ref()
                .is_some_and(|sc| sc.w == w && sc.h == h && sc.batch == batch)
            {
                self.scratch = None;
                let b = batch as u64;
                self.scratch = Some(Scratch {
                    w,
                    h,
                    batch,
                    g: self.scratch_buf("edt_g", b * n * 4)?,
                    vb: self.scratch_buf("edt_v", b * n * 4)?,
                    zb: self.scratch_buf("edt_z", b * (h * (w + 1)) as u64 * 4)?,
                    dd: self.scratch_buf("edt_dd", b * n * 4)?,
                });
            }
            let sc = self.scratch.as_ref().unwrap();
            let reach = reach(s);
            let limit = s.max_neighbors.clamp(1, MAX_NEIGHBORS) as u32;
            for group in ks.chunks(batch) {
                let mut e = vec![
                    w as u32,
                    h as u32,
                    group.len() as u32,
                    limit,
                    (reach * reach).to_bits(),
                    0,
                    0,
                    0,
                ];
                let mut kk = [0u32; MAX_BATCH];
                kk[..group.len()].copy_from_slice(group);
                e.extend_from_slice(&kk);
                let e_b = self.buffer("edt_params", &words_bytes(&e))?;
                let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("edt"),
                    layout: &self.pl.edt_layout,
                    entries: &[
                        entry(0, &e_b),
                        entry(1, &labels_b),
                        entry(2, &sc.g),
                        entry(3, &sc.vb),
                        entry(4, &sc.zb),
                        entry(5, &sc.dd),
                        entry(6, &near),
                    ],
                });
                let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("edt"),
                    timestamp_writes: None,
                });
                let nb = group.len() as u32;
                pass.set_bind_group(0, &bind, &[]);
                pass.set_pipeline(&self.pl.col);
                pass.dispatch_workgroups((w as u32).div_ceil(64), nb, 1);
                pass.set_pipeline(&self.pl.row);
                pass.dispatch_workgroups((h as u32).div_ceil(64), nb, 1);
                pass.set_pipeline(&self.pl.merge);
                pass.dispatch_workgroups((w as u32).div_ceil(16), (h as u32).div_ceil(16), 1);
            }
        }
        self.prepared = Some(PreparedBufs {
            key,
            w,
            h,
            empty,
            labels: labels_b,
            near,
        });
        timing.prepared = true;
        Ok(())
    }
}

fn entry(binding: u32, b: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: b.as_entire_binding(),
    }
}

/// 設定値を shade.wgsl の Params と同じ並びの語にする
fn params_words(
    w: usize,
    h: usize,
    s: &Settings,
    tex: &Texture,
    use_tex: bool,
    cdf: (f32, f32),
) -> Vec<u32> {
    let v = tex.gpu_view();
    let p = v.p;
    let kind = match p.kind {
        TextureKind::Smooth => 0u32,
        TextureKind::Dot => 1,
        TextureKind::Line => 2,
        TextureKind::Bayer => 3,
        TextureKind::Grain => 4,
        TextureKind::Noise => 5,
        TextureKind::Layer => 6,
    };
    let shape = match p.dot_shape {
        DotShape::Round => 0u32,
        DotShape::Circle => 1,
        DotShape::Ellipse => 2,
        DotShape::Square => 3,
        DotShape::Diamond => 4,
    };
    let (lw, lh, lox, loy) = v.layer.map_or((0, 0, 0, 0), |l| {
        (l.width as u32, l.height as u32, l.origin.0, l.origin.1)
    });
    let power = if s.curve == Curve::Custom {
        1.0
    } else {
        s.curve_power.clamp(0.05, 20.0)
    };
    let u = [
        w as u32,
        h as u32,
        s.colors.len() as u32,
        s.aa_samples.clamp(1, 16),
        use_tex as u32,
        kind,
        (s.texture_mode == TextureMode::Map) as u32,
        match s.curve {
            Curve::Linear => 0,
            Curve::Smooth => 1,
            Curve::Custom => 2,
        },
        (s.output == OutputMode::Gradient) as u32,
        (s.mid_position == MidPosition::Core) as u32,
        s.rough_seed,
        p.seed,
        p.octaves,
        shape,
        p.invert as u32,
        lw,
        lh,
        lox as u32,
        loy as u32,
        0,
    ];
    let f = [
        s.origin.0 as f32,
        s.origin.1 as f32,
        s.softness.max(0.0),
        1.0 + 3.0 * (1.0 - s.mid_width.clamp(0.0, 1.0)),
        1.0 / s.rough_scale.max(0.5),
        power,
        s.amount,
        s.mid_amount,
        s.roughness,
        v.cos_a,
        v.sin_a,
        v.inv_cell,
        p.offset.0,
        p.offset.1,
        p.dither_size,
        p.noise_contrast,
        p.layer_scale,
        p.layer_black,
        p.layer_white,
        cdf.0,
        cdf.1,
        p.aspect,
        s.mid_color[0],
        s.mid_color[1],
        s.mid_color[2],
        0.0,
        0.0,
        0.0,
    ];
    u.into_iter().chain(f.iter().map(|x| x.to_bits())).collect()
}

/// `prepare` + `shade` の GPU 版。`key` は `prepare_key(w, h, src, s)`
/// （同じなら GPU 上の距離変換の結果を使い回す）
pub fn render(
    w: usize,
    h: usize,
    src: &[[f32; 4]],
    key: u64,
    s: &Settings,
    texture: &Texture,
) -> Result<Output, String> {
    render_timed(w, h, src, key, s, texture).map(|(o, _)| o)
}

pub fn render_timed(
    w: usize,
    h: usize,
    src: &[[f32; 4]],
    key: u64,
    s: &Settings,
    texture: &Texture,
) -> Result<(Output, Timing), String> {
    assert_eq!(src.len(), w * h);
    render_with(
        w,
        h,
        || src.to_vec(),
        key,
        s,
        texture,
        |px| Output {
            changed: px.map_or_else(Vec::new, |px| unpack(px, src)),
        },
    )
}

/// `render` と同じ計算をして、読み戻した全画素（w：0 = 変更なし、1 = 変更でアルファは入力のまま、
/// 2 = 変更でアルファ 1）を `f` に渡す。変わった画素の並びを作らずに、そのまま出力へ書き込める。
/// 合成するものが無いときは `None` を渡す。
/// 入力の画素 `src` は距離変換をやり直すとき（キャッシュが効かないとき）だけ読む。
pub fn render_with<R>(
    w: usize,
    h: usize,
    src: impl FnOnce() -> Vec<[f32; 4]>,
    key: u64,
    s: &Settings,
    texture: &Texture,
    f: impl FnOnce(Option<&[[f32; 4]]>) -> R,
) -> Result<(R, Timing), String> {
    if s.colors.len() < 2 || w == 0 || h == 0 {
        return Ok((f(None), Timing::default()));
    }
    let m = ctx()?;
    let mut c = m.lock().map_err(|_| "GPU のロックが壊れています")?;
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run(&mut c, w, h, src, key, s, texture, f)
    })) {
        Ok(Ok(r)) => Ok(r),
        Ok(Err(e)) => {
            c.prepared = None;
            c.scratch = None;
            c.out = None;
            Err(e)
        }
        Err(_) => {
            c.prepared = None;
            c.scratch = None;
            c.out = None;
            Err("GPU の処理で問題が起きました".into())
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run<R>(
    c: &mut Ctx,
    w: usize,
    h: usize,
    src: impl FnOnce() -> Vec<[f32; 4]>,
    key: u64,
    s: &Settings,
    texture: &Texture,
    f: impl FnOnce(Option<&[[f32; 4]]>) -> R,
) -> Result<(R, Timing), String> {
    let mut timing = Timing::default();
    let n = w * h;
    let t = Instant::now();
    c.device.push_error_scope(wgpu::ErrorFilter::Validation);
    c.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let mut enc = c
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("tonegradient"),
        });

    let hit = c
        .prepared
        .as_ref()
        .is_some_and(|b| b.key == key && b.w == w && b.h == h);
    if !hit {
        let src = src();
        if src.len() != w * h {
            let _ = c.device.pop_error_scope();
            let _ = c.device.pop_error_scope();
            return Err("入力の大きさが合いません".into());
        }
        c.encode_prepare(&mut enc, key, w, h, &src, s, &mut timing)?;
    }
    if c.prepared.as_ref().unwrap().empty {
        let _ = c.device.pop_error_scope();
        let _ = c.device.pop_error_scope();
        return Ok((f(None), timing));
    }

    let rank = ranks(s);
    let wid = widths(s);
    let bias = biases(s, &rank);
    let mut cols = Vec::with_capacity(s.colors.len() * 8);
    for k in 0..s.colors.len() {
        let rgb = s.colors[k].rgb;
        cols.extend_from_slice(&[rgb[0], rgb[1], rgb[2], wid[k], bias[k], rank[k], 0.0, 0.0]);
    }
    let use_tex = texture.kind() != TextureKind::Smooth && s.amount > 0.0;
    let view = texture.gpu_view();
    let mut tables = vec![0f32; LAYER_OFF];
    let mut cdf = (0.0, 1.0);
    if let Some((min, inv, table)) = view.cdf {
        tables[..table.len()].copy_from_slice(table);
        cdf = (min, inv);
    }
    if s.curve == Curve::Custom {
        let lut = CurveLut::new(&s.curve_points);
        tables[4097..4097 + lut.values().len()].copy_from_slice(lut.values());
    }
    if let Some(l) = view.layer {
        if view.p.kind == TextureKind::Layer {
            tables.extend_from_slice(&l.value[..l.width * l.height]);
        }
    }
    let params = params_words(w, h, s, texture, use_tex, cdf);
    let params_b = c.buffer("params", &words_bytes(&params))?;
    let colors_b = c.buffer("colors", &floats_bytes(&cols))?;
    let tables_b = c.buffer("tables", &floats_bytes(&tables))?;

    let out_size = (n * 16) as u64;
    if out_size > c.max_binding {
        return Err("出力が GPU の上限より大きい".into());
    }
    if c.out.as_ref().map_or(true, |o| o.0 != out_size) {
        let out = c.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("out"),
            size: out_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let read = c.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("read"),
            size: out_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        c.out = Some((out_size, out, read));
    }
    let pb = c.prepared.as_ref().unwrap();
    let (_, out_b, read_b) = c.out.as_ref().unwrap();
    let bind = c.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("shade"),
        layout: &c.pl.shade_layout,
        entries: &[
            entry(0, &params_b),
            entry(1, &pb.labels),
            entry(2, &pb.near),
            entry(3, &colors_b),
            entry(4, &tables_b),
            entry(5, out_b),
        ],
    });
    {
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("shade"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&c.pl.shade);
        pass.set_bind_group(0, &bind, &[]);
        pass.dispatch_workgroups((w as u32).div_ceil(16), (h as u32).div_ceil(16), 1);
    }
    enc.copy_buffer_to_buffer(out_b, 0, read_b, 0, out_size);
    c.queue.submit([enc.finish()]);

    let slice = read_b.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    c.device.poll(wgpu::Maintain::Wait);
    let oom = pollster::block_on(c.device.pop_error_scope());
    let val = pollster::block_on(c.device.pop_error_scope());
    if let Some(e) = oom.or(val) {
        return Err(e.to_string());
    }
    rx.recv()
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    timing.run = t.elapsed().as_secs_f64() * 1e3 - timing.labels;

    let t = Instant::now();
    let r = {
        let data = slice.get_mapped_range();
        let bytes: &[u8] = &data;
        // SAFETY: 読み戻したバッファは 16 バイト単位に並んだ vec4<f32>（境界が合わないときは止める）
        let (pre, px, _) = unsafe { bytes.align_to::<[f32; 4]>() };
        if pre.is_empty() {
            Some(f(Some(&px[..n])))
        } else {
            None
        }
    };
    read_b.unmap();
    let r = r.ok_or("読み戻したデータの境界が合いません")?;
    timing.unpack = t.elapsed().as_secs_f64() * 1e3;
    Ok((r, timing))
}

/// 出力（w：0 = 変更なし、1 = 入力のアルファ、2 = アルファ 1）を変わった画素の並びにする
fn unpack(px: &[[f32; 4]], src: &[[f32; 4]]) -> Vec<(u32, [f32; 4])> {
    let n = px.len();
    let threads = std::thread::available_parallelism()
        .map_or(1, |v| v.get())
        .min(n.div_ceil(65536))
        .max(1);
    let per = n.div_ceil(threads);
    let parts: Vec<Vec<(u32, [f32; 4])>> = std::thread::scope(|sc| {
        let hs: Vec<_> = px
            .chunks(per)
            .zip(src.chunks(per))
            .enumerate()
            .map(|(ci, (pc, sc_))| {
                sc.spawn(move || {
                    let base = ci * per;
                    let mut v = Vec::new();
                    for (j, (o, s0)) in pc.iter().zip(sc_).enumerate() {
                        if o[3] == 0.0 {
                            continue;
                        }
                        let alpha = if o[3] >= 1.5 { 1.0 } else { s0[3] };
                        v.push(((base + j) as u32, [o[0], o[1], o[2], alpha]));
                    }
                    v
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    parts.concat()
}

#[cfg(test)]
mod tests {
    use super::super::process::{prepare, prepare_key, shade as cpu_shade, GroupColor};
    use super::super::texture::DotShape;
    use super::super::texture::{LayerTexture, TextureParams};
    use super::*;

    /// 3色が接する絵（グループ外の色も少し入れる）
    fn picture(w: usize, h: usize) -> Vec<[f32; 4]> {
        let cols = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        (0..w * h)
            .map(|i| {
                let (x, y) = ((i % w) as f32, (i / w) as f32);
                if (x - 20.0).abs() < 3.0 && (y - 20.0).abs() < 3.0 {
                    return [0.5, 0.5, 0.5, 1.0];
                }
                let a = (y - h as f32 * 0.5)
                    .atan2(x - w as f32 * 0.4)
                    .to_degrees()
                    .rem_euclid(360.0);
                let c = cols[(a / 120.0) as usize % 3];
                [c[0], c[1], c[2], if x < 4.0 { 0.0 } else { 1.0 }]
            })
            .collect()
    }

    fn colors() -> Vec<GroupColor> {
        let mut c: Vec<GroupColor> = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
            .iter()
            .map(|&c| GroupColor::new(c))
            .collect();
        c[1].grow = 4.0;
        c[2].spread = 1.5;
        c
    }

    /// GPU と CPU の結果を比べる。網点のしきい値ぎりぎりの画素は計算誤差で入れ替わるので、
    /// 大きく違う画素が少ないこと、平均の差が小さいことを確かめる
    fn compare(name: &str, s: &Settings, tp: TextureParams, layer: Option<LayerTexture>) {
        let (w, h) = (160, 120);
        let src = picture(w, h);
        let tex = Texture::new(tp, layer);
        let p = prepare(w, h, &src, s);
        let key = prepare_key(w, h, &src, s);
        let cpu = cpu_shade(&p, &src, s, &tex).to_dense(&src);
        let gpu = render(w, h, &src, key, s, &tex).unwrap().to_dense(&src);
        let mut big = 0;
        let mut sum = 0.0f64;
        let mut flag = 0;
        for i in 0..w * h {
            if cpu.changed[i] != gpu.changed[i] {
                flag += 1;
            }
            let d = (0..4)
                .map(|c| (cpu.rgba[i][c] - gpu.rgba[i][c]).abs())
                .fold(0.0f32, f32::max);
            if d > 2e-3 {
                big += 1;
            }
            sum += d as f64;
        }
        let n = (w * h) as f64;
        let mean = sum / n;
        let nch = cpu.changed.iter().filter(|&&c| c).count();
        eprintln!("{name}: changed={nch} big={big} mean={mean:.2e} flag={flag}");
        assert!(nch > 0, "{name}: 何も変わっていない");
        assert!(
            (big as f64) < n * 0.003 && mean < 1e-3 && flag as f64 <= n * 0.003,
            "{name}: big={big} mean={mean} flag={flag}"
        );
    }

    #[test]
    fn gpu_matches_cpu() {
        if !available() {
            eprintln!("GPU が無いので飛ばします");
            return;
        }
        eprintln!("GPU: {:?}", adapter_name());
        let base = Settings {
            colors: colors(),
            width: 30.0,
            offset: 2.0,
            ..Default::default()
        };
        let tp = |kind| TextureParams {
            kind,
            cell: 9.0,
            angle_deg: 30.0,
            seed: 7,
            offset: (3.0, -5.0),
            ..Default::default()
        };
        compare("smooth", &base, tp(TextureKind::Smooth), None);
        for kind in [
            TextureKind::Dot,
            TextureKind::Line,
            TextureKind::Bayer,
            TextureKind::Grain,
            TextureKind::Noise,
        ] {
            compare(&format!("{kind:?}"), &base, tp(kind), None);
            compare(
                &format!("{kind:?} map"),
                &Settings {
                    texture_mode: TextureMode::Map,
                    amount: 0.8,
                    ..base.clone()
                },
                tp(kind),
                None,
            );
            compare(
                &format!("{kind:?} soft aa"),
                &Settings {
                    softness: 1.5,
                    aa_samples: 3,
                    ..base.clone()
                },
                tp(kind),
                None,
            );
        }
        for shape in [
            DotShape::Circle,
            DotShape::Ellipse,
            DotShape::Square,
            DotShape::Diamond,
        ] {
            compare(
                &format!("{shape:?}"),
                &base,
                TextureParams {
                    dot_shape: shape,
                    invert: true,
                    ..tp(TextureKind::Dot)
                },
                None,
            );
        }
        compare(
            "mid rough custom reverse",
            &Settings {
                mid_amount: 0.7,
                mid_width: 0.3,
                mid_color: [1.0, 1.0, 0.0],
                roughness: 3.0,
                rough_seed: 11,
                curve: Curve::Custom,
                curve_points: vec![[0.3, 0.7]],
                reverse: true,
                ..base.clone()
            },
            tp(TextureKind::Dot),
            None,
        );
        compare(
            "mid rim smooth power",
            &Settings {
                mid_amount: 0.5,
                mid_position: MidPosition::Rim,
                curve: Curve::Smooth,
                curve_power: 1.7,
                ..base.clone()
            },
            tp(TextureKind::Noise),
            None,
        );
        compare(
            "gradient",
            &Settings {
                output: OutputMode::Gradient,
                ..base.clone()
            },
            tp(TextureKind::Dot),
            None,
        );
        let vals: Vec<f32> = (0..37 * 23)
            .map(|i| ((i * 7919) % 101) as f32 / 100.0)
            .collect();
        let layer = LayerTexture {
            width: 37,
            height: 23,
            value: &vals,
            origin: (-5, 3),
        };
        compare(
            "layer",
            &base,
            TextureParams {
                layer_scale: 1.7,
                layer_black: 0.1,
                layer_white: 0.8,
                ..tp(TextureKind::Layer)
            },
            Some(layer),
        );
        compare(
            "layer map",
            &Settings {
                texture_mode: TextureMode::Map,
                ..base.clone()
            },
            tp(TextureKind::Layer),
            Some(layer),
        );
        // 混ぜる色の数を絞る（近くの色の入れ替えを通す）
        for limit in [1, 2] {
            compare(
                &format!("max_neighbors {limit}"),
                &Settings {
                    max_neighbors: limit,
                    width: 60.0,
                    ..base.clone()
                },
                tp(TextureKind::Dot),
                None,
            );
        }
        // 原点がずれていても同じ
        compare(
            "origin",
            &Settings {
                origin: (-37, 91),
                ..base.clone()
            },
            tp(TextureKind::Noise),
            None,
        );
    }

    /// いろいろな大きさ・設定の入力で、GPU と CPU が同じ結果になる
    #[test]
    fn gpu_matches_cpu_random() {
        if !available() {
            return;
        }
        let mut r = 0x9e37_79b9u32;
        let mut rnd = move || {
            r ^= r << 13;
            r ^= r >> 17;
            r ^= r << 5;
            r
        };
        let palette = [
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
            [0.0, 0.0, 0.0],
        ];
        let mut worst = 0usize;
        for case in 0..80 {
            let w = 1 + (rnd() % 70) as usize;
            let h = 1 + (rnd() % 50) as usize;
            let blob = 1 + (rnd() % 9) as usize;
            let src: Vec<[f32; 4]> = (0..w * h)
                .map(|i| {
                    let (x, y) = (i % w / blob, i / w / blob);
                    let c = palette[(super::super::texture::hash2(x as i32, y as i32, case) % 6)
                        .min(4) as usize];
                    let a = if rnd() % 13 == 0 { 0.0 } else { 1.0 };
                    [c[0], c[1], c[2], a]
                })
                .collect();
            let nc = (rnd() % 6) as usize;
            let mut colors: Vec<GroupColor> =
                palette[..nc].iter().map(|&c| GroupColor::new(c)).collect();
            for c in &mut colors {
                c.grow = (rnd() % 11) as f32 - 5.0;
                c.spread = (rnd() % 4) as f32 * 0.5;
            }
            let s = Settings {
                colors,
                width: (rnd() % 40) as f32,
                offset: (rnd() % 7) as f32 - 3.0,
                reverse: rnd() % 2 == 0,
                max_neighbors: 1 + (rnd() % 6) as usize,
                roughness: (rnd() % 3) as f32,
                ..Default::default()
            };
            let tex = Texture::new(
                TextureParams {
                    kind: TextureKind::Noise,
                    cell: 3.0,
                    ..Default::default()
                },
                None,
            );
            let cpu = cpu_shade(&prepare(w, h, &src, &s), &src, &s, &tex).to_dense(&src);
            let gpu = render(w, h, &src, prepare_key(w, h, &src, &s), &s, &tex)
                .unwrap()
                .to_dense(&src);
            let bad = (0..w * h)
                .filter(|&i| {
                    cpu.changed[i] != gpu.changed[i]
                        || (0..4).any(|c| (cpu.rgba[i][c] - gpu.rgba[i][c]).abs() > 2e-3)
                })
                .count();
            worst = worst.max(bad);
            assert!(
                bad <= 1 + w * h / 500,
                "case {case} ({w}x{h}): {bad} 画素が違う"
            );
        }
        eprintln!("random: 最大 {worst} 画素の違い");
    }
}
