#[cfg(any(windows, target_os = "macos"))]
use pipl::*;

#[cfg(any(windows, target_os = "macos"))]
const PF_PLUG_IN_VERSION: u16 = 13;
#[cfg(any(windows, target_os = "macos"))]
const PF_PLUG_IN_SUBVERS: u16 = 28;

/// Cargo.toml の version を AE のエフェクトバージョンに変換する。
/// AE_Effect_Version はビット幅が小さい（major 0-127 / minor 0-15 / patch 0-15）ので、
/// 範囲外ならビルドを止める。
#[cfg(any(windows, target_os = "macos"))]
fn effect_version() -> (u32, u32, u32) {
    let part = |name: &str, max: u32| -> u32 {
        let value: u32 = std::env::var(name)
            .unwrap_or_else(|_| panic!("{name} is not set"))
            .parse()
            .unwrap_or_else(|_| panic!("{name} is not a number"));
        assert!(
            value <= max,
            "{name}={value} exceeds the After Effects limit ({max}). Adjust version in Cargo.toml."
        );
        value
    };
    (
        part("CARGO_PKG_VERSION_MAJOR", 127),
        part("CARGO_PKG_VERSION_MINOR", 15),
        part("CARGO_PKG_VERSION_PATCH", 15),
    )
}

#[rustfmt::skip]
fn main() {
    // Linux などAE非対応環境ではPiPLを作らない（コアのテストだけ行う）
    #[cfg(any(windows, target_os = "macos"))]
    {
        // バージョンは Cargo.toml の version だけで管理する
        let (effect_version_major, effect_version_minor, effect_version_patch) = effect_version();

        const EFFECT_NAME: &str = "JK Tone Gradient";

        pipl::plugin_build(vec![
            Property::Kind(PIPLType::AEEffect),
            Property::Name(EFFECT_NAME),
            Property::Category("JK Plugins Original"),

            #[cfg(target_os = "windows")]
            Property::CodeWin64X86("EffectMain"),
            #[cfg(target_os = "macos")]
            Property::CodeMacIntel64("EffectMain"),
            #[cfg(target_os = "macos")]
            Property::CodeMacARM64("EffectMain"),

            Property::AE_PiPL_Version { major: 2, minor: 0 },
            Property::AE_Effect_Spec_Version { major: PF_PLUG_IN_VERSION, minor: PF_PLUG_IN_SUBVERS },
            Property::AE_Effect_Version {
                version: effect_version_major,
                subversion: effect_version_minor,
                bugversion: effect_version_patch,
                stage: Stage::Develop,
                build: 1,
            },
            Property::AE_Effect_Info_Flags(0),
            // 周囲のピクセルを参照するので PixIndependent は付けない
            Property::AE_Effect_Global_OutFlags(
                OutFlags::DeepColorAware |
                OutFlags::SendUpdateParamsUI |
                // Texture が Layer のときはテクスチャのレイヤーを動かすと結果が変わる（QueryDynamicFlags で切り替える）
                OutFlags::NonParamVary |
                // カーブのグラフを描く
                OutFlags::CustomUI
            ),
            Property::AE_Effect_Global_OutFlags_2(
                OutFlags2::FloatColorAware |
                OutFlags2::SupportsSmartRender |
                OutFlags2::SupportsQueryDynamicFlags |
                OutFlags2::SupportsThreadedRendering |
                OutFlags2::SupportsGetFlattenedSequenceData
            ),
            Property::AE_Effect_Match_Name(EFFECT_NAME),
            Property::AE_Reserved_Info(8),
            Property::AE_Effect_Support_URL("https://github.com/JK-Plugins"),
        ]);
    }
}
