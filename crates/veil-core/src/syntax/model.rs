//! 语法树纯数据模型：块 / 行内节点 / 表格派生数据。
//!
//! 本模块只含类型定义，无任何解析逻辑；块级扫描见 `block`、行内扫描见
//! `inline`、增量状态维护见父模块（Syntax / reparse）。

use crate::range::Range;

/// GFM Admonition 五种类型（`> [!NOTE]` / `> [!TIP]` / `> [!IMPORTANT]` /
/// `> [!WARNING]` / `> [!CAUTION]`）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AdmonitionType {
    Note,
    Tip,
    Important,
    Warning,
    Caution,
}

impl AdmonitionType {
    /// 解析变体名（大小写不敏感；`WARN` 为 `WARNING` 的别名）。
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_uppercase().as_str() {
            "NOTE" => Some(Self::Note),
            "TIP" => Some(Self::Tip),
            "IMPORTANT" => Some(Self::Important),
            "WARNING" | "WARN" => Some(Self::Warning),
            "CAUTION" => Some(Self::Caution),
            _ => None,
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            Self::Note => "Note",
            Self::Tip => "Tip",
            Self::Important => "Important",
            Self::Warning => "Warning",
            Self::Caution => "Caution",
        }
    }

    /// 小写类型名（导出 CSS 类 `callout-<name>` 与配置文件键共用）。
    pub fn name(&self) -> &'static str {
        match self {
            Self::Note => "note",
            Self::Tip => "tip",
            Self::Important => "important",
            Self::Warning => "warning",
            Self::Caution => "caution",
        }
    }

    /// 统一强调色（预览 / PDF / HTML 导出共用同一组值）。
    pub fn color(&self) -> [u8; 3] {
        match self {
            Self::Note => [9, 105, 218],
            Self::Tip => [26, 127, 55],
            Self::Important => [130, 80, 223],
            Self::Warning => [154, 103, 0],
            Self::Caution => [207, 34, 46],
        }
    }

    /// 统一图标（单字符/短文本，三通道同样使用）。
    pub fn icon(&self) -> &'static str {
        match self {
            Self::Note => "i",
            Self::Tip => "★",
            Self::Important => "!",
            Self::Warning => "▲",
            Self::Caution => "✖",
        }
    }
}

/// 自定义 Callout（`> [!NAME]`）：由 `callouts.json` 定义，解析期已解析成
/// 展示所需的全部字段，视图/导出不再查配置（渲染层零配置依赖）。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Callout {
    /// 规范化小写名（`fixme`），同时用于 CSS 类名 `callout-fixme`。
    pub name: String,
    /// 展示标签（`待修复`）。
    pub label: String,
    /// 强调色（rail / 标题 / 浅底色来源）。
    pub color: [u8; 3],
    /// 图标（短文本，默认 `!`）。
    pub icon: String,
}

/// 行内/块级白名单 HTML 标签。枚举成员命名即小写标签名。
/// 非白名单 HTML 在 inline 层退回字面文本、在 block 层进 HtmlBlock 代码卡。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HtmlTag {
    Kbd,
    Sub,
    Sup,
    Br,
    Mark,
    Abbr,
    Img,
    Code,
    Pre,
    A,
    Strong,
    B,
    Em,
    I,
    U,
    S,
    Del,
    Ins,
    Q,
    Cite,
    Var,
    Samp,
    Small,
    Span,
    Details,
    Summary,
}

