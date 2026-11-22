//! 自绘 chrome：菜单栏、弹出菜单、状态栏与滚动条（切片 15）。
//!
//! 零 GUI 依赖：几何、命中、滚动换算、状态文本全部为纯函数，headless 可测；
//! 绘制只依赖 `Frame` 与 `LayoutEngine`（标签经 preedit 排版 + 光栅化）。
//! 策略（何时开菜单、动作如何执行）在 shell；本层只回答「画在哪/点到谁」。
//!
//! 三责拆分：本文件 = 几何布局 + 菜单模型 + 命中矩形（模型层）；
//! paint.rs = 菜单栏/状态栏/滚动条绘制；cache.rs = 标签位图缓存。

mod cache;
mod paint;

use std::path::Path;
use std::time::Instant;

use crate::anim::Fade;
use crate::document::ExternalState;
use crate::i18n::{t, tf, Key};
use crate::theme::ThemeMode;
use crate::tokens::{space, typography};

pub(crate) use cache::LabelCache;

/// chrome 基准高度/宽度（乘 DPI scale 使用）。
pub const MENU_H: f32 = 30.0;
pub const STATUS_H: f32 = 26.0;
pub const SCROLL_W: f32 = 12.0;
/// 滚动条滑块最小高度（基准值）。
pub const THUMB_MIN_H: f32 = 24.0;

/// 内容视口：菜单栏以下、状态栏以上。
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Viewport {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

pub fn viewport(win_w: u32, win_h: u32, scale: f32) -> Viewport {
    let menu_h = MENU_H * scale;
    let status_h = STATUS_H * scale;
    Viewport {
        x: 0.0,
        y: menu_h,
        w: win_w.max(1) as f32,
        h: (win_h as f32 - menu_h - status_h).max(1.0),
    }
}

/// 菜单动作 + 分隔线哨兵（不参与动作路由、不进命中区、不绘制文字）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuAction {
    /// 分隔线：无动作，仅在布局和绘制层识别。
    Separator,
    /// 新建空白草稿（Ctrl+N）。
    New,
    /// 新建独立窗口（Ctrl+Shift+N）。
    NewWindow,
    Open,
    /// 快速打开浮层（Ctrl+P）。
    QuickOpen,
    Save,
    SaveAs,
    Export,
    /// 直接导出 PDF（Ctrl+Shift+E）：原生矢量管线，写用户确认的 `.pdf`。
    ExportPdf,
    /// 打印（3b）：系统打印对话框 + GDI，复用 PDF 同一分页版面。
    Print,
    /// 版本历史（N1/B8a）：左侧快照列表 + 正文区只读预览。
    VersionHistory,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    /// 全选（Ctrl+A）。
    SelectAll,
    /// 切换大纲面板（P3-2）。
    ToggleOutline,
    /// 打开文档内查找栏（P3-3）。
    Find,
    /// 打开查找替换栏（Ctrl+H）。
    Replace,
    /// 插入图片（2f）：系统对话框选图，插入相对路径链接。
    InsertImage,
    /// 行内格式（3c-1）：加粗/斜体切换、插入链接。
    ToggleBold,
    ToggleItalic,
    InsertLink,
    ThemeSystem,
    ThemeLight,
    ThemeDark,
    /// 切换专注模式（阶段 1）。
    ToggleFocus,
    /// 切换源码模式（Ctrl+/）：全文按 Markdown 源码排版。
    ToggleSourceMode,
    /// 全局阻止/允许远程图片（默认自动加载）。
    ToggleRemoteImages,
    /// 视图缩放（3c-2）：放大/缩小一档、还原 100%。
    ZoomIn,
    ZoomOut,
    ZoomReset,
    /// 全屏切换（3c-3，F11）：无边框全屏。
    ToggleFullscreen,
    /// 打开设置模态（Ctrl+,）：所有配置项集中编辑，写 config.toml。
    Settings,
    /// 关于 Veil：应用内模态展示版本/构建提交/许可/隐私与官方来源。
    About,
    /// 导出诊断信息：显式保存对话框写出白名单字段文本。
    ExportDiagnostics,
    /// 检查更新：只用默认浏览器打开官方 Release 页，不做进程内网络请求。
    CheckUpdate,
    Quit,
    /// 打开最近文件（0b）：下标为最近列表位置（0 = 最近一次）。
    OpenRecent(usize),
}

