//! After Effects 向けの組み込み部分。
//! パラメータを読み、入力ワールドを f32 に変換してコア処理に渡し、結果を書き戻す。

use after_effects as ae;

use crate::engine::{
    self, Curve, DotShape, GroupColor, LayerTexture, OutputMode, Settings, Texture, TextureKind,
    TextureParams,
};

/// 色スロットの数とグループ（UI上の折りたたみ）の大きさ
const NUM_SLOTS: u8 = engine::MAX_COLORS as u8;
const SLOTS_PER_GROUP: u8 = 8;

#[derive(Eq, PartialEq, Hash, Clone, Copy, Debug)]
enum Params {
    Tolerance,
    AlphaThreshold,
    ColorsStart(u8),
    ColorsEnd(u8),
    Enable(u8),
    Color(u8),
    ColorTolerance(u8),
    Spread(u8),
    Grow(u8),

    GradStart,
    Width,
    Offset,
    Reverse,
    CurveType,
    CurvePower,
    MaxNeighbors,
    MidColor,
    MidAmount,
    GradEnd,

    TexStart,
    TextureType,
    Amount,
    CellSize,
    Angle,
    TexOffsetX,
    TexOffsetY,
    DotShapeType,
    Aspect,
    DitherSize,
    Octaves,
    NoiseContrast,
    TexLayer,
    LayerChannel,
    LayerScale,
    LayerBlack,
    LayerWhite,
    Invert,
    Seed,
    AaSamples,
    TexEnd,

    RoughStart,
    Roughness,
    RoughScale,
    RoughSeed,
    RoughEnd,

    AnimateSeed,
    Output,
}

const TEXTURE_OPTIONS: [&str; 7] = ["Smooth", "Dot", "Line", "Bayer", "Grain", "Noise", "Layer"];
const DOT_OPTIONS: [&str; 5] = ["Round", "Circle", "Ellipse", "Square", "Diamond"];
const AA_OPTIONS: [&str; 4] = ["Off", "2x2", "4x4", "8x8"];
const CHANNEL_OPTIONS: [&str; 5] = ["Luminance", "Alpha", "Red", "Green", "Blue"];

fn texture_kind(popup: i32) -> TextureKind {
    match popup {
        2 => TextureKind::Dot,
        3 => TextureKind::Line,
        4 => TextureKind::Bayer,
        5 => TextureKind::Grain,
        6 => TextureKind::Noise,
        7 => TextureKind::Layer,
        _ => TextureKind::Smooth,
    }
}

fn dot_shape(popup: i32) -> DotShape {
    match popup {
        2 => DotShape::Circle,
        3 => DotShape::Ellipse,
        4 => DotShape::Square,
        5 => DotShape::Diamond,
        _ => DotShape::Round,
    }
}

/// スロットの初期色（実際はスポイトで指定する）
fn default_color(i: u8) -> ae::Pixel8 {
    const C: [(u8, u8, u8); 8] = [
        (255, 255, 255),
        (192, 192, 192),
        (128, 128, 128),
        (64, 64, 64),
        (255, 0, 0),
        (0, 255, 0),
        (0, 0, 255),
        (255, 255, 0),
    ];
    let (r, g, b) = C[(i % 8) as usize];
    ae::Pixel8 {
        alpha: 255,
        red: r,
        green: g,
        blue: b,
    }
}

#[derive(Default)]
struct Plugin {}

ae::define_effect!(Plugin, (), Params);

fn float_slider(
    default: f32,
    valid: (f32, f32),
    slider: (f32, f32),
    precision: i16,
) -> ae::FloatSliderDef<'static> {
    ae::FloatSliderDef::setup(|f| {
        f.set_default(default as f64);
        f.set_precision(precision);
        f.set_valid_min(valid.0);
        f.set_valid_max(valid.1);
        f.set_slider_min(slider.0);
        f.set_slider_max(slider.1);
        f.set_value(f.default());
    })
}

