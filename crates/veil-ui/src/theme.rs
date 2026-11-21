//! 主题与配置（切片 17）：调色板、极简 TOML 子集解析、配置路径，以及平台接缝 trait。
//!
//! 零 GUI 依赖：解析/序列化/路径推导/主题判定全部为纯函数，headless 可测。
//! 平台能力（注册表系统外观、DWM 深色标题栏/Mica、配置落盘）以 trait 声明，由产品层注入。

//!
//! 配置格式（极简，共 15 个顶层键）：
//! ```toml
//! mode = "system"        # system | light | dark
//! export_margin_mm = 20  # PDF 页边距（毫米）
//! export_page_numbers = true
//! export_toc = true
//! doc_font_family = "system"  # system | sans | serif | mono
//! doc_font_size = 16     # 正文基准字号（物理像素），夹取 10..=32
//! doc_line_height = 22   # 正文基准行高（物理像素），夹取 字号..=字号×3
//! allow_remote_images = false  # 默认阻止普通远程图片；true = 用户显式允许
//! tab_width = 4          # 制表符宽度（2 或 4），默认 4
//! image_cache_limit_mb = 200  # 图片缓存字节上限（MiB），0 = 不限制
//! # 以下四项仅在窗口几何已知时成组写入（旧配置不含这些键）
//! window_width = 1200    # 逻辑宽，480..=8192
//! window_height = 800    # 逻辑高，360..=8192
//! window_x = 100         # 物理坐标
//! window_y = 100
//! ```
//! 历史格式的 `[theme]` 段标题行仍被接受但直接跳过，便于旧配置无损升级。

use std::ffi::OsString;
use std::io::Cursor;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::OnceLock;

/// 主题模式：跟随系统 / 强制浅色 / 强制深色。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ThemeMode {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeMode {
    /// 配置值解析（严格小写，与 `as_str` 严格互逆）。
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "system" => Some(ThemeMode::System),
            "light" => Some(ThemeMode::Light),
            "dark" => Some(ThemeMode::Dark),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ThemeMode::System => "system",
            ThemeMode::Light => "light",
            ThemeMode::Dark => "dark",
        }
    }

    /// 状态栏提示用中文标签。
    pub fn label(self) -> &'static str {
        crate::i18n::t(match self {
            ThemeMode::System => crate::i18n::Key::MenuViewThemeSystem,
            ThemeMode::Light => crate::i18n::Key::MenuViewThemeLight,
            ThemeMode::Dark => crate::i18n::Key::MenuViewThemeDark,
        })
    }
}

/// 代码块语法高亮色板。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SyntaxColors {
    pub keyword: [u8; 3],
    pub string: [u8; 3],
    pub comment: [u8; 3],
    pub number: [u8; 3],
    pub function: [u8; 3],
    pub type_name: [u8; 3],
    pub punctuation: [u8; 3],
}

/// plist 解析失败时的硬编码兜底值（与原始硬编码版本同值，确保对比度门禁不退化）。
const SYNTAX_FALLBACK: SyntaxColors = SyntaxColors {
    keyword: [173, 107, 169],
    string: [86, 141, 64],
    comment: [128, 128, 128],
    number: [181, 106, 31],
    function: [47, 105, 174],
    type_name: [78, 136, 195],
    punctuation: [100, 100, 100],
};

impl SyntaxColors {
    /// 默认色板（LayoutEngine 无主题时的兜底）。
    /// 与 Theme::light().syntax 同源：都从 nord_light.tmTheme 加载，保证一致。
    pub fn default_light() -> Self {
        static CACHE: OnceLock<SyntaxColors> = OnceLock::new();
        *CACHE.get_or_init(|| {
            Self::from_tmtheme_bytes(include_bytes!("../assets/themes/nord_light.tmTheme"))
        })
    }

    /// 从 .tmTheme plist 字节直接解析出 Veil 的 7 字段色板。
    /// syntect 5.x 的 ScopeSelector 没有 Display，无法可靠提取 scope 字符串做前缀匹配，
    /// 所以这里绕开 syntect Theme 遍历，直接用 plist crate 按 "<scope> → <fg color>" 解析。
    ///
    /// 我们的 tmTheme 只包含恰好 7 条规则（keyword/comment/string/constant.numeric/
    /// entity.name.function/entity.name.type/punctuation），所以用 plist 匹配 key 前缀即可。
    /// 未匹配到的字段保留硬编码兜底值（确保对比度门禁不退化）。
    pub(crate) fn from_tmtheme_bytes(bytes: &[u8]) -> Self {
        let mut out = SYNTAX_FALLBACK;

        let cursor = Cursor::new(bytes);
        let root = match plist::Value::from_reader_xml(cursor) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("[theme] plist 解析失败: {e}");
                return out;
            }
        };

        // tmTheme 顶层是 dict → "settings" 数组 → 跳过第一个全局 settings dict → 遍历 scope rule dicts
        let Some(root_dict) = root.as_dictionary() else {
            return out;
        };
        let Some(settings_arr) = root_dict.get("settings").and_then(|v| v.as_array()) else {
            return out;
        };

        for dict in settings_arr.iter().skip(1) {
            let Some(d) = dict.as_dictionary() else {
                continue;
            };
            // scope 字符串
            let Some(scope_str) = d.get("scope").and_then(|v| v.as_string()) else {
                continue;
            };
            // foreground 颜色 "#RRGGBB"
            let Some(style) = d.get("settings").and_then(|v| v.as_dictionary()) else {
                continue;
            };
            let Some(fg_hex) = style.get("foreground").and_then(|v| v.as_string()) else {
                continue;
            };

            let rgb = match parse_hex_color(fg_hex) {
                Some(c) => c,
                None => continue,
            };

            if scope_str.starts_with("keyword") {
                out.keyword = rgb;
            } else if scope_str.starts_with("comment") {
                out.comment = rgb;
            } else if scope_str.starts_with("string") {
                out.string = rgb;
            } else if scope_str.starts_with("constant.numeric")
                || scope_str.starts_with("constant.language")
                || scope_str.starts_with("constant.character")
            {
                out.number = rgb;
            } else if scope_str.starts_with("entity.name.function")
                || scope_str.starts_with("support.function")
            {
                out.function = rgb;
            } else if scope_str.starts_with("entity.name.type")
                || scope_str.starts_with("storage.type")
                || scope_str.starts_with("support.type")
                || scope_str.starts_with("entity.name.class")
            {
                out.type_name = rgb;
            } else if scope_str.starts_with("punctuation") {
                out.punctuation = rgb;
            }
        }
        out
    }
}

fn parse_hex_color(hex: &str) -> Option<[u8; 3]> {
    // "#RRGGBB" 或 "RRGGBB" 或 "#RRGGBBAA"
    let hex = hex.trim_start_matches('#');
    if hex.len() < 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some([r, g, b])
}

/// 调色板：正文/ chrome / 交互态全套颜色。浅色值与原硬编码常量逐字节一致（视觉不回退）。
///
/// **约束**：本结构体所有字段必须为颜色值（[u8; 3] / [u8; 4]）。几何令牌归 `tokens::surface`；
/// 新增字段必须是颜色，不得引入 `corner_radius`、`shadow_offset` 等几何量。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Theme {
    pub mode: ThemeMode,
    pub dark: bool,
    pub bg: [u8; 4],
    pub fg: [u8; 3],
    pub code_bg: [u8; 4],
    pub sel: [u8; 4],
    pub table_head_bg: [u8; 4],
    pub grid: [u8; 4],
    pub chrome_bg: [u8; 4],
    pub chrome_border: [u8; 4],
    pub hover: [u8; 4],
    pub popup_bg: [u8; 4],
    pub thumb: [u8; 4],
    pub muted: [u8; 3],
    pub text: [u8; 3],
    /// 链接前景色（渲染态链接与正文的视觉区分）。
    pub link: [u8; 3],
    /// 危险操作前景/描边色（删除确认等模态按钮）：在各自 bg 上满足 ≥4.5:1。
    pub danger: [u8; 3],
    /// 主操作强调色（主按钮填充、焦点环）：与 `link` 分开，因为链接色要浅到能读，
    /// 而填充底色要深到能承载反色文字——两者不可复用同一值。
    pub accent: [u8; 3],
    /// 强调色上的文字色（主按钮文字）。
    pub accent_text: [u8; 3],
    pub syntax: SyntaxColors,
}

/// Surface 底色三档语义（S5）：Theme 的底色字段按层级归为 Base / Card / Popup。
///
/// | 档 | Theme 字段 | 含义 |
/// |---|---|---|
/// | Base | bg | 页面/窗口底层背景 |
/// | Card | chrome_bg | chrome 区条带、表头（不弹起） |
/// | Popup | popup_bg | 浮层、弹出、模态（带阴影弹起） |
///
/// 代码块底色（code_bg）与表头底色（table_head_bg）是 Card 档的特化，
/// 不归本枚举直接映射——调用方按需经 `tokens::palette::code_bg` / `table_head_bg` 取。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(dead_code)] // API 预留：paint 层仍经 palette 模块消费，SurfaceLevel 后续批次会有更多使用点
pub enum SurfaceLevel {
    Base,
    Card,
    Popup,
}

