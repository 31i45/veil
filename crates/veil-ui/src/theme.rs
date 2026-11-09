//! 主题与配置（切片 17）：调色板、极简 TOML 子集解析、配置路径，以及平台接缝 trait。
//!
//! 零 GUI 依赖：解析/序列化/路径推导/主题判定全部为纯函数，headless 可测。
//! 平台能力（注册表系统外观、DWM 深色标题栏/Mica、配置落盘）以 trait 声明，由产品层注入。
//!
//! 配置格式（极简，唯一支持的项写在顶层）：
//! ```toml
//! mode = "system"   # system | light | dark
//! ```
//! 历史格式的 `[theme]` 段标题行仍被接受但直接跳过，便于旧配置无损升级。

use std::ffi::OsString;
use std::path::PathBuf;

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
        match self {
            ThemeMode::System => "跟随系统",
            ThemeMode::Light => "浅色",
            ThemeMode::Dark => "深色",
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

impl SyntaxColors {
    /// 浅色默认色板（LayoutEngine 无主题时的兜底）。
    pub const fn default_light() -> Self {
        SyntaxColors {
            keyword: [132, 48, 137],
            string: [3, 106, 78],
            comment: [105, 105, 105],
            number: [12, 94, 188],
            function: [115, 82, 3],
            type_name: [26, 82, 168],
            punctuation: [90, 90, 90],
        }
    }
}

/// 调色板：正文/ chrome / 交互态全套颜色。浅色值与原硬编码常量逐字节一致（视觉不回退）。
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

impl Theme {
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

    pub const fn light() -> Self {
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
            syntax: SyntaxColors {
                keyword: [132, 48, 137],   // 紫
                string: [3, 106, 78],      // 深绿
                comment: [105, 105, 105],  // 灰
                number: [12, 94, 188],     // 蓝
                function: [115, 82, 3],    // 棕
                type_name: [26, 82, 168],  // 蓝
                punctuation: [90, 90, 90], // 深灰
            },
        }
    }

    pub const fn dark() -> Self {
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
            syntax: SyntaxColors {
                keyword: [197, 134, 192],     // 亮紫
                string: [125, 191, 153],      // 青绿
                comment: [150, 150, 150],     // 灰
                number: [104, 166, 232],      // 亮蓝
                function: [220, 180, 100],    // 金
                type_name: [120, 170, 230],   // 蓝
                punctuation: [160, 160, 160], // 浅灰
            },
        }
    }
}

