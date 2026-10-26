//! 帧合成：背景/块内容/表格网格/选区/光标与 preedit 绘制。所有输入为
//! 已排版的几何与缓存，本文件不触发重新排版（除 paint 开头的两遍收敛）。

use super::*;
use crate::view::InlineAsset;

impl App {
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
