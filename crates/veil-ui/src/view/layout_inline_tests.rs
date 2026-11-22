use super::linebreak::{LINE_END_FORBIDDEN, LINE_START_FORBIDDEN};
use super::tests::*;
use super::*;
use crate::theme::Theme;
use veil_core::Session;

// ---- 选区高亮（切片 3）：doc 区间 → 各视觉行 x 矩形（物理像素） ----

#[test]
fn selection_rects_single_line_matches_caret_span() {
    let (eng, l) = layout_of("hello", 200.0);
    let rects = eng.selection_rects(&l, 0, 5);
    assert_eq!(rects.len(), 1);
    let (x, y, w, h) = rects[0];
    let (x0, _, ch) = eng.caret_xy(&l, 0);
    let (x1, _, _) = eng.caret_xy(&l, 5);
    assert_eq!((x, y, h), (x0, 0.0, ch));
    assert_eq!(w, x1 - x0, "高亮宽度 = 两端光标 x 之差");
}

// ---- 代码块语法高亮（P3-1） ----

#[test]
fn fenced_code_with_lang_layouts_without_panic() {
    let src = "```rust\nlet x = 1;\nfn f() {}\n```";
    let (_eng, l) = layout_of(src, 400.0);
    assert!(l.height > BASE_LINE_HEIGHT * 2.0, "代码块应有多行高度");
}

#[test]
fn fenced_code_without_lang_uses_default_style() {
    let src = "```\nlet x = 1;\n```";
    let (_eng, l) = layout_of(src, 400.0);
    assert!(l.height > BASE_LINE_HEIGHT);
}

// ---- 删除线（P3-4） ----

#[test]
fn strikethrough_span_recorded_whole_node() {
    // 样式恒按整节点（含标记）记录；paint 层经 OffsetMap 映射到可视文本
    let (_eng, l) = layout_of("a ~~del~~ c", 400.0);
    assert_eq!(l.strikes, vec![(2, 9)]);
}

#[test]
fn strikethrough_markers_hidden_when_inactive() {
    // 非激活（无光标）时标记消隐：删除线矩形宽 ≈ 可视文本 "del" 宽
    let (eng, l) = layout_of("a ~~del~~ c", 400.0);
    let rects = eng.selection_rects(&l, 2, 9);
    assert_eq!(rects.len(), 1, "单行删除线");
    let (sx, _, sw, _) = rects[0];
    let (x_del, _, _) = eng.caret_xy(&l, 4);
    let (x_end, _, _) = eng.caret_xy(&l, 7);
    assert!((sx - x_del).abs() < 1.0, "矩形起点 = 可视文本起点");
    assert!((sw - (x_end - x_del)).abs() < 1.0, "矩形宽 = 可视文本宽");
}

#[test]
fn strikethrough_absent_without_syntax() {
    let (_eng, l) = layout_of("a ~ b", 400.0);
    assert!(l.strikes.is_empty());
}

// ---- 任务列表复选框（P3-5） ----

#[test]
fn task_item_checkbox_substitution() {
    // 非激活：`- [ ] foo` → `☐ foo`；勾选 → `☑`
    let (_eng, l) = layout_of("- [ ] foo", 400.0);
    assert_eq!(l.layout_text(), "☐ foo");
    let (_eng, l) = layout_of("- [x] bar", 400.0);
    assert_eq!(l.layout_text(), "☑ bar");
    // 有序列表任务同规则
    let (_eng, l) = layout_of("1. [x] done", 400.0);
    assert_eq!(l.layout_text(), "☑ done");
}

#[test]
fn task_item_active_shows_source() {
    // 光标进入块内：显源码 `- [ ] foo`（与列表符号/引用的激活行为一致）
    let (_eng, l) = layout_with_caret("- [ ] foo", 4, 400.0);
    assert_eq!(l.layout_text(), "- [ ] foo");
}

#[test]
fn plain_list_item_unaffected_by_task_rules() {
    let (_eng, l) = layout_of("- [ ]a", 400.0); // 后无空白：非任务 → `• [ ]a`
    assert_eq!(l.layout_text(), "• [ ]a");
}

#[test]
fn selection_rects_span_newline_per_line() {
    // 「ab\ncd」选 [1,4)：行1 高亮 "b"，行2 高亮 "c"，行间换行符不产生矩形
    let (eng, l) = layout_of("ab\ncd", 200.0);
    let rects = eng.selection_rects(&l, 1, 4);
    assert_eq!(rects.len(), 2);
    let (x_b, y1, _) = eng.caret_xy(&l, 1);
    assert_eq!(rects[0].0, x_b);
    assert_eq!(rects[0].1, y1);
    let (x_c, y2, h2) = eng.caret_xy(&l, 3);
    let (x_d, _, _) = eng.caret_xy(&l, 4);
    assert_eq!((rects[1].0, rects[1].1, rects[1].3), (x_c, y2, h2));
    assert_eq!(rects[1].2, x_d - x_c);
}

#[test]
fn selection_rects_collapsed_is_empty() {
    let (eng, l) = layout_of("hello", 200.0);
    assert!(eng.selection_rects(&l, 2, 2).is_empty());
}

#[test]
fn selection_rects_respect_reveal_elision() {
    // caret=0 时「**b**」标记消隐：选 doc [4,5)="b"，高亮须落在可见 "b" 的 x 上
    let (eng, l) = layout_with_caret("a **b** c", 0, 200.0);
    let rects = eng.selection_rects(&l, 4, 5);
    assert_eq!(rects.len(), 1);
    let (x0, _, _) = eng.caret_xy(&l, 4);
    let (x1, _, _) = eng.caret_xy(&l, 5);
    assert_eq!(rects[0].0, x0);
    assert_eq!(rects[0].2, x1 - x0);
}

// ---- 视觉行首尾（切片 4）：Home/End 落在当前视觉行两端 ----

#[test]
fn caret_xy_on_wrapped_line_uses_its_own_run() {
    // 回归：折行块内靠后偏移的光标曾恒落第一视觉行（line_bases 仅记硬换行，
    // 循环在首个 line_i 匹配的 run 即返回）。文档尾光标必须落在更靠下的视觉行。
    let (eng, l) = layout_of("aaaa aaaa aaaa", 30.0);
    let (_, y0, _) = eng.caret_xy(&l, 0);
    let (_, y1, _) = eng.caret_xy(&l, 14);
    assert!(y1 > y0, "折行后文档尾光标 y 应大于首行: y0={y0} y1={y1}");
}

#[test]
fn visual_line_edges_single_line() {
    let (eng, l) = layout_of("hello", 200.0);
    assert_eq!(eng.visual_line_edges(&l, 2), (0, 5));
    assert_eq!(eng.visual_line_edges(&l, 0), (0, 5));
    assert_eq!(eng.visual_line_edges(&l, 5), (0, 5));
}

#[test]
fn visual_line_edges_hard_break_lines() {
    // 「ab\ncd」逐字节：a=0 b=1 \n=2 c=3 d=4，文档长 5
    let (eng, l) = layout_of("ab\ncd", 200.0);
    assert_eq!(eng.visual_line_edges(&l, 1), (0, 2), "第一行");
    assert_eq!(eng.visual_line_edges(&l, 2), (0, 2), "第一行行尾（\\n 处）");
    assert_eq!(eng.visual_line_edges(&l, 3), (3, 5), "第二行行首");
    assert_eq!(eng.visual_line_edges(&l, 4), (3, 5), "第二行");
}

#[test]
fn visual_line_edges_wrap_stays_within_visual_line() {
    // 窄宽强制折行（切片 3 已证 ≥2 视觉行）：末偏移在末视觉行，行尾即文档尾
    let (eng, l) = layout_of("aaaa aaaa aaaa", 30.0);
    let (s, e) = eng.visual_line_edges(&l, 14);
    assert!(s > 0, "折行后末视觉行起点必非文档起点: s={s}");
    assert!(s <= 14);
    assert_eq!(e, 14, "末视觉行行尾 = 文档尾");
    // 与 hit 直接换算一致（shell 接线的参照行为）
    let (_, y, _) = eng.caret_xy(&l, 14);
    assert_eq!(eng.hit(&l, 0.0, y + 1.0), s);
    assert_eq!(eng.hit(&l, f32::MAX, y + 1.0), e);
}

#[test]
fn visual_line_edges_respect_elision() {
    // caret=0 时「**b**」标记消隐，layout_text="a b c"；全文长 9，单行
    let (eng, l) = layout_with_caret("a **b** c", 0, 200.0);
    assert_eq!(eng.visual_line_edges(&l, 8), (0, 9));
}

#[test]
fn selection_rects_wrap_gives_rect_per_visual_line() {
    // 窄宽度强制折行：全选时每条视觉行一个矩形，y 递增、宽高为正
    let (eng, l) = layout_of("aaaa aaaa aaaa", 30.0);
    let rects = eng.selection_rects(&l, 0, 14);
    assert!(rects.len() >= 2, "折行后应至少两行高亮: {:?}", rects);
    let mut prev_y = -1.0;
    for &(_, y, w, h) in &rects {
        assert!(w > 0.0 && h > 0.0);
        assert!(y > prev_y, "视觉行 y 严格递增");
        prev_y = y;
    }
}