impl Theme {
    /// 按档取 surface 底色。
    #[allow(dead_code)] // 同 SurfaceLevel：后续批次消费
    pub fn surface(&self, level: SurfaceLevel) -> [u8; 4] {
        match level {
            SurfaceLevel::Base => self.bg,
            SurfaceLevel::Card => self.chrome_bg,
            SurfaceLevel::Popup => self.popup_bg,
        }
    }
    /// 按模式与系统外观解析出最终调色板（System 跟随传入的系统态）。
    pub fn of(mode: ThemeMode, system_dark: bool) -> Self {
        let dark = match mode {
            ThemeMode::System => system_dark,
            ThemeMode::Light => false,
            ThemeMode::Dark => true,
        };
        let mut t = if dark { Self::dark() } else { Self::light() };
        t.mode = mode;
        t
    }

    pub fn light() -> Self {
        Theme {
            mode: ThemeMode::System,
            dark: false,
            bg: [255, 255, 255, 255],
            fg: [30, 30, 30],
            code_bg: [245, 245, 245, 255],
            sel: [40, 110, 210, 80],
            table_head_bg: [246, 248, 250, 255],
            grid: [136, 141, 148, 255],
            chrome_bg: [248, 248, 248, 255],
            chrome_border: [208, 213, 221, 255],
            hover: [228, 236, 246, 255],
            popup_bg: [255, 255, 255, 255],
            thumb: [176, 184, 195, 255],
            muted: [110, 110, 110],
            text: [45, 45, 45],
            link: [43, 108, 176],
            danger: [178, 34, 34],
            accent: [43, 108, 176],
            accent_text: [255, 255, 255],
            syntax: light_syntax(),
        }
    }

    pub fn dark() -> Self {
        Theme {
            mode: ThemeMode::System,
            dark: true,
            bg: [30, 30, 30, 255],
            fg: [220, 220, 220],
            code_bg: [40, 40, 40, 255],
            sel: [70, 130, 220, 110],
            table_head_bg: [42, 42, 42, 255],
            grid: [112, 116, 122, 255],
            chrome_bg: [38, 38, 38, 255],
            chrome_border: [60, 64, 70, 255],
            hover: [55, 65, 80, 255],
            popup_bg: [45, 45, 45, 255],
            thumb: [96, 102, 110, 255],
            muted: [160, 160, 160],
            text: [225, 225, 225],
            link: [127, 180, 245],
            danger: [255, 130, 130],
            accent: [14, 99, 156],
            accent_text: [255, 255, 255],
            syntax: dark_syntax(),
        }
    }
}

/// 浅色语法色板：从 embed 的 nord_light.tmTheme 加载（OnceLock 缓存，进程内只 parse 一次）。
fn light_syntax() -> SyntaxColors {
    static CACHE: OnceLock<SyntaxColors> = OnceLock::new();
    *CACHE.get_or_init(|| {
        let bytes = include_bytes!("../assets/themes/nord_light.tmTheme");
        SyntaxColors::from_tmtheme_bytes(bytes)
    })
}

/// 深色语法色板：从 embed 的 one_dark.tmTheme 加载（OnceLock 缓存）。
fn dark_syntax() -> SyntaxColors {
    static CACHE: OnceLock<SyntaxColors> = OnceLock::new();
    *CACHE.get_or_init(|| {
        let bytes = include_bytes!("../assets/themes/one_dark.tmTheme");
        SyntaxColors::from_tmtheme_bytes(bytes)
    })
}

pub(crate) use crate::tokens::surface;

/// 正文字体族（保持 `Copy`：只存枚举，排版时映射到 `&'static str` / 通用族）。
///
/// `System` 不显式设置 family，沿用既有字体回退链（视觉不变）。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum DocFontFamily {
    #[default]
    System,
    Sans,
    Serif,
    Mono,
}

impl DocFontFamily {
    /// 配置值解析（严格小写，与 `as_str` 严格互逆）。
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "system" => Some(DocFontFamily::System),
            "sans" => Some(DocFontFamily::Sans),
            "serif" => Some(DocFontFamily::Serif),
            "mono" => Some(DocFontFamily::Mono),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            DocFontFamily::System => "system",
            DocFontFamily::Sans => "sans",
            DocFontFamily::Serif => "serif",
            DocFontFamily::Mono => "mono",
        }
    }

    /// 缓存键用的稳定判别值（与 `as_str` 同序，仅需进程内稳定）。
    pub(crate) fn key(self) -> u8 {
        match self {
            DocFontFamily::System => 0,
            DocFontFamily::Sans => 1,
            DocFontFamily::Serif => 2,
            DocFontFamily::Mono => 3,
        }
    }
}

/// HTML 导出样式快照：把「当前主题」压缩为导出所需的最小颜色集合。
///
/// 为什么单独一个结构：`Theme` 是渲染内部细节（含 chrome/选区/滚动条等导出用不到的色），
/// 产品层（veil）只应拿到不可变的值语义快照，故只导出具名字段的全 `Copy` 结构，
/// 导出端不依赖任何 GUI/主题模块即可生成样式。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ExportStyle {
    pub dark: bool,
    pub bg: [u8; 3],
    pub text: [u8; 3],
    pub muted: [u8; 3],
    pub link: [u8; 3],
    pub code_bg: [u8; 3],
    pub border: [u8; 3],
}

impl Default for ExportStyle {
    /// 浅色默认：与改造前硬编码的导出样式逐色一致，未注入主题时视觉不回退。
    fn default() -> Self {
        ExportStyle {
            dark: false,
            bg: [0xff, 0xff, 0xff],
            text: [0x1e, 0x1e, 0x1e],
            muted: [0x66, 0x66, 0x66],
            link: [0x2b, 0x6c, 0xb0],
            code_bg: [0xf5, 0xf5, 0xf5],
            border: [0xdd, 0xdd, 0xdd],
        }
    }
}

impl ExportStyle {
    /// 主题 → 导出样式。字段映射：
    /// - `bg`/`text`/`muted`/`link` 直接取同名主题字段（`text` 取 `fg`，正文前景）；
    /// - `code_bg` 复用主题 `code_bg`（编辑器代码块底色，导出与屏幕一致）；
    /// - `border` 复用 `chrome_border`（主题里唯一通用的分界色，表格/引用/分隔线共用）。
    ///
    /// 为什么 `pub(crate)`：产品层只消费导出的快照，不需要自己从 `Theme` 推颜色；
    /// 保持 `Theme` 不进入公共 API（ADR-009 S5 按实际使用收窄导出面）。
    pub(crate) fn from_theme(t: &Theme) -> Self {
        /// 主题色为 4 字节（含 alpha），导出 CSS 只需 RGB。
        const fn rgb(c: [u8; 4]) -> [u8; 3] {
            [c[0], c[1], c[2]]
        }
        ExportStyle {
            dark: t.dark,
            bg: rgb(t.bg),
            text: t.fg,
            muted: t.muted,
            link: t.link,
            code_bg: rgb(t.code_bg),
            border: rgb(t.chrome_border),
        }
    }
}

/// 窗口几何（逻辑宽高 + 物理坐标），由 UI 层在启动/退出时读写。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct WindowGeom {
    pub width: u32,
    pub height: u32,
    pub x: i32,
    pub y: i32,
}

/// 正文字号默认值（物理像素基准）：单一来源 `tokens::typography::BODY_SIZE`。
const DEFAULT_DOC_FONT_SIZE: f32 = crate::tokens::typography::BODY_SIZE;
/// 正文行高默认值（物理像素基准）：单一来源 `tokens::typography::BODY_LINE_HEIGHT`。
const DEFAULT_DOC_LINE_HEIGHT: f32 = crate::tokens::typography::BODY_LINE_HEIGHT;
/// 正文字号允许范围（物理像素基准）。
const DOC_FONT_SIZE_MIN: f32 = 10.0;
const DOC_FONT_SIZE_MAX: f32 = 32.0;
/// 字体族值最大长度：超长视为无法识别（防止恶意/损坏配置）。
const DOC_FONT_FAMILY_MAX_LEN: usize = 64;
/// 制表符宽度默认值（兼容 2 与 4，Task D 新增）。
const DEFAULT_TAB_WIDTH: u8 = 4;
/// 图片缓存字节上限默认值（MiB）：容纳一屏图文混排的若干张大图，超出后按 LRU 逐出。
const DEFAULT_IMAGE_CACHE_LIMIT_MB: u32 = 200;
/// 图片缓存上限允许的最大值（MiB）：防止手写配置把上限设成天文数字变相关闭逐出。
const IMAGE_CACHE_LIMIT_MB_MAX: u32 = 4096;

