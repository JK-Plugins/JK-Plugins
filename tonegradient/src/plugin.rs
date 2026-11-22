//! After Effects 向けの組み込み部分。
//! パラメータを読み、入力ワールドを f32 に変換してコア処理に渡し、結果を書き戻す。

use std::sync::{Arc, Mutex};

use after_effects as ae;

use crate::world_io::{compose, read_world, Layout, Rows, Source};

mod curve_ui;
use curve_ui::CurveData;

use crate::engine::{
    self, Curve, DotShape, GroupColor, LayerTexture, OutputMode, Prepared, Settings, Texture,
    TextureKind, TextureMode, TextureParams,
};

/// 距離変換まで済ませた途中結果のキャッシュ（新しい順）。
/// テクスチャ・Amount・Curve などだけを変えたときは、重い距離変換を飛ばせる。
/// 複数フレームを同時にレンダリングするので、プラグイン全体で共有して Mutex で守る。
static CACHE: Mutex<Vec<(u64, Arc<Prepared>)>> = Mutex::new(Vec::new());
const CACHE_ENTRIES: usize = 4;
const CACHE_MAX_BYTES: usize = 512 << 20;

fn prepared(w: usize, h: usize, src: &[[f32; 4]], s: &Settings) -> Arc<Prepared> {
    let key = engine::prepare_key(w, h, src, s);
    if let Ok(mut cache) = CACHE.lock() {
        if let Some(pos) = cache.iter().position(|(k, _)| *k == key) {
            let hit = cache.remove(pos);
            let p = hit.1.clone();
            cache.insert(0, hit);
            return p;
        }
    }
    // 計算中はロックを持たない（他のフレームを止めない）
    let p = Arc::new(engine::prepare(w, h, src, s));
    if let Ok(mut cache) = CACHE.lock() {
        cache.retain(|(k, _)| *k != key);
        cache.insert(0, (key, p.clone()));
        let mut bytes = 0;
        let mut keep = 0;
        for (_, e) in cache.iter() {
            bytes += e.memory_bytes();
            if keep >= CACHE_ENTRIES || (keep > 0 && bytes > CACHE_MAX_BYTES) {
                break;
            }
            keep += 1;
        }
        cache.truncate(keep);
    }
    p
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

/// 色スロットの数とグループ（UI上の折りたたみ）の大きさ
const NUM_SLOTS: u8 = engine::MAX_COLORS as u8;
const SLOTS_PER_GROUP: u8 = 8;

/// パラメータ。並び順 = エフェクトコントロールでの並び順（params_setup で追加する順）
#[derive(Eq, PartialEq, Hash, Clone, Copy, Debug)]
enum Params {
    // --- 全体（一番上） ---
    Output,
    Width,
    TextureType,
    TexLayer,
    TexMode,
    Amount,
    AnimateSeed,

    // --- Color Select ---
    SelStart,
    Tolerance,
    AlphaThreshold,
    ColorsStart(u8),
    ColorsEnd(u8),
    Enable(u8),
    Color(u8),
    DetailStart(u8),
    DetailEnd(u8),
    ColorTolerance(u8),
    Spread(u8),
    Grow(u8),
    SelEnd,

    // --- Gradient ---
    GradStart,
    Offset,
    Reverse,
    CurveType,
    CurveGraph,
    CurvePower,
    GradEnd,

    // --- Mid Color ---
    MidStart,
    MidColor,
    MidAmount,
    MidWidth,
    MidEnd,

    // --- Texture Settings ---
    TexStart,
    Softness,
    TexSize,
    Angle,
    TexPosition,
    DotShapeType,
    Aspect,
    Octaves,
    LayerChannel,
    Invert,
    Seed,
    AaSamples,
    TexEnd,

    // --- Roughness ---
    RoughStart,
    Roughness,
    RoughScale,
    RoughEnd,
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

/// 値を変えると他の項目のグレーアウトが変わるパラメータ（監視する）
const SUPERVISE: ae::ParamFlag = ae::ParamFlag::SUPERVISE;

impl AdobePluginGlobal for Plugin {
    fn can_load(_host_name: &str, _host_version: &str) -> bool {
        true
    }

    fn params_setup(
        &self,
        params: &mut ae::Parameters<Params>,
        in_data: InData,
        _: OutData,
    ) -> Result<(), Error> {
        // ===== 全体：よく触るものを一番上に =====
        params.add(
            Params::Output,
            "Output",
            popup(&["Result", "Gradient (check)"], 1),
        )?;
        // 境界の両側に半分ずつ広がる全体の幅
        params.add(
            Params::Width,
            "Width",
            float_slider(20.0, (0.0, 2000.0), (0.0, 200.0), 1),
        )?;
        params.add_with_flags(
            Params::TextureType,
            "Texture",
            popup(&TEXTURE_OPTIONS, 2),
            SUPERVISE,
            ae::ParamUIFlags::empty(),
        )?;
        // Texture が Layer のときに使うレイヤー（Texture のすぐ下で選べるように）
        params.add(
            Params::TexLayer,
            "Texture Layer",
            ae::LayerDef::setup(|_| {}),
        )?;
        // しきい値（網点のように2色に分かれる）／マップ（濃淡で連続的に混ぜる）
        params.add_with_flags(
            Params::TexMode,
            "Texture Mode",
            popup(&["Threshold", "Map"], 1),
            SUPERVISE,
            ae::ParamUIFlags::empty(),
        )?;
        params.add(Params::Amount, "Amount", percent_slider(100.0))?;
        // フレームごとに Seed を変える（砂目・ノイズ・荒らしが毎フレーム動く）
        params.add(Params::AnimateSeed, "Animate Seed", checkbox(false))?;

        // ===== Color Select：色を選ぶ =====
        params.add_group(
            Params::SelStart,
            Params::SelEnd,
            "Color Select",
            false,
            |params| {
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
                // 8色ずつ折りたたみ。2つ目以降は閉じておく
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
                                params.add_with_flags(
                                    Params::Enable(i),
                                    &format!("Use {n}"),
                                    checkbox(i < 2),
                                    SUPERVISE,
                                    ae::ParamUIFlags::empty(),
                                )?;
                                params.add(
                                    Params::Color(i),
                                    &format!("Color {n}"),
                                    ae::ColorDef::setup(|f| {
                                        f.set_default(default_color(i));
                                        f.set_value(f.default());
                                    }),
                                )?;
                                // 細かい調整は閉じたグループの中へ
                                params.add_group(
                                    Params::DetailStart(i),
                                    Params::DetailEnd(i),
                                    &format!("Color {n} Detail"),
                                    true,
                                    |params| {
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
                                        Ok(())
                                    },
                                )?;
                            }
                            Ok(())
                        },
                    )?;
                }
                Ok(())
            },
        )?;

        // ===== Gradient：グラデーションの形 =====
        params.add_group(
            Params::GradStart,
            Params::GradEnd,
            "Gradient",
            true,
            |params| {
                // ＋で番号の大きい色の側へ境界を進める
                params.add(
                    Params::Offset,
                    "Offset",
                    float_slider(0.0, (-1000.0, 1000.0), (-50.0, 50.0), 1),
                )?;
                params.add(Params::Reverse, "Reverse Order", checkbox(false))?;
                // Custom ではグラフで直接カーブを編集する
                params.add_with_flags(
                    Params::CurveType,
                    "Curve",
                    popup(&["Linear", "Smooth", "Custom"], 1),
                    SUPERVISE,
                    ae::ParamUIFlags::empty(),
                )?;
                params.add_customized(
                    Params::CurveGraph,
                    "Curve Graph",
                    ae::ArbitraryDef::setup(|f| {
                        let _ = f.set_default::<CurveData>(CurveData::default());
                    }),
                    |param| {
                        param.set_ui_flags(ae::ParamUIFlags::CONTROL);
                        param.set_ui_width(curve_ui::UI_WIDTH);
                        param.set_ui_height(curve_ui::UI_HEIGHT);
                        -1
                    },
                )?;
                params.add(
                    Params::CurvePower,
                    "Curve Power",
                    float_slider(1.0, (0.05, 20.0), (0.25, 4.0), 2),
                )?;
                Ok(())
            },
        )?;

        // ===== Mid Color：混ざっているところに乗せる色 =====
        params.add_group(
            Params::MidStart,
            Params::MidEnd,
            "Mid Color",
            true,
            |params| {
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
                // 0% で中間色なし
                params.add(Params::MidAmount, "Mid Amount", percent_slider(0.0))?;
                // 中間色の帯の太さ（100% で混ざっているところ全体、小さいほど真ん中に寄る）
                params.add(Params::MidWidth, "Mid Width", percent_slider(50.0))?;
                Ok(())
            },
        )?;

        // ===== Texture Settings：選んだテクスチャに関係する項目だけ有効 =====
        params.add_group(
            Params::TexStart,
            Params::TexEnd,
            "Texture Settings",
            true,
            |params| {
                // 網点や粒のふちをぼかす幅（px）。0 でくっきり
                params.add(
                    Params::Softness,
                    "Softness",
                    float_slider(0.0, (0.0, 100.0), (0.0, 10.0), 1),
                )?;
                // テクスチャの大きさ。8 が標準：Dot・Line の周期とノイズの粒は 8px、
                // Bayer・Grain の1粒は 1px、Layer は 100%
                params.add(
                    Params::TexSize,
                    "Texture Size",
                    float_slider(8.0, (0.5, 1000.0), (1.0, 64.0), 1),
                )?;
                params.add(
                    Params::Angle,
                    "Angle",
                    ae::AngleDef::setup(|f| {
                        f.set_default(45.0);
                        f.set_value(f.default());
                    }),
                )?;
                // テクスチャの基準の位置（コンポジションで点を動かすとテクスチャがずれる）
                params.add(
                    Params::TexPosition,
                    "Texture Position",
                    ae::PointDef::setup(|f| {
                        f.set_default((50.0, 50.0));
                        f.set_restrict_bounds(false);
                    }),
                )?;
                params.add_with_flags(
                    Params::DotShapeType,
                    "Dot Shape",
                    popup(&DOT_OPTIONS, 1),
                    SUPERVISE,
                    ae::ParamUIFlags::empty(),
                )?;
                params.add(
                    Params::Aspect,
                    "Ellipse Aspect",
                    float_slider(2.0, (0.05, 20.0), (0.25, 4.0), 2),
                )?;
                params.add(Params::Octaves, "Noise Detail", int_slider(3, 1, 6))?;
                params.add(
                    Params::LayerChannel,
                    "Layer Channel",
                    popup(&CHANNEL_OPTIONS, 1),
                )?;
                params.add(Params::Invert, "Invert", checkbox(false))?;
                params.add(Params::Seed, "Seed", int_slider(0, 0, 100000))?;
                // 初期値はアンチエイリアスなし。レイヤーの画質が「ドラフト」のときも自動でなし
                params.add(Params::AaSamples, "Antialias", popup(&AA_OPTIONS, 1))?;
                Ok(())
            },
        )?;

        // ===== Roughness：境界を荒らす =====
        params.add_group(
            Params::RoughStart,
            Params::RoughEnd,
            "Roughness",
            true,
            |params| {
                params.add(
                    Params::Roughness,
                    "Rough Amount",
                    float_slider(0.0, (0.0, 500.0), (0.0, 30.0), 1),
                )?;
                params.add(
                    Params::RoughScale,
                    "Rough Scale",
                    float_slider(20.0, (1.0, 2000.0), (2.0, 200.0), 1),
                )?;
                Ok(())
            },
        )?;

        // カーブのグラフ（Curve Graph）を描くためのカスタム UI
        in_data
            .interact()
            .register_ui(CustomUIInfo::new().events(ae::CustomEventFlags::EFFECT))?;

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
                    "JK Tone Gradient v{}\rGradients between grouped colors with halftone / dither / texture.\rhttps://github.com/JK-Plugins/JK-Plugins",
                    env!("CARGO_PKG_VERSION")
                ));
            }
            ae::Command::GlobalSetup => {
                self.global_setup(&in_data)?;
            }
            ae::Command::UpdateParamsUi => {
                // 失敗しても描画には影響しないので、エラーは返さない
                if let Err(e) = update_enabled_state(params) {
                    log::warn!("update_enabled_state: {e:?}");
                }
            }
            ae::Command::UserChangedParam { param_index } => {
                let p = params.type_at(param_index);
                if matches!(
                    p,
                    Params::Enable(_)
                        | Params::TextureType
                        | Params::DotShapeType
                        | Params::CurveType
                        | Params::TexMode
                ) {
                    if let Err(e) = update_enabled_state(params) {
                        log::warn!("update_enabled_state: {e:?}");
                    }
                    out_data.set_out_flag(ae::OutFlags::RefreshUi, true);
                }
            }
            ae::Command::Event { mut extra } => {
                // カーブのグラフ（失敗しても描画には影響しないのでエラーは返さない）
                let r = match extra.event() {
                    ae::Event::Draw(_) => curve_ui::draw(params, &mut extra),
                    ae::Event::Click(_) => curve_ui::click(params, &mut extra),
                    ae::Event::Drag(_) => curve_ui::drag(params, &mut extra),
                    _ => Ok(()),
                };
                if let Err(e) = r {
                    log::warn!("curve ui: {e:?}");
                }
            }
            ae::Command::ArbitraryCallback { mut extra } => {
                extra.dispatch::<CurveData, Params>(Params::CurveGraph)?;
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

/// 使っていない項目をグレーアウトする（値は残す）。
/// - `Use n` がオフのスロットの Color / Tolerance / Spread / Grow
/// - Texture が Smooth のときのテクスチャの項目と Amount
/// - 選んだテクスチャの種類に関係ない項目
fn update_enabled_state(params: &ae::Parameters<Params>) -> Result<(), Error> {
    let checked = |p: Params| -> Result<bool, Error> { Ok(params.get(p)?.as_checkbox()?.value()) };
    let kind = texture_kind(params.get(Params::TextureType)?.as_popup()?.value());
    let shape = dot_shape(params.get(Params::DotShapeType)?.as_popup()?.value());
    let tex_on = kind != TextureKind::Smooth;
    let custom_curve = params.get(Params::CurveType)?.as_popup()?.value() == 3;

    use TextureKind as K;
    let is = |ks: &[TextureKind]| tex_on && ks.contains(&kind);
    let mut wanted: Vec<(Params, bool)> = vec![
        (Params::Amount, tex_on),
        (Params::TexMode, tex_on),
        (
            Params::Softness,
            tex_on && params.get(Params::TexMode)?.as_popup()?.value() == 1,
        ),
        (Params::TexSize, tex_on),
        (Params::Angle, is(&[K::Dot, K::Line, K::Noise])),
        (Params::TexPosition, tex_on),
        (Params::DotShapeType, is(&[K::Dot])),
        (Params::Aspect, is(&[K::Dot]) && shape == DotShape::Ellipse),
        (Params::Octaves, is(&[K::Noise])),
        (Params::TexLayer, is(&[K::Layer])),
        (Params::LayerChannel, is(&[K::Layer])),
        (Params::Invert, tex_on),
        (Params::AaSamples, tex_on),
        (Params::CurveGraph, custom_curve),
        (Params::CurvePower, !custom_curve),
    ];
    for i in 0..NUM_SLOTS {
        let on = checked(Params::Enable(i))?;
        for p in [
            Params::Color(i),
            Params::ColorTolerance(i),
            Params::Spread(i),
            Params::Grow(i),
        ] {
            wanted.push((p, on));
        }
    }

    // 読み取り専用で渡されるので、書き換えはコピーに対して行う
    let mut copy = params.cloned();
    for (p, enabled) in wanted {
        let mut def = copy.get_mut(p)?;
        let disabled = !enabled;
        if def.ui_flags().contains(ae::ParamUIFlags::DISABLED) != disabled {
            def.set_ui_flag(ae::ParamUIFlags::DISABLED, disabled);
            def.update_param_ui()?;
        }
    }
    Ok(())
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
    let checked = |p: Params| -> Result<bool, Error> { Ok(params.get(p)?.as_checkbox()?.value()) };

    let mut colors = Vec::new();
    for i in 0..NUM_SLOTS {
        if checked(Params::Enable(i))? {
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

    let frame = if checked(Params::AnimateSeed)? {
        frame_index(in_data)
    } else {
        0
    };
    let mid = params.get(Params::MidColor)?.as_color()?.value();
    let aa = if in_data.quality() == ae::Quality::Lo {
        // レイヤーの画質が「ドラフト」のときはサブピクセルのサンプリングを省く（いちばん重い部分）
        1
    } else {
        match popup(Params::AaSamples)? {
            1 => 1,
            2 => 2,
            4 => 8,
            _ => 4,
        }
    };
    let seed = (int(Params::Seed)? as u32).wrapping_add(frame.wrapping_mul(104_729));
    let size = fs(Params::TexSize)?;

    let settings = Settings {
        colors,
        tolerance: fs(Params::Tolerance)? / 255.0,
        alpha_threshold: fs(Params::AlphaThreshold)? / 255.0,
        width: fs(Params::Width)? * ds,
        offset: fs(Params::Offset)? * ds,
        reverse: checked(Params::Reverse)?,
        curve: match popup(Params::CurveType)? {
            2 => Curve::Smooth,
            3 => Curve::Custom,
            _ => Curve::Linear,
        },
        curve_points: params
            .get(Params::CurveGraph)?
            .as_arbitrary()?
            .value::<CurveData>()
            .map(|d| d.points())
            .unwrap_or_default(),
        curve_power: fs(Params::CurvePower)?,
        amount: fs(Params::Amount)? / 100.0,
        texture_mode: if popup(Params::TexMode)? == 2 {
            TextureMode::Map
        } else {
            TextureMode::Threshold
        },
        softness: fs(Params::Softness)? * ds,
        aa_samples: aa,
        roughness: fs(Params::Roughness)? * ds,
        rough_scale: fs(Params::RoughScale)? * ds,
        rough_seed: seed ^ 0x2545_f491,
        mid_color: [
            mid.red as f32 / 255.0,
            mid.green as f32 / 255.0,
            mid.blue as f32 / 255.0,
        ],
        mid_amount: fs(Params::MidAmount)? / 100.0,
        mid_width: fs(Params::MidWidth)? / 100.0,
        output: if popup(Params::Output)? == 2 {
            OutputMode::Gradient
        } else {
            OutputMode::Result
        },
        origin: (0, 0),
        // Max Mix Colors（4）と Mid Position（Core）は固定
        ..Default::default()
    };
    let angle: f32 = params.get(Params::Angle)?.as_angle()?.value();
    let tex = TextureParams {
        kind: texture_kind(popup(Params::TextureType)?),
        // Texture Size 8 を標準に、種類ごとに大きさの意味を合わせる
        cell: size * ds,
        angle_deg: angle,
        seed,
        invert: checked(Params::Invert)?,
        offset: {
            // 点はレイヤーの座標（フル解像度）で届く
            let (x, y) = params.get(Params::TexPosition)?.as_point()?.value();
            (x * ds, y * ds)
        },
        dot_shape: dot_shape(popup(Params::DotShapeType)?),
        aspect: fs(Params::Aspect)?,
        dither_size: size / 8.0 * ds,
        octaves: int(Params::Octaves)?.max(1) as u32,
        // テクスチャレイヤーのワールドも同じだけ縮小されて届くので、ここでは ds を掛けない
        layer_scale: size / 8.0,
        ..Default::default()
    };
    Ok(Read {
        settings,
        tex,
        layer_channel: popup(Params::LayerChannel)?,
    })
}

/// ワールドの行の並び（rowbytes が負なら下から）
fn rows_of(layer: &ae::Layer) -> Rows {
    Rows {
        stride: layer.buffer_stride(),
        flip: layer.row_bytes() < 0,
        h: layer.height(),
    }
}

/// ワールドを RGBA f32（0..1）に変換
fn world_to_f32(layer: &ae::Layer, layout: Layout) -> Vec<[f32; 4]> {
    if layer.width() == 0 || layer.height() == 0 {
        return Vec::new();
    }
    read_world(layer.buffer(), rows_of(layer), layer.width(), layout)
}

/// テクスチャレイヤーの取り出しに使う ID（0 = 入力、1 = 範囲の確認用）
const TEX_CHECKOUT_PROBE: i32 = 1;
const TEX_CHECKOUT: i32 = 2;

/// レイヤーパラメータのレイヤーを、全体を1枚として取り出す予約をする。
/// まず空の範囲で問い合わせてレイヤー全体の範囲を知り、その全体を取り出す
fn checkout_whole_layer(
    in_data: &InData,
    extra: &mut ae::PreRenderExtra,
    req: &ae::sys::PF_RenderRequest,
    index: i32,
    probe_id: i32,
    id: i32,
) {
    let mut probe = *req;
    probe.rect.left = 0;
    probe.rect.top = 0;
    probe.rect.right = 0;
    probe.rect.bottom = 0;
    let cb = extra.callbacks();
    if let Ok(info) = cb.checkout_layer(
        index,
        probe_id,
        &probe,
        in_data.current_time(),
        in_data.time_step(),
        in_data.time_scale(),
    ) {
        let mut full = *req;
        full.rect = info.max_result_rect;
        let _ = cb.checkout_layer(
            index,
            id,
            &full,
            in_data.current_time(),
            in_data.time_step(),
            in_data.time_scale(),
        );
    }
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
        // 境界の向こう側まで見えるように、要求範囲をグラデーションの届く距離だけ広げる
        // （engine の処理と同じ値を使うので、部分描画でも端で結果が変わらない）
        let pad = engine::reach(&r.settings).ceil() as i32 + 2;
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
                // テクスチャはレイヤー全体を1枚としてタイル状に並べる
                checkout_whole_layer(
                    in_data,
                    &mut extra,
                    &req,
                    index as i32,
                    TEX_CHECKOUT_PROBE,
                    TEX_CHECKOUT,
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
            cb.checkout_layer_pixels(TEX_CHECKOUT as u32).ok().flatten()
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

        if tex_world.is_some() {
            let _ = cb.checkin_layer_pixels(TEX_CHECKOUT as u32);
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
        let in_layout = layout(in_data, input);
        let src = world_to_f32(input, in_layout);
        let in_origin = input.origin();
        settings.origin = (in_origin.h, in_origin.v);

        // テクスチャレイヤー：選んだチャンネルだけ使う
        let tex_value;
        let layer_tex = match tex_world {
            Some(t) if tex.kind == TextureKind::Layer => {
                tex_value = world_to_f32(t, layout(in_data, t))
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

        let prepared = prepared(iw, ih, &src, &settings);
        let result = engine::shade(&prepared, &src, &settings, &texture);
        drop(src);

        // 出力ワールドと入力ワールドの位置合わせ（原点の差）
        let out_origin = output.origin();
        let dx = out_origin.h - in_origin.h;
        let dy = out_origin.v - in_origin.v;
        let ow = output.width();
        if ow == 0 || output.height() == 0 {
            return Ok(());
        }
        let out_layout = layout(in_data, output);

        // 入力の重なる部分を写し（外は透明）、変わったピクセルだけ上書きする
        let source = Source {
            // 空の入力のバッファは取り出せない（全部透明になる）
            buf: if iw == 0 || ih == 0 {
                &[]
            } else {
                input.buffer()
            },
            rows: rows_of(input),
            w: iw,
            layout: in_layout,
        };
        let out_rows = rows_of(output);
        compose(
            &source,
            output.buffer_mut(),
            out_rows,
            ow,
            out_layout,
            dx,
            dy,
            &result.changed,
        );
        Ok(())
    }
}
