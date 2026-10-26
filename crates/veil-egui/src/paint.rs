//! 帧合成：背景/块内容/表格网格/选区/光标与 preedit 绘制。所有输入为
//! 已排版的几何与缓存，本文件不触发重新排版（除 paint 开头的两遍收敛）。

use crate::app::{App, MARGIN};
use crate::chrome;
use crate::view::{node_active, Frame, InlineAsset, InlineImg};
use std::path::{Path, PathBuf};
use veil_core::{BlockId, BlockKind, InlineKind, InlineNode, Range};

impl App {
    /// 补齐缺失布局（干净块缓存命中则零开销）。Reveal 态以当前光标为准：
    /// 光标移动引发的激活变化由 invalidate_reveal 先行失效相关块。
    /// 行内公式（切片 8）：块内非激活 Math 节点渲染出图（tex 内容寻址缓存），
    /// 传占位信息给 layout；失败记 inline_failed 降级源码显示，不每帧重试。
    /// 文档内图片（切片 13）：非激活 Image 节点按文档目录解析相对 URL，经 ImageLoader
    /// 加载自然尺寸位图；宽超版心等比缩放后占位；失败/远程 URL 降级 alt 文本。
    pub(crate) fn ensure_layouts_range(&mut self, indices: std::ops::Range<usize>) {
        let width = self.content_width();
        let caret = self.editor.caret();
        self.engine.set_scale(self.scale);
        let Some(blocks) = self.index.blocks.get(indices) else {
            return;
        };
        // 先收集待排块与其行内公式/图片（session 借用期），再调渲染器/加载器（&mut self）
        let mut pending = Vec::new();
        for g in blocks {
            let (id, kind, range) = (g.id, g.kind, g.range);
            if self.cache.layouts.contains_key(&id) {
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
        let scale = self.scale;
        let fg = self.theme.fg;
        for (id, kind, range, text, inlines, maths, images) in pending {
            let mut imgs = Vec::new();
            for (nrange, tex) in maths {
                if self.cache.inline_failed.contains(&tex) {
                    continue; // 失败记忆：降级源码（Reveal 消隐态）显示
                }
                if !self.cache.inline_math.contains_key(&tex) {
                    match self.math.render(&tex, false, scale, fg) {
                        Ok(img) => {
                            self.cache.inline_math.insert(tex.clone(), img);
                        }
                        Err(e) => {
                            eprintln!("行内公式渲染失败（降级为源码显示）: {e}");
                            self.cache.inline_failed.insert(tex.clone());
                            continue;
                        }
                    }
                }
                let img = &self.cache.inline_math[&tex];
                imgs.push(InlineImg {
                    range: nrange,
                    key: InlineAsset::Math(tex),
                    width: img.width,
                    height: img.height,
                    baseline: img.baseline,
                });
            }
            for (nrange, url) in images {
                let Some(path) = local_image_path(&url, self.path.as_deref()) else {
                    continue; // 远程/内联 URL 或草稿相对路径：降级 alt 文本
                };
                let key = path.to_string_lossy().into_owned();
                if self.cache.image_failed.contains(&key) {
                    continue;
                }
                if !self.cache.inline_images.contains_key(&key) {
                    match self.images.load(&path) {
                        Ok(img) => {
                            self.cache.inline_images.insert(key.clone(), img);
                        }
                        Err(e) => {
                            eprintln!("图片加载失败（降级为 alt 文本） {}: {e}", path.display());
                            self.cache.image_failed.insert(key);
                            continue;
                        }
                    }
                }
                let nat = &self.cache.inline_images[&key];
                let (dw, dh) = fit_size(nat.width, nat.height, width);
                imgs.push(InlineImg {
                    range: nrange,
                    key: InlineAsset::Image(key),
                    width: dw,
                    height: dh,
                    baseline: dh, // 底边贴基线
                });
            }
            let l = self
                .engine
                .layout_block(kind, &text, range, &inlines, width, caret, &imgs);
            self.cache.layouts.insert(id, l);
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
        let lo = self.scroll_y - vp.h;
        let hi = self.scroll_y + 2.0 * vp.h;
        let blocks = &self.index.blocks;
        let start = blocks.partition_point(|g| g.top + g.height < lo);
        let end = blocks.partition_point(|g| g.top < hi);
        start.min(blocks.len())..end.max(start).min(blocks.len())
    }


    /// 光标在块内为源码态不渲染；失败记入 math_failed 不再每帧重试（降级源码显示）。
    pub(crate) fn ensure_math_range(&mut self, indices: std::ops::Range<usize>) {
        let caret = self.editor.caret();
        let scale = self.scale;
        let fg = self.theme.fg;
        let Some(blocks) = self.index.blocks.get(indices) else {
            return;
        };
        for g in blocks {
            let (id, kind, range) = (g.id, g.kind, g.range);
            if kind != BlockKind::Math
                || (range.start <= caret && caret <= range.end)
                || self.cache.math_imgs.contains_key(&id)
                || self.cache.math_failed.contains(&id)
            {
                continue;
            }
            let Some(tex) = self.editor.session().math_source(id) else {
                continue;
            };
            match self.math.render(&tex, true, scale, fg) {
                Ok(img) => {
                    self.cache.math_imgs.insert(id, img);
                }
                Err(e) => {
                    eprintln!("公式渲染失败（降级为源码显示）: {e}");
                    self.cache.math_failed.insert(id);
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
        !(range.start <= caret && caret <= range.end) && self.cache.math_imgs.contains_key(&id)
    }

    /// 表格按需网格排版（光标在表格内为源码态，不排版网格；离开后首次绘制时补齐）。
    /// 网格缓存与文本布局缓存独立：网格仅供渲染/命中，源码态仍走既有块排版。
    pub(crate) fn ensure_tables_range(&mut self, indices: std::ops::Range<usize>) {
        let width = self.content_width();
        let caret = self.editor.caret();
        let fg = self.theme.fg;
        let Some(blocks) = self.index.blocks.get(indices) else {
            return;
        };
        let mut pending = Vec::new();
        for g in blocks {
            let (id, kind, range) = (g.id, g.kind, g.range);
            if kind != BlockKind::Table
                || self.cache.tables.contains_key(&id)
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
            self.cache.tables.insert(id, l);
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
        !(range.start <= caret && caret <= range.end) && self.cache.tables.contains_key(&id)
    }


    pub(crate) fn paint(&mut self) {
        let (w, h) = match self.window.as_ref() {
            Some(win) => {
                let s = win.inner_size();
                (s.width.max(1), s.height.max(1))
            }
            None => return,
        };
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
        if self
            .index
            .blocks
            .get(range)
            .is_some_and(|bs| bs.iter().any(|g| !self.cache.layouts.contains_key(&g.id)))
        {
            self.ensure_layouts_visible();
            self.ensure_math_visible();
            self.ensure_tables_visible();
            self.recompute_geometry();
        }
        // 帧缓冲跨帧复用：仅尺寸变化才重新分配（每帧只做一次底色填充）
        let mut frame = self.frame.take().unwrap_or_else(|| Frame::new(w, h, self.theme.bg));
        frame.reset(w, h, self.theme.bg);
        let scale = self.scale;
        let theme = self.theme;
        let vp = self.viewport();
        // 窗口缩放/文档变短后滚动量可能越界：绘制前统一钳位（滚动条与内容同源）
        self.scroll_y = self
            .scroll_y
            .clamp(0.0, chrome::max_scroll(vp, self.doc_height()));
        let sel = self.editor.selection();
        let caret = self.editor.caret();
        // 只遍历视口覆盖的块（含 overscan），不再逐块判定后 continue
        let vis = self.visible_index_range();
        for g in &self.index.blocks[vis] {
            let (id, kind, range, top) = (g.id, g.kind, g.range, g.top);
            let sy = (vp.y + top - self.scroll_y) as i32;
            if (sy as f32) >= vp.y + vp.h
                || sy + self.block_height(g) as i32 <= vp.y as i32
            {
                continue; // 视口外不画（布局已缓存，光栅按需）
            }
            if self.math_shows_image(id, &range) {
                // 数学块渲染态：图整体替换文本光栅（display 公式居中）
                let img = &self.cache.math_imgs[&id];
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
            let Some(l) = self.cache.layouts.get(&id) else {
                continue;
            };
            if !self.cache.rasters.contains_key(&id) {
                let r = self.engine.rasterize(l, theme.fg);
                self.cache.rasters.insert(id, r);
            }
            if matches!(kind, BlockKind::FencedCode) {
                let (rw, rh) = (self.cache.rasters[&id].1, self.cache.rasters[&id].2);
                frame.fill_rect(
                    (MARGIN * scale) as i32 - 4,
                    sy - 2,
                    rw + 8,
                    rh + 4,
                    theme.code_bg,
                );
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
            let (px, rw, rh) = &self.cache.rasters[&id];
            frame.blit(px, *rw, *rh, (MARGIN * scale) as i32, sy);
            // 行内公式/图片：位置在排版期已定（PlacedImg），叠在文本光栅之上。
            // 图片按占位显示尺寸最近邻缩放（缓存保留自然尺寸，不生成缩放副本）。
            for p in &l.imgs {
                let dx = (MARGIN * scale + p.x) as i32;
                let dy = sy + p.y as i32;
                match &p.key {
                    InlineAsset::Math(tex) => {
                        if let Some(img) = self.cache.inline_math.get(tex) {
                            frame.blit(&img.pixels, img.width, img.height, dx, dy);
                        }
                    }
                    InlineAsset::Image(key) => {
                        if let Some(img) = self.cache.inline_images.get(key) {
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
        }
        let (line, col) = self.editor.session().line_col(caret).unwrap_or((1, 1));
        let dirty = self.editor.is_dirty();
        let external = self.external_state();
        let doc_h = self.doc_height();
        self.paint_caret_and_preedit(&mut frame);
        // chrome 最后绘制（不透明覆盖内容；滚动条区保留给状态栏行列）
        self.chrome.paint(
            &mut frame,
            &mut self.engine,
            vp,
            scale,
            &theme,
            self.path.as_deref(),
            dirty,
            external,
            line,
            col,
            doc_h,
            self.scroll_y,
        );
        if let Some(surface) = &mut self.surface {
            // 取缓冲失败属瞬时状态：丢帧不崩溃，下一帧自愈
            if let Ok(mut target) = surface.buffer_mut() {
                target.copy_from_slice(&frame.pixels);
                let _ = target.present();
            }
        }
        self.frame = Some(frame); // 归还缓冲供下一帧复用
    }

    /// 绘制表格网格：表头底色 → 单元格文本 → 网格线（先底后字，线压边）。
    /// 选区跨网格态表格时不画高亮（同数学块 v0 已知简化）。
    pub(crate) fn paint_table(&self, frame: &mut Frame, id: BlockId, top: i32) {
        let Some(t) = self.cache.tables.get(&id) else {
            return;
        };
        let x0 = (MARGIN * self.scale) as i32;
        let theme = self.theme;
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
        let fg = self.theme.fg;
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
            self.scroll_y += cy - vp.y - MARGIN * self.scale;
        } else if cy + ch > vp.y + vp.h {
            self.scroll_y += cy + ch - (vp.y + vp.h) + MARGIN * self.scale;
        }
        self.scroll_y = self
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