impl HtmlTag {
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(name: &str) -> Option<Self> {
        Some(match name {
            "kbd" => Self::Kbd,
            "sub" => Self::Sub,
            "sup" => Self::Sup,
            "br" => Self::Br,
            "mark" => Self::Mark,
            "abbr" => Self::Abbr,
            "img" => Self::Img,
            "code" => Self::Code,
            "pre" => Self::Pre,
            "a" => Self::A,
            "strong" => Self::Strong,
            "b" => Self::B,
            "em" => Self::Em,
            "i" => Self::I,
            "u" => Self::U,
            "s" => Self::S,
            "del" => Self::Del,
            "ins" => Self::Ins,
            "q" => Self::Q,
            "cite" => Self::Cite,
            "var" => Self::Var,
            "samp" => Self::Samp,
            "small" => Self::Small,
            "span" => Self::Span,
            "details" => Self::Details,
            "summary" => Self::Summary,
            _ => return None,
        })
    }
    pub fn to_str(self) -> &'static str {
        match self {
            Self::Kbd => "kbd",
            Self::Sub => "sub",
            Self::Sup => "sup",
            Self::Br => "br",
            Self::Mark => "mark",
            Self::Abbr => "abbr",
            Self::Img => "img",
            Self::Code => "code",
            Self::Pre => "pre",
            Self::A => "a",
            Self::Strong => "strong",
            Self::B => "b",
            Self::Em => "em",
            Self::I => "i",
            Self::U => "u",
            Self::S => "s",
            Self::Del => "del",
            Self::Ins => "ins",
            Self::Q => "q",
            Self::Cite => "cite",
            Self::Var => "var",
            Self::Samp => "samp",
            Self::Small => "small",
            Self::Span => "span",
            Self::Details => "details",
            Self::Summary => "summary",
        }
    }
    /// 仅允许自闭合写法的标签（`<br />` / `<img />`）。
    pub fn is_void(self) -> bool {
        matches!(self, Self::Br | Self::Img)
    }
    /// 仅允许成对写法的标签（有开闭）。
    pub fn is_pair(self) -> bool {
        !self.is_void()
    }
}

/// 独立图片的水平对齐（`<img align="left|center|right">`）。
/// 仅对独占视觉行的图片生效；行内图片忽略该属性，避免打断行内排版。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ImageAlign {
    Left,
    Center,
    Right,
}

impl ImageAlign {
    /// 解析属性值（大小写不敏感，`centre` 作为 `center` 别名）；未知值返回 None。
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "left" => Some(Self::Left),
            "center" | "centre" => Some(Self::Center),
            "right" => Some(Self::Right),
            _ => None,
        }
    }

    /// 规范值（导出 HTML 的 figure 类名后缀与调试输出共用）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Center => "center",
            Self::Right => "right",
        }
    }
}

/// 白名单 HTML 属性（abbr.title / img.src / img.alt / img.width / img.height /
/// img.align / a.href）。其余属性一律忽略；
/// `script` 事件属性、`style`、`javascript:` href 解析层判定非法后置空，渲染层读不到。
/// 远程（http/https）图片地址不写入 `src`（仍为空），只保存在 `remote_src` 供 UI 渲染层按需加载。
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct HtmlAttrs {
    pub title: Option<String>,
    pub src: Option<String>,
    /// 仅保存 http/https 图片地址（原始值），供 UI 渲染层按需加载；
    /// `src` 仍为空，导出/PDF 读不到该地址，以保持单文件自包含。
    pub remote_src: Option<String>,
    pub alt: Option<String>,
    /// `<img width="...">` 的显式像素宽（仅十进制数字；非法值不收集）。
    /// 显式尺寸让作者覆盖编辑器默认的图片限高，故须与 src 一同进白名单。
    pub width: Option<u32>,
    /// `<img height="...">` 的显式像素高（口径同 `width`）。
    pub height: Option<u32>,
    /// `<img align="...">` 的水平对齐（仅 left/center/right；未知值不收集）。
    pub align: Option<ImageAlign>,
    /// `<a href="...">` 的目标（仅 a 标签收集；`javascript:` 等危险 scheme 置空）。
    pub href: Option<String>,
    /// `<details open>` 的布尔属性存在性（仅白名单 details 标签收集）。
    pub open: bool,
    /// 净化后的 `style="..."`（仅允许的颜色/字体/装饰声明，已丢弃危险与未知声明）。
    pub style: Option<String>,
}

// ===== style 属性安全白名单 =====

/// `style` 允许的属性名（小写）。未列出的属性名一律丢弃。
pub const STYLE_ALLOWED_PROPS: &[&str] = &[
    "color",
    "background-color",
    "font-weight",
    "font-style",
    "text-decoration",
    "font-size",
    "font-family",
];

/// 值中出现即整条声明丢弃的危险片段（宁可丢弃不可放行）。
const STYLE_FORBIDDEN_TOKENS: &[&str] = &[
    "url(",
    "expression",
    "javascript:",
    "behavior",
    "position",
    "z-index",
    "content",
];