/// 正文字号夹取：10..=32。
fn clamp_font_size(v: f32) -> f32 {
    v.clamp(DOC_FONT_SIZE_MIN, DOC_FONT_SIZE_MAX)
}

/// 正文行高夹取：字号..=字号×3（保证行高不小于字号、又不至于过分稀疏）。
fn clamp_line_height(v: f32, size: f32) -> f32 {
    v.clamp(size, size * 3.0)
}

/// 配置内容：主题模式 + PDF 导出设置 + 正文排版 + 窗口几何。
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Config {
    pub mode: ThemeMode,
    /// PDF 页边距（毫米）。
    pub export_margin_mm: f32,
    /// PDF 页脚页码 `n / total`。
    pub export_page_numbers: bool,
    /// PDF 目录页。
    pub export_toc: bool,
    /// 正文字体族（仅作用于正文/表格文本，UI chrome 不变）。
    pub doc_font_family: DocFontFamily,
    /// 正文字号（物理像素基准，夹取 10..=32）。
    pub doc_font_size: f32,
    /// 正文行高（物理像素基准，夹取 字号..=字号×3）。
    pub doc_line_height: f32,
    /// 普通远程图片自动加载（默认关）；true = 用户显式允许，false = 全局阻止，只显示 alt 文本。
    pub allow_remote_images: bool,
    /// 制表符宽度（2 或 4，默认 4）；其它值夹到 4。
    pub tab_width: u8,
    /// 图片缓存字节上限（MiB，默认 200）；0 = 不限制，超出按 LRU 逐出。
    pub image_cache_limit_mb: u32,
    /// 上次退出时的窗口几何；首次运行或配置缺失时为 `None`。
    pub window: Option<WindowGeom>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            mode: ThemeMode::System,
            export_margin_mm: 20.0,
            export_page_numbers: true,
            export_toc: true,
            doc_font_family: DocFontFamily::System,
            doc_font_size: DEFAULT_DOC_FONT_SIZE,
            doc_line_height: DEFAULT_DOC_LINE_HEIGHT,
            allow_remote_images: false,
            tab_width: DEFAULT_TAB_WIDTH,
            image_cache_limit_mb: DEFAULT_IMAGE_CACHE_LIMIT_MB,
            window: None,
        }
    }
}

impl Config {
    /// 生成配置文件文本（写入即此格式；`parse_config(to_toml())` 恒等往返）。
    pub fn to_toml(self) -> String {
        let mut text = format!(
            "# Veil 配置（自动生成）\n# mode: system（跟随系统）| light | dark\nmode = \"{}\"\n\
             export_margin_mm = {}\nexport_page_numbers = {}\nexport_toc = {}\n\
             doc_font_family = \"{}\"\ndoc_font_size = {}\ndoc_line_height = {}\n\
             allow_remote_images = {}\ntab_width = {}\n\
             image_cache_limit_mb = {}\n",
            self.mode.as_str(),
            self.export_margin_mm,
            self.export_page_numbers,
            self.export_toc,
            self.doc_font_family.as_str(),
            self.doc_font_size,
            self.doc_line_height,
            self.allow_remote_images,
            self.tab_width,
            self.image_cache_limit_mb,
        );
        if let Some(w) = self.window {
            text.push_str(&format!(
                "window_width = {}\nwindow_height = {}\nwindow_x = {}\nwindow_y = {}\n",
                w.width, w.height, w.x, w.y
            ));
        }
        text
    }

    /// 保存配置文本：原文是严格 TOML 且不含表段时原地编辑（保留注释、未知键与键顺序），
    /// 否则回退整文件重生成（与替换前行为一致）。
    ///
    /// 「表段」一律回退：历史 `[theme]` 段的键在旧行解析里按顶层生效，保留嵌套表会改变
    /// 下次加载的语义（ADR-012 / S2）。
    pub fn to_toml_preserving(self, original: Option<&str>) -> String {
        original
            .and_then(|text| edit_existing_toml(text, self))
            .unwrap_or_else(|| self.to_toml())
    }
}

/// 原地编辑已存在的严格 TOML 配置；返回 `None` 表示必须回退整文件重生成。
///
/// 仅当整文档可被 TOML 解析且不含任何表段时启用：关键字值逐项覆盖，未知键与注释原样保留。
fn edit_existing_toml(text: &str, cfg: Config) -> Option<String> {
    let mut doc: toml_edit::DocumentMut = text.parse().ok()?;
    if doc
        .iter()
        .any(|(_, item)| item.is_table() || item.is_array_of_tables())
    {
        return None;
    }
    set_item(&mut doc, "mode", toml_edit::value(cfg.mode.as_str()));
    set_item(
        &mut doc,
        "export_margin_mm",
        toml_edit::value(f64::from(cfg.export_margin_mm)),
    );
    set_item(
        &mut doc,
        "export_page_numbers",
        toml_edit::value(cfg.export_page_numbers),
    );
    set_item(&mut doc, "export_toc", toml_edit::value(cfg.export_toc));
    set_item(
        &mut doc,
        "doc_font_family",
        toml_edit::value(cfg.doc_font_family.as_str()),
    );
    set_item(
        &mut doc,
        "doc_font_size",
        toml_edit::value(f64::from(cfg.doc_font_size)),
    );
    set_item(
        &mut doc,
        "doc_line_height",
        toml_edit::value(f64::from(cfg.doc_line_height)),
    );
    set_item(
        &mut doc,
        "allow_remote_images",
        toml_edit::value(cfg.allow_remote_images),
    );
    set_item(
        &mut doc,
        "tab_width",
        toml_edit::value(i64::from(cfg.tab_width)),
    );
    set_item(
        &mut doc,
        "image_cache_limit_mb",
        toml_edit::value(i64::from(cfg.image_cache_limit_mb)),
    );
    const WINDOW_KEYS: [&str; 4] = ["window_width", "window_height", "window_x", "window_y"];
    if let Some(w) = cfg.window {
        set_item(
            &mut doc,
            WINDOW_KEYS[0],
            toml_edit::value(i64::from(w.width)),
        );
        set_item(
            &mut doc,
            WINDOW_KEYS[1],
            toml_edit::value(i64::from(w.height)),
        );
        set_item(&mut doc, WINDOW_KEYS[2], toml_edit::value(i64::from(w.x)));
        set_item(&mut doc, WINDOW_KEYS[3], toml_edit::value(i64::from(w.y)));
    } else {
        for key in WINDOW_KEYS {
            doc.remove(key);
        }
    }
    Some(doc.to_string())
}

/// 覆盖一个键的值，同时保留原有的值前后装饰（行尾注释、空格）。
fn set_item(doc: &mut toml_edit::DocumentMut, key: &str, mut item: toml_edit::Item) {
    match doc.get_mut(key) {
        Some(existing) => {
            let decor = existing.as_value().map(|v| v.decor().clone());
            if let (Some(decor), Some(value)) = (decor, item.as_value_mut()) {
                *value.decor_mut() = decor;
            }
            *existing = item;
        }
        None => {
            doc.insert(key, item);
        }
    }
}

/// 解析结果：`ignored` 为无法识别的条目数（调用方提示后忽略）。
#[derive(Clone, PartialEq, Debug, Default)]
pub struct ConfigParse {
    pub config: Config,
    pub ignored: usize,
}

