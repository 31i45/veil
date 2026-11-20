//! 打印（阶段 3b）：分页引擎与打印会话。
//! 本层提供机制——整页干净帧（无 chrome/光标/选区，Reveal 全收）+ 行级分页几何；
//! 策略（打印对话框、设备上下文、逐页 blit）由产品层（veil）经 `Print` 端口注入。
//! 本文件零 Win32 依赖，headless 全测（同 Clipboard/FileStore 模式，INSTANCE §6）。

use std::path::{Path, PathBuf};

use crate::app::App;
use crate::i18n::{t, tf, Key};
use crate::mermaid_renderer::MermaidImage;
use crate::theme::NativeWindow;
use crate::view::Frame;
use veil_core::{BlockKind, Range};

/// PDF 导出选项（页边距由产品层读取 `Config` 后自行换算）。
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct ExportOptions {
    pub page_numbers: bool,
    pub toc: bool,
}

/// 打印端口：实现由产品层注入（Win32 GDI + 系统打印对话框；PDF 走原生矢量管线）。
/// 一次打印 = `begin` → 逐页 `render_page` → `end`；实现方须保证 begin 之后
/// 必调 end（含出错/中止路径），否则应用停留在打印态（尺寸/缩放未恢复）。
pub trait Print {
    fn print(&mut self, session: &mut PrintSession);

    /// 直接导出 PDF（无打印对话框）。默认实现仅提示，不产生输出。
    fn export_pdf(&mut self, session: &mut PrintSession) {
        session.notice(t(Key::PrintUnsupportedPdf));
    }
}

/// 无平台实现时的兜底（非 Windows / 产品层未注入）：只提示，不产生任何输出。
pub struct NoPrint;
impl Print for NoPrint {
    fn print(&mut self, session: &mut PrintSession) {
        session.notice(t(Key::PrintUnsupportedPrint));
    }
}

/// 打印中的瞬态状态（`WindowState.print`）：页帧尺寸 + 分页结果 + 被打断前的屏幕态。
/// 存在即表示「正在打印」：win_size/viewport/content_width/绘制全部切换到页帧语义。
pub(crate) struct PrintState {
    /// 打印页帧（物理像素）：win_size() 在打印期间返回它，render() 产出整页帧。
    pub(crate) w: u32,
    pub(crate) h: u32,
    /// 打印前的屏幕 DPI 缩放（end 时恢复）。
    pub(crate) saved_scale: f32,
    /// 打印前的滚动位置（end 时恢复）。
    pub(crate) saved_scroll: f32,
    /// 页首 y 坐标序列（文档坐标，物理像素）：breaks[0] = 0.0，页数 = len。
    pub(crate) breaks: Vec<f32>,
}

/// 打印会话：产品层 Print 实现与 App 之间的唯一通道。
/// 字段 pub(crate) 供 in-crate 测试直达；产品层只用下列 pub 方法。
pub struct PrintSession<'a> {
    pub(crate) app: &'a mut App,
}

