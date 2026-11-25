//! 块级结构测试：列表 / 引用 / 分隔线 reveal、details 折叠、行内 HTML 装饰像素断言。

use super::*;

#[test]
fn list_bullet_toggles_on_caret_enter_leave() {
    // 「p\n\n- a\n」：列表块 bytes [3,7)，初始 caret=0 在块外
    let (mut a, _) = app_math("p\n\n- a\n", false);
    a.ensure_layouts_all();
    let (id, kind, _) = a.editor.session().blocks()[1].clone();
    assert_eq!(
        kind,
        BlockKind::ListItem {
            ordered: false,
            depth: 0,
            task: None
        }
    );
    assert_eq!(a.cache.layouts[&id].layout_text(), "• a");
    // 进入块：激活指纹变化 → 键失配 → 重排显源码
    assert!(a.editor.set_caret(4));
    a.ensure_layouts_all();
    assert_eq!(a.cache.layouts[&id].layout_text(), "- a");
    // 离开块：恢复符号
    assert!(a.editor.set_caret(0));
    a.ensure_layouts_all();
    assert_eq!(a.cache.layouts[&id].layout_text(), "• a");
    assert_eq!(a.block_height(geom(&a, id)), a.cache.layouts[&id].height);
}

#[test]
fn blockquote_line_reveal_reflows_only_across_lines() {
    // 「p\n\n> a\n> b\n」：引用块 bytes [3,11)，行 1 [3,7)、行 2 [7,11)
    let (mut a, _) = app_math("p\n\n> a\n> b\n", false);
    a.ensure_layouts_all();
    let (id, kind, range) = a.editor.session().blocks()[1].clone();
    assert_eq!(a.cache.layouts[&id].layout_text(), "  a\n  b");
    // 块内跨行移动（0 → 9，行 2 内容）：逐行激活变化 → 键失配 → 重排
    assert!(a.editor.set_caret(9));
    a.ensure_layouts_all();
    assert_eq!(a.cache.layouts[&id].layout_text(), "  a\n> b");
    // 同一行内移动（9 → 10）：激活行不变 → 键命中，布局不回退重排
    assert!(a.editor.set_caret(10));
    a.ensure_layouts_all();
    assert_eq!(
        a.cache.layouts[&id].layout_text(),
        "  a\n> b",
        "行内移动不得改变引用块布局"
    );
    assert_eq!(a.cache.layouts[&id].key.reveal, {
        let l = &a.cache.layouts[&id].value;
        l.reveal_state(&kind, range, 10)
    });
}

#[test]
fn thematic_break_elides_and_keeps_line_height() {
    // 「p\n\n---\n」：分隔线块 bytes [3,7)，初始 caret=0 在块外
    let (mut a, _) = app_math("p\n\n---\n", false);
    a.ensure_layouts_all();
    let (id, kind, _) = a.editor.session().blocks()[1].clone();
    assert_eq!(kind, BlockKind::ThematicBreak);
    assert_eq!(a.cache.layouts[&id].layout_text(), "");
    // 消隐后仍占一行高，块高不塌缩
    assert_eq!(
        a.block_height(geom(&a, id)),
        BASE_LINE_HEIGHT * a.input_state.scale
    );
    // 进入块：显源码，块高不变
    assert!(a.editor.set_caret(5));
    a.ensure_layouts_all();
    assert_eq!(a.cache.layouts[&id].layout_text(), "---");
    assert_eq!(
        a.block_height(geom(&a, id)),
        BASE_LINE_HEIGHT * a.input_state.scale
    );
}

/// 默认收起：空 expanded_details 集合，Details 块不在其中 → block_height 钳到单行。
#[test]
fn details_block_default_collapsed() {
    let store = MemStore::default();
    let mut a = app(DETAILS_DOC, None, false, &store);
    a.refresh_index();
    assert!(
        a.ui_state.expanded_details.is_empty(),
        "空集 = 全收起（details 块都不在展开集里）"
    );
    let id = block_id_of(&a, |k| matches!(k, BlockKind::Details { .. }));
    a.ensure_layouts_all();
    let g = a.index.blocks.iter().find(|g| g.id == id).unwrap();
    let line_h = a.engine.doc_line_height();
    assert!(
        (a.block_height(g) - line_h).abs() < 0.5,
        "折叠态高度应 ≈ 单行（{line_h:.1}），实际: {}",
        a.block_height(g)
    );
}

/// open_default=true 的 Details 块自动展开（sync_expanded_details 预塞进集合）。
#[test]
fn details_open_default_auto_expands() {
    let store = MemStore::default();
    let mut a = app(DETAILS_OPEN_DOC, None, false, &store);
    a.refresh_index();
    let id = block_id_of(&a, |k| {
        matches!(
            k,
            BlockKind::Details {
                open_default: true,
                ..
            }
        )
    });
    assert!(
        a.ui_state.expanded_details.contains(&id),
        "open_default=true 的块应自动加入 expanded_details"
    );
    a.ensure_layouts_all();
    let g = a.index.blocks.iter().find(|g| g.id == id).unwrap();
    // B3：头块只含 summary 一行，正文是独立子块——整组高度应明显大于单行。
    assert!(
        a.block_height(g) + a.details_children_height(g) > a.engine.doc_line_height() * 1.5,
        "展开态整组高度应明显大于单行"
    );
}

/// 点击 Details 块切换折叠——收起时整块点击展开；展开时点击 summary 行收起。
#[test]
fn details_click_toggles_expand() {
    let store = MemStore::default();
    let mut a = app(DETAILS_DOC, None, false, &store);
    let before = a.editor.session().serialize();
    a.refresh_index();
    let id = block_id_of(&a, |k| matches!(k, BlockKind::Details { .. }));

    // 初始态：全收起
    assert!(!a.ui_state.expanded_details.contains(&id));

    // 点击 Details 块区域 → 展开
    a.ensure_layouts_all();
    let g = a.index.blocks.iter().find(|g| g.id == id).unwrap();
    let scale = a.input_state.scale;
    let vp = a.viewport();
    let click_y = (vp.y + g.top - a.window_state.scroll_y + 5.0 * scale) as f64;
    a.input_state.mouse = PhysicalPosition::new((a.content_x() + 10.0) as f64, click_y);
    a.on_left_click();
    assert!(a.ui_state.expanded_details.contains(&id), "点击后应展开");
    let mid = a.editor.session().serialize();
    assert_eq!(mid, before, "展开不改文档字节");

    // 再点击 summary 行 → 收起
    a.ensure_layouts_all();
    let g2 = a.index.blocks.iter().find(|g| g.id == id).unwrap();
    let click_y2 = (vp.y + g2.top - a.window_state.scroll_y + 2.0 * scale) as f64;
    a.input_state.mouse = PhysicalPosition::new((a.content_x() + 10.0) as f64, click_y2);
    a.on_left_click();
    assert!(!a.ui_state.expanded_details.contains(&id), "再次点击应收起");
    let after = a.editor.session().serialize();
    assert_eq!(after, before, "收起不改文档字节");
}

/// 非 100% DPI：summary 行点击仍能收起（y/top 是物理像素，行高须乘 scale）。
#[test]
fn details_click_collapses_at_non_unit_dpi() {
    let store = MemStore::default();
    let mut a = app(DETAILS_DOC, None, false, &store);
    a.input_state.set_dpi(1.25);
    a.refresh_index();
    let id = block_id_of(&a, |k| matches!(k, BlockKind::Details { .. }));
    a.ensure_layouts_all();
    a.recompute_geometry();
    let scale = a.input_state.scale;
    let vp = a.viewport();
    let g = a.index.blocks.iter().find(|g| g.id == id).unwrap();
    a.input_state.mouse = PhysicalPosition::new(
        (a.content_x() + 10.0) as f64,
        (vp.y + g.top - a.window_state.scroll_y + 5.0 * scale) as f64,
    );
    a.on_left_click();
    assert!(a.ui_state.expanded_details.contains(&id), "点击后应展开");
    a.ensure_layouts_all();
    let g2 = a.index.blocks.iter().find(|g| g.id == id).unwrap();
    a.input_state.mouse = PhysicalPosition::new(
        (a.content_x() + 10.0) as f64,
        (vp.y + g2.top - a.window_state.scroll_y + 2.0 * scale) as f64,
    );
    a.on_left_click();
    assert!(
        !a.ui_state.expanded_details.contains(&id),
        "1.25 DPI 下 summary 行点击应收起"
    );
}

