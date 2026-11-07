//! 打印（阶段 3b）：分页引擎与打印会话。
//! 本层提供机制——整页干净帧（无 chrome/光标/选区，Reveal 全收）+ 行级分页几何；
//! 策略（打印对话框、设备上下文、逐页 blit）由产品层（veil）经 `Print` 端口注入。
//! 本文件零 Win32 依赖，headless 全测（同 Clipboard/FileStore 模式，INSTANCE §6）。

use std::path::{Path, PathBuf};

use crate::app::App;
use crate::theme::NativeWindow;
use crate::view::Frame;
use veil_core::{BlockKind, Range};

/// PDF 导出选项（页边距由产品层读取 `Config` 后自行换算）。
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct ExportOptions {
    pub page_numbers: bool,
    pub toc: bool,
}

/// 打印端口：实现由产品层注入（Win32 GDI；PDF 走系统「Microsoft Print to PDF」）。
/// 一次打印 = `begin` → 逐页 `render_page` → `end`；实现方须保证 begin 之后
/// 必调 end（含出错/中止路径），否则应用停留在打印态（尺寸/缩放未恢复）。
pub trait Print {
    fn print(&mut self, session: &mut PrintSession);

    /// 直接导出 PDF（无打印对话框）。默认实现仅提示，不产生输出。
    fn export_pdf(&mut self, session: &mut PrintSession) {
        session.notice("当前平台暂不支持导出 PDF");
    }
}