pub struct MenuDef {
    pub title: Key,
    /// (标签 key, 快捷键提示 key, 动作)
    pub items: &'static [(Key, Key, MenuAction)],
}

pub const MENUS: [MenuDef; 4] = [
    MenuDef {
        title: Key::MenuFile,
        items: &[
            (Key::MenuFileNew, Key::ShortcutNew, MenuAction::New),
            (
                Key::MenuFileNewWindow,
                Key::ShortcutNewWindow,
                MenuAction::NewWindow,
            ),
            (Key::MenuFileOpen, Key::ShortcutOpen, MenuAction::Open),
            (
                Key::MenuFileQuickOpen,
                Key::ShortcutQuickOpen,
                MenuAction::QuickOpen,
            ),
            (Key::Empty, Key::Empty, MenuAction::Separator),
            (Key::MenuFileSave, Key::ShortcutSave, MenuAction::Save),
            (Key::MenuFileSaveAs, Key::ShortcutSaveAs, MenuAction::SaveAs),
            (Key::Empty, Key::Empty, MenuAction::Separator),
            (
                Key::MenuFileExportHtml,
                Key::ShortcutExportHtml,
                MenuAction::Export,
            ),
            (
                Key::MenuFileExportPdf,
                Key::ShortcutExportPdf,
                MenuAction::ExportPdf,
            ),
            (Key::MenuFilePrint, Key::ShortcutPrint, MenuAction::Print),
            (
                Key::MenuFileVersionHistory,
                Key::ShortcutVersionHistory,
                MenuAction::VersionHistory,
            ),
            (Key::Empty, Key::Empty, MenuAction::Separator),
            (Key::MenuFileQuit, Key::Empty, MenuAction::Quit),
        ],
    },
    MenuDef {
        title: Key::MenuEdit,
        items: &[
            (Key::MenuEditUndo, Key::ShortcutUndo, MenuAction::Undo),
            (Key::MenuEditRedo, Key::ShortcutRedo, MenuAction::Redo),
            (Key::Empty, Key::Empty, MenuAction::Separator),
            (Key::MenuEditCut, Key::ShortcutCut, MenuAction::Cut),
            (Key::MenuEditCopy, Key::ShortcutCopy, MenuAction::Copy),
            (Key::MenuEditPaste, Key::ShortcutPaste, MenuAction::Paste),
            (
                Key::MenuEditSelectAll,
                Key::ShortcutSelectAll,
                MenuAction::SelectAll,
            ),
            (Key::Empty, Key::Empty, MenuAction::Separator),
            (Key::MenuEditBold, Key::ShortcutBold, MenuAction::ToggleBold),
            (
                Key::MenuEditItalic,
                Key::ShortcutItalic,
                MenuAction::ToggleItalic,
            ),
            (Key::MenuEditLink, Key::ShortcutLink, MenuAction::InsertLink),
            (Key::Empty, Key::Empty, MenuAction::Separator),
            (Key::MenuEditFind, Key::ShortcutFind, MenuAction::Find),
            (
                Key::MenuEditReplace,
                Key::ShortcutReplace,
                MenuAction::Replace,
            ),
            (Key::Empty, Key::Empty, MenuAction::Separator),
            (
                Key::MenuEditInsertImage,
                Key::ShortcutInsertImage,
                MenuAction::InsertImage,
            ),
        ],
    },
    MenuDef {
        title: Key::MenuView,
        items: &[
            (
                Key::MenuViewOutline,
                Key::ShortcutOutline,
                MenuAction::ToggleOutline,
            ),
            (Key::Empty, Key::Empty, MenuAction::Separator),
            (Key::MenuViewZoomIn, Key::ShortcutZoomIn, MenuAction::ZoomIn),
            (
                Key::MenuViewZoomOut,
                Key::ShortcutZoomOut,
                MenuAction::ZoomOut,
            ),
            (
                Key::MenuViewZoomReset,
                Key::ShortcutZoomReset,
                MenuAction::ZoomReset,
            ),
            (Key::Empty, Key::Empty, MenuAction::Separator),
            (
                Key::MenuViewFullscreen,
                Key::ShortcutFullscreen,
                MenuAction::ToggleFullscreen,
            ),
            (Key::Empty, Key::Empty, MenuAction::Separator),
            (
                Key::MenuViewThemeSystem,
                Key::Empty,
                MenuAction::ThemeSystem,
            ),
            (Key::MenuViewThemeLight, Key::Empty, MenuAction::ThemeLight),
            (Key::MenuViewThemeDark, Key::Empty, MenuAction::ThemeDark),
            (Key::Empty, Key::Empty, MenuAction::Separator),
            (
                Key::MenuViewSourceMode,
                Key::ShortcutSourceMode,
                MenuAction::ToggleSourceMode,
            ),
            (Key::MenuViewFocusMode, Key::Empty, MenuAction::ToggleFocus),
            (Key::Empty, Key::Empty, MenuAction::Separator),
            (
                Key::MenuViewAllowRemoteImages,
                Key::Empty,
                MenuAction::ToggleRemoteImages,
            ),
            (Key::Empty, Key::Empty, MenuAction::Separator),
            (
                Key::MenuViewSettings,
                Key::ShortcutSettings,
                MenuAction::Settings,
            ),
        ],
    },
    MenuDef {
        title: Key::MenuHelp,
        items: &[
            (Key::MenuHelpAbout, Key::Empty, MenuAction::About),
            (
                Key::MenuHelpExportDiagnostics,
                Key::Empty,
                MenuAction::ExportDiagnostics,
            ),
            (
                Key::MenuHelpCheckUpdate,
                Key::Empty,
                MenuAction::CheckUpdate,
            ),
        ],
    },
];

