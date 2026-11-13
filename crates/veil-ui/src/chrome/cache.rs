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
        self.get_or_insert_styled(engine, text, color, scale, false)
    }

    /// 模态标题位图：字号/字重与正文不同，键加前缀避免与普通 UI 文本串味。
    pub(crate) fn get_or_insert_title(
        &mut self,
        engine: &mut LayoutEngine,
        text: &str,
        color: [u8; 3],
        scale: f32,
    ) -> Option<&CachedLabel> {
        self.get_or_insert_styled(engine, text, color, scale, true)
    }

    fn get_or_insert_styled(
        &mut self,
        engine: &mut LayoutEngine,
        text: &str,
        color: [u8; 3],
        scale: f32,
        title: bool,
    ) -> Option<&CachedLabel> {
        let key = if title {
            format!("t|{}", label_key(scale, text, color))
        } else {
            label_key(scale, text, color)
        };
        if !self.map.contains_key(&key) {
            let layout = if title {
                engine.layout_ui_title(text)
            } else {
                engine.layout_ui(text)
            };
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