/// 有限命名色白名单（小写）。
const STYLE_NAMED_COLORS: &[(&str, [u8; 3])] = &[
    ("black", [0, 0, 0]),
    ("white", [255, 255, 255]),
    ("red", [255, 0, 0]),
    ("green", [0, 128, 0]),
    ("blue", [0, 0, 255]),
    ("yellow", [255, 255, 0]),
    ("orange", [255, 165, 0]),
    ("purple", [128, 0, 128]),
    ("pink", [255, 192, 203]),
    ("brown", [165, 42, 42]),
    ("gray", [128, 128, 128]),
    ("grey", [128, 128, 128]),
    ("silver", [192, 192, 192]),
    ("maroon", [128, 0, 0]),
    ("olive", [128, 128, 0]),
    ("lime", [0, 255, 0]),
    ("aqua", [0, 255, 255]),
    ("cyan", [0, 255, 255]),
    ("teal", [0, 128, 128]),
    ("navy", [0, 0, 128]),
    ("fuchsia", [255, 0, 255]),
    ("magenta", [255, 0, 255]),
    ("gold", [255, 215, 0]),
    ("indigo", [75, 0, 130]),
    ("violet", [238, 130, 238]),
    ("darkred", [139, 0, 0]),
    ("darkgreen", [0, 100, 0]),
    ("darkblue", [0, 0, 139]),
    ("darkgray", [169, 169, 169]),
    ("darkgrey", [169, 169, 169]),
    ("lightgray", [211, 211, 211]),
    ("lightgrey", [211, 211, 211]),
    ("lightblue", [173, 216, 230]),
    ("lightgreen", [144, 238, 144]),
    ("crimson", [220, 20, 60]),
    ("tomato", [255, 99, 71]),
    ("salmon", [250, 128, 114]),
    ("khaki", [240, 230, 140]),
    ("beige", [245, 245, 220]),
    ("ivory", [255, 255, 240]),
];

/// `style` 解析结果：渲染三通道共用的结构化样式。
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct HtmlStyle {
    pub color: Option<[u8; 3]>,
    pub background_color: Option<[u8; 3]>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub overline: bool,
    /// 绝对字号（CSS px，允许区间 8–72）；渲染层按 16px 参考换算缩放。
    pub size_px: Option<f32>,
    /// 字体族含等宽关键词（mono/monospace/courier/consolas 等）。
    pub mono: bool,
}

impl HtmlStyle {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// 从（净化前或后的）style 文本解析：逐条声明独立校验，非法声明丢弃。
    pub fn parse(style: &str) -> Self {
        let mut out = Self::default();
        for decl in style.split(';') {
            let Some((prop, val)) = split_decl(decl) else {
                continue;
            };
            if !style_value_ok(prop, val) {
                continue;
            }
            match prop {
                "color" => out.color = parse_css_color(val),
                "background-color" => out.background_color = parse_css_color(val),
                "font-weight" => {
                    let v = val.to_ascii_lowercase();
                    if v == "bold" || v.parse::<u16>().is_ok_and(|n| n >= 600) {
                        out.bold = true;
                    }
                }
                "font-style" => out.italic = true,
                "text-decoration" => {
                    for t in val.split_whitespace() {
                        match t.to_ascii_lowercase().as_str() {
                            "underline" => out.underline = true,
                            "line-through" => out.strike = true,
                            "overline" => out.overline = true,
                            _ => {}
                        }
                    }
                }
                "font-size" => out.size_px = parse_font_size(val),
                "font-family" => {
                    let v = val.to_ascii_lowercase();
                    out.mono = ["mono", "courier", "consolas", "menlo", "monaco"]
                        .iter()
                        .any(|k| v.contains(k));
                }
                _ => {}
            }
        }
        out
    }
}

/// 净化 `style` 属性：只保留白名单属性与合法值，返回规范化 `prop:value; prop:value`。
/// 全部声明非法时返回 None（等价于没有 style）。
pub fn sanitize_style(raw: &str) -> Option<String> {
    let mut out: Vec<String> = Vec::new();
    for decl in raw.split(';') {
        let Some((prop, val)) = split_decl(decl) else {
            continue;
        };
        if !style_value_ok(prop, val) {
            continue;
        }
        out.push(format!("{prop}:{val}"));
    }
    (!out.is_empty()).then(|| out.join("; "))
}

/// 拆出 `(小写属性名, 去空白值)`；无冒号/空值/未知属性返回 None。
fn split_decl(decl: &str) -> Option<(&'static str, &str)> {
    let (prop, val) = decl.split_once(':')?;
    let prop = prop.trim().to_ascii_lowercase();
    let val = val.trim();
    if val.is_empty() {
        return None;
    }
    let allowed = STYLE_ALLOWED_PROPS.iter().copied().find(|p| *p == prop)?;
    Some((allowed, val))
}