/// chrome 专用近似文本宽度：CJK 全角按字号、ASCII 按半字号。
/// 只服务菜单栏/弹窗的矩形与命中区，不参与文档排版；文字绘制走 LayoutEngine
/// 实测光栅，二者允许数像素差异（命中框略宽无碍）。这样命中判定不依赖上一帧绘制状态。
const ASCII_ADVANCE_RATIO: f32 = 0.52;

fn text_width(text: &str, scale: f32) -> f32 {
    text.chars()
        .map(|c| {
            if c.is_ascii() {
                typography::UI_SIZE * ASCII_ADVANCE_RATIO * scale
            } else {
                typography::UI_SIZE * scale
            }
        })
        .sum()
}

/// 菜单标题矩形：(菜单下标, x, 宽)。标题间无间隙，命中按矩形判定。
pub fn menu_bar_rects(scale: f32) -> Vec<(usize, f32, f32)> {
    let pad = 14.0 * scale;
    let mut x = 8.0 * scale;
    let mut out = Vec::with_capacity(MENUS.len());
    for (i, m) in MENUS.iter().enumerate() {
        let w = text_width(t(m.title), scale) + 2.0 * pad;
        out.push((i, x, w));
        x += w;
    }
    out
}

/// 菜单栏命中：仅菜单栏高度内有效。
pub fn hit_menu_bar(x: f32, y: f32, scale: f32) -> Option<usize> {
    if y < 0.0 || y >= MENU_H * scale {
        return None;
    }
    menu_bar_rects(scale)
        .into_iter()
        .find(|&(_, rx, rw)| x >= rx && x < rx + rw)
        .map(|(i, _, _)| i)
}