// ---- 中西文自动间距（v0 切片） ----

#[test]
fn cjk_to_ascii_adds_space() {
    // 「中文text」→ 「中文 text」
    let (_, l) = layout_of("中文text", 400.0);
    assert_eq!(l.layout_text(), "中文 text");
}

#[test]
fn ascii_to_cjk_adds_space() {
    // 「text中文」→ 「text 中文」
    let (_, l) = layout_of("text中文", 400.0);
    assert_eq!(l.layout_text(), "text 中文");
}

#[test]
fn both_boundaries_add_spaces() {
    // 「中text文」→ 「中 text 文」
    let (_, l) = layout_of("中text文", 400.0);
    assert_eq!(l.layout_text(), "中 text 文");
}

#[test]
fn no_double_space_when_already_whitespace() {
    // 已有空白时不再加（边界跳过）
    let (_, l) = layout_of("中 text 文", 400.0);
    assert_eq!(l.layout_text(), "中 text 文");
}

#[test]
fn fenced_code_skips_cjk_spacing() {
    // 代码块里的 CJK↔ASCII 不插入空格（layout_text 含围栏消隐痕迹，但不含「中 text」）
    let (_, l) = layout_of("```\n中text\n```\n", 400.0);
    assert!(
        !l.layout_text().contains("中 text"),
        "代码块不应加 CJK 间距: {:?}",
        l.layout_text()
    );
}

#[test]
fn inline_code_hides_both_backticks_and_skips_cjk_spacing() {
    // 纯拉丁：两端反引号正常消隐
    let (_, l) = layout_of("A `plain` B.", 400.0);
    assert_eq!(l.layout_text(), "A plain B.");
    // 末尾为 CJK：闭反引号必须完全消隐（不得多画一个反引号）
    let (_, l) = layout_of("C `中文` D.", 400.0);
    assert_eq!(l.layout_text(), "C 中文 D.", "闭反引号须完全消隐");
    // 代码内 CJK↔ASCII 边界不得插空格
    let (_, l) = layout_of("E `中文abc` F.", 400.0);
    assert_eq!(
        l.layout_text(),
        "E 中文abc F.",
        "代码 span 内不得插 CJK 空格"
    );
    let (_, l) = layout_of("`纯中文`", 400.0);
    assert_eq!(l.layout_text(), "纯中文");
    // 混排整行（§7 真机反例）；WJ 为禁则胶水零宽字符，剥掉后再逐字断言
    let (_, l) = layout_of(
        "行内代码：`中文行内代码` 与 `let 变量 = 1;` 与 `纯 Latin mono`。",
        400.0,
    );
    assert_eq!(
        l.layout_text().replace('\u{2060}', ""),
        "行内代码：中文行内代码 与 let 变量 = 1; 与 纯 Latin mono。"
    );
}

#[test]
fn midline_frontmatter_setext_keeps_every_content_line() {
    // 中置 `---` 块走 Setext：内容可跨多行，除末行下划线外所有行都必须可见
    let text = "正文\n\n---\n标题: 中文标题\n作者: 张三\n描述: 这是一段中文描述\n---\n";
    let s = Session::open(text);
    let (id, kind, range) = s.blocks()[2].clone();
    assert_eq!(kind, BlockKind::SetextHeading(2));
    let inlines = s.block_inlines(id).unwrap();
    let mut eng = LayoutEngine::new();
    let l = eng.layout_block(
        kind,
        &text[range.start..range.end],
        range,
        &inlines,
        600.0,
        usize::MAX,
        &[],
        false,
    );
    let visible = l.layout_text();
    for want in ["中文标题", "张三", "这是一段中文描述"] {
        assert!(visible.contains(want), "可见文本丢行 {want:?}: {visible:?}");
    }
    assert_eq!(
        visible.lines().count(),
        4,
        "三行内容 + 下划线占位: {visible:?}"
    );
    assert!(!visible.contains("---"), "下划线行应消隐: {visible:?}");
}

#[test]
fn hit_roundtrip_works_with_cjk_spacing() {
    // 插入空格后 OffsetMap 仍正确：doc 偏移 3（中结束、t 开始）→ layout 应落到空格之后
    let (eng, l) = layout_of("中t", 400.0);
    // doc 文本：中=0..3, t=3..4；layout_text 应为 "中 t"
    let doc_t = 3; // 文档中 t 的位置
    let (x_before, _, _) = eng.caret_xy(&l, 0); // 中之前
    let (x_after, _, _) = eng.caret_xy(&l, doc_t); // t 之前（layout 里空格之后）
    assert!(x_after > x_before, "CJK 间距后光标 x 应递增");
    // hit 逆映射：layout 末尾 → 文档末尾
    let (x_last, y, h) = eng.caret_xy(&l, 4);
    assert_eq!(eng.hit(&l, x_last + 0.5, y + h / 2.0), 4);
}

#[test]
fn is_cjk_classifies_correctly() {
    assert!(is_cjk('中'));
    assert!(is_cjk('あ'));
    assert!(is_cjk('한'));
    assert!(!is_cjk('A'));
    assert!(!is_cjk('1'));
    assert!(!is_cjk('.'));
}

/// 每条视觉行的（首簇字符, 尾簇字符）：禁则测试的观测量。
fn visual_line_edges(l: &BlockLayout) -> Vec<(char, char)> {
    l.buffer
        .layout_runs()
        .filter_map(|r| {
            let f = r.glyphs.first()?;
            let g = r.glyphs.last()?;
            Some((
                r.text[f.start..f.end].chars().next()?,
                r.text[g.start..g.end].chars().next()?,
            ))
        })
        .collect()
}

/// 排版单行文本的实测行宽（禁则/悬挂测试的版心宽度来源）。
fn single_line_width(eng: &mut LayoutEngine, text: &str) -> f32 {
    let block = Range {
        start: 0,
        end: text.len(),
    };
    let l = eng.layout_block(
        BlockKind::Paragraph,
        text,
        block,
        &[],
        4000.0,
        usize::MAX,
        &[],
        false,
    );
    l.buffer.layout_runs().map(|r| r.line_w).fold(0.0, f32::max)
}

fn layout_paragraph(eng: &mut LayoutEngine, text: &str, width: f32) -> BlockLayout {
    let block = Range {
        start: 0,
        end: text.len(),
    };
    eng.layout_block(
        BlockKind::Paragraph,
        text,
        block,
        &[],
        width,
        usize::MAX,
        &[],
        false,
    )
}

#[test]
fn kinsoku_forbidden_chars_never_start_a_visual_line() {
    let mut eng = LayoutEngine::new();
    let prefix: String = "中".repeat(10);
    let w10 = single_line_width(&mut eng, &prefix);
    for &p in LINE_START_FORBIDDEN {
        let text = format!("{prefix}{p}{}{}", "中".repeat(4), "文".repeat(8));
        for delta in [-4.0, 0.0, 4.0] {
            // 版心恰好容纳 10 字：无禁则时第 11 位标点必顶到下一行行首
            let l = layout_paragraph(&mut eng, &text, w10 + 1.0 + delta);
            let edges = visual_line_edges(&l);
            assert!(edges.len() >= 2, "{p:?} 处应折行: {edges:?}");
            for (fc, _) in &edges {
                assert!(
                    !LINE_START_FORBIDDEN.contains(fc),
                    "行首禁则违反: {fc:?} 出现在行首（测试标点 {p:?}，宽 {delta:+.1}）"
                );
            }
            for (_, lc) in &edges {
                assert!(
                    !LINE_END_FORBIDDEN.contains(lc),
                    "行尾禁则违反: {lc:?} 出现在行尾（测试标点 {p:?}，宽 {delta:+.1}）"
                );
            }
        }
    }
}

#[test]
fn kinsoku_open_brackets_never_end_a_visual_line() {
    let mut eng = LayoutEngine::new();
    let w10 = single_line_width(&mut eng, &"中".repeat(10));
    // 括号在第 11 位（自然折行点）与第 10 位（无禁则时恰好滞留行尾）都要守住
    for prefix_len in [10usize, 9] {
        let prefix: String = "中".repeat(prefix_len);
        for &p in LINE_END_FORBIDDEN {
            let text = format!("{prefix}{p}{}{}", "中".repeat(4), "文".repeat(8));
            let l = layout_paragraph(&mut eng, &text, w10 + 1.0);
            let edges = visual_line_edges(&l);
            assert!(edges.len() >= 2, "{p:?} 处应折行: {edges:?}");
            for (_, lc) in &edges {
                assert!(
                    !LINE_END_FORBIDDEN.contains(lc),
                    "行尾禁则违反: {lc:?} 出现在行尾（括号 {p:?}，prefix={prefix_len}）"
                );
            }
            for (fc, _) in &edges {
                assert!(
                    !LINE_START_FORBIDDEN.contains(fc),
                    "行首禁则违反: {fc:?} 出现在行首（括号 {p:?}，prefix={prefix_len}）"
                );
            }
        }
    }
}

