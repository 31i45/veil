//! chrome 标签位图缓存：菜单/状态栏/弹窗文字经 preedit 排版光栅化后复用。
//! 键 = (scale, 前景色, 文本)——三者任一变化即须取新位图；主题切换/DPI 变化由
//! `Chrome::clear_label_cache` 整体作废。

use std::collections::HashMap;

use crate::view::LayoutEngine;

#[derive(Clone)]
pub(crate) struct CachedLabel {
    pub(crate) pixels: Vec<u8>,
    pub(crate) w: u32,
    pub(crate) h: u32,
}

/// 标签位图缓存（默认空）。
#[derive(Default)]
pub(crate) struct LabelCache {
    map: HashMap<String, CachedLabel>,
}

impl LabelCache {
    pub(crate) fn clear(&mut self) {
        self.map.clear();
    }

    /// 取标签位图；缺失时经 engine 排版光栅化后写入。
    pub(crate) fn get_or_insert(
        &mut self,
        engine: &mut LayoutEngine,
        text: &str,
        color: [u8; 3],
        scale: f32,
    ) -> Option<&CachedLabel> {
        let key = label_key(scale, text, color);
        if !self.map.contains_key(&key) {
            let layout = engine.layout_preedit(text);
            let (pixels, w, h) = engine.rasterize(&layout, color);
            self.map.insert(key.clone(), CachedLabel { pixels, w, h });
        }
        self.map.get(&key)
    }

    /// 缓存项数（测试观测用：同帧重复绘制不得增长）。
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.map.len()
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

pub(crate) fn label_key(scale: f32, text: &str, color: [u8; 3]) -> String {
    format!(
        "{scale:?}|{:02x}{:02x}{:02x}|{text}",
        color[0], color[1], color[2]
    )
}