/// surface 设计令牌（不可配置）：浮层（右键菜单 / 快速打开 / 大纲 / 搜索条）共用几何。
/// 所有长度以 1.0× scale 为基准，经取值函数按 scale 线性放大；深/浅主题共用几何，
/// 仅阴影 alpha 按主题微调（阴影是半透明黑叠加，深底下需稍强才有同等存在感）。
pub(crate) mod surface {
    /// 圆角半径（px，1.0× 基准）
    pub const CORNER_RADIUS: f32 = 6.0;
    /// 条目行高（px）
    pub const ROW_HEIGHT: f32 = 28.0;
    /// 内边距（px）
    pub const PADDING: f32 = 12.0;
    /// 分隔线高（px）
    pub const DIVIDER_HEIGHT: f32 = 4.0;
    /// 悬停内缩（px）：悬停底色相对条目外边界内缩，形成「浮起」边距
    pub const HOVER_INSET: f32 = 3.0;
    /// 面板外边距（px）：浮起卡片与窗口边缘/滚动条之间的间距
    pub const PANEL_MARGIN: f32 = 8.0;
    /// 控件间距（px）：同一浮层内相邻控件或与内容视口之间的间隙
    pub const CONTROL_GAP: f32 = 8.0;
    /// 文本光标宽（px）
    pub const CARET_WIDTH: f32 = 2.0;
    /// 文本光标高（px）
    pub const CARET_HEIGHT: f32 = 16.0;
    /// 滚动条拇指最小高度（px）
    pub const SCROLLBAR_MIN_THUMB: f32 = 24.0;
    /// 阴影整体向下偏移（px）
    pub const SHADOW_OFFSET_Y: f32 = 2.0;
    /// 阴影层（外扩 px, 基础 alpha），自外向内排列：外层大而淡、内层小而浓，
    /// 叠加大致等效 8px 高斯模糊 + 2px 下移。alpha 总和 72（深色 ×1.25 ≈ 90），
    /// 远低于「显脏」阈值（浅底上 alpha 和 >140 开始凝成灰团）。
    pub const SHADOW_LAYERS: &[(f32, u8)] = &[(8.0, 4), (6.0, 8), (4.0, 14), (2.0, 20), (1.0, 26)];
    /// 深色主题阴影 alpha 增益
    pub const SHADOW_DARK_GAIN: f32 = 1.25;
    /// 模态卡片圆角（px）：比普通浮层大一级，强调「应用内对话框」的层级。
    pub const DIALOG_RADIUS: f32 = 8.0;
    /// 模态卡片内边距（px）
    pub const DIALOG_PADDING: f32 = 20.0;
    /// 模态卡片最小宽度（px）
    pub const DIALOG_MIN_WIDTH: f32 = 320.0;
    /// 模态卡片最大宽度（px）
    pub const DIALOG_MAX_WIDTH: f32 = 520.0;
    /// 模态按钮高（px）：与 ROW_HEIGHT 同值但语义独立，避免行高一改按钮跟着变。
    pub const DIALOG_BUTTON_HEIGHT: f32 = 28.0;
    /// 模态按钮最小宽（px）
    pub const DIALOG_BUTTON_MIN_WIDTH: f32 = 88.0;
    /// 模态按钮水平间距（px）
    pub const DIALOG_BUTTON_GAP: f32 = 8.0;
    /// 标题与正文间距（px）
    pub const DIALOG_TITLE_GAP: f32 = 8.0;
    /// 正文与按钮行间距（px）
    pub const DIALOG_BODY_GAP: f32 = 20.0;
    /// 模态卡片距视口边缘的最小留白（px）
    pub const DIALOG_MARGIN: f32 = 24.0;
    /// 浅色主题遮罩 alpha（黑）：足以压暗正文又把弹窗推前。
    pub const SCRIM_ALPHA_LIGHT: u8 = 80;
    /// 深色主题遮罩 alpha（黑）：深底需更重才有分离感。
    pub const SCRIM_ALPHA_DARK: u8 = 120;

    pub fn corner_radius(scale: f32) -> f32 {
        CORNER_RADIUS * scale
    }
    pub fn row_height(scale: f32) -> f32 {
        ROW_HEIGHT * scale
    }
    pub fn padding(scale: f32) -> f32 {
        PADDING * scale
    }
    pub fn divider_height(scale: f32) -> f32 {
        DIVIDER_HEIGHT * scale
    }
    pub fn hover_inset(scale: f32) -> f32 {
        HOVER_INSET * scale
    }
    pub fn panel_margin(scale: f32) -> f32 {
        PANEL_MARGIN * scale
    }
    pub fn control_gap(scale: f32) -> f32 {
        CONTROL_GAP * scale
    }
    pub fn caret_width(scale: f32) -> f32 {
        CARET_WIDTH * scale
    }
    pub fn caret_height(scale: f32) -> f32 {
        CARET_HEIGHT * scale
    }
    /// 细滚动条拇指最小高度（大纲等浮层内使用；主滚动条沿用 chrome 的 THUMB_MIN_H）。
    pub fn scrollbar_min_thumb(scale: f32) -> f32 {
        SCROLLBAR_MIN_THUMB * scale
    }
    pub fn shadow_offset_y(scale: f32) -> f32 {
        SHADOW_OFFSET_Y * scale
    }
    pub fn dialog_radius(scale: f32) -> f32 {
        DIALOG_RADIUS * scale
    }
    pub fn dialog_padding(scale: f32) -> f32 {
        DIALOG_PADDING * scale
    }
    pub fn dialog_min_width(scale: f32) -> f32 {
        DIALOG_MIN_WIDTH * scale
    }
    pub fn dialog_max_width(scale: f32) -> f32 {
        DIALOG_MAX_WIDTH * scale
    }
    pub fn dialog_button_height(scale: f32) -> f32 {
        DIALOG_BUTTON_HEIGHT * scale
    }
    pub fn dialog_button_min_width(scale: f32) -> f32 {
        DIALOG_BUTTON_MIN_WIDTH * scale
    }
    pub fn dialog_button_gap(scale: f32) -> f32 {
        DIALOG_BUTTON_GAP * scale
    }
    pub fn dialog_title_gap(scale: f32) -> f32 {
        DIALOG_TITLE_GAP * scale
    }
    pub fn dialog_body_gap(scale: f32) -> f32 {
        DIALOG_BODY_GAP * scale
    }
    pub fn dialog_margin(scale: f32) -> f32 {
        DIALOG_MARGIN * scale
    }

