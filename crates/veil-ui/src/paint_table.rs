use crate::app::App;
use crate::paint::{is_remote_image_url, local_image_path};
use crate::view::table::{CellImgAsset, CellMath};
use crate::view::{cell_math_spans, Memo, TableKey};
use std::collections::HashMap;
use std::path::Path;
use veil_core::{BlockId, BlockKind, HtmlTag, InlineKind, InlineNode, Range};

impl App {
    /// 判定 caret 是否落在指定 Table 的任一单元格内容区间内（不含管道符/分隔行等结构字符）。
    pub(crate) fn caret_in_table_cell(&self, id: BlockId) -> bool {
        let caret = self.effective_caret();
        self.editor
            .session()
            .table(id)
            .map(|t| {
                t.header.iter().any(|c| cell_contains(&c.range, caret))
                    || t.rows
                        .iter()
                        .any(|row| row.iter().any(|c| cell_contains(&c.range, caret)))
            })
            .unwrap_or(false)
    }

    /// 找到 caret 所在的表格与单元格（flat 索引 = header.len() + rows 累积）。
    fn caret_table_and_cell(&self, caret: usize) -> Option<(BlockId, veil_core::Table)> {
        let blocks = &self.index.blocks;
        // 从后往前找最近的包含 caret 的 Table block（先到先得：上方的 Table 先遍历到）
        for g in blocks.iter().rev() {
            if g.kind != BlockKind::Table {
                continue;
            }
            if !(g.range.start <= caret && caret <= g.range.end) {
                continue;
            }
            let tbl = self.editor.session().table(g.id)?;
            return Some((g.id, tbl));
        }
        None
    }

    /// Tab 前进一格：caret 跳到下一个单元格的内容起点。最后一格时回到首格起点。
    /// 返回 true 表示已处理（不应再走 Indent/Dedent）。
    pub(crate) fn tab_next_cell(&mut self) -> bool {
        self.tab_step(1)
    }

    /// Shift+Tab 后退一格：caret 跳到上一个单元格的内容起点。首格时不移动。
    pub(crate) fn tab_prev_cell(&mut self) -> bool {
        self.tab_step(-1)
    }

    fn tab_step(&mut self, dir: i32) -> bool {
        let caret = self.editor.caret();
        let Some((_, tbl)) = self.caret_table_and_cell(caret) else {
            return false;
        };
        let total = tbl.header.len() + tbl.rows.iter().map(|r| r.len()).sum::<usize>();
        if total == 0 {
            return false;
        }
        let cur = active_cell_index(&tbl, caret);
        let Some(cur_idx) = cur else {
            // caret 落在管道符/分隔行等结构字符上：Tab 跳到其后第一个单元格；
            // 其后无格（如分隔行末尾）则按「末格」处理（追加行），Shift+Tab 交给 Indent 反缩进。
            if dir > 0 {
                if let Some(idx) = first_cell_after(&tbl, caret) {
                    self.editor.set_caret(flat_cell_start(&tbl, idx));
                    self.scroll_to_caret();
                    self.request_redraw();
                    return true;
                }
                return self.table_ctrl_enter();
            }
            return false;
        };
        let next = if dir > 0 {
            // 末格 Tab：与 Typora 对齐——追加一行并落首格，而不是绕回表首
            if cur_idx + 1 == total {
                return self.table_ctrl_enter();
            }
            cur_idx + 1
        } else if cur_idx == 0 {
            return true; // 首格 Shift+Tab：不动，吞掉按键
        } else {
            cur_idx - 1
        };
        let target = flat_cell_start(&tbl, next);
        self.editor.set_caret(target);
        self.scroll_to_caret();
        self.request_redraw();
        true
    }

    /// 网格态光标矩形：活动单元格内按源码偏移定位（与点击落点同一字形档位）。
    /// 表格显示为网格时，源码排版坐标与所见网格不一致，直接复用会画出「悬空光标」。
    /// `doc_top` 为块的文档坐标（非视口坐标），滚动量在此统一扣除。
    pub(crate) fn table_caret_xy(
        &self,
        id: BlockId,
        doc_top: f32,
        caret: usize,
    ) -> Option<(f32, f32, f32)> {
        let tbl = self.editor.session().table(id)?;
        let idx = active_cell_index(&tbl, caret)?;
        let grid = self.cache.table(id, self.table_key(id))?;
        let cell = grid.cells.get(idx)?;
        let scale = self.input_state.scale;
        let vp = self.viewport();
        let base_x = self.content_x() + grid.x - grid.pan;
        let base_y = vp.y + doc_top - self.window_state.scroll_y;
        let (dx, dy, h) = match cell.caret_map.as_ref() {
            Some(map) => {
                let rel = caret.saturating_sub(cell.doc_start).min(map.len);
                map.caret_at(rel).unwrap_or((0.0, 0.0, map.line_height()))
            }
            None => (0.0, 0.0, self.engine.doc_line_height() * scale),
        };
        Some((base_x + cell.x + dx, base_y + cell.y + dy, h))
    }

