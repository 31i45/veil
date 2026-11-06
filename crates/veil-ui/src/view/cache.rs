//! 派生缓存：布局/光栅/块级与行内公式/文档图片/表格网格的统一存放。
//!
//! 正确性由因果键承载（ADR-009 决策四）：每个缓存项记录生成它时的全部外部输入，
//! 读取时比对键，不匹配即视为缺失——不再有 `invalidate_*`/`clear_*` 失效方法族。
//! 键的构成逐类如下（与原手工失效语义逐一对应）：
//! - 块布局/光栅/表格网格：`BaseKey`（版心宽、DPI、主题、会话、图片基准）+ 激活指纹；
//!   宽/DPI/主题/新会话/另存为变化时键自然失配；
//! - 块级公式图：块 id + DPI + 主题 + 会话；失败记忆不含主题（换主题不重试渲染）；
//! - 行内公式：tex 内容寻址 + DPI + 主题；失败记忆仅按 DPI（跨文档保留）；
//! - 文档图片：解析后路径 + 图片基准；失败记忆同键。
//!
//! `BlockId` 在重解析脏块时新鲜分配（veil-core），故同一会话内 id 即内容修订；
//! 跨会话 id 会复用，故 `BaseKey` 与会话型键均携带会话代次。
//!
//! `prune` 只做内存回收：删掉已不存在块或已过期代次的条目，不承担正确性——
//! 即使某条目未被回收，读取时键不匹配同样不会命中。

use std::collections::{HashMap, HashSet};
use std::ops::Deref;

use veil_core::{BlockId, BlockKind, Range};

use super::layout::BlockLayout;
use super::table::TableLayout;
use crate::app::App;
use crate::image::ImageData;
use crate::math::MathImage;

/// 文本光栅：RGBA8 像素 + 宽高（物理像素）。
pub(crate) type Raster = (Vec<u8>, u32, u32);

/// 带因果键的缓存项。读取方只比较 `key`，命中后经 `Deref` 直接用值。
pub(crate) struct Memo<K, V> {
    pub(crate) key: K,
    pub(crate) value: V,
}

impl<K: PartialEq, V> Memo<K, V> {
    pub(crate) fn matches(&self, key: &K) -> bool {
        &self.key == key
    }
}

impl<K, V> Deref for Memo<K, V> {
    type Target = V;

    fn deref(&self) -> &V {
        &self.value
    }
}

/// 外部输入代次：主题、会话（新文档）、图片基准（打开/另存为）各自单调递增。
/// 键不存时间戳，只存代次值，故同一代次内的条目天然复用。
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(crate) struct Revs {
    /// 主题切换代次：颜色相关缓存据此作废。
    pub(crate) theme: u32,
    /// 会话代次：新文档（BlockId 可能复用）据此作废。
    pub(crate) session: u32,
    /// 图片相对路径基准代次：打开/另存为时递增。
    pub(crate) image_base: u32,
}

impl Revs {
    pub(crate) fn base(self, width: f32, scale: f32, source_mode: bool) -> BaseKey {
        BaseKey {
            width_q: quantize(width, 100.0),
            scale_q: quantize(scale, 1000.0),
            theme_rev: self.theme,
            session_rev: self.session,
            image_base: self.image_base,
            source_mode,
        }
    }
}

/// 与版心宽/DPI/主题/会话/图片基准/源码模式相关的键：布局、光栅、表格网格共用。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct BaseKey {
    width_q: u32,
    scale_q: u32,
    theme_rev: u32,
    session_rev: u32,
    image_base: u32,
    /// 源码模式：true 时全部块按源码排版（打印态恒 false，与屏幕态天然区分）。
    source_mode: bool,
}

/// 表格网格键 = 基础键 + 活动单元格 flat 索引（`u32::MAX` = 无活动格）。
/// 活动格切换会改变单元格文本来源（源码 vs 剥标记），必须换键重建。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct TableKey {
    pub(crate) base: BaseKey,
    pub(crate) active: u32,
}

/// 布局键 = 基础键 + Reveal 激活指纹（0 = 无激活，其余为块结构的激活状态哈希）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct LayoutKey {
    pub(crate) base: BaseKey,
    pub(crate) reveal: u64,
}

