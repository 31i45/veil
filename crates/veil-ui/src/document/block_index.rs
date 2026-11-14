//! 文档的几何投影：块清单索引（id/kind/range）、估算高度与累计 top 的增量维护。
//! 本文件是生产代码唯一允许全量枚举块的位置；其余读取一律走本索引。
//! 不依赖 cosmic-text/像素概念，属文档域而非排版细节。

use std::collections::{HashMap, HashSet};

use veil_core::{BlockContainer, BlockId, BlockKind, Range, Session};

use crate::app::App;
use crate::geometry::{MARGIN, PARA_GAP};
use crate::tokens::doc;
use crate::view::{list_marker_of, ordered_list_epoch, ordered_list_ordinals, ListMarker};

/// 单块几何缓存：块清单一次枚举后复用，避免每次击键反复全量枚举（P1a）。
pub(crate) struct BlockGeom {
    pub(crate) id: BlockId,
    pub(crate) kind: BlockKind,
    pub(crate) range: Range,
    pub(crate) top: f32,
    pub(crate) height: f32,
    /// 估算高度（P1b）：refresh_index 时按换行数与折行字符数廉价估算，
    /// 视口外块以它参与几何累计，几何重算不再读取 rope。
    pub(crate) est_h: f32,
    /// 容器上下文（仅表格块可能非默认；缩进/引用条/details 卡片判定用）。
    pub(crate) container: BlockContainer,
    /// 所属 details 头块（B3 归属链；块不在 details 内时为 None）。
    pub(crate) details_parent: Option<BlockId>,
    /// 因某个祖先 details 折叠而不可见（高度 0、不绘制、光标跳过）。
    pub(crate) details_hidden: bool,
    /// 段落块源码是否为 `[TOC]` 标记（视图层据此渲染生成目录，源码/编辑态仍显标记）。
    pub(crate) toc_marker: bool,
}

/// 块前间距（B5）：标题块上下对称，统一为 `HEADING_GAP`；其余块为 0。
pub(crate) fn gap_before(kind: &BlockKind) -> f32 {
    match kind {
        BlockKind::AtxHeading(_) | BlockKind::SetextHeading(_) => doc::HEADING_GAP.round(),
        _ => 0.0,
    }
}

/// 块后间距（逻辑像素，未乘 scale）：标题类块与后续块拉开距离，
/// 形成「标题—正文」段距大于正文段距的层级。几何累计与测试均用此单一真源。
/// 单一来源：`tokens::doc::HEADING_GAP` / `PARA_GAP`。
pub(crate) fn gap_after(kind: &BlockKind) -> f32 {
    match kind {
        BlockKind::AtxHeading(_) | BlockKind::SetextHeading(_) => doc::HEADING_GAP.round(),
        _ => PARA_GAP,
    }
}

/// 文档几何索引：`blocks` 与 `total_h` 只在本层刷新/重算后读取。
#[derive(Default)]
pub(crate) struct BlockIndex {
    pub(crate) blocks: Vec<BlockGeom>,
    /// 有序列表显示编号（BlockId → 编号）：随块清单一同重建，布局按块 id 取用。
    /// 编号是全文派生量（`BlockKind` 不含源码起始号），由 `view::layout` 的纯函数派生。
    pub(crate) list_ordinals: HashMap<BlockId, u32>,
    /// 有序列表编号代次：任一编号变化都改变布局基础键，避免文首增删项后
    /// 后续块（文本与 BlockId 未变）命中陈旧编号。
    pub(crate) list_epoch: u64,
    /// 文档总高（含首尾外边距），即原 `doc_height()` 语义。
    pub(crate) total_h: f32,
    /// 上一轮估算高度的输入快照：三者不变时，`(id, kind, range)` 相同的块可原样复用
    /// `est_h`（估算只依赖块内字节长度与块内行数，均随 range 固化），免去每块两次
    /// `line_of_offset`。
    est_scale: f32,
    est_cpl: f32,
    est_rev: u32,
}

impl App {
    /// 重建块清单：生产代码唯一允许全量枚举块的位置；其余读取一律走索引。
    /// 估算高度在此一次性算好（P1b）：几何重算不再读取 rope。
    ///
    /// 四步拆成独立方法（生产按同一顺序调用，无计时钩子），性能探针可逐步计时：
    /// `blocks_enum` → `geom_fill` → `recompute_geometry` → `refresh_stats`。
    pub(crate) fn refresh_index(&mut self) {
        let blocks = self.blocks_enum();
        self.geom_fill(blocks);
        // expanded_details 同步：移除已消失的 id + 尊重 HTML <details open>。
        // 必须早于几何重算——open_default 展开会改变子块可见性/高度。
        self.sync_expanded_details();
        // 索引重建后 top/height 归零：立即按估算高度累计一次，可见区间/滚动条
        // 才有有效几何；排版后用 recompute_geometry 以真实高度覆盖。
        self.recompute_geometry();
        // 全文统计与块结构同源重建：状态栏字数不可能落后于真值
        self.refresh_stats();
    }

