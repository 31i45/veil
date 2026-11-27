//! 主题与配置（切片 17）：调色板、极简 TOML 子集解析、配置路径，以及平台接缝 trait。
//!
//! 零 GUI 依赖：解析/序列化/路径推导/主题判定全部为纯函数，headless 可测。
//! 平台能力（注册表系统外观、DWM 深色标题栏/Mica、配置落盘）以 trait 声明，由产品层注入。

//!
//! 配置格式（极简，共 17 个顶层键）：
//! ```toml
//! mode = "system"        # system | light | dark
//! ui_language = "auto"   # auto | zh-CN | en-US（重启生效）
//! export_margin_mm = 20  # PDF 页边距（毫米）
//! export_page_numbers = true
//! export_toc = true
//! doc_font_family = "system"  # system | sans | serif | mono
//! doc_font_size = 16     # 正文基准字号（物理像素），夹取 10..=32
//! doc_line_height = 22   # 正文基准行高（物理像素），夹取 字号..=字号×3
//! allow_remote_images = false  # 默认阻止普通远程图片；true = 用户显式允许
//! tab_width = 4          # 制表符宽度（2 或 4），默认 4
//! # 图片缓存字节上限固化为常量 DEFAULT_IMAGE_CACHE_LIMIT_MB（200 MiB），不再可配置
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

use crate::i18n::{t, Key};

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

/// 界面语言选择：自动（跟随系统）/ 中文 / English。
///
/// 只在启动时决定 `i18n::init_locale`，会话内不热切换；改动写盘后重启生效。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum UiLanguage {
    #[default]
    Auto,
    ZhCn,
    EnUs,
}

impl UiLanguage {
    /// 配置值解析（严格大小写，与 `as_str` 严格互逆）。
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "auto" => Some(UiLanguage::Auto),
            "zh-CN" => Some(UiLanguage::ZhCn),
            "en-US" => Some(UiLanguage::EnUs),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            UiLanguage::Auto => "auto",
            UiLanguage::ZhCn => "zh-CN",
            UiLanguage::EnUs => "en-US",
        }
    }

    /// 手动选择时的启动语言码；`Auto` 返回 None（交给环境变量/系统语言决定）。
    pub fn code(self) -> Option<&'static str> {
        match self {
            UiLanguage::Auto => None,
            UiLanguage::ZhCn => Some("zh-CN"),
            UiLanguage::EnUs => Some("en-US"),
        }
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

