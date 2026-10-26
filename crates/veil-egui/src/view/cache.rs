//! 派生缓存：布局/光栅/块级与行内公式/文档图片/表格网格的统一存放与失效入口。
//!
//! 失效语义（逐条对照原 shell 散落调用点，行为零变化）：
//! - 编辑脏块：内容变化使该块的布局/光栅/公式图/失败记忆/表格网格全部作废；
//! - Reveal 激活变化：只影响块内文本排版与其光栅（公式/表格网格与激活无关）；
//! - 主题切换：按颜色生成的缓存作废（光栅/表格位图/公式图），布局几何不受影响；
//! - 窗口宽度变化：换行与列宽依赖版心宽，布局/光栅/表格作废（公式图尺寸不依赖宽度）；
//! - DPI 变化：一切按 scale 生成的缓存作废，含失败记忆（新 DPI 下渲染可能成功）；
//! - 另存为/打开：图片相对路径基准变化，图片缓存与失败记忆一并作废；
//!   行内公式按 tex 内容寻址，与文档无关，跨文档保留。

use std::collections::{HashMap, HashSet};

use veil_core::BlockId;

use super::layout::BlockLayout;
use super::table::TableLayout;
use crate::image::ImageData;
use crate::math::MathImage;

/// 五类派生缓存的统一存放。字段对 crate 内可见（ensure_* 编排仍由 App 方法承担，
/// 缓存本身不做策略）；失效时机一律经 `invalidate_*`/`clear_*` 方法族表达。
#[derive(Default)]
pub(crate) struct Cache {
    /// 块布局：脏块驱动失效，干净块不重排。
    pub(crate) layouts: HashMap<BlockId, BlockLayout>,
    /// 文本光栅：布局失效/主题切换后按新前景色重光栅。
    pub(crate) rasters: HashMap<BlockId, (Vec<u8>, u32, u32)>,
    /// 数学块渲染图与失败记忆：脏块驱动失效。
    pub(crate) math_imgs: HashMap<BlockId, MathImage>,
    pub(crate) math_failed: HashSet<BlockId>,
    /// 行内公式图与失败记忆：按 tex 内容寻址，无需随编辑显式失效。
    pub(crate) inline_math: HashMap<String, MathImage>,
    pub(crate) inline_failed: HashSet<String>,
    /// 文档内图片（自然尺寸位图）与失败记忆：键 = 按文档目录解析后的路径。
    pub(crate) inline_images: HashMap<String, ImageData>,
    pub(crate) image_failed: HashSet<String>,
    /// 表格网格：脏块/宽度/DPI 变化即失效。
    pub(crate) tables: HashMap<BlockId, TableLayout>,
}

impl Cache {
    /// 编辑脏块失效（after_edit 唯一失效入口）。
    pub(crate) fn invalidate_dirty(&mut self, dirty: &[BlockId]) {
        for id in dirty {
            self.layouts.remove(id);
            self.rasters.remove(id);
            self.math_imgs.remove(id);
            self.math_failed.remove(id);
            self.tables.remove(id);
        }
    }

    /// Reveal 激活态变化：仅块内文本排版与光栅受影响。
    pub(crate) fn invalidate_reveal(&mut self, id: BlockId) {
        self.layouts.remove(&id);
        self.rasters.remove(&id);
    }

    /// 主题切换：按颜色生成的缓存全部作废；失败记忆与颜色无关，保留。
    /// （chrome 标签缓存随 `Chrome::clear_label_cache` 同步作废。）
    pub(crate) fn clear_themed(&mut self) {
        self.rasters.clear();
        self.tables.clear();
        self.math_imgs.clear();
        self.inline_math.clear();
    }

    /// 窗口宽度变化：换行/列宽依赖版心宽。
    pub(crate) fn clear_resize(&mut self) {
        self.layouts.clear();
        self.rasters.clear();
        self.tables.clear();
    }

    /// DPI 变化：全部按 scale 生成的缓存（含失败记忆）作废。
    /// （chrome 标签缓存随 `Chrome::clear_label_cache` 同步作废。）
    pub(crate) fn clear_scale(&mut self) {
        self.layouts.clear();
        self.rasters.clear();
        self.math_imgs.clear();
        self.math_failed.clear();
        self.inline_math.clear();
        self.inline_failed.clear();
        self.tables.clear();
    }

    /// 另存为：图片相对路径基准变化；占位尺寸变化使布局/光栅/表格一并重排。
    pub(crate) fn clear_repath(&mut self) {
        self.inline_images.clear();
        self.image_failed.clear();
        self.layouts.clear();
        self.rasters.clear();
        self.tables.clear();
    }

    /// 打开新文档：内容寻址的行内公式跨文档保留，其余内容缓存全部作废。
    pub(crate) fn clear_document(&mut self) {
        self.layouts.clear();
        self.rasters.clear();
        self.math_imgs.clear();
        self.math_failed.clear();
        self.inline_images.clear();
        self.image_failed.clear();
        self.tables.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids() -> Vec<BlockId> {
        // 脏块失效只要求可构造的 BlockId；经由 Session 取得真实 id。
        let s = veil_core::Session::open("# a\n\nb\n\nc\n");
        s.blocks().iter().map(|(id, _, _)| *id).collect()
    }

    #[test]
    fn invalidate_dirty_drops_per_block_entries() {
        let ids = ids();
        let mut c = Cache::default();
        for &id in &ids {
            c.math_failed.insert(id);
        }
        c.invalidate_dirty(&[ids[0]]);
        assert!(!c.math_failed.contains(&ids[0]));
        assert!(c.math_failed.contains(&ids[1]));
    }

    #[test]
    fn clear_themed_keeps_layouts_and_failure_memory() {
        let ids = ids();
        let mut c = Cache::default();
        c.math_failed.insert(ids[0]);
        c.inline_failed.insert("x".into());
        c.inline_math.insert(
            "y".into(),
            MathImage {
                pixels: vec![0; 4],
                width: 1,
                height: 1,
                baseline: 1,
            },
        );
        c.clear_themed();
        assert!(c.inline_math.is_empty(), "公式图按前景色生成，须作废");
        assert!(c.math_failed.contains(&ids[0]), "失败记忆与颜色无关，保留");
        assert!(c.inline_failed.contains("x"), "行内失败记忆保留");
    }

    #[test]
    fn clear_document_keeps_tex_keyed_inline_math() {
        let mut c = Cache::default();
        c.inline_math.insert(
            "z".into(),
            MathImage {
                pixels: vec![0; 4],
                width: 1,
                height: 1,
                baseline: 1,
            },
        );
        c.image_failed.insert("img".into());
        c.clear_document();
        assert!(c.inline_math.contains_key("z"), "tex 内容寻址，跨文档有效");
        assert!(c.image_failed.is_empty(), "路径基准变化，失败记忆作废");
    }
}