/// B3：块级 details 展开后，内部标题/列表/代码/表格/段落各自成为原生块并排版。
#[test]
fn details_expanded_children_use_native_layouts() {
    let store = MemStore::default();
    let doc = "前\n\n<details open>\n<summary>S</summary>\n## 标题\n\n- 项\n\n```rust\nfn x() {}\n```\n\n| a |\n| --- |\n| 1 |\n\n段落\n</details>\n\n后\n";
    let mut a = app(doc, None, false, &store);
    a.refresh_index();
    a.ensure_layouts_all();
    a.ensure_tables();
    let header = block_id_of(&a, |k| matches!(k, BlockKind::Details { .. }));
    let kinds: Vec<BlockKind> = a
        .index
        .blocks
        .iter()
        .filter(|g| g.container.details_depth == 1)
        .map(|g| g.kind.clone())
        .collect();
    assert!(kinds.iter().any(|k| matches!(k, BlockKind::AtxHeading(2))));
    assert!(kinds
        .iter()
        .any(|k| matches!(k, BlockKind::ListItem { .. })));
    assert!(kinds.iter().any(|k| matches!(k, BlockKind::FencedCode(_))));
    assert!(kinds.contains(&BlockKind::Table));
    assert!(kinds.contains(&BlockKind::Paragraph));
    // 标题走标题字号/间距：块高大于正文行高。
    let hg = a
        .index
        .blocks
        .iter()
        .find(|g| matches!(g.kind, BlockKind::AtxHeading(2)))
        .unwrap();
    assert!(
        a.block_height(hg) > a.engine.doc_line_height(),
        "子块标题应使用标题字号层级"
    );
    // 代码块按代码卡排版（等宽文本进入布局）。
    let cg = a
        .index
        .blocks
        .iter()
        .find(|g| matches!(g.kind, BlockKind::FencedCode(_)))
        .unwrap();
    let ctext = a.cache.layouts[&cg.id].value.layout_text().to_string();
    assert!(ctext.contains("fn x() {}"), "代码子块应原生排版：{ctext:?}");
    // 表格走 B2 网格。
    let tg = a
        .index
        .blocks
        .iter()
        .find(|g| g.kind == BlockKind::Table)
        .unwrap();
    assert!(a.table_shows_grid(tg.id, &tg.range));
    assert!(a.cache.tables[&tg.id].value.rows.len() >= 2);
    // 卡片高度覆盖整组。
    let hdr = a.index.blocks.iter().find(|g| g.id == header).unwrap();
    assert!(
        a.details_children_height(hdr) > a.engine.doc_line_height() * 3.0,
        "卡片应覆盖全部子块高度"
    );
}

/// B3：折叠态子块高度为 0（整组不占排版空间），展开后再获得真实高度。
#[test]
fn details_collapsed_child_blocks_have_zero_height() {
    let store = MemStore::default();
    let doc = "p\n\n<details>\n<summary>S</summary>\n## t\n\n- a\n\n尾段\n</details>\n\nq\n";
    let mut a = app(doc, None, false, &store);
    a.refresh_index();
    a.ensure_layouts_all();
    a.recompute_geometry();
    let header = block_id_of(&a, |k| matches!(k, BlockKind::Details { .. }));
    for g in a
        .index
        .blocks
        .iter()
        .filter(|g| g.container.details_depth == 1)
    {
        assert_eq!(a.block_height(g), 0.0, "折叠态子块应为 0 高：{:?}", g.kind);
    }
    let hdr = a.index.blocks.iter().find(|g| g.id == header).unwrap();
    assert_eq!(a.details_children_height(hdr), 0.0);
    // 紧跟其后的顶层块从 summary 行底部开始（不存在被隐藏内容的空白）。
    let next = a
        .index
        .blocks
        .iter()
        .find(|g| g.range.start > hdr.range.start && g.container.details_depth == 0)
        .unwrap();
    assert!(next.top > hdr.top, "后续块仍在 summary 之后");
    // 展开后子块有真实高度。
    a.toggle_details(header);
    a.ensure_layouts_all();
    a.recompute_geometry();
    let any = a
        .index
        .blocks
        .iter()
        .filter(|g| g.container.details_depth == 1)
        .any(|g| a.block_height(g) > 0.0);
    assert!(any, "展开后子块应恢复真实高度");
}

#[test]
fn details_active_summary_recollapse_restores_geometry_and_tail_anchor() {
    let store = MemStore::default();
    let doc = "# Details 验收\n\n## 1. 默认收起\n\n<details>\n<summary>点我展开（默认收起）</summary>\n\n折叠正文第一段。\n\n### 折叠内三级标题\n\n- 折叠内列表项\n\n</details>\n\n## 2. 后续标题\n\n尾部锚点。\n";
    let mut a = app(doc, None, false, &store);
    a.refresh_index();
    a.ensure_layouts_all();
    a.recompute_geometry();
    let details_id = block_id_of(&a, |k| matches!(k, BlockKind::Details { .. }));
    let next_at = doc.find("2. 后续标题").unwrap();
    let tail_at = doc.find("尾部锚点").unwrap();
    let next_id = a
        .index
        .blocks
        .iter()
        .find(|g| g.range.start <= next_at && next_at <= g.range.end)
        .unwrap()
        .id;
    let tail_id = a
        .index
        .blocks
        .iter()
        .find(|g| g.range.start <= tail_at && tail_at <= g.range.end)
        .unwrap()
        .id;
    let snapshot = |a: &App| {
        let header = a.index.blocks.iter().find(|g| g.id == details_id).unwrap();
        let next = a.index.blocks.iter().find(|g| g.id == next_id).unwrap();
        let tail = a.index.blocks.iter().find(|g| g.id == tail_id).unwrap();
        (header.top, header.height, next.top, tail.top)
    };
    let collapsed = snapshot(&a);
    let logical_line_h = a.engine.doc_line_height();
    let scale = a.input_state.scale;
    let line_h = logical_line_h * scale;
    assert!(
        (collapsed.1 - line_h).abs() < 0.6,
        "初始折叠只保留 summary 行高"
    );
    let header = a.index.blocks.iter().find(|g| g.id == details_id).unwrap();
    let next = a.index.blocks.iter().find(|g| g.id == next_id).unwrap();
    // 源码为 </details>\n\n## 后续标题：折叠后仅保留闭标签后的一个真空行，
    // 隐藏正文的行数不得再以 blank_lines 形式进入后续标题的 top。
    let expected_gap = (gap_after(&header.kind).max(gap_before(&next.kind))
        + crate::tokens::doc::BLANK_LINE_LINES * logical_line_h)
        * scale;
    assert!(
        (collapsed.2 - (collapsed.0 + collapsed.1 + expected_gap)).abs() < 0.01,
        "后续标题应紧跟 summary 与正常块间距：top={} expected={}",
        collapsed.2,
        collapsed.0 + collapsed.1 + expected_gap
    );

    // 模拟右键菜单落光标：激活 summary 不得恢复标签行或隐藏正文高度。
    a.editor.set_caret(doc.find("点我展开").unwrap());
    a.ensure_layouts_all();
    a.recompute_geometry();
    assert_eq!(snapshot(&a), collapsed, "summary 激活前后折叠几何必须一致");
    let header_text = a.cache.layouts[&details_id].value.layout_text().to_string();
    assert!(
        header_text.starts_with("点我展开"),
        "折叠态应只画 summary：{header_text:?}"
    );
    assert!(!header_text.contains("<details") && !header_text.contains("<summary"));
    assert!(a
        .index
        .blocks
        .iter()
        .filter(|g| g.container.details_depth > 0)
        .all(|g| a.block_height(g) == 0.0));

    a.toggle_details(details_id);
    a.ensure_layouts_all();
    a.recompute_geometry();
    let expanded = snapshot(&a);
    assert!(expanded.2 > collapsed.2, "展开应把后续标题向下推");
    assert!(expanded.3 > collapsed.3, "展开应把尾部锚点向下推");
    let nested_at = doc.find("折叠内三级标题").unwrap();
    let nested = a
        .index
        .blocks
        .iter()
        .find(|g| g.range.start <= nested_at && nested_at <= g.range.end)
        .expect("嵌套 Markdown 标题应拆成子块");
    assert!(a.block_height(nested) > 0.0);
    assert!(a.cache.layouts[&nested.id]
        .value
        .layout_text()
        .contains("折叠内三级标题"));

    // 光标在即将隐藏的正文中再折叠：应吸附到 summary，且后续块回到初始位置。
    a.editor.set_caret(doc.find("折叠正文第一段").unwrap());
    a.toggle_details(details_id);
    a.ensure_layouts_all();
    a.recompute_geometry();
    assert_eq!(
        snapshot(&a),
        collapsed,
        "再折叠必须完整恢复后续块与尾部锚点"
    );
    assert!(a
        .index
        .blocks
        .iter()
        .filter(|g| g.container.details_depth > 0)
        .all(|g| a.block_height(g) == 0.0));
    let header_text = a.cache.layouts[&details_id].value.layout_text().to_string();
    assert!(!header_text.contains("折叠正文") && !header_text.contains("<details"));

    let vp = a.viewport();
    let cx = a.content_x() as i32;
    let cw = a.content_width() as u32;
    let summary_y = (vp.y + collapsed.0) as i32;
    let next_y = (vp.y + collapsed.2) as i32;
    let next_h = a
        .index
        .blocks
        .iter()
        .find(|g| g.id == next_id)
        .unwrap()
        .height;
    let frame = a.render();
    assert!(
        ink_pixels(frame, cx, summary_y, cw, line_h as u32) > 0,
        "再折叠后 summary 应可见"
    );
    assert!(
        ink_pixels(frame, cx, next_y, cw, next_h as u32) > 0,
        "后续标题应紧邻可见"
    );
}

