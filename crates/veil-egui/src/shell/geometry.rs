//! 块几何与派生缓存：块清单索引、估算/真实高度、累计 top、视口区间、
//! 文本/行内公式/表格的按需排版与失效。绘制与交互不在此文件。

use super::paint::{fit_size, local_image_path};
use super::*;
use crate::layout::{InlineAsset, InlineImg, BASE_FONT_SIZE};
use veil_core::{InlineKind, Range};

/// 单块几何缓存：块清单一次枚举后复用，避免每次击键反复全量枚举（P1a）。
pub(super) struct BlockGeom {
    pub(super) id: BlockId,
    pub(super) kind: BlockKind,
    pub(super) range: Range,
    pub(super) top: f32,
    pub(super) height: f32,
    /// 估算高度（P1b）：refresh_index 时按换行数与折行字符数廉价估算，
    /// 视口外块以它参与几何累计，几何重算不再读取 rope。
    pub(super) est_h: f32,
}

/// 文档几何索引：`blocks` 与 `total_h` 只在本层刷新/重算后读取。
#[derive(Default)]
pub(super) struct BlockIndex {
    pub(super) blocks: Vec<BlockGeom>,
    /// 文档总高（含首尾外边距），即原 `doc_height()` 语义。
    pub(super) total_h: f32,
}

impl App {
    pub(super) fn content_width(&self) -> f32 {
        let (w, _) = self.win_size();
        (w as f32 - 2.0 * MARGIN * self.scale - chrome::SCROLL_W * self.scale).max(32.0)
    }

    /// 文档总高（含首尾外边距），滚动条与滚轮钳位依据。
    pub(super) fn doc_height(&self) -> f32 {
        if self.index.blocks.is_empty() {
            2.0 * MARGIN * self.scale
        } else {
            self.index.total_h
        }
    }

    /// 补齐缺失布局（干净块缓存命中则零开销）。Reveal 态以当前光标为准：
    /// 光标移动引发的激活变化由 invalidate_reveal 先行失效相关块。
    /// 行内公式（切片 8）：块内非激活 Math 节点渲染出图（tex 内容寻址缓存），
    /// 传占位信息给 layout；失败记 inline_failed 降级源码显示，不每帧重试。
    /// 文档内图片（切片 13）：非激活 Image 节点按文档目录解析相对 URL，经 ImageLoader
    /// 加载自然尺寸位图；宽超版心等比缩放后占位；失败/远程 URL 降级 alt 文本。
    pub(super) fn ensure_layouts_range(&mut self, indices: std::ops::Range<usize>) {
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
            if self.layouts.contains_key(&id) {
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
                if self.inline_failed.contains(&tex) {
                    continue; // 失败记忆：降级源码（Reveal 消隐态）显示
                }
                if !self.inline_math.contains_key(&tex) {
                    match self.math.render(&tex, false, scale, fg) {
                        Ok(img) => {
                            self.inline_math.insert(tex.clone(), img);
                        }
                        Err(e) => {
                            eprintln!("行内公式渲染失败（降级为源码显示）: {e}");
                            self.inline_failed.insert(tex.clone());
                            continue;
                        }
                    }
                }
                let img = &self.inline_math[&tex];
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
                if self.image_failed.contains(&key) {
                    continue;
                }
                if !self.inline_images.contains_key(&key) {
                    match self.images.load(&path) {
                        Ok(img) => {
                            self.inline_images.insert(key.clone(), img);
                        }
                        Err(e) => {
                            eprintln!("图片加载失败（降级为 alt 文本） {}: {e}", path.display());
                            self.image_failed.insert(key);
                            continue;
                        }
                    }
                }
                let nat = &self.inline_images[&key];
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
            self.layouts.insert(id, l);
        }
    }

    /// 仅补齐视口内（含上下各一屏 overscan）缺失布局（P1b 惰性排版）。
    pub(super) fn ensure_layouts_visible(&mut self) {
        let range = self.visible_index_range();
        self.ensure_layouts_range(range);
    }

    /// 测试专用：等价全量排版；不参与非测试构建，避免 dead_code。
    #[cfg(test)]
    pub(super) fn ensure_layouts_all(&mut self) {
        self.ensure_layouts_range(0..self.index.blocks.len());
    }

    /// 按需排版单块（含其数学渲染/表格网格）：光标、命中、垂直移动等路径使用。
    pub(super) fn ensure_block(&mut self, id: BlockId) {
        let Some(i) = self.index.blocks.iter().position(|g| g.id == id) else {
            return;
        };
        self.ensure_layouts_range(i..i + 1);
        self.ensure_math_range(i..i + 1);
        self.ensure_tables_range(i..i + 1);
    }