/// plist 解析失败时的兜底色板：与 `nord_light.tmTheme` 的浅色取值逐 token 同源，
/// 避免兜底路径与正常路径出现两套色（一致性由测试锁定）。
const SYNTAX_FALLBACK: SyntaxColors = SyntaxColors {
    keyword: [47, 107, 138],
    string: [76, 122, 79],
    comment: [97, 110, 136],
    number: [122, 90, 140],
    function: [44, 122, 134],
    type_name: [44, 122, 112],
    punctuation: [59, 66, 82],
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
    /// 未匹配到的字段保留兜底值（与浅色 tmTheme 取值一致的常量）。
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
    pub modal_bg: [u8; 4],
    pub thumb: [u8; 4],
    pub muted: [u8; 3],
    pub text: [u8; 3],
    /// `<kbd>` 按键底色。深色下必须是暗底：若沿用浅色 [245,245,245]，字色（fg）
    /// 也是浅色，会形成近乎全白的眩光块，整段文字失去可读性。
    pub kbd_bg: [u8; 4],
    /// `<kbd>` 按键描边色（1px）。
    pub kbd_border: [u8; 3],
    /// `<mark>` 高亮底色（带 alpha）。浅色用黄，深色用暗琥珀，
    /// 保证与 `fg` 的对比度 ≥ 4.5。
    pub mark_bg: [u8; 4],
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

/// Surface 底色四档语义（S5）：Theme 的底色字段按层级归为 Base / Card / Popup / Modal。
///
/// | 档 | Theme 字段 | 含义 |
/// |---|---|---|
/// | Base | bg | 页面/窗口底层背景 |
/// | Card | chrome_bg | chrome 区条带、表头（不弹起） |
/// | Popup | popup_bg | 浮层、弹出（带 Low 档阴影弹起） |
/// | Modal | modal_bg | 模态对话框（带 High 档阴影弹起，最深一层弹起） |
///
/// 代码块底色（code_bg）与表头底色（table_head_bg）是 Card 档的特化，
/// 不归本枚举直接映射——调用方按需经 `tokens::palette::code_bg` / `table_head_bg` 取。
///
/// 层级约束：card 与 popup 的 OKLab 色差 ΔE_OK ≥ 0.02（≈ CIELAB ΔE 2），modal 与
/// popup/card 之一 ≥ 0.01，且 modal_bg ≠ popup_bg；门禁见 `tokens` 的
/// `surface_hierarchy_is_perceptually_separated`。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(dead_code)] // API 预留：paint 层仍经 palette 模块消费，SurfaceLevel 后续批次会有更多使用点（当前仅测试锁定四档映射）
pub enum SurfaceLevel {
    Base,
    Card,
    Popup,
    Modal,
}

impl Theme {
    /// 按档取 surface 底色。
    #[allow(dead_code)] // 同 SurfaceLevel：后续批次消费（当前仅测试锁定四档映射）
    pub fn surface(&self, level: SurfaceLevel) -> [u8; 4] {
        match level {
            SurfaceLevel::Base => self.bg,
            SurfaceLevel::Card => self.chrome_bg,
            SurfaceLevel::Popup => self.popup_bg,
            SurfaceLevel::Modal => self.modal_bg,
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
            // modal_bg 必须与 popup_bg 有可感知色差（OKLab ΔE_OK ≈ 0.015，≈ CIELAB ΔE 1.5）：
            // 冷调灰 [249,250,252] 提供明度差 + 蓝调偏移，靠色彩差异增强弹起层级，
            // 又不过度偏离低干扰基调。
            modal_bg: [249, 250, 252, 255],
            thumb: [176, 184, 195, 255],
            muted: [110, 110, 110],
            text: [45, 45, 45],
            kbd_bg: [245, 245, 245, 255],
            kbd_border: [160, 160, 160],
            mark_bg: [255, 247, 153, 180],
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
            // 深主题靠明度差：modal 比 popup 再亮 3 级，与 popup 有 ΔL ≈ 3.5。
            modal_bg: [49, 49, 49, 255],
            thumb: [96, 102, 110, 255],
            muted: [160, 160, 160],
            text: [225, 225, 225],
            kbd_bg: [58, 58, 58, 255],
            kbd_border: [110, 110, 110],
            // 暗琥珀底：与 fg [220,220,220] 对比度 ≈ 5:1；黄底浅字会糊成一片。
            mark_bg: [122, 96, 0, 210],
            link: [127, 180, 245],
            danger: [255, 130, 130],
            accent: [14, 99, 156],
            accent_text: [255, 255, 255],
            syntax: dark_syntax(),
        }
    }

    /// 高对比度主题（veil-ui 内部预设，覆盖系统 light/dark 外观）。
    /// 遵循 WCAG AAA：前景/底色对比度 ≥ 7:1，按钮/链接强调色对比度 ≥ 4.5:1。
    /// 禁用态使用深灰文字而非半透明——避免灰色透明在高对比底下不可见。
    pub fn high_contrast() -> Self {
        Theme {
            mode: ThemeMode::System,
            dark: true,
            // 纯黑底 + 纯白前景（30:1 对比度，远高于 AAA 7:1）
            bg: [0, 0, 0, 255],
            fg: [255, 255, 255],
            code_bg: [20, 20, 20, 255],
            sel: [0, 80, 255, 180],
            table_head_bg: [30, 30, 30, 255],
            grid: [180, 180, 180, 255],
            chrome_bg: [15, 15, 15, 255],
            chrome_border: [255, 255, 255, 255],
            // 悬停：纯白边框 + 极浅灰底，保证可见性
            hover: [50, 50, 50, 255],
            popup_bg: [10, 10, 10, 255],
            modal_bg: [20, 20, 20, 255],
            thumb: [200, 200, 200, 255],
            // 禁用态：深灰但在黑底上仍有足够对比度（≈ 5:1，接近 AA 4.5）
            muted: [190, 190, 190],
            text: [255, 255, 255],
            kbd_bg: [25, 25, 25, 255],
            kbd_border: [255, 255, 255],
            // 暗黄底 + 纯白字 ≈ 8.6:1（AAA）
            mark_bg: [90, 75, 0, 255],
            // 鲜艳蓝色链接（在黑底上 ≈ 10:1）
            link: [80, 220, 255],
            // 高饱和红色危险色
            danger: [255, 80, 80],
            // 高饱和绿色强调色
            accent: [90, 220, 90],
            accent_text: [0, 0, 0],
            // 语法色：高饱和、各 token 可区分
            syntax: high_contrast_syntax(),
        }
    }
}

/// 浅色语法色板：从 embed 的 nord_light.tmTheme 加载（OnceLock 缓存，进程内只 parse 一次）。
///
/// 该 asset 不是 Nord 原版：Nord 的极光色是为深底（#2E3440）设计的前景色，直接搬到
/// 近白代码块底（#F5F5F5）上只有 1.83~2.60:1。浅色色板因此在保持色相族不变的前提下
/// 只降明度，从深底方案重推导为浅底可读方案（7 个 token 对 code_bg 全部 ≥ 4.5:1）。
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

/// 高对比度语法色板：全部高饱和纯色，保证在纯黑底上各 token 可区分。
/// 硬编码常量（不依赖 tmTheme 文件）。
fn high_contrast_syntax() -> SyntaxColors {
    SyntaxColors {
        keyword: [255, 100, 100],     // 红色：关键词
        string: [130, 255, 130],      // 绿色：字符串
        comment: [200, 200, 200],     // 浅灰：注释（可读但不抢眼）
        number: [255, 200, 80],       // 黄色：数字
        function: [150, 200, 255],    // 淡蓝：函数
        type_name: [255, 255, 130],   // 黄色偏亮：类型名
        punctuation: [220, 220, 220], // 近白：标点
    }
}

pub(crate) use crate::tokens::surface;

/// 正文字体族预置（保持 `Copy`：只存枚举，排版时映射到 `&'static str` / 通用族）。
///
/// `System` 不显式设置 family，沿用既有字体回退链（视觉不变）。
/// 具体已安装字族由 [`Config::doc_font_face`] 单独承载（`None` = 沿用这里四档）。
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

/// 屏幕版心边距档位（与 PDF 导出边距 `export_margin_mm` 完全无关）。
///
/// 只描述「屏幕阅读时的左右留白」；可用宽度超过可读列宽上限时，
/// 除 `Full` 外仍受上限约束并居中。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum DocMargin {
    /// 紧凑：8 逻辑像素。
    Compact,
    /// 标准（默认）：16 逻辑像素，与改造前 `tokens::doc::MARGIN` 一致。
    #[default]
    Standard,
    /// 宽松：32 逻辑像素。
    Relaxed,
    /// 全宽：16 逻辑像素，且不启用可读列宽上限（按窗口铺满）。
    Full,
    /// 自定义：取 `Config::doc_margin_px`（夹取 4..=200 逻辑像素）。
    Custom,
}

impl DocMargin {
    /// 配置值解析（严格小写，与 `as_str` 严格互逆）。
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "compact" => Some(DocMargin::Compact),
            "standard" => Some(DocMargin::Standard),
            "relaxed" => Some(DocMargin::Relaxed),
            "full" => Some(DocMargin::Full),
            "custom" => Some(DocMargin::Custom),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            DocMargin::Compact => "compact",
            DocMargin::Standard => "standard",
            DocMargin::Relaxed => "relaxed",
            DocMargin::Full => "full",
            DocMargin::Custom => "custom",
        }
    }

    /// 屏幕版心边距（逻辑像素，未乘 scale）；`Custom` 用用户值并夹取。
    pub fn margin_px(self, custom_px: f32) -> f32 {
        match self {
            DocMargin::Compact => 8.0,
            DocMargin::Standard | DocMargin::Full => DEFAULT_DOC_MARGIN_PX,
            DocMargin::Relaxed => 32.0,
            DocMargin::Custom => clamp_doc_margin_px(custom_px),
        }
    }

    /// 是否启用可读列宽上限（只有 `Full` 关闭）。
    pub(crate) fn readable_cap(self) -> bool {
        !matches!(self, DocMargin::Full)
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
    /// `<mark>` 底色：`theme.mark_bg`（带 alpha）src-over 到 `theme.bg` 后的
    /// 不透明呈现色。导出 CSS 不保留 alpha，必须在快照里先合成。
    pub mark_bg: [u8; 3],
    /// 危险/失败前景色（公式渲染失败源码等）：取 `theme.danger`。
    pub danger: [u8; 3],
    /// 正文字族名（`None` = 导出沿用浏览器默认 `system-ui`）。
    /// 指向排版引擎内驻留的 `'static` 名字，避免导出端口持有所有权或复制。
    pub font_family: Option<&'static str>,
}

