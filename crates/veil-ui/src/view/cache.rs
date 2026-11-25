//! 派生缓存：布局/光栅/块级与行内公式/文档图片/表格网格的统一存放。
//!
//! 正确性由因果键承载（ADR-009 决策四）：每个缓存项记录生成它时的全部外部输入，
//! 读取时比对键，不匹配即视为缺失——不再有 `invalidate_*`/`clear_*` 失效方法族。
//! 键的构成逐类如下（与原手工失效语义逐一对应）：
//! - 块布局/光栅/表格网格：`BaseKey`（版心宽、DPI、独占图片限高、主题、会话、图片基准、
//!   远程图片代次、脚注代次）+ 激活指纹；宽/DPI/窗口高度/主题/新会话/另存为/远程图到达/
//!   脚注编号变化时键自然失配；
//! - 块级公式图：块 id + DPI + 主题 + 会话；失败记忆不含主题（换主题不重试渲染）；
//! - 行内公式：tex 内容寻址 + DPI + 主题；失败记忆仅按 DPI（跨文档保留）；
//! - 文档图片：解析后路径 + 图片基准；失败记忆同键。
//!
//! `BlockId` 在重解析脏块时新鲜分配（veil-core），故同一会话内 id 即内容修订；
//! 跨会话 id 会复用，故 `BaseKey` 与会话型键均携带会话代次。
//!
//! `prune` 只做内存回收：删掉已不存在块或已过期代次的条目，不承担正确性——
//! 即使某条目未被回收，读取时键不匹配同样不会命中。

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::ops::Deref;

use veil_core::{BlockId, BlockKind, Range};

use super::layout::{BlockLayout, FootnoteCtx};
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
    /// 图片内容代次：「重新加载图片」等显式重读时递增，使引用该图的块布局失配
    /// （图片自然尺寸可能已变，块高/占位必须按新图重排）。
    pub(crate) image_content: u32,
    /// 远程图片代次：远程图到达或全局开关切换时递增，使全部布局失配重排。
    pub(crate) remote_images: u32,
}

impl Revs {
    pub(crate) fn base(
        self,
        width: f32,
        scale: f32,
        img_max_h: f32,
        source_mode: bool,
        footnote_epoch: u64,
    ) -> BaseKey {
        BaseKey {
            width_q: quantize(width, 100.0),
            scale_q: quantize(scale, 1000.0),
            // 独占图片的限高随窗口高度变化：不量化入键会让「先矮后高」的窗口复用
            // 旧的小图布局。按整数像素量化，避免亚像素尺寸抖动引发无谓重排。
            img_h_q: quantize(img_max_h, 1.0),
            theme_rev: self.theme,
            session_rev: self.session,
            image_base: self.image_base,
            image_content: self.image_content,
            remote_images: self.remote_images,
            source_mode,
            footnote_epoch,
            font_size_q: 0,
            line_height_q: 0,
            font_family: 0,
            font_face: 0,
            doc_margin_q: 0,
            fold_epoch: 0,
            list_epoch: 0,
        }
    }
}

/// 与版心宽/DPI/主题/会话/图片基准/源码模式相关的键：布局、光栅、表格网格共用。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct BaseKey {
    width_q: u32,
    scale_q: u32,
    /// 独占一行图片的高度上限（整数像素）：由窗口可视高度派生，是布局输入。
    img_h_q: u32,
    theme_rev: u32,
    session_rev: u32,
    image_base: u32,
    /// 图片内容代次：显式「重新加载图片」后递增，强制相关块按新自然尺寸重排。
    image_content: u32,
    /// 正文排版输入：字号/行高（×1000 量化）与字体族判别值。
    /// 设置变更即时生效依赖它——布局/光栅/表格键随排版输入失配。
    font_size_q: u32,
    line_height_q: u32,
    font_family: u8,
    /// 显式字族名指纹（0 = 沿用四档预置）：切换具体字族时换键重排。
    font_face: u64,
    /// 屏幕版心边距（逻辑像素 ×100）：宽屏封顶/全宽切换时版心宽可能相同，
    /// 但画布锚点不同，必须换键重排。
    doc_margin_q: u32,
    /// 远程图片到达/开关切换代次：远程图占位转为实图时换键重排；
    /// 本地图片缓存键仍只用 `image_base`，不因远程图重读磁盘。
    remote_images: u32,
    /// 源码模式：true 时全部块按源码排版（打印态恒 false，与屏幕态天然区分）。
    source_mode: bool,
    /// 脚注代次：全文引用顺序 + 编号的确定性哈希。编号是全文派生量——文首新增引用
    /// 会改变文末定义块的前缀编号，而定义块文本不变；不带此代次会命中旧布局。
    footnote_epoch: u64,
    /// 行内 details 折叠代次（B3）：折叠态改变块的排版文本（正文整段消隐），
    /// 必须换键重排；集合按「条目哈希异或」折叠，与迭代顺序无关。
    fold_epoch: u64,
    /// 有序列表编号代次：编号是全文派生量，文首增删列表项会改变后续块的显示编号，
    /// 而块文本与 BlockId 均可不变（增量重解析只平移），不带此代次会命中陈旧编号。
    list_epoch: u64,
}

