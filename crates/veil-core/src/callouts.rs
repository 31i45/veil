//! 自定义 Callout 配置（`%APPDATA%\Veil\callouts.json`）的纯解析层。
//!
//! 格式：对象数组，每项 `{ "name": "FIXME", "label": "待修复", "color": "#c00",
//! "icon": "!" }`。`name`/`label` 必填，`color`/`icon` 可省略（灰/`!`）。
//! 宽容策略：文件级语法错误 → 空表；单条非法（字段缺失/名字不合法/颜色非法）→
//! 静默跳过该条，不影响其余条目，也不产生任何 UI 噪音。
//!
//! 实现：整文件 `serde_json::from_str::<Value>` 确认数组后逐条目手动提取四个已知键的
//! 字符串值（键缺失或值非字符串都记为 None，与手写 parser 的"跳过非字符串值"一致），
//! 再送入 `validate` 做业务校验（CSS 类名字符集、颜色范围、label/icon 长度）。
//! 替换手写 Parser struct（~172 行）后消除代理对缺失、转义不全、错误不可定位三个缺陷。

use crate::syntax::Callout;

/// 自定义 Callout 注册表（大小写不敏感；名统一小写）。
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct CalloutRegistry {
    entries: Vec<Callout>,
}

impl CalloutRegistry {
    /// 从 JSON 文本解析。任何非法内容都被静默忽略（返回可用条目）。
    pub fn from_json(text: &str) -> Self {
        match serde_json::from_str::<serde_json::Value>(text) {
            Ok(serde_json::Value::Array(arr)) => {
                let entries = arr
                    .iter()
                    .filter_map(parse_entry)
                    .collect();
                Self { entries }
            }
            _ => Self::default(),
        }
    }

    /// 按名查找（大小写不敏感）。
    pub fn get(&self, name: &str) -> Option<&Callout> {
        let key = name.trim().to_ascii_lowercase();
        self.entries.iter().find(|c| c.name == key)
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entries(&self) -> &[Callout] {
        &self.entries
    }
}

/// 单条提取：从 Value 对象里取 name/label/color/icon 四个已知键的字符串值，
/// 未知键或值非字符串都忽略（保持宽容）。
fn parse_entry(item: &serde_json::Value) -> Option<Callout> {
    let obj = item.as_object()?;
    let name = obj.get("name").and_then(v_as_string);
    let label = obj.get("label").and_then(v_as_string);
    let color = obj.get("color").and_then(v_as_string);
    let icon = obj.get("icon").and_then(v_as_string);
    validate(name, label, color, icon)
}

/// Value → &str：只接受 String 类型；null/number/bool/array/object 全部返回 None。
fn v_as_string(v: &serde_json::Value) -> Option<&str> {
    v.as_str()
}

/// 单条业务校验：名字/标签必填，颜色解析失败则整条忽略；图标缺省 `!`。
/// 只负责校验规则，不负责 JSON 语法解析。
fn validate(
    name: Option<&str>,
    label: Option<&str>,
    color: Option<&str>,
    icon: Option<&str>,
) -> Option<Callout> {
    let name = name?.trim();
    // 名字进 CSS 类名与解析匹配：只接受字母/数字/下划线/短横线，且必须以字母开头。
    let mut chars = name.chars();
    let first = chars.next()?;
    if !first.is_ascii_alphabetic()
        || !chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return None;
    }
    let label = label?.trim().to_string();
    if label.is_empty() || label.chars().count() > 64 {
        return None;
    }
    let color = match color {
        Some(v) => parse_hex(v)?,
        None => [136, 136, 136],
    };
    let icon = icon.unwrap_or("!").trim();
    if icon.chars().count() > 4 {
        return None;
    }
    Some(Callout {
        name: name.to_ascii_lowercase(),
        label,
        color,
        icon: icon.to_string(),
    })
}