    /// Details 块展开集合维护：每次重建块索引后清理已消失 id、补入 open_default=true 的块；
    /// 用户手动折叠/展开的条目只在对应块消失时被清理。
    fn sync_expanded_details(&mut self) {
        let mut alive_details = std::collections::HashSet::new();
        for g in &self.index.blocks {
            if let BlockKind::Details { open_default, .. } = &g.kind {
                alive_details.insert(g.id);
                if *open_default {
                    self.ui_state.expanded_details.insert(g.id);
                }
            }
        }
        // 清理已消失的 Details 块 id
        self.ui_state
            .expanded_details
            .retain(|id| alive_details.contains(id));
        // 行内 details 折叠态同样按块存活清理（编辑/删除后自然复位）。
        let alive_all: std::collections::HashSet<BlockId> =
            self.index.blocks.iter().map(|g| g.id).collect();
        self.ui_state
            .expanded_inline_details
            .retain(|(id, _)| alive_all.contains(id));
    }

    /// 推导 details 归属链与可见性（B3）：块序列的 `details_depth` 递增/回退
    /// 恰好对应嵌套结构，用栈一次线性扫描。任一祖先头块折叠 → 该块 `details_hidden`。
    /// 打印态恒展开（跳过折叠判定），与既有「打印不折叠」约定一致。
    pub(crate) fn refresh_details_visibility(&mut self) {
        let print = self.window_state.print.is_some();
        let expanded = &self.ui_state.expanded_details;
        let mut stack: Vec<(u8, BlockId, bool)> = Vec::new();
        for g in &mut self.index.blocks {
            let depth = g.container.details_depth;
            while stack.last().is_some_and(|&(d, _, _)| d >= depth) {
                stack.pop();
            }
            g.details_parent = stack.last().map(|&(_, id, _)| id);
            g.details_hidden = depth > 0 && !print && stack.last().is_some_and(|&(_, _, vis)| !vis);
            if matches!(g.kind, BlockKind::Details { .. }) {
                let visible = !g.details_hidden && (print || expanded.contains(&g.id));
                stack.push((depth, g.id, visible));
            }
        }
    }

    /// 切换 Details 块折叠/展开（纯 UI 状态，不写文档字节）。
    /// 折叠态改变会影响 Details 块的可视高度（单行 summary vs 全高），
    /// 所以切换后立即重算几何 + 请求重绘（invalidate 由 render 链路自动处理）。
    pub(crate) fn toggle_details(&mut self, id: BlockId) {
        let was_expanded = self.ui_state.expanded_details.remove(&id);
        if !was_expanded {
            self.ui_state.expanded_details.insert(id);
        } else {
            // 收起：光标若在子块内（即将隐藏），移到 summary 头块起点。
            let (start, end) = self.details_group_bounds(id);
            let caret = self.editor.caret();
            if start < caret && caret <= end {
                self.editor.set_caret(start);
                self.window_state.goal_x = None;
            }
        }
        // 高度变了 → 重算几何让后续块顶偏移
        self.recompute_geometry();
        self.request_redraw();
    }

    /// details 组字节区间：头块起点 → 最深后代的终点（B3）。
    pub(crate) fn details_group_bounds(&self, id: BlockId) -> (usize, usize) {
        let Some(i) = self.index.blocks.iter().position(|b| b.id == id) else {
            return (0, 0);
        };
        let g = &self.index.blocks[i];
        let mut end = g.range.end;
        for next in &self.index.blocks[i + 1..] {
            if next.container.details_depth <= g.container.details_depth {
                break;
            }
            end = end.max(next.range.end);
        }
        (g.range.start, end)
    }

    /// 步骤 1：全量枚举块（含 `session.blocks()` 内部的 line→byte 换算）。
    pub(crate) fn blocks_enum(&self) -> Vec<(BlockId, BlockKind, Range, BlockContainer)> {
        let session = self.editor.session();
        session
            .blocks()
            .into_iter()
            .map(|(id, kind, range)| (id, kind, range, session.block_container(id)))
            .collect()
    }

    /// 有序列表编号及其代次：只见列表项取块文本（首行标记的源码起始号），
    /// 其余块零读取；列表不存在的文档代价接近零。
    fn derive_list_ordinals(
        &self,
        blocks: &[(BlockId, BlockKind, Range, BlockContainer)],
    ) -> (HashMap<BlockId, u32>, u64) {
        let session = self.editor.session();
        let items = blocks.iter().map(|(id, kind, range, _)| {
            let marker = match kind {
                BlockKind::ListItem { .. } => session
                    .text_slice(*range)
                    .map_or(ListMarker::None, |t| list_marker_of(kind, &t)),
                _ => ListMarker::None,
            };
            (*id, kind, marker)
        });
        let ordinals = ordered_list_ordinals(items);
        let epoch = ordered_list_epoch(&ordinals);
        (ordinals, epoch)
    }