impl PrintSession<'_> {
    /// 进入打印：按页帧尺寸与 DPI 缩放重排全文（Reveal 全收），算好分页，返回页数。
    /// 页帧即「可打印区域」（产品层已扣除打印机硬边距）：宽 = w，高 = h（物理像素）。
    pub fn begin(&mut self, w: u32, h: u32, scale: f32) -> usize {
        self.begin_inner(w, h, scale)
    }

    fn begin_inner(&mut self, w: u32, h: u32, scale: f32) -> usize {
        let app = &mut *self.app;
        app.window_state.print = Some(PrintState {
            w: w.max(1),
            h: h.max(1),
            saved_scale: app.input_state.scale,
            saved_scroll: app.window_state.scroll_y,
            breaks: Vec::new(),
        });
        app.input_state.scale = scale.max(0.1);
        app.window_state.scroll_y = 0.0;
        // 宽度/缩放已变：全量重排（缓存按新 base_key 整体失配）后以真实高度重算几何
        app.refresh_index();
        let len = app.index.blocks.len();
        app.ensure_layouts_range(0..len);
        app.ensure_math_range(0..len);
        app.ensure_tables_range(0..len);
        app.recompute_geometry();
        let frame_h = app.window_state.print.as_ref().unwrap().h as f32;
        let breaks = app.compute_page_breaks(frame_h);
        let pages = breaks.len();
        app.window_state.print.as_mut().unwrap().breaks = breaks;
        pages
    }

    /// 渲染第 i 页（0-based，i < begin 返回值）：返回整页帧。
    /// 滚动到页首，逐页重建光栅（跨页累积会让长文档打印的内存随页数线性增长）。
    pub fn render_page(&mut self, i: usize) -> &Frame {
        let app = &mut *self.app;
        let scroll = app
            .window_state
            .print
            .as_ref()
            .and_then(|p| p.breaks.get(i).copied())
            .unwrap_or(0.0);
        app.window_state.scroll_y = scroll;
        app.cache.rasters.clear();
        app.render()
    }

    /// 当前配置的导出选项（原生 PDF 导出前读取）。
    pub fn export_options(&self) -> ExportOptions {
        ExportOptions {
            page_numbers: self.app.config.export_page_numbers,
            toc: self.app.config.export_toc,
        }
    }

    /// 产品层换算页边距用（毫米）。
    pub fn export_margin_mm(&self) -> f32 {
        self.app.config.export_margin_mm
    }

    /// 导出建议路径：文档同目录 `<stem>.pdf`；草稿为裸名（平台层补「文档」目录）。
    /// 仅用于预填对话框，绝不作为静默落盘目标。
    pub fn export_suggestion(&self) -> PathBuf {
        match self.app.doc.path.as_ref() {
            Some(p) => p.with_extension("pdf"),
            None => PathBuf::from(t(Key::ExportUntitledPdf)),
        }
    }

    /// 导出输出路径：每次导出都由用户确认（`ext` 决定标题与过滤器）。
    /// None = 用户取消（产品层据此中止，不产生任何文件）。
    pub fn choose_export_path(&mut self, suggested: &Path, ext: &str) -> Option<PathBuf> {
        self.app
            .ports
            .dialogs
            .save_export(&suggested.to_string_lossy(), ext)
    }

    /// 结束打印：恢复屏幕态（DPI/滚动/窗口尺寸），触发按屏幕参数重排。
    pub fn end(&mut self) {
        let app = &mut *self.app;
        let Some(state) = app.window_state.print.take() else {
            return;
        };
        app.input_state.scale = state.saved_scale;
        app.window_state.scroll_y = state.saved_scroll;
        app.refresh_index();
        app.recompute_geometry();
        app.request_redraw();
    }

    /// 状态栏提示（取消/失败/完成由产品层决定文案）。
    pub fn notice(&mut self, msg: &str) {
        self.app.ui_state.chrome.notice = msg.to_string();
        self.app.request_redraw();
    }

    /// 打印失败/中断：与其它失败态同一错误弹窗（重试重新打印 / 取消）。
    pub fn print_error(&mut self, reason: &str) {
        self.app.ui_state.chrome.notice = tf(Key::NoticePrintFailed, &[reason]);
        self.app.show_error_modal(
            t(Key::ModalPrintFailedTitle),
            tf(Key::ModalPrintFailedBody, &[reason]),
            crate::app::ErrorPurpose::Print,
            vec![
                crate::view::modal::ModalButton::new(
                    1,
                    t(Key::BtnRetry),
                    crate::view::modal::ButtonRole::Primary,
                )
                .with_mnemonic('r'),
                crate::view::modal::ModalButton::new(
                    3,
                    t(Key::BtnCancel),
                    crate::view::modal::ButtonRole::Secondary,
                ),
            ],
        );
    }

    /// 导出 PDF 失败：与应用内其它失败态共用同一错误弹窗（重试 / 取消）。
    pub fn export_pdf_error(&mut self, reason: &str) {
        self.app.ui_state.chrome.notice = tf(Key::NoticeExportPdfFailed, &[reason]);
        let buttons = vec![
            crate::view::modal::ModalButton::new(
                1,
                t(Key::BtnRetry),
                crate::view::modal::ButtonRole::Primary,
            )
            .with_mnemonic('r'),
            crate::view::modal::ModalButton::new(
                3,
                t(Key::BtnCancel),
                crate::view::modal::ButtonRole::Secondary,
            ),
        ];
        self.app.show_error_modal(
            t(Key::ModalExportPdfFailedTitle),
            tf(Key::ModalExportPdfFailedBody, &[reason]),
            crate::app::ErrorPurpose::ExportPdf,
            buttons,
        );
    }

    /// 窗口句柄（打印对话框的 owner）；无窗口/非 Windows 返回 None。
    pub fn window_handle(&self) -> Option<NativeWindow> {
        #[cfg(windows)]
        {
            use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
            let win = self.app.window_state.window.as_ref()?;
            let handle = win.window_handle().ok()?;
            if let RawWindowHandle::Win32(p) = handle.as_raw() {
                return Some(NativeWindow { hwnd: p.hwnd.get() });
            }
        }
        None
    }

    /// 分页结果（测试断言用）：页首 y 坐标序列；非打印中返回空。
    pub fn page_breaks(&self) -> &[f32] {
        match &self.app.window_state.print {
            Some(p) => &p.breaks,
            None => &[],
        }
    }

    /// 当前文档的完整 Markdown 源文本（原生 PDF / 导出侧用）。
    pub fn markdown_text(&self) -> String {
        let sess = self.app.editor.session();
        let len = sess.text_len();
        sess.text_slice(Range::new(0, len)).unwrap_or_default()
    }

    /// 只读语法会话（原生 PDF 版面重建用；不暴露可变入口）。
    pub fn session(&self) -> &veil_core::Session {
        self.app.editor.session()
    }

    /// 当前文档所在目录（相对图片路径的解析基准）；未保存返回 None。
    pub fn document_dir(&self) -> Option<std::path::PathBuf> {
        self.app
            .doc
            .path
            .as_ref()
            .and_then(|p| p.parent())
            .map(|p| p.to_path_buf())
    }

    /// 当前缓存中已光栅化的 Mermaid 图表（BlockId → MermaidImage）。
    /// 导出/打印时用于把 Mermaid 块绘制成位图。空 HashMap 表示无数据。
    pub fn mermaid_images(&self) -> std::collections::HashMap<veil_core::BlockId, MermaidImage> {
        self.app.cache.mermaid_imgs.clone()
    }

    /// 导出/打印前补齐缺失的 Mermaid 渲染（同步、有超时预算）。
    ///
    /// 由平台实现方在「用户已确认输出位置之后」调用：放在对话框之前会让取消操作
    /// 也白等网络超时。重复调用幂等（命中缓存即跳过）。
    pub fn prepare_mermaid(&mut self) {
        self.app.ensure_mermaid_for_export();
    }
}