/// 半透明色 `top` src-over 到不透明底 `bottom` 的实际呈现色。
///
/// 导出 CSS 的底色必须是可独立渲染的不透明值（浏览器不会替我们合成到页底），
/// 所以带 alpha 的主题色（如 `mark_bg`）要先在这里按主题底色算出 RGB。
/// 主题底色恒为不透明（alpha = 255），故只取 `bottom` 的 RGB 分量。
fn src_over(top: [u8; 4], bottom: [u8; 4]) -> [u8; 3] {
    let a = top[3] as f32 / 255.0;
    let ch = |t: u8, b: u8| (t as f32 * a + b as f32 * (1.0 - a)).round() as u8;
    [
        ch(top[0], bottom[0]),
        ch(top[1], bottom[1]),
        ch(top[2], bottom[2]),
    ]
}

impl Default for ExportStyle {
    /// 浅色默认：固定色与改造前硬编码的导出样式逐色一致；`mark_bg` / `danger`
    /// 从浅色主题派生（颜色不再在导出侧另起一套）。
    fn default() -> Self {
        ExportStyle {
            dark: false,
            bg: [0xff, 0xff, 0xff],
            text: [0x1e, 0x1e, 0x1e],
            muted: [0x66, 0x66, 0x66],
            link: [0x2b, 0x6c, 0xb0],
            code_bg: [0xf5, 0xf5, 0xf5],
            border: [0xdd, 0xdd, 0xdd],
            mark_bg: src_over(Theme::light().mark_bg, Theme::light().bg),
            danger: Theme::light().danger,
            font_family: None,
        }
    }
}

impl ExportStyle {
    /// 主题 → 导出样式。字段映射：
    /// - `bg`/`text`/`muted`/`link` 直接取同名主题字段（`text` 取 `fg`，正文前景）；
    /// - `code_bg` 复用主题 `code_bg`（编辑器代码块底色，导出与屏幕一致）；
    /// - `border` 复用 `chrome_border`（主题里唯一通用的分界色，表格/引用/分隔线共用）；
    /// - `mark_bg` 把主题 `mark_bg`（带 alpha）src-over 到 `bg`，得到不透明呈现色；
    /// - `danger` 复用主题 `danger`（公式渲染失败等失败态前景）。
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
            mark_bg: src_over(t.mark_bg, t.bg),
            danger: t.danger,
            font_family: None,
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
/// 屏幕版心边距默认值（逻辑像素）：单一来源 `tokens::doc::MARGIN`（Standard 档）。
const DEFAULT_DOC_MARGIN_PX: f32 = crate::tokens::doc::MARGIN;
/// 自定义版心边距允许范围（逻辑像素）。
pub(crate) const DOC_MARGIN_PX_MIN: f32 = 4.0;
pub(crate) const DOC_MARGIN_PX_MAX: f32 = 200.0;

/// 自定义版心边距夹取：4..=200 逻辑像素。
pub(crate) fn clamp_doc_margin_px(v: f32) -> f32 {
    v.clamp(DOC_MARGIN_PX_MIN, DOC_MARGIN_PX_MAX)
}
/// 正文字号允许范围（物理像素基准）。
const DOC_FONT_SIZE_MIN: f32 = 10.0;
const DOC_FONT_SIZE_MAX: f32 = 32.0;
/// 字体族值最大长度：超长视为无法识别（防止恶意/损坏配置）。
const DOC_FONT_FAMILY_MAX_LEN: usize = 64;
/// 显式字族名最大长度（字节）：超长/空串视为无法识别。
pub(crate) const DOC_FONT_FACE_MAX_LEN: usize = 64;
/// 制表符宽度默认值（兼容 2 与 4，Task D 新增）。
const DEFAULT_TAB_WIDTH: u8 = 4;
/// 图片缓存字节上限默认值（MiB）：容纳一屏图文混排的若干张大图，超出后按 LRU 逐出。
/// 已从 Config 固化为常量：UI 不再暴露、config.toml 不再读取/写出（旧键静默忽略）。
pub(crate) const DEFAULT_IMAGE_CACHE_LIMIT_MB: u32 = 200;

/// 正文字号夹取：10..=32。
fn clamp_font_size(v: f32) -> f32 {
    v.clamp(DOC_FONT_SIZE_MIN, DOC_FONT_SIZE_MAX)
}

/// 字符串 → TOML 基本字符串字面量（全量转义，保证与 `toml_edit` 解析严格互逆）。
fn toml_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// 正文行高夹取：字号..=字号×3（保证行高不小于字号、又不至于过分稀疏）。
fn clamp_line_height(v: f32, size: f32) -> f32 {
    v.clamp(size, size * 3.0)
}

/// 配置内容：主题模式 + PDF 导出设置 + 正文排版 + 窗口几何。
///
/// 因 `doc_font_face` 承载任意字族名，本结构不再 `Copy`（改为 `Clone`）。
#[derive(Clone, PartialEq, Debug)]
pub struct Config {
    pub mode: ThemeMode,
    /// 界面语言（默认自动跟随系统）；仅启动时生效，改动需重启。
    pub ui_language: UiLanguage,
    /// PDF 页边距（毫米）。
    pub export_margin_mm: f32,
    /// PDF 页脚页码 `n / total`。
    pub export_page_numbers: bool,
    /// PDF 目录页。
    pub export_toc: bool,
    /// 正文字体族（仅作用于正文/表格文本，UI chrome 不变）。
    pub doc_font_family: DocFontFamily,
    /// 显式字族名（已安装系统字体）；`None` = 沿用 `doc_font_family` 四档预置，
    /// `Some(name)` = 正文优先使用该族（缺字仍走既有 CJK 回退链）。
    pub doc_font_face: Option<String>,
    /// 正文字号（物理像素基准，夹取 10..=32）。
    pub doc_font_size: f32,
    /// 正文行高（物理像素基准，夹取 字号..=字号×3）。
    pub doc_line_height: f32,
    /// 屏幕版心/页边距档位（与 PDF 页边距 `export_margin_mm` 无关）。
    pub doc_margin: DocMargin,
    /// 自定义版心边距（逻辑像素，仅 `doc_margin = custom` 生效，夹取 4..=200）。
    pub doc_margin_px: f32,
    /// 普通远程图片自动加载（默认关）；true = 用户显式允许，false = 全局阻止，只显示 alt 文本。
    pub allow_remote_images: bool,
    /// 源码模式「纯文本行视图」（默认开）：开启后空行成为可寻址行。
    /// 仅影响源码模式的排版/光标几何，不改变保存字节。
    pub source_plain_lines: bool,
    /// 制表符宽度（2 或 4，默认 4）；其它值夹到 4。
    pub tab_width: u8,
    /// 上次退出时的窗口几何；首次运行或配置缺失时为 `None`。
    pub window: Option<WindowGeom>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            mode: ThemeMode::System,
            ui_language: UiLanguage::Auto,
            export_margin_mm: 20.0,
            export_page_numbers: true,
            export_toc: true,
            doc_font_family: DocFontFamily::System,
            doc_font_face: None,
            doc_font_size: DEFAULT_DOC_FONT_SIZE,
            doc_line_height: DEFAULT_DOC_LINE_HEIGHT,
            doc_margin: DocMargin::Standard,
            doc_margin_px: DEFAULT_DOC_MARGIN_PX,
            allow_remote_images: false,
            source_plain_lines: true,
            tab_width: DEFAULT_TAB_WIDTH,
            window: None,
        }
    }
}

