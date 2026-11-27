//! After Effects 向けの組み込み部分。
//! パラメータを読み、入力ワールドを f32 に変換してコア処理に渡し、消した結果を書き戻す。

use after_effects as ae;

use crate::engine::{Settings, MAX_POINTS};
use crate::frame;
use crate::world_io::{Layout, Rows, Source};

/// パラメータ。並び順 = エフェクトコントロールでの並び順（params_setup で追加する順）
#[derive(Eq, PartialEq, Hash, Clone, Copy, Debug)]
enum Params {
    PointCount,
    Point(u8),
    Tolerance,
    Softness,
    Choke,
    Contiguous,
    CompareAlpha,
    Invert,
}

#[derive(Default)]
struct Plugin {}

ae::define_effect!(Plugin, (), Params);

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
        // 点の数を変えると、使わない点を隠す
        params.add_with_flags(
            Params::PointCount,
            "Point Count",
            ae::SliderDef::setup(|f| {
                f.set_default(1);
                f.set_valid_min(1);
                f.set_valid_max(MAX_POINTS as i32);
                f.set_slider_min(1);
                f.set_slider_max(MAX_POINTS as i32);
                f.set_value(f.default());
            }),
            ae::ParamFlag::SUPERVISE,
            ae::ParamUIFlags::empty(),
        )?;
        // AE ではパラメータをあとから足せないので、最大数を作っておき、使わないものを隠す
        for i in 0..MAX_POINTS as u8 {
            params.add(
                Params::Point(i),
                &format!("Point {}", i + 1),
                ae::PointDef::setup(|f| {
                    f.set_default(default_point(i));
                    f.set_restrict_bounds(false);
                }),
            )?;
        }
        params.add(Params::Tolerance, "Tolerance", percent_slider(10.0))?;
        params.add(Params::Softness, "Softness", percent_slider(0.0))?;
        params.add(
            Params::Choke,
            "Choke",
            ae::FloatSliderDef::setup(|f| {
                f.set_default(0.0);
                f.set_precision(1);
                f.set_valid_min(-1000.0);
                f.set_valid_max(1000.0);
                f.set_slider_min(-20.0);
                f.set_slider_max(20.0);
                f.set_value(f.default());
            }),
        )?;
        params.add(Params::Contiguous, "Contiguous", checkbox(true))?;
        params.add(Params::CompareAlpha, "Compare Alpha", checkbox(true))?;
        params.add(Params::Invert, "Invert", checkbox(false))?;
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
                out_data.set_return_msg(&format!(
                    "JK Fill Erase v{}\rErase the area connected to the points, like a paint bucket.\rhttps://github.com/JK-Plugins/JK-Plugins",
                    env!("CARGO_PKG_VERSION")
                ));
            }
            ae::Command::GlobalSetup => {
                global_setup(&in_data)?;
            }
            ae::Command::UpdateParamsUi => {
                // 失敗しても描画には影響しないので、エラーは返さない
                if let Err(e) = update_visibility(&in_data, params) {
                    log::warn!("update_visibility: {e:?}");
                }
            }
            ae::Command::UserChangedParam { param_index } => {
                if params.type_at(param_index) == Params::PointCount {
                    if let Err(e) = update_visibility(&in_data, params) {
                        log::warn!("update_visibility: {e:?}");
                    }
                    out_data.set_out_flag(ae::OutFlags::RefreshUi, true);
                }
            }
            ae::Command::Render {
                in_layer,
                mut out_layer,
            } => {
                if !in_data.is_premiere() {
                    // AE では SmartFX のみ
                    return Err(Error::BadCallbackParameter);
                }
                do_render(&in_data, &in_layer, &mut out_layer, params)?;
            }
            ae::Command::SmartPreRender { extra } => {
                smart_pre_render(&in_data, extra)?;
            }
            ae::Command::SmartRender { extra } => {
                let cb = extra.callbacks();
                let Some(input) = cb.checkout_layer_pixels(INPUT)? else {
                    return Ok(());
                };
                if let Ok(Some(mut output)) = cb.checkout_output() {
                    do_render(&in_data, &input, &mut output, params)?;
                }
                cb.checkin_layer_pixels(INPUT)?;
            }
            _ => {}
        }
        Ok(())
    }
}

