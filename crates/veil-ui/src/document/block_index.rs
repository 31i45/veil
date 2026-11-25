//! 文档的几何投影：块清单索引（id/kind/range）、估算高度与累计 top 的增量维护。
//! 本文件是生产代码唯一允许全量枚举块的位置；其余读取一律走本索引。
//! 不依赖 cosmic-text/像素概念，属文档域而非排版细节。

use std::collections::{HashMap, HashSet};

use veil_core::{BlockContainer, BlockId, BlockKind, InlineKind, Range, Session};

use crate::app::App;
use crate::geometry::PARA_GAP;
use crate::stats::count_words;
use crate::tokens::doc;
use crate::view::{
    list_content_start, list_marker_of, ordered_list_epoch, ordered_list_ordinals, toc_list_epoch,
    ListMarker,
};

/// 单块几何缓存：块清单一次枚举后复用，避免每次击键反复全量枚举（P1a）。
pub(crate) struct BlockGeom {
    pub(crate) id: BlockId,
    pub(crate) kind: BlockKind,
    pub(crate) range: Range,
    /// 块首/块尾行号（语法层快照）：块间空行数 = 下一块首行 - 上一块尾行，
    /// 几何累计不再为每对相邻块切片文本数 `\n`。
    pub(crate) start_line: usize,
    pub(crate) end_line: usize,
    pub(crate) top: f32,
    pub(crate) height: f32,
    /// 估算高度（P1b）：refresh_index 时按换行数与折行字符数廉价估算，
    /// 视口外块以它参与几何累计，几何重算不再读取 rope。
    pub(crate) est_h: f32,
    /// 块内中文字数（与 `stats::count_words` 同口径）：内容未变的块直接复用，
    /// 状态栏统计不再每次击键重扫全文。
    pub(crate) words: usize,
    /// 容器上下文（仅表格块可能非默认；缩进/引用条/details 卡片判定用）。
    pub(crate) container: BlockContainer,
    /// 所属 details 头块（B3 归属链；块不在 details 内时为 None）。
    pub(crate) details_parent: Option<BlockId>,
    /// 因某个祖先 details 折叠而不可见（高度 0、不绘制、光标跳过）。
    pub(crate) details_hidden: bool,
    /// 段落块源码是否为 `[TOC]` 标记（视图层据此渲染生成目录，源码/编辑态仍显标记）。
    pub(crate) toc_marker: bool,
    /// 旧版插入的目录列表项（整块恰为指向标题锚点的链接列表）：渲染时不叠加列表序号。
    pub(crate) toc_list: bool,
    /// Front Matter（含段落形态 TOML）：索引期一次判定，绘制/几何每帧只读本标志。
    /// 若每帧对全部块切片解析，10500 块文档的击键帧会平白多出数毫秒。
    pub(crate) frontmatter_like: bool,
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
    /// 旧版目录列表项集合（整组列表均为标题锚点链接）：列表标记按空格显示，不编号。
    pub(crate) toc_lists: HashSet<BlockId>,
    /// 有序列表编号代次：任一编号变化都改变布局基础键，避免文首增删项后
    /// 后续块（文本与 BlockId 未变）命中陈旧编号。
    pub(crate) list_epoch: u64,
    /// 全文标题集合指纹（`(BlockId, 级别)` 异或累计）：目录文本是全文派生量。
    /// 新增/删除/改写标题时 TOC 标记块的字节与 BlockId 均可不变，故目录布局键
    /// 必须带上本代次，标题变化后下一帧即重排（DEF-T05-005）。
    pub(crate) toc_epoch: u64,
    /// 文档总高（含首尾外边距），即原 `doc_height()` 语义。
    pub(crate) total_h: f32,
    /// 全文块字数累计：随块索引增量维护，状态栏不必每键重扫/重加全文。
    pub(crate) total_words: usize,
    /// 单块原位编辑的脏块序号：几何重算可只更新该块邻域并平移后续 top。
    pub(crate) incremental_pos: Option<usize>,
    /// 排版/公式/表格缓存自上次几何重算后是否有新增：有则下一个渲染帧必须
    /// 重算几何（块高可能从估算变为实测）。无新增时渲染可跳过 O(块数) 重算。
    pub(crate) geometry_stale: bool,
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