/// 无平台实现时的兜底（非 Windows / 产品层未注入）：只提示，不产生任何输出。
pub struct NoPrint;
impl Print for NoPrint {
    fn print(&mut self, session: &mut PrintSession) {
        session.notice("当前平台暂不支持打印");
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
    /// 目录页数（物理页 0..toc_pages 为目录）。
    pub(crate) toc_pages: usize,
    /// 预渲染的目录页帧。
    pub(crate) toc_frames: Vec<Frame>,
    /// 页脚文本光栅，下标 = 物理页（0-based）；非页码模式或目录页为空。
    pub(crate) footers: Vec<(Vec<u8>, u32, u32)>,
    /// 物理总页数（目录页 + 正文页）。
    pub(crate) total_pages: usize,
    /// 本次会话的导出选项（`begin` 为全关）。
    pub(crate) options: ExportOptions,
    /// 目录条目快照（层级、文本、物理页码）。
    pub(crate) toc_entries: Vec<TocEntry>,
}

/// 目录条目：层级（1-based）、渲染态文本、物理页码（1-based）。
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct TocEntry {
    pub(crate) level: u8,
    pub(crate) text: String,
    pub(crate) page: usize,
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

    /// 进入 PDF 导出：在 `begin` 基础上预渲染目录页与页脚，返回物理总页数。
    pub fn begin_export(&mut self, w: u32, h: u32, scale: f32, opt: ExportOptions) -> usize {
        let body_pages = self.begin_inner(w, h, scale);
        let mut toc_pages = 0usize;
        let mut toc_frames = Vec::new();
        let mut entries = Vec::new();
        if opt.toc {
            // 目录页数影响正文页码 → 迭代到稳定（最多 3 次，防不收敛）
            for _ in 0..3 {
                entries = self.collect_toc_entries(toc_pages);
                if entries.is_empty() {
                    toc_frames.clear();
                    break;
                }
                toc_frames = self.render_toc_pages(&entries);
                if toc_frames.len() == toc_pages {
                    break;
                }
                toc_pages = toc_frames.len();
            }
            toc_pages = toc_frames.len();
        }
        let total = toc_pages + body_pages;
        let footers = if opt.page_numbers {
            self.render_footers(toc_pages, total)
        } else {
            Vec::new()
        };
        let state = self.app.window_state.print.as_mut().unwrap();
        state.toc_pages = toc_pages;
        state.toc_frames = toc_frames;
        state.footers = footers;
        state.total_pages = total;
        state.options = opt;
        state.toc_entries = entries;
        total
    }

    fn begin_inner(&mut self, w: u32, h: u32, scale: f32) -> usize {
        let app = &mut *self.app;
        app.window_state.print = Some(PrintState {
            w: w.max(1),
            h: h.max(1),
            saved_scale: app.input_state.scale,
            saved_scroll: app.window_state.scroll_y,
            breaks: Vec::new(),
            toc_pages: 0,
            toc_frames: Vec::new(),
            footers: Vec::new(),
            total_pages: 0,
            options: ExportOptions::default(),
            toc_entries: Vec::new(),
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
        let state = app.window_state.print.as_mut().unwrap();
        state.breaks = breaks;
        state.total_pages = state.breaks.len();
        state.breaks.len()
    }

    /// 渲染第 i 页（0-based，i < begin 返回值）：返回整页帧。
    /// 目录页直接返回预渲染帧；正文页滚动到页首，并在页码模式下叠加页脚。
    pub fn render_page(&mut self, i: usize) -> &Frame {
        let toc_pages = self
            .app
            .window_state
            .print
            .as_ref()
            .map(|p| p.toc_pages)
            .unwrap_or(0);
        if i < toc_pages {
            return &self.app.window_state.print.as_ref().unwrap().toc_frames[i];
        }
        let app = &mut *self.app;
        let body = i.saturating_sub(toc_pages);
        let scroll = app
            .window_state
            .print
            .as_ref()
            .and_then(|p| p.breaks.get(body).copied())
            .unwrap_or(0.0);
        let footer = app
            .window_state
            .print
            .as_ref()
            .and_then(|p| p.footers.get(i).cloned())
            .filter(|(_, w, h)| *w > 0 && *h > 0);
        let pad = (8.0 * app.input_state.scale).round() as i32;
        app.window_state.scroll_y = scroll;
        // 逐页重建光栅：跨页累积会让长文档打印的内存随页数线性增长（布局保留，光栅即弃）
        app.cache.rasters.clear();
        let frame = app.render();
        if let Some((px, w, h)) = footer {
            let x = frame.width as i32 - w as i32 - pad;
            let y = frame.height as i32 - h as i32 - pad;
            frame.blit(&px, w, h, x, y);
        }
        frame
    }

    /// 目录条目快照（测试断言页码映射）。
    #[cfg(test)]
    pub(crate) fn toc_entries(&self) -> &[TocEntry] {
        match &self.app.window_state.print {
            Some(p) => &p.toc_entries,
            None => &[],
        }
    }

    /// 目录条目数（测试用）。
    pub fn toc_entry_count(&self) -> usize {
        match &self.app.window_state.print {
            Some(p) => p.toc_entries.len(),
            None => 0,
        }
    }

    /// 物理总页数（目录页 + 正文页）；非导出会话 = 正文页数。
    pub fn page_count(&self) -> usize {
        match &self.app.window_state.print {
            Some(p) => p.total_pages,
            None => 0,
        }
    }

    /// 当前配置的导出选项（产品层在 begin_export 之前读取）。
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
            None => PathBuf::from("未命名.pdf"),
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

    /// 排版并光栅化单行文本，返回 (straight-alpha RGBA8, 宽, 高)。
    fn text_raster(&mut self, text: &str, width: f32) -> (Vec<u8>, u32, u32) {
        let fg = self.app.input_state.theme.fg;
        let layout = self.app.engine.layout_block(
            BlockKind::Paragraph,
            text,
            Range::new(0, text.len()),
            &[],
            width,
            usize::MAX,
            &[],
            false,
        );
        self.app.engine.rasterize(&layout, fg)
    }

    /// 标题条目（层级 / 渲染态文本 / 物理页码）。正文页序取 `breaks.partition_point`。
    fn collect_toc_entries(&self, toc_pages: usize) -> Vec<TocEntry> {
        let breaks = self.page_breaks();
        let mut out = Vec::new();
        for g in &self.app.index.blocks {
            let level = match g.kind {
                BlockKind::AtxHeading(l) | BlockKind::SetextHeading(l) => l,
                _ => continue,
            };
            let Some(memo) = self.app.cache.layouts.get(&g.id) else {
                continue;
            };
            let text = memo.value.layout_text().trim().to_string();
            if text.is_empty() {
                continue;
            }
            let page = breaks.partition_point(|&b| b <= g.top) + 1 + toc_pages;
            out.push(TocEntry { level, text, page });
        }
        out
    }

    /// 预渲染目录页：每页首行「目录」标题，条目按层级缩进、页码右对齐，贪心分页。
    fn render_toc_pages(&mut self, entries: &[TocEntry]) -> Vec<Frame> {
        let (frame_w, frame_h) = {
            let Some(p) = self.app.window_state.print.as_ref() else {
                return Vec::new();
            };
            (p.w, p.h)
        };
        let bg = self.app.input_state.theme.bg;
        let scale = self.app.input_state.scale;
        let pad = (12.0 * scale).round() as i32;
        let gap = (6.0 * scale).max(1.0);
        let number_col = (48.0 * scale).max(24.0);
        let w_avail = (frame_w as f32 - 2.0 * pad as f32 - number_col).max(16.0);
        let bottom = frame_h as f32 - pad as f32;
        let mut frames: Vec<Frame> = Vec::new();
        let mut frame = Frame::new(frame_w, frame_h, bg);
        let mut y = self.draw_toc_title(&mut frame, pad, w_avail, gap);
        for e in entries {
            let indent = (e.level.saturating_sub(1) as f32) * 16.0 * scale;
            let (px, pw, ph) = self.text_raster(&e.text, (w_avail - indent).max(16.0));
            let (nx, nw, nh) = self.text_raster(&e.page.to_string(), number_col);
            let row_h = ph.max(nh) as f32;
            // 至少保留标题后的第一行：避免页高过小导致空页死循环
            if y + row_h > bottom && y > pad as f32 {
                frames.push(frame);
                frame = Frame::new(frame_w, frame_h, bg);
                y = self.draw_toc_title(&mut frame, pad, w_avail, gap);
            }
            frame.blit(&px, pw, ph, pad + indent as i32, y as i32);
            frame.blit(&nx, nw, nh, frame_w as i32 - pad - nw as i32, y as i32);
            y += row_h + gap;
        }
        frames.push(frame);
        frames
    }

    /// 目录页标题行（左对齐），返回正文条目起点 y。
    fn draw_toc_title(&mut self, frame: &mut Frame, pad: i32, width: f32, gap: f32) -> f32 {
        let (px, pw, ph) = self.text_raster("目录", width);
        frame.blit(&px, pw, ph, pad, pad);
        pad as f32 + ph as f32 + gap
    }

    /// 预渲染页脚：目录页留空，正文页为 `n / total`（n = 物理页码）。
    fn render_footers(&mut self, toc_pages: usize, total: usize) -> Vec<(Vec<u8>, u32, u32)> {
        let w = self
            .app
            .window_state
            .print
            .as_ref()
            .map(|p| p.w)
            .unwrap_or(1);
        let mut out = Vec::with_capacity(total);
        for i in 0..total {
            if i < toc_pages {
                out.push((Vec::new(), 0, 0));
                continue;
            }
            let (px, pw, ph) = self.text_raster(&format!("{} / {}", i + 1, total), w as f32);
            out.push((px, pw, ph));
        }
        out
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
}

impl App {
    /// 菜单「打印…」入口（3b）：端口置换规避 app 与 ports.print 的同时借用。
    pub(crate) fn print_document(&mut self) {
        let mut port = std::mem::replace(&mut self.ports.print, Box::new(NoPrint));
        port.print(&mut PrintSession { app: self });
        self.ports.print = port;
    }

    /// 菜单「导出 PDF」入口：与打印同一端口置换，产品层决定直接导出或回退对话框。
    pub(crate) fn export_pdf(&mut self) {
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
            if let Some(l) = self.cache.layout(g.id, &g.kind, g.range, base, caret) {
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