/// 极简配置解析：识别顶层 `mode`/`export_*`、正文排版键
/// `doc_font_family`/`doc_font_size`/`doc_line_height`/`allow_remote_images`
/// 与成组的窗口键 `window_width`/`window_height`/`window_x`/`window_y`，其余条目计数忽略。
/// 排版数值越界按夹取处理（不计数），类型/字面量非法才计入 ignored；
/// 字号先夹 10..=32，行高再按「字号..=字号×3」夹取（与键出现顺序无关）。
/// 窗口键必须四个齐全且宽 ≥480、高 ≥360（均 ≤8192）才产出几何；只出现一部分或值非法时不产出，
/// 已出现（含非法）的窗口键各计一次 `ignored`（完全不含窗口键的旧配置不计数）。
/// 段标题行（如历史格式 `[theme]`）不承载配置项，直接跳过；
/// 同一键重复出现时后者生效（无歧义地覆盖）。
pub fn parse_config(text: &str) -> ConfigParse {
    let mut out = ConfigParse::default();
    let mut font_size: Option<f32> = None;
    let mut line_height: Option<f32> = None;
    let mut win_seen = [false; 4];
    let mut win_w = None;
    let mut win_h = None;
    let mut win_x = None;
    let mut win_y = None;

    // 从 toml_edit AST 或 fallback 中拉平所有 key → Value 对（段标题下的键也归入顶层）。
    let pairs = collect_entries(text, &mut out.ignored);

    for (key, value) in pairs {
        match key.as_str() {
            "mode" => match value.as_str().and_then(ThemeMode::parse) {
                Some(m) => out.config.mode = m,
                None => out.ignored += 1,
            },
            "export_margin_mm" => {
                let v = value
                    .as_float()
                    .or_else(|| value.as_integer().map(|i| i as f64));
                match v {
                    Some(v) if v.is_finite() && v >= 0.0 => out.config.export_margin_mm = v as f32,
                    _ => out.ignored += 1,
                }
            }
            "export_page_numbers" => match value.as_bool() {
                Some(v) => out.config.export_page_numbers = v,
                None => out.ignored += 1,
            },
            "export_toc" => match value.as_bool() {
                Some(v) => out.config.export_toc = v,
                None => out.ignored += 1,
            },
            "doc_font_family" => match value
                .as_str()
                .filter(|s| s.len() <= DOC_FONT_FAMILY_MAX_LEN)
                .and_then(DocFontFamily::parse)
            {
                Some(f) => out.config.doc_font_family = f,
                None => out.ignored += 1,
            },
            "doc_font_size" => {
                let v = value
                    .as_float()
                    .or_else(|| value.as_integer().map(|i| i as f64));
                match v {
                    Some(v) if v.is_finite() => font_size = Some(v as f32),
                    _ => out.ignored += 1,
                }
            }
            "doc_line_height" => {
                let v = value
                    .as_float()
                    .or_else(|| value.as_integer().map(|i| i as f64));
                match v {
                    Some(v) if v.is_finite() => line_height = Some(v as f32),
                    _ => out.ignored += 1,
                }
            }
            // 已移除的旧键（平滑滚动开关）：滚动固定为瞬时，静默忽略且不提示未知键。
            "smooth_scroll" => {}
            "allow_remote_images" => match value.as_bool() {
                Some(v) => out.config.allow_remote_images = v,
                None => out.ignored += 1,
            },
            // 已移除的旧键（Mermaid 插入策略）：静默忽略，快照动作永远落盘本地资产。
            "mermaid_insert_url" => {}
            // 已移除的旧键（进程内 Mermaid 渲染开关）：静默忽略，不产生「无法识别」提示。
            "mermaid_render" => {}
            "tab_width" => match value.as_integer() {
                Some(v) if v == 2 || v == 4 => out.config.tab_width = v as u8,
                _ => out.ignored += 1,
            },
            // 0 = 不限制；上限 4096 MiB 防止手写值变相关闭逐出，越界按非法计数
            "image_cache_limit_mb" => match value.as_integer() {
                Some(v) if (0..=i64::from(IMAGE_CACHE_LIMIT_MB_MAX)).contains(&v) => {
                    out.config.image_cache_limit_mb = v as u32;
                }
                _ => out.ignored += 1,
            },
            "window_width" => {
                win_seen[0] = true;
                win_w = value
                    .as_integer()
                    .and_then(|n| (480..=8192).contains(&(n as u32)).then_some(n as u32));
            }
            "window_height" => {
                win_seen[1] = true;
                win_h = value
                    .as_integer()
                    .and_then(|n| (360..=8192).contains(&(n as u32)).then_some(n as u32));
            }
            "window_x" => {
                win_seen[2] = true;
                win_x = value.as_integer().map(|n| n as i32);
            }
            "window_y" => {
                win_seen[3] = true;
                win_y = value.as_integer().map(|n| n as i32);
            }
            _ => out.ignored += 1,
        }
    }
    let size = clamp_font_size(font_size.unwrap_or(DEFAULT_DOC_FONT_SIZE));
    out.config.doc_font_size = size;
    out.config.doc_line_height =
        clamp_line_height(line_height.unwrap_or(DEFAULT_DOC_LINE_HEIGHT), size);
    out.config.window = match (win_w, win_h, win_x, win_y) {
        (Some(width), Some(height), Some(x), Some(y)) => Some(WindowGeom {
            width,
            height,
            x,
            y,
        }),
        _ => {
            out.ignored += win_seen.iter().filter(|seen| **seen).count();
            None
        }
    };
    out
}

/// 提取所有 key → Value 对。优先用 toml_edit 严格解析（合法 TOML）；
/// 若失败（重复键等宽容场景）则 fallback 到手动逐行 key=value 提取，
/// 值类型由 toml_edit::Value::from_str 解析（自动处理引号/布尔/数字）。
fn collect_entries(text: &str, ignored: &mut usize) -> Vec<(String, toml_edit::Value)> {
    match toml_edit::DocumentMut::from_str(text) {
        Ok(doc) => {
            let mut out = Vec::new();
            flatten_table(doc.as_table(), &mut out);
            out
        }
        Err(_) => {
            let mut out = Vec::new();
            for raw in text.lines() {
                let line = strip_comment_fallback(raw).trim();
                if line.is_empty() || line.starts_with('[') {
                    continue;
                }
                let Some((key, val_str)) = line.split_once('=') else {
                    *ignored += 1;
                    continue;
                };
                let key = key.trim().to_string();
                let val_str = val_str.trim();
                match toml_edit::Value::from_str(val_str) {
                    Ok(v) => out.push((key, v)),
                    Err(_) => *ignored += 1,
                }
            }
            out
        }
    }
}

/// 把 toml_edit 解析成功的 Table 里所有条目拉平，包括嵌套表（旧格式 [theme] 段下的键）。
fn flatten_table(t: &toml_edit::Table, out: &mut Vec<(String, toml_edit::Value)>) {
    for (k, item) in t.iter() {
        match item {
            toml_edit::Item::Value(v) => out.push((k.to_string(), v.clone())),
            toml_edit::Item::Table(sub) => flatten_table(sub, out),
            _ => {}
        }
    }
}

/// 宽容解析 fallback 用的极简注释剥离。引号内 `#` 不计；不处理转义。
fn strip_comment_fallback(line: &str) -> &str {
    let mut quote: Option<char> = None;
    for (i, c) in line.char_indices() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None => match c {
                '"' | '\'' => quote = Some(c),
                '#' => return &line[..i],
                _ => {}
            },
        }
    }
    line
}

/// 配置文件字节 → 解析结果：剥 BOM、要求合法 UTF-8（非法编码整体拒绝，用默认主题）。
pub fn decode_config(bytes: &[u8]) -> Result<ConfigParse, String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| crate::i18n::t(crate::i18n::Key::ErrConfigNotUtf8).to_string())?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    Ok(parse_config(text))
}

/// 配置文件路径：Windows `%APPDATA%\Veil\config.toml`；其余平台
/// `$XDG_CONFIG_HOME/veil/config.toml` 或 `$HOME/.config/veil/config.toml`。
/// 内部环境变量 `VEIL_HOME` 非空时优先：配置改到 `<VEIL_HOME>/config.toml`
/// （仅供测试/自动化取证隔离，不进入设置界面与用户文档）。
pub fn config_path() -> Option<PathBuf> {
    config_path_with(
        std::env::var_os("VEIL_HOME").filter(|s| !s.is_empty()),
        std::env::var_os("APPDATA").filter(|s| !s.is_empty()),
        std::env::var_os("XDG_CONFIG_HOME").filter(|s| !s.is_empty()),
        std::env::var_os("HOME").filter(|s| !s.is_empty()),
    )
}

/// 应用数据目录（0b 恢复/会话文件的公共父目录）：与配置同根。
/// Windows `%APPDATA%\Veil`；其余 `$XDG_CONFIG_HOME/veil` 或 `$HOME/.config/veil`。
/// `VEIL_HOME` 非空时即为该目录本身（同 `config_path` 的隔离用途）。
pub fn data_dir() -> Option<PathBuf> {
    data_dir_with(
        std::env::var_os("VEIL_HOME").filter(|s| !s.is_empty()),
        std::env::var_os("APPDATA").filter(|s| !s.is_empty()),
        std::env::var_os("XDG_CONFIG_HOME").filter(|s| !s.is_empty()),
        std::env::var_os("HOME").filter(|s| !s.is_empty()),
    )
}

/// 配置路径推导（含 `VEIL_HOME` 覆盖）的纯函数形态：空串视为未设置，回落旧行为。
pub fn config_path_with(
    veil_home: Option<OsString>,
    appdata: Option<OsString>,
    xdg: Option<OsString>,
    home: Option<OsString>,
) -> Option<PathBuf> {
    if let Some(dir) = veil_home.filter(|s| !s.is_empty()) {
        return Some(PathBuf::from(dir).join("config.toml"));
    }
    config_path_of(appdata, xdg, home)
}