/// 行内公式图键：按 tex 内容寻址，另含 DPI 与主题。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct InlineKey {
    scale_q: u32,
    theme_rev: u32,
}

/// 块级公式图键：块 id 即内容修订，另含 DPI/主题/会话。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct MathKey {
    pub(crate) block: BlockId,
    pub(crate) scale_q: u32,
    pub(crate) theme_rev: u32,
    pub(crate) session_rev: u32,
}

/// 块级公式失败记忆：不含主题（换主题不重试），按 DPI/会话作废。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct MathFailKey {
    pub(crate) block: BlockId,
    pub(crate) scale_q: u32,
    pub(crate) session_rev: u32,
}

fn quantize(v: f32, factor: f32) -> u32 {
    (v * factor).round().clamp(0.0, u32::MAX as f32) as u32
}

/// DPI 量化值：公式渲染器与缓存键共用同一取整口径。
pub(crate) fn scale_q(scale: f32) -> u32 {
    quantize(scale, 1000.0)
}

/// 五类派生缓存的统一存放。字段对 crate 内可见；读取一律经本文件的带键查询方法，
/// 不经字段直接判断存在性。
#[derive(Default)]
pub(crate) struct Cache {
    /// 块布局：键 = 宽度/DPI/主题/图片基准 + 激活指纹。
    pub(crate) layouts: HashMap<BlockId, Memo<LayoutKey, BlockLayout>>,
    /// 文本光栅：键与布局键一致（光栅由该次布局生成、按前景色光栅化）。
    pub(crate) rasters: HashMap<BlockId, Memo<LayoutKey, Raster>>,
    /// 数学块渲染图与失败记忆。
    pub(crate) math_imgs: HashMap<BlockId, Memo<MathKey, MathImage>>,
    pub(crate) math_failed: HashMap<BlockId, Memo<MathFailKey, ()>>,
    /// 行内公式图与失败记忆：按 tex 内容寻址。
    pub(crate) inline_math: HashMap<String, Memo<InlineKey, MathImage>>,
    pub(crate) inline_failed: HashMap<String, Memo<u32, ()>>,
    /// 文档内图片（自然尺寸位图）与失败记忆：键 = 按文档目录解析后的路径。
    pub(crate) inline_images: HashMap<String, Memo<u32, ImageData>>,
    pub(crate) image_failed: HashMap<String, Memo<u32, ()>>,
    /// 表格网格：键 = 基础键 + 活动单元格。
    pub(crate) tables: HashMap<BlockId, Memo<TableKey, TableLayout>>,
}

impl Cache {
    /// 命中校验：基础键相等且激活指纹一致。指纹由布局自身记录的结构重算，
    /// 不读 session（缓存命中路径零分配）。
    fn layout_key(
        &self,
        id: BlockId,
        kind: &BlockKind,
        range: Range,
        base: BaseKey,
        caret: usize,
    ) -> Option<LayoutKey> {
        let m = self.layouts.get(&id)?;
        (m.key.base == base && m.key.reveal == m.value.reveal_state(kind, range, caret))
            .then_some(m.key)
    }

    pub(crate) fn layout(
        &self,
        id: BlockId,
        kind: &BlockKind,
        range: Range,
        base: BaseKey,
        caret: usize,
    ) -> Option<&BlockLayout> {
        self.layout_key(id, kind, range, base, caret)
            .map(|_| &self.layouts[&id].value)
    }

    /// 绘制路径用：光栅与布局共用同一键，布局命中后直接取键比对光栅。
    pub(crate) fn raster_key(
        &self,
        id: BlockId,
        kind: &BlockKind,
        range: Range,
        base: BaseKey,
        caret: usize,
    ) -> Option<LayoutKey> {
        self.layout_key(id, kind, range, base, caret)
    }

    pub(crate) fn raster(&self, id: BlockId, key: LayoutKey) -> Option<&Raster> {
        self.rasters
            .get(&id)
            .filter(|m| m.matches(&key))
            .map(|m| &m.value)
    }

    pub(crate) fn math(&self, id: BlockId, key: MathKey) -> Option<&MathImage> {
        self.math_imgs
            .get(&id)
            .filter(|m| m.matches(&key))
            .map(|m| &m.value)
    }

    pub(crate) fn math_failed(&self, id: BlockId, key: MathFailKey) -> bool {
        self.math_failed.get(&id).is_some_and(|m| m.matches(&key))
    }

