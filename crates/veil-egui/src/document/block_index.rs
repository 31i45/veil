//! 文档的几何投影：块清单索引（id/kind/range）、估算高度与累计 top 的增量维护。
//! 本文件是生产代码唯一允许全量枚举块的位置；其余读取一律走本索引。
//! 不依赖 cosmic-text/像素概念，属文档域而非排版细节。

use veil_core::{BlockId, BlockKind, Range};

use crate::shell::{App, MARGIN, PARA_GAP};
use crate::view::BASE_LINE_HEIGHT;

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
}

/// 文档几何索引：`blocks` 与 `total_h` 只在本层刷新/重算后读取。
#[derive(Default)]
pub(crate) struct BlockIndex {
    pub(crate) blocks: Vec<BlockGeom>,
    /// 文档总高（含首尾外边距），即原 `doc_height()` 语义。
    pub(crate) total_h: f32,
}

impl App {
    /// 重建块清单：生产代码唯一允许全量枚举块的位置；其余读取一律走索引。
    /// 估算高度在此一次性算好（P1b）：几何重算不再读取 rope。
    pub(crate) fn refresh_index(&mut self) {
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

    /// 按当前布局/公式/表格缓存重算高度与累计 top（不再枚举块）。
    /// 取出 Vec 再循环：`block_height(&self)` 与 `index` 的可变借用无法同时成立。
    pub(crate) fn recompute_geometry(&mut self) {
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
}

/// 视口外块的廉价高度估算（P1b）：换行数与原文字符数折算的折行数取大，
/// 乘以基准行高；不触发 shaping，仅用于几何累计。
fn estimate_height(text: &str, chars_per_line: f32, scale: f32) -> f32 {
    let hard_lines = text.bytes().filter(|&b| b == b'\n').count() as f32 + 1.0;
    let chars = text.chars().count() as f32;
    let wrapped = (chars / chars_per_line.max(1.0)).ceil().max(1.0);
    hard_lines.max(wrapped) * BASE_LINE_HEIGHT * scale
}
