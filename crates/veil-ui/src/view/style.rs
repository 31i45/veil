//! inline 节点 → 样式 spans（致密覆盖块文本，`cosmic-text` set_rich_text 的输入要求）。
//!
//! v0 取舍（逐条钉在测试上）：
//! - 样式恒按整节点（含标记）产出；标记消隐（Reveal）由 layout 层 OffsetMap 映射完成。
//! - 样式叠加走并集（Strong 内嵌 Emph → bold+italic）。
//! - Link 整 range（含 URL）置 link 标记，由 layout 按主题链接色上色；行内 Math 不加样式。
//! - 标题放大是块级属性，由调用方乘进 metrics，不在 spans 内。

use veil_core::{InlineKind, InlineNode, Range};

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub mono: bool,
    /// 链接标记：layout 在无显式前景色时使用主题链接色。
    pub link: bool,
    /// 删除线：由 paint 层按区间绘制横线（cosmic-text 无原生支持）。
    pub strikethrough: bool,
    /// 语法高亮前景色；None 表示沿用主题默认前景色。
    pub color: Option<[u8; 3]>,
    /// Sub/Sup 缩字系数（HTML5：约 0.75）。None 表示不缩。
    /// layout 层 attrs_of 会乘以 metrics.size 应用到 cosmic-text Attrs::size。
    pub size_factor: Option<f32>,
    /// Sub/Sup 基线偏移方向（em 的符号倍率）：-1 下沉（sub）、+1 上浮（sup）。
    /// layout 层转成物理像素偏移，paint 光栅化时按字形区间施加。
    pub baseline_shift: Option<f32>,
    /// Mark 底色启用位（Markdown `==x==` 与 HTML `<mark>`）。`false` 表示不画底色。
    /// 实际色值由绘制层取 `Theme::mark_bg`（浅色柔黄 / 深色暗琥珀 / 高对比度暗黄），
    /// 样式层不携带色值——否则深色主题会沿用写死的浅色。
    pub mark_bg: bool,
    /// Abbr 虚线下划线（HTML `<abbr>`）。paint 层绘制。
    pub dotted_underline: bool,
    /// Kbd 边框标记（HTML `<kbd>`）。paint 层绘制圆角矩形。
    pub kbd_frame: bool,
    /// 行内代码底色（Markdown `` `x` `` 与 HTML `<code>`/`<pre>`）。paint 层绘制圆角底色。
    pub code_bg: bool,
    /// `style="background-color:..."` 的底色；paint 层按此色填充字形矩形。
    pub bg: Option<[u8; 3]>,
    /// `style="text-decoration:underline"`（含 `<u>` / `<ins>`）：paint 层画实线下划线。
    pub underline: bool,
    /// `style="text-decoration:overline"`：paint 层画上划线。
    pub overline: bool,
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
                st.link |= mst.link;
                st.strikethrough |= mst.strikethrough;
                st.kbd_frame |= mst.kbd_frame;
                st.dotted_underline |= mst.dotted_underline;
                st.code_bg |= mst.code_bg;
                st.mark_bg |= mst.mark_bg;
                st.underline |= mst.underline;
                st.overline |= mst.overline;
                if st.bg.is_none() {
                    st.bg = mst.bg;
                }
                // Option<f32> / Option<[u8;3]> 取第一个非 None（白名单 HTML 标签互斥，
                // 同一段不会同时被 Sub 和 Sup 或 Mark 和 Abbr 覆盖）。
                if st.size_factor.is_none() {
                    st.size_factor = mst.size_factor;
                }
                if st.baseline_shift.is_none() {
                    st.baseline_shift = mst.baseline_shift;
                }
                if st.color.is_none() {
                    st.color = mst.color;
                }
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
                code_bg: true,
                ..Default::default()
            },
            InlineKind::Link => Style {
                link: true,
                ..Default::default()
            },
            InlineKind::Strikethrough => Style {
                strikethrough: true,
                ..Default::default()
            },
            // N3c：`==x==` 与 `<mark>` 同一底色 token；色值由绘制层取 `Theme::mark_bg`。
            InlineKind::Highlight => Style {
                mark_bg: true,
                ..Default::default()
            },
            // Image 非激活时整段被占位图替换（layout），无可见文本需要样式；
            // 降级态（加载失败/远程 URL）只显 alt 文本，与正文同式。
            // FootnoteRef 的 `[n]` 是 layout 层替换文本，由 layout 按其替换区间补链接色；
            // 样式层不置 link（否则表格单元格等纯文本降级路径也会被染色）。
            InlineKind::Image | InlineKind::Math | InlineKind::FootnoteRef => Style::default(),
            // Task 3-5：行内白名单 HTML（开+闭）。
            // kbd: 粗体；code: 等宽；a: 链接色；sub/sup/mark/br/abbr/img 不在 Style 层处理
            // （sub/sup 的字体缩放+基线偏移、mark 底色、br 断行、abbr 下划线、img 占位
            // 都由 layout 层单独分派；样式层产出默认 span 即可让 cosmic-text 正常排字形）。
            InlineKind::Html { tag, ref attrs } => {
                let mut st = match tag {
                    veil_core::HtmlTag::Kbd => Style {
                        bold: true,
                        kbd_frame: true,
                        ..Default::default()
                    },
                    veil_core::HtmlTag::Code | veil_core::HtmlTag::Pre => Style {
                        mono: true,
                        code_bg: true,
                        ..Default::default()
                    },
                    veil_core::HtmlTag::A => Style {
                        link: true,
                        ..Default::default()
                    },
                    // 语义映射与导出 CSS / PDF 一致：strong/b 粗体、em/i/cite 斜体、
                    // u/ins 下划线、s/del 删除线、var 斜体等宽、samp 等宽、small 0.85em。
                    veil_core::HtmlTag::Strong | veil_core::HtmlTag::B => Style {
                        bold: true,
                        ..Default::default()
                    },
                    veil_core::HtmlTag::Em | veil_core::HtmlTag::I | veil_core::HtmlTag::Cite => {
                        Style {
                            italic: true,
                            ..Default::default()
                        }
                    }
                    veil_core::HtmlTag::Var => Style {
                        italic: true,
                        mono: true,
                        ..Default::default()
                    },
                    veil_core::HtmlTag::Samp => Style {
                        mono: true,
                        ..Default::default()
                    },
                    veil_core::HtmlTag::U | veil_core::HtmlTag::Ins => Style {
                        underline: true,
                        ..Default::default()
                    },
                    veil_core::HtmlTag::S | veil_core::HtmlTag::Del => Style {
                        strikethrough: true,
                        ..Default::default()
                    },
                    veil_core::HtmlTag::Small => Style {
                        size_factor: Some(0.85),
                        ..Default::default()
                    },
                    // Sub/Sup：HTML5 推荐缩字 ~0.75 倍（cosmic-text Attrs::size 接收 pt 值，
                    // 直接乘 metrics pt 即可；y 轴偏移由 paint 层按类型决定——sub 下移、sup 上移）。
                    veil_core::HtmlTag::Sub => Style {
                        size_factor: Some(0.75),
                        baseline_shift: Some(1.0),
                        ..Default::default()
                    },
                    veil_core::HtmlTag::Sup => Style {
                        size_factor: Some(0.75),
                        baseline_shift: Some(-1.0),
                        ..Default::default()
                    },
                    // Mark：HTML5 语义高亮；色值由绘制层取 `Theme::mark_bg`。
                    veil_core::HtmlTag::Mark => Style {
                        mark_bg: true,
                        ..Default::default()
                    },
                    // Abbr：虚线下划线提示可展开缩写；tooltip 由 shell 层负责，此处只绘下划线。
                    veil_core::HtmlTag::Abbr => Style {
                        dotted_underline: true,
                        ..Default::default()
                    },
                    // q 的双引号由 layout/PDF 插入可见文本；span 无默认样式但承载 style。
                    _ => Style::default(),
                };
                // style 属性（解析层已净化）：叠加到标签语义之上。
                if let Some(raw) = attrs.style.as_deref() {
                    let hs = veil_core::HtmlStyle::parse(raw);
                    st.bold |= hs.bold;
                    st.italic |= hs.italic;
                    st.underline |= hs.underline;
                    st.strikethrough |= hs.strike;
                    st.overline |= hs.overline;
                    st.mono |= hs.mono;
                    if hs.color.is_some() {
                        st.color = hs.color;
                    }
                    if hs.background_color.is_some() {
                        st.bg = hs.background_color;
                    }
                    if st.size_factor.is_none() {
                        if let Some(px) = hs.size_px {
                            // CSS 参考字号 16px；渲染时按正文字号比例缩放。
                            st.size_factor = Some(px / 16.0);
                        }
                    }
                }
                st
            }
            // 自闭合白名单：br 断行、img 占位，样式层都产出默认；layout/paint 另行处理。
            InlineKind::HtmlSelfClosed { .. } => Style::default(),
            // 非白名单 HTML 源码原样：不染色，按纯文本 span。
            InlineKind::HtmlLiteral => Style::default(),
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
        let (id, _, range) = s.blocks()[0].clone();
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
    fn link_marks_text_and_url_but_not_body() {
        // 「[t](http://a)」整节点（文本 + URL）link；前后正文不置位
        let sp = spans("a [t](http://a) b");
        assert!(find(&sp, 3).link, "链接文本");
        assert!(find(&sp, 6).link, "URL 起点");
        assert!(find(&sp, 14).link, "URL 末端");
        for at in 2..15 {
            assert!(find(&sp, at).link, "链接整段都应置位: at={at}");
        }
        assert!(!find(&sp, 0).link && !find(&sp, 16).link, "正文不得置位");
    }

    #[test]
    fn multibyte_offsets_stay_bytes() {
        // 「中文 **粗**」：strong 起点 7（字节）
        let sp = spans("中文 **粗**");
        assert!(find(&sp, 9).bold);
        assert!(!find(&sp, 3).bold);
    }

    #[test]
    fn strikethrough_covers_markers() {
        // 「~~del~~」整节点（含标记）strikethrough
        let sp = spans("a ~~del~~ c");
        assert!(find(&sp, 2).strikethrough && find(&sp, 8).strikethrough);
        assert!(!find(&sp, 0).strikethrough && !find(&sp, 10).strikethrough);
    }

    #[test]
    fn html_semantic_style_attr_maps_to_color_and_weights() {
        // <strong style="color:red">VISUAL</strong>：内容段 bold + 红色
        let sp = spans("<strong style=\"color:red\">VISUAL</strong>");
        let st = find(&sp, 26);
        assert!(st.bold, "strong 应粗体");
        assert_eq!(st.color, Some([255, 0, 0]), "color:red 应映射为前景色");
        // em/i/u/s/del/ins 语义映射
        assert!(find(&spans("<em>x</em>"), 1).italic);
        assert!(find(&spans("<i>x</i>"), 1).italic);
        assert!(find(&spans("<u>x</u>"), 1).underline);
        assert!(find(&spans("<ins>x</ins>"), 1).underline);
        assert!(find(&spans("<s>x</s>"), 1).strikethrough);
        assert!(find(&spans("<del>x</del>"), 1).strikethrough);
        assert!(find(&spans("<small>x</small>"), 1).size_factor == Some(0.85));
        assert!(find(&spans("<samp>x</samp>"), 1).mono);
        let var = find(&spans("<var>x</var>"), 1);
        assert!(var.italic && var.mono, "var 应斜体等宽");
        // style 背景色 / 删除线 / 上划线
        let bg = find(&spans("<span style=\"background-color:#ff0\">x</span>"), 1);
        assert_eq!(bg.bg, Some([255, 255, 0]));
        let deco = find(
            &spans("<span style=\"text-decoration:underline line-through overline\">x</span>"),
            1,
        );
        assert!(deco.underline && deco.strikethrough && deco.overline);
        // 危险声明被丢弃，合法声明保留
        let safe = find(
            &spans("<span style=\"color:red;position:fixed\">x</span>"),
            1,
        );
        assert_eq!(safe.color, Some([255, 0, 0]));
    }

    #[test]
    fn strikethrough_nests_with_strong() {
        // 「~~**x**~~」：内层同时 bold + strikethrough
        let sp = spans("~~**x**~~");
        let mid = find(&sp, 4);
        assert!(mid.strikethrough && mid.bold);
    }

    #[test]
    fn highlight_uses_mark_bg_token() {
        // `==hi==` 整节点（含标记）与 `<mark>` 同一底色标记，正文色不变；
        // 色值由绘制层取 Theme::mark_bg，样式层只置位。
        let sp = spans("a ==hi== b");
        assert!(find(&sp, 3).mark_bg);
        assert!(find(&sp, 5).mark_bg);
        assert_eq!(find(&sp, 0), Style::default());
        // 与 `<mark>` 标签输出完全同式
        assert_eq!(
            find(&sp, 3),
            find(&spans("<mark>hi</mark>"), 7),
            "==x== 与 <mark> 应产出同一样式"
        );
    }

    #[test]
    fn highlight_nests_with_strong() {
        let sp = spans("==**x**==");
        let mid = find(&sp, 4);
        assert!(mid.mark_bg && mid.bold);
    }
}