/// B3：折叠态下上下移动光标跳过隐藏子块（落到 summary 或下一块）。
#[test]
fn collapsed_details_caret_skips_hidden_children() {
    let store = MemStore::default();
    let doc = "A\n\n<details>\n<summary>S</summary>\nB\n\nC\n</details>\n\nZ\n";
    let mut a = app(doc, None, false, &store);
    a.refresh_index();
    a.ensure_layouts_all();
    a.recompute_geometry();
    a.editor.set_caret(1);
    a.move_vertical(1, false);
    let first = a.editor.caret();
    assert!(
        first < doc.find("B").unwrap(),
        "第一次下移应落在 details 头块（summary），实际 {first}"
    );
    a.move_vertical(1, false);
    let second = a.editor.caret();
    assert!(
        second >= doc.find("Z").unwrap(),
        "第二次下移应跳过隐藏子块直达下一顶层块，实际 {second}"
    );
}

/// B3：行内 `<details>` 默认只显示 summary；summary 点击切换展开/收起。
#[test]
fn inline_details_collapses_body_and_summary_click_toggles() {
    let store = MemStore::default();
    let doc = "前 <details><summary>更多</summary>隐藏正文XYZ</details> 后\n";
    let mut a = app(doc, None, false, &store);
    a.refresh_index();
    a.ensure_layouts_all();
    let id = block_id_of(&a, |k| *k == BlockKind::Paragraph);
    let undo_before = a.editor.session().undo_depth();
    let collapsed = a.cache.layouts[&id].value.layout_text().to_string();
    assert!(
        collapsed.contains("更多"),
        "应收起态显示 summary：{collapsed:?}"
    );
    assert!(
        !collapsed.contains("隐藏正文"),
        "收起态正文必须隐藏（不出现 <details> 裸标签）：{collapsed:?}"
    );
    assert!(!collapsed.contains("summary"), "不应显示裸标签");
    let (g_top, g_start) = {
        let g = a.index.blocks.iter().find(|g| g.id == id).unwrap();
        (g.top, g.range.start)
    };
    let scale = a.input_state.scale;
    let vp = a.viewport();
    let abs = doc.find("更多").unwrap() + 1;
    let (cx, cy) = {
        let l = &a.cache.layouts[&id].value;
        let (x, y, _) = a.engine.caret_xy(l, abs - g_start);
        (x, y)
    };
    let _ = scale;
    a.input_state.mouse = PhysicalPosition::new(
        (a.content_x() + cx) as f64,
        (vp.y + g_top - a.window_state.scroll_y + cy) as f64,
    );
    a.on_left_click();
    assert_eq!(
        a.ui_state.expanded_inline_details.len(),
        1,
        "点击 summary 应展开行内 details"
    );
    a.ensure_layouts_all();
    let expanded = a.cache.layouts[&id].value.layout_text().to_string();
    assert!(
        expanded.contains("隐藏正文"),
        "展开后正文应以行内文本流继续：{expanded:?}"
    );
    assert_eq!(a.editor.session().serialize(), doc, "折叠/展开不改文档字节");
    assert_eq!(
        a.editor.session().undo_depth(),
        undo_before,
        "行内折叠/展开不得产生撤销步"
    );
    // 再点 summary → 收起。
    let g_top = a
        .index
        .blocks
        .iter()
        .find(|g| g.id == id)
        .map(|g| g.top)
        .unwrap();
    a.input_state.mouse = PhysicalPosition::new(
        (a.content_x() + cx) as f64,
        (vp.y + g_top - a.window_state.scroll_y + cy) as f64,
    );
    a.on_left_click();
    a.ensure_layouts_all();
    assert!(a.ui_state.expanded_inline_details.is_empty());
    let again = a.cache.layouts[&id].value.layout_text().to_string();
    assert!(!again.contains("隐藏正文"));
}

