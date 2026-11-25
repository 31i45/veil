//! 运行时界面语言（Task 2 / Task 6）：启动优先级 config 手动选择 > `VEIL_UI_LANG` > 系统 UI 语言。
//!
//! 只映射当前已支持的两种语言：中文（任意地区变体）→ zh-CN，其余非零主语言 → en-US。
//! 探测失败（Win32 返回 0 / 非 Windows 平台）一律回退 zh-CN，保证界面始终有词条。

/// LANGID 主语言 ID 的位宽（`LANG_PRIMARYLANGUAGE`，低 10 位）。
const PRIMARY_LANGUAGE_MASK: u16 = 0x03ff;

/// LANGID 主语言 ID：中文（`LANG_CHINESE`）。
const LANG_CHINESE: u16 = 0x04;

/// LANGID 主语言 ID：英文（`LANG_ENGLISH`）。
const LANG_ENGLISH: u16 = 0x09;

/// 语言 ID → 支持的语言码：中文（含 0x0804/0x0404 等地区变体）→ "zh-CN"；
/// 英文 → "en-US"；其他非零主语言 → "en-US"；0（语言中性/探测失败）→ "zh-CN"。
pub fn resolve_langid(langid: u16) -> &'static str {
    match langid & PRIMARY_LANGUAGE_MASK {
        0 => "zh-CN",
        LANG_CHINESE => "zh-CN",
        LANG_ENGLISH => "en-US",
        _ => "en-US",
    }
}

/// 启动语言决策（纯函数，便于单测）：环境变量严格等于支持的语言码时优先，否则用系统语言。
pub fn resolve_locale(env_value: Option<&str>, langid: u16) -> &'static str {
    match env_value {
        Some("zh-CN") => "zh-CN",
        Some("en-US") => "en-US",
        _ => resolve_langid(langid),
    }
}

/// 完整启动优先级（纯函数）：配置里的手动选择 > 环境变量 > 系统 UI 语言。
///
/// `config_value` 为配置文件里的 `ui_language` 原始字符串：`"zh-CN"`/`"en-US"` 直接生效；
/// `"auto"`、缺失或非法值都按自动处理（不阻断启动），继续看环境变量与系统语言。
pub fn resolve_startup_language(
    config_value: Option<&str>,
    env_value: Option<&str>,
    langid: u16,
) -> &'static str {
    match config_value {
        Some("zh-CN") => "zh-CN",
        Some("en-US") => "en-US",
        _ => resolve_locale(env_value, langid),
    }
}

/// 系统默认 UI 语言的 LANGID（失败返回 0，由 `resolve_langid` 归为 zh-CN）。
#[cfg(windows)]
pub(crate) fn system_langid() -> u16 {
    // SAFETY: GetUserDefaultUILanguage 无参数，只返回 LANGID；无指针/所有权约定。
    unsafe { GetUserDefaultUILanguage() }
}

#[cfg(not(windows))]
pub(crate) fn system_langid() -> u16 {
    0
}

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn GetUserDefaultUILanguage() -> u16;
}
#[cfg(test)]
mod tests {
    use super::*;

    /// LANGID 只看主语言 ID：中文地区变体归一为 zh-CN，其他语言兜底 en-US。
    #[test]
    fn langid_maps_primary_language() {
        assert_eq!(resolve_langid(0x0804), "zh-CN", "简体中文");
        assert_eq!(resolve_langid(0x0409), "en-US", "美国英语");
        assert_eq!(resolve_langid(0x0411), "en-US", "日语 → 兜底英文");
        assert_eq!(resolve_langid(0), "zh-CN", "语言中性/探测失败");
        assert_eq!(resolve_langid(0x7fff), "en-US", "未知语言 → 兜底英文");
        assert_eq!(resolve_langid(0x0404), "zh-CN", "繁体中文同样归为中文");
    }

    /// 环境变量严格匹配才生效；大小写/空白/未知值都视为未设置，回落到系统语言。
    #[test]
    fn env_override_is_strict() {
        assert_eq!(resolve_locale(Some("zh-CN"), 0x0409), "zh-CN");
        assert_eq!(resolve_locale(Some("en-US"), 0x0804), "en-US");
        assert_eq!(
            resolve_locale(Some("en-us"), 0x0409),
            "en-US",
            "大小写不忽略"
        );
        assert_eq!(
            resolve_locale(Some("en-US "), 0x0804),
            "zh-CN",
            "空白不裁剪"
        );
        assert_eq!(resolve_locale(Some(""), 0x0409), "en-US", "空串不算覆盖");
        assert_eq!(resolve_locale(None, 0x0804), "zh-CN");
        assert_eq!(resolve_locale(None, 0x0409), "en-US");
    }

    /// 真实探测路径只产出受支持的语言码（失败也回退 zh-CN）。
    #[cfg(windows)]
    #[test]
    fn system_langid_yields_supported_code() {
        assert!(matches!(resolve_langid(system_langid()), "zh-CN" | "en-US"));
    }

    /// 启动优先级：配置手动值 > 环境变量 > 系统语言；auto/非法/缺失都回落到后两者。
    #[test]
    fn startup_language_prefers_config_then_env_then_system() {
        // 1) 配置手动选择压过环境变量与系统语言。
        assert_eq!(
            resolve_startup_language(Some("zh-CN"), Some("en-US"), 0x0409),
            "zh-CN"
        );
        assert_eq!(
            resolve_startup_language(Some("en-US"), Some("zh-CN"), 0x0804),
            "en-US"
        );
        // 2) auto / 缺失 / 非法 → 环境变量（仍严格匹配）。
        for auto in [Some("auto"), None, Some("fr-FR"), Some("ZH-cn"), Some("")] {
            assert_eq!(
                resolve_startup_language(auto, Some("zh-CN"), 0x0409),
                "zh-CN",
                "非手动值时环境变量优先: {auto:?}"
            );
            assert_eq!(
                resolve_startup_language(auto, Some("en-US"), 0x0804),
                "en-US",
                "非手动值时环境变量优先: {auto:?}"
            );
        }
        // 3) 无有效环境变量 → 系统 UI 语言。
        assert_eq!(
            resolve_startup_language(Some("auto"), None, 0x0804),
            "zh-CN"
        );
        assert_eq!(
            resolve_startup_language(None, Some("en-us"), 0x0409),
            "en-US",
            "非法环境变量不生效，回落系统语言"
        );
        assert_eq!(
            resolve_startup_language(None, None, 0x0411),
            "en-US",
            "其他系统语言兜底英文"
        );
    }
}
