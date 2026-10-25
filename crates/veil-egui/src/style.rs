//! inline 节点 → 样式 spans（致密覆盖块文本，`cosmic-text` set_rich_text 的输入要求）。
//!
//! v0 取舍（逐条钉在测试上）：
//! - 样式恒按整节点（含标记）产出；标记消隐（Reveal）由 layout 层 OffsetMap 映射完成。
//! - 样式叠加走并集（Strong 内嵌 Emph → bold+italic）。
//! - Link 整 range（含 URL）下划线；行内 Math 不加样式（行内公式渲染属后续切片）。
//! - 标题放大是块级属性，由调用方乘进 metrics，不在 spans 内。

use veil_core::{InlineKind, InlineNode, Range};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub mono: bool,
    pub underline: bool,
}

/// 产出致密覆盖 `[0, text_len)` 的样式段。`inlines` 为文档绝对偏移，`block` 为块区间。
/// text_len 不含块尾换行（shell 布局前已剥除）。
pub fn style_spans(
    text_len: usize,
    block: Range,
    inlines: &[InlineNode],
) -> Vec<(usize, usize, Style)> {
    let mut marks: Vec<(usize, usize, Style)> = Vec::new();
    collect(inlines, block, &mut marks);
    // 扫描线：边界切分 → 逐段求并集样式 → 合并相邻同式段
    let mut bounds: Vec<usize> = vec![0, text_len];
    for &(s, e, _) in &marks {
        bounds.push(s);
        bounds.push(e);
    }
    bounds.sort_unstable();
    bounds.dedup();
    let mut out: Vec<(usize, usize, Style)> = Vec::new();
    for w in bounds.windows(2) {
        let (s, e) = (w[0], w[1]);
        if s >= e || s >= text_len {
            continue;
        }
        let mut st = Style::default();
        for &(ms, me, mst) in &marks {
            if ms <= s && e <= me {
                st.bold |= mst.bold;
                st.italic |= mst.italic;
                st.mono |= mst.mono;
                st.underline |= mst.underline;
            }
        }
        match out.last_mut() {
            Some(last) if last.2 == st && last.1 == s => last.1 = e,
            _ => out.push((s, e, st)),
        }
    }
    out
}

fn collect(nodes: &[InlineNode], block: Range, out: &mut Vec<(usize, usize, Style)>) {
    for n in nodes {
        let style = match n.kind {
            InlineKind::Strong => Style {
                bold: true,
                ..Default::default()
            },
            InlineKind::Emph => Style {
                italic: true,
                ..Default::default()
            },
            InlineKind::Code => Style {
                mono: true,
                ..Default::default()
            },
            InlineKind::Link => Style {
                underline: true,
                ..Default::default()
            },
            // Image 非激活时整段被占位图替换（layout），无可见文本需要样式；
            // 降级态（加载失败/远程 URL）只显 alt 文本，与正文同式。
            InlineKind::Image | InlineKind::Math => Style::default(),
        };
        if style != Style::default() {
            let s = n.range.start.saturating_sub(block.start);
            let e = n.range.end.saturating_sub(block.start);
            if s < e {
                out.push((s, e, style));
            }
        }
        collect(&n.children, block, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use veil_core::Session;

    /// 便捷：以 Session 解析拿真实 inline 树，保证与内核语义一致。
    fn spans(text: &str) -> Vec<(usize, usize, Style)> {
        let s = Session::open(text);
        let (id, _, range) = s.blocks()[0];
        style_spans(range.len(), range, &s.block_inlines(id).unwrap())
    }

    fn find(spans: &[(usize, usize, Style)], at: usize) -> Style {
        spans
            .iter()
            .find(|(s, e, _)| *s <= at && at < *e)
            .map(|x| x.2)
            .unwrap_or_default()
    }

    #[test]
    fn plain_text_single_default_span() {
        assert_eq!(spans("hello"), vec![(0, 5, Style::default())]);
    }

    #[test]
    fn strong_covers_markers() {
        // 「**b**」整节点（含标记）bold
        let sp = spans("a **b** c");
        assert!(find(&sp, 2).bold && find(&sp, 4).bold && find(&sp, 6).bold);
        assert!(!find(&sp, 0).bold && !find(&sp, 8).bold);
    }

    #[test]
    fn dense_cover_no_gaps_no_overlap() {
        for text in ["a **b** `c` [d](u) $e$ f", "*i* normal **b**"] {
            let sp = spans(text);
            let mut pos = 0;
            for &(s, e, _) in &sp {
                assert_eq!(s, pos, "致密覆盖不得有缝/重叠: {text}");
                pos = e;
            }
            assert_eq!(pos, text.len());
        }
    }

    #[test]
    fn nested_styles_union() {
        // `**a *b* c**`：b 段 bold+italic，其余 strong 段仅 bold
        let sp = spans("**a *b* c**");
        let b = find(&sp, 5); // 'b'
        assert!(b.bold && b.italic);
        let a = find(&sp, 2); // 'a'
        assert!(a.bold && !a.italic);
    }

    #[test]
    fn code_mono_math_plain() {
        let sp = spans("`x` $y$");
        assert!(find(&sp, 1).mono);
        assert_eq!(find(&sp, 5), Style::default());
    }

    #[test]
    fn link_underlines_url_too() {
        // 「[t](http://a)」整节点下划线
        let sp = spans("[t](http://a)");
        assert!(find(&sp, 0).underline && find(&sp, 12).underline);
    }

    #[test]
    fn multibyte_offsets_stay_bytes() {
        // 「中文 **粗**」：strong 起点 7（字节）
        let sp = spans("中文 **粗**");
        assert!(find(&sp, 9).bold);
        assert!(!find(&sp, 3).bold);
    }
}