fn percent_slider(default: f32) -> ae::FloatSliderDef<'static> {
    ae::FloatSliderDef::setup(|f| {
        f.set_default(default as f64);
        f.set_precision(1);
        f.set_valid_min(0.0);
        f.set_valid_max(100.0);
        f.set_slider_min(0.0);
        f.set_slider_max(100.0);
        f.set_display_flags(ae::ValueDisplayFlag::PERCENT);
        f.set_value(f.default());
    })
}

fn int_slider(default: i32, min: i32, max: i32) -> ae::SliderDef<'static> {
    ae::SliderDef::setup(|f| {
        f.set_default(default);
        f.set_valid_min(min);
        f.set_valid_max(max);
        f.set_slider_min(min);
        f.set_slider_max(max.min(1000));
        f.set_value(f.default());
    })
}

fn checkbox(default: bool) -> ae::CheckBoxDef<'static> {
    ae::CheckBoxDef::setup(|f| {
        f.set_default(default);
        f.set_value(f.default());
    })
}

fn popup(options: &[&str], default: i32) -> ae::PopupDef<'static> {
    let mut p = ae::PopupDef::new();
    p.set_options(options);
    p.set_default(default);
    p.set_value(default);
    p
}

impl AdobePluginGlobal for Plugin {
    fn can_load(_host_name: &str, _host_version: &str) -> bool {
        true
    }