    /// 表格按需网格排版：caret 在结构字符上（管道/分隔行）或表外 → 排版网格；
    /// caret 在单元格内容区间内 → 同样排版网格（活动单元格高亮）；
    /// 仅 caret 在表内但不落在任何单元格时跳过（源码态）。
    /// 网格缓存与文本布局缓存独立：网格仅供渲染/命中，源码态仍走既有块排版。
    pub(crate) fn ensure_tables_range(&mut self, indices: std::ops::Range<usize>) {
        if self.source_mode() {
            return; // 源码模式：表格显源码，不建网格缓存
        }
        let width = self.content_width();
        let caret = self.effective_caret();
        let fg = self.input_state.theme.fg;
        let Some(blocks) = self.index.blocks.get(indices) else {
            return;
        };
        let image_base = self.input_state.revs.image_base;
        let doc_path = self.doc.path.clone();
        let mut pending = Vec::new();
        for g in blocks {
            let (id, kind, range) = (g.id, g.kind.clone(), g.range);
            if kind != BlockKind::Table {
                continue;
            }
            let key = self.table_key_at(id, caret);
            if self.cache.table(id, key).is_some() {
                continue;
            }
            let inside_block = range.start <= caret && caret <= range.end;
            if inside_block && key.active == u32::MAX {
                // caret 落在管道符/分隔行等结构字符上 → 源码态，不排版网格
                continue;
            }
            if let Some(t) = self.editor.session().table(id) {
                // 活动格显示源码（含标记）；离开即恢复富文本。
                let active = (key.active != u32::MAX)
                    .then(|| {
                        let idx = key.active as usize;
                        flat_cell_range(&t, idx)
                            .and_then(|r| self.editor.session().text_slice(r))
                            .map(|src| (idx, src))
                    })
                    .flatten();
                // 含 `[^id]` 的单元格按纯文本降级：core 的 cell.text 已剥掉 `[^`/`]`，
                // 这里取源码原文交给网格（不编号、不着色），避免「引用的 id 混进正文」。
                let raw_cells = footnote_cell_sources(self.editor.session(), &t);
                // 非活动格内联公式：先收可见区间与 TeX，渲染放到下面的循环（&mut self）。
                let seeds = table_cell_math_seeds(self.editor.session(), &t, key.active);
                // 非活动格内联图片：直接从 cell.inlines 提取 URL 并加载。
                let seeds_img = table_cell_img_seeds(self.editor.session(), &t, key.active);
                pending.push((id, key, t, active, raw_cells, seeds, seeds_img, g.container));
            }
        }
        for (id, key, t, active, raw_cells, seeds, seeds_img, container) in pending {
            let maths = self.cell_math_assets(seeds);
            let img_assets = self.cell_img_assets(seeds_img, doc_path.as_deref(), image_base);
            let widths = self.ui_state.tables.widths.get(&id).cloned();
            // 容器内表格按容器缩进排版：版心宽扣除缩进，网格整体右移。
            let indent = self.table_indent(container);
            let width =
                (width - indent).max(crate::view::table::MIN_COL_WIDTH * self.input_state.scale);
            let wide = key.wide;
            let mut l = self.engine.layout_table_opts_wide(
                &t,
                width,
                fg,
                active.as_ref().map(|(idx, src)| (*idx, src.as_str())),
                Some(&raw_cells),
                (!maths.is_empty()).then_some(&maths),
                widths.as_deref(),
                (!img_assets.is_empty()).then_some(&img_assets),
                wide,
            );
            l.x = indent;
            // 可读宽表：平移量夹在 [0, 表宽 - 可用宽]，并把夹取结果写回视图态，
            // 保证绘制 / 命中 / 光标读到同一个 pan。
            if wide {
                let max_pan = (l.width - width).max(0.0);
                let pan = self
                    .ui_state
                    .tables
                    .pan
                    .get(&id)
                    .copied()
                    .unwrap_or(0.0)
                    .clamp(0.0, max_pan);
                l.pan = pan;
                self.ui_state.tables.pan.insert(id, pan);
            }
            self.cache.tables.insert(id, Memo { key, value: l });
            self.index.geometry_stale = true;
        }
    }

