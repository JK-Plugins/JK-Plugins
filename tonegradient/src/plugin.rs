//! After Effects 向けの組み込み部分。
//! パラメータを読み、入力ワールドを f32 に変換してコア処理に渡し、結果を書き戻す。

use std::sync::{Arc, Mutex};

use after_effects as ae;

use crate::result_cache::{ChangedList, ResultCache};
use crate::world_io::{compose, dense_to_list, read_world, Changes, Layout, Rows, Source};

mod curve_ui;
use curve_ui::CurveData;

use crate::engine::{
    self, Curve, DotShape, GroupColor, Interpolation, LayerTexture, OutputMode, Prepared, Settings,
    Texture, TextureKind, TextureMode, TextureParams,
};

/// 距離変換まで済ませた途中結果のキャッシュ（新しい順）。
/// テクスチャ・Amount・Curve などだけを変えたときは、重い距離変換を飛ばせる。
/// 複数フレームを同時にレンダリングするので、プラグイン全体で共有して Mutex で守る。
static CACHE: Mutex<Vec<(u64, Arc<Prepared>)>> = Mutex::new(Vec::new());
const CACHE_ENTRIES: usize = 4;
const CACHE_MAX_BYTES: usize = 512 << 20;

/// 色の合成まで済ませた結果のキャッシュ。同じ絵が続くフレーム（2コマ・3コマ打ち）では
/// 合成も float への変換も飛ばして、覚えておいた結果をそのまま書き込む
static RESULTS: ResultCache = ResultCache::new(8, 512 << 20);