/// 菜单项行（0b 动态化）：静态项 + 文件菜单末尾插入的最近文件（在「退出」之前）。
/// 返回 (标签, 快捷键提示, 动作)；最近文件标签由调用方按 `path_label` 生成
/// （`文件名 · 父目录`，同名文件可区分）。
/// Separator 项原样返回，由 popup_layout / paint 层处理（跳过命中、绘制分隔线）。
pub(crate) fn menu_rows(open: usize, recent: &[String]) -> Vec<(String, &'static str, MenuAction)> {
    let Some(menu) = MENUS.get(open) else {
        return Vec::new();
    };
    let statics = || {
        menu.items
            .iter()
            .map(|(l, s, a)| (t(*l).to_string(), t(*s), *a))
    };
    // 仅文件菜单（下标 0）且有最近文件时动态插入
    if open != 0 || recent.is_empty() {
        return statics().collect();
    }
    // 插入点：最后一条分隔线之后（即「退出」区块之前）。
    // 结果结构：… 打印 │ 最近文件… │ 退出，最近文件块上下各有分隔线。
    let insert = menu
        .items
        .iter()
        .rposition(|(_, _, a)| matches!(a, MenuAction::Separator))
        .map(|i| i + 1)
        .unwrap_or(menu.items.len());
    let mut out: Vec<_> = statics().take(insert).collect();
    out.extend(
        recent
            .iter()
            .enumerate()
            .map(|(i, l)| (l.clone(), "", MenuAction::OpenRecent(i))),
    );
    out.push((String::new(), "", MenuAction::Separator));
    out.extend(statics().skip(insert));
    out
}

/// 弹出菜单布局：位置贴当前菜单标题左缘，宽取最长项（含快捷键）并留内边距，
/// 但不超过窗口宽度减去菜单左缘与外边距，且钳制到 [150px, 420px]（1.0× 基准）。
pub struct PopupLayout {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// (项下标, x, y, w, h)；Separator 项不进 hit rect
    pub items: Vec<(usize, f32, f32, f32, f32)>,
}

/// 弹出菜单宽度上限（1.0× 基准）：专业编辑器菜单一般不超过此值。
const POPUP_MAX_W: f32 = 420.0;
/// 弹出菜单宽度下限（1.0× 基准）。
const POPUP_MIN_W: f32 = 150.0;
/// 弹出菜单与窗口右缘的最小外边距（1.0× 基准）。
const POPUP_RIGHT_MARGIN: f32 = 8.0;

pub fn popup_layout(open: usize, scale: f32, recent: &[String], win_w: f32) -> PopupLayout {
    let rows = menu_rows(open, recent);
    if rows.is_empty() {
        return PopupLayout {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
            items: Vec::new(),
        };
    }
    let x = menu_bar_rects(scale)
        .into_iter()
        .find(|&(i, _, _)| i == open)
        .map(|(_, x, _)| x)
        .unwrap_or(0.0);
    let pad = crate::theme::surface::padding(scale);
    let gap = 24.0 * scale;
    // 宽度同时为「禁用原因」预留：原因在快捷键列位置显示，若不计入宽度，
    // 动作名会被迫中段省略（用户看不到自己在选什么）。上限仍是 POPUP_MAX_W。
    let widest = rows
        .iter()
        .filter(|(_, _, a)| !matches!(a, MenuAction::Separator))
        .map(|(label, sc, action)| {
            let shortcut_w = if sc.is_empty() {
                0.0
            } else {
                gap + text_width(sc, scale)
            };
            // 原因列留一点余量：text_width 是近似值，实际光栅 ASCII 略宽；
            // 余量保证动作名与原因同时整段可见，而不是靠省略号收场。
            let reason_w = disabled_reason_key(*action)
                .map(|key| gap + text_width(t(key), scale) + space::M * scale)
                .unwrap_or(0.0);
            text_width(label, scale) + shortcut_w.max(reason_w)
        })
        .fold(0.0_f32, f32::max);
    let content_w = widest + 2.0 * pad;
    let win_limit = (win_w - x - POPUP_RIGHT_MARGIN * scale).max(POPUP_MIN_W * scale);
    let cap = POPUP_MAX_W * scale;
    let w = content_w.min(win_limit).min(cap).max(POPUP_MIN_W * scale);
    let item_h = crate::theme::surface::row_height(scale);
    // 分组只用间距，不画分隔线（S3）：8px 落在 4pt 网格上。
    let sep_h = space::S * scale;
    let top_pad = 4.0 * scale;
    let y = MENU_H * scale;
    let mut cur_y = top_pad;
    let mut items = Vec::new();
    for (i, (_label, _sc, action)) in rows.iter().enumerate() {
        if matches!(action, MenuAction::Separator) {
            cur_y += sep_h;
        } else {
            items.push((i, x, y + cur_y, w, item_h));
            cur_y += item_h;
        }
    }
    PopupLayout {
        x,
        y,
        w,
        h: cur_y + top_pad,
        items,
    }
}