impl Config {
    /// 生成配置文件文本（写入即此格式；`parse_config(to_toml())` 恒等往返）。
    pub fn to_toml(&self) -> String {
        let mut text = format!(
            "{}\nmode = \"{}\"\nui_language = \"{}\"\n\
             export_margin_mm = {}\nexport_page_numbers = {}\nexport_toc = {}\n\
             doc_font_family = \"{}\"\ndoc_font_size = {}\ndoc_line_height = {}\n\
             doc_margin = \"{}\"\ndoc_margin_px = {}\n\
             allow_remote_images = {}\nsource_plain_lines = {}\ntab_width = {}\n",
            t(Key::ConfigHeader),
            self.mode.as_str(),
            self.ui_language.as_str(),
            self.export_margin_mm,
            self.export_page_numbers,
            self.export_toc,
            self.doc_font_family.as_str(),
            self.doc_font_size,
            self.doc_line_height,
            self.doc_margin.as_str(),
            self.doc_margin_px,
            self.allow_remote_images,
            self.source_plain_lines,
            self.tab_width,
        );
        if let Some(face) = &self.doc_font_face {
            text.push_str(&format!("doc_font_face = {}\n", toml_string(face)));
        }
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
    pub fn to_toml_preserving(&self, original: Option<&str>) -> String {
        original
            .and_then(|text| edit_existing_toml(text, self))
            .unwrap_or_else(|| self.to_toml())
    }

    /// 源码模式纯文本行视图是否启用：环境变量 `VEIL_SOURCE_PLAIN=1` 仅用于灰度，
    /// 值为 `1` 时强制开启；其他值（含 `0`/空串/未设置）不覆盖配置。
    pub fn source_plain_lines_enabled(&self) -> bool {
        env_force_source_plain(std::env::var("VEIL_SOURCE_PLAIN").ok().as_deref())
            || self.source_plain_lines
    }
}

/// 环境变量灰度开关的纯函数形态（便于单测，不触碰进程环境）。
fn env_force_source_plain(raw: Option<&str>) -> bool {
    raw == Some("1")
}

/// 原地编辑已存在的严格 TOML 配置；返回 `None` 表示必须回退整文件重生成。
///
/// 仅当整文档可被 TOML 解析且不含任何表段时启用：关键字值逐项覆盖，未知键与注释原样保留。
fn edit_existing_toml(text: &str, cfg: &Config) -> Option<String> {
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
        "ui_language",
        toml_edit::value(cfg.ui_language.as_str()),
    );
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
    // 显式字族：None 时主动移除旧键，避免「改回四档预置」后重启仍读回旧字族。
    match &cfg.doc_font_face {
        Some(face) => set_item(&mut doc, "doc_font_face", toml_edit::value(face.as_str())),
        None => {
            doc.remove("doc_font_face");
        }
    }
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
        "doc_margin",
        toml_edit::value(cfg.doc_margin.as_str()),
    );
    set_item(
        &mut doc,
        "doc_margin_px",
        toml_edit::value(f64::from(cfg.doc_margin_px)),
    );
    set_item(
        &mut doc,
        "allow_remote_images",
        toml_edit::value(cfg.allow_remote_images),
    );
    set_item(
        &mut doc,
        "source_plain_lines",
        toml_edit::value(cfg.source_plain_lines),
    );
    set_item(
        &mut doc,
        "tab_width",
        toml_edit::value(i64::from(cfg.tab_width)),
    );
    // 已固化的旧键：保存时主动剥离，旧配置在下次保存后自动清理。
    doc.remove("image_cache_limit_mb");
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
/// `doc_font_family`/`doc_font_face`/`doc_font_size`/`doc_line_height`/
/// `doc_margin`/`doc_margin_px`/`allow_remote_images`/`source_plain_lines`
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
            // 界面语言：旧配置无此键 → Auto（跟随系统，行为与改造前一致）。
            "ui_language" => match value.as_str().and_then(UiLanguage::parse) {
                Some(l) => out.config.ui_language = l,
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
            "doc_font_face" => match value.as_str() {
                Some(s) if !s.is_empty() && s.len() <= DOC_FONT_FACE_MAX_LEN => {
                    out.config.doc_font_face = Some(s.to_string());
                }
                _ => out.ignored += 1,
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
            // 屏幕版心档位：旧配置无此键时保持 Standard（16px，行为不变）。
            "doc_margin" => match value.as_str().and_then(DocMargin::parse) {
                Some(m) => out.config.doc_margin = m,
                None => out.ignored += 1,
            },
            "doc_margin_px" => {
                let v = value
                    .as_float()
                    .or_else(|| value.as_integer().map(|i| i as f64));
                match v {
                    Some(v) if v.is_finite() => {
                        out.config.doc_margin_px = clamp_doc_margin_px(v as f32)
                    }
                    _ => out.ignored += 1,
                }
            }
            // 已移除的旧键（平滑滚动开关）：滚动固定为瞬时，静默忽略且不提示未知键。
            "smooth_scroll" => {}
            "allow_remote_images" => match value.as_bool() {
                Some(v) => out.config.allow_remote_images = v,
                None => out.ignored += 1,
            },
            "source_plain_lines" => match value.as_bool() {
                Some(v) => out.config.source_plain_lines = v,
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
            // 已移除的旧键（图片缓存上限）：已固化为常量 DEFAULT_IMAGE_CACHE_LIMIT_MB，
            // 任何值静默忽略，不改变行为、不产生「无法识别」提示。
            "image_cache_limit_mb" => {}
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
/// 启动前读取配置里的界面语言（供产品层在 `i18n::init_locale` 之前决策）。
///
/// 只关心 `ui_language` 这一个键，故对非 UTF-8/缺失/非法值一律返回 `None`
/// （等同 `auto`：交给 `VEIL_UI_LANG` 或系统 UI 语言），不因配置损坏而拒绝启动。
pub fn ui_language_from_config_bytes(bytes: &[u8]) -> Option<&'static str> {
    let text = std::str::from_utf8(bytes).ok()?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    parse_config(text).config.ui_language.code()
}

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

    /// 系统是否启用高对比度模式（Windows 高对比度预设）。
    /// 显式用户主题选择（light/dark）不被覆盖，仅「跟随系统」时生效。
    /// 默认 false：无此能力的平台/实现无需改动。
    fn is_high_contrast(&mut self) -> bool {
        false
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
    fn doc_font_face_round_trips_and_old_config_keeps_presets() {
        // 自定义字族（含需转义字符）必须严格往返；四档预置语义不受影响。
        let cfg = Config {
            doc_font_family: DocFontFamily::Serif,
            doc_font_face: Some("Noto Serif CJK SC".to_string()),
            ..Config::default()
        };
        let p = parse(&cfg.to_toml());
        assert_eq!(p.config, cfg);
        assert_eq!(p.ignored, 0);
        assert!(cfg
            .to_toml()
            .contains("doc_font_face = \"Noto Serif CJK SC\""));

        let escaped = Config {
            doc_font_face: Some("Quote\"Back\\slash".to_string()),
            ..Config::default()
        };
        assert_eq!(parse(&escaped.to_toml()).config, escaped);

        // 旧配置没有该键 → None，其余字段照旧（视觉行为完全不变）。
        let legacy = parse("mode = \"dark\"\ndoc_font_family = \"serif\"\n");
        assert_eq!(legacy.config.doc_font_face, None);
        assert_eq!(legacy.config.doc_font_family, DocFontFamily::Serif);
        assert_eq!(legacy.config.mode, ThemeMode::Dark);
        assert_eq!(legacy.ignored, 0);
    }

    /// 版心档位：TOML 往返严格互逆；旧配置无键 → Standard + 16（行为不变）。
    #[test]
    fn doc_margin_round_trips_and_legacy_defaults_to_standard() {
        for mode in [
            DocMargin::Compact,
            DocMargin::Standard,
            DocMargin::Relaxed,
            DocMargin::Full,
            DocMargin::Custom,
        ] {
            let cfg = Config {
                doc_margin: mode,
                doc_margin_px: 28.5,
                ..Config::default()
            };
            let text = cfg.to_toml();
            assert!(
                text.contains(&format!("doc_margin = \"{}\"", mode.as_str())),
                "{text}"
            );
            assert!(text.contains("doc_margin_px = 28.5"), "{text}");
            let p = parse(&text);
            assert_eq!(p.config, cfg, "往返必须恒等：{mode:?}");
            assert_eq!(p.ignored, 0);
        }
        let legacy = parse("mode = \"dark\"\n");
        assert_eq!(legacy.config.doc_margin, DocMargin::Standard);
        assert_eq!(legacy.config.doc_margin_px, 16.0);
        assert_eq!(legacy.config.doc_margin.margin_px(16.0), 16.0);
        assert_eq!(legacy.ignored, 0);
    }

    /// 档位字符串严格小写互逆；自定义像素值夹取 4..=200，非法字面量计数忽略。
    #[test]
    fn doc_margin_values_are_strict_and_clamped() {
        for mode in [
            DocMargin::Compact,
            DocMargin::Standard,
            DocMargin::Relaxed,
            DocMargin::Full,
            DocMargin::Custom,
        ] {
            assert_eq!(DocMargin::parse(mode.as_str()), Some(mode));
        }
        assert_eq!(DocMargin::parse("Wide"), None, "严格小写");
        assert_eq!(parse("doc_margin = \"wide\"").ignored, 1);
        assert_eq!(
            parse("doc_margin = \"full\"").config.doc_margin,
            DocMargin::Full
        );

        assert_eq!(parse("doc_margin_px = 1").config.doc_margin_px, 4.0);
        assert_eq!(parse("doc_margin_px = 999").config.doc_margin_px, 200.0);
        assert_eq!(parse("doc_margin_px = 32.5").config.doc_margin_px, 32.5);
        assert_eq!(parse("doc_margin_px = abc").ignored, 1);
        assert_eq!(parse("doc_margin_px = inf").ignored, 1);

        // 档位解析出的实际边距：Full 忽略自定义值，Custom 用夹取后的值。
        assert_eq!(DocMargin::Full.margin_px(120.0), 16.0);
        assert_eq!(DocMargin::Custom.margin_px(0.0), 4.0);
        assert_eq!(DocMargin::Custom.margin_px(999.0), 200.0);
    }

    /// 界面语言：三值 TOML 严格互逆；旧配置无键 → auto（行为与改造前一致）。
    #[test]
    fn ui_language_round_trips_and_legacy_defaults_to_auto() {
        for lang in [UiLanguage::Auto, UiLanguage::ZhCn, UiLanguage::EnUs] {
            assert_eq!(UiLanguage::parse(lang.as_str()), Some(lang));
            let cfg = Config {
                ui_language: lang,
                ..Config::default()
            };
            let text = cfg.to_toml();
            assert!(
                text.contains(&format!("ui_language = \"{}\"", lang.as_str())),
                "{text}"
            );
            let p = parse(&text);
            assert_eq!(p.config, cfg, "往返必须恒等：{lang:?}");
            assert_eq!(p.ignored, 0);
        }
        assert_eq!(UiLanguage::parse("Auto"), None, "严格大小写");

        let legacy = parse("mode = \"dark\"\n");
        assert_eq!(legacy.config.ui_language, UiLanguage::Auto);
        assert_eq!(legacy.ignored, 0);
        // 非法值：按 auto 处理并计入 ignored，不影响启动。
        let bad = parse("ui_language = \"fr-FR\"\n");
        assert_eq!(bad.config.ui_language, UiLanguage::Auto);
        assert_eq!(bad.ignored, 1);

        // 启动前读取：BOM/非法 UTF-8/auto 都返回 None（交给环境变量/系统语言）。
        assert_eq!(
            ui_language_from_config_bytes("\u{feff}ui_language = \"en-US\"\n".as_bytes()),
            Some("en-US")
        );
        assert_eq!(
            ui_language_from_config_bytes(b"ui_language = \"zh-CN\"\n"),
            Some("zh-CN")
        );
        assert_eq!(
            ui_language_from_config_bytes(b"ui_language = \"auto\"\n"),
            None
        );
        assert_eq!(ui_language_from_config_bytes(b"mode = \"dark\"\n"), None);
        assert_eq!(ui_language_from_config_bytes(&[0xff, 0xfe]), None);
    }

    /// PDF 页边距（`export_margin_mm`）与屏幕版心互不影响：解析、序列化、取值都独立。
    #[test]
    fn pdf_margin_is_independent_of_doc_margin() {
        let cfg = Config {
            doc_margin: DocMargin::Full,
            doc_margin_px: 120.0,
            export_margin_mm: 12.5,
            ..Config::default()
        };
        let p = parse(&cfg.to_toml());
        assert_eq!(p.config.export_margin_mm, 12.5, "PDF 边距必须原样保留");
        assert_eq!(p.config.doc_margin, DocMargin::Full);
        assert_eq!(p.config.doc_margin_px, 120.0);

        // 只给 PDF 边距：屏幕版心保持默认 Standard + 16。
        let p = parse("export_margin_mm = 30\n");
        assert_eq!(p.config.export_margin_mm, 30.0);
        assert_eq!(p.config.doc_margin, DocMargin::Standard);
        assert_eq!(p.config.doc_margin_px, 16.0);

        // 只给屏幕版心：PDF 边距保持默认 20mm，运行时边距只读档位。
        let p = parse("doc_margin = \"relaxed\"\ndoc_margin_px = 48\n");
        assert_eq!(p.config.export_margin_mm, 20.0);
        assert_eq!(p.config.doc_margin.margin_px(p.config.doc_margin_px), 32.0);
    }

    #[test]
    fn doc_font_face_rejects_overlong_and_empty_values() {
        let long = "字".repeat(22); // 66 字节 > 64
        let p = parse(&format!("doc_font_face = \"{long}\"\n"));
        assert_eq!(p.config.doc_font_face, None);
        assert_eq!(p.ignored, 1, "超长字族视为无法识别");

        let p = parse("doc_font_face = \"\"\n");
        assert_eq!(p.config.doc_font_face, None);
        assert_eq!(p.ignored, 1, "空字族视为无法识别");
    }

    #[test]
    fn preserving_save_writes_and_removes_doc_font_face() {
        let original = "# 顶部注释\nmode = \"light\"\n";
        let with_face = Config {
            doc_font_face: Some("SimSun".to_string()),
            ..Config::default()
        };
        let saved = with_face.to_toml_preserving(Some(original));
        assert!(saved.contains("doc_font_face = \"SimSun\""), "{saved}");
        assert_eq!(parse(&saved).config, with_face);

        // 改回四档预置：旧键必须移除，重启后不得读回旧字族。
        let back_to_preset = Config {
            doc_font_family: DocFontFamily::Sans,
            ..Config::default()
        };
        let saved_again = back_to_preset.to_toml_preserving(Some(&saved));
        assert!(!saved_again.contains("doc_font_face"), "{saved_again}");
        assert_eq!(parse(&saved_again).config, back_to_preset);
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

    /// 配置兼容（Task 6.1）：未知键不崩溃且不影响已知键解析；保存时保留未知键与注释
    /// 并更新已知键；保存后再次解析，已知键值不变（往返）。
    #[test]
    fn unknown_keys_ignored_then_preserved_on_save_roundtrip() {
        let original = "# 顶部注释\nmode = \"light\" # 行尾注释\nfuture_flag = true\nfuture_table = { a = 1 }\n";
        // 读取：未知键计 ignored，已知键照常解析。
        let parsed = parse(original);
        assert_eq!(parsed.config.mode, ThemeMode::Light);
        assert_eq!(parsed.ignored, 2, "未知键只计数，不影响已知键");

        // 保存：注释、未知键、键顺序保留，已知键被更新。
        let updated = Config {
            mode: ThemeMode::Dark,
            tab_width: 2,
            ..Config::default()
        };
        let saved = updated.to_toml_preserving(Some(original));
        assert!(saved.contains("# 顶部注释"), "{saved}");
        assert!(saved.contains("# 行尾注释"), "{saved}");
        assert!(saved.contains("future_flag = true"), "{saved}");
        assert!(saved.contains("future_table = { a = 1 }"), "{saved}");
        assert!(saved.contains("mode = \"dark\""), "{saved}");

        // 往返：再次解析得到更新后的已知键；再次保存不改变已知键值。
        let reloaded = parse(&saved);
        assert_eq!(reloaded.config.mode, ThemeMode::Dark);
        assert_eq!(reloaded.config.tab_width, 2);
        let saved_again = reloaded.config.to_toml_preserving(Some(&saved));
        assert_eq!(parse(&saved_again).config, reloaded.config);
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

    /// 图片缓存上限已固化为常量 DEFAULT_IMAGE_CACHE_LIMIT_MB：旧配置中的
    /// `image_cache_limit_mb` 键（任何值，含非法字面量）一律静默忽略，
    /// 不改变行为、不产生「无法识别」提示。
    #[test]
    fn image_cache_limit_defaults_and_parses_integer() {
        // 固化常量仍为 200 MiB（运行时图片缓存限额的单一来源）
        assert_eq!(DEFAULT_IMAGE_CACHE_LIMIT_MB, 200);
        // 空配置不产生 ignored
        let p = parse("");
        assert_eq!(p.ignored, 0);
        // 旧键任何值都不得改变配置、不得计数
        for text in [
            "image_cache_limit_mb = 512\n",
            "image_cache_limit_mb = 0\n",
            "image_cache_limit_mb = -1",
            "image_cache_limit_mb = 4097",
            "image_cache_limit_mb = \"200\"",
            "image_cache_limit_mb = 1.5",
        ] {
            let p = parse(text);
            assert_eq!(p.ignored, 0, "旧键不得产生无法识别提示：{text}");
            assert_eq!(p.config, Config::default(), "旧键不得改变任何行为：{text}");
        }
    }

    /// 新序列化不再写出 `image_cache_limit_mb`；含旧键的配置在保留式保存时被剥离，
    /// 往返解析恒等（旧键不回流为配置项）。
    #[test]
    fn image_cache_limit_round_trips_through_toml() {
        // 全新序列化不得包含该键
        assert!(
            !Config::default().to_toml().contains("image_cache_limit_mb"),
            "序列化不应再写出 image_cache_limit_mb"
        );
        // 含旧键的配置：解析不受影响（ignored 为 0、配置为默认）
        let legacy = "mode = \"dark\"\nimage_cache_limit_mb = 512\n";
        let p = parse(legacy);
        assert_eq!(p.ignored, 0);
        let expected = Config {
            mode: ThemeMode::Dark,
            ..Config::default()
        };
        assert_eq!(p.config, expected);
        // 保留式保存后旧键被剥离，重新解析仍恒等
        let saved = expected.to_toml_preserving(Some(legacy));
        assert!(!saved.contains("image_cache_limit_mb"), "保存应剥离旧键");
        let p2 = parse(&saved);
        assert_eq!(p2.config, expected);
        assert_eq!(p2.ignored, 0);
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

    #[test]
    fn source_plain_lines_defaults_true_and_parses_bool() {
        assert!(Config::default().source_plain_lines);
        assert!(parse("mode = \"dark\"\n").config.source_plain_lines);
        assert!(
            parse("source_plain_lines = \"true\"")
                .config
                .source_plain_lines,
            "非法类型保留默认（开）并计入忽略"
        );
        assert!(
            !parse("source_plain_lines = false")
                .config
                .source_plain_lines
        );
        assert!(parse("source_plain_lines = true").config.source_plain_lines);
        let p = parse("source_plain_lines = \"yes\"");
        assert!(p.config.source_plain_lines);
        assert_eq!(p.ignored, 1, "非法字面量计入忽略");
    }

    #[test]
    fn source_plain_lines_round_trips_through_toml() {
        for enabled in [true, false] {
            let cfg = Config {
                source_plain_lines: enabled,
                ..Config::default()
            };
            let text = cfg.to_toml();
            assert!(
                text.contains(&format!("source_plain_lines = {enabled}")),
                "序列化必须写 {enabled}: {text}"
            );
            let p = parse(&text);
            assert_eq!(p.config, cfg, "往返恒等");
            assert_eq!(p.ignored, 0);
        }
    }

    #[test]
    fn source_plain_env_override_only_accepts_one() {
        assert!(env_force_source_plain(Some("1")));
        for other in [Some("0"), Some(""), Some("true"), Some("yes"), None] {
            assert!(!env_force_source_plain(other), "只认 1: {other:?}");
        }
        // 配置为 true 时即便环境变量非 1 也保持开启。
        let cfg = Config {
            source_plain_lines: true,
            ..Config::default()
        };
        assert!(cfg.source_plain_lines_enabled());
        let off = Config {
            source_plain_lines: false,
            ..Config::default()
        };
        // 测试环境默认不设置 VEIL_SOURCE_PLAIN；若被外部设置也不应与 false 配置冲突。
        if std::env::var("VEIL_SOURCE_PLAIN").ok().as_deref() != Some("1") {
            assert!(!off.source_plain_lines_enabled());
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
        assert_eq!(t.modal_bg, [249, 250, 252, 255]);
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
        // 兜底色板必须与浅色 tmTheme 解析结果逐 token 一致：plist 解析失败时会直接用
        // SYNTAX_FALLBACK，若两者漂移就会出现两套浅色语法色。
        assert_eq!(
            SyntaxColors::default_light(),
            SYNTAX_FALLBACK,
            "SYNTAX_FALLBACK 必须与 nord_light.tmTheme 的浅色取值一致"
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
        // 模态底色（text 在 modal_bg 上）≥ 4.5（模态弹起最深，前景必须可读）；
        // 悬停态（text 在 hover 上，短暂交互态，按大字级放宽）≥ 3.0。
        // 若调整 muted / hover / modal_bg 取值，必须保持本断言成立。
        for t in [Theme::light(), Theme::dark()] {
            let popup = [t.popup_bg[0], t.popup_bg[1], t.popup_bg[2]];
            let modal = [t.modal_bg[0], t.modal_bg[1], t.modal_bg[2]];
            let hover = [t.hover[0], t.hover[1], t.hover[2]];
            let c_text_popup = contrast(t.text, popup);
            let c_muted_popup = contrast(t.muted, popup);
            let c_text_modal = contrast(t.text, modal);
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
                c_text_modal >= 4.5,
                "text/modal_bg 对比度不足: dark={} {c_text_modal:.2}",
                t.dark
            );
            assert!(
                c_text_hover >= 3.0,
                "text/hover 对比度不足: dark={} {c_text_hover:.2}",
                t.dark
            );
            // modal ≠ popup：弹起层级必须分离
            assert_ne!(
                t.modal_bg, t.popup_bg,
                "modal_bg 必须与 popup_bg 不同才能建立模态弹起层级（dark={}）",
                t.dark
            );
        }
    }

    #[test]
    fn surface_levels_map_to_correct_fields() {
        // S5 语义层门禁：SurfaceLevel 四档必须精确映射到 Theme 字段。
        for t in [Theme::light(), Theme::dark()] {
            assert_eq!(t.surface(SurfaceLevel::Base), t.bg);
            assert_eq!(t.surface(SurfaceLevel::Card), t.chrome_bg);
            assert_eq!(t.surface(SurfaceLevel::Popup), t.popup_bg);
            assert_eq!(t.surface(SurfaceLevel::Modal), t.modal_bg);
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
        // mark 底与 danger 从主题派生：mark 先与页面底合成（导出 CSS 不留 alpha）
        assert_eq!(light.mark_bg, [255, 249, 183]);
        assert_eq!(light.danger, [178, 34, 34]);
        let dark = ExportStyle::from_theme(&Theme::dark());
        assert!(dark.dark);
        assert_eq!(dark.bg, [30, 30, 30]);
        assert_eq!(dark.text, [220, 220, 220]); // 取 fg（正文前景），非菜单文字色
        assert_eq!(dark.code_bg, [40, 40, 40]);
        assert_eq!(dark.border, [60, 64, 70]);
        assert_eq!(dark.mark_bg, [106, 84, 5]);
        assert_eq!(dark.danger, [255, 130, 130]);
        // Default 与主题无关：浅色兜底值固定，供未注入主题的调用方使用
        assert_eq!(ExportStyle::default().bg, [255, 255, 255]);
        assert_eq!(ExportStyle::default().mark_bg, [255, 249, 183]);
        assert_eq!(ExportStyle::default().danger, [178, 34, 34]);
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

    // ------------------------------------------------------------------
    // 配置读写 round-trip（文件级）：写 config.toml → 重新读取 → 语义比较。
    // 用真实落盘字节而非内存字符串，锁住 UTF-8/BOM/换行与 toml_edit 输出格式。
    // ------------------------------------------------------------------

    /// 唯一临时配置文件：`std::env::temp_dir()` + 进程 id/时间戳/自增序号。
    /// Drop 时删除，断言失败展开也会清理。
    struct TempConfig(std::path::PathBuf);

    impl TempConfig {
        fn new(tag: &str) -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static SEQ: AtomicU64 = AtomicU64::new(0);
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let mut path = std::env::temp_dir();
            path.push(format!(
                "veil-config-roundtrip-{}-{nanos}-{}-{tag}.toml",
                std::process::id(),
                SEQ.fetch_add(1, Ordering::Relaxed)
            ));
            TempConfig(path)
        }

        fn write(&self, text: &str) {
            std::fs::write(&self.0, text.as_bytes()).expect("写临时配置");
        }

        /// 产品读取路径等价物：文件字节 → `decode_config`（剥 BOM、要求合法 UTF-8）。
        fn reload(&self) -> ConfigParse {
            let bytes = std::fs::read(&self.0).expect("读临时配置");
            decode_config(&bytes).expect("自产/临时配置必须是合法 UTF-8")
        }

        fn text(&self) -> String {
            std::fs::read_to_string(&self.0).expect("读临时配置文本")
        }
    }

    impl Drop for TempConfig {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    /// 全字段（15 个配置字段 + 4 个窗口键）写盘再读回必须恒等。
    /// 这里刻意逐个列出字段而非 `..Default::default()`：新增字段时本测试编译失败，
    /// 强制把新字段纳入 round-trip 门禁。
    #[test]
    fn config_roundtrip_all_fields_through_file() {
        let cfg = Config {
            mode: ThemeMode::Dark,
            ui_language: UiLanguage::ZhCn,
            export_margin_mm: 12.5,
            export_page_numbers: false,
            export_toc: false,
            doc_font_family: DocFontFamily::Serif,
            doc_font_face: Some("Noto Serif CJK SC".to_string()),
            doc_font_size: 18.0,
            doc_line_height: 27.0,
            doc_margin: DocMargin::Custom,
            doc_margin_px: 28.5,
            allow_remote_images: true,
            source_plain_lines: true,
            tab_width: 2,
            window: Some(WindowGeom {
                width: 1280,
                height: 800,
                x: -40,
                y: 60,
            }),
        };
        let file = TempConfig::new("all-fields");
        file.write(&cfg.to_toml());

        let p = file.reload();
        assert_eq!(p.config, cfg, "全字段写盘再读回必须恒等");
        assert_eq!(p.ignored, 0, "自产配置不得含无法识别条目");
        // 新版格式：不写历史段标题，窗口几何成组出现。
        let text = file.text();
        assert!(!text.contains("[theme]"), "{text}");
        for key in ["window_width", "window_height", "window_x", "window_y"] {
            assert!(text.contains(key), "有几何时必须写 {key}: {text}");
        }
    }

    /// 手写配置（注释 + 未知键）→ 修改 → 保留式写回 → 再读取：
    /// 已知字段按修改生效，未知键/注释保留，语义不漂移；再次保存字节稳定。
    #[test]
    fn config_roundtrip_edit_then_reload_preserves_semantics() {
        let original = "# 手写注释\nmode = \"light\" # 行尾注释\n\
                        future_flag = true\nfuture_table = { a = 1 }\n\
                        image_cache_limit_mb = 512\nsmooth_scroll = false\n";
        let file = TempConfig::new("hand-written");
        file.write(original);

        let first = file.reload();
        assert_eq!(first.config.mode, ThemeMode::Light);
        assert_eq!(first.ignored, 2, "只有两个未知键计数；已移除旧键静默忽略");

        // 用户在设置里改动若干字段。
        let updated = Config {
            mode: ThemeMode::Dark,
            export_toc: false,
            doc_font_face: Some("SimSun".to_string()),
            tab_width: 2,
            allow_remote_images: true,
            window: Some(WindowGeom {
                width: 1024,
                height: 720,
                x: 10,
                y: -20,
            }),
            ..first.config.clone()
        };
        let saved = updated.to_toml_preserving(Some(original));
        file.write(&saved);

        let reloaded = file.reload();
        assert_eq!(reloaded.config, updated, "保存后重新加载必须等价");
        assert_eq!(reloaded.ignored, 2, "未知键仍按既有口径计数");
        let text = file.text();
        for keep in [
            "# 手写注释",
            "# 行尾注释",
            "future_flag = true",
            "future_table = { a = 1 }",
            "mode = \"dark\"",
            "doc_font_face = \"SimSun\"",
        ] {
            assert!(text.contains(keep), "写回不得丢 {keep}: {text}");
        }
        assert!(
            !text.contains("image_cache_limit_mb"),
            "已固化旧键应在保存时剥离: {text}"
        );
        // 既有容错语义（记录而非修正）：`smooth_scroll` 等已移除旧键会留在文件里，
        // 读取端静默忽略、不计数、不改变行为；保存端只有 `image_cache_limit_mb` 被剥离。
        assert!(
            text.contains("smooth_scroll = false"),
            "已移除旧键按既有语义原样保留: {text}"
        );
        assert_eq!(
            parse_config(&text).config,
            updated,
            "保留的旧键不得回流为行为"
        );
        assert_eq!(parse_config(&text).ignored, 2, "保留的旧键不得计入无法识别");

        // 幂等：对已保存文件再存一次，字节不变（避免每次保存产生 diff）。
        let saved_again = reloaded.config.to_toml_preserving(Some(&text));
        assert_eq!(saved_again, text, "重复保存必须字节稳定");
    }

    /// 数值边界（含非二进制精确小数）经写盘/读回后必须逐位相等；
    /// 锁定 `to_toml` 的 f32 文本格式与 `parse_config` 的 f64→f32 读取不产生漂移。
    #[test]
    fn config_roundtrip_numeric_edges_through_file() {
        let file = TempConfig::new("numeric-edges");
        for margin_mm in [0.0_f32, 12.5, 13.7, 20.0, 50.0] {
            for margin_px in [4.0_f32, 28.5, 200.0] {
                for (font_size, line_height) in [
                    (10.0_f32, 10.0_f32),
                    (16.0, 22.0),
                    (18.0, 27.0),
                    (32.0, 96.0),
                ] {
                    let cfg = Config {
                        export_margin_mm: margin_mm,
                        doc_margin: DocMargin::Custom,
                        doc_margin_px: margin_px,
                        doc_font_size: font_size,
                        doc_line_height: line_height,
                        ..Config::default()
                    };
                    file.write(&cfg.to_toml());
                    let p = file.reload();
                    assert_eq!(
                        p.config, cfg,
                        "数值往返漂移: margin_mm={margin_mm} margin_px={margin_px} \
                         size={font_size} line_height={line_height}"
                    );
                    assert_eq!(p.ignored, 0);
                }
            }
        }
    }

    /// 真实世界容错：BOM + CRLF + 历史 `[theme]` 段。读取语义正确；
    /// 保存走整文件重生成（有意为之），重新加载语义仍等价。
    #[test]
    fn config_roundtrip_legacy_section_bom_crlf_keeps_semantics() {
        let original = "\u{feff}[theme]\r\nmode = \"dark\"\r\n# 旧注释\r\n";
        let file = TempConfig::new("legacy-bom");
        file.write(original);

        let p = file.reload();
        assert_eq!(p.config.mode, ThemeMode::Dark);
        assert_eq!(p.ignored, 0, "段标题行不承载配置项，不计数");

        let cfg = Config {
            mode: ThemeMode::Dark,
            ..Config::default()
        };
        // 含表段 → 回退整文件重生成（历史注释被丢弃是既有语义，记录于证据文档）。
        let saved = cfg.to_toml_preserving(Some(original));
        assert!(
            !saved.contains("[theme]"),
            "重生成后不再保留段标题: {saved}"
        );
        file.write(&saved);
        assert_eq!(file.reload().config, cfg);
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