fn prepared(key: u64, w: usize, h: usize, src: &[[f32; 4]], s: &Settings) -> Arc<Prepared> {
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

/// 距離変換と色の合成をして、`write` で出力に書き込む。
/// GPU を選んでいて使えるときは GPU（読み戻した全画素をそのまま書き込む）、
/// 使えない・失敗したときは CPU（変わった画素の並びを書き込む）
/// 入力の画素 `src` は必要なとき（距離変換をやり直すとき・CPU で合成するとき）だけ読む。
/// `key` は入力の画素と設定から作ったキャッシュのキー。
/// 戻り値は結果のキャッシュに入れる「変わった画素の並び」（GPU の全画素は `input` のアルファで並びにする）
#[allow(clippy::too_many_arguments)]
fn render_into(
    use_gpu: bool,
    w: usize,
    h: usize,
    src: impl Fn() -> Vec<[f32; 4]>,
    key: u64,
    s: &Settings,
    texture: &Texture,
    input: &Source,
    mut write: impl FnMut(Changes),
) -> ChangedList {
    if use_gpu {
        let done = engine::gpu::render_with(w, h, &src, key, s, texture, |px| match px {
            Some(px) => {
                write(Changes::Dense(px));
                dense_to_list(px, input)
            }
            None => {
                write(Changes::List(&[]));
                Vec::new()
            }
        });
        if let Ok((list, _)) = done {
            return list;
        }
    }
    let src = src();
    let p = prepared(key, w, h, &src, s);
    let out = engine::shade(&p, &src, s, texture);
    write(Changes::List(&out.changed));
    out.changed
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
    StopAtLines,
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
    Interpolation,
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

    // --- 計算に使うもの ---
    Render,
}

const TEXTURE_OPTIONS: [&str; 7] = ["Smooth", "Dot", "Line", "Bayer", "Grain", "Noise", "Layer"];
const DOT_OPTIONS: [&str; 5] = ["Round", "Circle", "Ellipse", "Square", "Diamond"];
/// 取り込んだもののライセンス表示（.aex だけを配るときも中に入るように）
#[used]
static THIRD_PARTY_NOTICES: &str = include_str!("../THIRD_PARTY_NOTICES.txt");

const INTERP_OPTIONS: [&str; 4] = ["Classic", "Linear", "Perceptual", "Pigment"];
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
        // 線（グループの色以外）や透明なところをまたいで混ぜない
        params.add(Params::StopAtLines, "Stop at Lines", checkbox(true))?;
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
                // 色の混ぜ方。Classic は従来どおり（前のプロジェクトの見た目を変えない）
                params.add(
                    Params::Interpolation,
                    "Interpolation",
                    popup(&INTERP_OPTIONS, 1),
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

        // GPU で色を合成する（使えないときは自動で CPU）。結果は同じ
        params.add(Params::Render, "Render", popup(&["GPU", "CPU"], 1))?;

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
                    "JK Tone Gradient v{}\rGradients between grouped colors with halftone / dither / texture.\rhttps://github.com/JK-Plugins/JK-Plugins\r\rPigment: Spectral.js, Copyright (c) 2025 Ronald van Wijnen (MIT License)",
                    env!("CARGO_PKG_VERSION")
                ));
            }
            ae::Command::GlobalSetup => {
                self.global_setup(&in_data)?;
            }
            ae::Command::QueryDynamicFlags => {
                // Texture が Layer のときだけ「パラメータが同じでも結果が変わる」ことにする。
                // テクスチャのレイヤーの位置・スケール・回転はパラメータではないので、
                // そうしないと AE は前の結果を使い回し、レイヤーを動かしても描き直さない
                // （このときのパラメータの配列は正しくないので checkout で読む）
                let layer = params
                    .checkout(Params::TextureType)
                    .ok()
                    .and_then(|p| p.as_popup().ok().map(|p| p.value()))
                    .map_or(true, |v| texture_kind(v) == TextureKind::Layer);
                out_data.set_out_flag(ae::OutFlags::NonParamVary, layer);
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

/// Mid Color を使うか。しきい値（Threshold）で2値に分けるときは色が中間にならないので使わない。
/// テクスチャなし（Smooth）か Map のときだけ使う
fn mid_color_used(kind: TextureKind, map_mode: bool) -> bool {
    kind == TextureKind::Smooth || map_mode
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
        // Layer はテクスチャのレイヤーの位置・スケール・回転に合わせるので、大きさ・位置は使わない
        (Params::TexSize, tex_on && kind != K::Layer),
        (Params::Angle, is(&[K::Dot, K::Line, K::Noise])),
        (Params::TexPosition, tex_on && kind != K::Layer),
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
    // Mid Color はグラデーションが2値にならないとき（Smooth か Map）だけ
    let mid_on = mid_color_used(kind, params.get(Params::TexMode)?.as_popup()?.value() == 2);
    for p in [Params::MidColor, Params::MidAmount, Params::MidWidth] {
        wanted.push((p, mid_on));
    }
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
    /// GPU で合成する
    use_gpu: bool,
}

/// AEGP（レイヤーの位置などを読む AE の機能）を使うための ID。GlobalSetup で一度だけ登録する
static AEGP_ID: std::sync::OnceLock<Option<ae::aegp::PluginId>> = std::sync::OnceLock::new();

/// テクスチャのレイヤーの位置・スケール・回転を、エフェクトのレイヤーから見た変換にする（フル解像度の座標）。
/// エフェクトのレイヤーの座標 (x, y) → テクスチャのレイヤーの座標 `(a·x + b·y + c, d·x + e·y + f)` の
/// `[a, b, c, d, e, f]`。親レイヤーの分も含む。2D のみ（Z は無視）。読めないときは None
fn texture_layer_xform(in_data: &InData, tex_param_index: i32) -> Option<[f64; 6]> {
    use ae::aegp::suites::{Layer, PFInterface, Stream};
    let plugin_id = (*AEGP_ID.get()?)?;
    let pfi = PFInterface::new().ok()?;
    let layers = Layer::new().ok()?;
    let streams = Stream::new().ok()?;
    let effect_ref = in_data.effect_ref();
    let comp_time = pfi
        .convert_effect_to_comp_time(effect_ref, in_data.current_time(), in_data.time_scale())
        .ok()?;
    // パラメータで選んだレイヤーの ID
    let effect = pfi.new_effect_for_effect(effect_ref, plugin_id).ok()?;
    let stream = streams
        .new_effect_stream_by_index(&effect, plugin_id, tex_param_index)
        .ok()?;
    let ae::aegp::StreamValue::LayerId(id) = streams
        .new_stream_value(
            &stream,
            plugin_id,
            ae::aegp::TimeMode::CompTime,
            comp_time,
            false,
        )
        .ok()?
    else {
        return None;
    };
    if id == 0 {
        return None;
    }
    let eff_layer = pfi.effect_layer(effect_ref).ok()?;
    let comp = layers.layer_parent_comp(&eff_layer).ok()?;
    let tex_layer = layers.layer_from_layer_id(&comp, id as u32).ok()?;
    let e = layers.layer_to_world_xform(&eff_layer, comp_time).ok()?;
    let t = layers.layer_to_world_xform(&tex_layer, comp_time).ok()?;
    compose_xform(&<[f64; 16]>::from(e), &<[f64; 16]>::from(t))
}

/// AE の 4×4 行列（行ベクトル：world = [x y z 1]·M、平行移動は4行目）2つから、
/// 「エフェクトのレイヤー → ワールド → テクスチャのレイヤー」の 2D アフィン変換を作る
fn compose_xform(e: &[f64; 16], t: &[f64; 16]) -> Option<[f64; 6]> {
    // world = A·p + T（列ベクトルの形に直す）
    let lin = |m: &[f64; 16]| ([m[0], m[4], m[1], m[5]], [m[12], m[13]]);
    let (ae_, te) = lin(e);
    let (at, tt) = lin(t);
    let det = at[0] * at[3] - at[1] * at[2];
    if !det.is_finite() || det.abs() < 1e-12 {
        return None;
    }
    let inv = [at[3] / det, -at[1] / det, -at[2] / det, at[0] / det];
    // tex = inv·(Ae·p + Te − Tt)
    let l = [
        inv[0] * ae_[0] + inv[1] * ae_[2],
        inv[0] * ae_[1] + inv[1] * ae_[3],
        inv[2] * ae_[0] + inv[3] * ae_[2],
        inv[2] * ae_[1] + inv[3] * ae_[3],
    ];
    let d = [te[0] - tt[0], te[1] - tt[1]];
    let r = [
        l[0],
        l[1],
        inv[0] * d[0] + inv[1] * d[1],
        l[2],
        l[3],
        inv[2] * d[0] + inv[3] * d[1],
    ];
    r.iter().all(|v| v.is_finite()).then_some(r)
}

/// 2つの範囲の重なり（重ならなければ大きさ 0）
fn clip_rect(r: ae::sys::PF_LRect, q: ae::sys::PF_LRect) -> ae::sys::PF_LRect {
    let mut out = r;
    out.left = r.left.max(q.left);
    out.top = r.top.max(q.top);
    out.right = r.right.min(q.right);
    out.bottom = r.bottom.min(q.bottom);
    if out.left >= out.right || out.top >= out.bottom {
        out.right = out.left;
        out.bottom = out.top;
    }
    out
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
    let map_mode = popup(Params::TexMode)? == 2;
    let mid_on = mid_color_used(texture_kind(popup(Params::TextureType)?), map_mode);
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
        texture_mode: if map_mode {
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
        mid_amount: if mid_on {
            fs(Params::MidAmount)? / 100.0
        } else {
            0.0
        },
        mid_width: fs(Params::MidWidth)? / 100.0,
        stop_at_lines: checked(Params::StopAtLines)?,
        interpolation: match popup(Params::Interpolation)? {
            2 => Interpolation::Linear,
            3 => Interpolation::Perceptual,
            4 => Interpolation::Pigment,
            _ => Interpolation::Classic,
        },
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
        use_gpu: popup(Params::Render)? == 1,
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
        // テクスチャのレイヤーの位置を読むために AEGP に登録する（Premiere などで使えなければ None）
        AEGP_ID.get_or_init(|| {
            ae::aegp::suites::Utility::new()
                .ok()?
                .register_with_aegp(None, "JK Tone Gradient")
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
        let asked = extra.output_request().rect;
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
            // 入力は広げて読むが、出力は頼まれた範囲だけ作る（はみ出した分を合成・書き込みしない）。
            // 出力と入力の原点のずれは do_render（compose の dx, dy）で合わせる
            let _ = extra.union_result_rect(clip_rect(in_result.result_rect, asked).into());
            let _ = extra.union_max_result_rect(in_result.max_result_rect.into());
        }

        // テクスチャのレイヤーを動かしたら描き直すよう、位置・スケール・回転をキャッシュのキーに混ぜる
        // （レイヤーパラメータの画素は変わらないので、AE はそれだけでは描き直さない）。
        // I_MIX_GUID_DEPENDENCIES を宣言しているので、Layer でないときも毎回呼ぶ必要がある
        let mut mix = [0.0f64; 6];
        if r.tex.kind == TextureKind::Layer {
            if let Some(index) = params.index(Params::TexLayer) {
                if let Some(m) = texture_layer_xform(in_data, index as i32) {
                    mix = m;
                }
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
        let _ = extra.callbacks().guid_mix_in_ptr(&mix);
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
            use_gpu,
        } = read_params(in_data, params)?;

        let (iw, ih) = (input.width(), input.height());
        let in_layout = layout(in_data, input);
        // キャッシュのキーは入力のバイト列から作る（キャッシュが効けば画素を float にしない）
        let key = {
            let rows = rows_of(input);
            let n = iw * in_layout.bytes_per_pixel();
            let buf = if iw == 0 || ih == 0 {
                &[][..]
            } else {
                input.buffer()
            };
            let lines: Vec<&[u8]> = (0..ih)
                .map(|y| &buf[rows.start(y)..rows.start(y) + n])
                .collect();
            engine::prepare_key_with(
                iw,
                ih,
                &settings,
                engine::hash_rows(&lines) ^ (in_layout as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15),
            )
        };
        let src = || world_to_f32(input, in_layout);
        let in_origin = input.origin();
        settings.origin = (in_origin.h, in_origin.v);

        // テクスチャレイヤー：位置合わせの変換は先に作り、画素はキャッシュが効かないときだけ float にする
        let tex_layer = match tex_world {
            Some(t) if tex.kind == TextureKind::Layer && t.width() > 0 && t.height() > 0 => {
                let o = t.origin();
                // テクスチャのレイヤーの位置・スケール・回転（読めなければエフェクトのレイヤーと同じ位置）
                let m = params
                    .index(Params::TexLayer)
                    .and_then(|i| texture_layer_xform(in_data, i as i32))
                    .unwrap_or([1.0, 0.0, 0.0, 0.0, 1.0, 0.0]);
                // 描画もテクスチャも同じ縮小率の座標：tex = S·A·(S⁻¹·p)（S = 縮小率）。
                // テクスチャの画素の座標は「ワールドの原点を引き、画素の中心を整数にする」
                let (sx, sy): (f32, f32) =
                    (in_data.downsample_x().into(), in_data.downsample_y().into());
                let xform = [
                    m[0] as f32,
                    (m[1] * sx as f64 / sy as f64) as f32,
                    (m[2] * sx as f64) as f32 - o.h as f32 - 0.5,
                    (m[3] * sy as f64 / sx as f64) as f32,
                    m[4] as f32,
                    (m[5] * sy as f64) as f32 - o.v as f32 - 0.5,
                ];
                Some((t, layout(in_data, t), xform))
            }
            _ => None,
        };

        // 合成の結果のキー：入力の画素・すべての設定・テクスチャのレイヤーの画素と変換・縮小率・GPU/CPU
        let result_key = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            downsample(in_data).to_bits().hash(&mut h);
            use_gpu.hash(&mut h);
            if let Some((t, tl, xform)) = &tex_layer {
                let rows = rows_of(t);
                let n = t.width() * tl.bytes_per_pixel();
                let buf = t.buffer();
                let lines: Vec<&[u8]> = (0..t.height())
                    .map(|y| &buf[rows.start(y)..rows.start(y) + n])
                    .collect();
                engine::hash_rows(&lines).hash(&mut h);
                (t.width(), t.height(), *tl as u8, layer_channel).hash(&mut h);
                xform.map(f32::to_bits).hash(&mut h);
            }
            engine::shade_key(key, &settings, &tex, h.finish())
        };

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
        let out_buf = output.buffer_mut();

        // 前と同じ絵・同じ設定なら、覚えておいた結果を書き込むだけ
        if let Some(hit) = RESULTS.get(result_key) {
            compose(
                &source,
                out_buf,
                out_rows,
                ow,
                out_layout,
                dx,
                dy,
                Changes::List(&hit),
            );
            return Ok(());
        }

        // テクスチャレイヤー：選んだチャンネルだけ使う
        let tex_value;
        let layer_tex = match tex_layer {
            Some((t, tl, xform)) => {
                tex_value = world_to_f32(t, tl)
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
                    xform,
                })
            }
            None => None,
        };
        let texture = Texture::new(tex, layer_tex);

        let list = render_into(
            use_gpu,
            iw,
            ih,
            src,
            key,
            &settings,
            &texture,
            &source,
            |changes| compose(&source, out_buf, out_rows, ow, out_layout, dx, dy, changes),
        );
        RESULTS.put(result_key, Arc::new(list));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{clip_rect, compose_xform, mid_color_used, TextureKind};

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> after_effects::sys::PF_LRect {
        after_effects::sys::PF_LRect {
            left,
            top,
            right,
            bottom,
        }
    }

    /// 出力は「入力の範囲」と「頼まれた範囲」の重なりだけ
    #[test]
    fn output_rect_is_clipped_to_request() {
        let r = clip_rect(rect(-40, -40, 1000, 600), rect(100, 50, 400, 300));
        assert_eq!((r.left, r.top, r.right, r.bottom), (100, 50, 400, 300));
        let r = clip_rect(rect(0, 0, 200, 100), rect(150, 50, 400, 300));
        assert_eq!((r.left, r.top, r.right, r.bottom), (150, 50, 200, 100));
        // 重ならなければ大きさ 0
        let r = clip_rect(rect(0, 0, 10, 10), rect(20, 20, 30, 30));
        assert!(r.left >= r.right && r.top >= r.bottom);
    }

    /// AE の行列（行ベクトル、平行移動は4行目）
    fn m(a: f64, b: f64, c: f64, d: f64, tx: f64, ty: f64) -> [f64; 16] {
        // world.x = a·x + c·y + tx、world.y = b·x + d·y + ty
        [
            a, b, 0.0, 0.0, c, d, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, tx, ty, 0.0, 1.0,
        ]
    }

    fn apply(x: &[f64; 6], p: (f64, f64)) -> (f64, f64) {
        (
            x[0] * p.0 + x[1] * p.1 + x[2],
            x[3] * p.0 + x[4] * p.1 + x[5],
        )
    }

    #[test]
    fn mid_color_only_when_not_binary() {
        assert!(mid_color_used(TextureKind::Smooth, false));
        assert!(mid_color_used(TextureKind::Dot, true));
        assert!(!mid_color_used(TextureKind::Dot, false));
        assert!(!mid_color_used(TextureKind::Noise, false));
    }

    #[test]
    fn same_place_is_identity() {
        let x = compose_xform(
            &m(1.0, 0.0, 0.0, 1.0, 50.0, 20.0),
            &m(1.0, 0.0, 0.0, 1.0, 50.0, 20.0),
        )
        .unwrap();
        let p = apply(&x, (12.0, 34.0));
        assert!((p.0 - 12.0).abs() < 1e-9 && (p.1 - 34.0).abs() < 1e-9);
    }

    #[test]
    fn moved_scaled_rotated_texture_layer() {
        // エフェクトのレイヤーは (100, 0) に置かれている。テクスチャのレイヤーは2倍・90度回転して (10, 10) に
        let e = m(1.0, 0.0, 0.0, 1.0, 100.0, 0.0);
        let t = m(0.0, 2.0, -2.0, 0.0, 10.0, 10.0);
        let x = compose_xform(&e, &t).unwrap();
        // テクスチャのレイヤーの (3, 4) はワールドの (10 − 8, 10 + 6) = (2, 16) = エフェクトのレイヤーの (−98, 16)
        let p = apply(&x, (-98.0, 16.0));
        assert!(
            (p.0 - 3.0).abs() < 1e-9 && (p.1 - 4.0).abs() < 1e-9,
            "{p:?}"
        );
    }

    #[test]
    fn degenerate_scale_is_none() {
        assert!(compose_xform(
            &m(1.0, 0.0, 0.0, 1.0, 0.0, 0.0),
            &m(0.0, 0.0, 0.0, 0.0, 0.0, 0.0)
        )
        .is_none());
    }
}