/// `#rgb` / `#rrggbb`（大小写不敏感）；其余格式视为非法。
fn parse_hex(v: &str) -> Option<[u8; 3]> {
    let hex = v.trim().strip_prefix('#')?;
    let digits: Vec<u8> = hex
        .bytes()
        .map(|b| (b as char).to_digit(16).map(|d| d as u8))
        .collect::<Option<_>>()?;
    match digits.len() {
        3 => Some([digits[0] * 17, digits[1] * 17, digits[2] * 17]),
        6 => Some([
            digits[0] * 16 + digits[1],
            digits[2] * 16 + digits[3],
            digits[4] * 16 + digits[5],
        ]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_entry_with_all_fields() {
        let r = CalloutRegistry::from_json(
            r##"[{"name":"FIXME","label":"待修复","color":"#c00","icon":"!"}]"##,
        );
        let c = r.get("fixme").expect("应解析出 fixme");
        assert_eq!(c.name, "fixme");
        assert_eq!(c.label, "待修复");
        assert_eq!(c.color, [204, 0, 0]);
        assert_eq!(c.icon, "!");
        assert!(r.get("FIXME").is_some(), "名字大小写不敏感");
    }

    #[test]
    fn optional_color_and_icon_default() {
        let r = CalloutRegistry::from_json(r#"[{"name":"todo","label":"待办"}]"#);
        let c = r.get("todo").unwrap();
        assert_eq!(c.color, [136, 136, 136]);
        assert_eq!(c.icon, "!");
    }

    #[test]
    fn invalid_entries_are_silently_skipped() {
        let json = r##"[
            {"name":"ok","label":"可用","color":"#0a0"},
            {"name":"","label":"空名"},
            {"name":"1bad","label":"数字开头"},
            {"name":"nocolor","label":"坏色","color":"red"},
            {"name":"noLabel"},
            {"label":"无名"},
            {"name":"ok2","label":"第二","color":"#123456","icon":"?","extra":123}
        ]"##;
        let r = CalloutRegistry::from_json(json);
        let names: Vec<&str> = r.entries().iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["ok", "ok2"], "只保留合法条目");
    }

    #[test]
    fn malformed_json_yields_empty_and_never_panics() {
        for bad in [
            "",
            "null",
            "{",
            "[",
            "[{]",
            r#"[{"name":"x","label":"y"}"#,
            r#"[{"name":1,"label":"y"}]"#,
            "[{\"name\":\"x\",\"label\":\"y\"},]",
        ] {
            let r = CalloutRegistry::from_json(bad);
            assert!(r.is_empty(), "非法配置应静默为空: {bad}");
        }
    }

    #[test]
    fn unknown_keys_and_escapes_are_tolerated() {
        let r = CalloutRegistry::from_json(
            r##"[{"name":"warn2","label":"注意\n换行","color":"#ABCDEF","icon":"▲","n":1,"b":true,"arr":[1,2]}]"##,
        );
        let c = r.get("warn2").unwrap();
        assert_eq!(c.label, "注意\n换行");
        assert_eq!(c.color, [0xab, 0xcd, 0xef]);
    }

    // — 新增 serde_json 特有的覆盖（手写 parser 无法处理） —

    #[test]
    fn emoji_and_non_bmp_characters_are_valid() {
        // 代理对 emoji 🎨 = U+1F3A8 = surrogate pair D83C DFA8
        let r = CalloutRegistry::from_json(
            r##"[{"name":"art","label":"艺术 🎨","icon":"🎨"}]"##,
        );
        let c = r.get("art").unwrap();
        assert_eq!(c.label, "艺术 🎨");
        assert_eq!(c.icon, "🎨");
    }

    #[test]
    fn standard_json_escapes_are_supported() {
        // 手写 parser 缺 \b 和 \f；serde_json 原生支持全部标准转义
        let r = CalloutRegistry::from_json(
            r##"[{"name":"esc","label":"退格\b换页\f引号\"反斜\\","icon":"\u0041\u0042"}]"##,
        );
        let c = r.get("esc").unwrap();
        assert_eq!(c.label, "退格\x08换页\x0c引号\"反斜\\");
        assert_eq!(c.icon, "AB");
    }

    #[test]
    fn control_characters_in_string_are_accepted() {
        // 手写 parser 原样 push(c) 无校验；serde_json 也接受（标准 JSON 允许 U+0000~U+001F）
        let r = CalloutRegistry::from_json(
            r##"[{"name":"ctrl","label":"制表\t水平","icon":"\t"}]"##,
        );
        let c = r.get("ctrl").unwrap();
        assert_eq!(c.label, "制表\t水平");
    }

    #[test]
    fn non_string_values_for_known_keys_are_ignored() {
        // name 是 number，不是 string → v_as_string 返回 None → validate name? 返回 None → 整条跳过
        let r = CalloutRegistry::from_json(
            r##"[{"name":123,"label":"数字名"},{"name":"ok","label":"ok","color":["not","a","string"]}]"##,
        );
        let names: Vec<&str> = r.entries().iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["ok"], "name 非 string 整条跳过，color 非 string 只 color 缺省");
        assert_eq!(r.get("ok").unwrap().color, [136, 136, 136]);
    }
}
