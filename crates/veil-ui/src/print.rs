//! 打印（阶段 3b）：分页引擎与打印会话。
//! 本层提供机制——整页干净帧（无 chrome/光标/选区，Reveal 全收）+ 行级分页几何；
//! 策略（打印对话框、设备上下文、逐页 blit）由产品层（veil）经 `Print` 端口注入。
//! 本文件零 Win32 依赖，headless 全测（同 Clipboard/FileStore 模式，INSTANCE §6）。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::app::App;
use crate::i18n::{t, tf, Key};
use crate::mermaid_renderer::MermaidImage;
use crate::theme::NativeWindow;
use crate::view::Frame;
use veil_core::{BlockId, BlockKind, Range};

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
        session.notice_error(t(Key::PrintUnsupportedPdf));
    }
}

/// 无平台实现时的兜底（非 Windows / 产品层未注入）：只提示，不产生任何输出。
pub struct NoPrint;
impl Print for NoPrint {
    fn print(&mut self, session: &mut PrintSession) {
        session.notice_error(t(Key::PrintUnsupportedPrint));
    }
}

/// 续页表头（R2）：页首落在表格内部时，本页顶部需要重绘该表的表头行。
/// `height` 是表头行高（物理像素），页面内容整体下移同等高度。
#[derive(Clone)]
pub(crate) struct ResumeHeader {
    pub(crate) table: BlockId,
    pub(crate) height: f32,
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
    /// 与 `breaks` 一一对应：该页页首落在表格内部时要重绘的表头（否则 None）。
    pub(crate) headers: Vec<Option<ResumeHeader>>,
    /// 当前页（`render_page` 设置）需要重绘的续页表头，供 paint 收尾使用。
    pub(crate) resume: Option<ResumeHeader>,
}

/// 打印会话：产品层 Print 实现与 App 之间的唯一通道。
/// 字段 pub(crate) 供 in-crate 测试直达；产品层只用下列 pub 方法。
pub struct PrintSession<'a> {
    pub(crate) app: &'a mut App,
    /// 本次打印/PDF 会话专用的 light Mermaid 图；与屏幕/HTML 主题缓存隔离。
    mermaid_imgs: HashMap<veil_core::BlockId, MermaidImage>,
}