    /// 遮罩 alpha：只随主题明暗切换，不随 scale 变化。
    pub fn scrim_alpha(dark: bool) -> u8 {
        if dark {
            SCRIM_ALPHA_DARK
        } else {
            SCRIM_ALPHA_LIGHT
        }
    }

    /// 阴影单层 alpha：深色主题按增益放大（钳制 255）。
    pub fn shadow_alpha(base: u8, dark: bool) -> u8 {
        if dark {
            (base as f32 * SHADOW_DARK_GAIN).round().min(255.0) as u8
        } else {
            base
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

/// 配置内容：主题模式 + PDF 导出设置。
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Config {
    pub mode: ThemeMode,
    /// PDF 页边距（毫米）。
    pub export_margin_mm: f32,
    /// PDF 页脚页码 `n / total`。
    pub export_page_numbers: bool,
    /// PDF 目录页。
    pub export_toc: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            mode: ThemeMode::System,
            export_margin_mm: 20.0,
            export_page_numbers: true,
            export_toc: true,
        }
    }
}

impl Config {
    /// 生成配置文件文本（写入即此格式；`parse_config(to_toml())` 恒等往返）。
    pub fn to_toml(self) -> String {
        format!(
            "# Veil 配置（自动生成）\n# mode: system（跟随系统）| light | dark\nmode = \"{}\"\n\
             export_margin_mm = {}\nexport_page_numbers = {}\nexport_toc = {}\n",
            self.mode.as_str(),
            self.export_margin_mm,
            self.export_page_numbers,
            self.export_toc
        )
    }
}

/// 解析结果：`ignored` 为无法识别的条目数（调用方提示后忽略）。
#[derive(Clone, PartialEq, Debug, Default)]
pub struct ConfigParse {
    pub config: Config,
    pub ignored: usize,
}

/// 极简配置解析：只认顶层 `mode` 键，其余条目计数忽略。
/// 段标题行（如历史格式 `[theme]`）不承载配置项，直接跳过；
/// 同一键重复出现时后者生效（无歧义地覆盖）。
pub fn parse_config(text: &str) -> ConfigParse {
    let mut out = ConfigParse::default();
    for raw in text.lines() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            out.ignored += 1;
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        match key {
            "mode" => match unquote(value).and_then(ThemeMode::parse) {
                Some(m) => out.config.mode = m,
                None => out.ignored += 1,
            },
            "export_margin_mm" => match value.parse::<f32>() {
                Ok(v) if v.is_finite() && v >= 0.0 => out.config.export_margin_mm = v,
                _ => out.ignored += 1,
            },
            "export_page_numbers" => match parse_bool(value) {
                Some(v) => out.config.export_page_numbers = v,
                None => out.ignored += 1,
            },
            "export_toc" => match parse_bool(value) {
                Some(v) => out.config.export_toc = v,
                None => out.ignored += 1,
            },
            _ => out.ignored += 1,
        }
    }
    out
}

/// TOML 布尔字面量（只接受裸 `true`/`false`）。
fn parse_bool(v: &str) -> Option<bool> {
    match v {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// 配置文件字节 → 解析结果：剥 BOM、要求合法 UTF-8（非法编码整体拒绝，用默认主题）。
pub fn decode_config(bytes: &[u8]) -> Result<ConfigParse, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "配置文件不是合法 UTF-8".to_string())?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    Ok(parse_config(text))
}