    /// Shift+滚轮：把指针下的表格切入「可读宽表」模式并横向平移。
    /// 表格本身不需要平移（内在宽不超版心）时返回 false，交回正文滚动。
    pub(crate) fn try_pan_table(&mut self, dx: f32) -> bool {
        if self.source_mode() || self.window_state.print.is_some() {
            return false;
        }
        let vp = self.viewport();
        let y = (self.input_state.mouse.y as f32 - vp.y + self.window_state.scroll_y).max(0.0);
        let Some(idx) = self
            .index
            .blocks
            .iter()
            .rposition(|g| !self.block_hidden(g) && g.top <= y)
        else {
            return false;
        };
        let (id, kind, top, range) = {
            let g = &self.index.blocks[idx];
            (g.id, g.kind.clone(), g.top, g.range)
        };
        if kind != BlockKind::Table || !self.table_shows_grid(id, &range) {
            return false;
        }
        let was_wide = self.table_is_wide(id);
        if !was_wide {
            // 先进入宽表模式建立网格；若内在宽并不超版心则回退，避免无意义的重排。
            self.ui_state.tables.pan.insert(id, 0.0);
            self.ensure_tables_range(idx..idx + 1);
        }
        let Some(t) = self.cache.tables.get(&id) else {
            if !was_wide {
                self.ui_state.tables.pan.remove(&id);
            }
            return false;
        };
        if y > top + t.height {
            if !was_wide {
                self.ui_state.tables.pan.remove(&id);
            }
            return false;
        }
        let avail = (self.content_width() - t.x).max(1.0);
        let max_pan = (t.width - avail).max(0.0);
        if max_pan <= 0.5 {
            if !was_wide {
                // 不需要平移：撤掉宽表模式，按默认（收纳到版心）重新排版。
                self.ui_state.tables.pan.remove(&id);
                self.ensure_tables_range(idx..idx + 1);
            }
            return false;
        }
        let cur = self.ui_state.tables.pan.get(&id).copied().unwrap_or(0.0);
        let next = (cur + dx).clamp(0.0, max_pan);
        self.ui_state.tables.pan.insert(id, next);
        if let Some(m) = self.cache.tables.get_mut(&id) {
            m.value.pan = next;
        }
        self.request_redraw();
        true
    }

    /// 表格单元格内联公式：渲染（复用 inline_math 缓存）并整理成排版输入。
    /// 渲染失败/已失败记忆的公式跳过 → 该格降级为可见文本（与正文一致）。
    fn cell_math_assets(
        &mut self,
        seeds: HashMap<usize, Vec<CellMathSeed>>,
    ) -> HashMap<usize, Vec<CellMath>> {
        let mut out = HashMap::new();
        for (flat, list) in seeds {
            let mut items = Vec::with_capacity(list.len());
            for seed in list {
                let Some(img) = self.inline_math_image(&seed.tex) else {
                    continue;
                };
                let (w, h) = (img.width, img.height);
                items.push(CellMath {
                    range: seed.range,
                    tex: seed.tex,
                    w,
                    h,
                });
            }
            if !items.is_empty() {
                out.insert(flat, items);
            }
        }
        out
    }