/// 单条声明校验：属性已由 `split_decl` 限定，值按属性语义再校验。
fn style_value_ok(prop: &str, val: &str) -> bool {
    let lower = val.to_ascii_lowercase();
    if STYLE_FORBIDDEN_TOKENS.iter().any(|t| lower.contains(t)) {
        return false;
    }
    match prop {
        "color" | "background-color" => parse_css_color(val).is_some(),
        "font-weight" => {
            lower == "normal"
                || lower == "bold"
                || matches!(
                    lower.parse::<u16>(),
                    Ok(n @ 100..=900) if n % 100 == 0
                )
        }
        "font-style" => matches!(lower.as_str(), "normal" | "italic" | "oblique"),
        "text-decoration" => {
            let mut any = false;
            for t in lower.split_whitespace() {
                match t {
                    "none" | "underline" | "line-through" | "overline" => any = true,
                    _ => return false,
                }
            }
            any
        }
        "font-size" => parse_font_size(val).is_some(),
        "font-family" => {
            let n = val.chars().count();
            (1..=64).contains(&n)
                && val.chars().all(|c| {
                    c.is_ascii_alphanumeric()
                        || matches!(c, ' ' | ',' | '-' | '\'' | '"')
                        || !c.is_ascii()
                })
                && balanced_quotes(val)
        }
        _ => false,
    }
}

fn balanced_quotes(s: &str) -> bool {
    let mut single = 0usize;
    let mut double = 0usize;
    for c in s.chars() {
        match c {
            '\'' => single += 1,
            '"' => double += 1,
            _ => {}
        }
    }
    single.is_multiple_of(2) && double.is_multiple_of(2)
}

fn parse_font_size(val: &str) -> Option<f32> {
    let v = val.trim().to_ascii_lowercase();
    let num = v
        .strip_suffix("px")
        .or_else(|| v.strip_suffix("em"))
        .or_else(|| v.strip_suffix('%'))?;
    let n: f32 = num.trim().parse().ok()?;
    (8.0..=72.0).contains(&n).then_some(n)
}

/// CSS 颜色解析：`#rgb` / `#rrggbb` / `rgb()` / `rgba()` / `hsl()` / 有限命名色。
fn parse_css_color(val: &str) -> Option<[u8; 3]> {
    let v = val.trim().to_ascii_lowercase();
    if let Some(hex) = v.strip_prefix('#') {
        return parse_hex_color(hex);
    }
    if let Some(inner) = v.strip_prefix("rgba(").and_then(|s| s.strip_suffix(')')) {
        return parse_rgb_args(inner);
    }
    if let Some(inner) = v.strip_prefix("rgb(").and_then(|s| s.strip_suffix(')')) {
        return parse_rgb_args(inner);
    }
    if let Some(inner) = v.strip_prefix("hsl(").and_then(|s| s.strip_suffix(')')) {
        return parse_hsl_args(inner);
    }
    STYLE_NAMED_COLORS
        .iter()
        .find(|(name, _)| *name == v)
        .map(|(_, rgb)| *rgb)
}