/// 入力レイヤーの取り出しに使う ID（PROBE は範囲を調べるだけ）
const INPUT: u32 = 0;
const PROBE: i32 = 1;

/// 点の既定の位置（レイヤーの大きさに対する %）。1つ目は中央、残りは格子状に並べて重ならないようにする
fn default_point(i: u8) -> (f32, f32) {
    if i == 0 {
        return (50.0, 50.0);
    }
    let j = (i - 1) as f32;
    (10.0 + 10.0 * (j % 9.0), 10.0 + 8.0 * (j / 9.0).floor())
}

/// パラメータを隠すために AEGP に登録した ID（Premiere などで使えなければ None）
static AEGP_ID: std::sync::OnceLock<Option<ae::aegp::PluginId>> = std::sync::OnceLock::new();

fn point_count(params: &ae::Parameters<Params>) -> Result<usize, Error> {
    Ok(params
        .get(Params::PointCount)?
        .as_slider()?
        .value()
        .clamp(1, MAX_POINTS as i32) as usize)
}

/// Point Count より後ろの点を隠す
fn update_visibility(in_data: &InData, params: &ae::Parameters<Params>) -> Result<(), Error> {
    let count = point_count(params)?;
    if in_data.is_premiere() {
        // Premiere では UI フラグで隠す（読み取り専用で渡されるので、書き換えはコピーに対して行う）
        let mut copy = params.cloned();
        for i in 0..MAX_POINTS as u8 {
            let hidden = i as usize >= count;
            let mut def = copy.get_mut(Params::Point(i))?;
            if def.ui_flags().contains(ae::ParamUIFlags::INVISIBLE) != hidden {
                def.set_ui_flag(ae::ParamUIFlags::INVISIBLE, hidden);
                def.update_param_ui()?;
            }
        }
        return Ok(());
    }
    // AE ではストリームの Hidden フラグで隠す
    use ae::aegp::suites::{DynamicStream, PFInterface, Stream};
    let Some(plugin_id) = AEGP_ID.get().copied().flatten() else {
        return Ok(());
    };
    let effect = PFInterface::new()?.new_effect_for_effect(in_data.effect_ref(), plugin_id)?;
    let streams = Stream::new()?;
    let dynamic = DynamicStream::new()?;
    for i in 0..MAX_POINTS as u8 {
        let Some(index) = params.index(Params::Point(i)) else {
            continue;
        };
        let hidden = i as usize >= count;
        let stream = streams.new_effect_stream_by_index(&effect, plugin_id, index as i32)?;
        // 今のフラグは読めない（ラッパーがビットの組み合わせを扱えない）ので毎回設定する。
        // 取り消しの履歴には残さない
        dynamic.set_dynamic_stream_flag(
            &stream,
            ae::aegp::DynamicStreamFlags::Hidden,
            false,
            hidden,
        )?;
    }
    Ok(())
}