    fn params_setup(
        &self,
        params: &mut ae::Parameters<Params>,
        _in_data: InData,
        _: OutData,
    ) -> Result<(), Error> {
        // 色の許容差（0..255、F's Plugins の Level と同じ感覚）。各色の Tolerance と足して使う
        params.add(
            Params::Tolerance,
            "Tolerance",
            float_slider(0.0, (0.0, 255.0), (0.0, 32.0), 1),
        )?;
        // これ以上のアルファのピクセルだけを対象にする
        params.add(
            Params::AlphaThreshold,
            "Alpha Threshold",
            float_slider(255.0, (0.0, 255.0), (0.0, 255.0), 0),
        )?;

        // 色グループ：8色ずつ折りたたみ。2つ目以降は閉じておく
        for g in 0..NUM_SLOTS / SLOTS_PER_GROUP {
            let first = g * SLOTS_PER_GROUP;
            let name = format!("Colors {}-{}", first + 1, first + SLOTS_PER_GROUP);
            params.add_group(
                Params::ColorsStart(g),
                Params::ColorsEnd(g),
                &name,
                g > 0,
                |params| {
                    for i in first..first + SLOTS_PER_GROUP {
                        let n = i + 1;
                        params.add(Params::Enable(i), &format!("Use {n}"), checkbox(i < 2))?;
                        params.add(
                            Params::Color(i),
                            &format!("Color {n}"),
                            ae::ColorDef::setup(|f| {
                                f.set_default(default_color(i));
                                f.set_value(f.default());
                            }),
                        )?;
                        params.add(
                            Params::ColorTolerance(i),
                            &format!("Tolerance {n}"),
                            float_slider(0.0, (0.0, 255.0), (0.0, 32.0), 1),
                        )?;
                        // にじむ幅の倍率
                        params.add(
                            Params::Spread(i),
                            &format!("Spread {n}"),
                            float_slider(1.0, (0.0, 20.0), (0.0, 4.0), 2),
                        )?;
                        // 領域を広げる量（px）
                        params.add(
                            Params::Grow(i),
                            &format!("Grow {n}"),
                            float_slider(0.0, (-500.0, 500.0), (-30.0, 30.0), 1),
                        )?;
                    }
                    Ok(())
                },
            )?;
        }

        params.add_group(
            Params::GradStart,
            Params::GradEnd,
            "Gradient",
            false,
            |params| {
                // 境界の両側に半分ずつ広がる全体の幅
                params.add(
                    Params::Width,
                    "Width",
                    float_slider(20.0, (0.0, 2000.0), (0.0, 200.0), 1),
                )?;
                // ＋で番号の大きい色の側へ境界を進める
                params.add(
                    Params::Offset,
                    "Offset",
                    float_slider(0.0, (-1000.0, 1000.0), (-50.0, 50.0), 1),
                )?;
                params.add(Params::Reverse, "Reverse Order", checkbox(false))?;
                params.add(Params::CurveType, "Curve", popup(&["Linear", "Smooth"], 1))?;
                params.add(
                    Params::CurvePower,
                    "Curve Power",
                    float_slider(1.0, (0.05, 20.0), (0.25, 4.0), 2),
                )?;
                params.add(
                    Params::MaxNeighbors,
                    "Max Mix Colors",
                    int_slider(4, 1, engine::MAX_NEIGHBORS as i32),
                )?;
                params.add(
                    Params::MidColor,
                    "Mid Color",
                    ae::ColorDef::setup(|f| {
                        f.set_default(ae::Pixel8 {
                            alpha: 255,
                            red: 255,
                            green: 255,
                            blue: 255,
                        });
                        f.set_value(f.default());
                    }),
                )?;
                params.add(Params::MidAmount, "Mid Amount", percent_slider(0.0))?;
                Ok(())
            },
        )?;

        params.add_group(
            Params::TexStart,
            Params::TexEnd,
            "Texture",
            false,
            |params| {
                params.add(Params::TextureType, "Texture", popup(&TEXTURE_OPTIONS, 2))?;
                params.add(Params::Amount, "Amount", percent_slider(100.0))?;
                params.add(
                    Params::CellSize,
                    "Cell Size",
                    float_slider(8.0, (1.0, 1000.0), (2.0, 64.0), 1),
                )?;
                params.add(
                    Params::Angle,
                    "Angle",
                    ae::AngleDef::setup(|f| {
                        f.set_default(45.0);
                        f.set_value(f.default());
                    }),
                )?;
                params.add(
                    Params::TexOffsetX,
                    "Offset X",
                    float_slider(0.0, (-10000.0, 10000.0), (-64.0, 64.0), 1),
                )?;
                params.add(
                    Params::TexOffsetY,
                    "Offset Y",
                    float_slider(0.0, (-10000.0, 10000.0), (-64.0, 64.0), 1),
                )?;
                params.add(Params::DotShapeType, "Dot Shape", popup(&DOT_OPTIONS, 1))?;
                params.add(
                    Params::Aspect,
                    "Ellipse Aspect",
                    float_slider(2.0, (0.05, 20.0), (0.25, 4.0), 2),
                )?;
                params.add(
                    Params::DitherSize,
                    "Dither Size",
                    float_slider(1.0, (0.25, 100.0), (1.0, 8.0), 1),
                )?;
                params.add(Params::Octaves, "Noise Detail", int_slider(3, 1, 6))?;
                params.add(
                    Params::NoiseContrast,
                    "Noise Contrast",
                    float_slider(2.2, (0.1, 10.0), (0.5, 5.0), 2),
                )?;
                params.add(
                    Params::TexLayer,
                    "Texture Layer",
                    ae::LayerDef::setup(|_| {}),
                )?;
                params.add(
                    Params::LayerChannel,
                    "Layer Channel",
                    popup(&CHANNEL_OPTIONS, 1),
                )?;
                params.add(
                    Params::LayerScale,
                    "Layer Scale",
                    float_slider(100.0, (1.0, 10000.0), (10.0, 400.0), 1),
                )?;
                params.add(Params::LayerBlack, "Layer Black", percent_slider(0.0))?;
                params.add(Params::LayerWhite, "Layer White", percent_slider(100.0))?;
                params.add(Params::Invert, "Invert", checkbox(false))?;
                params.add(Params::Seed, "Seed", int_slider(0, 0, 100000))?;
                params.add(Params::AaSamples, "Antialias", popup(&AA_OPTIONS, 3))?;
                Ok(())
            },
        )?;

        params.add_group(
            Params::RoughStart,
            Params::RoughEnd,
            "Roughness",
            true,
            |params| {
                params.add(
                    Params::Roughness,
                    "Amount",
                    float_slider(0.0, (0.0, 500.0), (0.0, 30.0), 1),
                )?;
                params.add(
                    Params::RoughScale,
                    "Scale",
                    float_slider(20.0, (1.0, 2000.0), (2.0, 200.0), 1),
                )?;
                params.add(Params::RoughSeed, "Seed", int_slider(0, 0, 100000))?;
                Ok(())
            },
        )?;

        // フレームごとに Seed を変える（砂目・ノイズ・荒らしが毎フレーム動く）
        params.add(Params::AnimateSeed, "Animate Seed", checkbox(false))?;
        params.add(
            Params::Output,
            "Output",
            popup(&["Result", "Gradient (check)"], 1),
        )?;

        Ok(())
    }