    /// 编辑后的索引刷新：单脏块原位替换走增量路径（不重建整篇几何），
    /// 其余情况（块数变化/多脏块/列表项变动）回退全量重建。
    /// 增量路径把几何标记为待重算，由调用方在可见块排版后统一累计一次。
    pub(crate) fn refresh_index_for_edit(&mut self, dirty: &[BlockId]) {
        if self.try_incremental_index(dirty) {
            // expanded_details 仍按新块存活清理（Details 块可能被编辑替换）
            self.sync_expanded_details();
            self.refresh_stats();
            self.index.geometry_stale = true;
            return;
        }
        self.refresh_index();
    }

    /// 单块、块数不变、且除该块外所有 BlockId 逐位不变的编辑：
    /// 只替换该块并整体平移其后块的区间/行号，避免每次击键克隆全篇 BlockKind。
    fn try_incremental_index(&mut self, dirty: &[BlockId]) -> bool {
        if dirty.len() != 1 {
            return false;
        }
        let scale = self.input_state.scale;
        let chars_per_line = self.chars_per_line();
        if self.index.est_scale != scale || self.index.est_cpl != chars_per_line {
            return false;
        }
        let session = self.editor.session();
        let n = session.block_count();
        if n == 0 || n != self.index.blocks.len() {
            return false;
        }
        let Some(pos) = session.block_position(dirty[0]) else {
            return false;
        };
        if pos >= n {
            return false;
        }
        // Events 契约保证「未列出的块未变（含 id 不变）」；release 直接依赖该契约，
        // debug 构建保留逐位校验作为回归防线（不进入正式测量）。
        #[cfg(debug_assertions)]
        for i in 0..n {
            let Some(id) = session.block_id_at(i) else {
                return false;
            };
            if i == pos {
                if id != dirty[0] {
                    return false;
                }
            } else if self.index.blocks[i].id != id {
                return false;
            }
        }
        let Some((new_id, new_kind, new_range, new_container, new_start, new_end)) =
            session.block_meta(pos)
        else {
            return false;
        };
        let old = &self.index.blocks[pos];
        // 列表编号是全文派生量：只有列表项本身变动才需重算，除此之外保持原表。
        if matches!(&old.kind, BlockKind::ListItem { .. })
            || matches!(&new_kind, BlockKind::ListItem { .. })
        {
            return false;
        }
        // 引用/details 的段距与可见性依赖容器与块序列：这类编辑回退全量重建。
        if matches!(&old.kind, BlockKind::BlockQuote | BlockKind::Details { .. })
            || matches!(&new_kind, BlockKind::BlockQuote | BlockKind::Details { .. })
            || old.container != new_container
        {
            return false;
        }
        // 脚注编号是全文派生量：新块引入 `[^` 时回退，让 build_footnotes 重编号。
        if session
            .text_slice(new_range)
            .is_some_and(|t| t.contains("[^"))
        {
            return false;
        }
        // 光标必须仍在该块内：否则旧光标块的 reveal 高度也可能变化。
        if self.caret_block_index() != Some(pos) {
            return false;
        }
        // 上一帧光标也须落在该块：否则旧光标块的 reveal 布局/高度可能在本次
        // 编辑中变化，需要全量重算覆盖。
        if let Some(prev_caret) = self.window_state.last_painted_caret {
            if self.block_index_at_offset(prev_caret) != Some(pos) {
                return false;
            }
        }
        // 后缀块的平移量由「脏块尾」决定（编辑可能同时移动块首/块尾）。
        let byte_delta = new_range.end as isize - old.range.end as isize;
        let line_delta = new_end as isize - old.end_line as isize;
        let (old_top, old_height, old_words) = (old.top, old.height, old.words);
        let old_toc = toc_entry_epoch(old.id, &old.kind);
        let new_toc = toc_entry_epoch(new_id, &new_kind);
        let open_default_details = matches!(
            &new_kind,
            BlockKind::Details {
                open_default: true,
                ..
            }
        );
        let line_h = self.engine.doc_line_height();
        let est_h = if let BlockKind::Details { .. } = &new_kind {
            if !self.ui_state.expanded_details.contains(&new_id) {
                (line_h * scale).round()
            } else {
                estimate_h(session, new_range, chars_per_line, scale, line_h)
            }
        } else {
            estimate_h(session, new_range, chars_per_line, scale, line_h)
        };
        let words = session.text_slice(new_range).map_or(0, |t| count_words(&t));
        let toc_marker = matches!(new_kind, BlockKind::Paragraph)
            && new_range.end - new_range.start <= 16
            && session
                .text_slice(new_range)
                .is_some_and(|t| veil_core::is_toc_marker(&t));
        let frontmatter_like = frontmatter_like_of(session, &new_kind, new_range);
        for b in self.index.blocks[pos + 1..].iter_mut() {
            b.range = Range::new(
                (b.range.start as isize + byte_delta) as usize,
                (b.range.end as isize + byte_delta) as usize,
            );
            b.start_line = (b.start_line as isize + line_delta) as usize;
            b.end_line = (b.end_line as isize + line_delta) as usize;
        }
        self.index.blocks[pos] = BlockGeom {
            id: new_id,
            kind: new_kind,
            range: new_range,
            start_line: new_start,
            end_line: new_end,
            top: old_top,
            height: old_height,
            est_h,
            words,
            container: new_container,
            details_parent: None,
            details_hidden: false,
            toc_marker,
            // 增量路径只处理非列表块（列表项变动回退全量重建），目录列表标记恒 false。
            toc_list: false,
            frontmatter_like,
        };
        if open_default_details {
            self.ui_state.expanded_details.insert(new_id);
        }
        self.index.total_words = self.index.total_words.saturating_sub(old_words) + words;
        // 标题集合指纹：摘除旧块贡献、并入新块贡献（异或累计，O(1) 更新）。
        self.index.toc_epoch ^= old_toc ^ new_toc;
        self.index.incremental_pos = Some(pos);
        self.index.est_rev = self.input_state.revs.session;
        // 旧脏块 id 的布局/光栅条目已无块引用，借判活清理（缓存条目为视口量级）。
        let revs = self.input_state.revs;
        let base = self.base_key();
        let session = self.editor.session();
        self.cache
            .prune_alive(|id| session.contains_block(id), &revs, &base);
        true
    }