#[test]
fn kinsoku_glue_keeps_ellipsis_attached_to_previous_char() {
    // 版心满到省略号前一字：…… 必须整体随前字下沉，不得出现在行首，
    // 也不得内部断开（WJ × WJ 链）
    let mut eng = LayoutEngine::new();
    let w10 = single_line_width(&mut eng, &"中".repeat(10));
    let text = format!("{}……{}", "中".repeat(10), "文".repeat(6));
    let l = layout_paragraph(&mut eng, &text, w10 + 1.0);
    let edges = visual_line_edges(&l);
    assert!(edges.len() >= 2, "应折行: {edges:?}");
    assert!(
        edges.iter().all(|(fc, _)| *fc != '…'),
        "…… 不得在行首（内部断开必致后半在行首）: {edges:?}"
    );
}

#[test]
fn hanging_punctuation_shifts_full_line_end_glyph() {
    let mut eng = LayoutEngine::new();
    let w10 = single_line_width(&mut eng, &"中".repeat(10));
    let width = w10 + 1.0;
    // 9 字 + 句号恰满一行：末字应悬挂
    let full = format!("{}。{}", "中".repeat(9), "文".repeat(6));
    let l = layout_paragraph(&mut eng, &full, width);
    let runs: Vec<_> = l.buffer.layout_runs().collect();
    assert!(runs.len() >= 2, "满行句号文本应折行");
    let (dx, hi) = hang_shift(&runs[0], Some(width));
    assert!(dx > 0.0 && hi.is_some(), "满行末字句号应悬挂: dx={dx}");
    // 非满行（行尾同句号但行宽差很多）不悬挂
    let short = format!("{}。{}", "中".repeat(5), "文".repeat(2));
    let l2 = layout_paragraph(&mut eng, &short, width);
    let run2 = l2.buffer.layout_runs().next().unwrap();
    assert_eq!(hang_shift(&run2, Some(width)), (0.0, None), "非满行不悬挂");
    // 末字非闭标点不悬挂
    let plain = "中".repeat(12);
    let l3 = layout_paragraph(&mut eng, &plain, width);
    let run3 = l3.buffer.layout_runs().next().unwrap();
    assert_eq!(
        hang_shift(&run3, Some(width)),
        (0.0, None),
        "末字非闭标点不悬挂"
    );
}

#[test]
fn hanging_punctuation_widens_raster_beyond_text_edge() {
    let mut eng = LayoutEngine::new();
    let w10 = single_line_width(&mut eng, &"中".repeat(10));
    let hung = format!("{}。{}", "中".repeat(9), "文".repeat(6));
    let l = layout_paragraph(&mut eng, &hung, w10 + 1.0);
    let (_px, w_hung, _h) = eng.rasterize(&l, [0, 0, 0]);
    assert!(
        w_hung as f32 > w10,
        "悬挂后位图应宽于 10 字版心: w_hung={w_hung} w10={w10}"
    );
}

#[test]
fn preedit_applies_body_typography_rules() {
    let mut eng = LayoutEngine::new();
    // 中西文间距：与正文同规则注入
    let l = eng.layout_preedit("ni好a");
    assert_eq!(l.layout_text(), "ni 好 a");
    // 组合串内光标（IME 报告原串字节偏移）经 OffsetMap 映射后单调
    let (x_hao, _, _) = eng.caret_xy(&l, 2); // 「好」前
    let (x_a, _, _) = eng.caret_xy(&l, 5); // 「a」前
    assert!(x_a > x_hao, "preedit 光标映射应单调: {x_hao} -> {x_a}");
    // kinsoku 胶水：省略号 run 注入 WJ
    let l2 = eng.layout_preedit("中……");
    assert!(
        l2.layout_text().contains('\u{2060}'),
        "胶水段应注入 WJ: {:?}",
        l2.layout_text()
    );
}

#[test]
fn word_joiner_adds_no_advance() {
    // tofu 守卫：fallback 字体链中 WJ 必须近零宽（实测 1.29px 零墨字形）。
    // 若某天字体链退化把 WJ 渲染成缺字形盒（advance ≥ 半字宽），此测试先红。
    let mut eng = LayoutEngine::new();
    let plain = single_line_width(&mut eng, "中A");
    let glued = single_line_width(&mut eng, "中\u{2060}A");
    assert!(
        (glued - plain).abs() < 2.0,
        "WJ advance 不可见级（<2px）: plain={plain} glued={glued}"
    );
}

// ---- 表格单元格富文本（B3）：剥标记坐标 → 可见文本坐标 ----

type CellSpans = Option<Vec<(usize, usize, Style)>>;

/// 打开单行表格，取第 row 行第 col 个单元格的样式段与可见文本长。
fn cell_spans(doc: &str, row: usize, col: usize) -> (CellSpans, usize) {
    let s = Session::open(doc);
    let (id, _, _) = s.blocks()[0].clone();
    let t = s.table(id).unwrap();
    let cell = if row == 0 {
        &t.header[col]
    } else {
        &t.rows[row - 1][col]
    };
    let spans = cell_style_spans(cell.range, cell.text.len(), &cell.inlines, [0, 0, 0]);
    (spans, cell.text.len())
}

fn at(spans: &[(usize, usize, Style)], p: usize) -> Style {
    spans
        .iter()
        .find(|(s, e, _)| *s <= p && p < *e)
        .map(|x| x.2)
        .unwrap_or_default()
}

#[test]
fn cell_spans_map_markers_to_visible_text() {
    let doc = "| **粗** | *斜* | `code` | [t](http://a) |\n| --- | --- | --- | --- |\n";
    let (bold, n) = cell_spans(doc, 0, 0);
    let bold = bold.expect("粗体应可映射");
    assert_eq!(n, "粗".len());
    assert_eq!(
        bold,
        vec![(
            0,
            n,
            Style {
                bold: true,
                ..Default::default()
            }
        )]
    );

    let (it, n) = cell_spans(doc, 0, 1);
    let it = it.expect("斜体应可映射");
    assert_eq!(
        it,
        vec![(
            0,
            n,
            Style {
                italic: true,
                ..Default::default()
            }
        )]
    );

    let (mono, n) = cell_spans(doc, 0, 2);
    let mono = mono.expect("行内代码应可映射");
    // 行内代码：等宽 + 代码底色（与正文同一装饰语义）。
    assert_eq!(
        mono,
        vec![(
            0,
            n,
            Style {
                mono: true,
                code_bg: true,
                ..Default::default()
            }
        )]
    );

    let (link, n) = cell_spans(doc, 0, 3);
    let link = link.expect("链接应可映射");
    assert_eq!(n, 1);
    assert!(link.iter().all(|(s, e, _)| (*s, *e) == (0, 1)) && at(&link, 0).link);
}

#[test]
fn cell_spans_cover_visible_text_densely() {
    let docs = [
        "| 中文 **粗** 混排 |\n| --- |\n",
        "| a *b* `c` [d](u) |\n| --- |\n",
        "|  |\n| --- |\n",
    ];
    for doc in docs {
        let (spans, n) = cell_spans(doc, 0, 0);
        let spans = spans.expect("应可映射");
        let mut pos = 0;
        for &(s, e, _) in &spans {
            assert_eq!(s, pos, "致密覆盖不得有缝: {doc:?}");
            assert!(s < e);
            pos = e;
        }
        assert_eq!(pos, n, "覆盖须恰好等于可见文本长: {doc:?}");
    }
}

#[test]
fn cell_spans_none_when_bytes_diverge() {
    // `\|` 在 core 侧被解转义为单个 `|`，与剥标记长度不一致 → 降级信号
    let (spans, n) = cell_spans("| a\\|b |\n| --- |\n", 0, 0);
    assert!(spans.is_none(), "转义改写字节时应返回 None");
    assert_eq!(n, "a|b".len());
}

// ---- 脚注（渲染态）：引用 `[^id]` → `[n]`，定义块前缀/返回标记 ----

/// 打开文档，取第 `idx` 个块，带全文编号表排版（caret 为文档绝对偏移）。
fn layout_footnote_block(
    text: &str,
    idx: usize,
    caret: usize,
    width: f32,
    reveal_all: bool,
) -> (LayoutEngine, BlockLayout, Range) {
    let s = Session::open(text);
    let (id, kind, range) = s.blocks()[idx].clone();
    let inlines = s.block_inlines(id).unwrap_or_default();
    let ctx = FootnoteCtx::new(s.footnote_ids(), Theme::light().muted);
    let mut eng = LayoutEngine::new();
    let l = eng.layout_block_with_footnotes(
        kind,
        &text[range.start..range.end],
        range,
        &inlines,
        width,
        caret,
        &[],
        reveal_all,
        Some(&ctx),
        &[],
        None,
        false,
    );
    (eng, l, range)
}

fn first_footnote_ref(nodes: &[InlineNode]) -> Option<&InlineNode> {
    for n in nodes {
        if n.kind == InlineKind::FootnoteRef {
            return Some(n);
        }
        if let Some(f) = first_footnote_ref(&n.children) {
            return Some(f);
        }
    }
    None
}