/// 行内注释剥离：引号内的 `#` 不是注释（极简子集不支持字符串内转义）。
fn strip_comment(line: &str) -> &str {
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

/// 去引号：仅接受成对单/双引号包裹的字符串（TOML 字符串值语义）。
fn unquote(v: &str) -> Option<&str> {
    let b = v.as_bytes();
    if b.len() >= 2 && (b[0] == b'"' || b[0] == b'\'') && b[b.len() - 1] == b[0] {
        Some(&v[1..v.len() - 1])
    } else {
        None
    }
}

/// 配置文件路径：Windows `%APPDATA%\Veil\config.toml`；其余平台
/// `$XDG_CONFIG_HOME/veil/config.toml` 或 `$HOME/.config/veil/config.toml`。
pub fn config_path() -> Option<PathBuf> {
    config_path_of(
        std::env::var_os("APPDATA").filter(|s| !s.is_empty()),
        std::env::var_os("XDG_CONFIG_HOME").filter(|s| !s.is_empty()),
        std::env::var_os("HOME").filter(|s| !s.is_empty()),
    )
}

/// 应用数据目录（0b 恢复/会话文件的公共父目录）：与配置同根。
/// Windows `%APPDATA%\Veil`；其余 `$XDG_CONFIG_HOME/veil` 或 `$HOME/.config/veil`。
pub fn data_dir() -> Option<PathBuf> {
    data_dir_of(
        std::env::var_os("APPDATA").filter(|s| !s.is_empty()),
        std::env::var_os("XDG_CONFIG_HOME").filter(|s| !s.is_empty()),
        std::env::var_os("HOME").filter(|s| !s.is_empty()),
    )
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
}

/// 窗口系统外观效果：深色标题栏与 Mica 背景材质。失败静默——内容区始终绘制不透明底色，
/// 因此 Win10/无 Mica 环境下自动退化为纯色外观，无需调用方分支。
pub trait WindowEffects {
    fn apply(&mut self, window: NativeWindow, dark: bool);
}

/// 主题相关平台能力集合（产品层一次性注入，避免 `run` 参数继续膨胀）。
pub struct ThemeEnv {
    pub config: Box<dyn ConfigStore>,
    pub system: Box<dyn SystemTheme>,
    pub effects: Box<dyn WindowEffects>,
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
            };
            let text = cfg.to_toml();
            assert!(!text.contains("[theme]"), "新版配置不再写段标题");
            let p = parse(&text);
            assert_eq!(p.config, cfg);
            assert_eq!(p.ignored, 0, "自产配置不得含无法识别条目");
        }
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
        let p = parse(
            "export_margin_mm = abc\nexport_page_numbers = \"true\"\nexport_toc = yes\n",
        );
        assert_eq!(p.ignored, 3);
        assert_eq!(p.config, Config::default());
        // 负/非有限边距同样拒绝
        assert_eq!(parse("export_margin_mm = -1").ignored, 1);
        assert_eq!(parse("export_margin_mm = inf").config.export_margin_mm, 20.0);
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
            assert!(c_fg >= 7.0, "正文/背景对比度不足: dark={} {c_fg:.2}", t.dark);

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
            assert!(c >= 4.5, "accent_text/accent 对比度不足: dark={} {c:.2}", t.dark);
            let popup = [t.popup_bg[0], t.popup_bg[1], t.popup_bg[2]];
            let edge = contrast(t.accent, popup);
            assert!(edge >= 2.0, "accent/popup 边界对比不足: dark={} {edge:.2}", t.dark);
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
    fn surface_tokens_scale_linearly() {
        // 2.0× 时几何令牌数值翻倍
        assert_eq!(surface::corner_radius(2.0), 12.0);
        assert_eq!(surface::row_height(2.0), 56.0);
        assert_eq!(surface::padding(2.0), 24.0);
        assert_eq!(surface::divider_height(2.0), 8.0);
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
}