/// Task 8：引用容器内的 `<details>` 默认收起，点击 summary 展开/收起。
/// 折叠是纯视图态：文档字节与撤销栈不变；块高与后续块顶偏移随折叠更新。
#[test]
fn quote_container_details_collapse_toggles_by_summary_click() {
    let store = MemStore::default();
    let doc = "> <details>\n> <summary>更多</summary>\n> 隐藏正文XYZ\n> </details>\n\n尾\n";
    let mut a = app(doc, None, false, &store);
    a.refresh_index();
    // 光标放到容器外：容器各行按非激活态渲染。
    a.editor.set_caret(doc.find("尾").unwrap());
    a.ensure_layouts_all();
    a.recompute_geometry();
    let id = block_id_of(&a, |k| *k == BlockKind::BlockQuote);
    let next_id = block_id_of(&a, |k| *k == BlockKind::Paragraph);
    let undo_before = a.editor.session().undo_depth();

    let collapsed = a.cache.layouts[&id].value.layout_text().to_string();
    assert!(
        collapsed.contains("更多"),
        "收起态应显示 summary：{collapsed:?}"
    );
    assert!(
        !collapsed.contains("隐藏正文"),
        "收起态正文必须隐藏：{collapsed:?}"
    );
    assert!(
        !collapsed.to_ascii_lowercase().contains("details")
            && !collapsed.to_ascii_lowercase().contains("summary"),
        "不应露出裸标签：{collapsed:?}"
    );

    let geom = |a: &App, id: BlockId| {
        a.index
            .blocks
            .iter()
            .find(|g| g.id == id)
            .map(|g| (g.top, a.block_height(g), g.range.start))
            .expect("块应存在")
    };
    let (top_c, h_c, start_c) = geom(&a, id);
    let (top_n, _, _) = geom(&a, next_id);
    let (cx, cy) = {
        let l = &a.cache.layouts[&id].value;
        let (x, y, _) = a.engine.caret_xy(l, doc.find("更多").unwrap() - start_c);
        (x, y)
    };
    let vp = a.viewport();
    a.input_state.mouse = PhysicalPosition::new(
        (a.content_x() + cx) as f64,
        (vp.y + top_c - a.window_state.scroll_y + cy) as f64,
    );
    a.on_left_click();
    assert_eq!(
        a.ui_state.expanded_inline_details.len(),
        1,
        "点击容器内 summary 应展开 details"
    );
    a.ensure_layouts_all();
    a.recompute_geometry();
    let expanded = a.cache.layouts[&id].value.layout_text().to_string();
    assert!(
        expanded.contains("隐藏正文"),
        "展开后正文应可见：{expanded:?}"
    );
    let (top_e, h_e, _) = geom(&a, id);
    let (top_n_e, _, _) = geom(&a, next_id);
    assert_eq!(top_e, top_c, "块顶不应因展开而移动");
    assert!(h_e > h_c, "展开后块高应增大：{h_c} → {h_e}");
    assert!(top_n_e > top_n, "后续块应随展开下移：{top_n} → {top_n_e}");

    // 再点 summary → 收起。
    a.input_state.mouse = PhysicalPosition::new(
        (a.content_x() + cx) as f64,
        (vp.y + top_e - a.window_state.scroll_y + cy) as f64,
    );
    a.on_left_click();
    a.ensure_layouts_all();
    assert!(a.ui_state.expanded_inline_details.is_empty());
    let again = a.cache.layouts[&id].value.layout_text().to_string();
    assert!(!again.contains("隐藏正文"), "收起后正文再次隐藏");
    assert_eq!(a.editor.session().serialize(), doc, "折叠/展开不改文档字节");
    assert_eq!(
        a.editor.session().undo_depth(),
        undo_before,
        "容器内折叠/展开不得产生撤销步"
    );
}

/// Task 8：列表项容器内的 `<details>` 同样收起正文并可点击 summary 展开。
#[test]
fn list_item_container_details_collapse_toggles() {
    let store = MemStore::default();
    let doc = "- 项\n  <details>\n  <summary>更多</summary>\n  隐藏正文XYZ\n  </details>\n\n尾\n";
    let mut a = app(doc, None, false, &store);
    a.refresh_index();
    a.editor.set_caret(doc.find("尾").unwrap());
    a.ensure_layouts_all();
    let id = block_id_of(&a, |k| matches!(k, BlockKind::ListItem { .. }));
    let collapsed = a.cache.layouts[&id].value.layout_text().to_string();
    assert!(
        collapsed.contains("更多"),
        "收起态应显示 summary：{collapsed:?}"
    );
    assert!(
        !collapsed.contains("隐藏正文"),
        "收起态正文必须隐藏：{collapsed:?}"
    );
    // 直接写入折叠集合（点击路径已由引用容器用例覆盖），验证列表项同口径生效。
    let start = a
        .index
        .blocks
        .iter()
        .find(|g| g.id == id)
        .map(|g| g.range.start)
        .unwrap();
    let key = doc.find("<details>").unwrap() - start;
    a.ui_state.expanded_inline_details.insert((id, key));
    a.ensure_layouts_all();
    let expanded = a.cache.layouts[&id].value.layout_text().to_string();
    assert!(
        expanded.contains("隐藏正文"),
        "展开后正文应可见：{expanded:?}"
    );
    assert_eq!(a.editor.session().serialize(), doc, "折叠不改文档字节");
}

/// Task 8：光标进入容器内 details 的隐藏正文时自动展开；不同块的折叠状态互不串扰。
#[test]
fn caret_inside_container_details_body_auto_expands_without_cross_block_leak() {
    let store = MemStore::default();
    let doc = "> <details>\n> <summary>A</summary>\n> 甲正文\n> </details>\n\n> <details>\n> <summary>B</summary>\n> 乙正文\n> </details>\n";
    let mut a = app(doc, None, false, &store);
    a.refresh_index();
    a.ensure_layouts_all();
    let ids: Vec<BlockId> = a
        .index
        .blocks
        .iter()
        .filter(|g| matches!(g.kind, BlockKind::BlockQuote))
        .map(|g| g.id)
        .collect();
    assert_eq!(ids.len(), 2, "两段引用应各自成块");
    // 光标落在正文首个字符（CJK 多字节，偏移必须是字符边界）。
    a.editor.set_caret(doc.find("甲正文").unwrap());
    a.sync_details_at_caret();
    assert_eq!(a.ui_state.expanded_inline_details.len(), 1);
    assert!(
        a.ui_state
            .expanded_inline_details
            .iter()
            .all(|(id, _)| *id == ids[0]),
        "只应展开光标所属块"
    );
    a.ensure_layouts_all();
    assert!(
        a.cache.layouts[&ids[0]]
            .value
            .layout_text()
            .contains("甲正文"),
        "光标进入后正文应展开"
    );
    assert!(
        !a.cache.layouts[&ids[1]]
            .value
            .layout_text()
            .contains("乙正文"),
        "另一块的 details 不应被连带展开"
    );
    assert_eq!(a.editor.session().serialize(), doc, "自动展开不写文档");
}

/// details 闭标签行是纯 chrome（不属于任何块）：光标停在那里时吸附到前一块末尾。
#[test]
fn caret_in_details_close_line_snaps_to_previous_block() {
    let store = MemStore::default();
    let doc = "<details>\n<summary>S</summary>\n正文\n</details>\n\n尾\n";
    let mut a = app(doc, None, false, &store);
    a.refresh_index();
    a.ensure_layouts_all();
    let close = doc.find("</details>").unwrap();
    a.editor.set_caret(close + 2);
    a.sync_details_at_caret();
    let caret = a.editor.caret();
    assert!(caret < close, "光标不应停在闭标签行，实际 {caret}");
    assert_eq!(a.editor.session().serialize(), doc, "吸附不改文档字节");
}

/// B3：搜索命中位于折叠 details 内时自动展开（块级与行内两种形态都覆盖）。
#[test]
fn search_match_inside_collapsed_details_auto_expands() {
    let store = MemStore::default();
    let doc =
        "p\n\n<details>\n<summary>S</summary>\nneedle_block\n</details>\n\n前 <details><summary>M</summary>needle_inline</details> 后\n";
    let mut a = app(doc, None, false, &store);
    a.refresh_index();
    a.ensure_layouts_all();
    let id = block_id_of(&a, |k| matches!(k, BlockKind::Details { .. }));
    assert!(!a.ui_state.expanded_details.contains(&id));
    a.open_search();
    a.search_input("needle_block");
    assert!(
        a.ui_state.expanded_details.contains(&id),
        "块级命中应自动展开"
    );
    // 继续搜行内正文命中。
    a.close_search();
    a.open_search();
    a.search_input("needle_inline");
    assert_eq!(
        a.ui_state.expanded_inline_details.len(),
        1,
        "行内 details 命中应自动展开"
    );
    assert_eq!(a.editor.caret(), doc.find("needle_inline").unwrap());
    a.ensure_layouts_all();
    assert_eq!(a.editor.session().serialize(), doc, "搜索展开不写文档");
}

/// 折叠态与展开态保存的字节完全一致——折叠是纯 UI 视图态。
/// 高度断言留给 details_block_default_collapsed / details_open_default_auto_expands 两个测试覆盖。
#[test]
fn details_fold_is_view_state_not_byte_change() {
    let store = MemStore::default();
    let mut a = app(DETAILS_DOC, None, false, &store);
    a.refresh_index();
    let id = block_id_of(&a, |k| matches!(k, BlockKind::Details { .. }));

    let collapsed_bytes = a.editor.session().serialize();
    let undo_before = a.editor.session().undo_depth();
    a.toggle_details(id);
    let expanded_bytes = a.editor.session().serialize();
    assert_eq!(
        expanded_bytes, collapsed_bytes,
        "展开态字节应与折叠态一致（视图态不改文档）"
    );
    assert_eq!(
        a.editor.session().undo_depth(),
        undo_before,
        "折叠/展开不得产生撤销步"
    );

    // 再 toggle 回去还是一样
    a.toggle_details(id);
    let after_toggle_back = a.editor.session().serialize();
    assert_eq!(after_toggle_back, collapsed_bytes, "再切回折叠态字节不变");
}

