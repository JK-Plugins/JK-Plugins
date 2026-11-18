#[cfg(any(windows, target_os = "macos"))]
use pipl::*;

#[cfg(any(windows, target_os = "macos"))]
const PF_PLUG_IN_VERSION: u16 = 13;
#[cfg(any(windows, target_os = "macos"))]
const PF_PLUG_IN_SUBVERS: u16 = 28;

#[rustfmt::skip]
fn main() {
    // Linux などAE非対応環境ではPiPLを作らない（コアのテストだけ行う）
    #[cfg(any(windows, target_os = "macos"))]
    {
        const EFFECT_VERSION_MAJOR: u32 = 0;
        const EFFECT_VERSION_MINOR: u32 = 0;
        const EFFECT_VERSION_PATCH: u32 = 1;

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
                version: EFFECT_VERSION_MAJOR,
                subversion: EFFECT_VERSION_MINOR,
                bugversion: EFFECT_VERSION_PATCH,
                stage: Stage::Develop,
                build: 1,
            },
            Property::AE_Effect_Info_Flags(0),
            // 周囲のピクセルを参照するので PixIndependent は付けない
            Property::AE_Effect_Global_OutFlags(
                OutFlags::DeepColorAware
            ),
            Property::AE_Effect_Global_OutFlags_2(
                OutFlags2::FloatColorAware |
                OutFlags2::SupportsSmartRender |
                OutFlags2::SupportsThreadedRendering |
                OutFlags2::SupportsGetFlattenedSequenceData
            ),
            Property::AE_Effect_Match_Name(EFFECT_NAME),
            Property::AE_Reserved_Info(8),
            Property::AE_Effect_Support_URL("https://github.com/JK-Plugins"),
        ]);
    }
}
