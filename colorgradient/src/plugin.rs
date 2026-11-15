//! After Effects 向けの組み込み部分。
//! パラメータを読み、入力ワールドを f32 に変換してコア処理に渡し、結果を書き戻す。

use after_effects as ae;

use crate::engine::{
    self, Curve, LayerTexture, OutputMode, Settings, Texture, TextureKind, TextureParams,
};

/// 色スロットの数とグループ（UI上の折りたたみ）の大きさ
const NUM_SLOTS: u8 = engine::MAX_COLORS as u8;
const SLOTS_PER_GROUP: u8 = 8;

#[derive(Eq, PartialEq, Hash, Clone, Copy, Debug)]
enum Params {
    Tolerance,
    ColorsStart(u8),
    ColorsEnd(u8),
    Enable(u8),
    Color(u8),

    GradStart,
    Width,
    Offset,
    CurveType,
    GradEnd,

    TexStart,
    TextureType,
    Amount,
    CellSize,
    Angle,
    TexLayer,
    Invert,
    Seed,
    Antialias,
    TexEnd,

    RoughStart,
    Roughness,
    RoughScale,
    RoughSeed,
    RoughEnd,

    Output,
}

const TEXTURE_OPTIONS: [&str; 7] = ["Smooth", "Dot", "Line", "Bayer", "Grain", "Noise", "Layer"];

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

/// スロットの初期色（よく使う仕上げ色っぽいもの。実際はスポイトで指定する）
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

fn int_slider(default: i32, max: i32) -> ae::SliderDef<'static> {
    ae::SliderDef::setup(|f| {
        f.set_default(default);
        f.set_valid_min(0);
        f.set_valid_max(max);
        f.set_slider_min(0);
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
        // 色の許容差（0..255、F's Plugins の Level と同じ感覚）
        params.add(
            Params::Tolerance,
            "Tolerance",
            float_slider(0.0, (0.0, 255.0), (0.0, 32.0), 1),
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
                        params.add(
                            Params::Enable(i),
                            &format!("Use {}", i + 1),
                            checkbox(i < 2),
                        )?;
                        params.add(
                            Params::Color(i),
                            &format!("Color {}", i + 1),
                            ae::ColorDef::setup(|f| {
                                f.set_default(default_color(i));
                                f.set_value(f.default());
                            }),
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
                params.add(
                    Params::CurveType,
                    "Curve",
                    ae::PopupDef::setup(|f| {
                        f.set_options(&["Linear", "Smooth"]);
                        f.set_default(1);
                        f.set_value(f.default());
                    }),
                )?;
                Ok(())
            },
        )?;

        params.add_group(
            Params::TexStart,
            Params::TexEnd,
            "Texture",
            false,
            |params| {
                params.add(
                    Params::TextureType,
                    "Texture",
                    ae::PopupDef::setup(|f| {
                        f.set_options(&TEXTURE_OPTIONS);
                        f.set_default(2);
                        f.set_value(f.default());
                    }),
                )?;
                params.add(
                    Params::Amount,
                    "Amount",
                    ae::FloatSliderDef::setup(|f| {
                        f.set_default(100.0);
                        f.set_precision(1);
                        f.set_valid_min(0.0);
                        f.set_valid_max(100.0);
                        f.set_slider_min(0.0);
                        f.set_slider_max(100.0);
                        f.set_display_flags(ae::ValueDisplayFlag::PERCENT);
                        f.set_value(f.default());
                    }),
                )?;
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
                        f.set_value(f.default().into());
                    }),
                )?;
                params.add(
                    Params::TexLayer,
                    "Texture Layer",
                    ae::LayerDef::setup(|_| {}),
                )?;
                params.add(Params::Invert, "Invert", checkbox(false))?;
                params.add(Params::Seed, "Seed", int_slider(0, 100000))?;
                params.add(Params::Antialias, "Antialias", checkbox(true))?;
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
                params.add(Params::RoughSeed, "Seed", int_slider(0, 100000))?;
                Ok(())
            },
        )?;

        params.add(
            Params::Output,
            "Output",
            ae::PopupDef::setup(|f| {
                f.set_options(&["Result", "Gradient (check)"]);
                f.set_default(1);
                f.set_value(f.default());
            }),
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
                out_data.set_return_msg(
                    "JK Color Gradient\rGradients between grouped colors with halftone / dither / texture.",
                );
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

fn read_params(in_data: &InData, params: &ae::Parameters<Params>) -> Result<Read, Error> {
    let ds = downsample(in_data);
    let fs =
        |p: Params| -> Result<f32, Error> { Ok(params.get(p)?.as_float_slider()?.value() as f32) };

    let mut colors = Vec::new();
    for i in 0..NUM_SLOTS {
        if params.get(Params::Enable(i))?.as_checkbox()?.value() {
            let c = params.get(Params::Color(i))?.as_color()?.value();
            colors.push([
                c.red as f32 / 255.0,
                c.green as f32 / 255.0,
                c.blue as f32 / 255.0,
            ]);
        }
    }

    let angle: f32 = params.get(Params::Angle)?.as_angle()?.value().into();
    let settings = Settings {
        colors,
        tolerance: fs(Params::Tolerance)? / 255.0,
        width: fs(Params::Width)? * ds,
        offset: fs(Params::Offset)? * ds,
        curve: if params.get(Params::CurveType)?.as_popup()?.value() == 2 {
            Curve::Smooth
        } else {
            Curve::Linear
        },
        amount: fs(Params::Amount)? / 100.0,
        roughness: fs(Params::Roughness)? * ds,
        rough_scale: fs(Params::RoughScale)? * ds,
        rough_seed: params.get(Params::RoughSeed)?.as_slider()?.value() as u32,
        antialias: params.get(Params::Antialias)?.as_checkbox()?.value(),
        output: if params.get(Params::Output)?.as_popup()?.value() == 2 {
            OutputMode::Gradient
        } else {
            OutputMode::Result
        },
        origin: (0, 0),
    };
    let tex = TextureParams {
        kind: texture_kind(params.get(Params::TextureType)?.as_popup()?.value()),
        cell: fs(Params::CellSize)? * ds,
        angle_deg: angle,
        seed: params.get(Params::Seed)?.as_slider()?.value() as u32,
        invert: params.get(Params::Invert)?.as_checkbox()?.value(),
    };
    Ok(Read { settings, tex })
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
        let pad = (s.width * 0.5 + s.offset.abs() + s.roughness.abs()).ceil() as i32 + 2;
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
        let Read { mut settings, tex } = read_params(in_data, params)?;

        let (iw, ih) = (input.width(), input.height());
        let src = world_to_f32(input);
        let in_origin = input.origin();
        settings.origin = (in_origin.h, in_origin.v);

        // テクスチャレイヤーは輝度だけ使う
        let tex_luma;
        let layer_tex = match tex_world {
            Some(t) if tex.kind == TextureKind::Layer => {
                tex_luma = world_to_f32(t)
                    .iter()
                    .map(|p| 0.299 * p[0] + 0.587 * p[1] + 0.114 * p[2])
                    .collect::<Vec<f32>>();
                let o = t.origin();
                Some(LayerTexture {
                    width: t.width(),
                    height: t.height(),
                    luma: &tex_luma,
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
