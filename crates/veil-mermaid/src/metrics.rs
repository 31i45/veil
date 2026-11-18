//! cosmic-text 文本度量：mermaid DOM shim 的 `__measureTextRaw` 桥接实现。
//!
//! 与正文排版同源（spec「度量一致」）：按序探测 [`MermaidTheme::families`]
//! 字体栈设置 sans-serif 兜底，图内文字测量与正文字体栈一致，中文节点框
//! 与文字不错位。FontSystem/Buffer 在 worker 线程内创建并持有，绝不跨线程
//! （FontSystem 非 Sync；mermaid 渲染全程钉在 worker 线程）。

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, Weight};

/// 度量器：一个 FontSystem + 复用 Buffer（mermaid 布局一次会触发数十次测量，
/// 复用避免反复分配字形缓冲）。
pub(crate) struct TextMeasurer {
    fs: FontSystem,
    buf: Buffer,
}

impl TextMeasurer {
    /// `font_size` 为初始度量字号（实际字号每次调用按传入值重设）。
    pub(crate) fn new(families: &'static [&'static str], font_size: f32) -> Self {
        let mut fs = FontSystem::new();
        {
            // 与 veil-ui `new_font_system` 同策略：按序探测候选族是否存在，
            // 存在则显式设为 sans-serif 兜底（cosmic 默认族在 Windows 上常缺失）。
            let db = fs.db_mut();
            let picked = families.iter().find(|name| {
                db.faces()
                    .any(|f| f.families.iter().any(|(fam, _)| fam == **name))
            });
            if let Some(family) = picked {
                db.set_sans_serif_family(*family);
            }
        }
        let size = clamp_size(font_size);
        let buf = Buffer::new(&mut fs, Metrics::new(size, size * 1.25));
        Self { fs, buf }
    }

    /// 返回 `"宽|高"`（向上取整像素）——shim 解析为 `{width, height}`。
    pub(crate) fn measure(
        &mut self,
        text: &str,
        font_size: f64,
        weight: f64,
        family: &str,
    ) -> String {
        let size = if font_size.is_finite() && font_size > 0.0 {
            font_size as f32
        } else {
            16.0
        };
        let line_height = size * 1.25;
        // Buffer::set_metrics 断言字号非 0：此处已钳制。
        self.buf.set_metrics(Metrics::new(size, line_height));
        // 不限宽：mermaid 自己按测量结果决定节点宽度。
        self.buf.set_size(None, None);
        let weight = Weight(weight.round().clamp(100.0, 900.0) as u16);
        let attrs = parse_family(family)
            .map_or_else(|| Attrs::new().weight(weight), |f| Attrs::new().family(f).weight(weight));
        self.buf.set_text(text, &attrs, Shaping::Advanced, None);
        self.buf.shape_until_scroll(&mut self.fs, false);
        let mut width = 0.0f32;
        let mut height = 0.0f32;
        for run in self.buf.layout_runs() {
            for g in run.glyphs {
                width = width.max(g.x + g.w);
            }
            height = height.max(run.line_top + run.line_height);
        }
        if height <= 0.0 {
            // 空文本：shim 侧约定高度 = 字号 × 1.25（与 spike 硬编码度量同口径）。
            height = line_height;
        }
        format!("{:.0}|{:.0}", width.ceil(), height.ceil())
    }
}

fn clamp_size(font_size: f32) -> f32 {
    if font_size.is_finite() && font_size > 0.0 {
        font_size
    } else {
        16.0
    }
}

/// CSS `font-family` 串 → 第一个具体族名；generic 族 / 解析失败返回 `None`
/// （走 FontSystem 默认回退链）。mermaid 把 themeVariables 的 `fontFamily`
/// 原串回传给度量桥，形如 `"Microsoft YaHei UI", "Segoe UI"`。
fn parse_family(spec: &str) -> Option<Family<'_>> {
    let first = if let Some(start) = spec.find('"') {
        let rest = &spec[start + 1..];
        let end = rest.find('"')?;
        &rest[..end]
    } else {
        spec.split(',').next()?.trim()
    };
    let name = first.trim();
    if name.is_empty() || name.len() > 64 {
        return None;
    }
    const GENERIC: [&str; 6] = ["sans-serif", "serif", "monospace", "cursive", "fantasy", "system-ui"];
    if GENERIC.iter().any(|g| g.eq_ignore_ascii_case(name)) {
        return None;
    }
    Some(Family::Name(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_family_picks_first_quoted_name() {
        let fam = parse_family(r#""Microsoft YaHei UI", "Segoe UI""#).unwrap();
        match fam {
            Family::Name(n) => assert_eq!(n, "Microsoft YaHei UI"),
            _ => panic!("应为具名族"),
        }
    }

    #[test]
    fn parse_family_rejects_generic_and_garbage() {
        assert!(parse_family("sans-serif").is_none());
        assert!(parse_family(r#""SERIF""#).is_none());
        assert!(parse_family("").is_none());
        assert!(parse_family(&"x".repeat(65)).is_none());
    }

    #[test]
    fn measure_empty_text_falls_back_to_line_height() {
        let mut m = TextMeasurer::new(&["Microsoft YaHei UI", "Segoe UI"], 16.0);
        let r = m.measure("", 16.0, 400.0, "");
        assert_eq!(r, "0|20");
    }

    #[test]
    fn measure_cjk_is_full_width() {
        let mut m = TextMeasurer::new(&["Microsoft YaHei UI", "Segoe UI"], 16.0);
        // 断言度量链路可用（宽高均为有限正值）；具体宽度取决于系统字体，
        // 无 CJK 字体的机器回退字形宽度不定，不做全角宽硬断言。
        let r = m.measure("获取数据", 16.0, 400.0, r#""Microsoft YaHei UI""#);
        let (w, h) = r.split_once('|').unwrap();
        let (w, h) = (w.parse::<f64>().unwrap(), h.parse::<f64>().unwrap());
        assert!(w > 0.0 && h > 0.0, "CJK 度量异常: {r}");
    }
}
