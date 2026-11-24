//! 运行时界面语言（Task 2）：系统 UI 语言探测 + `VEIL_UI_LANG` 覆盖。
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

/// 系统默认 UI 语言的 LANGID（失败返回 0，由 `resolve_langid` 归为 zh-CN）。
#[cfg(windows)]
fn system_langid() -> u16 {
    // SAFETY: GetUserDefaultUILanguage 无参数，只返回 LANGID；无指针/所有权约定。
    unsafe { GetUserDefaultUILanguage() }
}

#[cfg(not(windows))]
fn system_langid() -> u16 {
    0
}

/// 系统 UI 语言 → 语言码；非 Windows 平台固定回退 zh-CN。
pub fn detect_locale() -> &'static str {
    resolve_locale(None, system_langid())
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
    fn detect_locale_yields_supported_code() {
        assert!(matches!(detect_locale(), "zh-CN" | "en-US"));
    }
}