    fn handle_command(
        &mut self,
        cmd: ae::Command,
        in_data: InData,
        mut out_data: OutData,
        params: &mut ae::Parameters<Params>,
    ) -> Result<(), ae::Error> {
        match cmd {
            ae::Command::About => {
                out_data.set_return_msg("JK Color Gradient\rGradients between grouped colors with halftone / dither / texture.");
            }
            ae::Command::GlobalSetup => {
                self.global_setup(&in_data)?;
            }
            ae::Command::Render {
                in_layer,
                out_layer,
            } => {
                self.legacy_render(&in_data, in_layer, out_layer, params)?;
            }
            ae::Command::SmartPreRender { extra } => {
                self.smart_pre_render(&in_data, extra, params)?;
            }
            ae::Command::SmartRender { extra } => {
                self.smart_render(&in_data, extra, params)?;
            }
            _ => {}
        }
        Ok(())
    }
}

/// パラメータをまとめて読む（px単位の値はダウンサンプルを掛ける）
struct Read {
    settings: Settings,
    tex: TextureParams,
    /// Layer Channel のポップアップ値（1 = 輝度）
    layer_channel: i32,
}

fn downsample(in_data: &InData) -> f32 {
    let x: f32 = in_data.downsample_x().into();
    let y: f32 = in_data.downsample_y().into();
    let d = x.max(y);
    if d.is_finite() && d > 0.0 {
        d
    } else {
        1.0
    }
}

/// 現在のフレーム番号（Animate Seed 用）
fn frame_index(in_data: &InData) -> u32 {
    let step = in_data.time_step();
    if step == 0 {
        0
    } else {
        (in_data.current_time() / step) as u32
    }
}