// ---- Wave 7 修复：行内 HTML 装饰 / sub-sup / img / 表格 br / details 像素回归 ----

#[test]
fn inline_html_kbd_mark_abbr_pixels_visible() {
    let store = MemStore::default();
    let text = "段落 <kbd>Ctrl</kbd> 与 <mark>高亮</mark> 与 <abbr title=\"x\">API</abbr>。\n";
    let mut a = app(text, None, false, &store);
    a.editor.set_caret(text.len() - 1); // 块内激活会显源码，这里让节点保持非激活
    a.ensure_layouts_all();
    a.recompute_geometry();
    let id = a.index.blocks[0].id;
    let text = a.editor.session().serialize();
    let (ck, cm, ca) = (
        rel_offset(&a, id, text.find("Ctrl").unwrap()),
        rel_offset(&a, id, text.find("高亮").unwrap()),
        rel_offset(&a, id, text.find("API").unwrap()),
    );
    let vp = a.viewport();
    let g = a.index.blocks.iter().find(|g| g.id == id).unwrap();
    let sy = (vp.y + g.top - a.window_state.scroll_y) as i32;
    let cx = a.content_x() as i32;
    let scale = a.input_state.scale;
    let _ = scale;
    let l = &a.cache.layouts[&id];
    let k_rect = a.engine.selection_rects(l, ck, ck + "Ctrl".len())[0];
    let m_rect = a.engine.selection_rects(l, cm, cm + "高亮".len())[0];
    let a_rect = a.engine.selection_rects(l, ca, ca + "API".len())[0];
    let kx = cx + (k_rect.0 - 1.0) as i32;
    let ky = sy + (k_rect.1 + k_rect.3 / 2.0) as i32;
    let mx = cx + (m_rect.0 + m_rect.2 - 2.0) as i32;
    let my = sy + (m_rect.1 + 2.0) as i32;
    let bg = a.input_state.theme.bg;
    let f = a.render();
    assert_eq!(
        f.get(kx as u32, ky as u32),
        [160, 160, 160, 255],
        "kbd 左侧描边像素应为灰描边（实际: {:?}）",
        f.get(kx as u32, ky as u32)
    );
    let mark = f.get(mx as u32, my as u32);
    assert!(
        mark[0] > 245 && mark[1] > 240 && mark[2] < 225,
        "mark 底色应为黄色系，实际: {mark:?}"
    );
    // abbr 下划线为点线：同一行内存在间隔暗像素段（非连续实线）
    let uy = sy + (a_rect.1 + a_rect.3) as i32 - 2;
    let ux = cx + a_rect.0 as i32;
    assert_dotted_row(f, ux, uy, a_rect.2 as i32, bg);
}

/// 深色回归：`<kbd>` 必须用暗底 + 中灰描边。曾在深色下渲染成近白块，
/// 浅色字（fg）坐在 [245,245,245] 上几乎不可见（截图事故）。
#[test]
fn dark_kbd_renders_on_dark_surface() {
    let store = MemStore::default();
    let text = "段落 <kbd>Ctrl</kbd> 结束\n";
    let mut a = app(text, None, false, &store);
    a.set_theme_mode(ThemeMode::Dark);
    a.editor.set_caret(text.len() - 1);
    a.ensure_layouts_all();
    a.recompute_geometry();
    let theme = a.input_state.theme;
    assert!(theme.dark, "本用例前提是深色主题");
    let id = a.index.blocks[0].id;
    let serialized = a.editor.session().serialize();
    let ck = rel_offset(&a, id, serialized.find("Ctrl").unwrap());
    let vp = a.viewport();
    let g = a.index.blocks.iter().find(|g| g.id == id).unwrap();
    let sy = (vp.y + g.top - a.window_state.scroll_y) as i32;
    let cx = a.content_x() as i32;
    let l = &a.cache.layouts[&id];
    let k_rect = a.engine.selection_rects(l, ck, ck + "Ctrl".len())[0];
    let kx = (cx + (k_rect.0 - 1.0) as i32).max(0) as u32;
    let ky = sy + (k_rect.1 + k_rect.3 / 2.0) as i32;
    let f = a.render();
    let border = [
        theme.kbd_border[0],
        theme.kbd_border[1],
        theme.kbd_border[2],
        255,
    ];
    assert_eq!(
        f.get(kx, ky as u32),
        border,
        "深色下 kbd 左缘应为主题描边色（非浅色硬编码）"
    );
    // 内缩 1px 后应是暗底：亮度必须明显低于正文前景，否则就是眩光块
    let inner = f.get(kx + 1, ky as u32);
    assert_eq!(inner, theme.kbd_bg, "深色下 kbd 内部应为主题暗底");
    assert!(
        inner[0] < 120 && inner[1] < 120 && inner[2] < 120,
        "深色 kbd 底色亮度必须低（实际 {inner:?}）"
    );
}

#[test]
fn inline_html_sub_sup_pixels_smaller_and_shifted() {
    let store = MemStore::default();
    let text = "H<sub>2</sub>O<sup>3</sup>\n";
    let mut a = app(text, None, false, &store);
    a.editor.set_caret(text.len() - 1);
    a.ensure_layouts_all();
    let id = a.index.blocks[0].id;
    let (px, w, _h) = {
        let l = &a.cache.layouts[&id];
        let glyphs: Vec<(usize, f32, f32, f32)> = l
            .buffer
            .layout_runs()
            .flat_map(|r| r.glyphs)
            .map(|g| (g.start, g.x, g.w, g.font_size))
            .collect();
        let body = glyphs.iter().find(|g| g.0 == 0).copied().unwrap();
        let sub = glyphs.iter().find(|g| g.0 == 1).copied().unwrap();
        let sup = glyphs.iter().find(|g| g.0 == 3).copied().unwrap();
        assert!(sub.3 < body.3, "sub 字号应小于正文: {sub:?} vs {body:?}");
        assert!(sup.3 < body.3, "sup 字号应小于正文: {sup:?} vs {body:?}");
        a.engine.rasterize(l, [0, 0, 0])
    };
    let bbox = |gx: f32, gw: f32| -> (usize, usize, usize) {
        let (x0, x1) = (gx.floor().max(0.0) as usize, (gx + gw).ceil() as usize);
        let (mut lo, mut hi, mut n) = (usize::MAX, 0usize, 0usize);
        for y in 0.._h as usize {
            for x in x0..x1.min(w as usize) {
                if px[(y * w as usize + x) * 4 + 3] > 32 {
                    lo = lo.min(y);
                    hi = hi.max(y);
                    n += 1;
                }
            }
        }
        assert!(n > 0, "字形区域应有墨迹");
        (lo, hi, n)
    };
    let body_g = {
        let l = &a.cache.layouts[&id];
        let gs: Vec<_> = l.buffer.layout_runs().flat_map(|r| r.glyphs).collect();
        (gs[0].x, gs[0].w, gs[1].x, gs[1].w, gs[3].x, gs[3].w)
    };
    let (b_lo, b_hi, _) = bbox(body_g.0, body_g.1);
    let (sub_lo, sub_hi, _) = bbox(body_g.2, body_g.3);
    let (sup_lo, sup_hi, _) = bbox(body_g.4, body_g.5);
    assert!(sub_hi - sub_lo < b_hi - b_lo, "sub 字形高度应小于正文");
    assert!(sup_hi - sup_lo < b_hi - b_lo, "sup 字形高度应小于正文");
    assert!(sub_hi > b_hi, "sub 应下沉（底部低于正文基线）");
    assert!(sup_hi < b_hi, "sup 应上浮（底部高于正文基线）");
}