/// 表格网格键 = 基础键 + 活动单元格 flat 索引（`u32::MAX` = 无活动格）
/// + 列宽覆盖指纹（0 = 无覆盖）。
///
/// 活动格切换会改变单元格文本来源（源码 vs 剥标记），必须换键重建。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct TableKey {
    pub(crate) base: BaseKey,
    pub(crate) active: u32,
    /// 用户拖动列宽（B2）内容的 FNV-1a 指纹：拖动即换键重建网格。
    pub(crate) widths: u64,
}

/// 布局键 = 基础键 + Reveal 激活指纹（0 = 无激活，其余为块结构的激活状态哈希）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct LayoutKey {
    pub(crate) base: BaseKey,
    pub(crate) reveal: u64,
    /// 全文标题集合指纹：仅 `[TOC]` 标记块取当前值，其余块恒 0。
    /// 目录文本由全文标题派生，标记块自身字节/BlockId 不变，必须单列才能失效（DEF-T05-005）。
    pub(crate) toc: u64,
    /// 长代码块后台异步高亮回填代次：任何一个超限 FencedCode 的后台 parse 完成
    /// 并回填颜色后递增。layout 重排时 epoch 自然失配 → 走完整同步路径（此时
    /// Highlighter 缓存已命中，瞬间产出颜色）。
    pub(crate) highlight_epoch: u64,
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

/// 显式字族名的稳定指纹（FNV-1a；`None` → 0）。仅用于缓存键，不参与展示。
fn face_key(face: Option<&str>) -> u64 {
    let Some(face) = face else { return 0 };
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for b in face.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h | 1
}

/// 五类派生缓存的统一存放。字段对 crate 内可见；读取一律经本文件的带键查询方法，
/// 不经字段直接判断存在性。
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
    /// 远程图片（自然尺寸位图）：会话级缓存，键 = 绝对 URL；文档切换时清空。
    pub(crate) remote_images: HashMap<String, ImageData>,
    /// 远程图片失败记忆：同会话内不重试（URL 绝对，不受文档目录影响）。
    pub(crate) remote_failed: HashSet<String>,
    /// 已发起、尚未返回的远程请求：去重与轮询唤醒的依据。
    pub(crate) remote_pending: HashSet<String>,
    /// Mermaid 块渲染位图：键 = BlockId，值 = resvg 光栅化的 RGBA 位图。
    pub(crate) mermaid_imgs: HashMap<BlockId, crate::mermaid_renderer::MermaidImage>,
    /// Mermaid 块失败记忆：键 = (source_hash, dark)，值 = 连续失败次数。
    /// ≥ MAX_MERMAID_FAIL_COUNT（3）则永久禁用该块自动预览，直到源码变化（key 自然变）
    /// 或主题切换（refresh_theme_visuals 主动 clear）。
    pub(crate) mermaid_failed: HashMap<(u64, bool), u8>,
    /// 脚注渲染上下文缓存：(编号代次, 前缀色, 表)。代次与颜色均不变时复用同一份。
    pub(crate) footnote_ctx: Option<(u64, [u8; 3], std::rc::Rc<FootnoteCtx>)>,
    /// 图片缓存 LRU 追踪：key（inline 路径或 remote URL）→ 最近一次使用（插入或命中）
    /// 的相对纳秒时间戳。字节总量超上限时按时间戳升序逐出——命中即刷新时间戳，
    /// 故仍在被绘制的图片不会被优先逐出（真 LRU，而非首次插入时间的 FIFO）。
    /// 值用 `Cell` 包住：命中路径在 `&self` 下也能刷新最近使用时间。
    image_last_used: HashMap<String, Cell<u64>>,
    /// 图片缓存字节总量上限（默认 200 MiB）。0 表示不限制。
    image_limit_bytes: usize,
    /// Cache 创建时的 anchor 时间，用于计算相对时间戳。
    anchor: std::time::Instant,
}

