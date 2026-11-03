//! 打印（阶段 3b）：分页引擎与打印会话。
//! 本层提供机制——整页干净帧（无 chrome/光标/选区，Reveal 全收）+ 行级分页几何；
//! 策略（打印对话框、设备上下文、逐页 blit）由产品层（veil）经 `Print` 端口注入。
//! 本文件零 Win32 依赖，headless 全测（同 Clipboard/FileStore 模式，INSTANCE §6）。

use crate::app::App;
use crate::theme::NativeWindow;
use crate::view::Frame;
use veil_core::BlockKind;

/// 打印端口：实现由产品层注入（Win32 GDI；PDF 走系统「Microsoft Print to PDF」）。
/// 一次打印 = `begin` → 逐页 `render_page` → `end`；实现方须保证 begin 之后
/// 必调 end（含出错/中止路径），否则应用停留在打印态（尺寸/缩放未恢复）。
pub trait Print {
    fn print(&mut self, session: &mut PrintSession);
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
        let state = app.window_state.print.as_mut().unwrap();
        state.breaks = breaks;
        state.breaks.len()
    }

    /// 渲染第 i 页（0-based，i < begin 返回值）：返回整页帧。
    /// 页首贴帧顶（滚动 = breaks[i]），打印机侧页边距由产品层 blit 偏移承担。
    pub fn render_page(&mut self, i: usize) -> &Frame {
        let app = &mut *self.app;
        let scroll = app
            .window_state
            .print
            .as_ref()
            .and_then(|p| p.breaks.get(i).copied())
            .unwrap_or(0.0);
        app.window_state.scroll_y = scroll;
        // 逐页重建光栅：跨页累积会让长文档打印的内存随页数线性增长（布局保留，光栅即弃）
        app.cache.rasters.clear();
        app.render()
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
    /// 菜单「打印 / 导出 PDF…」入口（3b）：端口置换规避 app 与 ports.print 的同时借用。
    pub(crate) fn print_document(&mut self) {
        let mut port = std::mem::replace(&mut self.ports.print, Box::new(NoPrint));
        port.print(&mut PrintSession { app: self });
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
