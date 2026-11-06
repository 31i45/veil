//! 帧合成：背景/块内容/表格网格/选区/光标与 preedit 绘制。所有输入为
//! 已排版的几何与缓存，本文件不触发重新排版（除 paint 开头的两遍收敛）。

use crate::app::App;
use crate::chrome;
use crate::geometry::{BASE_LINE_HEIGHT, MARGIN};
use crate::view::{
    ctx_menu, node_active, outline, oversize_notice, quick_open, search, BlockLayout, Frame,
    InlineAsset, InlineImg, LayoutKey, Memo, TableKey,
};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use veil_core::{BlockId, BlockKind, InlineKind, InlineNode, Range};

/// 专注模式压暗强度：背景色以该 alpha 覆盖非光标块（越大越暗，255 = 全遮）。
const FOCUS_DIM_ALPHA: u8 = 160;

/// 打字机模式锚点：视口高比例（0.42 ≈ 上四成处），光标行中心恒定位于此处。
const TYPEWRITER_ANCHOR: f32 = 0.42;

/// 拖放悬停压暗强度（阶段 1）：内容区背景覆盖，突出居中提示条。
const DROP_DIM_ALPHA: u8 = 120;

/// present 前置条件：帧像素数与 surface 缓冲长度一致才可整块拷贝。
/// 最小化时 Resized 宽/高为 0，surface 不 resize 而帧被钳成 1×1，长度失配即丢帧。
pub(crate) fn can_present(frame_len: usize, target_len: usize) -> bool {
    frame_len == target_len
}

