//! 帧合成：背景/块内容/表格网格/选区/光标与 preedit 绘制。所有输入为
//! 已排版的几何与缓存，本文件不触发重新排版（除 paint 开头的两遍收敛）。

use crate::app::App;
use crate::chrome;
use crate::geometry::{BASE_LINE_HEIGHT, MARGIN};
use crate::view::{
    node_active, outline, oversize_notice, search, BlockLayout, Frame, InlineAsset, InlineImg,
    LayoutKey, Memo,
};
use std::path::{Path, PathBuf};
use veil_core::{BlockId, BlockKind, InlineKind, InlineNode, Range};

impl App {
    /// 补齐缺失布局（键命中则零开销）。Reveal 激活已并入布局键，光标移动后
    /// 指纹失配即自动重排，无需另设失效调用（ADR-009 决策四）。
    /// 行内公式（切片 8）：块内非激活 Math 节点渲染出图（tex 内容寻址缓存），
    /// 传占位信息给 layout；失败记 inline_failed 降级源码显示，不每帧重试。
    /// 文档内图片（切片 13）：非激活 Image 节点按文档目录解析相对 URL，经 ImageLoader
    /// 加载自然尺寸位图；宽超版心等比缩放后占位；失败/远程 URL 降级 alt 文本。
    pub(crate) fn ensure_layouts_range(&mut self, indices: std::ops::Range<usize>) {
        let width = self.content_width();
        let caret = self.editor.caret();
        let base = self.base_key();
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
            let l =
                self.engine
                    .layout_block(kind.clone(), &text, range, &inlines, width, caret, &imgs);
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
    pub(crate) fn ensure_math_range(&mut self, indices: std::ops::Range<usize>) {
        let caret = self.editor.caret();
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
        let caret = self.editor.caret();
        !(range.start <= caret && caret <= range.end)
            && self.cache.math(id, self.math_key(id)).is_some()
    }

    /// 表格按需网格排版（光标在表格内为源码态，不排版网格；离开后首次绘制时补齐）。
    /// 网格缓存与文本布局缓存独立：网格仅供渲染/命中，源码态仍走既有块排版。
    pub(crate) fn ensure_tables_range(&mut self, indices: std::ops::Range<usize>) {
        let width = self.content_width();
        let caret = self.editor.caret();
        let fg = self.input_state.theme.fg;
        let base = self.base_key();
        let Some(blocks) = self.index.blocks.get(indices) else {
            return;
        };
        let mut pending = Vec::new();
        for g in blocks {
            let (id, kind, range) = (g.id, g.kind.clone(), g.range);
            if kind != BlockKind::Table
                || self.cache.table(id, base).is_some()
                || (range.start <= caret && caret <= range.end)
            {
                continue;
            }
            if let Some(t) = self.editor.session().table(id) {
                pending.push((id, t));
            }
        }
        for (id, t) in pending {
            let l = self.engine.layout_table(&t, width, fg);
            self.cache.tables.insert(
                id,
                Memo {
                    key: base,
                    value: l,
                },
            );
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

    /// 表格当前是否按网格显示：光标在表格内（Reveal 同语义）或未排版时显示源码。
    pub(crate) fn table_shows_grid(&self, id: BlockId, range: &veil_core::Range) -> bool {
        let caret = self.editor.caret();
        !(range.start <= caret && caret <= range.end)
            && self.cache.table(id, self.base_key()).is_some()
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
        let caret = self.editor.caret();
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
        // 窗口缩放/文档变短后滚动量可能越界：绘制前统一钳位（滚动条与内容同源）
        self.window_state.scroll_y = self
            .window_state
            .scroll_y
            .clamp(0.0, chrome::max_scroll(vp, self.doc_height()));
        let sel = self.editor.selection();
        // 只遍历视口覆盖的块（含 overscan），不再逐块判定后 continue
        let vis = self.visible_index_range();
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
            if matches!(kind, BlockKind::FencedCode(_) | BlockKind::IndentedCode) {
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
            // 查找命中高亮（P3-3）：全部命中用 hover 色，当前命中用选区色
            self.paint_match_highlights(&mut frame, l, &range, sy);
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
        // 超限块降级提示：块高已含提示行，画在文本之后、光标/选区之上
        for (sy, bh, full) in oversize {
            let notice = oversize_notice(full);
            let mid = sy as f32 + bh - BASE_LINE_HEIGHT * scale / 2.0;
            if let Some(l) = self
                .ui_state
                .chrome
                .labels
                .get_or_insert(&mut self.engine, &notice, theme.muted, scale)
            {
                frame.blit(
                    &l.pixels,
                    l.w,
                    l.h,
                    (MARGIN * scale).round() as i32,
                    (mid - l.h as f32 / 2.0).round() as i32,
                );
            }
        }
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
        // chrome 最后绘制（不透明覆盖内容；滚动条区保留给状态栏行列）
        let recent = self.recent_labels();
        self.ui_state.chrome.paint(
            &mut frame,
            &mut self.engine,
            vp,
            scale,
            &theme,
            self.doc.path.as_deref(),
            dirty,
            external,
            line,
            col,
            doc_h,
            self.window_state.scroll_y,
            &recent,
        );
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
        let Some(surface) = &mut self.window_state.surface else {
            return;
        };
        if let Ok(mut target) = surface.buffer_mut() {
            target.copy_from_slice(&frame.pixels);
            let _ = target.present();
        }
    }

    /// 帧合成完整路径：render → present。产品层事件循环 Redraw 分支唯一调用。
    pub(crate) fn paint(&mut self) {
        let _ = self.render();
        self.present();
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
        let mut qw = 0.0;
        if search.query.is_empty() {
            if self.editor.preedit().is_none() {
                if let Some(lab) = self.ui_state.chrome.labels.get_or_insert(
                    &mut self.engine,
                    "查找",
                    theme.muted,
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
        // IME 组合串：贴在查询文本之后（贴右缘钳位），下划线同文档 preedit；
        // 替换模式下焦点在替换框时由替换行负责绘制
        let focus = search.focus;
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

    /// 绘制表格网格：表头底色 → 单元格文本 → 网格线（先底后字，线压边）。
    /// 选区跨网格态表格时不画高亮（同数学块 v0 已知简化）。
    pub(crate) fn paint_table(&self, frame: &mut Frame, id: BlockId, top: i32) {
        let Some(t) = self.cache.table(id, self.base_key()) else {
            return;
        };
        let x0 = (MARGIN * self.input_state.scale) as i32;
        let theme = self.input_state.theme;
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
        if cy < vp.y {
            self.window_state.scroll_y += cy - vp.y - MARGIN * self.input_state.scale;
        } else if cy + ch > vp.y + vp.h {
            self.window_state.scroll_y += cy + ch - (vp.y + vp.h) + MARGIN * self.input_state.scale;
        }
        self.window_state.scroll_y = self
            .window_state
            .scroll_y
            .clamp(0.0, chrome::max_scroll(vp, self.doc_height()));
    }
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