/// 弹出菜单内命中项下标。
pub fn hit_popup_item(
    x: f32,
    y: f32,
    open: usize,
    scale: f32,
    recent: &[String],
    win_w: f32,
) -> Option<usize> {
    popup_layout(open, scale, recent, win_w)
        .items
        .into_iter()
        .find(|&(_, ix, iy, iw, ih)| x >= ix && x < ix + iw && y >= iy && y < iy + ih)
        .map(|(i, _, _, _, _)| i)
}

/// 点是否落在弹出菜单矩形内（含内边距；用于「点在菜单内但不在项上」时吞掉点击）。
pub fn popup_contains(
    x: f32,
    y: f32,
    open: usize,
    scale: f32,
    recent: &[String],
    win_w: f32,
) -> bool {
    let p = popup_layout(open, scale, recent, win_w);
    x >= p.x && x < p.x + p.w && y >= p.y && y < p.y + p.h
}

pub fn action_at(open: usize, item: usize, recent: &[String]) -> Option<MenuAction> {
    menu_rows(open, recent).get(item).map(|(_, _, a)| *a)
}

/// 最大滚动量（文档高不超过视口时恒 0）。
pub fn max_scroll(vp: Viewport, doc_h: f32) -> f32 {
    (doc_h - vp.h).max(0.0)
}

/// 滚动条滑块 (y, h)；文档不溢出视口时 None。
pub fn scroll_thumb(vp: Viewport, doc_h: f32, scroll: f32, scale: f32) -> Option<(f32, f32)> {
    if doc_h <= vp.h + 0.5 {
        return None;
    }
    // 退化视口（如最小化后 vp.h=1.0）根本无法容纳最小滑块，直接放弃；
    // 这也避免 clamp(THUMB_MIN_H*scale=36, vp.h=1) 时 min > max 的 panic。
    if vp.h < THUMB_MIN_H * scale {
        return None;
    }
    let h = (vp.h * vp.h / doc_h).clamp(THUMB_MIN_H * scale, vp.h);
    let max = max_scroll(vp, doc_h);
    if max <= 0.0 {
        return None;
    }
    let y = vp.y + (scroll.clamp(0.0, max) / max) * (vp.h - h);
    Some((y, h))
}

/// 滑块顶边位置 → 滚动量（拖动换算）。
pub fn scroll_from_thumb(vp: Viewport, doc_h: f32, thumb_y: f32, scale: f32) -> f32 {
    let Some((_, h)) = scroll_thumb(vp, doc_h, 0.0, scale) else {
        return 0.0;
    };
    let span = vp.h - h;
    if span <= 0.0 {
        return 0.0;
    }
    ((thumb_y - vp.y) / span).clamp(0.0, 1.0) * max_scroll(vp, doc_h)
}

/// 翻页（dir=±1），按视口高 90% 步进并钳位。
pub fn page_scroll(vp: Viewport, doc_h: f32, scroll: f32, dir: i32) -> f32 {
    (scroll + dir as f32 * vp.h * 0.9).clamp(0.0, max_scroll(vp, doc_h))
}

/// 滚动条命中：视口右缘 SCROLL_W 内，且文档确实溢出。
pub fn scrollbar_hit(vp: Viewport, doc_h: f32, x: f32, y: f32, scale: f32) -> bool {
    doc_h > vp.h + 0.5
        && y >= vp.y
        && y < vp.y + vp.h
        && x >= vp.x + vp.w - SCROLL_W * scale
        && x < vp.x + vp.w
}

/// 路径标签：`文件名 · 父目录`，无路径时返回「未命名草稿」。
/// 用于窗口标题、状态栏、notice 等所有需要展示路径的位置——统一避免完整路径太吵。
pub fn path_label(p: Option<&Path>) -> String {
    let Some(p) = p else {
        return t(Key::UntitledDraft).to_string();
    };
    let stem = p
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(t(Key::UntitledDraft));
    let parent = p
        .parent()
        .and_then(|dir| dir.file_name())
        .and_then(|n| n.to_str());
    match parent {
        Some(parent) if !parent.is_empty() => format!("{} · {}", stem, parent),
        _ => stem.to_string(),
    }
}