#[test]
fn html_img_missing_or_remote_placeholder_and_alt_visible() {
    let store = MemStore::default();
    let text = "图：<img src=\"https://example.com/r.png\" alt=\"远程\"> 后\n";
    let mut a = app(text, None, false, &store);
    a.editor.set_caret(text.len() - 1);
    a.ensure_layouts_all();
    a.recompute_geometry();
    let id = a.index.blocks[0].id;
    let l = &a.cache.layouts[&id];
    assert!(
        l.layout_text().contains("[远程]"),
        "远程/缺失 img 应显 alt 占位文本；layout_text: {:?}",
        l.layout_text()
    );
    let rect = l
        .decorations_under
        .iter()
        .find(|(_, _, k)| matches!(k, crate::view::UnderKind::ImagePlaceholder))
        .map(|&(s, e, _)| a.engine.selection_rects(l, s, e)[0])
        .expect("应登记图片占位框");
    let vp = a.viewport();
    let g = a.index.blocks.iter().find(|g| g.id == id).unwrap();
    let sy = (vp.y + g.top - a.window_state.scroll_y) as i32;
    let cx = a.content_x() as i32;
    let dx = cx + (rect.0 - 1.0) as i32;
    let dy = sy + (rect.1 - 1.0) as i32;
    let dw = (rect.2 + 2.0) as u32;
    let dh = (rect.3 + 2.0) as u32;
    let px = dx + dw as i32 - 3;
    let py = dy + dh as i32 / 2;
    let placeholder_fill = a.input_state.theme.code_bg;
    let f = a.render();
    assert!(
        any_pixel(
            f,
            dx + 1,
            dy + 1,
            dw.saturating_sub(2),
            dh.saturating_sub(2),
            |p| p == placeholder_fill
        ),
        "占位框内应有主题代码底色（浅色 = 浅灰），实际: {:?}",
        f.get(px as u32, py as u32)
    );
}

#[test]
fn html_img_local_inline_uses_image_path() {
    let text = "行内 <img src=\"logo.png\" alt=\"本地\"> 后\n";
    let (mut a, calls, _files) =
        app_with_images(text, Some("dir/doc.md"), &[("dir/logo.png", 20, 10)]);
    a.editor.set_caret(text.len() - 1);
    a.ensure_layouts_all();
    let id = a.index.blocks[0].id;
    let l = &a.cache.layouts[&id];
    assert!(
        calls.borrow().iter().any(|p| p.ends_with("dir/logo.png")),
        "行内 <img> 的本地 src 应交给 ImageLoader"
    );
    assert_eq!(l.imgs.len(), 1, "应有 1 个图片占位");
    assert!(
        l.imgs[0].width > 0 && l.imgs[0].height > 0,
        "图片占位宽高应 > 0"
    );
    assert!(
        !l.layout_text().contains("<img"),
        "激活/渲染态不应露出源码标签"
    );
}

#[test]
fn block_level_html_img_line_renders_placeholder_not_code_card() {
    let store = MemStore::default();
    let text = "<img src=\"veil-missing.png\" alt=\"缺失\">\n\n后\n";
    let mut a = app(text, None, false, &store);
    a.editor
        .set_caret(text.find('后').unwrap() + '后'.len_utf8()); // 光标在末段，img 块保持非激活渲染态
    a.refresh_index();
    let id = block_id_of(&a, |k| matches!(k, BlockKind::Paragraph));
    a.ensure_layouts_all();
    let l = &a.cache.layouts[&id];
    // 独立成行的缺失 `<img>`：占位提示 + 下方题注（alt），二者都在视图文本里。
    assert!(
        l.layout_text().contains(t(I18nKey::ImageAltFallback)) && l.layout_text().contains("缺失"),
        "块级 img 行应渲染占位 + 题注；layout_text: {:?}",
        l.layout_text()
    );
    assert!(
        l.decorations_under
            .iter()
            .any(|(_, _, k)| matches!(k, crate::view::UnderKind::ImagePlaceholder)),
        "块级 img 应登记占位框"
    );
}

#[test]
fn table_cell_br_renders_two_visual_lines() {
    let store = MemStore::default();
    let text = "前言\n\n| 列 |\n| --- |\n| 第一行<br>第二行 |\n";
    let mut a = app(text, None, false, &store);
    a.refresh_index();
    a.ensure_tables();
    let id = block_id_of(&a, |k| matches!(k, BlockKind::Table));
    let line_h = a.engine.doc_line_height() * a.input_state.scale;
    let tl = &a.cache.tables[&id].value;
    assert!(
        tl.rows[1].height > line_h * 1.5,
        "含 <br> 的表格行应有两行高，实际: {}（单行 {line_h}）",
        tl.rows[1].height
    );
    let cell = &tl.cells[tl.cols.len()]; // 表体第一行第一列
    let mut lo = usize::MAX;
    let mut hi = 0usize;
    for y in 0..cell.pixel_h as usize {
        for x in 0..cell.pixel_w as usize {
            if cell.pixels[(y * cell.pixel_w as usize + x) * 4 + 3] > 32 {
                lo = lo.min(y);
                hi = hi.max(y);
            }
        }
    }
    assert!(
        hi.saturating_sub(lo) as f32 > line_h,
        "单元格墨迹应跨两行（跨度 {}），单行高 {line_h}",
        hi.saturating_sub(lo)
    );
}

#[test]
fn details_collapsed_summary_and_expanded_body_are_painted() {
    let store = MemStore::default();
    let mut a = app(DETAILS_DOC, None, false, &store);
    a.editor.set_caret(DETAILS_DOC.len() - 1);
    a.refresh_index();
    let id = block_id_of(&a, |k| matches!(k, BlockKind::Details { .. }));
    a.ensure_layouts_all();
    a.recompute_geometry();
    let collapsed_h = {
        let g = a.index.blocks.iter().find(|g| g.id == id).unwrap();
        a.block_height(g)
    };
    let line_h = a.engine.doc_line_height() * a.input_state.scale;
    assert!(
        (collapsed_h - line_h).abs() < 0.6,
        "折叠高度应为一整行（{line_h}），实际 {collapsed_h}"
    );
    let layout_text = a.cache.layouts[&id].value.layout_text().to_string();
    assert!(
        layout_text.starts_with("安装步骤"),
        "折叠态首行应为 summary 文本，实际: {layout_text:?}"
    );
    let vp = a.viewport();
    let g = a.index.blocks.iter().find(|g| g.id == id).unwrap();
    let sy = (vp.y + g.top - a.window_state.scroll_y) as i32;
    let cx = a.content_x() as i32;
    let cw = a.content_width() as u32;
    let f = a.render();
    assert!(
        ink_pixels(f, cx - 2, sy, cw, collapsed_h as u32) > 0,
        "折叠条应画出 summary 文本"
    );
    // 展开：正文行必须可见，且整组（头块 + 子块）高度增长（B3：正文是独立子块）
    a.toggle_details(id);
    a.ensure_layouts_all();
    a.recompute_geometry();
    let expanded_h = {
        let g = a.index.blocks.iter().find(|g| g.id == id).unwrap();
        a.block_height(g) + a.details_children_height(g)
    };
    assert!(expanded_h > collapsed_h * 2.0, "展开后整组高度应明显增大");
    let body_id = a
        .index
        .blocks
        .iter()
        .find(|g| g.details_parent == Some(id))
        .map(|g| g.id)
        .expect("展开后正文应为 details 子块");
    let layout_text = a.cache.layouts[&body_id].value.layout_text().to_string();
    assert!(
        layout_text.contains("pip install veil") && layout_text.contains("然后运行 veil"),
        "展开后正文应作为可见文本渲染；layout_text: {layout_text:?}"
    );
    let f = a.render();
    let y0 = sy + collapsed_h as i32;
    assert!(
        ink_pixels(f, cx - 2, y0, cw, (expanded_h - collapsed_h) as u32) > 0,
        "展开后正文区域应有墨迹（不能被卡片底遮盖）"
    );
}