fn read_params(in_data: &InData, params: &ae::Parameters<Params>) -> Result<Read, Error> {
    let ds = downsample(in_data);
    let fs =
        |p: Params| -> Result<f32, Error> { Ok(params.get(p)?.as_float_slider()?.value() as f32) };
    let popup = |p: Params| -> Result<i32, Error> { Ok(params.get(p)?.as_popup()?.value()) };
    let int = |p: Params| -> Result<i32, Error> { Ok(params.get(p)?.as_slider()?.value()) };

    let mut colors = Vec::new();
    for i in 0..NUM_SLOTS {
        if params.get(Params::Enable(i))?.as_checkbox()?.value() {
            let c = params.get(Params::Color(i))?.as_color()?.value();
            colors.push(GroupColor {
                rgb: [
                    c.red as f32 / 255.0,
                    c.green as f32 / 255.0,
                    c.blue as f32 / 255.0,
                ],
                tolerance: fs(Params::ColorTolerance(i))? / 255.0,
                spread: fs(Params::Spread(i))?,
                grow: fs(Params::Grow(i))? * ds,
            });
        }
    }

    let frame = if params.get(Params::AnimateSeed)?.as_checkbox()?.value() {
        frame_index(in_data)
    } else {
        0
    };
    let mid = params.get(Params::MidColor)?.as_color()?.value();
    let aa = match popup(Params::AaSamples)? {
        1 => 1,
        2 => 2,
        4 => 8,
        _ => 4,
    };

    let settings = Settings {
        colors,
        tolerance: fs(Params::Tolerance)? / 255.0,
        alpha_threshold: fs(Params::AlphaThreshold)? / 255.0,
        width: fs(Params::Width)? * ds,
        offset: fs(Params::Offset)? * ds,
        reverse: params.get(Params::Reverse)?.as_checkbox()?.value(),
        curve: if popup(Params::CurveType)? == 2 {
            Curve::Smooth
        } else {
            Curve::Linear
        },
        curve_power: fs(Params::CurvePower)?,
        max_neighbors: int(Params::MaxNeighbors)?.max(1) as usize,
        amount: fs(Params::Amount)? / 100.0,
        aa_samples: aa,
        roughness: fs(Params::Roughness)? * ds,
        rough_scale: fs(Params::RoughScale)? * ds,
        rough_seed: (int(Params::RoughSeed)? as u32).wrapping_add(frame.wrapping_mul(7919)),
        mid_color: [
            mid.red as f32 / 255.0,
            mid.green as f32 / 255.0,
            mid.blue as f32 / 255.0,
        ],
        mid_amount: fs(Params::MidAmount)? / 100.0,
        output: if popup(Params::Output)? == 2 {
            OutputMode::Gradient
        } else {
            OutputMode::Result
        },
        origin: (0, 0),
    };
    let angle: f32 = params.get(Params::Angle)?.as_angle()?.value();
    let tex = TextureParams {
        kind: texture_kind(popup(Params::TextureType)?),
        cell: fs(Params::CellSize)? * ds,
        angle_deg: angle,
        seed: (int(Params::Seed)? as u32).wrapping_add(frame.wrapping_mul(104_729)),
        invert: params.get(Params::Invert)?.as_checkbox()?.value(),
        offset: (fs(Params::TexOffsetX)? * ds, fs(Params::TexOffsetY)? * ds),
        dot_shape: dot_shape(popup(Params::DotShapeType)?),
        aspect: fs(Params::Aspect)?,
        dither_size: fs(Params::DitherSize)? * ds,
        octaves: int(Params::Octaves)?.max(1) as u32,
        noise_contrast: fs(Params::NoiseContrast)?,
        layer_scale: fs(Params::LayerScale)? / 100.0 * ds,
        layer_black: fs(Params::LayerBlack)? / 100.0,
        layer_white: fs(Params::LayerWhite)? / 100.0,
    };
    Ok(Read {
        settings,
        tex,
        layer_channel: popup(Params::LayerChannel)?,
    })
}

/// ワールドを RGBA f32（0..1）に変換
fn world_to_f32(layer: &ae::Layer) -> Vec<[f32; 4]> {
    let (w, h) = (layer.width(), layer.height());
    let mut v = Vec::with_capacity(w * h);
    match layer.world_type() {
        ae::aegp::WorldType::U8 => {
            for y in 0..h {
                for x in 0..w {
                    let p = layer.as_pixel8(x, y);
                    v.push([
                        p.red as f32 / 255.0,
                        p.green as f32 / 255.0,
                        p.blue as f32 / 255.0,
                        p.alpha as f32 / 255.0,
                    ]);
                }
            }
        }
        ae::aegp::WorldType::U15 => {
            let m = ae::MAX_CHANNEL16 as f32;
            for y in 0..h {
                for x in 0..w {
                    let p = layer.as_pixel16(x, y);
                    v.push([
                        p.red as f32 / m,
                        p.green as f32 / m,
                        p.blue as f32 / m,
                        p.alpha as f32 / m,
                    ]);
                }
            }
        }
        _ => {
            for y in 0..h {
                for x in 0..w {
                    let p = layer.as_pixel32(x, y);
                    v.push([p.red, p.green, p.blue, p.alpha]);
                }
            }
        }
    }
    v
}