#[test]
fn footnote_ref_renders_number_with_link_color() {
    let src = "see [^a] now\n\n[^a]: note\n";
    let (_eng, l, _) = layout_footnote_block(src, 0, usize::MAX, 400.0, false);
    assert_eq!(
        l.layout_text(),
        "see [1] now",
        "引用应渲染为 [n]，不暴露 ^ 与 id 原文"
    );
    let link = Theme::light().link;
    let want = Color::rgb(link[0], link[1], link[2]);
    let colored = l
        .buffer
        .layout_runs()
        .flat_map(|r| r.glyphs.iter())
        .filter(|g| g.color_opt == Some(want))
        .count();
    assert!(colored >= 3, "`[1]` 字形应使用链接色: colored={colored}");
    // 上标：`[1]` 字形按 sup（负偏移）抬升，且缩到 0.75 倍；与 HTML `<sup>` 一致。
    assert!(
        l.script_ranges.iter().any(|&(_, _, dy)| dy < 0.0),
        "脚注引用应为上标（负基线偏移）: {:?}",
        l.script_ranges
    );
    // 块处于活动态但光标不在引用节点内时，仍保持上标编号（活动/非活动一致）。
    let (_eng2, l_active, _) = layout_footnote_block(src, 0, 0, 400.0, false);
    assert_eq!(l_active.layout_text(), "see [1] now");
    assert!(l_active.script_ranges.iter().any(|&(_, _, dy)| dy < 0.0));
}

#[test]
fn footnote_ref_without_number_keeps_source() {
    let src = "see [^a] now\n\n[^a]: note\n";
    let s = Session::open(src);
    let (id, kind, range) = s.blocks()[0].clone();
    let inlines = s.block_inlines(id).unwrap();
    let mut eng = LayoutEngine::new();
    // 无编号表（打印等降级路径）：不伪造编号，保留 `[^id]`。
    let l = eng.layout_block(
        kind,
        &src[range.start..range.end],
        range,
        &inlines,
        400.0,
        usize::MAX,
        &[],
        false,
    );
    assert_eq!(l.layout_text(), "see [^a] now");
    let (x, y, _) = eng.caret_xy(&l, 4);
    let off = eng.hit(&l, x + 0.5, y + 1.0);
    assert!((4..=8).contains(&off), "原文降级态点击不得串位: {off}");
}

#[test]
fn footnote_ref_source_mode_shows_raw() {
    let src = "see [^a] now\n\n[^a]: note\n";
    let (_eng, l, _) = layout_footnote_block(src, 0, usize::MAX, 400.0, true);
    assert_eq!(l.layout_text(), "see [^a] now", "源码模式不受替换影响");
}

/// DEF-T05-001：源码模式原样显示 `[TOC]` 标记（文档字节也未被生成目录替换）。
#[test]
fn toc_marker_shows_raw_in_source_mode() {
    let src = "[TOC]\n\n# Hello\n";
    let (_eng, l, _) = layout_footnote_block(src, 0, usize::MAX, 400.0, true);
    assert_eq!(l.layout_text(), "[TOC]", "源码模式必须显示原始标记");
}

#[test]
fn toc_display_text_lists_headings_with_indent() {
    let entries = vec![
        (1u8, "一".to_string()),
        (2u8, "二 and c".to_string()),
        (1u8, "三".to_string()),
    ];
    assert_eq!(toc_display_text(&entries), "• 一\n   • 二 and c\n• 三");
    assert_eq!(toc_display_text(&[]), "");
}

#[test]
fn footnote_ref_hit_maps_into_source_range_after_edits() {
    // 引用前后插入/删除字符后重新排版：点击 `[1]` 任一位置都必须落在源码区间内。
    for src in [
        "a[^n]b\n\n[^n]: x\n",
        "aX[^n]b\n\n[^n]: x\n",
        "a[^n]bY\n\n[^n]: x\n",
    ] {
        let (eng, l, range) = layout_footnote_block(src, 0, usize::MAX, 400.0, false);
        let s = Session::open(src);
        let (id, _, _) = s.blocks()[0].clone();
        let inlines = s.block_inlines(id).unwrap();
        let node = first_footnote_ref(&inlines).expect("应有脚注引用节点");
        let (rs, re) = (node.range.start - range.start, node.range.end - range.start);
        let (x0, y, _) = eng.caret_xy(&l, rs);
        let (x1, _, _) = eng.caret_xy(&l, re);
        assert!(x1 > x0, "替换段应有宽度: {src:?}");
        for t in [0.25, 0.75] {
            let off = eng.hit(&l, x0 + (x1 - x0) * t, y + 1.0);
            assert!(
                (rs..=re).contains(&off),
                "{src:?} 点击 `[1]` 应落在源码引用区间 [{rs},{re}]: {off}"
            );
        }
    }
}

#[test]
fn footnote_def_hides_marker_and_appends_return_mark() {
    let src = "body[^a]\n\n[^a]: note\n";
    let (eng, l, _range) = layout_footnote_block(src, 1, usize::MAX, 400.0, false);
    assert_eq!(l.layout_text(), "1. note↩");
    // 点击 `1. ` 前缀 → 内容起点（`[^a]: ` 长 6）。
    let (x0, y, _) = eng.caret_xy(&l, 0);
    assert_eq!(eng.hit(&l, x0 + 1.0, y + 1.0), 6, "前缀应映射到内容起点");
    // 点击块末 `↩` → 块末。
    let body_len = "[^a]: note".len();
    let (xs, ys, _) = eng.caret_xy(&l, body_len);
    assert_eq!(
        eng.hit(&l, xs + 1.0, ys + 1.0),
        body_len,
        "返回标记应映射到块末"
    );
}

#[test]
fn footnote_def_without_reference_hides_marker_without_number() {
    let src = "no refs\n\n[^b]: unused\n";
    let (_eng, l, _) = layout_footnote_block(src, 1, usize::MAX, 400.0, false);
    assert_eq!(l.layout_text(), "unused↩", "未编号定义不显示编号前缀");
}

#[test]
fn footnote_def_reveals_source_when_caret_inside() {
    let src = "body[^a]\n\n[^a]: note\n";
    let s = Session::open(src);
    let (_, _, range) = s.blocks()[1].clone();
    let (_eng, l, _) = layout_footnote_block(src, 1, range.start + 2, 400.0, false);
    assert_eq!(l.layout_text(), "[^a]: note", "光标进入沿用 reveal 显源码");
}

#[test]
fn footnote_def_number_prefix_survives_typo_spacing() {
    // `[^a]:中文`：中西文间距段与编号插入段同起点，插入段必须排在前，否则被吞。
    let src = "see [^a]\n\n[^a]:中文\n";
    let (_eng, l, _) = layout_footnote_block(src, 1, usize::MAX, 400.0, false);
    let t = l.layout_text();
    assert!(t.starts_with("1. "), "编号前缀不得被间距段挤掉: {t:?}");
    assert!(t.ends_with('↩'));
}

#[test]
fn hard_break_backslash_hidden_when_inactive_shown_when_active() {
    // REN-SWEEP-007：非活动块只消隐标记，源换行与文档字节/命中坐标不变。
    let src = "甲\\\n乙\n";
    let (eng, l) = layout_of(src, 400.0);
    assert_eq!(l.layout_text(), "甲\n乙", "不得显示反斜杠或遗留自动空格");
    let runs: Vec<_> = l.buffer.layout_runs().collect();
    assert_eq!(runs.len(), 2, "源硬换行必须保留为两个视觉行");
    let shaped = runs.iter().map(|r| r.text).collect::<Vec<_>>().join("\n");
    assert_eq!(shaped, l.layout_text(), "实际 shaped 文本同样不得泄露标记");
    assert_eq!(l.source_newlines, vec![4], "源换行仍参与块内坐标");
    let (x, y, _) = eng.caret_xy(&l, 5);
    assert_eq!(eng.hit(&l, x + 0.1, y + 1.0), 5, "第二行命中仍映射到乙");

    let raw = "a\\\nb\n";
    let (_, active) = layout_with_caret(raw, 1, 400.0);
    assert_eq!(active.layout_text(), "a\\\nb", "光标进入块显示源码");
    let s = Session::open(src);
    assert_eq!(s.serialize(), src, "视图消隐不得改写文档");
    let raw_session = Session::open(raw);
    let (_, kind, range) = raw_session.blocks()[0].clone();
    let inlines = raw_session
        .block_inlines(raw_session.blocks()[0].0)
        .unwrap();
    let source = LayoutEngine::new().layout_block(
        kind.clone(),
        &raw[range.start..range.end],
        range,
        &inlines,
        400.0,
        usize::MAX,
        &[],
        true,
    );
    assert_eq!(source.layout_text(), "a\\\nb", "源码模式保留反斜杠");
    assert_ne!(
        active.reveal_state(&kind, range, usize::MAX),
        active.reveal_state(&kind, range, 1),
        "光标进出普通段落必须使布局缓存失效"
    );
}