    pub(crate) fn inline_math(&self, tex: &str, key: InlineKey) -> Option<&MathImage> {
        self.inline_math
            .get(tex)
            .filter(|m| m.matches(&key))
            .map(|m| &m.value)
    }

    pub(crate) fn inline_failed(&self, tex: &str, key: u32) -> bool {
        self.inline_failed.get(tex).is_some_and(|m| m.matches(&key))
    }

    pub(crate) fn image(&self, path: &str, key: u32) -> Option<&ImageData> {
        self.inline_images
            .get(path)
            .filter(|m| m.matches(&key))
            .map(|m| &m.value)
    }

    pub(crate) fn image_failed(&self, path: &str, key: u32) -> bool {
        self.image_failed.get(path).is_some_and(|m| m.matches(&key))
    }

    pub(crate) fn table(&self, id: BlockId, key: TableKey) -> Option<&TableLayout> {
        self.tables
            .get(&id)
            .filter(|m| m.matches(&key))
            .map(|m| &m.value)
    }

    /// 索引重建驱动的全量回收（不承担正确性）：删除已消失块或过期代次的条目。
    /// 调用点唯一：`refresh_index → geom_fill` 末尾，与块真值同源。
    pub(crate) fn prune_alive(&mut self, alive: &HashSet<BlockId>, revs: &Revs, base: &BaseKey) {
        let inline_key = InlineKey {
            scale_q: base.scale_q,
            theme_rev: revs.theme,
        };
        self.layouts
            .retain(|id, m| alive.contains(id) && m.key.base == *base);
        self.rasters
            .retain(|id, m| alive.contains(id) && m.key.base == *base);
        self.tables
            .retain(|id, m| alive.contains(id) && m.key.base == *base);
        self.math_imgs.retain(|id, m| {
            alive.contains(id)
                && m.key.block == *id
                && m.key.scale_q == base.scale_q
                && m.key.theme_rev == revs.theme
                && m.key.session_rev == revs.session
        });
        self.math_failed.retain(|id, m| {
            alive.contains(id)
                && m.key.block == *id
                && m.key.scale_q == base.scale_q
                && m.key.session_rev == revs.session
        });
        self.inline_math.retain(|_, m| m.matches(&inline_key));
        self.inline_failed.retain(|_, m| m.key == base.scale_q);
        self.inline_images.retain(|_, m| m.key == revs.image_base);
        self.image_failed.retain(|_, m| m.key == revs.image_base);
    }

    /// 视口驱动回收（paint 帧末尾调用）：把 rasters/layouts/tables 从"全部累积"剪到
    /// "窗口 + 一屏 overscan + caret 块 + 块级公式图"量级，防止滚动过程中内存持续增长。
    /// 其他 6 个缓存（math_imgs 除外）不动：它们要么量小要么按内容寻址跨块共享。
    /// base_key 不匹配的条目按 `m.key.base == *base` 双重保险剪（resize/DPI 风暴期间旧键残留）。
    pub(crate) fn prune_viewport(
        &mut self,
        keep: &HashSet<BlockId>,
        base: &BaseKey,
    ) {
        self.layouts.retain(|id, m| keep.contains(id) && m.key.base == *base);
        self.rasters.retain(|id, m| keep.contains(id) && m.key.base == *base);
        self.tables.retain(|id, m| keep.contains(id) && m.key.base == *base);
    }
}

/// 键构造：全部来自 App 的外部输入与当前几何，各 ensure/绘制路径共用同一口径。
impl App {
    pub(crate) fn base_key(&self) -> BaseKey {
        self.input_state
            .revs
            .base(self.content_width(), self.input_state.scale, self.source_mode())
    }

    pub(crate) fn math_key(&self, block: BlockId) -> MathKey {
        MathKey {
            block,
            scale_q: scale_q(self.input_state.scale),
            theme_rev: self.input_state.revs.theme,
            session_rev: self.input_state.revs.session,
        }
    }

    pub(crate) fn math_fail_key(&self, block: BlockId) -> MathFailKey {
        MathFailKey {
            block,
            scale_q: scale_q(self.input_state.scale),
            session_rev: self.input_state.revs.session,
        }
    }