impl Plugin {
    fn global_setup(&mut self, in_data: &InData) -> Result<(), ae::Error> {
        win_dbg_logger::DEBUGGER_LOGGER.set_force_log_without_debugger(true);
        log::info!("GlobalSetup");
        if in_data.is_premiere() {
            let suite = ae::pf::suites::PixelFormat::new()?;
            suite.clear_supported_pixel_formats(in_data.effect_ref())?;
            for x in [
                ae::pr::PixelFormat::Bgra4444_8u,
                ae::pr::PixelFormat::Bgra4444_16u,
                ae::pr::PixelFormat::Bgra4444_32f,
            ] {
                suite.add_supported_pixel_format(in_data.effect_ref(), x)?;
            }
        }
        Ok(())
    }

    fn legacy_render(
        &mut self,
        in_data: &InData,
        in_layer: ae::Layer,
        mut out_layer: ae::Layer,
        params: &mut ae::Parameters<Params>,
    ) -> Result<(), ae::Error> {
        if !in_data.is_premiere() {
            // AE では SmartFX のみ
            return Err(Error::BadCallbackParameter);
        }
        // Premiere ではテクスチャレイヤーは使わない
        self.do_render(in_data, &in_layer, None, &mut out_layer, params)
    }

    fn smart_pre_render(
        &mut self,
        in_data: &InData,
        mut extra: ae::PreRenderExtra,
        params: &mut ae::Parameters<Params>,
    ) -> Result<(), ae::Error> {
        let r = read_params(in_data, params)?;
        let s = &r.settings;
        // 境界の向こう側まで見えるように、要求範囲をグラデーションの届く距離だけ広げる
        let n = s.colors.len().max(1) as f32;
        let spread = s.colors.iter().map(|c| c.spread).fold(1.0f32, f32::max);
        let grow = s.colors.iter().map(|c| c.grow.abs()).fold(0.0f32, f32::max);
        let pad = (s.width * spread * 0.5
            + 2.0 * grow
            + s.offset.abs() * (n - 1.0)
            + 2.0 * s.roughness.abs())
        .ceil() as i32
            + 2;
        let mut req = extra.output_request();
        req.rect.left -= pad;
        req.rect.top -= pad;
        req.rect.right += pad;
        req.rect.bottom += pad;

        if let Ok(in_result) = extra.callbacks().checkout_layer(
            0,
            0,
            &req,
            in_data.current_time(),
            in_data.time_step(),
            in_data.time_scale(),
        ) {
            let _ = extra.union_result_rect(in_result.result_rect.into());
            let _ = extra.union_max_result_rect(in_result.max_result_rect.into());
            extra.set_returns_extra_pixels(true);
        }

        if r.tex.kind == TextureKind::Layer {
            if let Some(index) = params.index(Params::TexLayer) {
                let _ = extra.callbacks().checkout_layer(
                    index as i32,
                    1,
                    &req,
                    in_data.current_time(),
                    in_data.time_step(),
                    in_data.time_scale(),
                );
            }
        }
        Ok(())
    }

    fn smart_render(
        &mut self,
        in_data: &InData,
        extra: ae::SmartRenderExtra,
        params: &mut ae::Parameters<Params>,
    ) -> Result<(), ae::Error> {
        let cb = extra.callbacks();
        let Some(input_world) = cb.checkout_layer_pixels(0)? else {
            return Ok(());
        };
        let use_layer = texture_kind(params.get(Params::TextureType)?.as_popup()?.value())
            == TextureKind::Layer;
        let tex_world = if use_layer {
            cb.checkout_layer_pixels(1).ok().flatten()
        } else {
            None
        };

        if let Ok(Some(mut output_world)) = cb.checkout_output() {
            self.do_render(
                in_data,
                &input_world,
                tex_world.as_ref(),
                &mut output_world,
                params,
            )?;
        }

        if use_layer {
            let _ = cb.checkin_layer_pixels(1);
        }
        cb.checkin_layer_pixels(0)?;
        Ok(())
    }