/// 状态栏左文分段：显示顺序 = 文件身份 → 外部变更 → 源码模式 → 保存态。
///
/// 显示顺序与优先级解耦：保存态虽然显示在最后，却是仅次于文件身份的必留信息。
/// 各段独立成串，供绘制层按可用宽度逐级丢弃低优先级段。
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct StatusParts {
    pub identity: String,
    pub external: String,
    pub source: String,
    pub save: String,
}

impl StatusParts {
    /// 全量左文（既有显示顺序）。
    pub fn full(&self) -> String {
        format!(
            "{}{}{}{}",
            self.identity, self.external, self.source, self.save
        )
    }

    /// 丢弃最低优先级的源码模式。
    pub fn without_source(&self) -> String {
        format!("{}{}{}", self.identity, self.external, self.save)
    }

    /// 仅保留文件身份与保存态（两者都不允许被静默吞掉）。
    pub fn core(&self) -> String {
        format!("{}{}", self.identity, self.save)
    }
}

/// 状态栏左文降级档位（优先级高 → 低：文件身份 > 保存态 > 外部变更 > 源码模式）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StatusTier {
    Full,
    NoSource,
    Core,
}

/// 纯决策：按优先级从最低段开始丢，返回可用宽能容纳的最高档位。
/// `Core` 仍超宽时由绘制层收缩文件名，因此这里恒定可返回。
pub fn pick_status_tier(full_w: f32, no_source_w: f32, avail: f32) -> StatusTier {
    if full_w <= avail {
        StatusTier::Full
    } else if no_source_w <= avail {
        StatusTier::NoSource
    } else {
        StatusTier::Core
    }
}

/// 状态栏左文分段构造（单一来源）。脏态只用「 · 未保存」表达，不再叠加 ` *`。
pub fn status_parts(
    path: Option<&Path>,
    dirty: bool,
    external: ExternalState,
    source_mode: bool,
    saved_hhmm: Option<&str>,
) -> StatusParts {
    let mut parts = StatusParts {
        identity: path_label(path),
        external: String::new(),
        source: String::new(),
        save: String::new(),
    };
    parts.external = match external {
        ExternalState::None => String::new(),
        ExternalState::Changed => t(Key::StatusExternalChanged).to_string(),
        ExternalState::BackedUp => t(Key::StatusExternalBackedUp).to_string(),
        ExternalState::Missing => t(Key::StatusExternalMissing).to_string(),
    };
    if source_mode {
        parts.source = t(Key::StatusSourceMode).to_string();
    }
    if dirty {
        parts.save = t(Key::StatusUnsaved).to_string();
    } else if let Some(hhmm) = saved_hhmm {
        parts.save = tf(Key::StatusSaved, &[hhmm]);
    }
    parts
}

/// 状态栏右文：全文字数 + 行列。
pub fn status_stats(words: usize, line: usize, col: usize) -> String {
    tf(
        Key::StatusStats,
        &[&words.to_string(), &line.to_string(), &col.to_string()],
    )
}

/// 极窄窗口下右文的退化形式：仅字数（行列信息让位于左文可读性）。
pub fn status_words(words: usize) -> String {
    tf(Key::StatusWords, &[&words.to_string()])
}

/// 状态栏文本：(左, 右)。左 = 文件身份 + 外部变更 + 源码模式 + 保存态；
/// 右 = 全文字数 + 行列。
///
/// 保存态是显性文字：脏 → `· 未保存`；干净且本会话保存过 → `· 已保存 HH:MM`
/// （时间由注入的 Clock 提供）。外部修改/文件缺失/源码模式后缀保持既有顺序与文案。
/// 绘制路径走 `layout_status`（同源分段 + 优先级降级），本函数保留给断言全量文案的测试。
#[cfg(test)]
#[allow(clippy::too_many_arguments)] // 状态栏聚合字段，调用点只有测试
pub fn status_line(
    path: Option<&Path>,
    dirty: bool,
    external: ExternalState,
    source_mode: bool,
    saved_hhmm: Option<&str>,
    words: usize,
    line: usize,
    col: usize,
) -> (String, String) {
    (
        status_parts(path, dirty, external, source_mode, saved_hhmm).full(),
        status_stats(words, line, col),
    )
}