    /// 步骤 2：填充 `BlockGeom` 向量并回收缓存（除 `blocks_enum` 外的循环体）。
    ///
    /// `est_h` 只依赖块内字节长度与块内行数；同一 `BlockId` 的块内容未变时二者不变
    ///（编辑点后的块只整体平移，字节长度不变）。块数未变时新旧清单逐位对应，
    /// 同 id/种类/字节长度的块直接复用上一轮 `est_h`，免去每块两次 `line_of_offset`；
    /// 块数变化（换行/撤销等）退化为全量估算，语义不变。
    /// 真实高度仍由紧随其后的 `recompute_geometry` 按布局缓存覆盖（与改动前逐块
    /// 探测缓存等价：命中的块在 recompute 中同样命中，未命中的块两处都回退估算）。
    pub(crate) fn geom_fill(&mut self, blocks: Vec<(BlockId, BlockKind, Range, BlockContainer)>) {
        let scale = self.input_state.scale;
        let chars_per_line = self.chars_per_line();
        let line_h = self.engine.doc_line_height();
        // 编号派生必须早于 base_key：代次进基础键，布局缓存才能随编号变化失效。
        let (list_ordinals, list_epoch) = self.derive_list_ordinals(&blocks);
        self.index.list_ordinals = list_ordinals;
        self.index.list_epoch = list_epoch;
        let base = self.base_key();
        let rev = self.input_state.revs.session;
        let can_reuse = self.index.est_scale == scale
            && self.index.est_cpl == chars_per_line
            && self.index.est_rev == rev
            && self.index.blocks.len() == blocks.len();
        let prev = std::mem::take(&mut self.index.blocks);
        let mut alive: HashSet<BlockId> = HashSet::with_capacity(blocks.len());
        let mut next: Vec<BlockGeom> = Vec::with_capacity(blocks.len());
        for (i, (id, kind, range, container)) in blocks.into_iter().enumerate() {
            alive.insert(id);
            let est_h = match prev.get(i) {
                Some(p)
                    if can_reuse
                        && p.id == id
                        && p.kind == kind
                        && p.range.end - p.range.start == range.end - range.start =>
                {
                    p.est_h
                }
                _ => {
                    // Details 块折叠时估算高度压到单行（summary），节省索引期重算开销。
                    if let BlockKind::Details { .. } = &kind {
                        if !self.ui_state.expanded_details.contains(&id) {
                            (self.engine.doc_line_height() * scale).round()
                        } else {
                            estimate_h(self.editor.session(), range, chars_per_line, scale, line_h)
                        }
                    } else {
                        estimate_h(self.editor.session(), range, chars_per_line, scale, line_h)
                    }
                }
            };
            // 仅段落块可能是标记；限长避免为普通短段落反复读 rope。
            let toc_marker = matches!(kind, BlockKind::Paragraph)
                && range.end - range.start <= 16
                && self
                    .editor
                    .session()
                    .text_slice(range)
                    .is_some_and(|t| veil_core::is_toc_marker(&t));
            next.push(BlockGeom {
                id,
                kind,
                range,
                top: 0.0,
                height: 0.0,
                est_h,
                container,
                details_parent: None,
                details_hidden: false,
                toc_marker,
            });
        }
        self.index.blocks = next;
        self.index.est_scale = scale;
        self.index.est_cpl = chars_per_line;
        self.index.est_rev = rev;
        // 内存回收：与块真值同源，删掉已消失的块与过期代次条目（键校验不受影响）
        let revs = self.input_state.revs;
        self.cache.prune_alive(&alive, &revs, &base);
    }