    pub(crate) fn inline_key(&self) -> InlineKey {
        InlineKey {
            scale_q: scale_q(self.input_state.scale),
            theme_rev: self.input_state.revs.theme,
        }
    }

    /// 行内公式/图片失败记忆的键（仅 DPI）。
    pub(crate) fn dpi_q(&self) -> u32 {
        scale_q(self.input_state.scale)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids() -> Vec<BlockId> {
        let s = veil_core::Session::open("# a\n\nb\n\nc\n");
        s.blocks().iter().map(|(id, _, _)| *id).collect()
    }

    #[test]
    fn base_key_quantizes_stably_and_tracks_inputs() {
        let revs = Revs::default();
        let b = revs.base(800.0, 1.25, false);
        assert_eq!(b, revs.base(800.0, 1.25, false), "同一输入必得同键");
        assert_ne!(b, revs.base(800.5, 1.25, false), "版心宽变化即换键");
        assert_ne!(b, revs.base(800.0, 1.5, false), "DPI 变化即换键");
        assert_ne!(b, revs.base(800.0, 1.25, true), "源码模式变化即换键");
        let revs2 = Revs { theme: 1, ..revs };
        assert_ne!(b, revs2.base(800.0, 1.25, false), "主题代次变化即换键");
        let revs3 = Revs {
            image_base: 1,
            ..revs
        };
        assert_ne!(b, revs3.base(800.0, 1.25, false), "图片基准变化即换键");
        let revs4 = Revs { session: 1, ..revs };
        assert_ne!(b, revs4.base(800.0, 1.25, false), "新会话（BlockId 复用）即换键");
    }

    #[test]
    fn prune_keeps_only_current_generation() {
        let ids = ids();
        let mut c = Cache::default();
        let revs = Revs::default();
        let base = revs.base(800.0, 1.0, false);
        c.math_failed.insert(
            ids[0],
            Memo {
                key: MathFailKey {
                    block: ids[0],
                    scale_q: base.scale_q,
                    session_rev: revs.session,
                },
                value: (),
            },
        );
        c.inline_math.insert(
            "x".into(),
            Memo {
                key: InlineKey {
                    scale_q: base.scale_q,
                    theme_rev: revs.theme,
                },
                value: MathImage {
                    pixels: vec![0; 4],
                    width: 1,
                    height: 1,
                    baseline: 1,
                },
            },
        );
        c.inline_failed.insert(
            "y".into(),
            Memo {
                key: base.scale_q,
                value: (),
            },
        );
        c.inline_images.insert(
            "img".into(),
            Memo {
                key: revs.image_base,
                value: ImageData {
                    pixels: vec![0; 4],
                    width: 1,
                    height: 1,
                },
            },
        );
        let alive: HashSet<BlockId> = ids.iter().copied().collect();
        c.prune_alive(&alive, &revs, &base);
        assert!(c.math_failed.contains_key(&ids[0]), "同代次保留");
        assert!(
            c.inline_math.contains_key("x"),
            "行内公式按 tex 寻址，同代次保留"
        );
        assert!(c.inline_failed.contains_key("y"), "行内失败记忆保留");
        assert!(c.inline_images.contains_key("img"), "图片同基准保留");
        // 主题切换：颜色相关条目过期；失败记忆与图片保留
        let mut c2 = c;
        let revs2 = Revs { theme: 1, ..revs };
        let base2 = revs2.base(800.0, 1.0, false);
        c2.prune_alive(&alive, &revs2, &base2);
        assert!(c2.inline_math.is_empty(), "公式图随主题作废");
        assert!(
            c2.math_failed.contains_key(&ids[0]),
            "失败记忆与颜色无关，保留"
        );
        assert!(c2.inline_images.contains_key("img"), "图片与主题无关，保留");
        // DPI 变化：失败记忆过期
        let revs3 = Revs { theme: 1, ..revs };
        let base3 = revs3.base(800.0, 2.0, false);
        c2.prune_alive(&alive, &revs3, &base3);
        assert!(!c2.inline_failed.contains_key("y"), "失败记忆随 DPI 作废");
        // 新会话：块相关条目过期，行内公式与失败记忆跨文档保留
        let revs4 = Revs {
            theme: 1,
            session: 1,
            image_base: 1,
        };
        let base4 = revs4.base(800.0, 2.0, false);
        // 行内失败记忆按当前 DPI 代次重新登记（上一轮 DPI 变化已使其过期）
        c2.inline_failed.insert(
            "y".into(),
            Memo {
                key: base4.scale_q,
                value: (),
            },
        );
        c2.prune_alive(&alive, &revs4, &base4);
        assert!(c2.math_failed.is_empty(), "新会话：块失效记忆作废");
        assert!(c2.inline_images.is_empty(), "新图片基准：位图作废");
        assert!(c2.inline_failed.contains_key("y"), "行内失败记忆跨文档保留");
    }

    #[test]
    fn prune_viewport_reclaims_non_visible_rasters() {
        let ids = ids();
        let mut c = Cache::default();
        let revs = Revs::default();
        let base = revs.base(800.0, 1.0, false);
        let lk0 = LayoutKey { base, reveal: 0 };
        let lk1 = LayoutKey { base, reveal: 0 };

        // 为 ids[0] 和 ids[1] 填充 rasters（值 = (像素, 宽, 高) 易构造）
        c.rasters.insert(
            ids[0],
            Memo { key: lk0, value: (vec![0u8; 4], 1, 1) },
        );
        c.rasters.insert(
            ids[1],
            Memo { key: lk1, value: (vec![0u8; 4], 1, 1) },
        );
        assert_eq!(c.rasters.len(), 2, "预填充成功");

        // 剪完只保留 ids[0]（模拟 ids[1] 滚出视口）
        let mut keep: HashSet<BlockId> = HashSet::new();
        keep.insert(ids[0]);
        c.prune_viewport(&keep, &base);

        assert!(c.rasters.contains_key(&ids[0]), "可见块 raster 保留");
        assert_eq!(c.rasters.len(), 1, "非可见块 raster 被剪");

        // base_key 不匹配的条目也被剪（双重保险：resize/DPI 风暴期间旧键残留）
        let revs2 = Revs { theme: 1, ..revs };
        let base2 = revs2.base(800.0, 1.0, false);
        // 用 ids[1] 插 base2 条目（ids[0] 已经被 prune_viewport 保留且还是 base 的条目）
        c.rasters.insert(
            ids[1],
            Memo { key: LayoutKey { base: base2, reveal: 0 }, value: (vec![0u8; 4], 1, 1) },
        );
        assert_eq!(c.rasters.len(), 2, "ids[0] base 条目 + ids[1] base2 条目");
        let keep2: HashSet<BlockId> = [ids[0], ids[1]].into_iter().collect();
        c.prune_viewport(&keep2, &base); // 用旧 base 剪 → ids[1] base2 被剪；ids[0] base 保留
        assert_eq!(c.rasters.len(), 1, "base_key 不匹配的条目被剪");
        assert!(c.rasters.contains_key(&ids[0]), "base_key 匹配的保留");
    }

    #[test]
    fn prune_viewport_leaves_content_addressed_caches_alone() {
        let ids = ids();
        let mut c = Cache::default();
        let revs = Revs::default();
        let base = revs.base(800.0, 1.0, false);

        // math_imgs / inline_math / inline_images 是内容寻址（tex/path），不按 BlockId 累积
        c.math_imgs.insert(
            ids[0],
            Memo {
                key: MathKey {
                    block: ids[0],
                    scale_q: base.scale_q,
                    theme_rev: revs.theme,
                    session_rev: revs.session,
                },
                value: MathImage {
                    pixels: vec![0; 4],
                    width: 1,
                    height: 1,
                    baseline: 1,
                },
            },
        );
        c.inline_math.insert(
            "x".into(),
            Memo {
                key: InlineKey {
                    scale_q: base.scale_q,
                    theme_rev: revs.theme,
                },
                value: MathImage {
                    pixels: vec![0; 4],
                    width: 1,
                    height: 1,
                    baseline: 1,
                },
            },
        );

        let keep: HashSet<BlockId> = HashSet::new();
        c.prune_viewport(&keep, &base);

        // prune_viewport 只剪 raster/layout/table，其他不动
        assert!(c.math_imgs.contains_key(&ids[0]), "块级公式图（MathKey 索引）不被 viewport prune 剪");
        assert!(c.inline_math.contains_key("x"), "行内公式（tex 内容寻址）不被 viewport prune 剪");
    }
}