    /// 表格单元格内联图片：本地路径复用 inline_images，远程 URL 复用远程会话缓存。
    /// 全局阻止 / 源码模式 / 已失败记忆 / 草稿相对路径 → 跳过（降级为 alt 文本）。
    fn cell_img_assets(
        &mut self,
        seeds: HashMap<usize, Vec<CellImgSeed>>,
        doc_path: Option<&Path>,
        image_base: u32,
    ) -> HashMap<usize, Vec<CellImgAsset>> {
        let mut out = HashMap::new();
        for (flat, seeds) in seeds {
            let mut items = Vec::with_capacity(seeds.len());
            for CellImgSeed { url, alt } in seeds {
                if let Some(path) = local_image_path(&url, doc_path) {
                    let key = path.to_string_lossy().into_owned();
                    if self.cache.image_failed(&key, image_base) {
                        continue;
                    }
                    if self.cache.image(&key, image_base).is_none() {
                        match self.ports.images.load(&path) {
                            Ok(img) => {
                                self.cache.insert_inline_image(
                                    key.clone(),
                                    Memo {
                                        key: image_base,
                                        value: img,
                                    },
                                );
                            }
                            Err(e) => {
                                eprintln!(
                                    "表格图片加载失败（降级为 alt 文本） {}: {e}",
                                    path.display()
                                );
                                self.cache.image_failed.insert(
                                    key,
                                    Memo {
                                        key: image_base,
                                        value: (),
                                    },
                                );
                                continue;
                            }
                        }
                    }
                    let nat = self.cache.inline_images.get(&key).unwrap();
                    items.push(CellImgAsset {
                        w: nat.value.width,
                        h: nat.value.height,
                        pixels: nat.value.pixels.clone(),
                        alt,
                    });
                } else if is_remote_image_url(&url) {
                    if self.source_mode()
                        || !self.config.allow_remote_images
                        || self.cache.remote_failed.contains(&url)
                    {
                        continue;
                    }
                    let Some(asset) = self.cache.remote_image(&url).map(|nat| CellImgAsset {
                        w: nat.width,
                        h: nat.height,
                        pixels: nat.pixels.clone(),
                        alt,
                    }) else {
                        self.request_remote_image(&url);
                        continue;
                    };
                    items.push(asset);
                }
            }
            if !items.is_empty() {
                out.insert(flat, items);
            }
        }
        out
    }

    /// 表格网格缓存键：基础键 + 活动单元格（flat 索引；None = 全部富文本）。
    pub(crate) fn table_key(&self, id: BlockId) -> TableKey {
        self.table_key_at(id, self.effective_caret())
    }

    fn table_key_at(&self, id: BlockId, caret: usize) -> TableKey {
        let active = self
            .editor
            .session()
            .table(id)
            .and_then(|t| active_cell_index(&t, caret))
            .map_or(u32::MAX, |i| i as u32);
        TableKey {
            base: self.base_key(),
            active,
            widths: self.ui_state.tables.fingerprint(id),
            wide: self.table_is_wide(id),
        }
    }

    /// 该表当前是否处于「可读宽表 + 横向平移」模式（打印态恒 false）。
    pub(crate) fn table_is_wide(&self, id: BlockId) -> bool {
        self.window_state.print.is_none() && self.ui_state.tables.pan.contains_key(&id)
    }

    /// 仅排版视口内（含 overscan）的表格网格。
    pub(crate) fn ensure_tables_visible(&mut self) {
        let range = self.visible_index_range();
        self.ensure_tables_range(range);
    }

    /// 测试专用：全量表格排版。
    #[cfg(test)]
    pub(crate) fn ensure_tables(&mut self) {
        self.ensure_tables_range(0..self.index.blocks.len());
    }

    /// 表格当前是否按网格显示：
    /// - caret 在表外 → 显示网格
    /// - caret 在单元格内容区间内 → 显示网格（活动单元格高亮，源码编辑同路径）
    /// - caret 在表内但落在结构字符上（管道/分隔行）→ 源码态
    pub(crate) fn table_shows_grid(&self, id: BlockId, range: &veil_core::Range) -> bool {
        if self.source_mode() {
            return false; // 源码模式：表格显管道与分隔行原文
        }
        let caret = self.effective_caret();
        let inside_block = range.start <= caret && caret <= range.end;
        if inside_block && !self.caret_in_table_cell(id) {
            return false;
        }
        self.cache.table(id, self.table_key(id)).is_some()
    }
    /// Ctrl+Enter：在 caret 所在表格末尾追加一个空 body 行。返回 true 表示已处理。
    pub(crate) fn table_ctrl_enter(&mut self) -> bool {
        let caret = self.editor.caret();
        let Some((_tid, _tbl)) = self.caret_table_and_cell(caret) else {
            return false;
        };
        let Some(block_range) = self
            .index
            .blocks
            .iter()
            .find(|g| g.kind == BlockKind::Table && g.range.start <= caret && caret <= g.range.end)
            .map(|g| g.range)
        else {
            return false;
        };
        let src = self
            .editor
            .session()
            .text_slice(block_range)
            .unwrap_or_default();
        let new_src = table_append_row(&src);
        if new_src == src {
            return false;
        }
        // 新行在块尾；光标落到新行首个单元格内容处（`|` 后跳过前导空白）。
        // 空单元格（`|  |`）若只按 `+2` 定位会落在管道符上，活动单元格判定失败、Tab 跳格错位。
        let last_line = new_src.trim_end_matches(['\n', '\r']).matches('\n').count();
        let caret_in_block = table_cell_start(&new_src, last_line, 0);
        let Some(dirty_blocks) =
            self.editor
                .replace_table_block(block_range, new_src, caret_in_block)
        else {
            return false;
        };
        self.after_edit(dirty_blocks);
        self.request_redraw();
        true
    }
}

