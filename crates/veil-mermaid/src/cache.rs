//! 双层缓存（spec「栅格化与缓存」）：`(源码哈希, 主题指纹)` → SVG（命中跳过
//! JS 布局、只重栅格化），`(SVG 键, scale)` → 位图（命中零拷贝克隆 Arc）。
//!
//! 上限保护：任一层超 128 条整体 clear（两层同清维持「位图存在 ⇒ SVG 存在」
//! 不变式，render() 从位图缓存命中回查 SVG 依赖它）。不做 LRU——容量受控且
//! 实现最简，淘汰整层只损失一次性重渲染。

use std::collections::HashMap;
use std::sync::Arc;

use veil_ui::MermaidBitmap;

const MAX_ENTRIES: usize = 128;

pub(crate) struct Cache {
    /// SVG 条目连同解析出的宽高（render() 组装场景 stub 时免二次解析）。
    svg: HashMap<u64, (Arc<str>, u32, u32)>,
    bitmap: HashMap<u64, MermaidBitmap>,
}

impl Cache {
    pub(crate) fn new() -> Self {
        Self {
            svg: HashMap::new(),
            bitmap: HashMap::new(),
        }
    }

    pub(crate) fn get_svg(&self, key: u64) -> Option<(Arc<str>, u32, u32)> {
        self.svg.get(&key).cloned()
    }

    pub(crate) fn put_svg(&mut self, key: u64, value: (Arc<str>, u32, u32)) {
        if self.svg.len() >= MAX_ENTRIES {
            self.svg.clear();
            self.bitmap.clear();
        }
        self.svg.insert(key, value);
    }

    pub(crate) fn get_bitmap(&self, key: u64) -> Option<MermaidBitmap> {
        self.bitmap.get(&key).cloned()
    }

    pub(crate) fn put_bitmap(&mut self, key: u64, value: MermaidBitmap) {
        if self.bitmap.len() >= MAX_ENTRIES {
            self.svg.clear();
            self.bitmap.clear();
        }
        self.bitmap.insert(key, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bitmap(w: u32) -> MermaidBitmap {
        MermaidBitmap {
            rgba: Arc::new(vec![0; (w as usize) * 4]),
            width: w,
            height: 1,
        }
    }

    #[test]
    fn overflow_clears_both_layers_together() {
        let mut c = Cache::new();
        c.put_svg(1, (Arc::from("<svg/>"), 1, 1));
        // 第 129 条触发整层 clear（含位图层）。
        for i in 0..MAX_ENTRIES {
            c.put_bitmap(1000 + i as u64, bitmap(2));
        }
        c.put_bitmap(0, bitmap(2));
        assert!(c.get_svg(1).is_none(), "溢出应连带清空另一层");
        assert!(c.get_bitmap(1000).is_none(), "溢出应清空旧条目");
    }

    #[test]
    fn get_returns_clone_of_stored_value() {
        let mut c = Cache::new();
        let bm = bitmap(4);
        c.put_bitmap(7, bm.clone());
        let got = c.get_bitmap(7).unwrap();
        assert_eq!(got.width, 4);
        assert!(Arc::ptr_eq(&got.rgba, &bm.rgba), "缓存命中应零拷贝");
        assert!(c.get_bitmap(8).is_none());
    }
}