impl Default for Cache {
    fn default() -> Self {
        Self {
            layouts: HashMap::new(),
            rasters: HashMap::new(),
            math_imgs: HashMap::new(),
            math_failed: HashMap::new(),
            inline_math: HashMap::new(),
            inline_failed: HashMap::new(),
            inline_images: HashMap::new(),
            image_failed: HashMap::new(),
            tables: HashMap::new(),
            remote_images: HashMap::new(),
            remote_failed: HashSet::new(),
            remote_pending: HashSet::new(),
            mermaid_imgs: HashMap::new(),
            mermaid_failed: HashMap::new(),
            footnote_ctx: None,
            image_last_used: HashMap::new(),
            image_limit_bytes: 200 * 1024 * 1024,
            anchor: std::time::Instant::now(),
        }
    }
}

impl Cache {
    /// 设置图片缓存字节上限（单位 MiB）。0 表示不限制。
    pub(crate) fn set_image_limit_mb(&mut self, mb: u32) {
        self.image_limit_bytes = if mb == 0 {
            0
        } else {
            (mb as usize) * 1024 * 1024
        };
    }

    /// 当前相对纳秒时间戳（自 Cache 创建起算）。单调递增，可直接比较先后。
    fn now_ns(&self) -> u64 {
        self.anchor.elapsed().as_nanos() as u64
    }

    /// 刷新某张已缓存图片的最近使用时间。命中路径调用——这是 LRU 与 FIFO 的分界：
    /// 只记首次插入时间的话，反复使用的旧图反而会先被逐出。
    fn touch_image(&self, key: &str) {
        if let Some(ts) = self.image_last_used.get(key) {
            ts.set(self.now_ns());
        }
    }

    /// 估算单张图片字节数（pixels.len() 即实际字节数）。
    fn estimate_image_bytes(data: &ImageData) -> usize {
        data.pixels.len()
    }

    /// 计算 inline + remote 图片缓存总字节。
    fn total_image_bytes(&self) -> usize {
        let inline_total: usize = self
            .inline_images
            .values()
            .map(|m| Self::estimate_image_bytes(&m.value))
            .sum();
        let remote_total: usize = self
            .remote_images
            .values()
            .map(Self::estimate_image_bytes)
            .sum();
        inline_total + remote_total
    }

    /// 图片插入后调用：按 LRU 逐出最久未使用条目直到总字节 ≤ 上限。
    /// 只在真的有逐出时才改变 HashMap，稳态零开销。
    fn evict_images_if_needed(&mut self) {
        if self.image_limit_bytes == 0 {
            return;
        }
        let total = self.total_image_bytes();
        if total <= self.image_limit_bytes {
            return;
        }

        // 收集所有图片 key 和最近使用时间，按时间戳升序（最久未用在前）
        let mut entries: Vec<(u64, String)> = self
            .image_last_used
            .iter()
            .map(|(k, ts)| (ts.get(), k.clone()))
            .collect();
        entries.sort_by_key(|(ts, _)| *ts);

        let target = self.image_limit_bytes * 3 / 4; // 逐出到上限的 75% 留余量
        let mut current = total;
        for (_, key) in entries {
            if current <= target {
                break;
            }
            // 尝试从 inline 或 remote 移除
            if let Some(memo) = self.inline_images.remove(&key) {
                current -= Self::estimate_image_bytes(&memo.value);
                self.image_failed.remove(&key);
            } else if let Some(data) = self.remote_images.remove(&key) {
                current -= Self::estimate_image_bytes(&data);
                self.remote_failed.remove(&key);
                self.remote_pending.remove(&key);
            }
            self.image_last_used.remove(&key);
        }
    }

    /// 统一的 inline 图片插入入口：记录最近使用时间 + 逐出检查。
    pub(crate) fn insert_inline_image(&mut self, key: String, memo: Memo<u32, ImageData>) {
        let now = self.now_ns();
        self.image_last_used.insert(key.clone(), Cell::new(now));
        self.inline_images.insert(key, memo);
        self.evict_images_if_needed();
    }