impl App {
    /// 补齐缺失布局（键命中则零开销）。Reveal 激活已并入布局键，光标移动后
    /// 指纹失配即自动重排，无需另设失效调用（ADR-009 决策四）。
    /// 行内公式（切片 8）：块内非激活 Math 节点渲染出图（tex 内容寻址缓存），
    /// 传占位信息给 layout；失败记 inline_failed 降级源码显示，不每帧重试。
    /// 文档内图片（切片 13）：非激活 Image 节点按文档目录解析相对 URL，经 ImageLoader
    /// 加载自然尺寸位图；宽超版心等比缩放后占位；失败/远程 URL 降级 alt 文本。
    pub(crate) fn ensure_layouts_range(&mut self, indices: std::ops::Range<usize>) {
        let width = self.content_width();
        let caret = self.effective_caret();
        let base = self.base_key();
        let reveal_all = self.source_mode();
        // 脚注编号表：每轮构建一次（O(引用数)），块内查询 O(1)，不逐块回查会话。
        let footnotes = self.footnote_ctx();
        self.engine.set_scale(self.input_state.scale);
        let Some(blocks) = self.index.blocks.get(indices) else {
            return;
        };
        // 先收集待排块与其行内公式/图片（session 借用期），再调渲染器/加载器（&mut self）
        let mut pending = Vec::new();
        for g in blocks {
            let (id, kind, range) = (g.id, g.kind.clone(), g.range);
            if self.cache.layout(id, &kind, range, base, caret).is_some() {
                continue;
            }
            let text = self.editor.session().text_slice(range).unwrap_or_default();
            let inlines = self.editor.session().block_inlines(id).unwrap_or_default();
            let mut maths: Vec<(Range, String)> = Vec::new(); // (节点区间, tex)
            let mut images: Vec<(Range, String)> = Vec::new(); // (节点区间, url)
            let mut stack: Vec<&InlineNode> = inlines.iter().collect();
            while let Some(n) = stack.pop() {
                if !node_active(n.range, caret) {
                    match n.kind {
                        InlineKind::Math => {
                            let tex = self
                                .editor
                                .session()
                                .text_slice(n.content)
                                .unwrap_or_default();
                            maths.push((n.range, tex));
                        }
                        InlineKind::Image => {
                            if let Some(url) = n.url {
                                let url = self.editor.session().text_slice(url).unwrap_or_default();
                                images.push((n.range, url));
                            }
                        }
                        _ => {}
                    }
                }
                stack.extend(n.children.iter());
            }
            pending.push((id, kind, range, text, inlines, maths, images));
        }
        let scale = self.input_state.scale;
        let fg = self.input_state.theme.fg;
        let inline_key = self.inline_key();
        let dpi_q = self.dpi_q();
        let image_base = self.input_state.revs.image_base;
        for (id, kind, range, text, inlines, maths, images) in pending {
            let mut imgs = Vec::new();
            for (nrange, tex) in maths {
                if self.cache.inline_failed(&tex, dpi_q) {
                    continue; // 失败记忆：降级源码（Reveal 消隐态）显示
                }
                if self.cache.inline_math(&tex, inline_key).is_none() {
                    match self.ports.math.render(&tex, false, scale, fg) {
                        Ok(img) => {
                            self.cache.inline_math.insert(
                                tex.clone(),
                                Memo {
                                    key: inline_key,
                                    value: img,
                                },
                            );
                        }
                        Err(e) => {
                            eprintln!("行内公式渲染失败（降级为源码显示）: {e}");
                            self.cache.inline_failed.insert(
                                tex.clone(),
                                Memo {
                                    key: dpi_q,
                                    value: (),
                                },
                            );
                            continue;
                        }
                    }
                }
                let img = &self.cache.inline_math[&tex].value;
                imgs.push(InlineImg {
                    range: nrange,
                    key: InlineAsset::Math(tex),
                    width: img.width,
                    height: img.height,
                    baseline: img.baseline,
                });
            }
            for (nrange, url) in images {
                let Some(path) = local_image_path(&url, self.doc.path.as_deref()) else {
                    continue; // 远程/内联 URL 或草稿相对路径：降级 alt 文本
                };
                let key = path.to_string_lossy().into_owned();
                if self.cache.image_failed(&key, image_base) {
                    continue;
                }
                if self.cache.image(&key, image_base).is_none() {
                    match self.ports.images.load(&path) {
                        Ok(img) => {
                            self.cache.inline_images.insert(
                                key.clone(),
                                Memo {
                                    key: image_base,
                                    value: img,
                                },
                            );
                        }
                        Err(e) => {
                            eprintln!("图片加载失败（降级为 alt 文本） {}: {e}", path.display());
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
                let nat = &self.cache.inline_images[&key].value;
                let (dw, dh) = fit_size(nat.width, nat.height, width);
                imgs.push(InlineImg {
                    range: nrange,
                    key: InlineAsset::Image(key),
                    width: dw,
                    height: dh,
                    baseline: dh, // 底边贴基线
                });
            }
            let l = self.engine.layout_block_with_footnotes(
                kind.clone(),
                &text,
                range,
                &inlines,
                width,
                caret,
                &imgs,
                reveal_all,
                Some(&footnotes),
            );
            let reveal = l.reveal_state(&kind, range, caret);
            self.cache.layouts.insert(
                id,
                Memo {
                    key: LayoutKey { base, reveal },
                    value: l,
                },
            );
        }
    }

    /// 仅补齐视口内（含上下各一屏 overscan）缺失布局（P1b 惰性排版）。
    pub(crate) fn ensure_layouts_visible(&mut self) {
        let range = self.visible_index_range();
        self.ensure_layouts_range(range);
    }

    /// 测试专用：等价全量排版；不参与非测试构建，避免 dead_code。
    #[cfg(test)]
    pub(crate) fn ensure_layouts_all(&mut self) {
        self.ensure_layouts_range(0..self.index.blocks.len());
    }

    /// 按需排版单块（含其数学渲染/表格网格）：光标、命中、垂直移动等路径使用。
    pub(crate) fn ensure_block(&mut self, id: BlockId) {
        let Some(i) = self.index.blocks.iter().position(|g| g.id == id) else {
            return;
        };
        self.ensure_layouts_range(i..i + 1);
        self.ensure_math_range(i..i + 1);
        self.ensure_tables_range(i..i + 1);
    }

    /// 视口覆盖的块索引区间（含上下各一屏 overscan）：`top` 已由几何重算保证
    /// 单调不减，二分定位首尾，避免逐块线性扫描。
    pub(crate) fn visible_index_range(&self) -> std::ops::Range<usize> {
        let vp = self.viewport();
        let lo = self.window_state.scroll_y - vp.h;
        let hi = self.window_state.scroll_y + 2.0 * vp.h;
        let blocks = &self.index.blocks;
        let start = blocks.partition_point(|g| g.top + g.height < lo);
        let end = blocks.partition_point(|g| g.top < hi);
        start.min(blocks.len())..end.max(start).min(blocks.len())
    }

    /// 光标在块内为源码态不渲染；失败记入 math_failed 不再每帧重试（降级源码显示）。
    /// 源码模式整块显示原文，不需要公式图，也不产图缓存。
    pub(crate) fn ensure_math_range(&mut self, indices: std::ops::Range<usize>) {
        if self.source_mode() {
            return;
        }
        let caret = self.effective_caret();
        let scale = self.input_state.scale;
        let fg = self.input_state.theme.fg;
        let Some(blocks) = self.index.blocks.get(indices) else {
            return;
        };
        for g in blocks {
            let (id, kind, range) = (g.id, g.kind.clone(), g.range);
            if kind != BlockKind::Math
                || (range.start <= caret && caret <= range.end)
                || self.cache.math(id, self.math_key(id)).is_some()
                || self.cache.math_failed(id, self.math_fail_key(id))
            {
                continue;
            }
            let Some(tex) = self.editor.session().math_source(id) else {
                continue;
            };
            match self.ports.math.render(&tex, true, scale, fg) {
                Ok(img) => {
                    self.cache.math_imgs.insert(
                        id,
                        Memo {
                            key: self.math_key(id),
                            value: img,
                        },
                    );
                }
                Err(e) => {
                    eprintln!("公式渲染失败（降级为源码显示）: {e}");
                    self.cache.math_failed.insert(
                        id,
                        Memo {
                            key: self.math_fail_key(id),
                            value: (),
                        },
                    );
                }
            }
        }
    }

    /// 仅渲染视口内（含 overscan）的数学块。
    pub(crate) fn ensure_math_visible(&mut self) {
        let range = self.visible_index_range();
        self.ensure_math_range(range);
    }

    /// 测试专用：全量数学渲染。
    #[cfg(test)]
    pub(crate) fn ensure_math(&mut self) {
        self.ensure_math_range(0..self.index.blocks.len());
    }

    /// 数学块当前是否按渲染图显示：光标在块内（Reveal 同语义）或未渲染时显示源码。
    pub(crate) fn math_shows_image(&self, id: BlockId, range: &veil_core::Range) -> bool {
        if self.source_mode() {
            return false; // 源码模式：全文显原文，不渲染公式图
        }
        let caret = self.effective_caret();
        !(range.start <= caret && caret <= range.end)
            && self.cache.math(id, self.math_key(id)).is_some()
    }

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

    /// 网格态光标矩形：活动单元格内容左上角（与点击落点、Tab 落点同源）。
    /// 表格显示为网格时，源码排版坐标与所见网格不一致，直接复用会画出「悬空光标」。
    /// `doc_top` 为块的文档坐标（非视口坐标），滚动量在此统一扣除。
    pub(crate) fn table_caret_xy(&self, id: BlockId, doc_top: f32, caret: usize) -> Option<(f32, f32, f32)> {
        let tbl = self.editor.session().table(id)?;
        let idx = active_cell_index(&tbl, caret)?;
        let cell = self.cache.table(id, self.table_key(id))?.cells.get(idx)?;
        let scale = self.input_state.scale;
        let vp = self.viewport();
        Some((
            MARGIN * scale + cell.x,
            vp.y + doc_top - self.window_state.scroll_y + cell.y,
            BASE_LINE_HEIGHT * scale,
        ))
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
                pending.push((id, key, t, active, raw_cells));
            }
        }
        for (id, key, t, active, raw_cells) in pending {
            let l = self.engine.layout_table_opts(
                &t,
                width,
                fg,
                active.as_ref().map(|(idx, src)| (*idx, src.as_str())),
                Some(&raw_cells),
            );
            self.cache.tables.insert(
                id,
                Memo {
                    key,
                    value: l,
                },
            );
        }
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
        }
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

    /// 渲染纯化（ADR-009 决策五）：产出完整帧缓冲，只读真值、只填派生缓存。
    /// 无 surface（headless）时仍能运行——`win_size()` 兜底尺寸，供幂等断言与真实像素路径测试。
    /// surface 存在性只影响 present() 侧的外部副作用。
    /// 返回 `&mut Frame` 是为了渲染外测试能继续读像素；产品层 paint() 会在 present 后归还。
    pub(crate) fn render(&mut self) -> &mut Frame {
        let (w, h) = self.win_size();
        // 首次绘制（或索引尚未建立）时补一次；其余时机由编辑/打开/resize 维护
        if self.index.blocks.is_empty() {
            self.refresh_index();
        }
        // 视口驱动惰性排版（P1b）：先排当前可见区间并重算几何；估算高度偏小
        // 可能让视口底部露出未排版块，故重算可见区间后再补跑一遍即收敛。
        self.ensure_layouts_visible();
        self.ensure_math_visible();
        self.ensure_tables_visible();
        self.recompute_geometry();
        let range = self.visible_index_range();
        let base = self.base_key();
        let caret = self.effective_caret();
        if self.index.blocks.get(range).is_some_and(|bs| {
            bs.iter().any(|g| {
                self.cache
                    .layout(g.id, &g.kind, g.range, base, caret)
                    .is_none()
            })
        }) {
            self.ensure_layouts_visible();
            self.ensure_math_visible();
            self.ensure_tables_visible();
            self.recompute_geometry();
        }
        // 帧缓冲跨帧复用：仅尺寸变化才重新分配（每帧只做一次底色填充）
        let mut frame = self
            .window_state
            .frame
            .take()
            .unwrap_or_else(|| Frame::new(w, h, self.input_state.theme.bg));
        frame.reset(w, h, self.input_state.theme.bg);
        let scale = self.input_state.scale;
        let theme = self.input_state.theme;
        let vp = self.viewport();
        // 窗口缩放/文档变短后滚动量可能越界：绘制前统一钳位（滚动条与内容同源）。
        // 打印中跳过：滚动即分页断点，末页可越过 max_scroll（页尾留空由帧底色承担）。
        if self.window_state.print.is_none() {
            self.window_state.scroll_y = self
                .window_state
                .scroll_y
                .clamp(0.0, chrome::max_scroll(vp, self.doc_height()));
        }
        // 打印中不画选区：输出为文档最终形态，不含任何交互态
        let sel = if self.window_state.print.is_none() {
            self.editor.selection()
        } else {
            None
        };
        // 只遍历视口覆盖的块（含 overscan），不再逐块判定后 continue
        let vis = self.visible_index_range();
        // 专注模式压暗需要二次遍历同一区间（主循环按值消费区间）
        let vis_focus = vis.clone();
        let inline_key = self.inline_key();
        let image_base = self.input_state.revs.image_base;
        // 超限块降级提示：内容画完后再画，避免与文本光栅/选区叠加顺序冲突
        let mut oversize: Vec<(i32, f32, usize)> = Vec::new();
        for g in &self.index.blocks[vis] {
            let (id, kind, range, top) = (g.id, g.kind.clone(), g.range, g.top);
            let sy = (vp.y + top - self.window_state.scroll_y) as i32;
            if (sy as f32) >= vp.y + vp.h || sy + self.block_height(g) as i32 <= vp.y as i32 {
                continue; // 视口外不画（布局已缓存，光栅按需）
            }
            if self.math_shows_image(id, &range) {
                // 数学块渲染态：图整体替换文本光栅（display 公式居中）
                let img = &self.cache.math_imgs[&id].value;
                let x = (MARGIN * scale
                    + ((self.content_width() - img.width as f32) / 2.0).max(0.0))
                    as i32;
                frame.blit(&img.pixels, img.width, img.height, x, sy);
                continue;
            }
            if kind == BlockKind::Table && self.table_shows_grid(id, &range) {
                self.paint_table(&mut frame, id, sy);
                continue;
            }
            let Some(lk) = self.cache.raster_key(id, &kind, range, base, caret) else {
                continue;
            };
            if self.cache.raster(id, lk).is_none() {
                let r = self
                    .engine
                    .rasterize(&self.cache.layouts[&id].value, theme.fg);
                self.cache.rasters.insert(id, Memo { key: lk, value: r });
            }
            let l = &self.cache.layouts[&id].value;
            if matches!(
                kind,
                BlockKind::FencedCode(_) | BlockKind::IndentedCode | BlockKind::HtmlBlock
            ) {
                let (rw, rh) = {
                    let r = &self.cache.rasters[&id].value;
                    (r.1, r.2)
                };
                frame.fill_rect(
                    (MARGIN * scale) as i32 - 4,
                    sy - 2,
                    rw + 8,
                    rh + 4,
                    theme.code_bg,
                );
            }
            // 查找命中高亮（P3-3）：全部命中用 hover 色，当前命中用选区色；打印不画
            if self.window_state.print.is_none() {
                self.paint_match_highlights(&mut frame, l, &range, sy);
            }
            // 选区高亮：先画底色再 blit 字形，文本保持可读
            if let Some(sel) = sel {
                let (s, e) = (sel.start.max(range.start), sel.end.min(range.end));
                if s < e {
                    for (rx, ry, rw, rh) in
                        self.engine
                            .selection_rects(l, s - range.start, e - range.start)
                    {
                        frame.fill_rect(
                            (MARGIN * scale + rx) as i32,
                            sy + ry as i32,
                            rw as u32,
                            rh as u32,
                            theme.sel,
                        );
                    }
                }
            }
            let (px, rw, rh) = &self.cache.rasters[&id].value;
            frame.blit(px, *rw, *rh, (MARGIN * scale) as i32, sy);
            // 删除线（P3-4）：叠加在字形之上；cosmic-text 无原生支持，
            // 按可视行矩形在文本中线画横线（粗细随 DPI 缩放）
            for (s, e) in &l.strikes {
                for (rx, ry, rw2, rh2) in self.engine.selection_rects(l, *s, *e) {
                    let th = (scale as i32).max(1);
                    frame.fill_rect(
                        (MARGIN * scale + rx) as i32,
                        sy + (ry + rh2 * 0.5) as i32 - th / 2,
                        rw2 as u32,
                        th as u32,
                        [theme.fg[0], theme.fg[1], theme.fg[2], 255],
                    );
                }
            }
            // 行内公式/图片：位置在排版期已定（PlacedImg），叠在文本光栅之上。
            // 图片按占位显示尺寸最近邻缩放（缓存保留自然尺寸，不生成缩放副本）。
            for p in &l.imgs {
                let dx = (MARGIN * scale + p.x) as i32;
                let dy = sy + p.y as i32;
                match &p.key {
                    InlineAsset::Math(tex) => {
                        if let Some(img) = self.cache.inline_math(tex, inline_key) {
                            frame.blit(&img.pixels, img.width, img.height, dx, dy);
                        }
                    }
                    InlineAsset::Image(key) => {
                        if let Some(img) = self.cache.image(key, image_base) {
                            frame.blit_scaled(
                                &img.pixels,
                                img.width,
                                img.height,
                                p.width,
                                p.height,
                                dx,
                                dy,
                            );
                        }
                    }
                }
            }
            // 块级装饰（切片 12）：引用左竖条；分隔线非激活时画居中横线
            match kind {
                BlockKind::BlockQuote => {
                    let bh = self.block_height(g) as u32;
                    frame.fill_rect(
                        (MARGIN * scale) as i32 - (8.0 * scale) as i32,
                        sy,
                        (3.0 * scale).max(1.0) as u32,
                        bh,
                        theme.grid,
                    );
                }
                BlockKind::ThematicBreak if !node_active(range, caret) => {
                    let bh = self.block_height(g);
                    frame.fill_rect(
                        (MARGIN * scale) as i32,
                        sy + (bh * 0.5) as i32,
                        self.content_width().max(1.0) as u32,
                        1,
                        theme.grid,
                    );
                }
                _ => {}
            }
            if l.truncated {
                oversize.push((sy, l.height, l.full_len));
            }
        }
        // 专注模式（阶段 1）：非光标块整体压暗（背景色半透明覆盖，内容与装饰一并变淡）。
        // 只读视觉，不动真值与派生缓存；当前块保持原样，光标随后绘制仍清晰。打印不压暗。
        if self.window_state.print.is_none() && self.ui_state.writing.focus {
            let focused = self.caret_block_index().map(|i| self.index.blocks[i].id);
            let dim = [theme.bg[0], theme.bg[1], theme.bg[2], FOCUS_DIM_ALPHA];
            for g in &self.index.blocks[vis_focus] {
                if focused == Some(g.id) {
                    continue;
                }
                let sy = (vp.y + g.top - self.window_state.scroll_y) as i32;
                let h = self.block_height(g).ceil() as u32;
                frame.fill_rect(0, sy, vp.w.ceil() as u32, h.max(1), dim);
            }
        }
        // 超限块降级提示：块高已含提示行，画在文本之后、光标/选区之上
        for (sy, bh, full) in oversize {
            let notice = oversize_notice(full);
            let mid = sy as f32 + bh - BASE_LINE_HEIGHT * scale / 2.0;
            if let Some(l) = self.ui_state.chrome.labels.get_or_insert(
                &mut self.engine,
                &notice,
                theme.muted,
                scale,
            ) {
                frame.blit(
                    &l.pixels,
                    l.w,
                    l.h,
                    (MARGIN * scale).round() as i32,
                    (mid - l.h as f32 / 2.0).round() as i32,
                );
            }
        }
        // 光标/浮层/chrome 只在屏幕语义下绘制；打印帧 = 纯文档内容（3b）
        if self.window_state.print.is_none() {
            let (line, col) = self.editor.session().line_col(caret).unwrap_or((1, 1));
            let dirty = self.editor.is_dirty();
            let external = self.external_state();
            let doc_h = self.doc_height();
            // 查找栏打开时文档只读：光标与 preedit 均由浮栏承接，不画在正文
            if !self.ui_state.search.open {
                self.paint_caret_and_preedit(&mut frame);
            }
            // 大纲面板（P3-2）：内容之后、chrome 之前（标题区/状态栏覆盖上溢行）
            self.paint_outline(&mut frame);
            // 查找栏（P3-3）：浮在内容/大纲之上、chrome 之前的最后一层内容装饰
            self.paint_search(&mut frame);
            // 快速打开（阶段 1）：浮在查找栏之上
            self.paint_quick(&mut frame);
            // 右键菜单（2f）：浮在全部内容浮层之上、chrome 之前
            self.paint_ctx_menu(&mut frame);
            // 拖放悬停提示（阶段 1）：浮在最上层内容装饰
            self.paint_drop_hint(&mut frame);
            // chrome 最后绘制（不透明覆盖内容；滚动条区保留给状态栏行列）
            let recent = self.recent_labels();
            let flags = chrome::MenuFlags {
                fullscreen: self.window_state.fullscreen,
                focus: self.ui_state.writing.focus,
                typewriter: self.ui_state.writing.typewriter,
                source: self.source_mode(),
            };
            self.ui_state.chrome.paint(
                &mut frame,
                &mut self.engine,
                vp,
                scale,
                &theme,
                self.doc.path.as_deref(),
                dirty,
                external,
                self.ui_state.writing.words,
                line,
                col,
                doc_h,
                self.window_state.scroll_y,
                &recent,
                flags,
            );
        }
        // 视口驱动缓存回收：paint 帧末尾把 rasters/layouts/tables 从"全部累积"剪到
        // "可见区间 + caret 块 + 块级公式图"量级，防止滚动过程中内存持续增长。
        // 滚动是纯 view-only 操作，不触发 refresh_index → prune_alive，光栅累积永不回收。
        let vp_range = self.visible_index_range();
        let mut keep: HashSet<BlockId> = self.index.blocks[vp_range].iter().map(|g| g.id).collect();
        if let Some(i) = self.caret_block_index() {
            keep.insert(self.index.blocks[i].id);
        }
        keep.extend(self.cache.math_imgs.keys().copied());
        self.cache.prune_viewport(&keep, &base);
        // 归还缓冲供下一帧复用；返回 &mut Frame 供产品层 present() 与测试断言
        self.window_state.frame = Some(frame);
        self.window_state.frame.as_mut().unwrap()
    }

    /// 呈现分离（ADR-009 决策五）：唯一产生外部副作用的出口。
    /// 从 self.window_state.frame 取缓冲，拷进 surface 并 present；失败属瞬时状态，丢帧不崩溃。
    /// 无 surface（headless）时纯 no-op。
    pub(crate) fn present(&mut self) {
        let Some(frame) = self.window_state.frame.as_ref() else {
            return;
        };
        let Some(surface) = self.window_state.surface.as_mut() else {
            return;
        };
        if let Ok(mut target) = surface.buffer_mut() {
            if !can_present(frame.pixels.len(), target.len()) {
                return;
            }
            target.copy_from_slice(&frame.pixels);
            let _ = target.present();
        }
    }

    /// 帧合成完整路径：render → present。产品层事件循环 Redraw 分支唯一调用。
    pub(crate) fn paint(&mut self) {
        let started = std::time::Instant::now();
        let _ = self.render();
        self.present();
        crate::perf::frame(started.elapsed());
    }

    /// 大纲面板（P3-2）：底色 → 条目（含悬停底色）→ 标题区（后画，覆盖上溢行）。
    /// 面板宽已从版心扣除；标签位图复用 chrome 缓存（键含颜色/scale）。
    pub(crate) fn paint_outline(&mut self, frame: &mut Frame) {
        let Some(panel) = self.outline_rect() else {
            return;
        };
        let scale = self.input_state.scale;
        let theme = self.input_state.theme;
        let count = self.ui_state.outline.entries.len();
        self.ui_state.outline.scroll = self
            .ui_state
            .outline
            .scroll
            .clamp(0.0, outline::max_scroll(panel, count, scale));
        let x = panel.x.round() as i32;
        let w = panel.w.ceil() as u32;
        frame.fill_rect(x, panel.y as i32, w, panel.h.ceil() as u32, theme.chrome_bg);
        for row in outline::visible_rows(panel, count, self.ui_state.outline.scroll, scale) {
            if self.ui_state.outline.hover == Some(row.index) {
                frame.fill_rect(x, row.y.round() as i32, w, row.h.ceil() as u32, theme.hover);
            }
            let (level, text) = {
                let e = &self.ui_state.outline.entries[row.index];
                (e.level, e.text.clone())
            };
            let indent = outline::INDENT_STEP * scale * level.saturating_sub(1) as f32;
            let label = outline::fit_label(&text, (row.w - indent).max(0.0), scale);
            let lx = row.x + indent;
            let mid = row.y + row.h / 2.0;
            if let Some(l) = self.ui_state.chrome.labels.get_or_insert(
                &mut self.engine,
                &label,
                theme.text,
                scale,
            ) {
                let ly = mid - l.h as f32 / 2.0;
                frame.blit(&l.pixels, l.w, l.h, lx.round() as i32, ly.round() as i32);
            }
        }
        if count == 0 {
            if let Some(l) = self.ui_state.chrome.labels.get_or_insert(
                &mut self.engine,
                "无标题",
                theme.muted,
                scale,
            ) {
                let top = outline::rows_top(panel, scale) + outline::PAD * scale;
                frame.blit(
                    &l.pixels,
                    l.w,
                    l.h,
                    (panel.x + outline::PAD * scale).round() as i32,
                    top.round() as i32,
                );
            }
        }
        // 标题区（覆盖滚动到标题区内的行）
        let head_h = (outline::HEAD_H * scale).ceil() as u32;
        frame.fill_rect(x, panel.y as i32, w, head_h, theme.chrome_bg);
        frame.fill_rect(
            x,
            (panel.y + outline::HEAD_H * scale) as i32 - 1,
            w,
            1,
            theme.chrome_border,
        );
        if let Some(l) =
            self.ui_state
                .chrome
                .labels
                .get_or_insert(&mut self.engine, "大纲", theme.muted, scale)
        {
            let ly = panel.y + (outline::HEAD_H * scale - l.h as f32) / 2.0;
            frame.blit(
                &l.pixels,
                l.w,
                l.h,
                (panel.x + outline::PAD * scale).round() as i32,
                ly.round() as i32,
            );
        }
        // 左边框最后画：标题区底色不得覆盖它
        frame.fill_rect(
            x,
            panel.y as i32,
            1,
            panel.h.ceil() as u32,
            theme.chrome_border,
        );
    }

    /// 查找命中高亮（P3-3）：全部命中用 hover 色、当前命中用选区色，画在字形之下。
    /// 与选区同层语义：仅覆盖按文本布局绘制的内容块（表格网格/公式图 v0 简化）。
    pub(crate) fn paint_match_highlights(
        &self,
        frame: &mut Frame,
        l: &BlockLayout,
        range: &Range,
        sy: i32,
    ) {
        if !self.ui_state.search.open || self.ui_state.search.matches.is_empty() {
            return;
        }
        let x0 = MARGIN * self.input_state.scale;
        // 命中按起点升序：二分定位首个相交者，向后多取一个以覆盖跨块命中
        let from = self
            .ui_state
            .search
            .matches
            .partition_point(|m| m.start < range.start)
            .saturating_sub(1);
        for (i, m) in self.ui_state.search.matches.iter().enumerate().skip(from) {
            if m.start >= range.end {
                break;
            }
            let (s, e) = (m.start.max(range.start), m.end.min(range.end));
            if s >= e {
                continue;
            }
            let color = if i == self.ui_state.search.current {
                self.input_state.theme.sel
            } else {
                self.input_state.theme.hover
            };
            for (rx, ry, rw, rh) in self
                .engine
                .selection_rects(l, s - range.start, e - range.start)
            {
                frame.fill_rect(
                    (x0 + rx) as i32,
                    sy + ry as i32,
                    rw as u32,
                    rh as u32,
                    color,
                );
            }
        }
    }

    /// 查找栏（P3-3）：底色 → 按钮悬停底色 → 边框/分隔线 → 查询文本/IME 组合串
    /// → 计数 → 按钮字形；替换模式（Ctrl+H）多一行替换输入与「替换」/「全部」。
    /// 栏不透明覆盖内容；标签复用 chrome 位图缓存（随主题失效）。
    pub(crate) fn paint_search(&mut self, frame: &mut Frame) {
        let Some(l) = self.search_rect() else {
            return;
        };
        let scale = self.input_state.scale;
        let theme = self.input_state.theme;
        let search = &self.ui_state.search;
        let replace = search.replace_open;
        let rl = replace.then(|| search::replace_layout(&l, scale));
        let bar = l.bar;
        let bx = bar.x.round() as i32;
        let by = bar.y.round() as i32;
        let bw = bar.w.ceil() as u32;
        let bh = bar.h.ceil() as u32;
        let bottom = rl.map_or(bar.y + bar.h, |r| r.row.y + r.row.h).round() as i32;
        let total_h = (bottom - by).max(1) as u32;
        let sep_y = (bar.y + bar.h).round() as i32;
        frame.fill_rect(bx, by, bw, total_h, theme.chrome_bg);
        // 按钮悬停底色（两行按钮同一枚举）
        let mut hovers = vec![
            (search::Hit::Prev, l.prev),
            (search::Hit::Next, l.next),
            (search::Hit::Close, l.close),
        ];
        if let Some(r) = rl {
            hovers.push((search::Hit::ReplaceOne, r.one));
            hovers.push((search::Hit::ReplaceAll, r.all));
        }
        for (hit, r) in hovers {
            if search.hover == Some(hit) {
                frame.fill_rect(
                    r.x.round() as i32 + 1,
                    r.y.round() as i32 + 1,
                    r.w.ceil() as u32 - 2,
                    r.h.ceil() as u32 - 2,
                    theme.hover,
                );
            }
        }
        // 边框与按钮分隔线（1px）
        frame.fill_rect(bx, by, bw, 1, theme.chrome_border);
        frame.fill_rect(bx, by + total_h as i32 - 1, bw, 1, theme.chrome_border);
        frame.fill_rect(bx, by, 1, total_h, theme.chrome_border);
        frame.fill_rect(bx + bw as i32 - 1, by, 1, total_h, theme.chrome_border);
        if replace {
            frame.fill_rect(bx, sep_y, bw, 1, theme.chrome_border);
        }
        for r in [l.prev, l.next, l.close] {
            frame.fill_rect(
                r.x.round() as i32,
                r.y.round() as i32 + 1,
                1,
                bh.saturating_sub(2),
                theme.chrome_border,
            );
        }
        let mid = bar.y + bar.h / 2.0;
        let text_y = |h: u32| (mid - h as f32 / 2.0).round() as i32;
        // 查询文本：空串显示占位提示；非空保留尾部（正在输入的一端）
        // 光标 x 只跟随真实查询文本（占位提示不计入）：空串时落在输入区左端，
        // 有文本时落在文本末端——两者共同表达「插入点」，占位文案不参与定位。
        let mut qw = 0.0;
        if search.query.is_empty() {
            if self.editor.preedit().is_none() {
                if let Some(lab) = self.ui_state.chrome.labels.get_or_insert(
                    &mut self.engine,
                    "查找",
                    theme.muted,
                    scale,
                ) {
                    frame.blit(
                        &lab.pixels,
                        lab.w,
                        lab.h,
                        l.text.x.round() as i32,
                        text_y(lab.h),
                    );
                }
            }
        } else {
            let shown = search::fit_query(&search.query, l.text.w, scale);
            if let Some(lab) = self.ui_state.chrome.labels.get_or_insert(
                &mut self.engine,
                &shown,
                theme.text,
                scale,
            ) {
                qw = lab.w as f32;
                frame.blit(
                    &lab.pixels,
                    lab.w,
                    lab.h,
                    l.text.x.round() as i32,
                    text_y(lab.h),
                );
            }
        }
        let focus = search.focus;
        // 查询框光标：聚焦 + 无 IME preedit → 可见文本末端画 2×ch 竖线
        if focus == search::Field::Query && self.editor.preedit().is_none() {
            let cx = (l.text.x + qw).round() as i32;
            let ch = 16.0 * scale;
            let cy = (mid - ch / 2.0).round() as i32;
            frame.fill_rect(cx, cy, 2, ch as u32, [theme.fg[0], theme.fg[1], theme.fg[2], 255]);
        }
        // IME 组合串：贴在查询文本之后（贴右缘钳位），下划线同文档 preedit；
        // 替换模式下焦点在替换框时由替换行负责绘制
        if focus == search::Field::Query || !replace {
            if let Some(pre) = self.editor.preedit() {
                let text = pre.text.clone();
                let pl = self.engine.layout_preedit(&text);
                let (px, w, h) = self.engine.rasterize(&pl, theme.fg);
                let x0 = (l.text.x + qw).min((l.text.x + l.text.w - w as f32).max(l.text.x));
                let y0 = mid - h as f32 / 2.0;
                frame.blit(&px, w, h, x0.round() as i32, y0.round() as i32);
                frame.fill_rect(
                    x0.round() as i32,
                    (y0 + h as f32) as i32 - 1,
                    w,
                    1,
                    [theme.fg[0], theme.fg[1], theme.fg[2], 255],
                );
            }
        }
        // 替换行：文本框 → 焦点下划线 → IME 组合串 → 两个按钮字形
        if let Some(r) = rl {
            let rmid = r.row.y + r.row.h / 2.0;
            let rtext_y = |h: u32| (rmid - h as f32 / 2.0).round() as i32;
            let mut rw = 0.0;
            if search.replace.is_empty() {
                if !(focus == search::Field::Replace && self.editor.preedit().is_some()) {
                    if let Some(lab) = self.ui_state.chrome.labels.get_or_insert(
                        &mut self.engine,
                        "替换为",
                        theme.muted,
                        scale,
                    ) {
                        frame.blit(
                            &lab.pixels,
                            lab.w,
                            lab.h,
                            r.text.x.round() as i32,
                            rtext_y(lab.h),
                        );
                    }
                }
            } else {
                let shown = search::fit_query(&search.replace, r.text.w, scale);
                if let Some(lab) = self.ui_state.chrome.labels.get_or_insert(
                    &mut self.engine,
                    &shown,
                    theme.text,
                    scale,
                ) {
                    rw = lab.w as f32;
                    frame.blit(
                        &lab.pixels,
                        lab.w,
                        lab.h,
                        r.text.x.round() as i32,
                        rtext_y(lab.h),
                    );
                }
            }
            // 替换框光标：聚焦 + 无 IME preedit → 可见文本末端画 2×ch 竖线
            if focus == search::Field::Replace && self.editor.preedit().is_none() {
                let cx = (r.text.x + rw).round() as i32;
                let ch = 16.0 * scale;
                let cy = (rmid - ch / 2.0).round() as i32;
                frame.fill_rect(cx, cy, 2, ch as u32, [theme.fg[0], theme.fg[1], theme.fg[2], 255]);
            }
            if focus == search::Field::Replace {
                if let Some(pre) = self.editor.preedit() {
                    let text = pre.text.clone();
                    let pl = self.engine.layout_preedit(&text);
                    let (px, w, h) = self.engine.rasterize(&pl, theme.fg);
                    let x0 = (r.text.x + rw).min((r.text.x + r.text.w - w as f32).max(r.text.x));
                    let y0 = rmid - h as f32 / 2.0;
                    frame.blit(&px, w, h, x0.round() as i32, y0.round() as i32);
                    frame.fill_rect(
                        x0.round() as i32,
                        (y0 + h as f32) as i32 - 1,
                        w,
                        1,
                        [theme.fg[0], theme.fg[1], theme.fg[2], 255],
                    );
                }
            }
            // 焦点框底边细线：点击/IME 都落在这里
            let (fy, fx, fw) = if focus == search::Field::Replace {
                (r.row.y + r.row.h, r.text.x, r.text.w)
            } else {
                (bar.y + bar.h, l.text.x, l.text.w)
            };
            frame.fill_rect(
                fx.round() as i32,
                fy.round() as i32 - 2,
                fw.ceil() as u32,
                1,
                theme.chrome_border,
            );
            for (btn, glyph) in [(r.one, "替换"), (r.all, "全部")] {
                frame.fill_rect(
                    btn.x.round() as i32,
                    btn.y.round() as i32 + 1,
                    1,
                    bh.saturating_sub(2),
                    theme.chrome_border,
                );
                if let Some(lab) = self.ui_state.chrome.labels.get_or_insert(
                    &mut self.engine,
                    glyph,
                    theme.text,
                    scale,
                ) {
                    let x = btn.x + (btn.w - lab.w as f32) / 2.0;
                    let y = btn.y + (btn.h - lab.h as f32) / 2.0;
                    frame.blit(
                        &lab.pixels,
                        lab.w,
                        lab.h,
                        x.round() as i32,
                        y.round() as i32,
                    );
                }
            }
        }
        // 计数「i/n」（右对齐；空查询不显示）
        let count = search.label();
        if !count.is_empty() {
            if let Some(lab) = self.ui_state.chrome.labels.get_or_insert(
                &mut self.engine,
                &count,
                theme.muted,
                scale,
            ) {
                let x = (l.count_right - lab.w as f32).round() as i32;
                frame.blit(&lab.pixels, lab.w, lab.h, x, text_y(lab.h));
            }
        }
        // 查询行按钮字形（居中）
        for (r, glyph) in [(l.prev, "‹"), (l.next, "›"), (l.close, "×")] {
            if let Some(lab) = self.ui_state.chrome.labels.get_or_insert(
                &mut self.engine,
                glyph,
                theme.text,
                scale,
            ) {
                let x = r.x + (r.w - lab.w as f32) / 2.0;
                let y = r.y + (r.h - lab.h as f32) / 2.0;
                frame.blit(
                    &lab.pixels,
                    lab.w,
                    lab.h,
                    x.round() as i32,
                    y.round() as i32,
                );
            }
        }
    }

    /// 快速打开浮层（阶段 1）：面板边框 → 输入行 → 候选行（选中/悬停底色、
    /// 文件名与来源提示）。面板不透明覆盖内容与查找栏；标签复用 chrome 位图缓存。
    pub(crate) fn paint_quick(&mut self, frame: &mut Frame) {
        let Some(l) = self.quick_rect() else {
            return;
        };
        let scale = self.input_state.scale;
        let theme = self.input_state.theme;
        let pad = quick_open::PAD * scale;
        let px = l.panel.x.round() as i32;
        let py = l.panel.y.round() as i32;
        let pw = l.panel.w.ceil() as u32;
        let ph = l.panel.h.ceil() as u32;
        frame.fill_rect(px, py, pw, ph, theme.popup_bg);
        frame.fill_rect(px, py, pw, 1, theme.chrome_border);
        frame.fill_rect(px, py + ph as i32 - 1, pw, 1, theme.chrome_border);
        frame.fill_rect(px, py, 1, ph, theme.chrome_border);
        frame.fill_rect(px + pw as i32 - 1, py, 1, ph, theme.chrome_border);
        // 输入行与候选行的分隔线
        frame.fill_rect(
            px,
            (l.input.y + l.input.h).round() as i32,
            pw,
            1,
            theme.chrome_border,
        );

        let query = self.ui_state.quick.query.clone();
        let top = self.ui_state.quick.top;
        let selected = self.ui_state.quick.selected;
        let hover = self.ui_state.quick.hover;
        let empty = self.ui_state.quick.filtered.is_empty();
        let visible: Vec<(usize, String, String, bool)> = self
            .ui_state
            .quick
            .visible()
            .map(|slot| {
                self.ui_state
                    .quick
                    .filtered
                    .get(slot)
                    .and_then(|&i| self.ui_state.quick.items.get(i))
                    .map(|it| (slot, it.label.clone(), it.hint.clone(), it.recent))
                    .unwrap_or((slot, String::new(), String::new(), false))
            })
            .collect();

        // 输入行：占位提示 / 过滤串（保留尾部，正在输入的一端）+ IME 组合串
        let imid = l.input.y + l.input.h * 0.5;
        let input_y = |h: u32| (imid - h as f32 / 2.0).round() as i32;
        let mut qw = 0.0;
        if query.is_empty() {
            if self.editor.preedit().is_none() {
                if let Some(lab) = self.ui_state.chrome.labels.get_or_insert(
                    &mut self.engine,
                    "快速打开：输入文件名，回车打开",
                    theme.muted,
                    scale,
                ) {
                    qw = lab.w as f32;
                    frame.blit(
                        &lab.pixels,
                        lab.w,
                        lab.h,
                        (l.input.x + pad).round() as i32,
                        input_y(lab.h),
                    );
                }
            }
        } else {
            let shown = search::fit_query(&query, l.input.w - 2.0 * pad, scale);
            if let Some(lab) = self.ui_state.chrome.labels.get_or_insert(
                &mut self.engine,
                &shown,
                theme.text,
                scale,
            ) {
                qw = lab.w as f32;
                frame.blit(
                    &lab.pixels,
                    lab.w,
                    lab.h,
                    (l.input.x + pad).round() as i32,
                    input_y(lab.h),
                );
            }
        }
        if let Some(pre) = self.editor.preedit() {
            let text = pre.text.clone();
            let pl = self.engine.layout_preedit(&text);
            let (pxs, w, h) = self.engine.rasterize(&pl, theme.fg);
            let x0 = (l.input.x + pad + qw)
                .min((l.input.x + l.input.w - pad - w as f32).max(l.input.x + pad));
            let y0 = imid - h as f32 / 2.0;
            frame.blit(&pxs, w, h, x0.round() as i32, y0.round() as i32);
            frame.fill_rect(
                x0.round() as i32,
                (y0 + h as f32) as i32 - 1,
                w,
                1,
                [theme.fg[0], theme.fg[1], theme.fg[2], 255],
            );
        }

        // 候选行：底色 → 来源提示（右对齐）→ 文件名（左对齐，避让提示后截断）
        for (slot, label, hint, recent) in visible {
            let Some(row) = l.rows.get(slot - top) else {
                continue;
            };
            if slot == selected || hover == Some(slot) {
                frame.fill_rect(
                    row.x.round() as i32 + 1,
                    row.y.round() as i32,
                    row.w.ceil() as u32 - 2,
                    row.h.ceil() as u32,
                    theme.hover,
                );
            }
            if label.is_empty() {
                continue;
            }
            let mid = row.y + row.h / 2.0;
            let ty = |h: u32| (mid - h as f32 / 2.0).round() as i32;
            let hint = if recent { "最近".to_string() } else { hint };
            let hint_shown = quick_open::fit_label(&hint, row.w * 0.45, scale);
            let hint_w = if hint_shown.is_empty() {
                0.0
            } else {
                quick_open::text_width(&hint_shown, scale)
            };
            if hint_w > 0.0 {
                if let Some(lab) = self.ui_state.chrome.labels.get_or_insert(
                    &mut self.engine,
                    &hint_shown,
                    theme.muted,
                    scale,
                ) {
                    frame.blit(
                        &lab.pixels,
                        lab.w,
                        lab.h,
                        (row.x + row.w - pad - lab.w as f32).round() as i32,
                        ty(lab.h),
                    );
                }
            }
            let label_w =
                (row.w - 2.0 * pad - hint_w - if hint_w > 0.0 { pad } else { 0.0 }).max(0.0);
            let shown = quick_open::fit_label(&label, label_w, scale);
            if let Some(lab) = self.ui_state.chrome.labels.get_or_insert(
                &mut self.engine,
                &shown,
                theme.text,
                scale,
            ) {
                frame.blit(
                    &lab.pixels,
                    lab.w,
                    lab.h,
                    (row.x + pad).round() as i32,
                    ty(lab.h),
                );
            }
        }
        // 空态提示：占首行
        if empty {
            if let Some(row) = l.rows.first() {
                if let Some(lab) = self.ui_state.chrome.labels.get_or_insert(
                    &mut self.engine,
                    "无匹配文件",
                    theme.muted,
                    scale,
                ) {
                    let y = (row.y + row.h / 2.0 - lab.h as f32 / 2.0).round() as i32;
                    frame.blit(&lab.pixels, lab.w, lab.h, (row.x + pad).round() as i32, y);
                }
            }
        }
    }

    /// 右键上下文菜单（2f）：面板 → 悬停底色 → 项标签。瞬态浮层：
    /// 只读视觉，不触碰文档真值与缓存；标签位图复用 chrome 缓存。
    pub(crate) fn paint_ctx_menu(&mut self, frame: &mut Frame) {
        if !self.ui_state.ctx.open {
            return;
        }
        let scale = self.input_state.scale;
        let theme = self.input_state.theme;
        let items: Vec<(&'static str, ctx_menu::CtxAction)> = self.ui_state.ctx.items().to_vec();
        let vp = self.viewport();
        let l = ctx_menu::layout(
            &items,
            self.ui_state.ctx.x,
            self.ui_state.ctx.y,
            vp.x + vp.w,
            vp.y + vp.h,
            scale,
        );
        let px = l.x.round() as i32;
        let py = l.y.round() as i32;
        let pw = l.w.ceil() as u32;
        let ph = l.h.ceil() as u32;
        frame.fill_rect(px, py, pw, ph, theme.popup_bg);
        frame.fill_rect(px, py, pw, 1, theme.chrome_border);
        frame.fill_rect(px, py + ph as i32 - 1, pw, 1, theme.chrome_border);
        frame.fill_rect(px, py, 1, ph, theme.chrome_border);
        frame.fill_rect(px + pw as i32 - 1, py, 1, ph, theme.chrome_border);
        let hover = self.ui_state.ctx.hover;
        for (i, (label, _)) in items.iter().enumerate() {
            let iy = l.y + l.top_pad + l.item_h * i as f32;
            if hover == Some(i) {
                frame.fill_rect(
                    px + 1,
                    iy.round() as i32,
                    pw.saturating_sub(2),
                    l.item_h.ceil() as u32,
                    theme.hover,
                );
            }
            if let Some(lab) = self.ui_state.chrome.labels.get_or_insert(
                &mut self.engine,
                label,
                theme.text,
                scale,
            ) {
                let y = (iy + l.item_h / 2.0 - lab.h as f32 / 2.0).round() as i32;
                frame.blit(&lab.pixels, lab.w, lab.h, (l.x + l.pad).round() as i32, y);
            }
        }
    }

    /// 拖放落点提示（阶段 1）：内容区压暗 + 居中提示条。
    /// 图片与文档给不同文案；草稿拖图片明确说明「无法插入」而不是静默失败。
    /// 只读视觉：不触碰文档真值与缓存；悬停文件名来自 winit HoveredFile。
    pub(crate) fn paint_drop_hint(&mut self, frame: &mut Frame) {
        let Some(path) = self.input_state.drop_file.clone() else {
            return;
        };
        let scale = self.input_state.scale;
        let theme = self.input_state.theme;
        let vp = self.viewport();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let text = if crate::image::is_image_path(&path) {
            if self.doc.path.is_none() {
                format!("「{name}」无法插入：草稿没有落盘目录")
            } else {
                format!("松开以插入图片：{name}")
            }
        } else {
            format!("松开以打开文档：{name}")
        };
        frame.fill_rect(
            vp.x.round() as i32,
            vp.y.round() as i32,
            vp.w.ceil() as u32,
            vp.h.ceil() as u32,
            [theme.bg[0], theme.bg[1], theme.bg[2], DROP_DIM_ALPHA],
        );
        let pad = (12.0 * scale).round() as u32;
        let Some(lab) =
            self.ui_state
                .chrome
                .labels
                .get_or_insert(&mut self.engine, &text, theme.text, scale)
        else {
            return;
        };
        let bw = lab.w + pad * 2;
        let bh = lab.h + pad * 2;
        let bx = (vp.x + (vp.w - bw as f32) / 2.0).round() as i32;
        let by = (vp.y + (vp.h - bh as f32) / 2.0).round() as i32;
        frame.fill_rect(bx, by, bw, bh, theme.popup_bg);
        frame.fill_rect(bx, by, bw, 1, theme.chrome_border);
        frame.fill_rect(bx, by + bh as i32 - 1, bw, 1, theme.chrome_border);
        frame.fill_rect(bx, by, 1, bh, theme.chrome_border);
        frame.fill_rect(bx + bw as i32 - 1, by, 1, bh, theme.chrome_border);
        frame.blit(
            &lab.pixels,
            lab.w,
            lab.h,
            bx + pad as i32,
            (by + pad as i32) + ((bh - pad * 2 - lab.h) / 2) as i32,
        );
    }

    /// 绘制表格网格：活动单元格底色 → 表头底色 → 单元格文本 → 网格线（先底后字，线压边）。
    /// 活动单元格由排版键记录的 `active` 判定，光标绘制复用既有 `paint_caret_and_preedit` 路径。
    /// 选区跨网格态表格时不画高亮（同数学块 v0 已知简化）。
    pub(crate) fn paint_table(&self, frame: &mut Frame, id: BlockId, top: i32) {
        let Some(t) = self.cache.table(id, self.table_key(id)) else {
            return;
        };
        let x0 = (MARGIN * self.input_state.scale) as i32;
        let theme = self.input_state.theme;

        // 活动单元格：排版键已记录 caret 命中的格（源码态显示），画选中色底色
        if let Some(cell_layout) = t.active.and_then(|idx| t.cells.get(idx)) {
            let cx = x0 + cell_layout.x.round() as i32;
            let cy = top + cell_layout.y.round() as i32;
            let cw = cell_layout.w.round().max(1.0) as u32;
            let ch = cell_layout.h.round().max(1.0) as u32;
            frame.fill_rect(cx, cy, cw, ch, theme.sel);
        }

        if let Some(first) = t.rows.first() {
            frame.fill_rect(
                x0,
                top,
                t.width.ceil() as u32,
                first.height.ceil() as u32,
                theme.table_head_bg,
            );
        }
        for c in &t.cells {
            let cy = top + c.y.round() as i32;
            if cy + c.pixel_h as i32 <= 0 || cy >= frame.height as i32 {
                continue; // 行在视口外
            }
            frame.blit(&c.pixels, c.pixel_w, c.pixel_h, x0 + c.x.round() as i32, cy);
        }
        let w = t.width.ceil().max(1.0) as u32;
        let h = t.height.ceil().max(1.0) as u32;
        let mut x: f32 = 0.0;
        for cw in &t.cols {
            frame.fill_rect(x0 + x.round() as i32, top, 1, h, theme.grid);
            x += cw;
        }
        let right = t.width.round().max(1.0) as i32;
        frame.fill_rect(x0 + right - 1, top, 1, h, theme.grid);
        frame.fill_rect(x0, top, w, 1, theme.grid);
        for r in &t.rows {
            frame.fill_rect(
                x0,
                top + (r.top + r.height).round() as i32 - 1,
                w,
                1,
                theme.grid,
            );
        }
    }

    pub(crate) fn paint_caret_and_preedit(&mut self, frame: &mut Frame) {
        let (cx, cy, ch) = self.caret_screen();
        let vp = self.viewport();
        if cy + ch <= vp.y || cy >= vp.y + vp.h {
            return; // 光标在视口外：不画（被 chrome 覆盖前已越界）
        }
        let fg = self.input_state.theme.fg;
        let caret_color = [fg[0], fg[1], fg[2], 255];
        if let Some(pre) = self.editor.preedit() {
            // preedit：排版小串 → 位图贴在光标处 + 下划线；光标移至组合串内光标位
            let text = pre.text.clone();
            let l = self.engine.layout_preedit(&text);
            let (px, w, h) = self.engine.rasterize(&l, fg);
            frame.blit(&px, w, h, cx as i32, (cy + ch - h as f32).max(0.0) as i32);
            frame.fill_rect(
                cx as i32,
                (cy + ch) as i32 - 1,
                w,
                1,
                [fg[0], fg[1], fg[2], 255],
            );
            let inner = pre.cursor.map(|(s, _)| s).unwrap_or(0);
            let (ix, _, _) = self.engine.caret_xy(&l, inner);
            frame.fill_rect((cx + ix) as i32, cy as i32, 1, ch as u32, caret_color);
        } else {
            frame.fill_rect(cx as i32, cy as i32, 2, ch as u32, caret_color);
        }
    }

    pub(crate) fn scroll_to_caret(&mut self) {
        let vp = self.viewport();
        let (_, cy, ch) = self.caret_screen();
        if self.ui_state.writing.typewriter {
            // 打字机模式（阶段 1）：光标行固定在视口上部锚点，行中心对齐锚点；
            // 文档首尾处仍由下方钳位兜底（无法再居中时贴边）
            let target = vp.y + vp.h * TYPEWRITER_ANCHOR;
            self.window_state.scroll_y += cy + ch / 2.0 - target;
        } else if cy < vp.y {
            self.window_state.scroll_y += cy - vp.y - MARGIN * self.input_state.scale;
        } else if cy + ch > vp.y + vp.h {
            self.window_state.scroll_y += cy + ch - (vp.y + vp.h) + MARGIN * self.input_state.scale;
        }
        self.window_state.scroll_y = self
            .window_state
            .scroll_y
            .clamp(0.0, chrome::max_scroll(vp, self.doc_height()));
    }

    /// Ctrl+Enter：在 caret 所在表格末尾追加一个空 body 行。返回 true 表示已处理。
    pub(crate) fn table_ctrl_enter(&mut self) -> bool {
        let caret = self.editor.caret();
        let Some((_tid, _tbl)) = self.caret_table_and_cell(caret) else {
            return false;
        };
        let Some(block_range) = self.index.blocks.iter()
            .find(|g| g.kind == BlockKind::Table && g.range.start <= caret && caret <= g.range.end)
            .map(|g| g.range) else {
            return false;
        };
        let src = self.editor.session().text_slice(block_range).unwrap_or_default();
        let new_src = table_append_row(&src);
        if new_src == src {
            return false;
        }
        // 新行在块尾；光标落到新行首个单元格内容处（`|` 后跳过前导空白）。
        // 空单元格（`|  |`）若只按 `+2` 定位会落在管道符上，活动单元格判定失败、Tab 跳格错位。
        let last_line = new_src.trim_end_matches(['\n', '\r']).matches('\n').count();
        let caret_in_block = table_cell_start(&new_src, last_line, 0);
        let Some(dirty_blocks) = self
            .editor
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
#[allow(dead_code)]
fn split_row(row: &str) -> Vec<String> {
    // 去掉首尾空白和首/尾管道符
    let trimmed = row.trim();
    let trimmed = trimmed.trim_start_matches('|').trim_end_matches('|');
    if trimmed.is_empty() {
        return Vec::new();
    }
    trimmed
        .split('|')
        .map(|s| s.trim().to_string())
        .collect()
}

#[allow(dead_code)]
fn join_row(cells: &[String]) -> String {
    let parts: Vec<String> = cells.iter().map(|s| format!(" {} ", s)).collect();
    format!("|{}|", parts.join("|"))
}

#[allow(dead_code)]
fn sep_cell_for(align: veil_core::CellAlign) -> String {
    match align {
        veil_core::CellAlign::None => "---".into(),
        veil_core::CellAlign::Left => ":--".into(),
        veil_core::CellAlign::Center => ":-:".into(),
        veil_core::CellAlign::Right => "--:".into(),
    }
}

#[allow(dead_code)]
fn parse_aligns(sep_line: &str) -> Vec<veil_core::CellAlign> {
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
pub(crate) fn table_cell_start(src: &str, line_index: usize, col: usize) -> usize {
    let lines: Vec<&str> = src.split('\n').collect();
    let Some(line) = lines.get(line_index) else {
        return 0;
    };
    let line_off: usize = lines[..line_index].iter().map(|l| l.len() + 1).sum();
    let mut i = line.len() - line.trim_start().len();
    if line[i..].starts_with('|') {
        i += 1;
    }
    let mut cur = 0usize;
    loop {
        while i < line.len() && line.as_bytes()[i] == b' ' {
            i += 1;
        }
        if cur == col {
            return line_off + i;
        }
        match line[i..].find('|') {
            Some(k) => {
                i += k + 1;
                cur += 1;
            }
            None => return 0,
        }
    }
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

/// 分隔行单元格（含两侧空白）的字节区间：剥除行首尾空白与可选外层竖线后按 `|` 切分。
fn sep_cell_spans(line: &str) -> Vec<(usize, usize)> {
    let b = line.as_bytes();
    let (mut s, mut e) = (0, b.len());
    while s < e && b[s].is_ascii_whitespace() {
        s += 1;
    }
    while e > s && b[e - 1].is_ascii_whitespace() {
        e -= 1;
    }
    if s < e && b[s] == b'|' {
        s += 1;
    }
    if e > s && b[e - 1] == b'|' {
        e -= 1;
    }
    let mut out = Vec::new();
    let mut seg = s;
    let mut i = s;
    while i < e {
        if b[i] == b'|' {
            out.push((seg, i));
            i += 1;
            seg = i;
        } else {
            i += 1;
        }
    }
    out.push((seg, e));
    out
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

/// 图片 URL → 本地路径（切片 13）。相对 URL 相对文档所在目录；草稿（无路径）的
/// 相对 URL 无法解析；`http:`/`https:`/`data:`/`file:` 等带 scheme 的 URL v0 不加载。
/// Windows 盘符（`C:`）只 1 个字母，不误判为 scheme。
pub(crate) fn local_image_path(url: &str, doc: Option<&Path>) -> Option<PathBuf> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    if let Some((scheme, _)) = url.split_once(':') {
        if scheme.len() >= 2 && scheme.chars().all(|c| c.is_ascii_alphabetic()) {
            return None;
        }
    }
    let p = Path::new(url);
    if p.is_absolute() {
        return Some(p.to_path_buf());
    }
    Some(doc?.parent()?.join(p))
}

/// 图片显示尺寸：宽超版心按比例缩放（高同比），至少 1px。
pub(crate) fn fit_size(w: u32, h: u32, max_width: f32) -> (u32, u32) {
    let w = w.max(1) as f32;
    let h = h.max(1) as f32;
    let s = if w > max_width { max_width / w } else { 1.0 };
    (
        (w * s).round().max(1.0) as u32,
        (h * s).round().max(1.0) as u32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str = "| a | b |\n| --- | --- |\n| c | d |\n";
    const TABLE_2X2_ALIGNED: &str = "| a | b |\n| :-- | --: |\n| c | d |\n| e | f |\n";

    #[test]
    fn append_row_adds_empty_row_with_same_columns() {
        assert_eq!(table_append_row(TABLE), format!("{TABLE}|  |  |\n"));
        // 不足 header + 分隔行时不改动
        assert_eq!(table_append_row("| a |\n"), "| a |\n");
    }

    #[test]
    fn insert_row_above_and_below_keep_alignment() {
        assert_eq!(
            table_insert_row(TABLE_2X2_ALIGNED, 1, true),
            "| a | b |\n| :-- | --: |\n| c | d |\n|  |  |\n| e | f |\n"
        );
        assert_eq!(
            table_insert_row(TABLE_2X2_ALIGNED, 1, false),
            "| a | b |\n| :-- | --: |\n| c | d |\n| e | f |\n|  |  |\n"
        );
        // 分隔行与列数不变
        assert_eq!(
            table_insert_row(TABLE_2X2_ALIGNED, 0, true)
                .lines()
                .nth(1),
            Some("| :-- | --: |")
        );
    }

    #[test]
    fn insert_row_out_of_range_appends_to_tail() {
        assert_eq!(
            table_insert_row(TABLE_2X2_ALIGNED, 99, true),
            format!("{TABLE_2X2_ALIGNED}|  |  |\n")
        );
        assert_eq!(
            table_insert_row(TABLE_2X2_ALIGNED, 99, false),
            format!("{TABLE_2X2_ALIGNED}|  |  |\n")
        );
    }

    #[test]
    fn delete_row_and_last_body_row() {
        assert_eq!(
            table_delete_row(TABLE_2X2_ALIGNED, 0),
            "| a | b |\n| :-- | --: |\n| e | f |\n"
        );
        // 仅剩 1 行 body → 删成只有 header + 分隔行
        assert_eq!(table_delete_row(TABLE, 0), "| a | b |\n| --- | --- |\n");
        // 越界不改动
        assert_eq!(table_delete_row(TABLE, 9), TABLE);
    }

    #[test]
    fn insert_column_keeps_aligns_and_column_count() {
        let out = table_insert_column(TABLE_2X2_ALIGNED, 1);
        assert_eq!(
            out,
            "| a |  | b |\n| :-- | --- | --: |\n| c |  | d |\n| e |  | f |\n"
        );
        assert_eq!(
            parse_aligns(out.lines().nth(1).unwrap()),
            vec![
                veil_core::CellAlign::Left,
                veil_core::CellAlign::None,
                veil_core::CellAlign::Right
            ],
            "插入列不得改动既有列对齐"
        );
        // 追加到末尾
        assert_eq!(
            table_insert_column(TABLE, 9),
            "| a | b |  |\n| --- | --- | --- |\n| c | d |  |\n"
        );
    }

    #[test]
    fn delete_column_and_single_column_boundary() {
        assert_eq!(
            table_delete_column(TABLE_2X2_ALIGNED, 0),
            "| b |\n| --: |\n| d |\n| f |\n"
        );
        assert_eq!(
            table_delete_column(TABLE_2X2_ALIGNED, 1),
            "| a |\n| :-- |\n| c |\n| e |\n"
        );
        // 单列不删除
        assert_eq!(table_delete_column("| a |\n| --- |\n| b |\n", 0), "| a |\n| --- |\n| b |\n");
    }

    #[test]
    fn set_align_round_trips_and_leaves_other_bytes_untouched() {
        let src = "| a | b |\n| --- | --- |\n| c | d |\n";
        let left = table_set_align(src, 1, veil_core::CellAlign::Left).unwrap();
        assert_eq!(left, "| a | b |\n| --- | :-- |\n| c | d |\n");
        let center = table_set_align(&left, 1, veil_core::CellAlign::Center).unwrap();
        assert_eq!(center, "| a | b |\n| --- | :-: |\n| c | d |\n");
        let right = table_set_align(&center, 1, veil_core::CellAlign::Right).unwrap();
        assert_eq!(right, "| a | b |\n| --- | --: |\n| c | d |\n");
        assert_eq!(
            table_set_align(&right, 1, veil_core::CellAlign::None).unwrap(),
            src,
            "回到无对齐时与原文逐字节一致"
        );
        // 幂等：同值再设一次输出相等
        assert_eq!(table_set_align(&right, 1, veil_core::CellAlign::Right), Some(right));
    }

    #[test]
    fn set_align_keeps_crlf_and_wide_cell_padding() {
        let src = "| a | b |\r\n| :---: | ---- |\r\n| c | d |\r\n";
        let out = table_set_align(src, 1, veil_core::CellAlign::Center).unwrap();
        assert_eq!(out, "| a | b |\r\n| :---: | :--: |\r\n| c | d |\r\n");
        // 未选中列的 `:---:` 与该列宽度原样保留（总字节长度不变）
        assert_eq!(out.len(), src.len());
        assert_eq!(
            table_set_align(&out, 1, veil_core::CellAlign::None).unwrap(),
            src
        );
        // 窄于 3 的分隔格补齐到 3
        assert_eq!(
            table_set_align("| a |\n| - |\n| b |\n", 0, veil_core::CellAlign::Center).unwrap(),
            "| a |\n| :-: |\n| b |\n"
        );
        // 无外层竖线的单列表格
        assert_eq!(
            table_set_align("a\n---\nb\n", 0, veil_core::CellAlign::Right).unwrap(),
            "a\n--:\nb\n"
        );
    }

    #[test]
    fn set_align_out_of_range_and_non_delimiter_rows() {
        // 单列表格
        assert_eq!(
            table_set_align("| a |\n| --- |\n| b |\n", 0, veil_core::CellAlign::Left).unwrap(),
            "| a |\n| :-- |\n| b |\n"
        );
        // col 越界 / 无分隔行 / 第 2 行不是分隔格
        assert_eq!(table_set_align(TABLE, 2, veil_core::CellAlign::Left), None);
        assert_eq!(table_set_align("| a |\n", 0, veil_core::CellAlign::Left), None);
        assert_eq!(
            table_set_align("| a |\n| x |\n| b |\n", 0, veil_core::CellAlign::Left),
            None
        );
    }

    #[test]
    fn cell_start_skips_pipe_and_spaces() {
        assert_eq!(table_cell_start(TABLE, 0, 0), 2);
        assert_eq!(table_cell_start(TABLE, 0, 1), 6);
        assert_eq!(table_cell_start(TABLE, 1, 0), 12);
        assert_eq!(table_cell_start(TABLE, 2, 0), 26);
        assert_eq!(table_cell_start(TABLE, 2, 1), 30);
        // 无首管道符的行
        assert_eq!(table_cell_start("a | b\n--- | ---\n", 0, 0), 0);
        assert_eq!(table_cell_start("a | b\n--- | ---\n", 0, 1), 4);
        // 越界
        assert_eq!(table_cell_start(TABLE, 9, 0), 0);
        assert_eq!(table_cell_start(TABLE, 0, 9), 0);
    }

    #[test]
    fn cell_start_of_inserted_empty_row_matches_caret_convention() {
        let src = format!("{TABLE}|  |  |\n");
        // 空单元格内容起点 = 闭合管道符位置（与原 Ctrl+Enter 落点一致）
        assert_eq!(table_cell_start(&src, 3, 0), 37);
        assert_eq!(table_cell_start(&src, 3, 1), 40);
    }
}

