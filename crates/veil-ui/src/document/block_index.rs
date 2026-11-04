//! 文档的几何投影：块清单索引（id/kind/range）、估算高度与累计 top 的增量维护。
//! 本文件是生产代码唯一允许全量枚举块的位置；其余读取一律走本索引。
//! 不依赖 cosmic-text/像素概念，属文档域而非排版细节。

use std::collections::HashSet;

use veil_core::{BlockId, BlockKind, Range, Session};

use crate::app::App;
use crate::geometry::{BASE_LINE_HEIGHT, MARGIN, PARA_GAP};

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
        // 索引重建后 top/height 归零：立即按估算高度累计一次，可见区间/滚动条
        // 才有有效几何；排版后用 recompute_geometry 以真实高度覆盖。
        self.recompute_geometry();
        // 全文统计与块结构同源重建：状态栏字数不可能落后于真值
        self.refresh_stats();
    }

    /// 步骤 1：全量枚举块（含 `session.blocks()` 内部的 line→byte 换算）。
    pub(crate) fn blocks_enum(&self) -> Vec<(BlockId, BlockKind, Range)> {
        self.editor.session().blocks()
    }

    /// 步骤 2：填充 `BlockGeom` 向量并回收缓存（除 `blocks_enum` 外的循环体）。
    ///
    /// `est_h` 只依赖块内字节长度与块内行数；同一 `BlockId` 的块内容未变时二者不变
    ///（编辑点后的块只整体平移，字节长度不变）。块数未变时新旧清单逐位对应，
    /// 同 id/种类/字节长度的块直接复用上一轮 `est_h`，免去每块两次 `line_of_offset`；
    /// 块数变化（换行/撤销等）退化为全量估算，语义不变。
    /// 真实高度仍由紧随其后的 `recompute_geometry` 按布局缓存覆盖（与改动前逐块
    /// 探测缓存等价：命中的块在 recompute 中同样命中，未命中的块两处都回退估算）。
    pub(crate) fn geom_fill(&mut self, blocks: Vec<(BlockId, BlockKind, Range)>) {
        let scale = self.input_state.scale;
        let chars_per_line = self.chars_per_line();
        let base = self.base_key();
        let rev = self.input_state.revs.session;
        let can_reuse = self.index.est_scale == scale
            && self.index.est_cpl == chars_per_line
            && self.index.est_rev == rev
            && self.index.blocks.len() == blocks.len();
        let prev = std::mem::take(&mut self.index.blocks);
        let mut alive: HashSet<BlockId> = HashSet::with_capacity(blocks.len());
        let mut next: Vec<BlockGeom> = Vec::with_capacity(blocks.len());
        for (i, (id, kind, range)) in blocks.into_iter().enumerate() {
            alive.insert(id);
            let est_h = match prev.get(i) {
                Some(p) if can_reuse
                    && p.id == id
                    && p.kind == kind
                    && p.range.end - p.range.start == range.end - range.start =>
                {
                    p.est_h
                }
                _ => estimate_h(self.editor.session(), range, chars_per_line, scale),
            };
            next.push(BlockGeom {
                id,
                kind,
                range,
                top: 0.0,
                height: 0.0,
                est_h,
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
        let mut blocks = std::mem::take(&mut self.index.blocks);
        let mut y = MARGIN * self.input_state.scale;
        for g in &mut blocks {
            let h = self.block_height(g);
            g.height = h;
            g.top = y;
            y += h + PARA_GAP * self.input_state.scale;
        }
        // 末块后不累加块间距（与原 block_tops 一致），文档尾部只留外边距
        self.index.total_h = if blocks.is_empty() {
            2.0 * MARGIN * self.input_state.scale
        } else {
            y - PARA_GAP * self.input_state.scale + MARGIN * self.input_state.scale
        };
        self.index.blocks = blocks;
    }
}

/// 廉价估算高度：块内行数 × 折行数取大（不切片、不建 String）。
/// 仅依赖 `range`（块内行数与字节长度）与字符宽度/缩放。
fn estimate_h(session: &Session, range: Range, chars_per_line: f32, scale: f32) -> f32 {
    let start_line = session.line_of_offset(range.start);
    let end_line = session.line_of_offset(range.end);
    let hard_lines = (end_line - start_line) as f32 + 1.0;
    let byte_len = (range.end - range.start) as f32;
    let wrapped = (byte_len / chars_per_line.max(1.0)).ceil().max(1.0);
    hard_lines.max(wrapped) * BASE_LINE_HEIGHT * scale
}