#[test]
fn hard_break_backslash_skips_even_runs_and_code_content() {
    // 偶数反斜杠不是 CommonMark 硬换行标记；普通文本两个都保留。
    let (_, literal) = layout_of("a\\\\\nb\n", 400.0);
    assert_eq!(literal.layout_text(), "a\\\\\nb");
    // 行内代码与围栏代码中的反斜杠是代码内容，不得消隐。
    let (_, inline) = layout_of("`a\\\nb`\n", 400.0);
    assert!(
        inline.layout_text().contains("a\\\nb"),
        "{:?}",
        inline.layout_text()
    );
    let (_, fenced) = layout_of("```text\na\\\nb\n```\n", 400.0);
    assert!(
        fenced.layout_text().contains("a\\\nb"),
        "{:?}",
        fenced.layout_text()
    );
}

#[test]
fn footnote_def_number_is_not_duplicated_in_shaped_text() {
    // REN-SWEEP-006：`cell.text/layout_text` 正确不代表字形输入正确 —— 基础样式段
    // 曾整段覆盖含装饰的 layout 文本，set_rich_text 拼接后行尾多出一份 `n.`。
    let src = "# 脚注\n\n正文[^note]与[^two]。\n\n[^note]: 第一个脚注。\n[^two]: 第二个脚注。\n";
    let s = Session::open(src);
    let ctx = FootnoteCtx::new(s.footnote_ids(), Theme::light().muted);
    let mut eng = LayoutEngine::new();
    let mut defs = 0usize;
    for (id, kind, range) in s.blocks().iter() {
        if !matches!(kind, BlockKind::FootnoteDef) {
            continue;
        }
        let inlines = s.block_inlines(*id).unwrap_or_default();
        let l = eng.layout_block_with_footnotes(
            kind.clone(),
            &src[range.start..range.end],
            *range,
            &inlines,
            600.0,
            usize::MAX,
            &[],
            false,
            Some(&ctx),
            &[],
            None,
            false,
        );
        defs += 1;
        let shaped: String = l.buffer.layout_runs().map(|r| r.text).collect();
        assert_eq!(
            shaped,
            l.layout_text(),
            "字形输入必须与 layout 文本一致（不得重复编号）"
        );
        assert_eq!(
            shaped.matches("↩").count(),
            1,
            "返回标记只出现一次: {shaped:?}"
        );
    }
    assert_eq!(defs, 2, "夹具应有两个脚注定义块");
}

#[test]
fn offset_map_insertion_at_end_keeps_suffix_out_of_caret() {
    let (m, _) = OffsetMap::from_spans(3, &[Sub::insert(3, "↩")]);
    assert_eq!(m.layout_len, 3 + "↩".len());
    assert_eq!(m.doc_to_layout(3), 3, "块末光标停在装饰之前");
    assert_eq!(m.layout_to_doc(3), 3, "点击装饰 → 块末");
    assert_eq!(m.layout_to_doc(m.layout_len), 3);
}

#[test]
fn offset_map_insertion_after_elided_prefix_maps_to_content_start() {
    // "[^a]: x"：前缀 [0,6) 消隐，`1. ` 于 6 处插入；内容 "x" 仍在 6..7。
    let (m, _) = OffsetMap::from_spans(7, &[Sub::elide(0, 6), Sub::insert(6, "1. ")]);
    assert_eq!(m.layout_len, "1. x".len());
    assert_eq!(m.doc_to_layout(0), 0);
    assert_eq!(m.doc_to_layout(6), 3);
    assert_eq!(m.doc_to_layout(7), 4);
    assert_eq!(m.layout_to_doc(0), 6, "前缀任一点击 → 内容起点");
    assert_eq!(m.layout_to_doc(2), 6);
    assert_eq!(m.layout_to_doc(3), 6);
}

#[test]
fn footnote_epoch_invalidates_tail_definition_layout() {
    use crate::view::cache::{Cache, LayoutKey, Memo, Revs};

    let before = "see [^x]\n\n[^x]: X\n\n[^y]: Y\n";
    let after = "[^y] see [^x]\n\n[^x]: X\n\n[^y]: Y\n";
    let revs = Revs::default();
    let mut eng = LayoutEngine::new();

    let s1 = Session::open(before);
    let (id1, kind1, range1) = s1.blocks()[2].clone();
    assert_eq!(kind1, BlockKind::FootnoteDef);
    let ctx1 = FootnoteCtx::new(s1.footnote_ids(), Theme::light().muted);
    let base1 = revs.base(800.0, 1.0, 600.0, false, s1.footnote_epoch());
    let inlines1 = s1.block_inlines(id1).unwrap_or_default();
    let l1 = eng.layout_block_with_footnotes(
        kind1.clone(),
        &before[range1.start..range1.end],
        range1,
        &inlines1,
        800.0,
        usize::MAX,
        &[],
        false,
        Some(&ctx1),
        &[],
        None,
        false,
    );
    assert_eq!(l1.layout_text(), "Y↩", "未被引用：仅隐藏标记");
    let reveal1 = l1.reveal_state(&kind1, range1, usize::MAX);
    let mut cache = Cache::default();
    cache.layouts.insert(
        id1,
        Memo {
            key: LayoutKey {
                base: base1,
                reveal: reveal1,
                toc: 0,
                highlight_epoch: 0,
            },
            value: l1,
        },
    );
    assert!(
        cache
            .layout(id1, &kind1, range1, base1, usize::MAX, 0)
            .is_some(),
        "代次未变必须命中"
    );

    let s2 = Session::open(after);
    let (id2, kind2, range2) = s2.blocks()[2].clone();
    let ctx2 = FootnoteCtx::new(s2.footnote_ids(), Theme::light().muted);
    let base2 = revs.base(800.0, 1.0, 600.0, false, s2.footnote_epoch());
    assert_ne!(base1, base2, "文首新增未出现的引用 → 全文编号变化");
    assert!(
        cache
            .layout(id2, &kind2, range2, base2, usize::MAX, 0)
            .is_none(),
        "旧布局不得以新代次命中"
    );
    let inlines2 = s2.block_inlines(id2).unwrap_or_default();
    let l2 = eng.layout_block_with_footnotes(
        kind2,
        &after[range2.start..range2.end],
        range2,
        &inlines2,
        800.0,
        usize::MAX,
        &[],
        false,
        Some(&ctx2),
        &[],
        None,
        false,
    );
    assert_eq!(l2.layout_text(), "1. Y↩", "新引用让文末定义块获得编号");
}

// ---- Mermaid v0：detect_type 接入卡片头 + 空块提示 ----

#[test]
fn mermaid_badge_flowchart_and_sequence() {
    let flowchart_src = "```mermaid\ngraph TD\nA-->B\n```";
    let (_, fc_layout) = layout_of(flowchart_src, 400.0);
    assert!(
        fc_layout.layout_text().contains("◈ 流程图"),
        "flowchart 应渲染为 ◈ 流程图 卡片头；实际: {}",
        fc_layout.layout_text()
    );
    // 开围栏行本身应被替换掉（没有 ``` 残留）
    assert!(!fc_layout.layout_text().contains("```mermaid"));

    let seq_src = "```mermaid\nsequenceDiagram\n  A->>B: hi\n```";
    let (_, seq_layout) = layout_of(seq_src, 400.0);
    assert!(
        seq_layout.layout_text().contains("◈ 时序图"),
        "sequenceDiagram 应渲染为 ◈ 时序图 卡片头；实际: {}",
        seq_layout.layout_text()
    );
}

#[test]
fn mermaid_badge_other_types_and_fallback() {
    let cases = [
        ("```mermaid\nclassDiagram\nclass A\n```", "◈ 类图"),
        ("```mermaid\nstateDiagram-v2\n[*] --> Idle\n```", "◈ 状态图"),
        ("```mermaid\nerDiagram\nA ||--o{ B : r\n```", "◈ ER图"),
        ("```mermaid\ngantt\ntitle Test\n```", "◈ 甘特图"),
        ("```mermaid\npie\ntitle Pie\nA : 1\n```", "◈ 饼图"),
        ("```mermaid\njourney\ntitle X\n```", "◈ mermaid"),
    ];
    for (src, want) in cases {
        let (_, l) = layout_of(src, 400.0);
        assert!(
            l.layout_text().contains(want),
            "{src:?} 应得到 {want}；实际: {}",
            l.layout_text()
        );
    }
}

#[test]
fn mermaid_source_mode_shows_fences_raw() {
    // 源码模式（reveal_all=true）不做卡片化，保留原始围栏。
    let src = "```mermaid\ngraph TD\nA-->B\n```";
    let s = Session::open(src);
    let (_, kind, range) = s.blocks()[0].clone();
    let inlines = s.block_inlines(s.blocks()[0].0).unwrap();
    let mut eng = LayoutEngine::new();
    let block_text = &src[range.start..range.end];
    let l = eng.layout_block(
        kind,
        block_text,
        range,
        &inlines,
        400.0,
        usize::MAX,
        &[],
        true, // reveal_all = 源码模式
    );
    assert!(
        l.layout_text().contains("```mermaid"),
        "源码模式应保留原始围栏；实际: {}",
        l.layout_text()
    );
    assert!(!l.layout_text().contains("◈ 流程图"));
}