/// 数据目录推导（含 `VEIL_HOME` 覆盖）的纯函数形态：空串视为未设置，回落旧行为。
pub fn data_dir_with(
    veil_home: Option<OsString>,
    appdata: Option<OsString>,
    xdg: Option<OsString>,
    home: Option<OsString>,
) -> Option<PathBuf> {
    if let Some(dir) = veil_home.filter(|s| !s.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    data_dir_of(appdata, xdg, home)
}

/// 数据目录推导的纯函数形态（测试注入环境变量，不触碰真实用户目录）。
pub fn data_dir_of(
    appdata: Option<OsString>,
    xdg: Option<OsString>,
    home: Option<OsString>,
) -> Option<PathBuf> {
    if let Some(dir) = appdata {
        return Some(PathBuf::from(dir).join("Veil"));
    }
    if let Some(dir) = xdg {
        return Some(PathBuf::from(dir).join("veil"));
    }
    home.map(|dir| PathBuf::from(dir).join(".config").join("veil"))
}

/// 路径推导的纯函数形态（测试注入环境变量，不触碰真实用户目录）。
pub fn config_path_of(
    appdata: Option<OsString>,
    xdg: Option<OsString>,
    home: Option<OsString>,
) -> Option<PathBuf> {
    if let Some(dir) = appdata {
        return Some(PathBuf::from(dir).join("Veil").join("config.toml"));
    }
    if let Some(dir) = xdg {
        return Some(PathBuf::from(dir).join("veil").join("config.toml"));
    }
    home.map(|dir| {
        PathBuf::from(dir)
            .join(".config")
            .join("veil")
            .join("config.toml")
    })
}

/// 配置读写抽象（切片 17）：实现由产品层注入，`save` 须原子写并按需创建父目录。
/// `load` 在文件不存在时返回 Err（正常状态：调用方静默使用默认配置）。
pub trait ConfigStore {
    fn load(&mut self) -> Result<String, String>;
    fn save(&mut self, text: &str) -> Result<(), String>;
}

/// 原生窗口句柄（v1 仅 Windows）：深色标题栏 / Mica 背景需要 HWND。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct NativeWindow {
    pub hwnd: isize,
}

/// 系统外观抽象：实现由产品层注入（Windows 读注册表 `AppsUseLightTheme`）。
/// 读取失败返回 None（按浅色处理，不影响其它功能）。
pub trait SystemTheme {
    fn is_dark(&mut self) -> Option<bool>;

    /// 系统是否启用界面动画（Windows「显示动画」设置，S6）。
    /// 默认 true：不关心此能力的实现无需改动；产品层读系统设置覆盖。
    fn animations_enabled(&mut self) -> bool {
        true
    }
}

/// 窗口系统外观效果：深色标题栏与 Mica 背景材质。失败静默——内容区始终绘制不透明底色，
/// 因此 Win10/无 Mica 环境下自动退化为纯色外观，无需调用方分支。
pub trait WindowEffects {
    fn apply(&mut self, window: NativeWindow, dark: bool);

    /// 把窗口类背景画刷设为指定 RGB：首帧呈现前任何未绘制区域显示主题底色，
    /// 而不是系统默认的黑色。默认空实现，平台/测试无需改动。
    fn set_background(&mut self, _window: NativeWindow, _color: [u8; 3]) {}
}

/// 主题相关平台能力集合（产品层一次性注入，避免 `run` 参数继续膨胀）。
pub struct ThemeEnv {
    pub config: Box<dyn ConfigStore>,
    pub system: Box<dyn SystemTheme>,
    pub effects: Box<dyn WindowEffects>,
    /// 自定义 Callout 表（`%APPDATA%\Veil\callouts.json`，产品层读取；空表 = 仅内置）。
    pub callouts: veil_core::CalloutRegistry,
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    /// 无配置文件（load 恒 Err、save 静默成功）。
    pub struct NoConfig;
    impl ConfigStore for NoConfig {
        fn load(&mut self) -> Result<String, String> {
            Err("测试无配置文件".into())
        }
        fn save(&mut self, _text: &str) -> Result<(), String> {
            Ok(())
        }
    }

    /// 固定系统外观（None = 读取不可用）。
    pub struct FixedSystem(pub Option<bool>);
    impl SystemTheme for FixedSystem {
        fn is_dark(&mut self) -> Option<bool> {
            self.0
        }
    }

    pub struct NoEffects;
    impl WindowEffects for NoEffects {
        fn apply(&mut self, _window: NativeWindow, _dark: bool) {}
    }