    /// 视口覆盖的块索引区间（含上下各一屏 overscan）：`top` 已由几何重算保证
    /// 单调不减，二分定位首尾，避免逐块线性扫描。
    pub(super) fn visible_index_range(&self) -> std::ops::Range<usize> {
        let vp = self.viewport();
        let lo = self.scroll_y - vp.h;
        let hi = self.scroll_y + 2.0 * vp.h;
        let blocks = &self.index.blocks;
        let start = blocks.partition_point(|g| g.top + g.height < lo);
        let end = blocks.partition_point(|g| g.top < hi);
        start.min(blocks.len())..end.max(start).min(blocks.len())
    }

    /// 光标移动引起的 Reveal 状态变化：仅旧/新光标所在块的激活态可能改变。
    pub(super) fn invalidate_reveal(&mut self, old: usize, new: usize) {
        if old == new {
            return;
        }
        for g in &self.index.blocks {
            let (id, kind, range) = (g.id, g.kind, g.range);
            let touches = |p: usize| range.start <= p && p <= range.end;
            if !touches(old) && !touches(new) {
                continue;
            }
            // 块级 Reveal（切片 12）：列表项/分隔线按块激活（进/出块才变）；
            // 引用块逐行激活（块内跨行移动亦变，行内移动不变）。
            let block_reveal = match kind {
                BlockKind::ListItem { .. } | BlockKind::ThematicBreak => {
                    touches(old) != touches(new)
                }
                BlockKind::BlockQuote => {
                    let text = self.editor.session().text_slice(range).unwrap_or_default();
                    quote_line_of(&text, &range, old) != quote_line_of(&text, &range, new)
                }
                _ => false,
            };
            let inlines = self.editor.session().block_inlines(id).unwrap_or_default();
            if block_reveal || inlines.iter().any(|n| activation_changed(n, old, new)) {
                self.layouts.remove(&id);
                self.rasters.remove(&id);
            }
        }
    }