impl App {
    /// 菜单「打印」入口（3b）：端口置换规避 app 与 ports.print 的同时借用。
    pub(crate) fn print_document(&mut self) {
        if !self.output_allowed() {
            return;
        }
        let mut port = std::mem::replace(&mut self.ports.print, Box::new(NoPrint));
        port.print(&mut PrintSession { app: self });
        self.ports.print = port;
    }

    /// 菜单「导出 PDF」入口：与打印同一端口置换，产品层只走原生矢量管线。
    pub(crate) fn export_pdf(&mut self) {
        if !self.output_allowed() {
            return;
        }
        let mut port = std::mem::replace(&mut self.ports.print, Box::new(NoPrint));
        port.export_pdf(&mut PrintSession { app: self });
        self.ports.print = port;
    }

    /// 布局/绘制的有效光标：打印中恒 usize::MAX —— 一切 Reveal 收拢（无激活节点、
    /// 无源码态、无光标），全文呈最终渲染形态；非打印即编辑器光标。
    pub(crate) fn effective_caret(&self) -> usize {
        if self.window_state.print.is_some() {
            usize::MAX
        } else {
            self.editor.caret()
        }
    }

    /// 分页（3b）：候选断点 = 各块 top + 文本块各视觉行行顶（渲染态数学图/表格
    /// 作原子块，只取块 top）。贪心取页内最深候选；无候选（单块/单行超页）硬切页底。
    /// 返回页首 y 序列（首项恒 0.0，项数即页数）。
    fn compute_page_breaks(&self, frame_h: f32) -> Vec<f32> {
        let mut breaks = vec![0.0];
        let total_h = self.index.total_h;
        if total_h <= frame_h {
            return breaks;
        }
        let base = self.base_key();
        let caret = self.effective_caret();
        let mut cands: Vec<f32> = Vec::new();
        for g in &self.index.blocks {
            cands.push(g.top);
            let atomic = (g.kind == BlockKind::Math && self.math_shows_image(g.id, &g.range))
                || (g.kind == BlockKind::Table && self.table_shows_grid(g.id, &g.range));
            if atomic {
                continue;
            }
            if let Some(l) = self.cache.layout(
                g.id,
                &g.kind,
                g.range,
                base,
                caret,
                self.toc_layout_epoch(g),
            ) {
                for t in self.engine.line_tops(l) {
                    cands.push(g.top + t);
                }
            }
        }
        cands.sort_by(|a, b| a.total_cmp(b));
        // 小于此步进的候选不得入选：防「断点 == 页首」零进展死循环
        const EPS: f32 = 0.5;
        loop {
            let start = *breaks.last().unwrap();
            let limit = start + frame_h;
            if limit >= total_h {
                break;
            }
            // 候选升序：取 (start + EPS, limit] 内最深者；空则硬切
            let lo = cands.partition_point(|&c| c <= start + EPS);
            let hi = cands.partition_point(|&c| c <= limit);
            if hi > lo {
                breaks.push(cands[hi - 1]);
            } else {
                breaks.push(limit);
            }
        }
        breaks
    }
}

/// 缩放一档：0（适宽）⇄ 0.5…4.0，步进 1.25。
pub(crate) fn next_zoom(cur: f32, up: bool) -> f32 {
    let base = if cur <= 0.0 { 1.0 } else { cur };
    let next = if up { base * 1.25 } else { base / 1.25 };
    if next < 0.4 {
        0.0 // 缩到很小即回到适宽
    } else {
        next.clamp(0.5, 4.0)
    }
}