    /// 按当前布局/公式/表格缓存重算高度与累计 top（不再枚举块）。
    /// 取出 Vec 再循环：`block_height(&self)` 与 `index` 的可变借用无法同时成立。
    pub(crate) fn recompute_geometry(&mut self) {
        let scale = self.input_state.scale;
        let line_h = self.engine.doc_line_height();
        // details 归属/可见性按块序列一次推导：折叠头块的整组后继高度为 0。
        self.refresh_details_visibility();
        let mut blocks = std::mem::take(&mut self.index.blocks);
        let mut y = MARGIN * scale;
        let mut prev: Option<(BlockKind, Range, BlockContainer)> = None;
        for g in &mut blocks {
            g.height = self.block_height(g);
            g.top = y;
            if g.details_hidden {
                // 隐藏子块不占高度，也不参与后续块间距计算（上一个可见块保持不变）。
                continue;
            }
            if let Some((pk, pr, pc)) = &prev {
                let blank = self.blank_lines_in_gap(*pr, g.range);
                y +=
                    gap_between(pk, *pr, *pc, &g.kind, g.range, g.container, blank, line_h) * scale;
                g.top = y;
            }
            y += g.height;
            prev = Some((g.kind.clone(), g.range, g.container));
        }
        // 文末留白（B5）：末行下方至少 `BOTTOM_PAD_LINES` 行可点击空白。
        // 另预留「下一个空段落」自身 1 行 + 尾空行行高：Enter 后在文末产生的新行
        // 光标矩形不会跑出文档高度，滚到底时留白仍 ≥2 行（含空块结尾）。
        self.index.total_h = match blocks.last() {
            Some(last) => {
                let tail_blank = self.blank_lines_in_gap(
                    last.range,
                    Range::new(
                        self.editor.session().text_len(),
                        self.editor.session().text_len(),
                    ),
                );
                let pad = gap_after(&last.kind)
                    + (tail_blank as f32 + doc::BLANK_LINE_LINES + doc::BOTTOM_PAD_LINES) * line_h;
                y + pad * scale
            }
            None => 2.0 * MARGIN * scale,
        };
        self.index.blocks = blocks;
    }

    /// 两块之间的空行数（块间空隙里的 `\n` 个数）。块区间含行尾换行，故
    /// 「正文\n\n正文」的间隙恰为 1 个 `\n` = 1 个空行；区间相邻时为 0。
    pub(crate) fn blank_lines_in_gap(&self, prev: Range, next: Range) -> u32 {
        if prev.end >= next.start {
            return 0;
        }
        let Some(text) = self
            .editor
            .session()
            .text_slice(Range::new(prev.end, next.start))
        else {
            return 0;
        };
        text.bytes().filter(|&b| b == b'\n').count() as u32
    }
}

/// 相邻块间距：容器切段（引用内表格、details 子表格）之间不留段落间距，
/// 其余沿用 `gap_after`（按当前块种类）。判据为字节区间相邻（无空行）。
/// `blank_lines`：块间空行数（B5）——每个空行额外占 1 个正文行高，保证
/// 「Enter 产生的空段落」不被压扁；`line_h` 为未乘 scale 的正文行高。
#[allow(clippy::too_many_arguments)] // 几何规则参数天然多（前后块属性 + 空行/行高标尺）
pub(crate) fn gap_between(
    prev_kind: &BlockKind,
    prev_range: Range,
    prev_container: BlockContainer,
    cur_kind: &BlockKind,
    cur_range: Range,
    cur_container: BlockContainer,
    blank_lines: u32,
    line_h: f32,
) -> f32 {
    let base = if prev_range.end == cur_range.start {
        let quote_left =
            matches!(prev_kind, BlockKind::BlockQuote) || prev_container.quote_depth > 0;
        let quote_right =
            matches!(cur_kind, BlockKind::BlockQuote) || cur_container.quote_depth > 0;
        if quote_left && quote_right {
            0.0 // 同一引用的切段：竖条与文本连续，不留段距
        } else if cur_container.details {
            // 头块 → 首子块：按普通块后段距（与顶层块节奏一致，summary 与正文不贴死）。
            if matches!(prev_kind, BlockKind::Details { .. })
                && prev_container.details_depth < cur_container.details_depth
            {
                gap_after(prev_kind)
            } else {
                // 子块 → 子块：卡片内连续，不留段距。
                // 组尾 → 组外块仍走下方 gap_after，保持普通间距。
                0.0
            }
        } else {
            gap_after(prev_kind)
        }
    } else {
        gap_after(prev_kind)
    };
    // B5：标题前间距与后间距统一为 HEADING_GAP，避免「上 10 / 下 16」的不对称。
    let base = base.max(gap_before(cur_kind));
    base + if blank_lines > 0 {
        blank_lines as f32 * doc::BLANK_LINE_LINES * line_h
    } else {
        0.0
    }
}

/// 廉价估算高度：块内行数 × 折行数取大（不切片、不建 String）。
/// 仅依赖 `range`（块内行数与字节长度）、字符宽度/缩放与正文字号基准行高。
fn estimate_h(
    session: &Session,
    range: Range,
    chars_per_line: f32,
    scale: f32,
    line_h: f32,
) -> f32 {
    let start_line = session.line_of_offset(range.start);
    let end_line = session.line_of_offset(range.end);
    let hard_lines = (end_line - start_line) as f32 + 1.0;
    let byte_len = (range.end - range.start) as f32;
    let wrapped = (byte_len / chars_per_line.max(1.0)).ceil().max(1.0);
    hard_lines.max(wrapped) * line_h * scale
}