// ---- Wave 7 收尾：表格内行内装饰 / abbr 点线 / abbr 悬停 / 行内代码底色 ----

/// 表格单元格内 kbd / mark / abbr / pre 与正文同语义：描边、黄底、点线、代码底色。
#[test]
fn table_cell_inline_decor_pixels_visible() {
    use crate::view::UnderKind;
    let store = MemStore::default();
    let text = "| a | b | c | d |\n| --- | --- | --- | --- |\n\
                | <kbd>K</kbd> | <mark>M</mark> | <abbr title=\"tip\">AB</abbr> | <pre>P</pre> |\n\n尾\n";
    let mut a = app(text, None, false, &store);
    a.editor.set_caret(text.len()); // 光标移出表格 → 全部单元格走富文本网格
    a.refresh_index();
    a.ensure_tables();
    let id = block_id_of(&a, |k| matches!(k, BlockKind::Table));
    let (top, xs, ys, unders, overs) = {
        let top = a.index.blocks.iter().find(|g| g.id == id).unwrap().top;
        let tl = &a.cache.tables[&id].value;
        let mut xs = Vec::new();
        let mut ys = Vec::new();
        let mut unders = Vec::new();
        let mut overs = Vec::new();
        for i in 4..8 {
            let c = &tl.cells[i];
            xs.push(c.x);
            ys.push(c.y);
            unders.push(c.under.clone());
            overs.push(c.over.clone());
        }
        (top, xs, ys, unders, overs)
    };
    let vp = a.viewport();
    let sy = (vp.y + top - a.window_state.scroll_y) as i32;
    let cx = a.content_x() as i32;
    let code_bg = a.input_state.theme.code_bg;
    let bg = a.input_state.theme.bg;
    let f = a.render();
    // kbd：左缘描边像素（灰 160）
    let kbd = unders[0]
        .iter()
        .position(|r| matches!(r.4, UnderKind::KbdFrame))
        .expect("kbd 单元格应登记边框装饰");
    {
        let (rx, ry, _, rh, _) = unders[0][kbd];
        let dx = cx + (xs[0] + rx - 1.0) as i32;
        let dy = sy + (ys[0] + ry - 1.0) as i32;
        let px = f.get(dx as u32, (dy + rh as i32 / 2) as u32);
        assert_eq!(px, [160, 160, 160, 255], "表格 kbd 左侧描边应为灰描边");
    }
    // mark：底色矩形内出现黄色系像素
    let mark = unders[1]
        .iter()
        .position(|r| matches!(r.4, UnderKind::MarkBg))
        .expect("mark 单元格应登记底色");
    {
        let (rx, ry, rw, rh, _) = unders[1][mark];
        let dx = cx + (xs[1] + rx - 1.0) as i32;
        let dy = sy + (ys[1] + ry - 1.0) as i32;
        assert!(
            any_pixel(f, dx, dy, rw as u32, rh as u32, |p| p[0] > 245
                && p[1] > 240
                && p[2] < 225),
            "表格 mark 应有黄色底"
        );
    }
    // pre：代码底色（主题 code_bg）出现在装饰矩形内
    let pre = unders[3]
        .iter()
        .position(|r| matches!(r.4, UnderKind::InlineCodeBg))
        .expect("行内 pre 单元格应登记代码底色");
    {
        let (rx, ry, rw, rh, _) = unders[3][pre];
        let dx = cx + (xs[3] + rx - 1.0) as i32;
        let dy = sy + (ys[3] + ry - 1.0) as i32;
        assert!(
            any_pixel(f, dx, dy, rw as u32, rh as u32, |p| p == code_bg),
            "表格行内 pre 应有主题代码底色"
        );
    }
    // abbr：下划线为点线（同一条行内存在暗段间隔）
    let abbr = overs[2].first().expect("abbr 单元格应登记下划线");
    let oy = sy + (ys[2] + abbr.1 + abbr.3) as i32 - 2;
    let ox = cx + (xs[2] + abbr.0) as i32;
    assert_dotted_row(f, ox, oy, abbr.2 as i32, bg);
}

/// `<strong style="color:red">` 在正文与表格单元格里都必须渲染为红色内容（非源码原文）。
#[test]
fn inline_strong_style_color_pixels_paragraph_and_table() {
    let store = MemStore::default();
    let text = "正文 <strong style=\"color:red\">VISUAL</strong> 结束\n\n\
                | 标题 |\n| --- |\n| <strong style=\"color:red\">VISUAL</strong> |\n\n尾\n";
    let mut a = app(text, None, false, &store);
    a.editor.set_caret(text.len()); // 光标移出表格 / 段落，保持非激活渲染
    a.refresh_index();
    a.ensure_layouts_all();
    a.ensure_tables();
    a.recompute_geometry();
    let is_red = |p: [u8; 4]| p[3] > 0 && p[0] > 150 && p[1] < 110 && p[2] < 110;

    // 1) 正文段落：glyph 区域内出现红色像素，且 layout_text 不泄漏标签。
    let pid = block_id_of(&a, |k| matches!(k, BlockKind::Paragraph));
    let src = a.editor.session().serialize();
    let vpos = src.find("VISUAL").unwrap();
    let rel = rel_offset(&a, pid, vpos);
    let (rect, l_text) = {
        let l = &a.cache.layouts[&pid];
        (
            a.engine.selection_rects(l, rel, rel + "VISUAL".len())[0],
            l.layout_text().to_string(),
        )
    };
    assert!(
        !l_text.contains("<strong"),
        "正文不得显示源码标签: {l_text:?}"
    );
    let vp = a.viewport();
    let g = a.index.blocks.iter().find(|g| g.id == pid).unwrap();
    let sy = (vp.y + g.top - a.window_state.scroll_y) as i32;
    let cx = a.content_x() as i32;
    let f = a.render();
    assert!(
        any_pixel(
            f,
            cx + rect.0 as i32,
            sy + rect.1 as i32,
            rect.2 as u32,
            rect.3 as u32,
            is_red
        ),
        "正文 VISUAL 应为红色（实际区域像素: {:?}）",
        f.get((cx as f32 + rect.0) as u32, (sy as f32 + rect.1) as u32)
    );

    // 2) 表格单元格：可见文本为 VISUAL（无标签），光栅像素含红色。
    let tid = block_id_of(&a, |k| matches!(k, BlockKind::Table));
    let tl = &a.cache.tables[&tid].value;
    assert_eq!(tl.cells.len(), 2, "表头 + 一行表体");
    let cell = &tl.cells[1];
    assert_eq!(cell.pixels.len() as u32, cell.pixel_w * cell.pixel_h * 4);
    assert!(
        cell.pixels
            .chunks_exact(4)
            .any(|p| is_red([p[0], p[1], p[2], p[3]])),
        "表格单元格 VISUAL 应为红色"
    );
}

/// 表格单元格内 sub/sup 缩小且基线偏移（与同文本普通单元格逐像素比较）。
#[test]
fn table_cell_sub_sup_smaller_and_shifted() {
    let store = MemStore::default();
    let text = "| 2 | 2 | 2 |\n| --- | --- | --- |\n| 2 | <sub>2</sub> | <sup>2</sup> |\n\n尾\n";
    let mut a = app(text, None, false, &store);
    a.editor.set_caret(text.len());
    a.refresh_index();
    a.ensure_tables();
    let id = block_id_of(&a, |k| matches!(k, BlockKind::Table));
    let tl = &a.cache.tables[&id].value;
    let boxed = |i: usize| {
        let c = &tl.cells[i];
        (c.pixels.clone(), c.pixel_w, c.pixel_h)
    };
    let (plain, pw, ph) = boxed(3);
    let (sub, sw, sh) = boxed(4);
    let (sup, uw, uh) = boxed(5);
    let (p_lo, p_hi) = raster_ink_rows(&plain, pw, ph).expect("普通格应有墨迹");
    let (s_lo, s_hi) = raster_ink_rows(&sub, sw, sh).expect("sub 格应有墨迹");
    let (u_lo, u_hi) = raster_ink_rows(&sup, uw, uh).expect("sup 格应有墨迹");
    assert!(
        s_hi - s_lo < p_hi - p_lo,
        "sub 字形高度应小于正文（sub {s_lo}..{s_hi} vs 正文 {p_lo}..{p_hi}）"
    );
    assert!(
        u_hi - u_lo < p_hi - p_lo,
        "sup 字形高度应小于正文（sup {u_lo}..{u_hi} vs 正文 {p_lo}..{p_hi}）"
    );
    // 基线偏移方向：同一字符在 sub 里整体低于 sup（下沉/上浮）
    assert!(
        s_lo > u_lo && s_hi > u_hi,
        "sub 应较 sup 下沉（sub {s_lo}..{s_hi} vs sup {u_lo}..{u_hi}）"
    );
    assert!(u_hi < p_hi, "sup 应上浮（底部高于正文）");
}

