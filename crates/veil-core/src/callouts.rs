//! 自定义 Callout 配置（`%APPDATA%\Veil\callouts.json`）的纯解析层。
//!
//! 格式：对象数组，每项 `{ "name": "FIXME", "label": "待修复", "color": "#c00",
//! "icon": "!" }`。`name`/`label` 必填，`color`/`icon` 可省略（灰/`!`）。
//! 宽容策略：文件级语法错误 → 空表；单条非法（字段缺失/名字不合法/颜色非法）→
//! 静默跳过该条，不影响其余条目，也不产生任何 UI 噪音。

use crate::syntax::Callout;

/// 自定义 Callout 注册表（大小写不敏感；名统一小写）。
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct CalloutRegistry {
    entries: Vec<Callout>,
}

impl CalloutRegistry {
    /// 从 JSON 文本解析。任何非法内容都被静默忽略（返回可用条目）。
    pub fn from_json(text: &str) -> Self {
        let entries = Parser::new(text).parse_registry().unwrap_or_default();
        Self { entries }
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

/// 极简 JSON 读取器：只支持本配置需要的 `[ { "k": "v", ... }, ... ]` 子集。
struct Parser {
    b: Vec<char>,
    i: usize,
}

impl Parser {
    fn new(s: &str) -> Self {
        Self {
            b: s.chars().collect(),
            i: 0,
        }
    }

    fn peek(&self) -> Option<char> {
        self.b.get(self.i).copied()
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t' | '\r' | '\n')) {
            self.i += 1;
        }
    }

    fn eat(&mut self, c: char) -> bool {
        self.skip_ws();
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn parse_string(&mut self) -> Option<String> {
        self.skip_ws();
        if self.peek() != Some('"') {
            return None;
        }
        self.i += 1;
        let mut out = String::new();
        loop {
            let c = self.peek()?;
            self.i += 1;
            match c {
                '"' => return Some(out),
                '\\' => {
                    let e = self.peek()?;
                    self.i += 1;
                    match e {
                        '"' => out.push('"'),
                        '\\' => out.push('\\'),
                        '/' => out.push('/'),
                        'n' => out.push('\n'),
                        't' => out.push('\t'),
                        'r' => out.push('\r'),
                        'u' => {
                            let mut n = 0u32;
                            for _ in 0..4 {
                                let h = self.peek()?.to_digit(16)?;
                                self.i += 1;
                                n = n * 16 + h;
                            }
                            out.push(char::from_u32(n)?);
                        }
                        _ => return None,
                    }
                }
                c => out.push(c),
            }
        }
    }

    /// 跳过任意非字符串值的原值（数字/布尔/null/对象/数组），用于忽略未知键。
    fn skip_value(&mut self) -> Option<()> {
        self.skip_ws();
        match self.peek()? {
            '"' => {
                self.parse_string()?;
            }
            '{' | '[' => {
                let (open, close) = if self.peek() == Some('{') {
                    ('{', '}')
                } else {
                    ('[', ']')
                };
                self.i += 1;
                let mut depth = 1usize;
                while depth > 0 {
                    let c = self.peek()?;
                    self.i += 1;
                    match c {
                        '"' => {
                            self.i -= 1;
                            self.parse_string()?;
                        }
                        x if x == open => depth += 1,
                        x if x == close => depth -= 1,
                        _ => {}
                    }
                }
            }
            _ => {
                while !matches!(self.peek(), None | Some(',' | '}' | ']')) {
                    self.i += 1;
                }
            }
        }
        Some(())
    }

    fn parse_registry(&mut self) -> Option<Vec<Callout>> {
        if !self.eat('[') {
            return None;
        }
        let mut out = Vec::new();
        let mut after_comma = false;
        loop {
            self.skip_ws();
            if self.eat(']') {
                // 尾随逗号非法 → 整体判非法（静默为空）
                return (!after_comma).then_some(out);
            }
            let entry = self.parse_object()?;
            if let Some(c) = validate(entry) {
                out.push(c);
            }
            self.skip_ws();
            if self.eat(',') {
                after_comma = true;
                continue;
            }
            if self.eat(']') {
                return Some(out);
            }
            return None; // 语法错误：整体判非法，调用方取空表
        }
    }

    fn parse_object(&mut self) -> Option<Vec<(String, String)>> {
        if !self.eat('{') {
            return None;
        }
        let mut fields = Vec::new();
        loop {
            self.skip_ws();
            if self.eat('}') {
                return Some(fields);
            }
            let key = self.parse_string()?;
            if !self.eat(':') {
                return None;
            }
            // 只有字符串值参与配置；其它类型直接跳过（该项后续按缺字段处理）。
            self.skip_ws();
            if self.peek() == Some('"') {
                let val = self.parse_string()?;
                fields.push((key, val));
            } else {
                self.skip_value()?;
            }
            self.skip_ws();
            if self.eat(',') {
                continue;
            }
            if self.eat('}') {
                return Some(fields);
            }
            return None;
        }
    }
}

/// 单条校验：名字/标签必填，颜色解析失败则整条忽略；图标缺省 `!`。
fn validate(fields: Vec<(String, String)>) -> Option<Callout> {
    let mut name = None;
    let mut label = None;
    let mut color = None;
    let mut icon = None;
    for (k, v) in fields {
        match k.as_str() {
            "name" => name = Some(v),
            "label" => label = Some(v),
            "color" => color = Some(v),
            "icon" => icon = Some(v),
            _ => {}
        }
    }
    let name = name?;
    let name = name.trim();
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
        Some(v) => parse_hex(&v)?,
        None => [136, 136, 136],
    };
    let icon = icon.unwrap_or_else(|| "!".to_string());
    let icon = icon.trim();
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
}