fn parse_hex_color(hex: &str) -> Option<[u8; 3]> {
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

fn parse_rgb_args(inner: &str) -> Option<[u8; 3]> {
    let parts: Vec<&str> = inner
        .split([',', '/'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if parts.len() < 3 {
        return None;
    }
    let mut rgb = [0u8; 3];
    for (i, p) in parts.iter().take(3).enumerate() {
        let (num, is_pct) = match p.strip_suffix('%') {
            Some(n) => (n, true),
            None => (*p, false),
        };
        let n: f32 = num.trim().parse().ok()?;
        if !n.is_finite() || n < 0.0 {
            return None;
        }
        let scaled = if is_pct { n * 2.55 } else { n };
        rgb[i] = scaled.round().clamp(0.0, 255.0) as u8;
    }
    Some(rgb)
}

fn parse_hsl_args(inner: &str) -> Option<[u8; 3]> {
    let parts: Vec<&str> = inner
        .split([',', '/'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if parts.len() < 3 {
        return None;
    }
    let h: f32 = parts[0].trim_end_matches("deg").trim().parse().ok()?;
    let s: f32 = parts[1].trim_end_matches('%').trim().parse::<f32>().ok()? / 100.0;
    let l: f32 = parts[2].trim_end_matches('%').trim().parse::<f32>().ok()? / 100.0;
    if !(h.is_finite() && s.is_finite() && l.is_finite())
        || !(0.0..=1.0).contains(&s)
        || !(0.0..=1.0).contains(&l)
    {
        return None;
    }
    let h = ((h % 360.0) + 360.0) % 360.0 / 360.0;
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let channel = |t: f32| -> u8 {
        let t = if t < 0.0 {
            t + 1.0
        } else if t > 1.0 {
            t - 1.0
        } else {
            t
        };
        let v = if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        };
        (v * 255.0).round().clamp(0.0, 255.0) as u8
    };
    Some([channel(h + 1.0 / 3.0), channel(h), channel(h - 1.0 / 3.0)])
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum BlockKind {
    Paragraph,
    /// ATX 标题，级别 1–6。
    AtxHeading(u8),
    /// Setext 标题（0c P0-2）：内容行 + `===`/`---` 下划线行，级别 1–2。
    /// 块区间含下划线行；下划线仅识别于段落续行位置（块起始的 `---` 仍为分隔线，
    /// 列表项/引用块后的 `---` 同理）。inline 仅解析内容行（下划线行不参与）。
    SetextHeading(u8),
    /// 围栏代码块。`lang` 为 info string 首词（```rust → Some("rust")），无标记为 None。
    FencedCode(Option<String>),
    /// 缩进代码块（4 列及以上前导缩进，tab 按 4 列制表位展开）。块内不解析 inline。
    /// 与 FencedCode 一样按源码原样渲染（不背叛源码）。
    IndentedCode,
    /// 展示数学块（`$$...$$`）。块内不解析 inline，TeX 源码经 `math_source` 提取。
    Math,
    /// GFM 表格（表头行 + 分隔行 + 至多若干表体行）。块级 inline 恒为空；
    /// 单元格文本/对齐/inline 节点经 `table` 提取（视图只读渲染，切片 9）。
    Table,
    /// 列表项（0c P0-3：多行项——项内续行/空行/懒续行并入本块；嵌套项与外层续块
    /// 各为独立块，续块无标记、`task` 恒 None）。`ordered` 区分有序/无序；
    /// `depth` = 缩进字节 / 2（上限 8，视觉深度；结构嵌套由块序列表达）。
    /// inline 仅解析标记后内容（续块无标记则解析全行）。
    /// `task` = GFM 任务列表复选框：`Some(false)`/`Some(true)` 为未勾/勾选。
    ListItem {
        ordered: bool,
        depth: u8,
        task: Option<bool>,
    },
    /// 引用块：连续 `>` 标记行合为一块，支持懒续行（0c P0-3：段落的无标记
    /// 续行并入，`---` 等可打断段落者除外）。inline 逐行解析标记后内容，
    /// 懒续行从行首解析。
    BlockQuote,
    /// GFM Admonition / 提示块：`> [!TYPE] 标题` 首行，后续 `>` 行组成正文。
    /// 五种类型 NOTE/TIP/IMPORTANT/WARNING/CAUTION 对应不同视觉样式；
    /// 本质是引用块的首行变体，inline 解析同 BlockQuote。
    Admonition(AdmonitionType),
    /// 配置文件定义的自定义 Callout（`> [!FIXME]`）：展示字段在解析期已解析，
    /// 渲染/导出直接消费；未知类型不产生本变体（回退普通引用块）。
    Callout(Callout),
    /// 分隔线：`---` / `***` / `___`（同字符 ≥ 3，其间可夹空白）。无 inline。
    ThematicBreak,
    /// 链接引用定义（`[label]: dest "title"`，可多行）：注册进文档引用表，
    /// 无 HTML 输出、无 inline；编辑器按源码行渲染（不背叛源码）。
    LinkRefDef,
    /// 脚注定义（GFM `[^id]: 内容`）：与 [`BlockKind::LinkRefDef`] 同构——仅块起始
    /// 识别、缩进 ≤3、无 inline、编辑器按源码渲染（不背叛源码）；续行须缩进 ≥4 列。
    /// 定义本身不占编号，只有被 `[^id]` 引用时才按首次引用顺序编号。
    FootnoteDef,
    /// 原始 HTML 块（CommonMark §4.6，类型 1–7）：块内不解析 inline，源码原样渲染。
    /// 类型 1–5 以闭合模式终止；类型 6（块级标签）与 7（完整标签行）以空行终止。
    HtmlBlock,
    /// 块级白名单 `<details>` 的**头块**：区间只含开标签行与 summary 行（B3 起）。
    /// summary 从 `<summary>X</summary>` 提取（HTML 实体不解析；允许嵌 inline 标签）。
    /// 内部其余行拆成独立兄弟块（标题/列表/段落/代码/表格等），由
    /// [`BlockContainer::details_depth`] 标注归属；闭标签行不属于任何块。
    /// `open_default` 按 HTML spec 默认收起（false）；`<details open>` 时为 true。
    Details {
        summary: String,
        open_default: bool,
    },
    /// Front Matter（Jekyll / Hugo / MkDocs 等）：文档起始处 `---`（YAML）或 `+++`（TOML）
    /// 围栏，内嵌元数据。块内不解析 inline，body 内容原样保存供视图态渲染为卡片。
    /// 仅首块（首非空行）命中，中间文档的 `---` 围栏仍按 ThematicBreak 处理。
    FrontMatter(String),
}

impl BlockKind {
    /// 块是否不含任何 inline 内容。脚注编号扫描、`Session::inlines` 与脏区脚注标记
    /// 检测共用此单一判定，避免多处手写集合漂移。
    /// `Details` 不属于此类：其正文块各自持有 inline，`<details>` 内的脚注引用
    /// 也必须参与编号。
    pub fn has_no_inlines(&self) -> bool {
        matches!(
            self,
            BlockKind::FencedCode(_)
                | BlockKind::IndentedCode
                | BlockKind::Math
                | BlockKind::Table
                | BlockKind::ThematicBreak
                | BlockKind::LinkRefDef
                | BlockKind::FootnoteDef
                | BlockKind::HtmlBlock
                | BlockKind::FrontMatter(_)
        )
    }
}

/// 表格列对齐（GFM 分隔行 `:--` / `--:` / `:-:` 语法）。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum CellAlign {
    #[default]
    None,
    Left,
    Center,
    Right,
}

/// 单元格派生数据：可见文本 + inline 节点 + 内容区间（文档字节偏移，已 trim）。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TableCell {
    /// 剥除 inline 标记后的可见文本（`**b**` → `b`），供只读渲染直接使用。
    pub text: String,
    /// 单元格内 inline 节点（GFM「inlines are parsed」）：供样式渲染与一致性门禁。
    pub inlines: Vec<InlineNode>,
    /// 内容区间；网格点击以此起点进入表格源码态。
    pub range: Range,
}

/// 表格派生数据（不持久化、不进撤销栈，随块结构自动正确）。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Table {
    /// 每列对齐，长度 = 列数。
    pub aligns: Vec<CellAlign>,
    pub header: Vec<TableCell>,
    /// 表体行：每行长度恒等于列数（缺格补空，多格截断，GFM 行为）。
    pub rows: Vec<Vec<TableCell>>,
}

/// 表格所在容器（仅 [`BlockKind::Table`] 块携带）：视图据此缩进/画引用条，
/// 导出据此保持容器包裹关系。容器块本身仍按原块型渲染（引用文本/列表项/details 头）。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct BlockContainer {
    /// 外层引用层数（`> > | a |` → 2）。
    pub quote_depth: u8,
    /// 列表项内容缩进列数（`- | a |` → 2；顶层表格 → 0）。
    pub list_indent: u8,
    /// 位于块级 `<details>` 内（等价于 `details_depth > 0`，保留字段以兼容既有
    /// 渲染/导出判定；新建块由扫描层同时写入二者）。
    pub details: bool,
    /// 块级 `<details>` 嵌套层级：0 = 不在 details 内，1 = 顶层 details 的直接子块，
    /// 2 = 嵌套 details 的子块…… Details 头块自身取其所在层级（顶层头块为 0）。
    /// 视图按「父子层级相邻」分组：折叠时整组（含嵌套）高度为 0。
    pub details_depth: u8,
}

impl BlockContainer {
    /// 是否位于任一容器内（渲染缩进/引用条判定）。
    pub fn is_nested(&self) -> bool {
        self.quote_depth > 0 || self.list_indent > 0 || self.details
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct BlockId(pub u64);

#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct Block {
    pub id: BlockId,
    pub kind: BlockKind,
    /// 行区间 `[start_line, end_line)`。仅解析内部（扫描/对齐/派生数据）使用；
    /// 对外块快照一律走 `range`。
    pub start_line: usize,
    pub end_line: usize,
    /// 字节区间 `[start, end)`：`start` 为 `start_line` 首字节，`end` 为
    /// `end_line` 行首（`end_line` 越过末行时为文档长度）。由 reparse 增量维护，
    /// `Session::blocks()` 直接拷贝，不做任何 rope 行号换算。
    pub range: Range,
    /// 表格容器上下文（非表格块恒为默认值）。
    pub container: BlockContainer,
    /// 表格实际行数（含表头/分隔行）。仅 Details 内表格在块区间末尾吞入
    /// `</details>` 行时小于 `end_line - start_line`，其余为 None（= 整块）。
    pub table_lines: Option<usize>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum InlineKind {
    /// `**strong**`
    Strong,
    /// `*emph*`
    Emph,
    /// `` `code` ``，内容原样不嵌套
    Code,
    /// `[text](url)`
    Link,
    /// `![alt](url)`：`content` 为 alt 文本（可为空），`url` 为图片路径/URL。
    Image,
    /// `$math$`，内容原样不嵌套
    Math,
    /// `~~del~~`（GFM 删除线）
    Strikethrough,
    /// `==text==`（高亮，等价 `<mark>`）：标记 `==` 整段区间，内容非空。
    Highlight,
    /// `[^id]`（GFM 脚注引用）：`content` 为 id 文本区间（不含 `[^` / `]`）。
    /// 解析层不校验定义是否存在——缺失定义仍产出节点，由视图按编号表降级。
    FootnoteRef,
    /// 行内白名单成对 HTML：`<kbd>Ctrl</kbd>` / `<sub>1</sub>` / `<abbr title="...">X</abbr>`
    /// / `<code>x</code>` / `<a href="...">` / `<details><summary>` 等。
    /// 子节点递归解析；未闭合或白名单外退回 InlineKind::Text（整段源码原样）。
    Html { tag: HtmlTag, attrs: HtmlAttrs },
    /// 行内白名单自闭合 HTML：`<br>` / `<img src="..." alt="...">`。无子节点。
    HtmlSelfClosed { tag: HtmlTag, attrs: HtmlAttrs },
    /// 行内 HTML 未被白名单接管时的源码原样节点（含尖括号）。不参与嵌套；
    /// bracket_pairs 扫描照常跳过其 `[`/`]`（保持现有 HTML 跳过逻辑）。
    HtmlLiteral,
}

/// 一棵 inline 子树。所有区间为文档字节偏移、半开 `[start, end)`。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct InlineNode {
    pub kind: InlineKind,
    /// 含开闭标记的整个节点区间。
    pub range: Range,
    /// 内容区间（不含标记；Link 为可见文本，FootnoteRef 为 id 文本）。
    pub content: Range,
    /// 链接 URL 区间（仅 Link 为 Some）。
    pub url: Option<Range>,
    /// 嵌套子节点（Code/Math 恒为空）。
    pub children: Vec<InlineNode>,
}

impl InlineNode {
    /// 是否为图片节点：Markdown `![alt](url)` 或白名单 `<img ...>`。
    /// 视图、HTML 导出与 PDF 导出共用，避免各自手写 `matches!` 漂移。
    pub fn is_image(&self) -> bool {
        matches!(self.kind, InlineKind::Image)
            || matches!(
                self.kind,
                InlineKind::HtmlSelfClosed {
                    tag: HtmlTag::Img,
                    ..
                }
            )
    }
}

// ===== 安全过滤辅助（export.rs / paint.rs 使用，解析层仍产出原始节点）=====

/// 强制剥除的危险 HTML 标签：export 层遇到这些 tag 直接丢，不 escape 也不显示占位。
pub const HTML_STRIP_TAGS: &[&str] = &[
    "script", "style", "iframe", "object", "embed", "form", "textarea",
];

pub fn should_strip_html(tag: &str) -> bool {
    let t = tag.to_ascii_lowercase();
    HTML_STRIP_TAGS.iter().any(|w| *w == t)
}

/// 允许在 export/paint 层原样保留的行内 HTML 白名单（小写，供其他层交叉验证）。
#[allow(dead_code)]
pub const HTML_INLINE_WHITELIST: &[&str] = &[
    "kbd", "sub", "sup", "mark", "abbr", "br", "img", "code", "pre", "a", "strong", "b", "em", "i",
    "u", "s", "del", "ins", "q", "cite", "var", "samp", "small", "span",
];