/// 行内 code（Markdown 反引号与 `<pre>`）均有代码底色，且与正文同一装饰语义。
#[test]
fn inline_code_and_pre_have_code_background() {
    use crate::view::UnderKind;
    let store = MemStore::default();
    let text = "行内 `code` 与 <pre>预格式</pre> 尾\n";
    let mut a = app(text, None, false, &store);
    a.editor.set_caret(text.len() - 1);
    a.ensure_layouts_all();
    a.recompute_geometry();
    let id = a.index.blocks[0].id;
    let (rects, code_bg, sy, cx) = {
        let g = a.index.blocks.iter().find(|g| g.id == id).unwrap();
        let vp = a.viewport();
        let sy = (vp.y + g.top - a.window_state.scroll_y) as i32;
        let cx = a.content_x() as i32;
        let l = &a.cache.layouts[&id];
        let kinds: Vec<_> = l
            .decorations_under
            .iter()
            .filter(|(_, _, k)| matches!(k, UnderKind::InlineCodeBg))
            .map(|&(s, e, _)| (s, e))
            .collect();
        let mut rects = Vec::new();
        for (s, e) in kinds {
            for r in a.engine.selection_rects(l, s, e) {
                rects.push(r);
            }
        }
        (rects, a.input_state.theme.code_bg, sy, cx)
    };
    assert_eq!(rects.len(), 2, "`code` 与行内 `<pre>` 应各有一段代码底色");
    let f = a.render();
    for r in rects {
        assert!(
            any_pixel(
                f,
                cx + r.0 as i32,
                sy + r.1 as i32,
                r.2 as u32,
                r.3 as u32,
                |p| p == code_bg
            ),
            "行内代码底色应为主题 code_bg：{:?}",
            code_bg
        );
    }
}

/// abbr 悬停：命中 `<abbr title>` 后状态含 title 文本（浮层由既有 tooltip 绘制）。
#[test]
fn abbr_hover_exposes_title_tooltip() {
    let store = MemStore::default();
    let text = "这里 <abbr title=\"Application Programming Interface\">API</abbr> 缩写。\n";
    let mut a = app(text, None, false, &store);
    let off = text.find("API").unwrap();
    let (x, y) = point_at_doc(&mut a, off);
    a.input_state.mouse = PhysicalPosition::new(x as f64, y as f64);
    a.update_hover();
    assert_eq!(
        a.ui_state.hover_link.as_ref().map(|t| t.0.as_str()),
        Some("Application Programming Interface"),
        "abbr 悬停应暴露 title 文本"
    );
    // 移到正文区外 → 清除
    a.input_state.mouse = PhysicalPosition::new(x as f64, 2.0);
    a.update_hover();
    assert!(a.ui_state.hover_link.is_none());
}

// ---- B5：排版留白与输入空间（底部留白 / 空行行高 / 末尾空块 / 点击落末尾） ----

// ---- 首帧优化回归锁（P1b / geometry_stale） ----

/// refresh_index 在窗口创建前只做估算累计：必须让 geometry_stale = true，
/// 告诉首次 render 走真实重算路径。此前它会在 App::build 里调用完整
/// recompute_geometry（全量 cache.layout 查表 + cosmic-text 首次排版）——
/// 改后 refresh_index 走 estimate_geometry（只用 est_h），省掉 cosmic-text 开销。
#[test]
fn refresh_index_sets_geometry_stale_before_render() {
    let store = MemStore::default();
    // 多块文档，确保 estimate_geometry 的累计路径被覆盖
    let text = "# H1\n\np1\n\np2\n\n# H2\n\np3\n\n```\ncode\n```\n";
    let a = app(text, None, false, &store);
    // App::new 内部已调 refresh_index → estimate_geometry
    assert!(
        a.index.geometry_stale,
        "refresh_index 应设 geometry_stale=true，等首次 render 真实重算"
    );
    assert!(
        !a.index.blocks.is_empty(),
        "estimate_geometry 后块清单不应为空"
    );
    assert!(
        a.index.total_h > 0.0,
        "estimate_geometry 应给出正的文档总高（用于滚动条初值）"
    );
    // 所有块的 height/top 已被估算累计填充（非零非 NaN）
    for g in &a.index.blocks {
        assert!(g.height.is_finite(), "块 height 应为有限值");
        assert!(g.top.is_finite(), "块 top 应为有限值");
        assert!(g.height >= 0.0, "块 height 不应为负");
        assert!(g.top >= 0.0, "块 top 不应为负");
    }
    // 估算高度本身应合理：首块高度 ≥ 1 行
    let first = &a.index.blocks[0];
    assert!(
        first.est_h >= a.engine.doc_line_height() * a.input_state.scale,
        "est_h 应至少一行高"
    );
}

/// 首次 render 必须把 geometry_stale 拉回 false——这是 paint.rs 的核心分支：
/// geometry_stale 为 true 时调 recompute_geometry()（真实排版），之后设为 false。
/// 如果 render 没清 stale，后续击键帧会反复重算整篇几何（P1b 被击穿）。
#[test]
fn render_clears_geometry_stale_after_recompute() {
    let store = MemStore::default();
    let text = "# H1\n\np1\n\np2\n\n# H2\n\np3\n";
    let mut a = app(text, None, false, &store);
    assert!(a.index.geometry_stale, "precondition: stale 应为 true");

    let _ = a.render();

    assert!(
        !a.index.geometry_stale,
        "render 后 geometry_stale 应为 false（真实 recompute_geometry 覆盖了估算）"
    );
    // render 后块高度应该是真实排版结果（与 est_h 可能不同，但至少合理）
    let first = &a.index.blocks[0];
    assert!(first.height > 0.0, "render 后首块高度应为正");
    assert!(first.top >= 0.0, "render 后首块 top 应非负");
}

/// 首次 render 后 doc_height 必须大于 0——滚动条、滚轮钳位、底部留白
/// 都依赖这个值。如果 estimate_geometry 累计给了有效初值，render 再覆盖一次
/// 应该更大或相等（真实排版不会比估算更矮太多）。
#[test]
fn first_frame_has_valid_total_height_for_scrollbar() {
    let store = MemStore::default();
    let text = format!("{}\n", "p1\np2\np3\n".repeat(200));
    let mut a = app(&text, None, false, &store);

    let est_total = a.index.total_h;
    assert!(est_total > 0.0, "估算阶段 doc 总高应已有效");

    let _ = a.render();

    let real_total = a.doc_height();
    assert!(real_total > 0.0, "render 后 doc_height 必须为正");
    // 真实总高和估算总高应在同一量级（估算误差不超过 ±50% 为异常）
    let ratio = real_total / est_total.max(1.0);
    assert!(
        ratio > 0.5 && ratio < 2.0,
        "真实/估算 doc 总高应在 0.5x–2.0x 内，实际 ratio={ratio:.3} \
         (est={est_total:.1}, real={real_total:.1})——估算偏差过大可能导致首帧跳变"
    );
}