impl<'a> PrintSession<'a> {
    /// 仅测试 harness（T5）：无 Mermaid 缓存的打印会话，用于 headless 分页结构断言。
    #[doc(hidden)]
    pub(crate) fn for_qa(app: &'a mut App) -> Self {
        Self {
            app,
            mermaid_imgs: HashMap::new(),
        }
    }
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
            headers: Vec::new(),
            resume: None,
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
        // R2：预先判定每页页首是否落在表格内部，render_page 据此下移内容并重绘表头。
        let headers: Vec<Option<ResumeHeader>> =
            breaks.iter().map(|&b| app.resume_table_header(b)).collect();
        let state = app.window_state.print.as_mut().unwrap();
        state.breaks = breaks;
        state.headers = headers;
        pages
    }

    /// 渲染第 i 页（0-based，i < begin 返回值）：返回整页帧。
    /// 滚动到页首，逐页重建光栅（跨页累积会让长文档打印的内存随页数线性增长）。
    /// 页首落在表格内部时 `scroll_y = 页首 - 表头高`（内容整体下移表头高，R2），
    /// 顶部空出的表头区由 paint 用重复表头覆盖。
    pub fn render_page(&mut self, i: usize) -> &Frame {
        let app = &mut *self.app;
        let (page_break, resume) = {
            let Some(p) = app.window_state.print.as_ref() else {
                return app.render();
            };
            (
                p.breaks.get(i).copied().unwrap_or(0.0),
                p.headers.get(i).cloned().flatten(),
            )
        };
        let shift = resume.as_ref().map_or(0.0, |r| r.height);
        app.window_state.scroll_y = (page_break - shift).max(0.0);
        if let Some(p) = app.window_state.print.as_mut() {
            p.resume = resume;
        }
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

    /// 错误状态栏提示：不参与自动超时，保留到被新通知替换或用户处理。
    pub fn notice_error(&mut self, msg: &str) {
        self.app.ui_state.chrome.notify_error(msg.to_string());
        self.app.request_redraw();
    }

    /// 打印失败/中断：与其它失败态同一错误弹窗（重试重新打印 / 取消）。
    pub fn print_error(&mut self, reason: &str) {
        self.notice_error(&tf(Key::NoticePrintFailed, &[reason]));
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
        self.notice_error(&tf(Key::NoticeExportPdfFailed, &[reason]));
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

    /// 当前打印会话已准备好的 light Mermaid 图表（BlockId → MermaidImage）。
    /// 不读取屏幕主题缓存，避免 dark 位图/SVG 混入固定浅色纸面。
    pub fn mermaid_images(&self) -> HashMap<veil_core::BlockId, MermaidImage> {
        self.mermaid_imgs.clone()
    }

    /// 导出/打印前补齐缺失的 light Mermaid 渲染（同步、有超时预算）。
    ///
    /// 由平台实现方在「用户已确认输出位置之后」调用：放在对话框之前会让取消操作
    /// 也白等渲染超时。重复调用幂等（命中本次会话的 light map 即跳过）。
    pub fn prepare_mermaid(&mut self) {
        self.app.ensure_mermaid_for_print(&mut self.mermaid_imgs);
    }
}

impl App {
    /// 菜单「打印」入口（3b）：端口置换规避 app 与 ports.print 的同时借用。
    pub(crate) fn print_document(&mut self) {
        if !self.output_allowed() {
            return;
        }
        let mut port = std::mem::replace(&mut self.ports.print, Box::new(NoPrint));
        port.print(&mut PrintSession {
            app: self,
            mermaid_imgs: HashMap::new(),
        });
        self.ports.print = port;
    }

    /// 菜单「导出 PDF」入口：与打印同一端口置换，产品层只走原生矢量管线。
    pub(crate) fn export_pdf(&mut self) {
        if !self.output_allowed() {
            return;
        }
        let mut port = std::mem::replace(&mut self.ports.print, Box::new(NoPrint));
        port.export_pdf(&mut PrintSession {
            app: self,
            mermaid_imgs: HashMap::new(),
        });
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

    /// 分页（3b）：候选断点 = 各块 top + 文本块各视觉行行顶 + 渲染态表格各行行顶
    /// （表头算第 0 行，行顶即断页点）；渲染态数学图/源码态表格作原子块，只取块顶；
    /// 高度不足一页的代码块同样整块不拆。贪心取页内最深候选；无候选（单块/单行/
    /// 单个表格行超页）硬切页底。返回页首 y 序列（首项恒 0.0，项数即页数）。
    ///
    /// R1：表格只在整行边界断页；单个表格行自身高于该页可用高度时才允许行内硬切。
    /// R2：页首落在表格内部时，该页可用高度扣掉重复表头高度（表体不被表头遮挡）。
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
            // 渲染态表格不原子：每行行顶都是合法的页首（表头行顶 = 块顶，已入列）。
            if g.kind == BlockKind::Table && self.table_shows_grid(g.id, &g.range) {
                if let Some(tl) = self.cache.table(g.id, self.table_key(g.id)) {
                    for r in &tl.rows {
                        cands.push(g.top + r.top);
                    }
                }
                continue;
            }
            // R4（接受并说明）：超高原子块（图片 / 渲染态公式 / Mermaid）在打印路径
            // 不做缩放——打印几何复用屏幕排版，改图高会破坏与正文的行距对应；
            // 单块高于一页时只能按页底硬切（见 `print_hard_cuts_atomic_block_taller_than_page`）。
            // 原生 PDF 导出则在 `pdf_layout::push_image` / `push_mermaid_image` 等比
            // 压到一页内、不产生硬切。两条路径此处是刻意保留的差异，规则以本注释为准。
            let atomic = g.kind == BlockKind::Math && self.math_shows_image(g.id, &g.range);
            if atomic {
                continue;
            }
            // 短代码块整块不拆：与原生 PDF 导出（`pdf_layout::code`）同一规则。
            // 块高 ≤ 一页时只保留块顶候选，页底放不下就整体移到下一页；
            // 超高代码块仍按行跨页，不做无意义的整页浪费。
            let short_code = matches!(&g.kind, BlockKind::FencedCode(_) | BlockKind::IndentedCode)
                && g.height <= frame_h;
            if short_code {
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
            // R2：续页页首要重绘表头，先把这页的可用高度扣减表头高。
            let reserved = self.resume_table_header(start).map_or(0.0, |h| h.height);
            let limit = start + (frame_h - reserved).max(1.0);
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

    /// `y` 落在某个渲染态表格的内部（严格在表顶之后、表底之前）时，返回续页表头。
    /// 用于 R2：该页内容下移表头高并在顶部重绘表头行。
    fn resume_table_header(&self, y: f32) -> Option<ResumeHeader> {
        for g in &self.index.blocks {
            if g.kind != BlockKind::Table || !self.table_shows_grid(g.id, &g.range) {
                continue;
            }
            if y <= g.top + 0.5 {
                continue; // 页首在该表顶端：表头本身就在本页正常绘制
            }
            let Some(tl) = self.cache.table(g.id, self.table_key(g.id)) else {
                continue;
            };
            if y >= g.top + tl.height - 0.5 {
                continue; // 已经越过表底
            }
            return tl.rows.first().map(|head| ResumeHeader {
                table: g.id,
                height: head.height,
            });
        }
        None
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