    /// 既有测试的默认主题环境：无配置、系统外观不可读、无窗口效果。
    pub fn env() -> ThemeEnv {
        ThemeEnv {
            config: Box::new(NoConfig),
            system: Box::new(FixedSystem(None)),
            effects: Box::new(NoEffects),
            callouts: veil_core::CalloutRegistry::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> ConfigParse {
        parse_config(text)
    }

    #[test]
    fn empty_config_defaults_to_system_without_notice() {
        let p = parse("");
        assert_eq!(p.config.mode, ThemeMode::System);
        assert_eq!(p.ignored, 0);
        assert_eq!(parse("# 只有注释\n\n").ignored, 0);
    }

    #[test]
    fn reads_mode_key_plain_and_legacy_section() {
        assert_eq!(parse("mode = \"dark\"").config.mode, ThemeMode::Dark);
        assert_eq!(parse("mode = 'light'").config.mode, ThemeMode::Light);
        assert_eq!(parse("mode = \"system\"").config.mode, ThemeMode::System);
        // 历史格式段标题行跳过，不产生 ignored
        let p = parse("[theme]\nmode = \"dark\"");
        assert_eq!((p.config.mode, p.ignored), (ThemeMode::Dark, 0));
    }

    #[test]
    fn ignores_unknown_keys_sections_and_bare_values() {
        // 裸值不是 TOML 字符串；未知键/无等号行均计入 ignored；段标题行静默跳过
        let p = parse("mode = dark\n[theme]\nfoo = \"x\"\nmode = \"light\"\nbar = 1\n");
        assert_eq!(p.config.mode, ThemeMode::Light);
        assert_eq!(p.ignored, 3, "顶层裸值 mode + foo + bar");
        assert_eq!(parse("[other]\nmode = \"dark\"\n").ignored, 0);
    }

    #[test]
    fn invalid_mode_value_is_ignored_and_keeps_default() {
        let p = parse("[theme]\nmode = \"auto\"");
        assert_eq!(p.config.mode, ThemeMode::System);
        assert_eq!(p.ignored, 1);
    }

    #[test]
    fn comments_respect_quotes_and_crlf() {
        let p = parse("[theme] # 段注释\r\nmode = \"dark\" # 行尾注释\r\n");
        assert_eq!(p.config.mode, ThemeMode::Dark);
        assert_eq!(p.ignored, 0);
        // 引号内的 # 属于值；该值非法 → 忽略而非误切
        assert_eq!(parse("[theme]\nmode = \"da#rk\"").ignored, 1);
    }

    #[test]
    fn duplicate_mode_last_wins() {
        let p = parse("[theme]\nmode = \"light\"\nmode = \"dark\"");
        assert_eq!(p.config.mode, ThemeMode::Dark);
        assert_eq!(p.ignored, 0);
    }

    #[test]
    fn decode_rejects_invalid_utf8_and_strips_bom() {
        assert!(decode_config(&[0xff, 0xfe, 0x00]).is_err());
        let bytes = b"\xef\xbb\xbf[theme]\nmode = \"dark\"\n";
        assert_eq!(decode_config(bytes).unwrap().config.mode, ThemeMode::Dark);
    }

    #[test]
    fn config_round_trips_through_toml() {
        for mode in [ThemeMode::System, ThemeMode::Light, ThemeMode::Dark] {
            let cfg = Config {
                mode,
                export_margin_mm: 12.5,
                export_page_numbers: false,
                export_toc: true,
                ..Config::default()
            };
            let text = cfg.to_toml();
            assert!(!text.contains("[theme]"), "新版配置不再写段标题");
            assert!(!text.contains("window_"), "无几何时不写窗口键");
            let p = parse(&text);
            assert_eq!(p.config, cfg);
            assert_eq!(p.ignored, 0, "自产配置不得含无法识别条目");
        }
    }

    #[test]
    fn save_preserves_comments_unknown_keys_and_order() {
        let original =
            "# 手写注释\nmode = \"light\" # 行尾注释\nmy_unknown = 123\ninline = { a = 1 }\n";
        let cfg = Config {
            mode: ThemeMode::Dark,
            export_toc: false,
            ..Config::default()
        };
        let out = cfg.to_toml_preserving(Some(original));
        assert!(out.contains("# 手写注释"), "{out}");
        assert!(out.contains("my_unknown = 123"), "{out}");
        assert!(out.contains("inline = { a = 1 }"), "{out}");
        assert!(out.contains("mode = \"dark\""), "{out}");
        let p = parse(&out);
        assert_eq!(p.config, cfg, "保存后重新加载必须等价");
        assert_eq!(p.ignored, 2, "未知键与内联表仍按既有口径计数");
    }

    #[test]
    fn save_falls_back_to_regeneration_for_non_strict_input() {
        let cfg = Config {
            mode: ThemeMode::Dark,
            ..Config::default()
        };
        // 非法值、重复键、任何表段（含历史 [theme]）都必须回退整文件重生成。
        for original in [
            "mode = \nexport_toc = true\n",
            "mode = \"dark\"\nmode = \"light\"\n",
            "[theme]\nmode = \"dark\"\n",
            "[other]\nfoo = 1\n",
        ] {
            assert_eq!(
                cfg.to_toml_preserving(Some(original)),
                cfg.to_toml(),
                "应回退：{original:?}"
            );
        }
        assert_eq!(cfg.to_toml_preserving(None), cfg.to_toml());
    }

    #[test]
    fn config_round_trips_window_geometry() {
        let cfg = Config {
            mode: ThemeMode::Dark,
            export_margin_mm: 15.0,
            export_page_numbers: false,
            export_toc: true,
            window: Some(WindowGeom {
                width: 1200,
                height: 800,
                x: -40,
                y: 60,
            }),
            ..Config::default()
        };
        let p = parse(&cfg.to_toml());
        assert_eq!(p.config, cfg);
        assert_eq!(p.ignored, 0);
    }

    #[test]
    fn legacy_config_without_window_keys_has_no_geometry() {
        let p = parse(
            "mode = \"light\"\nexport_margin_mm = 10\nexport_page_numbers = false\nexport_toc = false\n",
        );
        assert_eq!(p.config.window, None);
        assert_eq!(p.ignored, 0);
    }

    #[test]
    fn partial_window_keys_are_ignored_as_a_group() {
        let p = parse("window_width = 1200\nwindow_height = 800\n");
        assert_eq!(p.config.window, None);
        assert_eq!(p.ignored, 2, "已出现的两个窗口键各计一次");
    }

    #[test]
    fn out_of_range_window_width_is_ignored() {
        let p = parse("window_width = 0\n");
        assert_eq!(p.config.window, None);
        assert_eq!(p.ignored, 1, "只出现 width：非法值计一次");
    }

    #[test]
    fn minimum_window_size_is_480_by_360() {
        let ok = parse("window_width = 480\nwindow_height = 360\nwindow_x = 0\nwindow_y = 0\n");
        assert_eq!(
            ok.config.window,
            Some(WindowGeom {
                width: 480,
                height: 360,
                x: 0,
                y: 0
            })
        );
        let bad = parse("window_width = 480\nwindow_height = 359\nwindow_x = 0\nwindow_y = 0\n");
        assert_eq!(bad.config.window, None);
        assert_eq!(bad.ignored, 4, "四键齐全但 height 越界：整组计四次");
    }

    #[test]
    fn missing_export_keys_use_defaults() {
        let p = parse("mode = \"dark\"\n");
        assert_eq!(p.ignored, 0);
        assert_eq!(p.config.export_margin_mm, 20.0);
        assert!(p.config.export_page_numbers);
        assert!(p.config.export_toc);
        assert_eq!(p.config.mode, ThemeMode::Dark);
    }

    #[test]
    fn invalid_export_values_are_ignored_and_keep_defaults() {
        let p = parse("export_margin_mm = abc\nexport_page_numbers = \"true\"\nexport_toc = yes\n");
        assert_eq!(p.ignored, 3);
        assert_eq!(p.config, Config::default());
        // 负/非有限边距同样拒绝
        assert_eq!(parse("export_margin_mm = -1").ignored, 1);
        assert_eq!(
            parse("export_margin_mm = inf").config.export_margin_mm,
            20.0
        );
    }

    #[test]
    fn doc_typography_keys_round_trip_and_default_when_missing() {
        // 三键齐全往返
        let cfg = Config {
            doc_font_family: DocFontFamily::Serif,
            doc_font_size: 18.0,
            doc_line_height: 27.0,
            ..Config::default()
        };
        let text = cfg.to_toml();
        for key in ["doc_font_family", "doc_font_size", "doc_line_height"] {
            assert!(text.contains(key), "序列化必须包含 {key}");
        }
        let p = parse(&text);
        assert_eq!(p.config, cfg);
        assert_eq!(p.ignored, 0);
        // 旧配置缺三键 → 全部取默认值且不产生 ignored
        let p = parse("mode = \"dark\"\n");
        assert_eq!(p.config.doc_font_family, DocFontFamily::System);
        assert_eq!(p.config.doc_font_size, 16.0);
        assert_eq!(p.config.doc_line_height, 22.0);
        assert_eq!(p.ignored, 0);
        // 字体族四值严格互逆
        for f in [
            DocFontFamily::System,
            DocFontFamily::Sans,
            DocFontFamily::Serif,
            DocFontFamily::Mono,
        ] {
            assert_eq!(DocFontFamily::parse(f.as_str()), Some(f));
        }
    }

    #[test]
    fn doc_typography_values_are_clamped_never_panic() {
        // 字号下限/上限
        let p = parse("doc_font_size = 0\ndoc_line_height = 0\n");
        assert_eq!(
            (p.config.doc_font_size, p.config.doc_line_height),
            (10.0, 10.0)
        );
        assert_eq!(p.ignored, 0, "越界是夹取而非忽略");
        let p = parse("doc_font_size = 999\ndoc_line_height = 99999\n");
        assert_eq!(
            (p.config.doc_font_size, p.config.doc_line_height),
            (32.0, 96.0)
        );
        assert_eq!(p.ignored, 0);
        // 行高小于字号 → 抬到字号；超过 3×字号 → 压到 3×
        assert_eq!(
            parse("doc_font_size = 20\ndoc_line_height = 5")
                .config
                .doc_line_height,
            20.0
        );
        assert_eq!(
            parse("doc_font_size = 20\ndoc_line_height = 100")
                .config
                .doc_line_height,
            60.0
        );
        // 与键出现顺序无关（行高先出现也按最终字号夹取）
        let p = parse("doc_line_height = 100\ndoc_font_size = 20\n");
        assert_eq!(
            (p.config.doc_font_size, p.config.doc_line_height),
            (20.0, 60.0)
        );
        // 非数值：忽略并回落默认
        let p = parse("doc_font_size = abc\ndoc_line_height = inf\n");
        assert_eq!(
            (p.config.doc_font_size, p.config.doc_line_height),
            (16.0, 22.0)
        );
        assert_eq!(p.ignored, 2);
    }

    #[test]
    fn unknown_or_overlong_font_family_is_ignored() {
        // 未知值 / 超长值 / 裸值均不可识别，回落默认且计数
        let overlong = format!("doc_font_family = \"{}\"", "x".repeat(65));
        for text in [
            "doc_font_family = \"fantasy\"".to_string(),
            overlong,
            "doc_font_family = system".to_string(),
        ] {
            let p = parse(&text);
            assert_eq!(p.config.doc_font_family, DocFontFamily::System, "{text}");
            assert_eq!(p.ignored, 1, "{text}");
        }
        // 合法值（含 64 长度内的已知值）正常读取
        let p = parse("doc_font_family = \"mono\"");
        assert_eq!(p.config.doc_font_family, DocFontFamily::Mono);
        assert_eq!(p.ignored, 0);
    }

    /// 旧配置 `smooth_scroll` 已删除：任何值都静默忽略（不改变行为、不产生「无法识别」提示）。
    #[test]
    fn legacy_smooth_scroll_key_is_silently_ignored() {
        for text in [
            "smooth_scroll = true\n",
            "smooth_scroll = false\n",
            "smooth_scroll = \"yes\"\n",
        ] {
            let p = parse(text);
            assert_eq!(p.ignored, 0, "旧键不得产生无法识别提示：{text}");
            assert_eq!(p.config, Config::default(), "旧键不得改变任何行为：{text}");
        }
        // 新序列化不再写出该键
        assert!(!Config::default().to_toml().contains("smooth_scroll"));
    }

    #[test]
    fn allow_remote_images_defaults_false_and_parses_explicit_bool() {
        // 空配置 / 旧配置缺键：默认阻止，且不产生 ignored
        let p = parse("");
        assert!(!p.config.allow_remote_images);
        assert_eq!(p.ignored, 0);
        assert!(!parse("mode = \"dark\"\n").config.allow_remote_images);
        // 用户显式关闭 / 打开的既有配置均保留原语义
        assert!(
            !parse("allow_remote_images = false")
                .config
                .allow_remote_images
        );
        assert!(
            parse("allow_remote_images = true")
                .config
                .allow_remote_images
        );
        // 非法字面量：回落安全默认并计数
        let p = parse("allow_remote_images = \"false\"");
        assert!(!p.config.allow_remote_images);
        assert_eq!(p.ignored, 1);
    }

    #[test]
    fn image_cache_limit_defaults_and_parses_integer() {
        // 空配置 / 旧配置缺键：默认 200 MiB，且不产生 ignored
        let p = parse("");
        assert_eq!(p.config.image_cache_limit_mb, 200);
        assert_eq!(p.ignored, 0);
        assert_eq!(
            parse("image_cache_limit_mb = 512\n")
                .config
                .image_cache_limit_mb,
            512
        );
        // 0 = 不限制
        assert_eq!(
            parse("image_cache_limit_mb = 0\n")
                .config
                .image_cache_limit_mb,
            0
        );
        // 非法（负数/超上限/字符串/浮点）：回落默认并计数
        for text in [
            "image_cache_limit_mb = -1",
            "image_cache_limit_mb = 4097",
            "image_cache_limit_mb = \"200\"",
            "image_cache_limit_mb = 1.5",
        ] {
            let p = parse(text);
            assert_eq!(p.config.image_cache_limit_mb, 200, "{text}");
            assert_eq!(p.ignored, 1, "{text}");
        }
    }

    #[test]
    fn image_cache_limit_round_trips_through_toml() {
        for limit in [0u32, 200, 512] {
            let cfg = Config {
                image_cache_limit_mb: limit,
                ..Config::default()
            };
            let text = cfg.to_toml();
            assert!(
                text.contains(&format!("image_cache_limit_mb = {limit}")),
                "序列化必须写 {limit}: {text}"
            );
            let p = parse(&text);
            assert_eq!(p.config, cfg, "往返恒等");
            assert_eq!(p.ignored, 0);
        }
    }

    #[test]
    fn allow_remote_images_round_trips_through_toml() {
        for allow in [true, false] {
            let cfg = Config {
                allow_remote_images: allow,
                ..Config::default()
            };
            let text = cfg.to_toml();
            assert!(
                text.contains(&format!("allow_remote_images = {allow}")),
                "序列化必须写 {allow}: {text}"
            );
            let p = parse(&text);
            assert_eq!(p.config, cfg, "往返恒等");
            assert_eq!(p.ignored, 0);
        }
    }

    /// 旧配置 `mermaid_insert_url` 已删除：无论 true/false/非法类型都静默忽略，
    /// 不计入 ignored（不打扰用户），也不再写回配置文件。
    #[test]
    fn legacy_mermaid_insert_url_key_is_silently_ignored() {
        for text in [
            "mermaid_insert_url = true\n",
            "mermaid_insert_url = false\n",
            "mermaid_insert_url = \"yes\"\n",
        ] {
            let p = parse(text);
            assert_eq!(p.ignored, 0, "旧键不得产生无法识别提示：{text}");
            assert_eq!(p.config, Config::default(), "旧键不得改变任何行为：{text}");
        }
        let out = Config::default().to_toml();
        assert!(!out.contains("mermaid_insert_url"), "{out}");
        assert_eq!(parse(&out).ignored, 0);
    }

    #[test]
    fn mode_str_round_trips() {
        for mode in [ThemeMode::System, ThemeMode::Light, ThemeMode::Dark] {
            assert_eq!(ThemeMode::parse(mode.as_str()), Some(mode));
        }
        assert_eq!(ThemeMode::parse("Dark"), None, "严格小写");
    }

    #[test]
    fn light_palette_keeps_legacy_colors() {
        let t = Theme::light();
        assert!(!t.dark);
        assert_eq!(t.bg, [255, 255, 255, 255]);
        assert_eq!(t.fg, [30, 30, 30]);
        assert_eq!(t.chrome_bg, [248, 248, 248, 255]);
        assert_eq!(t.chrome_border, [208, 213, 221, 255]);
        assert_eq!(t.hover, [228, 236, 246, 255]);
        assert_eq!(t.popup_bg, [255, 255, 255, 255]);
        assert_eq!(t.thumb, [176, 184, 195, 255]);
        assert_eq!(t.muted, [110, 110, 110]);
        assert_eq!(t.text, [45, 45, 45]);
        assert_eq!(t.link, [43, 108, 176]);
    }

    #[test]
    fn link_colors_are_distinct_from_body_text() {
        assert_eq!(Theme::dark().link, [127, 180, 245]);
        for t in [Theme::light(), Theme::dark()] {
            assert_ne!(t.link, t.text, "链接色必须与菜单/正文文字色不同");
            assert_ne!(t.link, t.fg, "链接色必须与正文前景色不同");
        }
    }

    #[test]
    fn mode_resolution_follows_system_only_when_asked() {
        assert!(Theme::of(ThemeMode::System, true).dark);
        assert!(!Theme::of(ThemeMode::System, false).dark);
        assert!(!Theme::of(ThemeMode::Light, true).dark);
        assert!(Theme::of(ThemeMode::Dark, false).dark);
        assert_eq!(Theme::of(ThemeMode::Dark, false).mode, ThemeMode::Dark);
    }

    /// WCAG 相对亮度（sRGB → 线性）。
    fn luminance(rgb: [u8; 3]) -> f32 {
        let ch = |v: u8| {
            let u = v as f32 / 255.0;
            if u <= 0.039_28 {
                u / 12.92
            } else {
                ((u + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * ch(rgb[0]) + 0.7152 * ch(rgb[1]) + 0.0722 * ch(rgb[2])
    }

    fn contrast(a: [u8; 3], b: [u8; 3]) -> f32 {
        let (l1, l2) = (luminance(a), luminance(b));
        let (hi, lo) = if l1 > l2 { (l1, l2) } else { (l2, l1) };
        (hi + 0.05) / (lo + 0.05)
    }

    /// 4 字节色（含 alpha）→ 不透明 RGB：绘制底均为不透明色，对比度只取决于 RGB。
    fn rgb(c: [u8; 4]) -> [u8; 3] {
        [c[0], c[1], c[2]]
    }

    #[test]
    fn palettes_meet_text_contrast_targets() {
        for t in [Theme::light(), Theme::dark()] {
            let bg = [t.bg[0], t.bg[1], t.bg[2]];
            let chrome = [t.chrome_bg[0], t.chrome_bg[1], t.chrome_bg[2]];
            let popup = [t.popup_bg[0], t.popup_bg[1], t.popup_bg[2]];
            assert!(
                contrast(t.fg, bg) >= 7.0,
                "正文对比度不足: {:?} on {:?}",
                t.fg,
                bg
            );
            assert!(
                contrast(t.text, popup) >= 7.0,
                "菜单文字对比度不足: dark={}",
                t.dark
            );
            assert!(
                contrast(t.muted, chrome) >= 4.5,
                "状态栏次要文字对比度不足: dark={}",
                t.dark
            );
        }
    }

    #[test]
    fn syntax_and_grid_meet_contrast_targets() {
        // spec「对比度达标」：正文 ≥7:1、次要文本/语法注释 ≥4.5:1、非文本装饰线 ≥3:1，
        // 明暗两主题同时达标。语法注释出现在代码块（code_bg），也可能落在正文底（bg）；
        // grid 画表格线/引用竖条/分隔线，均直接叠在 bg 上。
        for t in [Theme::light(), Theme::dark()] {
            let bg = rgb(t.bg);
            let code_bg = rgb(t.code_bg);
            let grid = rgb(t.grid);
            let comment = t.syntax.comment;

            let c_fg = contrast(t.fg, bg);
            assert!(
                c_fg >= 7.0,
                "正文/背景对比度不足: dark={} {c_fg:.2}",
                t.dark
            );

            let c_muted = contrast(t.muted, bg);
            assert!(
                c_muted >= 4.5,
                "次要文本/背景对比度不足: dark={} {c_muted:.2}",
                t.dark
            );

            let c_comment_code = contrast(comment, code_bg);
            assert!(
                c_comment_code >= 4.5,
                "语法注释/code_bg 对比度不足: dark={} {c_comment_code:.2}",
                t.dark
            );

            let c_comment_bg = contrast(comment, bg);
            assert!(
                c_comment_bg >= 4.5,
                "语法注释/背景对比度不足: dark={} {c_comment_bg:.2}",
                t.dark
            );

            let c_grid = contrast(grid, bg);
            assert!(
                c_grid >= 3.0,
                "grid/背景对比度不足: dark={} {c_grid:.2}",
                t.dark
            );

            // 表格线同时压在表头底色上，同样按非文本装饰线门禁校核。
            let c_grid_head = contrast(grid, rgb(t.table_head_bg));
            assert!(
                c_grid_head >= 3.0,
                "grid/table_head_bg 对比度不足: dark={} {c_grid_head:.2}",
                t.dark
            );
        }
        // 无主题兜底色板与浅色主题同值，避免两条路径出现不同注释色。
        assert_eq!(
            SyntaxColors::default_light().comment,
            Theme::light().syntax.comment
        );
    }

    #[test]
    fn danger_colors_meet_contrast_targets() {
        // 危险操作文字/描边必须在各自底上可读（模态删除按钮的可用性门禁）。
        for t in [Theme::light(), Theme::dark()] {
            let bg = [t.bg[0], t.bg[1], t.bg[2]];
            let c = contrast(t.danger, bg);
            assert!(c >= 4.5, "danger/bg 对比度不足: dark={} {c:.2}", t.dark);
        }
    }

    #[test]
    fn accent_colors_carry_label_text() {
        // 主按钮是「accent 填充 + accent_text 文字」：两者对比度必须达标，
        // 且填充色在卡片底上要能看出边界（否则白卡上的主按钮会「消失」）。
        for t in [Theme::light(), Theme::dark()] {
            let c = contrast(t.accent_text, t.accent);
            assert!(
                c >= 4.5,
                "accent_text/accent 对比度不足: dark={} {c:.2}",
                t.dark
            );
            let popup = [t.popup_bg[0], t.popup_bg[1], t.popup_bg[2]];
            let edge = contrast(t.accent, popup);
            assert!(
                edge >= 2.0,
                "accent/popup 边界对比不足: dark={} {edge:.2}",
                t.dark
            );
        }
    }

    #[test]
    fn surface_pairs_meet_contrast_targets() {
        // 浮层（surface）三组关键配色的 WCAG 对比度门禁：
        // 正文级文字（text / muted 在 popup_bg 上）≥ 4.5；
        // 悬停态（text 在 hover 上，短暂交互态，按大字级放宽）≥ 3.0。
        // 若调整 muted / hover 取值，必须保持本断言成立。
        for t in [Theme::light(), Theme::dark()] {
            let popup = [t.popup_bg[0], t.popup_bg[1], t.popup_bg[2]];
            let hover = [t.hover[0], t.hover[1], t.hover[2]];
            let c_text_popup = contrast(t.text, popup);
            let c_muted_popup = contrast(t.muted, popup);
            let c_text_hover = contrast(t.text, hover);
            assert!(
                c_text_popup >= 4.5,
                "text/popup_bg 对比度不足: dark={} {c_text_popup:.2}",
                t.dark
            );
            assert!(
                c_muted_popup >= 4.5,
                "muted/popup_bg 对比度不足: dark={} {c_muted_popup:.2}",
                t.dark
            );
            assert!(
                c_text_hover >= 3.0,
                "text/hover 对比度不足: dark={} {c_text_hover:.2}",
                t.dark
            );
        }
    }

    #[test]
    fn surface_levels_map_to_correct_fields() {
        // S5 语义层门禁：SurfaceLevel 三档必须精确映射到 Theme 字段。
        for t in [Theme::light(), Theme::dark()] {
            assert_eq!(t.surface(SurfaceLevel::Base), t.bg);
            assert_eq!(t.surface(SurfaceLevel::Card), t.chrome_bg);
            assert_eq!(t.surface(SurfaceLevel::Popup), t.popup_bg);
            // Card 档对比度门禁：text/muted 必须在 chrome_bg 上 ≥4.5
            let card = [t.chrome_bg[0], t.chrome_bg[1], t.chrome_bg[2]];
            let c_text = contrast(t.text, card);
            let c_muted = contrast(t.muted, card);
            assert!(
                c_text >= 4.5,
                "Card text/chrome_bg 对比度不足: dark={} {c_text:.2}",
                t.dark
            );
            assert!(
                c_muted >= 4.5,
                "Card muted/chrome_bg 对比度不足: dark={} {c_muted:.2}",
                t.dark
            );
        }
    }

    #[test]
    fn surface_tokens_scale_linearly() {
        // 2.0× 时几何令牌数值翻倍
        assert_eq!(surface::corner_radius(2.0), 12.0);
        assert_eq!(surface::row_height(2.0), 56.0);
        assert_eq!(surface::padding(2.0), 24.0);
        assert_eq!(surface::hover_inset(2.0), 6.0);
        assert_eq!(surface::shadow_offset_y(2.0), 4.0);
        assert_eq!(surface::panel_margin(2.0), 16.0);
        assert_eq!(surface::control_gap(2.0), 16.0);
        assert_eq!(surface::caret_width(2.0), 4.0);
        assert_eq!(surface::caret_height(2.0), 32.0);
        assert_eq!(surface::scrollbar_min_thumb(2.0), 48.0);
        // 1.0× 即基准常量本身
        assert_eq!(surface::corner_radius(1.0), surface::CORNER_RADIUS);
        // 阴影 alpha：浅色不变，深色按增益放大并钳制
        assert_eq!(surface::shadow_alpha(20, false), 20);
        assert_eq!(surface::shadow_alpha(20, true), 25); // 20 × 1.25
    }

    #[test]
    fn export_style_maps_theme_colors() {
        let light = ExportStyle::from_theme(&Theme::light());
        assert!(!light.dark);
        assert_eq!(light.bg, [255, 255, 255]);
        assert_eq!(light.text, [30, 30, 30]);
        assert_eq!(light.muted, Theme::light().muted);
        assert_eq!(light.link, Theme::light().link);
        // code_bg / border 有对应主题字段：必须逐字节复用，不在导出侧另起一套色
        assert_eq!(light.code_bg, [245, 245, 245]);
        assert_eq!(light.border, [208, 213, 221]);
        let dark = ExportStyle::from_theme(&Theme::dark());
        assert!(dark.dark);
        assert_eq!(dark.bg, [30, 30, 30]);
        assert_eq!(dark.text, [220, 220, 220]); // 取 fg（正文前景），非菜单文字色
        assert_eq!(dark.code_bg, [40, 40, 40]);
        assert_eq!(dark.border, [60, 64, 70]);
        // Default 与主题无关：浅色兜底值固定，供未注入主题的调用方使用
        assert_eq!(ExportStyle::default().bg, [255, 255, 255]);
        assert!(!ExportStyle::default().dark);
    }

    #[test]
    fn config_path_prefers_appdata_then_xdg_then_home() {
        let appdata = OsString::from(r"C:\Users\u\AppData\Roaming");
        let xdg = OsString::from("/home/u/.config");
        let home = OsString::from("/home/u");
        assert_eq!(
            config_path_of(Some(appdata.clone()), Some(xdg.clone()), Some(home.clone())),
            Some(PathBuf::from(&appdata).join("Veil").join("config.toml"))
        );
        assert_eq!(
            config_path_of(None, Some(xdg.clone()), Some(home.clone())),
            Some(PathBuf::from(&xdg).join("veil").join("config.toml"))
        );
        assert_eq!(
            config_path_of(None, None, Some(home.clone())),
            Some(
                PathBuf::from(&home)
                    .join(".config")
                    .join("veil")
                    .join("config.toml")
            )
        );
        assert_eq!(config_path_of(None, None, None), None);
    }

    /// 取证隔离：`VEIL_HOME` 非空时配置与数据目录都落到该根下，压过 APPDATA/XDG/HOME。
    #[test]
    fn veil_home_overrides_config_and_data_paths() {
        let veil_home = OsString::from(r"D:\tmp\veil-home");
        let appdata = OsString::from(r"C:\Users\u\AppData\Roaming");
        let xdg = OsString::from("/home/u/.config");
        let home = OsString::from("/home/u");
        assert_eq!(
            config_path_with(
                Some(veil_home.clone()),
                Some(appdata.clone()),
                Some(xdg.clone()),
                Some(home.clone())
            ),
            Some(PathBuf::from(&veil_home).join("config.toml"))
        );
        assert_eq!(
            data_dir_with(
                Some(veil_home.clone()),
                Some(appdata.clone()),
                Some(xdg.clone()),
                Some(home.clone())
            ),
            Some(PathBuf::from(&veil_home))
        );
    }

    /// 空字符串等同未设置：不得生成相对路径，必须回落到旧行为。
    #[test]
    fn empty_veil_home_falls_back_to_legacy_paths() {
        let empty = OsString::from("");
        let appdata = OsString::from(r"C:\Users\u\AppData\Roaming");
        let xdg = OsString::from("/home/u/.config");
        let home = OsString::from("/home/u");
        assert_eq!(
            config_path_with(
                Some(empty.clone()),
                Some(appdata.clone()),
                Some(xdg.clone()),
                Some(home.clone())
            ),
            config_path_of(Some(appdata.clone()), Some(xdg.clone()), Some(home.clone()))
        );
        assert_eq!(
            data_dir_with(
                Some(empty.clone()),
                Some(appdata.clone()),
                Some(xdg.clone()),
                Some(home.clone())
            ),
            data_dir_of(Some(appdata.clone()), Some(xdg.clone()), Some(home.clone()))
        );
    }

    /// 未设置 `VEIL_HOME` 时与旧推导逐字节一致。
    #[test]
    fn unset_veil_home_matches_legacy_paths() {
        let appdata = OsString::from(r"C:\Users\u\AppData\Roaming");
        let xdg = OsString::from("/home/u/.config");
        let home = OsString::from("/home/u");
        for (a, x, h) in [
            (Some(appdata.clone()), Some(xdg.clone()), Some(home.clone())),
            (None, Some(xdg.clone()), Some(home.clone())),
            (None, None, Some(home.clone())),
            (None, None, None),
        ] {
            assert_eq!(
                config_path_with(None, a.clone(), x.clone(), h.clone()),
                config_path_of(a.clone(), x.clone(), h.clone())
            );
            assert_eq!(
                data_dir_with(None, a.clone(), x.clone(), h.clone()),
                data_dir_of(a.clone(), x.clone(), h.clone())
            );
        }
    }
}