/// 状态栏通知语义类别。显式标注，不按文案猜测：
/// 错误类不参与自动超时（保留到被新通知替换或用户处理），其余类别短暂显示。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum NoticeKind {
    #[default]
    Info,
    Success,
    Error,
}

impl NoticeKind {
    /// 是否参与定时消退（错误类恒不自动消失）。
    pub fn expires(self) -> bool {
        self != NoticeKind::Error
    }
}

/// chrome 交互态与标签缓存。`notice` 为状态栏临时提示（动作失败/成功）。
#[derive(Default)]
pub struct Chrome {
    pub open: Option<usize>,
    pub hover_menu: Option<usize>,
    /// (菜单下标, 项下标)
    pub hover_item: Option<(usize, usize)>,
    pub notice: String,
    /// `notice` 的语义类别，与 `notice_kind_text` 配对；文本被直接替换时失效为 Info。
    pub notice_kind: NoticeKind,
    pub(crate) notice_kind_text: String,
    /// 悬停最近文件项时，临时保存的原状态栏提示（离开后恢复）。
    pub(crate) saved_notice: Option<String>,
    /// 拖动滚动条时鼠标相对滑块顶边的偏移。
    pub scroll_drag: Option<f32>,
    /// 指针悬停在滚动条上（滑块或轨道）：滑块加亮，与拖动态同一视觉。
    pub scroll_hover: bool,
    pub(crate) labels: LabelCache,
    /// 状态栏左文中段省略结果缓存（S1）：键 = (原文, 可用宽 bits, scale bits)。
    /// 打字期右文变而左文/窗口不变，命中避免每帧二分重排。
    pub(crate) status_fit: std::collections::HashMap<(String, u32, u32), String>,
    /// 菜单栏项悬停过渡（S6）。
    pub(crate) menu_fade: Fade,
    /// 弹出菜单项悬停过渡（S6）。
    pub(crate) popup_fade: Fade,
}

impl Chrome {
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// 写入通知并显式标注语义类别。错误类不会自动超时。
    pub fn notify(&mut self, text: impl Into<String>, kind: NoticeKind) {
        let text = text.into();
        self.notice = text.clone();
        self.notice_kind = kind;
        self.notice_kind_text = text;
    }

    /// 错误通知：保留到被新通知替换或用户处理。
    pub fn notify_error(&mut self, text: impl Into<String>) {
        self.notify(text, NoticeKind::Error);
    }

    /// 当前通知的有效类别：文本被外部直接替换后配对失效，按 Info 处理，    /// 避免上一条错误通知的「不超时」语义污染后续成功提示。
    pub fn current_notice_kind(&self) -> NoticeKind {
        if self.notice == self.notice_kind_text {
            self.notice_kind
        } else {
            NoticeKind::Info
        }
    }

    pub fn close(&mut self) {
        self.open = None;
        self.hover_item = None;
        if let Some(saved) = self.saved_notice.take() {
            self.notice = saved;
        }
    }

    pub fn toggle(&mut self, menu: usize) {
        if self.open == Some(menu) {
            self.close();
        } else {
            self.open = Some(menu);
            self.hover_item = None;
        }
    }

    /// DPI 变化：标签缓存的像素尺寸按旧 scale 生成，必须整体作废。
    pub fn clear_label_cache(&mut self) {
        self.labels.clear();
        self.status_fit.clear();
    }

    /// 推进 chrome 悬停过渡（S6）：菜单栏项与弹出菜单项各一条。
    pub(crate) fn anim_tick(&mut self, now: Instant) {
        self.menu_fade.tick(now, self.hover_menu);
        // 弹出项下标在同一打开菜单内唯一，直接作为过渡槽标识。
        self.popup_fade.tick(now, self.hover_item.map(|(_, i)| i));
    }