/// 单元格 range 是否包含 caret（含 start/end，与 block range 判定同口径）。
pub(crate) fn cell_contains(r: &Range, caret: usize) -> bool {
    r.start <= caret && caret <= r.end
}

/// 按 flat 索引（header 先，rows 后）取第 idx 个单元格的 range.start。
fn flat_cell_start(tbl: &veil_core::Table, idx: usize) -> usize {
    if let Some(c) = tbl.header.get(idx) {
        return c.range.start;
    }
    let offset = tbl.header.len();
    let mut remaining = idx - offset;
    for row in &tbl.rows {
        if remaining < row.len() {
            return row[remaining].range.start;
        }
        remaining -= row.len();
    }
    0
}

/// caret 所在单元格的 flat 索引（header 先，rows 后）；不在任何单元格时 None。
fn flat_cell_range(tbl: &veil_core::Table, idx: usize) -> Option<Range> {
    if let Some(c) = tbl.header.get(idx) {
        return Some(c.range);
    }
    let mut remaining = idx.checked_sub(tbl.header.len())?;
    for row in &tbl.rows {
        if remaining < row.len() {
            return Some(row[remaining].range);
        }
        remaining -= row.len();
    }
    None
}

/// 含脚注引用的单元格 → 源码原文（flat 索引 → 文本）。
/// 单元格内的 `[^id]` 按纯文本降级（不编号不着色）；core 的可见文本已剥标记，
/// 因此这里回取源码，保持「原文可见」。空表/无引用时返回空表，调用方零成本。
fn footnote_cell_sources(
    session: &veil_core::Session,
    tbl: &veil_core::Table,
) -> HashMap<usize, String> {
    fn has_ref(nodes: &[InlineNode]) -> bool {
        nodes
            .iter()
            .any(|n| n.kind == InlineKind::FootnoteRef || has_ref(&n.children))
    }
    let mut out = HashMap::new();
    let cols = tbl.aligns.len();
    let mut idx = 0usize;
    for cells in std::iter::once(&tbl.header).chain(tbl.rows.iter()) {
        // 与表格排版的 flat 索引口径一致：按列数递增（缺格也占位）。
        for col in 0..cols {
            if let Some(cell) = cells.get(col) {
                if has_ref(&cell.inlines) {
                    if let Some(src) = session.text_slice(cell.range) {
                        out.insert(idx, src);
                    }
                }
            }
            idx += 1;
        }
    }
    out
}

/// 单元格内联公式种子：剥标记后可见文本区间 + TeX 源码（渲染前的纯数据）。
struct CellMathSeed {
    range: Range,
    tex: String,
}

/// 收集非活动单元格的 Math 节点：可见文本坐标（与 `cell.text` 同口径）+ TeX。
/// 活动格显源码，不参与；无 Math 或映射不可用（字节分歧）时该格不入表。
fn table_cell_math_seeds(
    session: &veil_core::Session,
    tbl: &veil_core::Table,
    active: u32,
) -> HashMap<usize, Vec<CellMathSeed>> {
    fn has_math(nodes: &[InlineNode]) -> bool {
        nodes
            .iter()
            .any(|n| n.kind == InlineKind::Math || has_math(&n.children))
    }
    let mut out = HashMap::new();
    let cols = tbl.aligns.len();
    let mut idx = 0usize;
    for cells in std::iter::once(&tbl.header).chain(tbl.rows.iter()) {
        for col in 0..cols {
            if let Some(cell) = cells.get(col) {
                if active as usize != idx && has_math(&cell.inlines) {
                    if let Some(spans) = cell_math_spans(cell.range, cell.text.len(), &cell.inlines)
                    {
                        let seeds: Vec<CellMathSeed> = spans
                            .into_iter()
                            .filter_map(|(s, e, content)| {
                                let tex = session.text_slice(content)?;
                                Some(CellMathSeed {
                                    range: Range::new(s, e),
                                    tex,
                                })
                            })
                            .collect();
                        if !seeds.is_empty() {
                            out.insert(idx, seeds);
                        }
                    }
                }
            }
            idx += 1;
        }
    }
    out
}