fn global_setup(in_data: &InData) -> Result<(), ae::Error> {
    win_dbg_logger::DEBUGGER_LOGGER.set_force_log_without_debugger(true);
    AEGP_ID.get_or_init(|| {
        ae::aegp::suites::Utility::new()
            .ok()?
            .register_with_aegp(None, "JK Fill Erase")
            .ok()
    });
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

/// 塗りつぶしの範囲は画面の外まで続くことがあるので、要求された範囲に関係なくレイヤー全体を取り出す。
/// まず空の範囲で問い合わせてレイヤー全体の範囲を知り、その全体を取り出す
fn smart_pre_render(in_data: &InData, mut extra: ae::PreRenderExtra) -> Result<(), ae::Error> {
    let req = extra.output_request();
    let mut probe = req;
    probe.rect.left = 0;
    probe.rect.top = 0;
    probe.rect.right = 0;
    probe.rect.bottom = 0;
    let (t, step, scale) = (
        in_data.current_time(),
        in_data.time_step(),
        in_data.time_scale(),
    );
    let cb = extra.callbacks();
    let full_rect = cb
        .checkout_layer(0, PROBE, &probe, t, step, scale)
        .map(|info| info.max_result_rect)
        .ok();
    let mut full = req;
    if let Some(r) = full_rect {
        full.rect = r;
    }
    if let Ok(in_result) = cb.checkout_layer(0, INPUT as i32, &full, t, step, scale) {
        // 読むのはレイヤー全体だが、出力は頼まれた範囲だけ作る（拡大表示や一部だけの描画で書き戻しが減る）
        let r = in_result.result_rect;
        let q = req.rect;
        let mut out = r;
        out.left = r.left.max(q.left);
        out.top = r.top.max(q.top);
        out.right = r.right.min(q.right);
        out.bottom = r.bottom.min(q.bottom);
        if out.left >= out.right || out.top >= out.bottom {
            out.right = out.left;
            out.bottom = out.top;
        }
        let _ = extra.union_result_rect(out.into());
        let _ = extra.union_max_result_rect(in_result.max_result_rect.into());
    }
    Ok(())
}

fn layout(in_data: &InData, layer: &ae::Layer) -> Layout {
    if in_data.is_premiere() {
        match layer.pr_pixel_format() {
            Ok(ae::pr::PixelFormat::Bgra4444_16u) => Layout::Bgra16,
            Ok(ae::pr::PixelFormat::Bgra4444_32f) => Layout::BgraF32,
            _ => Layout::Bgra8,
        }
    } else {
        match layer.world_type() {
            ae::aegp::WorldType::U8 => Layout::Argb8,
            ae::aegp::WorldType::U15 => Layout::Argb16,
            _ => Layout::ArgbF32,
        }
    }
}

fn rows_of(layer: &ae::Layer) -> Rows {
    Rows {
        stride: layer.buffer_stride(),
        flip: layer.row_bytes() < 0,
        h: layer.height(),
    }
}

/// 縮小表示の倍率（1/2 画質なら 0.5）
fn downsample(in_data: &InData) -> (f32, f32) {
    let ok = |v: f32| if v.is_finite() && v > 0.0 { v } else { 1.0 };
    (
        ok(in_data.downsample_x().into()),
        ok(in_data.downsample_y().into()),
    )
}

fn read_settings(
    in_data: &InData,
    params: &ae::Parameters<Params>,
    origin: (i32, i32),
) -> Result<Settings, Error> {
    let (sx, sy) = downsample(in_data);
    let fs =
        |p: Params| -> Result<f32, Error> { Ok(params.get(p)?.as_float_slider()?.value() as f32) };
    let checked = |p: Params| -> Result<bool, Error> { Ok(params.get(p)?.as_checkbox()?.value()) };

    let mut points = Vec::new();
    for i in 0..point_count(params)? as u8 {
        // 点はレイヤーの座標（フル解像度）で届く。ワールドの (0, 0) はレイヤーの origin にある
        let (x, y) = params.get(Params::Point(i))?.as_point()?.value();
        points.push((x * sx - origin.0 as f32, y * sy - origin.1 as f32));
    }
    Ok(Settings {
        points,
        tolerance: fs(Params::Tolerance)? / 100.0,
        softness: fs(Params::Softness)? / 100.0,
        choke: fs(Params::Choke)? * sx.max(sy),
        contiguous: checked(Params::Contiguous)?,
        compare_alpha: checked(Params::CompareAlpha)?,
        invert: checked(Params::Invert)?,
    })
}

fn do_render(
    in_data: &InData,
    input: &ae::Layer,
    output: &mut ae::Layer,
    params: &ae::Parameters<Params>,
) -> Result<(), Error> {
    let (iw, ih) = (input.width(), input.height());
    let in_origin = input.origin();
    let settings = read_settings(in_data, params, (in_origin.h, in_origin.v))?;
    let in_layout = layout(in_data, input);

    let (ow, oh) = (output.width(), output.height());
    if ow == 0 || oh == 0 {
        return Ok(());
    }
    let out_origin = output.origin();
    let dx = out_origin.h - in_origin.h;
    let dy = out_origin.v - in_origin.v;
    let out_layout = layout(in_data, output);

    // 空の入力のバッファは取り出せない（全部透明になる）
    let buf = if iw == 0 || ih == 0 {
        &[][..]
    } else {
        input.buffer()
    };
    let source = Source {
        buf,
        rows: rows_of(input),
        w: iw,
        layout: in_layout,
    };
    let out_rows = rows_of(output);
    frame::render(
        &source,
        output.buffer_mut(),
        out_rows,
        ow,
        out_layout,
        dx,
        dy,
        &settings,
    );
    Ok(())
}