// ---- N9：代码块/行内代码 CJK 字体族诊断与回归 ----

/// 常用 CJK 统一表意文字区（测试字符「准备练习环境」均在此）。
fn is_han(c: char) -> bool {
    matches!(c as u32, 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF)
}

/// 遍历排版字形，返回 (字符, 族名列表, 是否等宽, face id)。
/// 直接查 fontdb 才能看清实际命中的系统字体（cosmic 的 Attrs 只记录意图）。
fn glyph_faces(
    eng: &mut LayoutEngine,
    l: &BlockLayout,
) -> Vec<(char, Vec<String>, bool, cosmic_text::fontdb::ID)> {
    let mut out = Vec::new();
    for run in l.buffer.layout_runs() {
        for g in run.glyphs.iter() {
            let ch = run.text[g.start..g.end].chars().next().unwrap_or('\u{0}');
            let (families, monospaced) = eng
                .font_system()
                .db_mut()
                .face(g.font_id)
                .map(|f| {
                    (
                        f.families.iter().map(|(name, _)| name.clone()).collect(),
                        f.monospaced,
                    )
                })
                .unwrap_or_default();
            out.push((ch, families, monospaced, g.font_id));
        }
    }
    out
}

/// 首个 Han 字形的 (族名列表, face id)；缺 Han 时 panic（调用方文本必含 Han）。
fn han_face(
    eng: &mut LayoutEngine,
    l: &BlockLayout,
    ctx: &str,
) -> (Vec<String>, cosmic_text::fontdb::ID) {
    glyph_faces(eng, l)
        .into_iter()
        .find(|(c, ..)| is_han(*c))
        .map(|(_, fam, _, id)| (fam, id))
        .unwrap_or_else(|| panic!("{ctx}: 未找到 Han 字形"))
}

/// 排版指定索引的块并返回块类型（诊断不同块类共用同一字体路径）。
fn layout_kind_at(text: &str, idx: usize, width: f32) -> (LayoutEngine, BlockLayout, BlockKind) {
    let s = Session::open(text);
    let (id, kind, range) = s.blocks()[idx].clone();
    let inlines = s.block_inlines(id).unwrap_or_default();
    let mut eng = LayoutEngine::new();
    let l = eng.layout_block(
        kind.clone(),
        &text[range.start..range.end],
        range,
        &inlines,
        width,
        usize::MAX,
        &[],
        false,
    );
    (eng, l, kind)
}

