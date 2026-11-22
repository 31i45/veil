use super::*;
use crate::theme::Theme;
use veil_core::Session;

pub(super) fn layout_of(text: &str, width: f32) -> (LayoutEngine, BlockLayout) {
    // usize::MAX 表示「无光标」：一切可消隐节点均处于非激活态
    layout_with_caret(text, usize::MAX, width)
}

/// caret 为文档绝对偏移（激活判定用）。
pub(super) fn layout_with_caret(
    text: &str,
    caret: usize,
    width: f32,
) -> (LayoutEngine, BlockLayout) {
    layout_with_imgs(text, caret, width, &[])
}

/// imgs 为已渲染的非激活行内公式（切片 8 占位流程的输入侧）。
pub(super) fn layout_with_imgs(
    text: &str,
    caret: usize,
    width: f32,
    imgs: &[InlineImg],
) -> (LayoutEngine, BlockLayout) {
    let s = Session::open(text);
    let (id, kind, range) = s.blocks()[0].clone();
    let inlines = s.block_inlines(id).unwrap();
    let mut eng = LayoutEngine::new();
    let block_text = &text[range.start..range.end];
    let l = eng.layout_block(kind, block_text, range, &inlines, width, caret, imgs, false);
    (eng, l)
}

/// 按文档块序号排版（容器/嵌套回归用）：`caret` 为文档绝对偏移。
pub(super) fn layout_doc_block(
    text: &str,
    block_idx: usize,
    caret: usize,
    width: f32,
) -> (LayoutEngine, BlockLayout) {
    let s = Session::open(text);
    let (id, kind, range) = s.blocks()[block_idx].clone();
    let inlines = s.block_inlines(id).unwrap();
    let mut eng = LayoutEngine::new();
    let block_text = &text[range.start..range.end];
    let l = eng.layout_block(kind, block_text, range, &inlines, width, caret, &[], false);
    (eng, l)
}

/// captions 为独立成行图片的题注输入（alt 非空时由 shell 派生）。
pub(super) fn layout_with_captions(
    text: &str,
    caret: usize,
    width: f32,
    imgs: &[InlineImg],
    captions: &[ImageCaption],
) -> (LayoutEngine, BlockLayout) {
    let s = Session::open(text);
    let (id, kind, range) = s.blocks()[0].clone();
    let inlines = s.block_inlines(id).unwrap();
    let mut eng = LayoutEngine::new();
    let block_text = &text[range.start..range.end];
    let l = eng.layout_block_with_captions(
        kind,
        block_text,
        range,
        &inlines,
        width,
        caret,
        imgs,
        false,
        None,
        &[],
        None,
        false,
        captions,
    );
    (eng, l)
}

pub(super) fn caption(range: Range, text: &str) -> ImageCaption {
    ImageCaption {
        range,
        text: text.to_string(),
    }
}

/// 整篇逐块排版：按 Session 块顺序派生有序列表编号后排版每个块。
/// 单块辅助函数拿不到文档顺序，无法验证跨块递增编号。
pub(super) fn layout_blocks_with_caret(
    src: &str,
    caret: usize,
    width: f32,
) -> (LayoutEngine, Vec<BlockLayout>) {
    let s = Session::open(src);
    let blocks = s.blocks();
    let items = blocks.iter().map(|(id, kind, range)| {
        let marker = match kind {
            BlockKind::ListItem { .. } => list_marker_of(kind, &src[range.start..range.end]),
            _ => ListMarker::None,
        };
        (*id, kind, marker)
    });
    let ordinals = ordered_list_ordinals(items);
    let mut eng = LayoutEngine::new();
    let layouts = blocks
        .iter()
        .map(|(id, kind, range)| {
            let inlines = s.block_inlines(*id).unwrap_or_default();
            eng.layout_block_with_footnotes(
                kind.clone(),
                &src[range.start..range.end],
                *range,
                &inlines,
                width,
                caret,
                &[],
                false,
                None,
                &[],
                ordinals.get(id).copied(),
                false,
            )
        })
        .collect();
    (eng, layouts)
}