/// 非活动单元格图片候选：URL + 该图 alt 在单元格可见文本中的区间。
/// alt 区间供排版层在图片加载成功后剔除重复文本（REN-SWEEP-008）。
pub(crate) struct CellImgSeed {
    url: String,
    pub(crate) alt: Option<Range>,
}

/// 收集非活动单元格的图片 URL（递归扫描 inlines，活动格跳过）。
pub(crate) fn table_cell_img_seeds(
    session: &veil_core::Session,
    tbl: &veil_core::Table,
    active: u32,
) -> HashMap<usize, Vec<CellImgSeed>> {
    fn collect_urls(
        nodes: &[InlineNode],
        out: &mut Vec<CellImgSeed>,
        session: &veil_core::Session,
        alts: Option<&[Range]>,
        img_idx: &mut usize,
    ) {
        for n in nodes {
            match &n.kind {
                InlineKind::Image => {
                    let alt = alts.and_then(|a| a.get(*img_idx)).copied();
                    *img_idx += 1;
                    if let Some(url_range) = n.url {
                        if let Some(url) = session.text_slice(url_range) {
                            out.push(CellImgSeed { url, alt });
                        }
                    }
                }
                // 单元格内 HTML `<img>`：本地 src 与 Markdown Image 同路径；
                // 远程地址在 `remote_src`（src 为空），走同一套远程加载。
                // 标签整段不对应 Markdown alt 的 visible-text 区间，故绝不给删除区间；
                // core 为 HTML img 派生的 alt 文本必须保留（REN-SWEEP-008）。
                InlineKind::HtmlSelfClosed {
                    tag: HtmlTag::Img,
                    attrs,
                } => {
                    if let Some(src) = attrs.src.as_deref().or(attrs.remote_src.as_deref()) {
                        out.push(CellImgSeed {
                            url: src.to_string(),
                            alt: None,
                        });
                    }
                }
                _ => {}
            }
            collect_urls(&n.children, out, session, alts, img_idx);
        }
    }
    let mut out: HashMap<usize, Vec<CellImgSeed>> = HashMap::new();
    let cols = tbl.aligns.len();
    let mut idx = 0usize;
    for cells in std::iter::once(&tbl.header).chain(tbl.rows.iter()) {
        for col in 0..cols {
            if let Some(cell) = cells.get(col) {
                if active as usize != idx {
                    let alts = crate::view::cell_image_alt_spans(
                        cell.range,
                        cell.text.len(),
                        &cell.inlines,
                    );
                    let mut seeds = Vec::new();
                    let mut img_idx = 0usize;
                    collect_urls(
                        &cell.inlines,
                        &mut seeds,
                        session,
                        alts.as_deref(),
                        &mut img_idx,
                    );
                    if !seeds.is_empty() {
                        out.insert(idx, seeds);
                    }
                }
            }
            idx += 1;
        }
    }
    out
}

/// caret 所在单元格的 flat 索引（header 先，rows 后）；不在任何单元格时 None。
pub(crate) fn active_cell_index(tbl: &veil_core::Table, caret: usize) -> Option<usize> {
    let mut idx = 0usize;
    for c in tbl.header.iter() {
        if cell_contains(&c.range, caret) {
            return Some(idx);
        }
        idx += 1;
    }
    for row in tbl.rows.iter() {
        for c in row.iter() {
            if cell_contains(&c.range, caret) {
                return Some(idx);
            }
            idx += 1;
        }
    }
    None
}

/// caret 之后（不含所在格）第一个单元格的 flat 索引：结构字符上的 Tab 落点。
fn first_cell_after(tbl: &veil_core::Table, caret: usize) -> Option<usize> {
    let total = tbl.header.len() + tbl.rows.iter().map(|r| r.len()).sum::<usize>();
    (0..total).find(|&i| flat_cell_start(tbl, i) > caret)
}