    /// TOC 目录布局键的标题集合代次：仅 `[TOC]` 标记块取当前指纹，其余块恒 0。
    /// 只让目录块重排，编辑标题时不牵连视口内其他块。
    pub(crate) fn toc_layout_epoch(&self, g: &BlockGeom) -> u64 {
        if g.toc_marker {
            self.index.toc_epoch
        } else {
            0
        }
    }

    /// Details 块展开集合维护：只清理已消失 id（集合远小于块数）；
    /// `open_default` 的新块在索引重建/增量替换时直接插入，避免每键全篇扫描。
    /// 行内 details 折叠态同样按块存活清理（编辑/删除后自然复位）。
    fn sync_expanded_details(&mut self) {
        let session = self.editor.session();
        self.ui_state
            .expanded_details
            .retain(|id| session.contains_block(*id));
        self.ui_state
            .expanded_inline_details
            .retain(|(id, _)| session.contains_block(*id));
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

    /// 步骤 1：全量枚举块（一次快照同时带出容器上下文与块行号）。
    #[allow(clippy::type_complexity)]
    pub(crate) fn blocks_enum(
        &self,
    ) -> Vec<(BlockId, BlockKind, Range, BlockContainer, usize, usize)> {
        self.editor.session().block_entries()
    }

    /// 有序列表编号及其代次：只见列表项取块文本（首行标记的源码起始号），
    /// 其余块零读取；列表不存在的文档代价接近零。
    fn derive_list_ordinals(
        &self,
        blocks: &[(BlockId, BlockKind, Range, BlockContainer, usize, usize)],
    ) -> (HashMap<BlockId, u32>, u64) {
        let session = self.editor.session();
        let items = blocks.iter().map(|(id, kind, range, _, _, _)| {
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

    /// 旧版目录列表识别：连续列表项块中，每一项「标记之后恰为一个同文档锚点链接」。
    ///
    /// 只按块内结构判定（不要求锚点对应标题存在）：普通外部链接列表、含普通文本的
    /// 列表都不会命中；标题编辑也不会让判定结果变化，避免全文布局失效。
    fn derive_toc_lists(
        &self,
        blocks: &[(BlockId, BlockKind, Range, BlockContainer, usize, usize)],
    ) -> HashSet<BlockId> {
        let session = self.editor.session();
        let mut out = HashSet::new();
        let mut group: Vec<BlockId> = Vec::new();
        let mut group_ok = true;
        for (id, kind, range, _, _, _) in blocks {
            if !matches!(kind, BlockKind::ListItem { .. }) {
                flush_toc_group(&mut group, group_ok, &mut out);
                group_ok = true;
                continue;
            }
            let body = session.text_slice(*range).unwrap_or_default();
            if list_marker_of(kind, &body) == ListMarker::None {
                // 项内续块：该项含额外内容，整组不成立（继续收集到组结束）。
                group_ok = false;
                continue;
            }
            group.push(*id);
            group_ok &= is_legacy_toc_entry(session, *id, kind, *range, &body);
        }
        flush_toc_group(&mut group, group_ok, &mut out);
        out
    }

    /// 步骤 2：填充 `BlockGeom` 向量并回收缓存（除 `blocks_enum` 外的循环体）。
    ///
    /// `est_h` 只依赖块内字节长度与块内行数；同一 `BlockId` 的块内容未变时二者不变
    ///（编辑点后的块只整体平移，字节长度不变）。块数未变时新旧清单逐位对应，
    /// 同 id/种类/字节长度的块直接复用上一轮 `est_h`，免去每块两次 `line_of_offset`；
    /// 块数变化（换行/撤销等）退化为全量估算，语义不变。
    /// 真实高度仍由紧随其后的 `recompute_geometry` 按布局缓存覆盖（与改动前逐块
    /// 探测缓存等价：命中的块在 recompute 中同样命中，未命中的块两处都回退估算）。
    #[allow(clippy::type_complexity)]
    pub(crate) fn geom_fill(
        &mut self,
        blocks: Vec<(BlockId, BlockKind, Range, BlockContainer, usize, usize)>,
    ) {
        let scale = self.input_state.scale;
        let chars_per_line = self.chars_per_line();
        let line_h = self.engine.doc_line_height();
        let can_reuse = self.index.est_scale == scale
            && self.index.est_cpl == chars_per_line
            && self.index.est_rev == self.input_state.revs.session
            && self.index.blocks.len() == blocks.len();
        // 有序列表编号只在列表项内容/块序列变化时才重新派生：普通段落击键时
        // 列表项 id 与种类逐位不变，编号表可直接沿用，免去数千次块文本切片。
        let list_unchanged = can_reuse
            && blocks
                .iter()
                .enumerate()
                .all(|(i, (id, kind, _, _, _, _))| {
                    !matches!(kind, BlockKind::ListItem { .. })
                        || self
                            .index
                            .blocks
                            .get(i)
                            .is_some_and(|p| p.id == *id && p.kind == *kind)
                });
        if !list_unchanged {
            // 编号派生必须早于 base_key：代次进基础键，布局缓存才能随编号变化失效。
            let (list_ordinals, list_epoch) = self.derive_list_ordinals(&blocks);
            self.index.list_ordinals = list_ordinals;
            self.index.toc_lists = self.derive_toc_lists(&blocks);
            // 目录列表集合同样影响标记显隐：并入列表代次，邻居变更后不得命中旧布局。
            self.index.list_epoch = list_epoch ^ toc_list_epoch(&self.index.toc_lists);
        }
        let base = self.base_key();
        let rev = self.input_state.revs.session;
        let prev = std::mem::take(&mut self.index.blocks);
        let mut next: Vec<BlockGeom> = Vec::with_capacity(blocks.len());
        let mut total_words = 0usize;
        let mut toc_epoch = 0u64;
        for (i, (id, kind, range, container, start_line, end_line)) in
            blocks.into_iter().enumerate()
        {
            let prior = prev.get(i);
            let same_block = prior.is_some_and(|p| {
                can_reuse
                    && p.id == id
                    && p.kind == kind
                    && p.range.end - p.range.start == range.end - range.start
            });
            // 新 Details 块若默认展开，直接补入展开集合（不再全篇重扫）。
            let open_default_details = matches!(
                &kind,
                BlockKind::Details {
                    open_default: true,
                    ..
                }
            );
            let est_h = if same_block {
                prior.expect("same_block 已判定存在").est_h
            } else if let BlockKind::Details { .. } = &kind {
                // Details 块折叠时估算高度压到单行（summary），节省索引期重算开销。
                if !self.ui_state.expanded_details.contains(&id) {
                    (self.engine.doc_line_height() * scale).round()
                } else {
                    estimate_h(self.editor.session(), range, chars_per_line, scale, line_h)
                }
            } else {
                estimate_h(self.editor.session(), range, chars_per_line, scale, line_h)
            };
            // 字数与块内容同源：id/种类/字节长度均未变的块内容必然未变（脏块会换新 id）。
            let words = if same_block {
                prior.expect("same_block 已判定存在").words
            } else {
                self.editor
                    .session()
                    .text_slice(range)
                    .map_or(0, |t| count_words(&t))
            };
            // 仅段落块可能是标记；限长避免为普通短段落反复读 rope。
            let toc_marker = matches!(kind, BlockKind::Paragraph)
                && range.end - range.start <= 16
                && self
                    .editor
                    .session()
                    .text_slice(range)
                    .is_some_and(|t| veil_core::is_toc_marker(&t));
            let toc_list = self.index.toc_lists.contains(&id);
            let frontmatter_like = frontmatter_like_of(self.editor.session(), &kind, range);
            toc_epoch ^= toc_entry_epoch(id, &kind);
            next.push(BlockGeom {
                id,
                kind,
                range,
                start_line,
                end_line,
                top: 0.0,
                height: 0.0,
                est_h,
                words,
                container,
                details_parent: None,
                details_hidden: false,
                toc_marker,
                toc_list,
                frontmatter_like,
            });
            total_words += words;
            if open_default_details {
                self.ui_state.expanded_details.insert(id);
            }
        }
        self.index.blocks = next;
        self.index.total_words = total_words;
        self.index.toc_epoch = toc_epoch;
        self.index.incremental_pos = None;
        self.index.est_scale = scale;
        self.index.est_cpl = chars_per_line;
        self.index.est_rev = rev;
        // 内存回收：与块真值同源，删掉已消失的块与过期代次条目（键校验不受影响）。
        // 判活直接查 Session 的 id 索引（O(1)），不再为每次击键构建整篇 id 集合。
        let revs = self.input_state.revs;
        let session = self.editor.session();
        self.cache
            .prune_alive(|id| session.contains_block(id), &revs, &base);
    }

    /// 按当前布局/公式/表格缓存重算高度与累计 top（不再枚举块）。
    /// 取出 Vec 再循环：`block_height(&self)` 与 `index` 的可变借用无法同时成立。
    pub(crate) fn recompute_geometry(&mut self) {
        let scale = self.input_state.scale;
        let line_h = self.engine.doc_line_height();
        // details 归属/可见性按块序列一次推导：折叠头块的整组后继高度为 0。
        self.refresh_details_visibility();
        // 全文派生键与光标在循环外取一次：块高只依赖它们与块自身缓存键。
        let base = self.base_key();
        let caret = self.effective_caret();
        let mut blocks = std::mem::take(&mut self.index.blocks);
        // 版心边距是运行时配置（档位/自定义值），不再直引常量。
        let margin = self.engine.doc_margin_px() * scale;
        let mut y = margin;
        // 前块只保留间距判定所需的轻量信息（不克隆 BlockKind，也不重扫块间文本）。
        #[allow(clippy::type_complexity)]
        let mut prev: Option<(f32, bool, bool, Range, BlockContainer, usize, bool)> = None;
        let mut hidden_since_prev = false;
        for g in &mut blocks {
            g.height = self.block_height_with(g, base, caret);
            g.top = y;
            if g.details_hidden {
                // 隐藏子块不占高度，也不参与后续块间距计算（上一个可见块保持不变）。
                hidden_since_prev = true;
                continue;
            }
            // REN-SWEEP-001：隐藏的 Front Matter 高 0，且不留前后段距。
            let hidden_fm = self.frontmatter_hidden(g, caret);
            if let Some((
                prev_gap,
                prev_quote,
                prev_details,
                pr,
                pc,
                prev_end_line,
                prev_hidden_fm,
            )) = prev
            {
                // 普通块间只含空行，直接以行号差计算。跳过折叠 Details 子块后，
                // 原始行号差还含隐藏正文与闭标签，必须只数当前块之前连续的空行。
                let blank = if hidden_since_prev {
                    self.blank_lines_directly_before(g.start_line)
                } else {
                    g.start_line.saturating_sub(prev_end_line) as u32
                };
                if !prev_hidden_fm {
                    y += gap_between_parts(
                        prev_gap,
                        prev_quote,
                        prev_details,
                        pr,
                        pc,
                        &g.kind,
                        g.range,
                        g.container,
                        blank,
                        line_h,
                    ) * scale;
                }
                g.top = y;
            }
            y += g.height;
            prev = Some((
                gap_after(&g.kind),
                matches!(g.kind, BlockKind::BlockQuote),
                matches!(g.kind, BlockKind::Details { .. }),
                g.range,
                g.container,
                g.end_line,
                hidden_fm,
            ));
            hidden_since_prev = false;
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
            None => 2.0 * margin,
        };
        self.index.blocks = blocks;
        self.index.incremental_pos = None;
        self.index.geometry_stale = false;
    }

    /// 编辑路径的几何刷新：单块原位编辑只更新脏块邻域（高度可能变）并平移后缀 top；
    /// 其余情况（多脏块/结构变化/列表/details）走全量重算。
    pub(crate) fn recompute_geometry_after_edit(&mut self) {
        if let Some(pos) = self.index.incremental_pos {
            if pos < self.index.blocks.len() {
                self.recompute_geometry_incremental(pos);
                return;
            }
        }
        self.recompute_geometry();
    }

    fn recompute_geometry_incremental(&mut self, pos: usize) {
        let n = self.index.blocks.len();
        let start = pos.saturating_sub(1);
        let end = (pos + 2).min(n);
        // details 的隐藏/折叠会改变「是否占高度」与后续块归属：退全量。
        // Front Matter 的隐藏/显示会改变块高与前后段距（REN-SWEEP-001）：同样退全量。
        if self.index.blocks[start..end].iter().any(|g| {
            g.details_hidden
                || matches!(
                    g.kind,
                    BlockKind::Details { .. } | BlockKind::FrontMatter(_)
                )
                || g.container.details_depth > 0
        }) || self.index.blocks[start..end]
            .iter()
            .any(|g| self.block_is_frontmatter(g))
        {
            self.recompute_geometry();
            return;
        }
        let scale = self.input_state.scale;
        let line_h = self.engine.doc_line_height();
        let base = self.base_key();
        let caret = self.effective_caret();
        let mut y = self.index.blocks[start].top;
        for i in start..end {
            let h = {
                let g = &self.index.blocks[i];
                self.block_height_with(g, base, caret)
            };
            self.index.blocks[i].height = h;
            if i > start {
                let p = i - 1;
                let blank = self.index.blocks[i]
                    .start_line
                    .saturating_sub(self.index.blocks[p].end_line)
                    as u32;
                let gap = gap_between(
                    &self.index.blocks[p].kind,
                    self.index.blocks[p].range,
                    self.index.blocks[p].container,
                    &self.index.blocks[i].kind,
                    self.index.blocks[i].range,
                    self.index.blocks[i].container,
                    blank,
                    line_h,
                ) * scale;
                y += gap;
            }
            self.index.blocks[i].top = y;
            y += h;
        }
        if end < n {
            // 邻域末块与首块之外的后缀之间还有一段间距：平移量必须包含它。
            let p = end - 1;
            let blank = self.index.blocks[end]
                .start_line
                .saturating_sub(self.index.blocks[p].end_line) as u32;
            y += gap_between(
                &self.index.blocks[p].kind,
                self.index.blocks[p].range,
                self.index.blocks[p].container,
                &self.index.blocks[end].kind,
                self.index.blocks[end].range,
                self.index.blocks[end].container,
                blank,
                line_h,
            ) * scale;
            let delta = y - self.index.blocks[end].top;
            if delta != 0.0 {
                for g in &mut self.index.blocks[end..] {
                    g.top += delta;
                }
            }
        }
        let text_len = self.editor.session().text_len();
        if let Some(last) = self.index.blocks.last() {
            let tail_blank = self.blank_lines_in_gap(last.range, Range::new(text_len, text_len));
            let pad = gap_after(&last.kind)
                + (tail_blank as f32 + doc::BLANK_LINE_LINES + doc::BOTTOM_PAD_LINES) * line_h;
            self.index.total_h = last.top + last.height + pad * scale;
        }
        self.index.incremental_pos = None;
        self.index.geometry_stale = false;
    }

    /// 当前块之前连续空白行的数量。折叠 Details 后跨过的源码含正文与闭标签，
    /// 不能用首尾块行号差；逐行回扫会在最近的非空行（通常是 `</details>`）停止，
    /// 因而只保留组外真实间距，且扫描量与组内正文长度无关。
    fn blank_lines_directly_before(&self, line: usize) -> u32 {
        let session = self.editor.session();
        let mut count = 0;
        for previous in (0..line).rev() {
            let range = Range::new(
                session.offset_of_line(previous),
                session.offset_of_line(previous + 1),
            );
            if session
                .text_slice(range)
                .is_some_and(|text| text.trim().is_empty())
            {
                count += 1;
            } else {
                break;
            }
        }
        count
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
    gap_between_parts(
        gap_after(prev_kind),
        matches!(prev_kind, BlockKind::BlockQuote),
        matches!(prev_kind, BlockKind::Details { .. }),
        prev_range,
        prev_container,
        cur_kind,
        cur_range,
        cur_container,
        blank_lines,
        line_h,
    )
}

/// `gap_between` 的内核：前块种类以预先取好的三个判据传入，
/// 供几何累计的 O(块数) 循环复用，免去逐块克隆 `BlockKind`。
#[allow(clippy::too_many_arguments)]
fn gap_between_parts(
    prev_gap_after: f32,
    prev_is_quote: bool,
    prev_is_details: bool,
    prev_range: Range,
    prev_container: BlockContainer,
    cur_kind: &BlockKind,
    cur_range: Range,
    cur_container: BlockContainer,
    blank_lines: u32,
    line_h: f32,
) -> f32 {
    let base = if prev_range.end == cur_range.start {
        let quote_left = prev_is_quote || prev_container.quote_depth > 0;
        let quote_right =
            matches!(cur_kind, BlockKind::BlockQuote) || cur_container.quote_depth > 0;
        if quote_left && quote_right {
            0.0 // 同一引用的切段：竖条与文本连续，不留段距
        } else if cur_container.details {
            // 头块 → 首子块：按普通块后段距（与顶层块节奏一致，summary 与正文不贴死）。
            if prev_is_details && prev_container.details_depth < cur_container.details_depth {
                prev_gap_after
            } else {
                // 子块 → 子块：卡片内连续，不留段距。
                // 组尾 → 组外块仍走下方 gap_after，保持普通间距。
                0.0
            }
        } else {
            prev_gap_after
        }
    } else {
        prev_gap_after
    };
    // B5：标题前间距与后间距统一为 HEADING_GAP，避免「上 10 / 下 16」的不对称。
    let base = base.max(gap_before(cur_kind));
    base + if blank_lines > 0 {
        blank_lines as f32 * doc::BLANK_LINE_LINES * line_h
    } else {
        0.0
    }
}

/// 单个标题块对 TOC 指纹的贡献（非标题为 0）：级别 + BlockId。
/// 编辑标题内容会分配新 BlockId，故文本改写同样改变指纹；标题增删/改级亦变。
fn toc_entry_epoch(id: BlockId, kind: &BlockKind) -> u64 {
    let level = match kind {
        BlockKind::AtxHeading(l) | BlockKind::SetextHeading(l) => u64::from(*l),
        _ => return 0,
    };
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    h = (h ^ id.0).wrapping_mul(0x0100_0000_01b3);
    h = (h ^ level).wrapping_mul(0x0100_0000_01b3);
    h.wrapping_mul(0x9e37_79b9_7f4a_7c15)
}

/// 旧版目录列表项判定：标记之后恰为一个 `#锚点` 链接，链接外只剩空白。
fn is_legacy_toc_entry(
    session: &Session,
    id: BlockId,
    kind: &BlockKind,
    block: Range,
    body: &str,
) -> bool {
    let Some(content_start) = list_content_start(kind, body) else {
        return false;
    };
    let inlines = session.block_inlines(id).unwrap_or_default();
    let mut links = inlines.iter().filter(|n| n.kind == InlineKind::Link);
    let Some(link) = links.next() else {
        return false;
    };
    if links.next().is_some() {
        return false; // 一项多个链接：普通链接列表，不是目录条目
    }
    let ls = link.range.start.saturating_sub(block.start);
    let le = link.range.end.saturating_sub(block.start).min(body.len());
    if ls < content_start || !body[content_start..ls].trim().is_empty() {
        return false; // 标记与链接之间还有可见文本
    }
    if !body[le..].trim().is_empty() {
        return false; // 链接之后还有可见文本
    }
    let Some(url) = link.url else {
        return false;
    };
    session.text_slice(url).is_some_and(|u| {
        let u = u.trim();
        u.starts_with('#') && u.len() > 1
    })
}

/// 目录组收尾：整组均合格才登记，否则丢弃（组内任一普通项即不视为目录）。
fn flush_toc_group(group: &mut Vec<BlockId>, ok: bool, out: &mut HashSet<BlockId>) {
    if ok && !group.is_empty() {
        out.extend(group.drain(..));
    } else {
        group.clear();
    }
}

/// Front Matter 标志（索引期一次判定）：顶层 `FrontMatter` 块直接命中；
/// 段落形态的 TOML Front Matter 按同构语法识别。普通段落先做长度与首字符
/// 预筛，不读文本，保证大文档全量重建不因本判定退化为全文解析。
fn frontmatter_like_of(session: &Session, kind: &BlockKind, range: Range) -> bool {
    match kind {
        BlockKind::FrontMatter(_) => true,
        BlockKind::Paragraph => {
            // 最短合法形态 `+++\n+++` 为 7 字节；不足直接排除。
            if range.end.saturating_sub(range.start) < 7 {
                return false;
            }
            session
                .text_slice(range)
                .is_some_and(|t| t.trim_start().starts_with("+++") && toml_frontmatter_like(&t))
        }
        _ => false,
    }
}

/// 段落形态的 TOML Front Matter（`+++` 包裹的 `key = value` 行）。
/// 与内核 `looks_like_toml_frontmatter` 同口径；仅用于视图层隐藏判定，
/// 不改变块结构，也不把 YAML `---` 误判（那已是块级分隔线/标题下划线）。
fn toml_frontmatter_like(text: &str) -> bool {
    let trimmed = text.trim_end_matches(['\r', '\n']);
    let lines: Vec<&str> = trimmed.split('\n').collect();
    if lines.len() < 3 {
        return false;
    }
    if lines[0].trim() != "+++" || lines[lines.len() - 1].trim() != "+++" {
        return false;
    }
    let body = &lines[1..lines.len() - 1];
    !body.is_empty()
        && body.iter().all(|line| {
            let s = line.trim();
            s.is_empty()
                || s.starts_with('#')
                || s.starts_with('[')
                || s.ends_with(',')
                || s.ends_with(']')
                || s.ends_with('}')
                || s.split_once('=').is_some_and(|(key, _)| {
                    let key = key.trim();
                    !key.is_empty()
                        && key.chars().all(|c| {
                            c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '"' | '\'')
                        })
                })
        })
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