/// 墨水覆盖率：alpha>0 像素数 / 非空包围盒面积（同文本同字体下可跨字号比较）。
fn ink_coverage(px: &[u8], w: u32, h: u32) -> f64 {
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (u32::MAX, u32::MAX, 0, 0);
    let mut ink = 0u64;
    for y in 0..h {
        for x in 0..w {
            if px[((y * w + x) * 4 + 3) as usize] > 0 {
                ink += 1;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }
    if ink == 0 {
        return 0.0;
    }
    let area = u64::from(max_x - min_x + 1) * u64::from(max_y - min_y + 1);
    ink as f64 / area as f64
}

#[test]
fn codeblock_cjk_glyph_font() {
    let (mut cb_eng, cb) = layout_of("```rust\n// 准备练习环境\n```", 400.0);
    let (mut par_eng, par) = layout_of("准备练习环境", 400.0);
    let (cb_fam, cb_id) = han_face(&mut cb_eng, &cb, "fenced code");
    let (par_fam, par_id) = han_face(&mut par_eng, &par, "paragraph");
    // 修复前基线取证（--nocapture 可见）
    println!("[N9] fenced Han families = {cb_fam:?}, face = {cb_id:?}");
    println!("[N9] paragraph Han families = {par_fam:?}, face = {par_id:?}");
    assert_eq!(cb_fam, par_fam, "代码块 Han 应与正文 Han 同族");
    assert_eq!(cb_id, par_id, "代码块 Han 与正文 Han 应为同一 face");
}

#[test]
fn mermaid_attrs_match_fenced_mono_and_preserve_paragraph_doc_font() {
    let attrs_for = |kind| {
        attrs_of(
            kind,
            Style::default(),
            [0, 0, 0],
            DocFontFamily::Serif,
            Some("Consolas"),
            16.0,
        )
    };
    let mermaid = attrs_for(BlockKind::Mermaid);
    let fenced = attrs_for(BlockKind::FencedCode(None));
    assert_eq!(mermaid.family, fenced.family);
    assert_eq!(mermaid.family, Family::Name("Consolas"));

    let paragraph = attrs_for(BlockKind::Paragraph);
    assert_eq!(paragraph.family, Family::Name("SimSun"));
}

#[test]
fn codeblock_latin_face_is_mono_family() {
    // Latin 仍必须是启动时探测到的等宽族（Consolas），不能被 CJK 修复带偏。
    let (mut eng, l) = layout_of("```rust\nlet x = 1;\n```", 400.0);
    let want = eng.mono_family().expect("测试机应探测到等宽族");
    let latin = glyph_faces(&mut eng, &l)
        .into_iter()
        .find(|(c, ..)| c.is_ascii_alphabetic())
        .expect("代码块应有 Latin 字形");
    println!(
        "[N9] fenced Latin families = {:?}, monospaced = {}, face = {:?}",
        latin.1, latin.2, latin.3
    );
    assert!(
        latin.1.iter().any(|name| name == want),
        "Latin 应命中等宽族 {want}；实际: {:?}",
        latin.1
    );
    assert!(latin.2, "Latin 字形应来自等宽 face");
}

#[test]
fn inline_code_cjk_matches_paragraph() {
    let (mut inline_eng, inline) = layout_of("a `中文` b", 400.0);
    let (mut par_eng, par) = layout_of("中文", 400.0);
    let (inline_fam, inline_id) = han_face(&mut inline_eng, &inline, "inline code");
    let (par_fam, par_id) = han_face(&mut par_eng, &par, "paragraph");
    assert_eq!(inline_fam, par_fam, "行内代码 Han 应与正文同族");
    assert_eq!(inline_id, par_id, "行内代码 Han 与正文 Han 应为同一 face");
}

#[test]
fn code_block_kinds_share_body_han_face() {
    let (mut par_eng, par) = layout_of("准备练习环境", 400.0);
    let (par_fam, par_id) = han_face(&mut par_eng, &par, "paragraph");
    // (源码, 期望块类)；HtmlBlock / FrontMatter 无卡片头，文本原样参与排版。
    let cases = [
        ("    准备练习环境", "indented code"),
        ("<div>\n准备练习环境\n</div>", "html block"),
        ("---\ntitle: 准备练习环境\n---", "front matter"),
    ];
    for (src, ctx) in cases {
        let (mut eng, l, kind) = layout_kind_at(src, 0, 400.0);
        assert!(
            matches!(
                kind,
                BlockKind::IndentedCode | BlockKind::HtmlBlock | BlockKind::FrontMatter(_)
            ),
            "{ctx}: 块类型不符，实际 {kind:?}"
        );
        let (fam, id) = han_face(&mut eng, &l, ctx);
        assert_eq!(fam, par_fam, "{ctx}: Han 应与正文同族");
        assert_eq!(id, par_id, "{ctx}: Han 与正文 Han 应为同一 face");
    }
}

#[test]
fn codeblock_cjk_ink_coverage_close_to_paragraph() {
    // 渲染门禁：同文本下代码块 Han 的墨水覆盖率不得显著低于正文。
    // IndentedCode 不带卡片头/边框，且与段落一样只有一行纯 Han，噪声最小。
    let (mut cb_eng, cb) = layout_of("    准备练习环境", 400.0);
    assert!(matches!(
        Session::open("    准备练习环境").blocks()[0].1,
        BlockKind::IndentedCode
    ));
    let (cb_px, cb_w, cb_h) = cb_eng.rasterize(&cb, [0, 0, 0]);
    let cb_cov = ink_coverage(&cb_px, cb_w, cb_h);

    let (mut par_eng, par) = layout_of("准备练习环境", 400.0);
    let (par_px, par_w, par_h) = par_eng.rasterize(&par, [0, 0, 0]);
    let par_cov = ink_coverage(&par_px, par_w, par_h);

    println!("[N9] code coverage = {cb_cov:.4}, paragraph coverage = {par_cov:.4}");
    assert!(par_cov > 0.0, "正文应有墨水像素");
    assert!(
        cb_cov >= par_cov * 0.85,
        "代码块 Han 墨水覆盖率应 ≥ 正文 85%：code={cb_cov:.4} body={par_cov:.4}"
    );
}

#[test]
fn mermaid_empty_block_shows_hint() {
    let empty_src = "```mermaid\n```";
    let (_, l) = layout_of(empty_src, 400.0);
    assert!(
        l.layout_text().contains("空 Mermaid 块 — 点击插入模板"),
        "空 mermaid 块应显示提示；实际: {}",
        l.layout_text()
    );
    // 同样全是 %% 注释也算空块
    let comment_only = "```mermaid\n%% 注释行\n%% 再一行\n```";
    let (_, l2) = layout_of(comment_only, 400.0);
    assert!(
        l2.layout_text().contains("空 Mermaid 块 — 点击插入模板"),
        "只有 %% 注释的块也应显示提示；实际: {}",
        l2.layout_text()
    );
}

// ---- Wave 7 Task 3b: inline HTML decorations / br / sub-sup ----

#[test]
fn inline_html_kbd_produces_decoration_under() {
    // <kbd>Ctrl</kbd> → decorations_under 应含 KbdFrame
    let (_, l) = layout_of("Press <kbd>Ctrl</kbd> to save", 500.0);
    assert!(
        l.decorations_under
            .iter()
            .any(|(_, _, k)| matches!(k, UnderKind::KbdFrame)),
        "<kbd> 应生成 KbdFrame 前置装饰；实际: {:?}",
        l.decorations_under
    );
    // 光栅化结果应可见「Ctrl」
    assert!(l.layout_text().contains("Ctrl"));
}

#[test]
fn inline_html_pre_renders_mono_not_literal_tags() {
    let text = "正文 <pre>预格式</pre> 结尾";
    let s = Session::open(text);
    let (id, _, range) = s.blocks()[0].clone();
    let inlines = s.block_inlines(id).unwrap();
    let segs = crate::view::style::style_spans(range.len(), range, &inlines);
    let (_, l) = layout_of(text, 500.0);
    assert!(
        !l.layout_text().contains("<pre>") && l.layout_text().contains("预格式"),
        "段落内 <pre> 应按行内代码渲染；实际: {:?}",
        l.layout_text()
    );
    assert!(
        segs.iter().any(|(_, _, st)| st.mono),
        "<pre> 应复用 code 的等宽样式；segs: {segs:?}"
    );
}

#[test]
fn inline_html_q_wraps_content_in_quotes_and_semantic_tags_render() {
    // <q> 非激活：内容两端插入中文双引号，标签本身消隐
    let (_, l) = layout_of("他说 <q>你好</q> 结束", 500.0);
    let text = l.layout_text();
    assert!(
        text.contains('“') && text.contains('”') && text.contains("你好"),
        "<q> 应渲染为双引号包裹；实际: {:?}",
        text
    );
    let open = text.find('“').unwrap();
    let close = text.find('”').unwrap();
    assert!(open < close, "引号顺序应为开在前；实际: {text:?}");
    assert!(
        text[open..close].contains("你好"),
        "内容应位于引号之间；实际: {text:?}"
    );
    assert!(!l.layout_text().contains("<q>"), "标签不应泄漏");
    // 语义标签不泄漏源码，内容可见
    for (src, content) in [
        ("<strong>x</strong>", "x"),
        ("<b>x</b>", "x"),
        ("<em>x</em>", "x"),
        ("<u>x</u>", "x"),
        ("<s>x</s>", "x"),
        ("<del>x</del>", "x"),
        ("<ins>x</ins>", "x"),
        ("<cite>x</cite>", "x"),
        ("<var>x</var>", "x"),
        ("<samp>x</samp>", "x"),
        ("<small>x</small>", "x"),
        ("<span>x</span>", "x"),
    ] {
        let (_, l) = layout_of(src, 500.0);
        assert_eq!(l.layout_text(), content, "{src} 应只渲染内容");
    }
}

#[test]
fn inline_html_style_bg_and_underline_decorations() {
    let (_, l) = layout_of(
        "<span style=\"background-color:#ff0;text-decoration:underline\">x</span>",
        500.0,
    );
    assert!(
        l.decorations_under
            .iter()
            .any(|(_, _, k)| matches!(k, UnderKind::StyleBg([255, 255, 0]))),
        "background-color 应生成 StyleBg 装饰；实际: {:?}",
        l.decorations_under
    );
    assert!(
        l.decorations_over
            .iter()
            .any(|(_, _, k)| matches!(k, OverKind::Underline)),
        "text-decoration:underline 应生成实线下划线；实际: {:?}",
        l.decorations_over
    );
}

#[test]
fn inline_html_mark_produces_decoration_under() {
    // <mark>重点</mark> → decorations_under 应含 MarkBg
    let (_, l) = layout_of("这是 <mark>重点</mark> 内容", 500.0);
    assert!(
        l.decorations_under
            .iter()
            .any(|(_, _, k)| matches!(k, UnderKind::MarkBg)),
        "<mark> 应生成 MarkBg 前置装饰；实际: {:?}",
        l.decorations_under
    );
}

#[test]
fn inline_html_br_breaks_line() {
    // foo<br>bar → layout_text 里应有换行符，line_bases ≥ 2
    let text = "foo<br>bar";
    let s = Session::open(text);
    let (id, kind, range) = s.blocks()[0].clone();
    let inlines = s.block_inlines(id).unwrap();
    eprintln!("DEBUG kind = {kind:?}, range = {range:?}");
    eprintln!("DEBUG inlines = {inlines:#?}");
    let (_, l) = layout_of(text, 500.0);
    eprintln!("DEBUG layout_text = {:?}", l.layout_text());
    eprintln!("DEBUG line_bases = {:?}", l.line_bases);
    assert!(
        l.layout_text().contains('\n'),
        "<br> 应替换为 \\n；layout_text: {:?}",
        l.layout_text()
    );
    assert!(
        l.line_bases.len() >= 2,
        "<br> 应断成至少 2 行；line_bases: {:?}",
        l.line_bases
    );
}

#[test]
fn inline_html_sub_sup_scaled_down() {
    // x<sup>2</sup>y<sub>1</sub> → style_segments 里 Sub/Sup 段 size_factor ≈ 0.75
    // cosmic-text run 的 line_height 是整行统一值，无法区分 sub/sup；
    // 改为验证 style_segments 里 size_factor 字段（attrs_of 据此乘进 Attrs::size）。
    let text = "x<sup>2</sup>y<sub>1</sub>";
    let s = Session::open(text);
    let (id, _, range) = s.blocks()[0].clone();
    let inlines = s.block_inlines(id).unwrap();
    let segs = crate::view::style::style_spans(range.len(), range, &inlines);
    let has_sup_factor = segs
        .iter()
        .any(|(_, _, st)| st.size_factor.is_some_and(|f| (f - 0.75).abs() < 1e-3));
    assert!(
        has_sup_factor,
        "sup 段应有 size_factor ≈ 0.75；segs: {segs:?}"
    );
    // 同时也验证 layout_text 里 Sub/Sup 内容可见（标签被消隐，只留内容）
    let (_, l) = layout_of(text, 500.0);
    let visible = l.layout_text();
    assert!(
        visible.contains('x')
            && visible.contains('2')
            && visible.contains('y')
            && visible.contains('1'),
        "sup/sub 内容应可见；layout_text: {visible:?}"
    );
    // 缩字：sup/sub 字形的 font_size 必须小于正文 'x'，且登记了下沉/上浮偏移。
    let sizes: Vec<f32> = l
        .buffer
        .layout_runs()
        .flat_map(|r| r.glyphs)
        .map(|g| g.font_size)
        .collect();
    assert!(
        sizes.iter().filter(|&&s| s < 16.0 * 1.0).count() >= 2,
        "sup/sub 应缩字；glyph font_size: {sizes:?}"
    );
    assert!(
        !l.script_ranges.is_empty(),
        "sup/sub 应登记基线偏移区间；实际: {:?}",
        l.script_ranges
    );
}

/// 可见文本：剥掉禁则胶水 WJ 与中西文自动间距的空格，便于对整句做精确断言。
fn visible_text(lt: &str) -> String {
    lt.chars()
        .filter(|c| *c != '\u{2060}' && *c != ' ')
        .collect()
}

#[test]
fn link_markers_elide_atomically_across_cjk_spacing() {
    // 回归：`个[链接]` 的 CJK↔ASCII 间距曾在 `[` 上生成 fill，与单字符消隐段
    // 同起点竞争时 fill 胜出（from_spans「长者优先」），导致 `](url)` 已消隐
    // 但 `[` 残留的半展开态。现要求：非激活时两端标记必须同时消失。
    let src = "以及一个[链接](https://example.com)。\n";
    let full = "以及一个[链接](https://example.com)。";
    // caret 在链接之前：完全折叠
    let (_, l) = layout_with_caret(src, 0, 400.0);
    assert_eq!(visible_text(l.layout_text()), "以及一个链接。");
    // caret 落在链接内（`链` 的字节偏移 13）：整段源码标记完整显示
    let (_, l) = layout_with_caret(src, 13, 400.0);
    assert_eq!(visible_text(l.layout_text()), full);
    // caret 紧贴 `)` 右侧（40 是 `)`，41 即其后）：闭区间激活 → 整段展开，
    // 不得只余 `[`。二者取「全显示」，与全局 node_active 语义一致。
    let (_, l) = layout_with_caret(src, 41, 400.0);
    assert_eq!(visible_text(l.layout_text()), full);
    // caret 在 `。` 之后：离开链接区间 → 完全折叠
    let (_, l) = layout_with_caret(src, 43, 400.0);
    assert_eq!(visible_text(l.layout_text()), "以及一个链接。");
}

#[test]
fn link_markers_elide_in_surrounding_shapes() {
    // 三种周边形态都必须原子折叠：链接在行首、后紧跟中文标点、前面有空格。
    let cases: &[(&str, &str, &str)] = &[
        (
            "[链接](https://example.com)。\n",
            "链接。",
            "[链接](https://example.com)。",
        ),
        (
            "前文[链接](https://example.com)，后文\n",
            "前文链接，后文",
            "前文[链接](https://example.com)，后文",
        ),
        (
            "前文 [链接](https://example.com)后文\n",
            "前文链接后文",
            "前文[链接](https://example.com)后文",
        ),
    ];
    for (src, folded, expanded) in cases {
        let inside = src.find('链').unwrap(); // 链接文本内，节点必然激活
        let (_, l) = layout_with_caret(src, usize::MAX, 400.0);
        assert_eq!(
            visible_text(l.layout_text()),
            *folded,
            "非激活应完全折叠: src={src:?}"
        );
        let (_, l) = layout_with_caret(src, inside, 400.0);
        assert_eq!(
            visible_text(l.layout_text()),
            *expanded,
            "激活应完整显示: src={src:?}"
        );
    }
}

#[test]
fn link_markdown_markers_are_hidden_when_not_active() {
    // 内联链接、引用式链接、隐式方括号——只要 core 解析为 Link 节点，
    // layout 层必须消隐两端 Markdown 标记 `[...]` `(url)`。裸方括号 `[foo]`
    // （无 URL/引用定义）不产生 Link 节点，标记按原文保留——符合 Markdown 规范。
    let cases: &[(&str, &str)] = &[
        ("foo [x](http://x.com) bar\n", "foo x bar"), // 内联
        ("foo [x][ref] bar\n\n[ref]: http://x.com\n", "foo x bar"), // 引用式
        ("foo [x]\n\n[x]: http://x.com\n", "foo x"),  // 隐式方括号
        ("foo [x] bar\n", "foo [x] bar"),             // 裸方括号：不产生 Link，保留标记
    ];
    for (src, want_visible_substr) in cases {
        let (_, l) = layout_with_caret(src, usize::MAX, 400.0);
        let lt = l.layout_text();
        // 只检查「可见子串必须存在」——不管 CJK 自动间距插入的 WORD JOINER
        // （因此避免 contains 含标点的精确匹配）
        assert!(
            lt.contains(want_visible_substr.trim()),
            "src={src:?}: expected visible {want_visible_substr:?} in layout_text {lt:?}"
        );
    }
}

// ===== 容器内嵌套块（REN-SWEEP-002 / 004 / 009）=====

#[test]
fn nested_quote_markers_elide_all_levels() {
    // REN-SWEEP-004：`> > > x` 的每一级 `>` 都要消隐，级间空白形成层级缩进。
    let src = "> 一级\n> > 二级\n> > > 三级\n";
    let (_, l) = layout_doc_block(src, 0, usize::MAX, 400.0);
    let lt = l.layout_text();
    assert!(!lt.contains('>'), "多层引用标记必须全部消隐: {lt:?}");
    let lines: Vec<&str> = lt.lines().collect();
    assert_eq!(lines.len(), 3);
    let lead = |s: &str| s.len() - s.trim_start_matches(' ').len();
    assert!(lead(lines[0]) < lead(lines[1]), "二级应有缩进: {lt:?}");
    assert!(lead(lines[1]) < lead(lines[2]), "三级应有缩进: {lt:?}");
    assert_eq!(visible_text(lt).replace('\n', ""), "一级二级三级");
}

#[test]
fn nested_quote_active_line_shows_source() {
    // 光标进入某行只还原该行源码，其余层级标记仍消隐。
    let src = "> 一级\n> > 二级\n> > > 三级\n";
    let caret = src.find("二级").unwrap();
    let (_, l) = layout_doc_block(src, 0, caret, 400.0);
    let lines: Vec<&str> = l.layout_text().lines().collect();
    assert!(lines[0].contains(' '), "非活动行仍消隐: {lines:?}");
    assert!(lines[1].contains("> > 二级"), "活动行显源码: {lines:?}");
    assert!(lines[2].contains("   "), "非活动行仍缩进消隐: {lines:?}");
}

#[test]
fn quote_container_renders_nested_list_task_and_quote() {
    // REN-SWEEP-002：引用块内的有序/无序/任务列表与再嵌套引用不得露出裸标记。
    let src = "> 引用中的列表：\n\
               >\n\
               > 1. 第一项\n\
               >    - 子项含 **加粗** 与 `code`\n\
               >      > 引用中再引用\n\
               >      >\n\
               >      > - [ ] 任务\n\
               >      > - [x] 已完成\n";
    let (_, l) = layout_doc_block(src, 0, usize::MAX, 400.0);
    let lt = l.layout_text();
    assert!(!lt.contains('>'), "引用标记必须全部消隐: {lt:?}");
    assert!(!lt.contains("- "), "无序/任务列表裸标记必须消失: {lt:?}");
    assert!(
        !lt.contains("[ ]") && !lt.contains("[x]"),
        "复选框裸标记: {lt:?}"
    );
    assert!(
        !lt.contains("**") && !lt.contains('`'),
        "行内标记应消隐: {lt:?}"
    );
    assert!(lt.contains('•'), "无序列表应显示圆点: {lt:?}");
    assert!(lt.contains('☐') && lt.contains('☑'), "任务框应显示: {lt:?}");
    assert!(lt.contains("1."), "有序列表编号保留源码编号: {lt:?}");
    assert!(lt.contains("加粗") && !lt.contains("**加粗**"));
}

#[test]
fn list_item_continuation_nested_quote_markers_elided() {
    // REN-SWEEP-002/004：列表项续行里的嵌套引用 `>`（含多层）同样消隐。
    let src = "- 列表中的引用：\n  > 引用段落\n  >\n  > > 二级引用\n";
    let (_, l) = layout_doc_block(src, 0, usize::MAX, 400.0);
    let lt = l.layout_text();
    assert!(!lt.contains('>'), "列表项内引用标记必须消隐: {lt:?}");
    assert!(lt.contains("引用段落") && lt.contains("二级引用"));
}

#[test]
fn list_item_nested_fence_renders_code_lines() {
    // REN-SWEEP-002：列表项续行里的围栏代码按代码行渲染（围栏行消隐）。
    let src = "- 列表中的代码：\n  ```text\n  code in list\n  ```\n";
    let (_, l) = layout_doc_block(src, 0, usize::MAX, 400.0);
    let lt = l.layout_text();
    assert!(!lt.contains("```"), "围栏标记必须消隐: {lt:?}");
    assert!(lt.contains("code in list"), "代码内容应可见: {lt:?}");
    assert!(lt.contains('•'), "列表项圆点应显示: {lt:?}");
}

#[test]
fn list_item_nested_details_tags_elided() {
    // REN-SWEEP-002：列表项续行里的 details/summary 裸标签消隐，正文按行内语法排版。
    let src = "- 列表中的 details：\n  <details>\n  <summary>摘要</summary>\n\n  正文 **加粗**。\n  </details>\n";
    let (_, l) = layout_doc_block(src, 0, usize::MAX, 400.0);
    let lt = l.layout_text();
    for raw in ["<details>", "</details>", "<summary>", "</summary>"] {
        assert!(!lt.contains(raw), "裸标签 {raw} 必须消隐: {lt:?}");
    }
    assert!(!lt.contains("**"), "正文行内标记应消隐: {lt:?}");
    assert!(lt.contains("摘要"), "summary 文本应可见: {lt:?}");
    assert!(lt.contains("加粗"), "正文应可见: {lt:?}");
}

#[test]
fn details_body_paragraph_parses_inline_markdown() {
    // REN-SWEEP-009：details 正文是原生子块，`**加粗**` 必须按行内语法渲染。
    let src = "# T\n\n<details>\n<summary>折叠摘要</summary>\n\n这里是折叠正文，含 **加粗**。\n\n</details>\n";
    let s = Session::open(src);
    let (id, kind, range) = s
        .blocks()
        .into_iter()
        .find(|(_, k, _)| matches!(k, BlockKind::Paragraph))
        .expect("details 正文应拆为 Paragraph 子块");
    let inlines = s.block_inlines(id).unwrap();
    let mut eng = LayoutEngine::new();
    let block_text = &src[range.start..range.end];
    let l = eng.layout_block(
        kind,
        block_text,
        range,
        &inlines,
        400.0,
        usize::MAX,
        &[],
        false,
    );
    let lt = l.layout_text();
    assert!(
        !lt.contains("**"),
        "加粗标记必须消隐（不得显示原文）: {lt:?}"
    );
    assert!(lt.contains("加粗"), "加粗文本应可见: {lt:?}");
}