    fn do_render(
        &self,
        in_data: &ae::InData,
        input: &ae::Layer,
        tex_world: Option<&ae::Layer>,
        output: &mut ae::Layer,
        params: &ae::Parameters<Params>,
    ) -> Result<(), Error> {
        let Read {
            mut settings,
            tex,
            layer_channel,
        } = read_params(in_data, params)?;

        let (iw, ih) = (input.width(), input.height());
        let src = world_to_f32(input);
        let in_origin = input.origin();
        settings.origin = (in_origin.h, in_origin.v);

        // テクスチャレイヤー：選んだチャンネルだけ使う
        let tex_value;
        let layer_tex = match tex_world {
            Some(t) if tex.kind == TextureKind::Layer => {
                tex_value = world_to_f32(t)
                    .iter()
                    .map(|p| match layer_channel {
                        2 => p[3],
                        3 => p[0],
                        4 => p[1],
                        5 => p[2],
                        _ => 0.299 * p[0] + 0.587 * p[1] + 0.114 * p[2],
                    })
                    .collect::<Vec<f32>>();
                let o = t.origin();
                Some(LayerTexture {
                    width: t.width(),
                    height: t.height(),
                    value: &tex_value,
                    origin: (o.h, o.v),
                })
            }
            _ => None,
        };
        let texture = Texture::new(tex, layer_tex);

        let result = engine::process(iw, ih, &src, &settings, &texture);

        // 出力ワールドと入力ワールドの位置合わせ（原点の差）
        let out_origin = output.origin();
        let dx = out_origin.h - in_origin.h;
        let dy = out_origin.v - in_origin.v;
        let (ow, oh) = (output.width(), output.height());
        let world_type = output.world_type();

        for y in 0..oh {
            for x in 0..ow {
                let sx = x as i32 + dx;
                let sy = y as i32 + dy;
                let inside = sx >= 0 && sy >= 0 && (sx as usize) < iw && (sy as usize) < ih;
                let i = if inside {
                    sy as usize * iw + sx as usize
                } else {
                    0
                };
                let changed = inside && result.changed[i];
                match world_type {
                    ae::aegp::WorldType::U8 => {
                        let o = output.as_pixel8_mut(x, y);
                        if !inside {
                            *o = ae::Pixel8 {
                                alpha: 0,
                                red: 0,
                                green: 0,
                                blue: 0,
                            };
                        } else if changed {
                            let p = result.rgba[i];
                            let c = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                            *o = ae::Pixel8 {
                                alpha: c(p[3]),
                                red: c(p[0]),
                                green: c(p[1]),
                                blue: c(p[2]),
                            };
                        } else {
                            *o = *input.as_pixel8(sx as usize, sy as usize);
                        }
                    }
                    ae::aegp::WorldType::U15 => {
                        let o = output.as_pixel16_mut(x, y);
                        if !inside {
                            *o = ae::Pixel16 {
                                alpha: 0,
                                red: 0,
                                green: 0,
                                blue: 0,
                            };
                        } else if changed {
                            let p = result.rgba[i];
                            let m = ae::MAX_CHANNEL16 as f32;
                            let c = |v: f32| (v.clamp(0.0, 1.0) * m + 0.5) as u16;
                            *o = ae::Pixel16 {
                                alpha: c(p[3]),
                                red: c(p[0]),
                                green: c(p[1]),
                                blue: c(p[2]),
                            };
                        } else {
                            *o = *input.as_pixel16(sx as usize, sy as usize);
                        }
                    }
                    _ => {
                        let o = output.as_pixel32_mut(x, y);
                        if !inside {
                            *o = ae::PixelF32 {
                                alpha: 0.0,
                                red: 0.0,
                                green: 0.0,
                                blue: 0.0,
                            };
                        } else if changed {
                            let p = result.rgba[i];
                            *o = ae::PixelF32 {
                                alpha: p[3],
                                red: p[0],
                                green: p[1],
                                blue: p[2],
                            };
                        } else {
                            *o = *input.as_pixel32(sx as usize, sy as usize);
                        }
                    }
                }
            }
        }
        Ok(())
    }
}