// ---- GFM 表格行/列 Markdown 变换（T3：纯字符串，零核心依赖） ----
/// 行 → 单元格文本：与内核解析同一转义口径（`\|`、code span 内的 `|` 不切列）。
/// 写路径不得自行按裸 `|` 切分，否则含转义竖线的单元格会被拆散并改写源码。
pub(crate) fn split_row(row: &str) -> Vec<String> {
    let cells: Vec<String> = veil_core::split_table_row(row)
        .into_iter()
        .map(|(s, e)| row[s..e].trim().to_string())
        .collect();
    if cells.len() == 1 && cells[0].is_empty() {
        return Vec::new();
    }
    cells
}

pub(crate) fn join_row(cells: &[String]) -> String {
    let parts: Vec<String> = cells.iter().map(|s| format!(" {} ", s)).collect();
    format!("|{}|", parts.join("|"))
}

fn sep_cell_for(align: veil_core::CellAlign) -> String {
    match align {
        veil_core::CellAlign::None => "---".into(),
        veil_core::CellAlign::Left => ":--".into(),
        veil_core::CellAlign::Center => ":-:".into(),
        veil_core::CellAlign::Right => "--:".into(),
    }
}

#[allow(dead_code)] // 仅 cfg(test) 使用：断言插入列不改动既有列对齐
pub(crate) fn parse_aligns(sep_line: &str) -> Vec<veil_core::CellAlign> {
    let cells = split_row(sep_line);
    let mut out = Vec::with_capacity(cells.len());
    for c in cells {
        let l = c.starts_with(':');
        let r = c.ends_with(':');
        out.push(match (l, r) {
            (true, true) => veil_core::CellAlign::Center,
            (true, false) => veil_core::CellAlign::Left,
            (false, true) => veil_core::CellAlign::Right,
            (false, false) => veil_core::CellAlign::None,
        });
    }
    out
}

pub(crate) fn table_append_row(src: &str) -> String {
    let lines: Vec<&str> = src.trim_end().split('\n').collect();
    if lines.len() < 3 {
        return src.to_string();
    }
    let ncols = split_row(lines[0]).len();
    let empty = vec!["".to_string(); ncols];
    let empty_row = join_row(&empty);
    let mut new_lines = lines.clone();
    new_lines.push(empty_row.as_str());
    format!("{}\n", new_lines.join("\n"))
}

/// 在 body 行前/后插入一个空行（body_index 0-based；越界按尾部处理）。
/// 列数取自表头，分隔行原样保留，故列数与对齐方式不变。
pub(crate) fn table_insert_row(src: &str, body_index: usize, above: bool) -> String {
    let lines: Vec<&str> = src.trim_end().split('\n').collect();
    if lines.len() < 2 {
        return src.to_string();
    }
    let ncols = split_row(lines[0]).len().max(1);
    let empty = join_row(&vec![String::new(); ncols]);
    let body_count = lines.len().saturating_sub(2);
    let at = if body_index >= body_count {
        lines.len()
    } else if above {
        2 + body_index
    } else {
        3 + body_index
    };
    let mut new_lines: Vec<&str> = Vec::with_capacity(lines.len() + 1);
    new_lines.extend_from_slice(&lines[..at]);
    new_lines.push(&empty);
    new_lines.extend_from_slice(&lines[at..]);
    format!("{}\n", new_lines.join("\n"))
}

/// 生成源码中第 line_index 行（0=header、1=分隔行、2+=body）第 col 列单元格
/// 内容起点（`|` 后跳过前导空白）；行/列越界返回 0。
/// 切列沿用内核转义口径，含 `\|` 与 code span 内 `|` 的单元格不会错位。
pub(crate) fn table_cell_start(src: &str, line_index: usize, col: usize) -> usize {
    let mut line_off = 0usize;
    for (i, line) in src.split('\n').enumerate() {
        if i == line_index {
            let Some(&(s, e)) = veil_core::split_table_row(line).get(col) else {
                return 0;
            };
            let cell = &line[s..e];
            return line_off + s + (cell.len() - cell.trim_start().len());
        }
        line_off += line.len() + 1;
    }
    0
}

/// 删除指定 body 行（body_index 0-based；0 = 第一行 body，不是 header）。
/// 保留表格至少 1 行 body（只有 header + 分隔行时退化为仅 header 行）。
pub(crate) fn table_delete_row(src: &str, body_index: usize) -> String {
    let lines: Vec<&str> = src.trim_end().split('\n').collect();
    if lines.len() < 3 {
        return src.to_string();
    }
    // body 行在 lines[2..]
    if body_index + 2 >= lines.len() {
        return src.to_string();
    }
    if lines.len() <= 3 {
        // 只剩 header + 分隔行 + 1 body → 删除 body，留 header + 分隔行
        let keep: Vec<&str> = lines[..2].to_vec();
        return format!("{}\n", keep.join("\n"));
    }
    let mut new_lines: Vec<&str> = lines.clone();
    new_lines.remove(body_index + 2);
    format!("{}\n", new_lines.join("\n"))
}