    /// 是否仍有过渡在跑。
    pub(crate) fn anim_running(&self) -> bool {
        self.menu_fade.running() || self.popup_fade.running()
    }

    /// 动效开关（系统「显示动画」）：关闭后悬停立即到位。
    pub(crate) fn set_animations(&mut self, on: bool) {
        self.menu_fade.set_enabled(on);
        self.popup_fade.set_enabled(on);
    }
}

/// 主题菜单项与当前模式是否一致（用于「当前」标记；非主题项恒 false）。
pub(super) fn theme_action_matches(action: MenuAction, mode: ThemeMode) -> bool {
    matches!(
        (action, mode),
        (MenuAction::ThemeSystem, ThemeMode::System)
            | (MenuAction::ThemeLight, ThemeMode::Light)
            | (MenuAction::ThemeDark, ThemeMode::Dark)
    )
}

/// 菜单开关项的当前开启状态（「当前」提示的唯一数据源）+ 项可用性。
/// 不包含大纲面板（用户要求：大纲是面板开关而非写作模式，不显示任何状态标识）。
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(crate) struct MenuFlags {
    pub(crate) fullscreen: bool,
    pub(crate) focus: bool,
    pub(crate) source: bool,
    /// 允许加载远程图片（与设置同一极性：勾选 = 允许）。
    pub(crate) allow_remote_images: bool,
    /// 有可保存内容（脏文档或未命名草稿）。
    pub(crate) can_save: bool,
    /// 正文非空：导出/打印有可输出内容（空白但非空的文档仍然可导出）。
    pub(crate) can_export: bool,
    pub(crate) can_undo: bool,
    pub(crate) can_redo: bool,
}

/// 开关项是否处于开启态（非开关项恒 false）：决定弹出菜单该行右侧是否显示「当前」。
pub(crate) fn action_checked(action: MenuAction, flags: MenuFlags) -> bool {
    match action {
        MenuAction::ToggleFullscreen => flags.fullscreen,
        MenuAction::ToggleFocus => flags.focus,
        MenuAction::ToggleSourceMode => flags.source,
        MenuAction::ToggleRemoteImages => flags.allow_remote_images,
        _ => false,
    }
}

/// 动作「可能」被禁用的原因词条（与运行态无关）：供菜单布局预留宽度，
/// 保证禁用原因出现时动作名无需让位。
pub(crate) fn disabled_reason_key(action: MenuAction) -> Option<Key> {
    match action {
        MenuAction::Save => Some(Key::DisabledNoChanges),
        MenuAction::Export | MenuAction::ExportPdf | MenuAction::Print => {
            Some(Key::DisabledEmptyDocument)
        }
        MenuAction::Undo => Some(Key::DisabledNothingToUndo),
        MenuAction::Redo => Some(Key::DisabledNothingToRedo),
        _ => None,
    }
}

/// 禁用原因（单一来源）：不可用时返回给用户看的原因词条，可用时 None。
/// 只表达「此刻必然无效果」的项；拿不准的（如剪贴板是否为空）一律保持可用。
/// 菜单在快捷键列以 muted 文本呈现该原因，避免只置灰不解释。
pub(crate) fn disabled_reason(action: MenuAction, flags: MenuFlags) -> Option<Key> {
    let key = disabled_reason_key(action)?;
    let disabled = match action {
        MenuAction::Save => !flags.can_save,
        MenuAction::Export | MenuAction::ExportPdf | MenuAction::Print => !flags.can_export,
        MenuAction::Undo => !flags.can_undo,
        MenuAction::Redo => !flags.can_redo,
        _ => false,
    };
    disabled.then_some(key)
}

/// 菜单项是否可用：不可用时置灰、不可高亮、不可回车/点击执行。
/// 与 `disabled_reason` 同源，保证「置灰」与「解释」不会各说各话。
pub(crate) fn action_enabled(action: MenuAction, flags: MenuFlags) -> bool {
    !matches!(action, MenuAction::Separator) && disabled_reason(action, flags).is_none()
}

#[cfg(test)]
mod tests;