/// 拼回整篇排版文本（块间以 `\n` 连接），便于断言跨块编号。
pub(super) fn block_texts(layouts: &[BlockLayout]) -> String {
    layouts
        .iter()
        .map(|l| l.layout_text())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn hit_then_caret_roundtrips_ascii() {
    let (eng, l, ..) = layout_of("hello world", 200.0);
    let (x, y, _) = eng.caret_xy(&l, 5);
    assert_eq!(eng.hit(&l, x + 0.1, y + 1.0), 5, "光标 x 命中应回原偏移");
}

#[test]
fn pick_family_prefers_first_available_and_falls_back() {
    let sans = typography::SANS_FAMILIES;
    assert_eq!(
        pick_family(sans, |n| n == "Microsoft YaHei UI"),
        Some("Microsoft YaHei UI")
    );
    assert_eq!(pick_family(sans, |n| n == "Segoe UI"), Some("Segoe UI"));
    assert_eq!(pick_family(sans, |_| false), None);
}

#[test]
fn ui_text_is_smaller_than_body_preedit() {
    let mut eng = LayoutEngine::new();
    let ui = eng.layout_ui("Aa中");
    let body = eng.layout_preedit("Aa中");
    assert!((ui.height - typography::UI_LINE_HEIGHT).abs() < 0.5);
    assert!((body.height - typography::BODY_LINE_HEIGHT).abs() < 0.5);
    assert!(ui.height < body.height);
}

#[test]
fn code_block_is_smaller_than_paragraph() {
    let mut eng = LayoutEngine::new();
    let text = "let x = 1;";
    let block = Range {
        start: 0,
        end: text.len(),
    };
    let cod = eng.layout_block(
        BlockKind::FencedCode(None),
        text,
        block,
        &[],
        500.0,
        usize::MAX,
        &[],
        false,
    );
    let par = eng.layout_block(
        BlockKind::Paragraph,
        text,
        block,
        &[],
        500.0,
        usize::MAX,
        &[],
        false,
    );
    let expected = typography::BODY_LINE_HEIGHT * typography::CODE_SCALE;
    assert!((cod.height - expected).abs() < 0.5);
    assert!(cod.height < par.height);
}
#[test]
fn caret_advances_monotonically() {
    let (eng, l, ..) = layout_of("abcdef", 400.0);
    let mut prev = -1.0;
    for i in 0..=6 {
        let (x, _, _) = eng.caret_xy(&l, i);
        assert!(x >= prev, "光标 x 随偏移单调不减");
        prev = x;
    }
}

#[test]
fn multibyte_hit_stays_on_grapheme_boundary() {
    // 「中a文」：点击「中」右半应落 3（hit 内建 grapheme 切分），不落 1/2
    let (eng, l, ..) = layout_of("中a文", 200.0);
    let (x1, y, _) = eng.caret_xy(&l, 3);
    let (x0, _, _) = eng.caret_xy(&l, 0);
    let mid = (x0 + x1) / 2.0 + 0.1;
    let off = eng.hit(&l, mid, y + 1.0);
    assert!(off == 0 || off == 3, "多字节字符内命中不得切碎: {off}");
}

#[test]
fn hit_second_line_maps_with_base() {
    let (eng, l, ..) = layout_of("ab\ncd", 200.0);
    // 第二行行首命中 → 文档偏移 3（a b \n 之后）
    let (_, y, h) = eng.caret_xy(&l, 3);
    assert!(y > 0.0, "第二行 y 应大于 0");
    assert!(h > 0.0);
    let off = eng.hit(&l, 0.0, y + 1.0);
    assert_eq!(off, 3);
}

#[test]
fn crlf_offsets_stay_doc_coordinates() {
    // 「ab\r\ncd」：第二行「c」文档偏移 4（a b \r \n）
    let (eng, l, ..) = layout_of("ab\r\ncd", 200.0);
    let (_, y, _) = eng.caret_xy(&l, 4);
    assert_eq!(eng.hit(&l, 0.0, y + 1.0), 4);
    // 行尾（\r 处，偏移 2）与第一行末字形同 x
    let (x_cr, _, _) = eng.caret_xy(&l, 2);
    let (x_b, _, _) = eng.caret_xy(&l, 1);
    assert!(x_cr > x_b);
}

#[test]
fn empty_text_still_has_line_height() {
    // 空串（preedit 清空/空行排版路径）也须给出正高度，否则光标矩形高度为 0
    let mut eng = LayoutEngine::new();
    let l = eng.layout_preedit("");
    assert!(l.height >= BASE_LINE_HEIGHT);
}

#[test]
fn rasterize_nonempty_pixels_for_text() {
    let (mut eng, l) = layout_of("hello", 200.0);
    let (px, w, h) = eng.rasterize(&l, [0, 0, 0]);
    assert!(w > 0 && h > 0);
    assert!(px.chunks_exact(4).any(|p| p[3] > 0), "文本应产生非透明像素");
}

#[test]
fn heading_layout_taller_than_paragraph() {
    let (_, h1) = layout_of("# t", 400.0);
    let (_, p) = layout_of("t", 400.0);
    assert!(h1.height > p.height);
}

// ---- Reveal（切片 2）：光标不在节点内时消隐标记 ----
// 偏移均逐字节手工核算：「a **b** c」a=0 ' '=1 *=2 *=3 b=4 *=5 *=6 ' '=7 c=8

#[test]
fn reveal_hides_strong_markers_when_caret_outside() {
    let (_, l) = layout_with_caret("a **b** c", 0, 200.0);
    assert_eq!(l.layout_text(), "a b c");
}

#[test]
fn reveal_shows_full_node_when_caret_inside() {
    let (_, l) = layout_with_caret("a **b** c", 4, 200.0);
    assert_eq!(l.layout_text(), "a **b** c");
}

#[test]
fn nested_emphasis_reveals_only_innermost_markers() {
    // D12-b 真机形态：`**Chi*n*ese**` 光标在 `n` 上时，`**` 不得同时显形
    let (_, l) = layout_with_caret("**Chi*n*ese**", 6, 400.0);
    assert_eq!(l.layout_text(), "Chi*n*ese");
    // 光标在粗体正文（内层之外）→ 只显外层标记
    let (_, l) = layout_with_caret("**Chi*n*ese**", 3, 400.0);
    assert_eq!(l.layout_text(), "**Chinese**");
    // 光标在块外 → 全部消隐
    let (_, l) = layout_with_caret("**Chi*n*ese**", usize::MAX, 400.0);
    assert_eq!(l.layout_text(), "Chinese");
}

#[test]
fn reveal_boundary_offsets_count_as_inside() {
    // 激活规则为闭区间：caret 贴 range.start(2)/range.end(7) 均激活
    for caret in [2, 7] {
        let (_, l) = layout_with_caret("a **b** c", caret, 200.0);
        assert_eq!(l.layout_text(), "a **b** c", "caret={caret} 应激活");
    }
    let (_, l) = layout_with_caret("a **b** c", 8, 200.0);
    assert_eq!(l.layout_text(), "a b c");
}

#[test]
fn reveal_link_hides_url_and_brackets() {
    // 「x [t](http://a) y」：Link range=[2,14) content=[3,4) → 消隐 [ 与 ](http://a)
    let (_, l) = layout_with_caret("x [t](http://a) y", 0, 300.0);
    assert_eq!(l.layout_text(), "x t y");
    let (_, l) = layout_with_caret("x [t](http://a) y", 3, 300.0);
    assert_eq!(l.layout_text(), "x [t](http://a) y");
}

#[test]
fn link_span_attrs_follow_theme_link_color() {
    let src = "see [官网](https://example.com) now";
    let s = Session::open(src);
    let (id, kind, range) = s.blocks()[0].clone();
    let inlines = s.block_inlines(id).unwrap();
    let spans = style_spans(range.len(), range, &inlines);
    let style_at = |at: usize| {
        spans
            .iter()
            .find(|(s, e, _)| *s <= at && at < *e)
            .map(|(_, _, st)| *st)
            .expect("style_spans 必须致密覆盖")
    };
    let link_at = src.find('官').unwrap();
    let body_at = 0; // 「see」中的 s
    assert!(style_at(link_at).link && !style_at(body_at).link);
    for theme in [Theme::light(), Theme::dark()] {
        let want = Color::rgb(theme.link[0], theme.link[1], theme.link[2]);
        let link_attrs = attrs_of(
            kind.clone(),
            style_at(link_at),
            theme.link,
            DocFontFamily::System,
            None,
            12.0,
        );
        assert_eq!(
            link_attrs.color_opt,
            Some(want),
            "链接字符应使用主题链接色: dark={}",
            theme.dark
        );
        let body_attrs = attrs_of(
            kind.clone(),
            style_at(body_at),
            theme.link,
            DocFontFamily::System,
            None,
            12.0,
        );
        assert_eq!(body_attrs.color_opt, None, "正文不得使用链接色");
        assert_ne!(theme.link, theme.fg, "链接色与正文前景色必须可区分");
    }
    // 活动链接显示源码，样式标记不变（仍由 link 色上色）
    let (_, shown) = layout_with_caret(src, link_at, 400.0);
    assert!(shown.layout_text().contains("https://example.com"));
}

#[test]
fn engine_link_color_defaults_light_and_follows_setter() {
    let mut eng = LayoutEngine::new();
    assert_eq!(eng.link_color, Theme::light().link);
    eng.set_link_color(Theme::dark().link);
    assert_eq!(eng.link_color, Theme::dark().link);
}

#[test]
fn doc_typography_scales_body_but_leaves_ui_untouched() {
    let text = "正文一行";
    let layout = |engine: &mut LayoutEngine| {
        engine
            .layout_block(
                BlockKind::Paragraph,
                text,
                Range::new(0, text.len()),
                &[],
                400.0,
                usize::MAX,
                &[],
                false,
            )
            .height
    };
    let mut def = LayoutEngine::new();
    let def_h = layout(&mut def);
    let ui_def = def.measure("菜单");
    let mut big = LayoutEngine::new();
    big.set_doc_typography(18.0, 27.0, DocFontFamily::Serif);
    let big_h = layout(&mut big);
    let ui_big = big.measure("菜单");
    assert!(
        big_h > def_h,
        "18px 正文块应高于默认 16px：{big_h} vs {def_h}"
    );
    assert!(big_h >= 27.0 - 0.5, "块高应使用配置行高：{big_h}");
    assert_eq!(ui_big, ui_def, "UI 文本度量不受正文排版配置影响");
}

#[test]
fn reveal_nested_nodes_reveal_independently() {
    // 「**a *b* c**」：caret=3 在外层内容、内层（[4,7)）外 → 仅外层显标记
    let (_, l) = layout_with_caret("**a *b* c**", 3, 300.0);
    assert_eq!(l.layout_text(), "**a b c**");
    // caret=5 进内层：深层优先，只显内层 `*b*`，外层 `**` 保持消隐（D12-b）
    let (_, l) = layout_with_caret("**a *b* c**", 5, 300.0);
    assert_eq!(l.layout_text(), "a *b* c");
}

#[test]
fn reveal_composes_with_crlf() {
    // 「ab\r\n**c**」：\r=2 消隐；Strong range=[4,9) content=[6,7) → 标记消隐
    let (_, l) = layout_with_caret("ab\r\n**c**", 0, 200.0);
    assert_eq!(l.layout_text(), "ab\nc");
}

#[test]
fn reveal_hit_roundtrips_visible_offsets() {
    // 消隐态下可见文档偏移的 caret_xy → hit 回环必须保持文档坐标
    let (eng, l) = layout_with_caret("a **b** c", 0, 200.0);
    for doc in [0, 1, 4, 7, 8, 9] {
        let (x, y, _) = eng.caret_xy(&l, doc);
        assert_eq!(
            eng.hit(&l, x + 0.1, y + 1.0),
            doc,
            "可见偏移回环: doc={doc}"
        );
    }
}

// ---- 块级替换（切片 12）：列表符号 / 引用标记 / 分隔线 ----

#[test]
fn list_bullet_hidden_outside_caret_renumbers_ordered() {
    // 无序标记 [0,1) 非激活 → `•`（与 strong 消隐叠加：「- **a**」→「• a」）
    let (_, l) = layout_with_caret("- **a**", usize::MAX, 200.0);
    assert_eq!(l.layout_text(), "• a");
    // 激活（闭区间含块首）→ 列表标记显源码；strong 未激活仍消隐
    let (_, l) = layout_with_caret("- **a**", 0, 200.0);
    assert_eq!(l.layout_text(), "- a");
    // 光标进 strong 内容 → 列表与 inline 标记均显源码
    let (_, l) = layout_with_caret("- **a**", 4, 200.0);
    assert_eq!(l.layout_text(), "- **a**");
    // 单块调用拿不到文档顺序（不传 ordinal）→ 有序列表保留源码编号
    let (_, l) = layout_with_caret("1. a", usize::MAX, 200.0);
    assert_eq!(l.layout_text(), "1. a");
}

#[test]
fn legacy_toc_list_hides_list_markers_but_keeps_indent() {
    // 旧目录列表：条目文本自带层级编号，标记换成等宽空格（不叠加 1./2.）。
    let src = "1. [第一章 介绍](#第一章 - 介绍)\n   1. [1.1 背景](#11- 背景)";
    let s = Session::open(src);
    let blocks = s.blocks();
    let mut eng = LayoutEngine::new();
    let render = |eng: &mut LayoutEngine, caret: usize| {
        blocks
            .iter()
            .map(|(id, kind, range)| {
                let inlines = s.block_inlines(*id).unwrap_or_default();
                eng.layout_block_with_footnotes(
                    kind.clone(),
                    &src[range.start..range.end],
                    *range,
                    &inlines,
                    400.0,
                    caret,
                    &[],
                    false,
                    None,
                    &[],
                    Some(1),
                    true,
                )
                .layout_text()
                .to_string()
            })
            .collect::<Vec<_>>()
    };
    // CJK 前会自动补一个中西文间距（既有排版规则），故首项为 4 空格。
    assert_eq!(
        render(&mut eng, usize::MAX),
        ["    第一章 介绍", "      1.1 背景"]
    );
    // 光标进入块 → 显示源码列表标记（与其它列表项一致），离开由调用方重排恢复。
    let first_start = blocks[0].2.start;
    let active = render(&mut eng, first_start);
    assert!(
        active[0].starts_with("1. "),
        "光标进入应显源码标记: {active:?}"
    );
    assert!(active[0].contains("第一章 介绍"));
    assert!(
        !active[0].contains("](#"),
        "链接标记仍按行内规则消隐: {active:?}"
    );
    assert_eq!(active[1], "      1.1 背景");
    // 无序旧目录同样只保留缩进而无 `•`。
    let src = "- [A](#a)\n- [B](#b)";
    let s = Session::open(src);
    let (id, kind, range) = s.blocks()[0].clone();
    let inlines = s.block_inlines(id).unwrap_or_default();
    let l = eng.layout_block_with_footnotes(
        kind,
        &src[range.start..range.end],
        range,
        &inlines,
        300.0,
        usize::MAX,
        &[],
        false,
        None,
        &[],
        None,
        true,
    );
    assert_eq!(l.layout_text(), "  A");
}

#[test]
fn list_content_start_matches_marker_end() {
    let cases = [
        (
            BlockKind::ListItem {
                ordered: true,
                depth: 0,
                task: None,
            },
            "1. x",
            Some(2),
        ),
        (
            BlockKind::ListItem {
                ordered: true,
                depth: 1,
                task: None,
            },
            "   1. x",
            Some(5),
        ),
        (
            BlockKind::ListItem {
                ordered: false,
                depth: 0,
                task: None,
            },
            "- x",
            Some(1),
        ),
        (
            BlockKind::ListItem {
                ordered: false,
                depth: 0,
                task: Some(false),
            },
            "- [ ] x",
            None,
        ),
        (BlockKind::Paragraph, "x", None),
    ];
    for (kind, body, want) in cases {
        assert_eq!(list_content_start(&kind, body), want, "{body:?}");
    }
}

#[test]
fn ordered_list_numbering_increments_across_blocks() {
    // 源码每项都写 `1.`（`[TOC]` 旧输出的典型形态）：非激活态按语义递增显示。
    let (_, ls) = layout_blocks_with_caret("1. a\n1. b\n1. c", usize::MAX, 300.0);
    assert_eq!(block_texts(&ls), "1. a\n2. b\n3. c");
}

#[test]
fn ordered_nested_list_restarts_and_parent_continues() {
    // 子列表从 1 起算；子列表结束后父层继续递增（父层源码写 `1.` 仍显示 2.）。
    let src = "1. a\n   1. x\n   1. y\n1. b";
    let (_, ls) = layout_blocks_with_caret(src, usize::MAX, 300.0);
    assert_eq!(block_texts(&ls), "1. a\n   1. x\n   2. y\n2. b");
}

#[test]
fn ordered_list_restarts_from_source_number_after_interruption() {
    // 新列表从源码首个编号起算（3. 开头 → 3./4.）。
    let (_, ls) = layout_blocks_with_caret("3. a\n1. b", usize::MAX, 300.0);
    assert_eq!(block_texts(&ls), "3. a\n4. b");
    // 非列表块打断 → 后续列表重新从源码编号起算。
    let (_, ls) = layout_blocks_with_caret("1. a\n\npara\n\n1. b", usize::MAX, 300.0);
    assert_eq!(block_texts(&ls), "1. a\npara\n1. b");
    // 无序项换序型 → 新列表，不得沿用上一列表的计数。
    let (_, ls) = layout_blocks_with_caret("1. a\n- b\n1. c", usize::MAX, 300.0);
    assert_eq!(block_texts(&ls), "1. a\n• b\n1. c");
}

#[test]
fn ordered_list_active_block_shows_source_number() {
    // caret=7 落在第二块内 → 该块显源码；其余块仍按语义编号。
    let (_, ls) = layout_blocks_with_caret("1. a\n1. b\n1. c", 7, 300.0);
    assert_eq!(block_texts(&ls), "1. a\n1. b\n3. c");
}

#[test]
fn ordered_number_width_growth_keeps_line_height() {
    // 9→10 编号变宽：单行块的行高不得因编号位数变化（必要时前导空格补齐）。
    let src = "1. a\n1. b\n1. c\n1. d\n1. e\n1. f\n1. g\n1. h\n1. i\n1. j";
    let (_, ls) = layout_blocks_with_caret(src, usize::MAX, 300.0);
    assert_eq!(block_texts(&ls).lines().last(), Some("10. j"));
    let h = ls[0].height;
    for l in &ls {
        assert!((l.height - h).abs() < 0.01, "编号宽度变化不得改变行高");
    }
}

#[test]
fn blockquote_marker_hidden_per_line() {
    // 「> a\n> b」：行 1 [0,4)、行 2 [4,7)
    let (_, l) = layout_with_caret("> a\n> b", usize::MAX, 200.0);
    assert_eq!(l.layout_text(), "  a\n  b", "光标在外：两行均隐 `>`");
    let (_, l) = layout_with_caret("> a\n> b", 6, 200.0);
    assert_eq!(l.layout_text(), "  a\n> b", "光标在行 2：仅该行显源码");
    let (_, l) = layout_with_caret("> a\n> b", 2, 200.0);
    assert_eq!(l.layout_text(), "> a\n  b", "光标在行 1：仅该行显源码");
    assert!(l.height >= 2.0 * BASE_LINE_HEIGHT, "两行引用块高不塌缩");
}

#[test]
fn blockquote_hidden_marker_hit_roundtrips() {
    // 隐标记为等长替换（`>` → 空格）：可见偏移回环仍为文档坐标
    let (eng, l) = layout_with_caret("> a", usize::MAX, 200.0);
    for doc in [0, 1, 2] {
        let (x, y, _) = eng.caret_xy(&l, doc);
        assert_eq!(eng.hit(&l, x + 0.1, y + 1.0), doc, "doc={doc}");
    }
}

// ---- B7：标题/围栏源码标记与列表/引用同一显隐规则（非激活消隐） ----

#[test]
fn admonition_marker_becomes_label_when_inactive() {
    // `> [!NOTE] body`：`>` 隐为空格，`[!NOTE]` 整体替换为可读标签
    let (_, l) = layout_with_caret("> [!NOTE] body", usize::MAX, 400.0);
    assert_eq!(l.layout_text(), "  Note body", "非激活：标记 → 标签");
    assert_eq!(
        l.layout_text().len(),
        "> [!NOTE] body".len() - 3,
        "标签替换为不等长 fill（7B → 4B），映射层按段处理"
    );
}

#[test]
fn admonition_variant_labels_and_alias() {
    for (src, label) in [
        ("> [!TIP] x", "Tip"),
        ("> [!IMPORTANT] x", "Important"),
        ("> [!WARNING] x", "Warning"),
        ("> [!WARN] x", "Warning"),
        ("> [!CAUTION] x", "Caution"),
    ] {
        let (_, l) = layout_with_caret(src, usize::MAX, 600.0);
        assert_eq!(l.layout_text(), format!("  {label} x"), "变体标签: {src}");
    }
}

#[test]
fn admonition_first_line_active_shows_source() {
    // 光标落在首行 → 还原源码（含 `[!TYPE]`），便于编辑标记
    let (_, l) = layout_with_caret("> [!NOTE] body", 5, 400.0);
    assert_eq!(l.layout_text(), "> [!NOTE] body");
}

#[test]
fn admonition_hides_each_line_marker_like_quote() {
    let src = "> [!TIP] a\n> b";
    let (_, l) = layout_with_caret(src, usize::MAX, 400.0);
    assert_eq!(
        l.layout_text(),
        "  Tip a\n  b",
        "两行 `>` 均隐，仅首行标签替换"
    );
    // 光标在第二行 → 仅第二行显源码，首行仍替换标签
    let (_, l) = layout_with_caret(src, src.len() - 1, 400.0);
    assert_eq!(l.layout_text(), "  Tip a\n> b");
    assert!(l.height >= 2.0 * BASE_LINE_HEIGHT, "两行提示块高不塌缩");
}

#[test]
fn admonition_height_stable_across_reveal() {
    let hidden = layout_with_caret("> [!WARNING] careful", usize::MAX, 400.0).1;
    let shown = layout_with_caret("> [!WARNING] careful", 5, 400.0).1;
    assert!(
        (hidden.height - shown.height).abs() < 0.01,
        "显隐切换不得改变行高: {} vs {}",
        hidden.height,
        shown.height
    );
}

#[test]
fn admonition_marker_only_matched_on_first_line() {
    // 第二行出现 `[!NOTE]` 不参与替换（仅首行是标记）
    let src = "> a\n> [!NOTE] b";
    let (_, l) = layout_with_caret(src, usize::MAX, 400.0);
    assert_eq!(l.layout_text(), "  a\n  [!NOTE] b");
}

pub(super) fn layout_with_callouts(
    src: &str,
    caret: usize,
    width: f32,
    registry: &veil_core::CalloutRegistry,
) -> BlockLayout {
    let s = Session::open_with_callouts(src, registry);
    let (id, kind, range) = s.blocks()[0].clone();
    let inlines = s.block_inlines(id).unwrap();
    let mut eng = LayoutEngine::new();
    let block_text = &src[range.start..range.end];
    eng.layout_block(kind, block_text, range, &inlines, width, caret, &[], false)
}

#[test]
fn custom_callout_uses_configured_label() {
    let reg = veil_core::CalloutRegistry::from_json(
        r##"[{"name":"FIXME","label":"待修复","color":"#c00","icon":"!"}]"##,
    );
    let l = layout_with_callouts("> [!FIXME] body", usize::MAX, 400.0, &reg);
    assert_eq!(l.layout_text(), "  待修复 body", "首行标记 → 配置标签");
    // 光标进入首行 → 还原源码标记
    let l = layout_with_callouts("> [!FIXME] body", 5, 400.0, &reg);
    assert_eq!(l.layout_text(), "> [!FIXME] body");
}

#[test]
fn unknown_callout_marker_hidden_in_plain_quote() {
    // 未配置类型回退普通引用块：`[!XXX]` 不得显示成裸文本
    let (_, l) = layout_with_caret("> [!BOGUS] body", usize::MAX, 400.0);
    assert_eq!(l.layout_text(), "   body");
    assert!(!l.layout_text().contains("[!"), "不得露出裸标记");
}

#[test]
fn highlight_markers_hidden_and_mark_bg_applied() {
    let (_, l) = layout_with_caret("a ==hi== b", usize::MAX, 400.0);
    assert_eq!(l.layout_text(), "a hi b", "非激活：`==` 标记消隐");
    assert!(
        l.decorations_under
            .iter()
            .any(|(_, _, k)| *k == UnderKind::MarkBg),
        "高亮须与 `<mark>` 同一底色装饰"
    );
    // 光标进入节点 → 显示源码
    let (_, l) = layout_with_caret("a ==hi== b", 4, 400.0);
    assert_eq!(l.layout_text(), "a ==hi== b");
}

#[test]
fn atx_heading_marker_hidden_when_inactive_and_height_stable() {
    let (_, hidden) = layout_with_caret("## title", usize::MAX, 400.0);
    assert_eq!(hidden.layout_text(), "title", "非激活：`## ` 消隐");
    let (_, shown) = layout_with_caret("## title", 3, 400.0);
    assert_eq!(shown.layout_text(), "## title", "光标进入块：还原源码");
    assert!(
        (hidden.height - shown.height).abs() < 0.01,
        "显隐切换不得改变行高: {} vs {}",
        hidden.height,
        shown.height
    );
    // 缩进 ≤3 的标题同样消隐；`#` 后无空白不算标题标记（内核不会给出该 kind）
    let (_, l) = layout_with_caret("   ###  x", usize::MAX, 400.0);
    assert_eq!(l.layout_text(), "x");
}

#[test]
fn setext_heading_underline_hidden_when_inactive() {
    let (_, hidden) = layout_with_caret("Title\n===", usize::MAX, 400.0);
    assert_eq!(
        hidden.layout_text().trim_end(),
        "Title",
        "下划线行字符替换为空格（保留行高）"
    );
    let (_, shown) = layout_with_caret("Title\n===", 0, 400.0);
    assert_eq!(shown.layout_text(), "Title\n===");
    assert!(
        (hidden.height - shown.height).abs() < 0.01,
        "下划线消隐后空行仍占高"
    );
}

#[test]
fn fenced_code_fences_hidden_when_inactive_revealed_inside() {
    let src = "```rust\nlet x = 1;\n```";
    let (_, hidden) = layout_with_caret(src, usize::MAX, 400.0);
    assert_eq!(
        hidden.layout_text().trim_end(),
        "\nlet x = 1;",
        "围栏行不显示、内容保留（闭合行空格占位保行高）"
    );
    assert!(!hidden.layout_text().contains("```"));
    // 光标在代码内容内 → 整块激活，围栏还原
    let (_, shown) = layout_with_caret(src, 10, 400.0);
    assert_eq!(shown.layout_text(), src);
    assert!(
        (hidden.height - shown.height).abs() < 0.01,
        "围栏消隐不得塌缩行数: {} vs {}",
        hidden.height,
        shown.height
    );
    // 未闭合围栏：只隐开围栏行，末行内容不能被误删
    let (_, open) = layout_with_caret("```\na\nb", usize::MAX, 400.0);
    assert_eq!(open.layout_text(), "\na\nb");
}

#[test]
fn thematic_break_elides_whole_line_inactive() {
    let (_, l) = layout_with_caret("---", usize::MAX, 200.0);
    assert_eq!(l.layout_text(), "");
    assert_eq!(l.height, BASE_LINE_HEIGHT, "消隐后仍占一行高");
    let (_, l) = layout_with_caret("---", 1, 200.0);
    assert_eq!(l.layout_text(), "---");
}

// ---- OffsetMap：doc↔layout 坐标换算 ----

#[test]
fn offset_map_identity_roundtrip() {
    let (m, _) = OffsetMap::from_spans(5, &[]);
    for p in 0..=5 {
        assert_eq!(m.doc_to_layout(p), p);
        assert_eq!(m.layout_to_doc(p), p);
    }
}

#[test]
fn offset_map_gap_clamps_and_inverts() {
    // "abcdef" 消隐 [2,4)="cd" → 布局 "abef"；消隐区内 doc 钳到其布局起点
    let (m, _) = OffsetMap::from_spans(6, &[Sub::elide(2, 4)]);
    for (doc, want) in [(0, 0), (1, 1), (2, 2), (3, 2), (4, 2), (5, 3), (6, 4)] {
        assert_eq!(m.doc_to_layout(doc), want, "doc={doc}");
    }
    for (l, want) in [(0, 0), (1, 1), (2, 4), (3, 5), (4, 6)] {
        assert_eq!(m.layout_to_doc(l), want, "layout={l}");
    }
}

#[test]
fn offset_map_merges_adjacent_and_tail() {
    let (m, _) = OffsetMap::from_spans(6, &[Sub::elide(2, 3), Sub::elide(3, 5)]);
    assert_eq!(m.doc_to_layout(4), 2);
    assert_eq!(m.doc_to_layout(5), 2);
    assert_eq!(m.layout_to_doc(2), 5);
    // 尾部消隐：布局末尾反映射回文档末尾
    let (tail, _) = OffsetMap::from_spans(4, &[Sub::elide(2, 4)]);
    assert_eq!(tail.doc_to_layout(4), 2);
    assert_eq!(tail.layout_to_doc(2), 4);
}

// ---- OffsetMap 替换段（切片 8 行内公式占位 / 切片 12 块级替换） ----
// 期望均逐字节手工核算（注释内给出 seg 表推导）。

#[test]
fn offset_map_substitute_placeholder() {
    // 「aXb」替 [1,2) 为 3 字节：segs=(0,0,1,1) (1,1,3,1) (4,2,1,1)，layout_len=5
    let (m, _) = OffsetMap::from_spans(3, &[Sub::fill(1, 2, "xyz")]);
    assert_eq!(m.layout_len, 5);
    // 占位段内 doc 一律钳到左缘；节点尾（doc=2）落右缘（下一段起点）
    for (doc, want) in [(0, 0), (1, 1), (2, 4), (3, 5)] {
        assert_eq!(m.doc_to_layout(doc), want, "doc={doc}");
    }
    // 占位段内 layout 按中点规则落首/尾（点击图左/右半 → 激活显源码）
    for (l, want) in [(0, 0), (1, 1), (2, 1), (3, 2), (4, 2)] {
        assert_eq!(m.layout_to_doc(l), want, "layout={l}");
    }
}

#[test]
fn offset_map_elide_contained_in_placeholder_is_dropped() {
    // 多行行内公式：节点 [2,8) 占位 3 字节，其内 \r（[4,5)）消隐被占位段覆盖丢弃。
    // segs=(0,0,2,2) (2,2,3,6) (5,8,2,2)，layout_len=7
    let (m, _) = OffsetMap::from_spans(10, &[Sub::fill(2, 8, "xyz"), Sub::elide(4, 5)]);
    assert_eq!(m.layout_len, 7);
    assert_eq!(
        m.doc_to_layout(4),
        2,
        "占位段内钳到左缘（\\r 消隐不得切进占位段）"
    );
    assert_eq!(m.doc_to_layout(8), 5);
    assert_eq!(m.layout_to_doc(3), 2);
    assert_eq!(m.layout_to_doc(4), 8);
    assert_eq!(m.layout_to_doc(5), 8);
    assert_eq!(m.layout_to_doc(6), 9);
}

#[test]
fn offset_map_same_start_placeholder_wins_over_elide() {
    // 同起点时占位段（长）先于消隐段（短）：节点 [0,3) 占位 2 字节，[0,1) 消隐被丢弃。
    // segs=(0,0,2,3) (2,3,2,2)
    let (m, _) = OffsetMap::from_spans(5, &[Sub::elide(0, 1), Sub::fill(0, 3, "xy")]); // 乱序输入亦正确
    assert_eq!(m.layout_len, 4);
    assert_eq!(m.doc_to_layout(1), 0);
    assert_eq!(m.layout_to_doc(1), 3, "(1-0)*2=2 不 <2 → 落节点尾");
    assert_eq!(m.layout_to_doc(2), 3);
}

#[test]
fn offset_map_equal_length_substitute_still_replaces() {
    // 切片 12 引用标记：1 字节 `>` → 1 字节空格（ll==dl 但仍是替换段）。
    // ">a"：segs=(0,0,1,1) (1,1,1,1)，layout_len=2；doc 0 钳左缘、doc 1 起为保留段
    let (m, acc) = OffsetMap::from_spans(2, &[Sub::fill(0, 1, " ")]);
    assert_eq!(m.layout_len, 2);
    assert_eq!(acc[0].fill, " ");
    assert_eq!(m.doc_to_layout(0), 0);
    assert_eq!(m.doc_to_layout(1), 1);
    assert_eq!(m.layout_to_doc(0), 0, "点击替换段左半 → 段首");
    assert_eq!(m.layout_to_doc(1), 1);
}

// ---- 行内公式占位（切片 8）：非激活 Math 节点 → n 个 NBSP 占位 + PlacedImg ----
// 「a $x$ b」逐字节：a=0 ' '=1 $=2 x=3 $=4 ' '=5 b=6，Math range=[2,5) content=[3,4)

pub(super) fn img(range: Range, width: u32, height: u32, baseline: u32) -> InlineImg {
    InlineImg {
        range,
        key: InlineAsset::Math("x".into()),
        width,
        height,
        baseline,
        align: None,
    }
}

/// 图片占位（切片 13）：键为解析后的路径，底边贴基线（baseline = height）。
pub(super) fn image(range: Range, width: u32, height: u32) -> InlineImg {
    InlineImg {
        range,
        key: InlineAsset::Image("p.png".into()),
        width,
        height,
        baseline: height,
        align: None,
    }
}

#[test]
fn html_alias_styles_match_strong_and_em() {
    let src = "<b>B</b> <strong>S</strong> <i>I</i> <em>E</em>";
    let s = Session::open(src);
    let (id, _kind, range) = s.blocks()[0].clone();
    let inlines = s.block_inlines(id).unwrap();
    let spans = style_spans(src.len(), range, &inlines);
    let style_at = |off: usize| {
        spans
            .iter()
            .find(|(s, e, _)| *s <= off && off < *e)
            .map(|(_, _, st)| *st)
            .expect("样式段应致密覆盖")
    };
    assert_eq!(
        style_at(4).bold,
        style_at(15).bold,
        "<b> 与 <strong> 同为粗体"
    );
    assert_eq!(
        style_at(30).italic,
        style_at(41).italic,
        "<i> 与 <em> 同为斜体"
    );
    // 活动/非活动都走同一 style_spans 入口；光栅层权重也一致。
    for (alias, canonical) in [
        ("<b>B</b>", "<strong>B</strong>"),
        ("<i>I</i>", "<em>I</em>"),
    ] {
        let (mut e1, l1) = layout_of(alias, 400.0);
        let (p1, _, _) = e1.rasterize(&l1, [0, 0, 0]);
        let (mut e2, l2) = layout_of(canonical, 400.0);
        let (p2, _, _) = e2.rasterize(&l2, [0, 0, 0]);
        let ink = |p: &[u8]| p.chunks(4).filter(|x| x[3] > 0).count();
        assert_eq!(ink(&p1), ink(&p2), "{alias} 与 {canonical} 光栅墨量一致");
    }
}

#[test]
fn cjk_inline_math_advance_covers_image() {
    let src = "中文 $x$ 后文";
    let imgs = [img(Range::new(7, 10), 40, 10, 8)];
    let (eng, l) = layout_with_imgs(src, usize::MAX, 400.0, &imgs);
    let p = &l.imgs[0];
    let (x_end, _, _) = eng.caret_xy(&l, 10); // 数学节点右缘（后续空格之前）
    assert!(
        x_end + 0.01 >= p.x + p.width as f32,
        "占位 advance 必须覆盖公式图：end={x_end} img_right={}",
        p.x + p.width as f32
    );
    assert!(
        x_end <= p.x + p.width as f32 + 8.0,
        "占位不应明显超过图宽（避免重复占位）：end={x_end} img_right={}",
        p.x + p.width as f32
    );
}

#[test]
fn inline_math_placeholder_width_matches_image() {
    let imgs = [img(Range::new(2, 5), 40, 10, 8)];
    let (eng, l) = layout_with_imgs("a $x$ b", usize::MAX, 400.0, &imgs);
    assert!(!l.layout_text().contains("$x$"), "非激活节点源码不入排版");
    // 图宽 40px：节点尾光标 x - 节点首光标 x ≈ 40（误差 ≤ 2 个 NBSP 宽，字体度量从宽）
    let (x0, _, _) = eng.caret_xy(&l, 2);
    let (x1, _, _) = eng.caret_xy(&l, 5);
    let w = x1 - x0;
    assert!((w - 40.0).abs() <= 9.0, "占位宽应≈图宽: w={w}");
}

#[test]
fn inline_math_placed_on_text_baseline() {
    let imgs = [img(Range::new(2, 5), 16, 8, 6)];
    let (eng, l) = layout_with_imgs("a $x$ b", usize::MAX, 400.0, &imgs);
    assert_eq!(l.imgs.len(), 1);
    let p = &l.imgs[0];
    assert_eq!(p.width, 16);
    let (x, top, h) = eng.caret_xy(&l, 2);
    assert_eq!(p.x, x, "图左缘 = 节点首光标 x");
    let base = p.y + 6.0; // 图顶 + baseline = 行基线
    assert!(
        base > top && base <= top + h,
        "基线落在本行内: base={base} top={top} h={h}"
    );
}

#[test]
fn tall_inline_image_extends_block_height() {
    let imgs = [img(Range::new(2, 5), 16, 100, 10)];
    let (_, l) = layout_with_imgs("a $x$ b", usize::MAX, 400.0, &imgs);
    let p = &l.imgs[0];
    assert!(
        l.height >= p.y + 100.0,
        "块高须覆盖图底: h={} y={}",
        l.height,
        p.y
    );
    assert!(l.height > BASE_LINE_HEIGHT, "图高超行高，块高必须扩展");
}

#[test]
fn hit_on_image_maps_to_node_edges() {
    let imgs = [img(Range::new(2, 5), 40, 10, 8)];
    let (eng, l) = layout_with_imgs("a $x$ b", usize::MAX, 400.0, &imgs);
    let p = &l.imgs[0];
    let mid_y = p.y + 5.0;
    assert_eq!(eng.hit(&l, p.x + 1.0, mid_y), 2, "图左半 → 节点首");
    assert_eq!(eng.hit(&l, p.x + 39.0, mid_y), 5, "图右半 → 节点尾");
}

#[test]
fn caret_at_node_edge_shows_source_not_image() {
    // 激活规则闭区间：caret 贴节点首/尾即显源码（占位被忽略）
    for caret in [2, 5] {
        let imgs = [img(Range::new(2, 5), 40, 10, 8)];
        let (_, l) = layout_with_imgs("a $x$ b", caret, 400.0, &imgs);
        assert_eq!(l.layout_text(), "a $x$ b", "caret={caret} 应显源码");
        assert!(l.imgs.is_empty());
    }
    let imgs = [img(Range::new(2, 5), 40, 10, 8)];
    let (_, l) = layout_with_imgs("a $x$ b", 0, 400.0, &imgs);
    assert_eq!(l.imgs.len(), 1, "光标在外出图");
}

#[test]
fn fit_size_box_caps_both_axes_without_enlarging() {
    assert_eq!(fit_size_box(10, 10, 100.0, 100.0), (10, 10), "不得放大");
    assert_eq!(fit_size_box(100, 100, 50.0, 200.0), (50, 50), "宽度上限");
    assert_eq!(fit_size_box(100, 100, 200.0, 44.0), (44, 44), "高度上限");
    // B4 场景：240×160 行内图，2 行限高 44 → 66×44
    assert_eq!(fit_size_box(240, 160, 756.0, 44.0), (66, 44));
    // A：独占一行图片改用视口 80% 限高（600 → 480），宽度不再卡死在小值
    assert_eq!(fit_size_box(800, 600, 756.0, 480.0), (640, 480));
    // A：行内限高从 2 行提到 4 行（行高 22 → 88）
    assert_eq!(fit_size_box(240, 160, 756.0, 88.0), (132, 88));
}

#[test]
fn explicit_size_prefers_author_dimensions() {
    // 显式宽高：原样采用（可放大），不受默认限高约束
    assert_eq!(
        explicit_size(100, 100, Some(300), Some(200), 3024.0, 3024.0),
        (300, 200),
        "宽高都给 → 直接用给定值"
    );
    // 只给宽：高按原图比例（100×50 → 宽 300 则高 150）
    assert_eq!(
        explicit_size(100, 50, Some(300), None, 3024.0, 3024.0),
        (300, 150),
        "只给宽 → 高按比例"
    );
    // 只给高：宽按原图比例（100×50 → 高 200 则宽 400）
    assert_eq!(
        explicit_size(100, 50, None, Some(200), 3024.0, 3024.0),
        (400, 200),
        "只给高 → 宽按比例"
    );
    // 超宽：等比夹到上限（上限 = 版心宽 ×4 = 1600）
    assert_eq!(
        explicit_size(100, 100, Some(5000), Some(1000), 1600.0, 1600.0),
        (1600, 320),
        "超宽 → 等比缩到宽度上限"
    );
    // 超高：等比夹到上限
    assert_eq!(
        explicit_size(100, 100, Some(500), Some(5000), 1600.0, 1600.0),
        (160, 1600),
        "超高 → 等比缩到高度上限"
    );
    // 小图：显式尺寸小于上限时保持原样（不做「不放大」限制）
    assert_eq!(
        explicit_size(1000, 1000, Some(40), Some(40), 1600.0, 1600.0),
        (40, 40),
        "显式小图保持显式尺寸"
    );
}

#[test]
fn image_display_size_uses_explicit_only_when_present() {
    // 无显式尺寸：走默认限宽限高（仅缩小不放大）
    assert_eq!(
        image_display_size(240, 160, (None, None), 756.0, 88.0),
        (132, 88)
    );
    // 有显式尺寸：上限放宽到版心宽 ×4，允许放大
    assert_eq!(
        image_display_size(24, 24, (Some(96), Some(96)), 756.0, 88.0),
        (96, 96)
    );
    // 显式尺寸超极大值：夹到 756 × 4 = 3024
    assert_eq!(
        image_display_size(10, 10, (Some(9999), Some(9999)), 756.0, 88.0),
        (3024, 3024)
    );
}

#[test]
fn tall_image_expands_own_line_and_shifts_following_lines() {
    // B4：100px 高图（> 22px 行高）→ 所在视觉行行高抬到 100，后续行下移，
    // 块高覆盖图片底边，图片下半区域仍可命中节点（不重叠下一块）。
    let imgs = [image(Range::new(0, 13), 100, 100)];
    let (eng, l) = layout_with_imgs(
        "![alt](p.png) bbbbbbbbbbbbbbbbbbbb",
        usize::MAX,
        140.0,
        &imgs,
    );
    let p = &l.imgs[0];
    assert!(p.y.abs() < 0.5, "图应贴视觉行顶: {}", p.y);
    assert!(
        l.height >= p.y + p.height as f32 - 0.5,
        "块高须覆盖图片: {} vs {}",
        l.height,
        p.y + p.height as f32
    );
    let runs: Vec<_> = l.buffer.layout_runs().collect();
    assert!(runs.len() >= 2, "窄宽应折行");
    assert!(
        runs[0].line_height >= 100.0,
        "图所在视觉行行高须容纳图: {}",
        runs[0].line_height
    );
    assert!(
        runs[1].line_top >= runs[0].line_top + 100.0 - 0.5,
        "后续视觉行必须下移: {}",
        runs[1].line_top
    );
    assert_eq!(
        eng.hit(&l, p.x + p.width as f32 * 0.75, p.y + 90.0),
        13,
        "图下半区域（行高内）命中节点尾"
    );
}

#[test]
fn standalone_image_placeholder_is_single_line_without_duplication() {
    // 满宽独立图片：NBSP 占位只能排版一次且单行，块高 = 图片显示高度。
    // 回归：占位 span 被重复拼接时占位文本翻倍 → 折成两行，每行都按图高
    // 计高，独立图片块高度变成 2×图高（下一标题出现半屏空白）。
    let imgs = [image(Range::new(0, 13), 200, 100)];
    let (_, l) = layout_with_imgs("![alt](p.png)", usize::MAX, 200.0, &imgs);
    let runs: Vec<_> = l.buffer.layout_runs().collect();
    assert_eq!(runs.len(), 1, "满宽占位不得折行");
    assert_eq!(l.imgs.len(), 1);
    assert_eq!(runs[0].line_height, 100.0, "视觉行行高 = 图高");
    assert_eq!(l.height, 100.0, "块高只占图片实际显示高度");
    let nbsp = l.layout_text().chars().filter(|c| *c == '\u{a0}').count();
    let glyphs: usize = runs.iter().map(|r| r.glyphs.len()).sum();
    assert_eq!(
        glyphs, nbsp,
        "占位文本不得被重复排版（字形数应等于 NBSP 数）"
    );
}

#[test]
fn mermaid_placeholder_uses_mono_attrs_without_inflating_block_height() {
    let src = "```mermaid\nmindmap\n  root((Veil))\n```\n\n## 后续标题";
    let session = Session::open(src);
    let (id, kind, range) = session.blocks()[0].clone();
    assert!(matches!(kind, BlockKind::Mermaid));
    let mermaid_key = id.0.to_string();
    let inlines = session.block_inlines(id).unwrap();
    let mut eng = LayoutEngine::new();
    let scale = kind_scale(&kind) * block_scale(kind.clone()) * eng.scale;
    let metrics = Metrics::new(eng.doc_font_size * scale, eng.doc_line_height * scale);
    let mermaid_attrs = attrs_of(
        kind.clone(),
        Style::default(),
        eng.link_color,
        eng.doc_font,
        eng.mono_family,
        metrics.font_size,
    );
    let default_w = eng.measure_nbsp_with_attrs(metrics, &Attrs::new());
    let mono_w = eng.measure_nbsp_with_attrs(metrics, &mermaid_attrs);
    assert!(
        mono_w > default_w * 1.25,
        "测试需暴露默认字体与等宽 NBSP 的宽度差: default={default_w}, mono={mono_w}"
    );

    // 旧算法会得到 default_count 个 NBSP：默认字体下单行可容纳，等宽成形后必折行。
    let width = 400.0f32;
    let default_count = (width / mono_w).floor() as usize + 1;
    let image_w = ((default_count - 1) as f32 * default_w + default_w * 0.5).ceil() as u32;
    assert!(image_w > 0 && (image_w as f32) <= width);
    assert_eq!((image_w as f32 / default_w).ceil() as usize, default_count);
    assert!(default_count as f32 * mono_w > width);
    assert!(default_count <= (width / default_w).floor() as usize);

    let image_h = 120u32;
    let imgs = [InlineImg {
        range,
        key: InlineAsset::Mermaid(mermaid_key),
        width: image_w,
        height: image_h,
        baseline: image_h,
        align: None,
    }];
    let block_text = &src[range.start..range.end];
    let l = eng.layout_block(
        kind,
        block_text,
        range,
        &inlines,
        width,
        usize::MAX,
        &imgs,
        false,
    );
    let runs: Vec<_> = l.buffer.layout_runs().collect();
    assert_eq!(runs.len(), 1, "Mermaid NBSP 占位不得折成多个视觉行");
    assert_eq!(l.imgs.len(), 1);
    assert!(
        (l.height - image_h as f32).abs() <= 1.0,
        "块高只能占位图显示高: layout={}, image={image_h}",
        l.height
    );
    // 后续块的 y 由前一块 height 累加；标题起点不得再包含第二条图高视觉行。
    let following_heading_top = l.height;
    assert!(following_heading_top <= image_h as f32 + 1.0);
}

#[test]
fn narrow_width_wrap_keeps_single_placement() {
    // 「aaaaa $x$ bbbbb」：窄宽强制折行；占位段无论是否跨视觉行，只出一张图，
    // 图上命中恒落节点区间 [6,9)（逐字节：a×5=0..4 ' '=5 $=6 x=7 $=8 ' '=9 b×5=10..14）
    let imgs = [img(Range::new(6, 9), 30, 10, 8)];
    let (eng, l) = layout_with_imgs("aaaaa $x$ bbbbb", usize::MAX, 60.0, &imgs);
    assert!(l.height > BASE_LINE_HEIGHT, "窄宽应折行");
    assert_eq!(l.imgs.len(), 1);
    let p = &l.imgs[0];
    let off = eng.hit(&l, p.x + 15.0, p.y + 5.0);
    assert!((6..=9).contains(&off), "图上命中须落节点区间: {off}");
}

#[test]
fn math_inside_strong_keeps_placeholder() {
    // 「**a $x$**」逐字节：*=0 *=1 a=2 ' '=3 $=4 x=5 $=6 *=7 *=8
    // Strong range=[0,9) content=[2,7)；Math range=[4,7)：强标记消隐与公式占位并存
    let imgs = [img(Range::new(4, 7), 20, 10, 8)];
    let (_, l) = layout_with_imgs("**a $x$**", usize::MAX, 200.0, &imgs);
    assert_eq!(l.imgs.len(), 1);
    assert!(!l.layout_text().contains('$'), "公式源码不入排版");
    assert!(!l.layout_text().contains('*'), "强标记消隐不受占位影响");
}

// ---- 文档内图片占位（切片 13）：同公式占位机制，键为解析后的路径 ----
// 「a ![alt](p.png) b」逐字节：a=0 ' '=1 !=2 [=3 a=4 l=5 t=6 ]=7 (=8 p=9 . . n g=9..14 )=14
// Image range=[2,15) content=[4,7) url=[9,14)

#[test]
fn image_placeholder_replaces_whole_node() {
    let (eng, l) = layout_with_imgs(
        "a ![alt](p.png) b",
        usize::MAX,
        400.0,
        &[image(Range::new(2, 15), 40, 10)],
    );
    assert_eq!(l.imgs.len(), 1);
    assert_eq!(l.imgs[0].key, InlineAsset::Image("p.png".into()));
    assert!(!l.layout_text().contains("alt") && !l.layout_text().contains("p.png"));
    // 图占位左右半 → 节点首/尾（点击进入源码态）
    let p = &l.imgs[0];
    assert_eq!(eng.hit(&l, p.x + 1.0, p.y + 5.0), 2);
    assert_eq!(eng.hit(&l, p.x + 39.0, p.y + 5.0), 15);
}

#[test]
fn standalone_image_alignment_offsets_within_content_width() {
    // 独立图片按 align 在版心宽内水平偏移；占位本身不可见，移动位图不压文本。
    let text = r#"<img src="p.png" align="center">"#;
    let node = Range::new(0, text.len());
    let mut center = image(node, 40, 10);
    center.align = Some(veil_core::ImageAlign::Center);
    let (_, l) = layout_with_imgs(text, usize::MAX, 400.0, &[center]);
    assert_eq!(l.imgs.len(), 1);
    assert!(
        (l.imgs[0].x - 180.0).abs() < 1.0,
        "center x={}",
        l.imgs[0].x
    );
    let mut right = image(node, 40, 10);
    right.align = Some(veil_core::ImageAlign::Right);
    let (_, l) = layout_with_imgs(text, usize::MAX, 400.0, &[right]);
    assert!((l.imgs[0].x - 360.0).abs() < 1.0, "right x={}", l.imgs[0].x);
    // 无 align（左对齐/行内图片）：沿用占位原位置。
    let (_, l) = layout_with_imgs(text, usize::MAX, 400.0, &[image(node, 40, 10)]);
    assert!(l.imgs[0].x.abs() < 1.0, "left x={}", l.imgs[0].x);
}

#[test]
fn missing_image_falls_back_to_alt_text() {
    // 未提供图（加载失败/远程 URL）：标记消隐，只显 alt
    let (_, l) = layout_of("a ![alt](p.png) b", 400.0);
    assert_eq!(l.layout_text(), "a alt b");
}

// ---- 图片题注（Task 5）：独立成行图片下方显示 alt 文本 ----

#[test]
fn standalone_image_caption_adds_own_line_and_block_height() {
    // 「![图注](p.png)」逐字节：!=0 [=1 图=2..5 注=5..8 ]=8 (=9 p.png=10..14 )=14
    let text = "![图注](p.png)";
    let node = Range::new(0, 15);
    let imgs = [image(node, 200, 100)];
    let caps = [caption(node, "图注")];
    let (_, l) = layout_with_captions(text, usize::MAX, 400.0, &imgs, &caps);
    let runs: Vec<_> = l.buffer.layout_runs().collect();
    assert_eq!(runs.len(), 2, "图片一行 + 题注一行");
    assert!(l.layout_text().contains("图注"), "题注须进入视图排版");
    assert!(
        runs[1].line_top >= runs[0].line_top + 100.0 - 0.5,
        "题注必须位于图片下方"
    );
    assert!(
        l.height >= runs[1].line_top + runs[1].line_height - 0.5,
        "块高须覆盖题注行: h={}",
        l.height
    );
    assert!(
        l.height >= 100.0 + BASE_LINE_HEIGHT * 0.9,
        "块高须含题注行（图片 100 + 题注行）: {}",
        l.height
    );
}

#[test]
fn standalone_image_without_caption_keeps_single_line() {
    // 空 alt：shell 不传题注，块内仍只有图片一行。
    let node = Range::new(0, 11); // 「![](p.png)」
    let imgs = [image(node, 200, 100)];
    let (_, l) = layout_with_captions("![](p.png)", usize::MAX, 400.0, &imgs, &[]);
    assert_eq!(l.buffer.layout_runs().count(), 1);
    assert_eq!(l.height, 100.0, "无题注时块高 = 图片显示高度");
}

#[test]
fn long_cjk_caption_wraps_within_content_width() {
    let alt = "这是一段很长的中文题注用于验证窄窗口下的换行行为不应溢出也不应覆盖下一块";
    let text = format!("![{alt}](p.png)");
    let node = Range::new(0, text.len());
    let imgs = [image(node, 120, 60)];
    let caps = [caption(node, alt)];
    let width = 160.0;
    let (eng, l) = layout_with_captions(&text, usize::MAX, width, &imgs, &caps);
    let runs: Vec<_> = l.buffer.layout_runs().collect();
    assert!(runs.len() > 2, "长题注应折行: runs={}", runs.len());
    for run in &runs {
        assert!(
            run.line_w <= width + 1.0,
            "题注/占位行不得溢出: line_w={} width={width}",
            run.line_w
        );
        for g in run.glyphs.iter() {
            assert!(
                g.x + g.w <= width + 1.0,
                "字形右缘不得超出内容宽: {}",
                g.x + g.w
            );
        }
    }
    let last = runs.last().unwrap();
    assert!(
        l.height >= last.line_top + last.line_height - 0.5,
        "块高须覆盖末行题注，避免覆盖下一块"
    );
    // 命中题注行只落到图片节点边界，不会产生越界的文档偏移。
    let off = eng.hit(&l, 10.0, last.line_top + 2.0);
    assert!(off <= text.len(), "题注上的命中须钳在块内: {off}");
}

#[test]
fn missing_image_still_renders_caption_with_placeholder() {
    // 图片加载失败（无 InlineImg）：节点降级为通用占位，题注照常可读。
    let text = "![缺失图](p.png)";
    let node = Range::new(0, 18); // 「![缺失图](p.png)」= 18 字节
    let caps = [caption(node, "缺失图")];
    let (_, l) = layout_with_captions(text, usize::MAX, 400.0, &[], &caps);
    assert!(l.layout_text().contains(t(Key::ImageAltFallback)));
    assert!(l.layout_text().contains("缺失图"));
    assert_eq!(l.buffer.layout_runs().count(), 2, "占位一行 + 题注一行");
    assert!(
        l.layout_text().matches("缺失图").count() == 1,
        "alt 不重复显示"
    );
}

#[test]
fn caption_hidden_while_caret_edits_image_source() {
    let text = "![图注](p.png)";
    let node = Range::new(0, 15);
    let imgs = [image(node, 200, 100)];
    let caps = [caption(node, "图注")];
    let (_, l) = layout_with_captions(text, 5, 400.0, &imgs, &caps);
    assert!(
        !l.layout_text().contains('\n'),
        "光标进入图片时显源码、不追加题注行: {:?}",
        l.layout_text()
    );
    assert!(l.layout_text().contains("p.png"), "应显示图片源码");
}

#[test]
fn image_activated_by_caret_shows_source() {
    for caret in [2, 4, 15] {
        let (_, l) = layout_with_imgs(
            "a ![alt](p.png) b",
            caret,
            400.0,
            &[image(Range::new(2, 15), 40, 10)],
        );
        assert_eq!(l.layout_text(), "a ![alt](p.png) b", "caret={caret}");
        assert!(l.imgs.is_empty());
    }
}

#[test]
fn tall_image_top_clamped_to_line_top_and_extends_block() {
    let (_, l) = layout_with_imgs(
        "a ![alt](p.png) b",
        usize::MAX,
        400.0,
        &[image(Range::new(2, 15), 40, 100)],
    );
    let p = &l.imgs[0];
    assert!(p.y >= 0.0, "图片顶部不得上侵前一块: y={}", p.y);
    assert!(
        l.height >= p.y + 100.0,
        "块高须覆盖图底: h={} y={}",
        l.height,
        p.y
    );
    assert!(l.height > BASE_LINE_HEIGHT);
}