/// 在指定列 index 前插入空列（col_index 0-based；包含 header 和 separator）。
pub(crate) fn table_insert_column(src: &str, col_index: usize) -> String {
    let lines: Vec<&str> = src.trim_end().split('\n').collect();
    if lines.len() < 3 {
        return src.to_string();
    }
    let ncols = split_row(lines[0]).len();
    let col_index = col_index.min(ncols);
    let mut new_lines = Vec::with_capacity(lines.len());
    for (i, line) in lines.iter().enumerate() {
        let cells = split_row(line);
        let mut new_cells = cells.clone();
        // 处理列数不足的行（表格结构异常时）
        if new_cells.len() < ncols {
            new_cells.resize(ncols, String::new());
        }
        let insert = if i == 1 {
            // 分隔行：生成 None 对齐
            sep_cell_for(veil_core::CellAlign::None)
        } else {
            String::new()
        };
        new_cells.insert(col_index, insert);
        new_lines.push(join_row(&new_cells));
    }
    format!("{}\n", new_lines.join("\n"))
}

/// 删除指定列（col_index 0-based）。
/// 保留至少 1 列（只有 header 时删除后仍有 1 列）。
pub(crate) fn table_delete_column(src: &str, col_index: usize) -> String {
    let lines: Vec<&str> = src.trim_end().split('\n').collect();
    if lines.len() < 3 {
        return src.to_string();
    }
    let ncols = split_row(lines[0]).len();
    if ncols <= 1 || col_index > ncols {
        return src.to_string();
    }
    let mut new_lines = Vec::with_capacity(lines.len());
    for line in lines.iter() {
        let cells = split_row(line);
        let mut new_cells = cells.clone();
        if new_cells.len() > col_index {
            new_cells.remove(col_index);
        }
        if new_cells.is_empty() {
            new_cells.push(String::new());
        }
        new_lines.push(join_row(&new_cells));
    }
    format!("{}\n", new_lines.join("\n"))
}

/// 分隔行单元格（含两侧空白）的字节区间：与内核同一转义感知切分。
fn sep_cell_spans(line: &str) -> Vec<(usize, usize)> {
    veil_core::split_table_row(line)
}

/// 改写分隔行第 `col` 列的对齐标记（左/中/右/无），其余字节逐字保留，返回新源码。
/// 标记宽度沿用该列原有分隔符宽度（不足 3 补到 3），避免列宽跳动；
/// `col` 越界或第 2 行不是合法分隔格时返回 None。已是目标对齐时输出与输入相等。
pub(crate) fn table_set_align(
    src: &str,
    col: usize,
    align: veil_core::CellAlign,
) -> Option<String> {
    // 第 2 行（分隔行）的字节区间；`\r` 归入行尾不参与改写。
    let mut line_start = 0usize;
    let mut sep = None;
    for (i, line) in src.split_inclusive('\n').enumerate() {
        if i == 1 {
            sep = Some((
                line_start,
                line_start + line.trim_end_matches(['\n', '\r']).len(),
            ));
            break;
        }
        line_start += line.len();
    }
    let (ls, le) = sep?;
    let line = &src[ls..le];
    let (cs, ce) = *sep_cell_spans(line).get(col)?;
    let content = &line[cs..ce];
    let trimmed = content.trim();
    if trimmed.is_empty() || !trimmed.bytes().all(|b| b == b'-' || b == b':') {
        return None;
    }
    let start = cs + (content.len() - content.trim_start().len());
    let width = trimmed.len().max(3);
    let marker = match align {
        veil_core::CellAlign::None => "-".repeat(width),
        veil_core::CellAlign::Left => format!(":{}", "-".repeat(width - 1)),
        veil_core::CellAlign::Center => format!(":{}:", "-".repeat(width - 2)),
        veil_core::CellAlign::Right => format!("{}:", "-".repeat(width - 1)),
    };
    let mut out = String::with_capacity(src.len() + 2);
    out.push_str(&src[..ls + start]);
    out.push_str(&marker);
    out.push_str(&src[ls + start + trimmed.len()..]);
    Some(out)
}