    /// 统一的 remote 图片插入入口：记录最近使用时间 + 逐出检查。
    pub(crate) fn insert_remote_image(&mut self, url: String, img: ImageData) {
        let now = self.now_ns();
        self.image_last_used.insert(url.clone(), Cell::new(now));
        self.remote_images.insert(url, img);
        self.evict_images_if_needed();
    }
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
        toc: u64,
    ) -> Option<LayoutKey> {
        let m = self.layouts.get(&id)?;
        (m.key.base == base
            && m.key.toc == toc
            && m.key.reveal == m.value.reveal_state(kind, range, caret))
        .then_some(m.key)
    }

    pub(crate) fn layout(
        &self,
        id: BlockId,
        kind: &BlockKind,
        range: Range,
        base: BaseKey,
        caret: usize,
        toc: u64,
    ) -> Option<&BlockLayout> {
        self.layout_key(id, kind, range, base, caret, toc)
            .map(|_| &self.layouts[&id].value)
    }

    /// 纯存在性查询（不校验键）：供几何重算快速跳过 cache miss + 视口外的块——
    /// 避免对 5000+ offscreen 块逐一构造 LayoutKey 并比对 reveal_state。
    pub(crate) fn has_layout(&self, id: BlockId) -> bool {
        self.layouts.contains_key(&id)
    }

    /// 绘制路径用：光栅与布局共用同一键，布局命中后直接取键比对光栅。
    pub(crate) fn raster_key(
        &self,
        id: BlockId,
        kind: &BlockKind,
        range: Range,
        base: BaseKey,
        caret: usize,
        toc: u64,
    ) -> Option<LayoutKey> {
        self.layout_key(id, kind, range, base, caret, toc)
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
        let hit = self.inline_images.get(path).filter(|m| m.matches(&key))?;
        // 命中即刷新最近使用时间：真 LRU（否则退化为按首次插入时间的 FIFO）
        self.touch_image(path);
        Some(&hit.value)
    }

    pub(crate) fn image_failed(&self, path: &str, key: u32) -> bool {
        self.image_failed.get(path).is_some_and(|m| m.matches(&key))
    }

    /// 清除单张图片的成功/失败缓存，供右键“重新加载图片”重试（DEF-T67-E1-005）。
    pub(crate) fn invalidate_image(&mut self, key: &str) {
        self.inline_images.remove(key);
        self.image_failed.remove(key);
        self.remote_images.remove(key);
        self.remote_failed.remove(key);
        self.image_last_used.remove(key);
    }

    /// 远程图片缓存查询（URL 内容寻址，无代次键）。命中即刷新 LRU 最近使用时间。
    pub(crate) fn remote_image(&self, url: &str) -> Option<&ImageData> {
        let hit = self.remote_images.get(url)?;
        self.touch_image(url);
        Some(hit)
    }

    /// 行内图片统一查询：先本地图片（按图片基准代次），再远程会话缓存。
    pub(crate) fn image_or_remote(&self, key: &str, image_base: u32) -> Option<&ImageData> {
        self.image(key, image_base)
            .or_else(|| self.remote_image(key))
    }

    /// 清空全部远程图片状态（位图/失败记忆/pending）：切换文档时调用，
    /// 防止旧文档的请求与结果串到新文档。
    pub(crate) fn clear_remote(&mut self) {
        // 先收集 URL key 以便从 LRU 追踪里移除
        let urls: Vec<String> = self.remote_images.keys().cloned().collect();
        self.remote_images.clear();
        self.remote_failed.clear();
        self.remote_pending.clear();
        for url in urls {
            self.image_last_used.remove(&url);
        }
    }

    pub(crate) fn table(&self, id: BlockId, key: TableKey) -> Option<&TableLayout> {
        self.tables
            .get(&id)
            .filter(|m| m.matches(&key))
            .map(|m| &m.value)
    }

    /// 索引重建驱动的全量回收（不承担正确性）：删除已消失块或过期代次的条目。
    /// 调用点唯一：`refresh_index → geom_fill` 末尾，与块真值同源。
    /// `alive` 以谓词判活（生产直接查 Session id 索引），避免每次击键构建 id 集合。
    pub(crate) fn prune_alive<F: Fn(BlockId) -> bool>(
        &mut self,
        alive: F,
        revs: &Revs,
        base: &BaseKey,
    ) {
        let inline_key = InlineKey {
            scale_q: base.scale_q,
            theme_rev: revs.theme,
        };
        self.layouts
            .retain(|id, m| alive(*id) && m.key.base == *base);
        self.rasters
            .retain(|id, m| alive(*id) && m.key.base == *base);
        self.tables
            .retain(|id, m| alive(*id) && m.key.base == *base);
        self.math_imgs.retain(|id, m| {
            alive(*id)
                && m.key.block == *id
                && m.key.scale_q == base.scale_q
                && m.key.theme_rev == revs.theme
                && m.key.session_rev == revs.session
        });
        self.math_failed.retain(|id, m| {
            alive(*id)
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
    pub(crate) fn prune_viewport(&mut self, keep: &HashSet<BlockId>, base: &BaseKey) {
        self.layouts
            .retain(|id, m| keep.contains(id) && m.key.base == *base);
        self.rasters
            .retain(|id, m| keep.contains(id) && m.key.base == *base);
        self.tables
            .retain(|id, m| keep.contains(id) && m.key.base == *base);
    }
}

/// 键构造：全部来自 App 的外部输入与当前几何，各 ensure/绘制路径共用同一口径。
impl App {
    pub(crate) fn base_key(&self) -> BaseKey {
        // 脚注编号代次随解析一次算好（`Syntax::footnote_epoch`），此处直接读，
        // 不再每帧克隆 id 列表并整表哈希。
        let epoch = self.editor.session().footnote_epoch();
        let mut key = self.input_state.revs.base(
            self.content_width(),
            self.input_state.scale,
            self.image_max_height(),
            self.source_mode(),
            epoch,
        );
        let cfg = &self.config;
        key.font_size_q = quantize(cfg.doc_font_size, 1000.0);
        key.line_height_q = quantize(cfg.doc_line_height, 1000.0);
        key.font_family = cfg.doc_font_family.key();
        key.font_face = face_key(cfg.doc_font_face.as_deref());
        key.doc_margin_q = quantize(self.engine.doc_margin_px(), 100.0);
        key.fold_epoch = self.inline_details_epoch();
        key.list_epoch = self.index.list_epoch;
        key
    }

    /// 行内 details 折叠集合的确定性指纹（与 HashSet 迭代顺序无关）。
    fn inline_details_epoch(&self) -> u64 {
        let mut acc = 0u64;
        for (id, off) in &self.ui_state.expanded_inline_details {
            let mut h = 0xcbf2_9ce4_8422_2325u64;
            h = (h ^ id.0).wrapping_mul(0x0100_0000_01b3);
            h = (h ^ *off as u64).wrapping_mul(0x0100_0000_01b3);
            acc ^= h.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        }
        // 非空集合强制非零，与默认键（空集 = 0）区分。
        if acc == 0 && !self.ui_state.expanded_inline_details.is_empty() {
            1
        } else {
            acc
        }
    }

    /// 当前文档的脚注渲染上下文：按（编号代次, 前缀色）缓存，代次不变时零重建。
    pub(crate) fn footnote_ctx(&mut self) -> std::rc::Rc<FootnoteCtx> {
        let epoch = self.editor.session().footnote_epoch();
        let muted = self.input_state.theme.muted;
        if let Some((e, m, ctx)) = &self.cache.footnote_ctx {
            if *e == epoch && *m == muted {
                return ctx.clone();
            }
        }
        let ctx = std::rc::Rc::new(FootnoteCtx::new(
            self.editor.session().footnote_ids(),
            muted,
        ));
        self.cache.footnote_ctx = Some((epoch, muted, ctx.clone()));
        ctx
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

    /// 图片缓存超上限时按 LRU 逐出最旧条目（TR-3.1）。
    #[test]
    fn image_cache_evicts_oldest_over_limit() {
        let mut c = Cache::default();
        c.set_image_limit_mb(1); // 1 MiB 上限
        let img = || crate::ImageData {
            pixels: vec![0u8; 768 * 1024], // 每张 768 KiB
            width: 512,
            height: 384,
        };
        c.insert_inline_image(
            "a.png".into(),
            Memo {
                key: 1,
                value: img(),
            },
        );
        // 时间戳来自 anchor.elapsed()，间隔 2ms 确保 a 严格早于 b
        std::thread::sleep(std::time::Duration::from_millis(2));
        c.insert_inline_image(
            "b.png".into(),
            Memo {
                key: 1,
                value: img(),
            },
        );
        // 两张共 1.5 MiB > 1 MiB：最旧的 a 被逐出，b 保留
        assert!(c.inline_images.contains_key("b.png"), "最新条目应保留");
        assert!(!c.inline_images.contains_key("a.png"), "最旧条目应被逐出");
        assert!(
            !c.image_last_used.contains_key("a.png"),
            "追踪表须同步清理被逐出条目"
        );
    }

    /// 命中路径刷新最近使用时间：反复绘制的旧图不得先于新的闲置图被逐出（真 LRU）。
    #[test]
    fn image_cache_hit_refreshes_recency() {
        let mut c = Cache::default();
        c.set_image_limit_mb(2); // 2 MiB 上限
        let img = || crate::ImageData {
            pixels: vec![0u8; 768 * 1024], // 每张 768 KiB
            width: 512,
            height: 384,
        };
        c.insert_inline_image(
            "a.png".into(),
            Memo {
                key: 1,
                value: img(),
            },
        );
        // 间隔 2ms 保证插入时间戳严格递增
        std::thread::sleep(std::time::Duration::from_millis(2));
        c.insert_inline_image(
            "b.png".into(),
            Memo {
                key: 1,
                value: img(),
            },
        );
        // 命中 a：a 的最近使用时间被刷新，b 反而成为最久未用
        assert!(c.image("a.png", 1).is_some());
        std::thread::sleep(std::time::Duration::from_millis(2));
        // 第三张使总量 2.25 MiB 超上限：须逐出 b（最久未用），而不是仍在使用中的 a
        c.insert_inline_image(
            "c.png".into(),
            Memo {
                key: 1,
                value: img(),
            },
        );
        assert!(c.inline_images.contains_key("a.png"), "刚命中的 a 应保留");
        assert!(c.inline_images.contains_key("c.png"), "新插入的 c 应保留");
        assert!(
            !c.inline_images.contains_key("b.png"),
            "最久未用的 b 应被逐出"
        );
    }

    /// clear_remote 同步清空远程条目与 LRU 追踪（TR-3.2）。
    #[test]
    fn clear_remote_drops_entries_and_tracking() {
        let mut c = Cache::default();
        let url = "https://example.com/a.png".to_string();
        c.insert_remote_image(
            url.clone(),
            crate::ImageData {
                pixels: vec![0u8; 16],
                width: 2,
                height: 2,
            },
        );
        assert!(c.remote_images.contains_key(&url));
        assert!(c.image_last_used.contains_key(&url));
        c.clear_remote();
        assert!(c.remote_images.is_empty(), "远程缓存须清空");
        assert!(c.image_last_used.is_empty(), "LRU 追踪须同步清空");
    }

    #[test]
    fn base_key_quantizes_stably_and_tracks_inputs() {
        let revs = Revs::default();
        let b = revs.base(800.0, 1.25, 600.0, false, 0);
        assert_eq!(
            b,
            revs.base(800.0, 1.25, 600.0, false, 0),
            "同一输入必得同键"
        );
        assert_ne!(
            b,
            revs.base(800.5, 1.25, 600.0, false, 0),
            "版心宽变化即换键"
        );
        assert_ne!(b, revs.base(800.0, 1.5, 600.0, false, 0), "DPI 变化即换键");
        assert_ne!(
            b,
            revs.base(800.0, 1.25, 600.5, false, 0),
            "独占图片限高变化即换键（按整数像素量化）"
        );
        assert_ne!(
            b,
            revs.base(800.0, 1.25, 300.0, false, 0),
            "窗口高度变化即换键"
        );
        assert_eq!(
            b,
            revs.base(800.0, 1.25, 600.2, false, 0),
            "亚像素抖动不得换键"
        );
        assert_ne!(
            b,
            revs.base(800.0, 1.25, 600.0, true, 0),
            "源码模式变化即换键"
        );
        assert_ne!(
            b,
            revs.base(800.0, 1.25, 600.0, false, 1),
            "脚注代次变化即换键"
        );
        let revs2 = Revs { theme: 1, ..revs };
        assert_ne!(
            b,
            revs2.base(800.0, 1.25, 600.0, false, 0),
            "主题代次变化即换键"
        );
        let revs3 = Revs {
            image_base: 1,
            ..revs
        };
        assert_ne!(
            b,
            revs3.base(800.0, 1.25, 600.0, false, 0),
            "图片基准变化即换键"
        );
        let revs5 = Revs {
            remote_images: 1,
            ..revs
        };
        assert_ne!(
            b,
            revs5.base(800.0, 1.25, 600.0, false, 0),
            "远程图片代次变化即换键"
        );
        let revs4 = Revs { session: 1, ..revs };
        assert_ne!(
            b,
            revs4.base(800.0, 1.25, 600.0, false, 0),
            "新会话（BlockId 复用）即换键"
        );
    }

    #[test]
    fn prune_keeps_only_current_generation() {
        let ids = ids();
        let mut c = Cache::default();
        let revs = Revs::default();
        let base = revs.base(800.0, 1.0, 600.0, false, 0);
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
        c.prune_alive(|id| alive.contains(&id), &revs, &base);
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
        let base2 = revs2.base(800.0, 1.0, 600.0, false, 0);
        c2.prune_alive(|id| alive.contains(&id), &revs2, &base2);
        assert!(c2.inline_math.is_empty(), "公式图随主题作废");
        assert!(
            c2.math_failed.contains_key(&ids[0]),
            "失败记忆与颜色无关，保留"
        );
        assert!(c2.inline_images.contains_key("img"), "图片与主题无关，保留");
        // DPI 变化：失败记忆过期
        let revs3 = Revs { theme: 1, ..revs };
        let base3 = revs3.base(800.0, 2.0, 600.0, false, 0);
        c2.prune_alive(|id| alive.contains(&id), &revs3, &base3);
        assert!(!c2.inline_failed.contains_key("y"), "失败记忆随 DPI 作废");
        // 新会话：块相关条目过期，行内公式与失败记忆跨文档保留
        let revs4 = Revs {
            theme: 1,
            session: 1,
            image_base: 1,
            ..revs
        };
        let base4 = revs4.base(800.0, 2.0, 600.0, false, 0);
        // 行内失败记忆按当前 DPI 代次重新登记（上一轮 DPI 变化已使其过期）
        c2.inline_failed.insert(
            "y".into(),
            Memo {
                key: base4.scale_q,
                value: (),
            },
        );
        c2.prune_alive(|id| alive.contains(&id), &revs4, &base4);
        assert!(c2.math_failed.is_empty(), "新会话：块失效记忆作废");
        assert!(c2.inline_images.is_empty(), "新图片基准：位图作废");
        assert!(c2.inline_failed.contains_key("y"), "行内失败记忆跨文档保留");
    }

    #[test]
    fn prune_viewport_reclaims_non_visible_rasters() {
        let ids = ids();
        let mut c = Cache::default();
        let revs = Revs::default();
        let base = revs.base(800.0, 1.0, 600.0, false, 0);
        let lk0 = LayoutKey {
            base,
            reveal: 0,
            toc: 0,
            highlight_epoch: 0,
        };
        let lk1 = LayoutKey {
            base,
            reveal: 0,
            toc: 0,
            highlight_epoch: 0,
        };

        // 为 ids[0] 和 ids[1] 填充 rasters（值 = (像素, 宽, 高) 易构造）
        c.rasters.insert(
            ids[0],
            Memo {
                key: lk0,
                value: (vec![0u8; 4], 1, 1),
            },
        );
        c.rasters.insert(
            ids[1],
            Memo {
                key: lk1,
                value: (vec![0u8; 4], 1, 1),
            },
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
        let base2 = revs2.base(800.0, 1.0, 600.0, false, 0);
        // 用 ids[1] 插 base2 条目（ids[0] 已经被 prune_viewport 保留且还是 base 的条目）
        c.rasters.insert(
            ids[1],
            Memo {
                key: LayoutKey {
                    base: base2,
                    reveal: 0,
                    toc: 0,
                    highlight_epoch: 0,
                },
                value: (vec![0u8; 4], 1, 1),
            },
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
        let base = revs.base(800.0, 1.0, 600.0, false, 0);

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
        assert!(
            c.math_imgs.contains_key(&ids[0]),
            "块级公式图（MathKey 索引）不被 viewport prune 剪"
        );
        assert!(
            c.inline_math.contains_key("x"),
            "行内公式（tex 内容寻址）不被 viewport prune 剪"
        );
    }
}