    /// 数学块按需渲染（同步 v0：spike 实测单公式 <1ms、百公式 21ms，预算内，ADR-002）。
    /// 光标在块内为源码态不渲染；失败记入 math_failed 不再每帧重试（降级源码显示）。
    pub(super) fn ensure_math_range(&mut self, indices: std::ops::Range<usize>) {
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
                || self.math_imgs.contains_key(&id)
                || self.math_failed.contains(&id)
            {
                continue;
            }
            let Some(tex) = self.editor.session().math_source(id) else {
                continue;
            };
            match self.math.render(&tex, true, scale, fg) {
                Ok(img) => {
                    self.math_imgs.insert(id, img);
                }
                Err(e) => {
                    eprintln!("公式渲染失败（降级为源码显示）: {e}");
                    self.math_failed.insert(id);
                }
            }
        }
    }

    /// 仅渲染视口内（含 overscan）的数学块。
    pub(super) fn ensure_math_visible(&mut self) {
        let range = self.visible_index_range();
        self.ensure_math_range(range);
    }

    /// 测试专用：全量数学渲染。
    #[cfg(test)]
    pub(super) fn ensure_math(&mut self) {
        self.ensure_math_range(0..self.index.blocks.len());
    }

    /// 数学块当前是否按渲染图显示：光标在块内（Reveal 同语义）或未渲染时显示源码。
    pub(super) fn math_shows_image(&self, id: BlockId, range: &veil_core::Range) -> bool {
        let caret = self.editor.caret();
        !(range.start <= caret && caret <= range.end) && self.math_imgs.contains_key(&id)
    }

    /// 表格按需网格排版（光标在表格内为源码态，不排版网格；离开后首次绘制时补齐）。
    /// 网格缓存与文本布局缓存独立：网格仅供渲染/命中，源码态仍走既有块排版。
    pub(super) fn ensure_tables_range(&mut self, indices: std::ops::Range<usize>) {
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
                || self.tables.contains_key(&id)
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
            self.tables.insert(id, l);
        }
    }

    /// 仅排版视口内（含 overscan）的表格网格。
    pub(super) fn ensure_tables_visible(&mut self) {
        let range = self.visible_index_range();
        self.ensure_tables_range(range);
    }

    /// 测试专用：全量表格排版。
    #[cfg(test)]
    pub(super) fn ensure_tables(&mut self) {
        self.ensure_tables_range(0..self.index.blocks.len());
    }

    /// 表格当前是否按网格显示：光标在表格内（Reveal 同语义）或未排版时显示源码。
    pub(super) fn table_shows_grid(&self, id: BlockId, range: &veil_core::Range) -> bool {
        let caret = self.editor.caret();
        !(range.start <= caret && caret <= range.end) && self.tables.contains_key(&id)
    }

    /// 块显示高度：数学块渲染态取图高、表格网格态取网格高，其余取文本布局高；
    /// 文本布局缺失（视口外未排版）时回退廉价估算高度。
    pub(super) fn block_height(&self, g: &BlockGeom) -> f32 {
        if g.kind == BlockKind::Math && self.math_shows_image(g.id, &g.range) {
            return self.math_imgs[&g.id].height as f32;
        }
        if g.kind == BlockKind::Table && self.table_shows_grid(g.id, &g.range) {
            return self.tables[&g.id].height;
        }
        self.layouts
            .get(&g.id)
            .map(|l| l.height)
            .unwrap_or(g.est_h)
    }

    /// 重建块清单：生产代码唯一允许全量枚举块的位置；其余读取一律走索引。
    /// 估算高度在此一次性算好（P1b）：几何重算不再读取 rope。
    pub(super) fn refresh_index(&mut self) {
        let blocks = self.editor.session().blocks();
        let scale = self.scale;
        let chars_per_line = self.chars_per_line();
        self.index.blocks.clear();
        self.index.blocks.reserve(blocks.len());
        for (id, kind, range) in blocks {
            let text = self.editor.session().text_slice(range).unwrap_or_default();
            let est_h = estimate_height(&text, chars_per_line, scale);
            self.index.blocks.push(BlockGeom {
                id,
                kind,
                range,
                top: 0.0,
                height: 0.0,
                est_h,
            });
        }
        // 索引重建后 top/height 归零：立即按估算高度累计一次，可见区间/滚动条
        // 才有有效几何；排版后用 recompute_geometry 以真实高度覆盖。
        self.recompute_geometry();
    }

    /// 每视觉行可容纳的字符数（按平均字宽 0.5em 估算，仅用于便宜高度）。
    pub(super) fn chars_per_line(&self) -> f32 {
        (self.content_width() / (0.5 * BASE_FONT_SIZE * self.scale)).max(1.0)
    }

    /// 按当前布局/公式/表格缓存重算高度与累计 top（不再枚举块）。
    /// 取出 Vec 再循环：`block_height(&self)` 与 `index` 的可变借用无法同时成立。
    pub(super) fn recompute_geometry(&mut self) {
        let mut blocks = std::mem::take(&mut self.index.blocks);
        let mut y = MARGIN * self.scale;
        for g in &mut blocks {
            let h = self.block_height(g);
            g.height = h;
            g.top = y;
            y += h + PARA_GAP * self.scale;
        }
        // 末块后不累加块间距（与原 block_tops 一致），文档尾部只留外边距
        self.index.total_h = if blocks.is_empty() {
            2.0 * MARGIN * self.scale
        } else {
            y - PARA_GAP * self.scale + MARGIN * self.scale
        };
        self.index.blocks = blocks;
    }

    /// 光标所在块索引；文末光标可能落在最后一块的尾换行之后，回退末块。
    pub(super) fn caret_block_index(&self) -> Option<usize> {
        let caret = self.editor.caret();
        self.index
            .blocks
            .iter()
            .position(|g| g.range.start <= caret && caret <= g.range.end)
            .or_else(|| {
                if caret >= self.editor.session().text_len() {
                    self.index.blocks.len().checked_sub(1)
                } else {
                    None
                }
            })
    }

    /// 光标屏幕矩形（物理像素，含菜单栏偏移）。(x, y, h)；找不到所在块（空文档）时落视口原点。
    pub(super) fn caret_screen(&mut self) -> (f32, f32, f32) {
        let caret = self.editor.caret();
        let vp = self.viewport();
        if let Some(i) = self.caret_block_index() {
            let (id, start, top) = {
                let g = &self.index.blocks[i];
                (g.id, g.range.start, g.top)
            };
            // 视口外块可能尚未布局：光标路径按需补齐
            self.ensure_block(id);
            if let Some(l) = self.layouts.get(&id) {
                let (x, y, h) = self.engine.caret_xy(l, caret - start);
                return (MARGIN * self.scale + x, vp.y + top + y - self.scroll_y, h);
            }
        }
        (
            MARGIN * self.scale,
            vp.y + MARGIN * self.scale - self.scroll_y,
            BASE_LINE_HEIGHT * self.scale,
        )
    }
}

/// 视口外块的廉价高度估算（P1b）：换行数与原文字符数折算的折行数取大，
/// 乘以基准行高；不触发 shaping，仅用于几何累计。
fn estimate_height(text: &str, chars_per_line: f32, scale: f32) -> f32 {
    let hard_lines = text.bytes().filter(|&b| b == b'\n').count() as f32 + 1.0;
    let chars = text.chars().count() as f32;
    let wrapped = (chars / chars_per_line.max(1.0)).ceil().max(1.0);
    hard_lines.max(wrapped) * BASE_LINE_HEIGHT * scale
}
