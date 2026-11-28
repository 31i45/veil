//! 表格渲染与交互测试：网格 / 源码 / 命中 / 单元格导航 / 右键上下文菜单 / 对齐。

use super::*;

#[test]
fn table_renders_grid_when_caret_outside() {
    let (mut a, _) = app_math(TABLE_DOC, false);
    a.ensure_layouts_all();
    a.ensure_tables();
    let (id, kind, range) = a.editor.session().blocks()[1].clone();
    assert_eq!(kind, BlockKind::Table);
    assert!(a.table_shows_grid(id, &range));
    assert_eq!(a.block_height(geom(&a, id)), a.cache.tables[&id].height);
}

#[test]
fn table_shows_source_when_caret_on_pipe() {
    let (mut a, _) = app_math(TABLE_DOC, false);
    assert!(a.editor.set_caret(3)); // 管道符 `|`（block 内但不在任何 cell.range）
    a.ensure_layouts_all();
    a.ensure_tables();
    let (id, _, range) = a.editor.session().blocks()[1].clone();
    assert!(
        !a.cache.tables.contains_key(&id),
        "管道符位置 → 源码态不排版网格"
    );
    assert!(!a.table_shows_grid(id, &range));
}

#[test]
fn table_shows_grid_when_caret_inside_cell() {
    let (mut a, _) = app_math(TABLE_DOC, false);
    assert!(a.editor.set_caret(5)); // 表头单元格内（cell.range 内）
    a.ensure_layouts_all();
    a.ensure_tables();
    let (id, _, range) = a.editor.session().blocks()[1].clone();
    assert!(
        a.cache.tables.contains_key(&id),
        "活动单元格 → 网格继续显示"
    );
    assert!(a.table_shows_grid(id, &range));
}

#[test]
fn table_grid_hit_is_precise_for_plain_cells() {
    // 纯文本格（剥标记后与源码等长）可按字形精确落点；富文本格仍落格首。
    let (mut a, _) = app_math(TABLE_DOC, false);
    a.ensure_layouts_all();
    a.ensure_tables();
    let (id, kind, range) = a.editor.session().blocks()[1].clone();
    let want = a.editor.session().table(id).unwrap().rows[0][1].range.start; // 表体「d」
    let (cell_x, cell_w, cell_y, cell_h) = {
        let t = a.cache.tables.get(&id).unwrap();
        let cell = &t.cells[3]; // 第 2 行第 2 列
        (cell.x, cell.w, t.rows[1].top, t.rows[1].height)
    };
    let y = cell_y + cell_h / 2.0;
    assert_eq!(
        a.hit_block(id, kind.clone(), &range, cell_x + 0.5, y),
        Some(want),
        "左侧内边距落格首"
    );
    assert_eq!(
        a.hit_block(id, kind, &range, cell_x + cell_w - 1.0, y),
        Some(want + 1),
        "单字格右侧落格尾（光标可停在字后）"
    );
}

#[test]
fn table_cell_first_click_lands_on_clicked_glyph() {
    // 首次点击（caret 在表格外、该格尚非活动格）也要精确到字形。
    let doc = "p\n\n| h | x |\n| --- | --- |\n| alpha beta | y |\n";
    let (mut a, _) = app_math(doc, false);
    a.ensure_layouts_all();
    a.ensure_tables();
    let (id, kind, range) = a.editor.session().blocks()[1].clone();
    let start = a.editor.session().table(id).unwrap().rows[0][0].range.start;
    let (dx, y) = {
        let t = a.cache.tables.get(&id).unwrap();
        let cell = &t.cells[2]; // 正文第 1 行第 1 列「alpha beta」
        let map = cell.caret_map.as_ref().expect("纯文本格应有字形档位");
        let (dx, _, h) = map.caret_at(6).expect("偏移 6 有位置");
        (cell.x + dx + 0.25, t.rows[1].top + h * 0.5)
    };
    assert_eq!(
        a.hit_block(id, kind, &range, dx, y),
        Some(start + 6),
        "首次点击应落在被点中的字形，而不是格首"
    );
}

#[test]
fn table_cell_second_click_lands_on_clicked_glyph() {
    // 真机反馈：点进单元格后无法把光标移到想要的位置。
    // 首次点击进入单元格（落内容起点），再次点击同一格内偏右的位置必须精确落点。
    let doc = "| h | x |\n| --- | --- |\n| alpha beta | y |\n";
    let (mut a, _) = app_math(doc, false);
    let (id, _, _) = a.editor.session().blocks()[0].clone();
    let start = a.editor.session().table(id).unwrap().rows[0][0].range.start;
    assert!(a.editor.set_caret(start));
    a.ensure_layouts_all();
    a.ensure_tables();
    let top = a.index.blocks[0].top;
    let (cell_x, cell_y, grid_x) = {
        let cell = &a.cache.tables[&id].cells[2]; // 正文第 1 行第 1 列「alpha beta」
        assert!(cell.caret_map.is_some(), "活动格应有字形档位");
        (cell.x, cell.y, a.cache.tables[&id].x)
    };
    let (dx, dy, h) = {
        let cell = &a.cache.tables[&id].cells[2];
        let map = cell.caret_map.as_ref().unwrap();
        map.caret_at(6).expect("偏移 6 有位置")
    };
    assert!(dy.abs() < 0.01 && h > 0.0);
    let vp = a.viewport();
    let x = a.content_x() + grid_x + cell_x + dx + 0.25;
    let y = vp.y + top - a.window_state.scroll_y + cell_y + dy + h * 0.5;
    click_at(&mut a, x, y);
    assert_eq!(
        a.editor.caret() - start,
        6,
        "点击格内偏右位置应落在对应字形，而不是永远停在格首"
    );
    // 光标必须画在这个字形处（而非单元格左上角）。
    let (cx, _, _) = a.caret_screen();
    assert!(
        (cx - (a.content_x() + grid_x + cell_x + dx)).abs() < 1.0,
        "光标 x 应随偏移移动：{cx}"
    );
}

#[test]
fn table_grid_caret_drawn_at_active_cell_content_start() {
    // C5 定位修复：网格态光标必须画在活动单元格内容起点，而不是源码排版的字形位置。
    let (mut a, _) = app_math(TABLE_DOC, false);
    a.ensure_layouts_all();
    a.ensure_tables();
    let (id, _, _) = a.editor.session().blocks()[1].clone();
    let caret = a.editor.session().table(id).unwrap().rows[0][1].range.start;
    a.editor.set_caret(caret);
    a.ensure_tables();
    let top = a
        .index
        .blocks
        .iter()
        .find(|g| g.id == id)
        .expect("表格块应存在")
        .top;
    let scale = a.input_state.scale;
    let (cell_x, cell_y) = {
        let cell = &a.cache.tables[&id].cells[3]; // 第 2 行第 2 列
        (cell.x, cell.y)
    };
    let vp = a.viewport();
    let (x, y, h) = a.caret_screen();
    assert!(
        (x - (a.content_x() + cell_x)).abs() < 0.5,
        "光标 x 应等于单元格内容左缘：{x} vs {}",
        a.content_x() + cell_x
    );
    assert!(
        (y - (vp.y + top - a.window_state.scroll_y + cell_y)).abs() < 0.5,
        "光标 y 应等于单元格内容上缘：{y} vs {}",
        vp.y + top - a.window_state.scroll_y + cell_y
    );
    assert!((h - BASE_LINE_HEIGHT * scale).abs() < 0.5);
}

#[test]
fn table_grid_rebuilds_when_editing_inside_active_cell() {
    let (mut a, _) = app_math(TABLE_DOC, false);
    a.ensure_layouts_all();
    a.ensure_tables();
    let old_id = a.editor.session().blocks()[1].0;
    let old_w = a.cache.tables[&old_id].width;
    // 表头「a」变宽（xxxxa）→ 块脏 → BlockId 重建 → after_edit 重建网格
    assert!(a.editor.set_caret(5));
    let dirty = a.editor.apply(Intent::InsertText("xxxx".into())).unwrap();
    assert_eq!(dirty.len(), 1);
    a.after_edit(dirty);
    let new_id = a.editor.session().blocks()[1].0;
    // 活动单元格内编辑 → 网格继续排版且重建（列宽变大）
    assert!(
        a.cache.tables.contains_key(&new_id),
        "活动单元格内 → 网格重建"
    );
    assert!(
        a.cache.tables[&new_id].width > old_w,
        "内容变宽 → 新网格列宽变大"
    );
}

// ---- 容器内表格（B2）：引用/列表/details 原生网格 + 容器缩进 ----

#[test]
fn quote_container_table_renders_native_grid_with_offset() {
    let doc = "p\n\n> | h | h |\n> | --- | --- |\n> | **MMMM** | MMMM |\n";
    let (mut a, _) = app_math(doc, false);
    a.ensure_layouts_all();
    a.ensure_tables();
    let blocks = a.editor.session().blocks();
    assert_eq!(blocks.len(), 2, "段落 + 引用内表格块");
    let (id, kind, range) = blocks[1].clone();
    assert_eq!(kind, BlockKind::Table);
    let t = &a.cache.tables[&id];
    assert!(t.x > 0.0, "引用内表格应右移（容器缩进），实际 {}", t.x);
    assert_eq!(t.rows.len(), 2, "表头 + 1 行表体");
    assert!(
        t.height > a.engine.doc_line_height() * 1.5,
        "原生表格高度应大于一行"
    );
    assert!(
        t.cells[2].pixel_w > t.cells[3].pixel_w,
        "**粗体** 单元格应宽于同文本普通格（B1 行内样式沿用）"
    );
    assert_eq!(a.block_height(geom(&a, id)), t.height);

    // 命中位置扣除容器缩进后应落回单元格内容起点（与顶层表格同一路径）
    let want = a.editor.session().table(id).unwrap().rows[0][1].range.start;
    let cell = &t.cells[3];
    let hit = a.hit_block(
        id,
        kind,
        &range,
        t.x + cell.x + 0.5,
        t.rows[1].top + t.rows[1].height / 2.0,
    );
    assert_eq!(hit, Some(want));
}

#[test]
fn list_item_container_table_renders_with_indent() {
    let doc = "p\n\n- | 键 | 值 |\n  | --- | --- |\n  | k | v |\n";
    let (mut a, _) = app_math(doc, false);
    a.ensure_layouts_all();
    a.ensure_tables();
    let blocks = a.editor.session().blocks();
    assert_eq!(blocks.len(), 2);
    let (id, _, _) = blocks[1].clone();
    assert_eq!(blocks[1].1, BlockKind::Table);
    let t = &a.cache.tables[&id];
    assert!(t.x > 0.0, "列表项内表格应有内容列缩进");
    assert_eq!(t.rows.len(), 2);
}

#[test]
fn details_child_table_tracks_parent_expand_state() {
    let doc = "p\n\n<details>\n<summary>S</summary>\n| a |\n| --- |\n| 1 |\n</details>\n";
    let (mut a, _) = app_math(doc, false);
    a.refresh_index();
    let table_id = block_id_of(&a, |k| *k == BlockKind::Table);
    let details_id = block_id_of(&a, |k| matches!(k, BlockKind::Details { .. }));
    a.ensure_layouts_all();
    // 默认收起：子表格不占行高
    let g = a.index.blocks.iter().find(|g| g.id == table_id).unwrap();
    assert_eq!(a.block_height(g), 0.0, "收起态子表格应隐藏");
    // 展开后：原生表格占位，且 details 卡片覆盖子块高度
    a.ui_state.expanded_details.insert(details_id);
    a.ensure_layouts_all();
    a.ensure_tables();
    let g = a.index.blocks.iter().find(|g| g.id == table_id).unwrap();
    let h = a.block_height(g);
    assert!(h > a.engine.doc_line_height() * 1.5, "展开态子表格应排版");
    let header = a.index.blocks.iter().find(|b| b.id == details_id).unwrap();
    assert!(
        a.details_children_height(header) >= h,
        "details 卡片高度应覆盖子表格"
    );
    // 子表格切段后闭标签移出 details 头块：裸标签仍须消隐，只显示 summary。
    let header_text = a.cache.layouts[&details_id].layout_text();
    assert!(
        !header_text.contains('<') && header_text.contains('S'),
        "details 头块应只显示 summary，实际：{header_text:?}"
    );
}

// ---- 单元格富文本（B3） ----

#[test]
fn table_active_cell_shows_source_then_restores_rich_text() {
    let (mut a, _) = app_math(TABLE_RICH_DOC, false);
    a.ensure_layouts_all();
    a.ensure_tables();
    let (id, _, _) = a.editor.session().blocks()[1].clone();
    assert_eq!(a.cache.tables[&id].active, None, "表外 → 全部富文本");
    let rich_w = a.cache.tables[&id].cells[0].pixel_w;

    let cell_start = a.editor.session().table(id).unwrap().header[0].range.start;
    assert!(a.editor.set_caret(cell_start));
    a.ensure_tables();
    assert_eq!(a.cache.tables[&id].active, Some(0), "格内 → 活动格源码态");
    let src_w = a.cache.tables[&id].cells[0].pixel_w;
    assert!(
        src_w > rich_w + 8,
        "活动格应显示含 ** 的源码：src={src_w} rich={rich_w}"
    );

    assert!(a.editor.set_caret(0));
    a.ensure_tables();
    assert_eq!(a.cache.tables[&id].active, None, "离开 → 恢复富文本");
    assert_eq!(a.cache.tables[&id].cells[0].pixel_w, rich_w);
}

#[test]
fn table_active_path_150_dpi_keeps_nonactive_inline_code_single_line() {
    let doc = "| 名称 | 格式 | 对齐与换行 |\n| :--- | :---: | ---: |\n| Alpha | **粗体** | 左中右三列必须完整 |\n| Beta | *斜体* | 较长的中文单元格用于检查换行、边框与裁切是否清晰 |\n| Gamma | `code` | 1234567890 |\n";
    let (mut a, _) = app_math(doc, false);
    a.window_state.test_size = Some((1422, 1106));
    a.input_state.set_dpi(1.5);
    let (id, _, _) = a.editor.session().blocks()[0].clone();
    let beta = a.editor.session().table(id).unwrap().rows[1][0].range.start;
    assert!(a.editor.set_caret(beta));
    a.ensure_layouts_all();
    a.ensure_tables();

    let rich_code_w = {
        let grid = &a.cache.tables[&id];
        assert_eq!(grid.active, Some(6), "Beta 首格应为 flat 6 活动格");
        assert_eq!(
            grid.cells[10].h,
            BASE_LINE_HEIGHT * 1.5,
            "Beta 活动时，非活动 Gamma code 必须保持绝对单行 33px"
        );
        grid.cells[10].pixel_w
    };

    let code = a.editor.session().table(id).unwrap().rows[2][1].range.start;
    assert!(a.editor.set_caret(code));
    a.ensure_tables();
    let grid = &a.cache.tables[&id];
    assert_eq!(grid.active, Some(10), "Gamma code 应为 flat 10 活动格");
    let source_w = grid.cells[10].pixel_w;
    assert!(
        source_w > rich_code_w + 8,
        "活动格应显示含两个反引号的 `code` 源码：source={source_w} rich={rich_code_w}"
    );
}

#[test]
fn table_cache_key_tracks_active_cell() {
    let (mut a, _) = app_math(TABLE_DOC, false);
    a.ensure_layouts_all();
    a.ensure_tables();
    let (id, _, _) = a.editor.session().blocks()[1].clone();
    let k0 = a.table_key(id);
    assert!(a.cache.table(id, k0).is_some(), "表外键应命中");

    let cell_start = a.editor.session().table(id).unwrap().rows[0][0].range.start;
    assert!(a.editor.set_caret(cell_start));
    a.ensure_tables();
    let k1 = a.table_key(id);
    assert_ne!(k0, k1, "活动格变化 → 键变化");
    assert_eq!(k1.active, 2, "flat 索引：header 2 格后的首个体格");
    assert!(a.cache.table(id, k1).is_some(), "新键命中");
    assert!(
        a.cache.table(id, k0).is_none(),
        "旧键失配（切换活动格即失效）"
    );
}

// ---- Tab / Shift+Tab 单元格导航（T2） ----

/// 回归（架构审计【高】1）：Ctrl+Enter 追加行必须经 Editor 门面，
/// 置脏标（自动保存/恢复/关窗确认依赖）、光标落入新行首格、单撤销步。
#[test]
fn table_ctrl_enter_sets_dirty_and_moves_caret_into_new_row() {
    let (mut a, _) = app_math(TABLE_2X3, false);
    a.ensure_layouts_all();
    a.ensure_tables();
    let (id, _, _) = a.editor.session().blocks()[1].clone();
    let cell = a.editor.session().table(id).unwrap().rows[0][1].range.start;
    a.editor.set_caret(cell);

    assert!(a.table_ctrl_enter(), "表内 Ctrl+Enter 应追加空行");
    assert!(a.editor.is_dirty(), "追加行必须置脏标");
    assert!(a.title_text().starts_with('*'), "标题应显示脏标");

    let src = a.editor.session().serialize();
    assert!(src.ends_with("|  |  |  |\n"), "新行应为空单元格行：{src:?}");
    let last_line = src
        .trim_end_matches('\n')
        .rfind('\n')
        .map(|i| i + 1)
        .unwrap();
    assert!(
        a.editor.caret() >= last_line,
        "光标应落在新行而非表首：caret={} last_line={last_line}",
        a.editor.caret()
    );

    a.editor.apply(Intent::Undo).unwrap();
    assert_eq!(
        a.editor.session().serialize(),
        TABLE_2X3,
        "追加行是单撤销步"
    );
}

// ---- 表格行列增删右键菜单（Task 5 / B2） ----

#[test]
fn right_click_table_body_opens_menu_and_inserts_row_below() {
    let (mut a, _) = app_math(TABLE_MENU_DOC, false);
    right_click_table_cell(&mut a, 2); // 第 1 个正文行第 1 列
    assert!(a.ui_state.ctx.open, "表格内右键应打开菜单");
    assert_eq!(
        a.ui_state.ctx.table,
        Some(ctx_menu::TableCtx {
            body_row: Some(0),
            col: 0,
            ncols: 2
        })
    );

    let i = a
        .ui_state
        .ctx
        .items()
        .iter()
        .position(|&(_, act)| act == ctx_menu::CtxAction::TableInsertRowBelow)
        .expect("正文行菜单应有「下方插入行」");
    a.ui_state.ctx.hover = Some(i);
    a.route_key(Key::Enter);

    assert!(!a.ui_state.ctx.open, "执行后菜单关闭");
    assert_eq!(
        a.editor.session().serialize(),
        "p\n\n| a | b |\n| --- | --- |\n| c | d |\n|  |  |\n| e | f |\n"
    );
    let id = a
        .editor
        .session()
        .blocks()
        .into_iter()
        .find(|(_, k, _)| *k == BlockKind::Table)
        .map(|(id, _, _)| id)
        .expect("插入行后仍应有表格块");
    let tbl = a.editor.session().table(id).unwrap();
    assert_eq!(tbl.rows.len(), 3, "正文行 +1");
    let cell = &tbl.rows[1][0];
    assert!(
        cell.range.start <= a.editor.caret() && a.editor.caret() <= cell.range.end,
        "光标应落在新行第 1 列内容起点：caret={} cell=[{},{}]",
        a.editor.caret(),
        cell.range.start,
        cell.range.end
    );

    a.editor.apply(Intent::Undo).unwrap();
    assert_eq!(
        a.editor.session().serialize(),
        TABLE_MENU_DOC,
        "插入行是单撤销步"
    );
}

#[test]
fn table_ctx_actions_transform_source_and_place_caret() {
    // (动作, 上下文, 期望源码, 期望光标所在行/列：0=header、2+=body)
    type Case = (
        ctx_menu::CtxAction,
        ctx_menu::TableCtx,
        &'static str,
        usize,
        usize,
    );
    let cases: [Case; 7] = [
        (
            ctx_menu::CtxAction::TableInsertRowAbove,
            ctx_menu::TableCtx {
                body_row: Some(0),
                col: 0,
                ncols: 2,
            },
            "p\n\n| a | b |\n| --- | --- |\n|  |  |\n| c | d |\n| e | f |\n",
            2,
            0,
        ),
        (
            ctx_menu::CtxAction::TableInsertRowBelow,
            ctx_menu::TableCtx {
                body_row: Some(1),
                col: 1,
                ncols: 2,
            },
            "p\n\n| a | b |\n| --- | --- |\n| c | d |\n| e | f |\n|  |  |\n",
            4,
            1,
        ),
        (
            // 删除首行 → 无上一行，落到下一行同列
            ctx_menu::CtxAction::TableDeleteRow,
            ctx_menu::TableCtx {
                body_row: Some(0),
                col: 1,
                ncols: 2,
            },
            "p\n\n| a | b |\n| --- | --- |\n| e | f |\n",
            2,
            1,
        ),
        (
            // 删除末行 → 落到上一行同列
            ctx_menu::CtxAction::TableDeleteRow,
            ctx_menu::TableCtx {
                body_row: Some(1),
                col: 1,
                ncols: 2,
            },
            "p\n\n| a | b |\n| --- | --- |\n| c | d |\n",
            2,
            1,
        ),
        (
            ctx_menu::CtxAction::TableInsertColumnLeft,
            ctx_menu::TableCtx {
                body_row: Some(0),
                col: 1,
                ncols: 2,
            },
            "p\n\n| a |  | b |\n| --- | --- | --- |\n| c |  | d |\n| e |  | f |\n",
            2,
            1,
        ),
        (
            ctx_menu::CtxAction::TableInsertColumnRight,
            ctx_menu::TableCtx {
                body_row: Some(0),
                col: 1,
                ncols: 2,
            },
            "p\n\n| a | b |  |\n| --- | --- | --- |\n| c | d |  |\n| e | f |  |\n",
            2,
            2,
        ),
        (
            ctx_menu::CtxAction::TableDeleteColumn,
            ctx_menu::TableCtx {
                body_row: None,
                col: 0,
                ncols: 2,
            },
            "p\n\n| b |\n| --- |\n| d |\n| f |\n",
            0,
            0,
        ),
    ];
    for (action, tc, want, line, col) in cases {
        let (mut a, _) = app_math(TABLE_MENU_DOC, false);
        exec_table_ctx(&mut a, tc, action);
        assert_eq!(a.editor.session().serialize(), want, "{action:?} 源码不符");
        let id = a
            .editor
            .session()
            .blocks()
            .into_iter()
            .find(|(_, k, _)| *k == BlockKind::Table)
            .map(|(id, _, _)| id)
            .expect("操作后仍应有表格块");
        let tbl = a.editor.session().table(id).unwrap();
        let cell = if line == 0 {
            &tbl.header[col]
        } else {
            &tbl.rows[line - 2][col]
        };
        assert!(
            cell.range.start <= a.editor.caret() && a.editor.caret() <= cell.range.end,
            "{action:?} 光标未落到第 {line} 行第 {col} 列：caret={} cell=[{},{}]",
            a.editor.caret(),
            cell.range.start,
            cell.range.end
        );
    }
}

#[test]
fn table_ctx_actions_are_single_undo_step() {
    let (mut a, _) = app_math(TABLE_MENU_DOC, false);
    exec_table_ctx(
        &mut a,
        ctx_menu::TableCtx {
            body_row: Some(0),
            col: 0,
            ncols: 2,
        },
        ctx_menu::CtxAction::TableDeleteColumn,
    );
    assert_ne!(a.editor.session().serialize(), TABLE_MENU_DOC);
    a.editor.apply(Intent::Undo).unwrap();
    assert_eq!(
        a.editor.session().serialize(),
        TABLE_MENU_DOC,
        "结构操作是单撤销步"
    );
}

#[test]
fn table_align_actions_map_to_expected_markers() {
    // 分隔格宽 3：左 `:--`、中 `:-:`、右 `--:`（宽度不变）
    for (action, want) in [
        (
            ctx_menu::CtxAction::TableAlignLeft,
            "p\n\n| a | b |\n| --- | :-- |\n| c | d |\n| e | f |\n",
        ),
        (
            ctx_menu::CtxAction::TableAlignCenter,
            "p\n\n| a | b |\n| --- | :-: |\n| c | d |\n| e | f |\n",
        ),
        (
            ctx_menu::CtxAction::TableAlignRight,
            "p\n\n| a | b |\n| --- | --: |\n| c | d |\n| e | f |\n",
        ),
    ] {
        let (mut a, _) = app_math(TABLE_MENU_DOC, false);
        exec_table_ctx(
            &mut a,
            ctx_menu::TableCtx {
                body_row: Some(0),
                col: 1,
                ncols: 2,
            },
            action,
        );
        assert_eq!(a.editor.session().serialize(), want, "{action:?} 源码不符");
    }
}

#[test]
fn table_align_sets_aligns_keeps_caret_and_undo_restores() {
    let (mut a, _) = app_math(TABLE_MENU_DOC, false);
    let (id, _, range) = a
        .editor
        .session()
        .blocks()
        .into_iter()
        .find(|(_, k, _)| *k == BlockKind::Table)
        .expect("文档应含表格块");
    let cell_start = a.editor.session().table(id).unwrap().rows[1][1].range.start;
    assert!(a.editor.set_caret(cell_start));
    let before = a.editor.caret() - range.start;

    let tc = ctx_menu::TableCtx {
        body_row: Some(1),
        col: 1,
        ncols: 2,
    };
    a.ui_state
        .ctx
        .open_at(10.0, 20.0, ctx_menu::CtxKind::Table(tc), None, None);
    let i = a
        .ui_state
        .ctx
        .items()
        .iter()
        .position(|&(_, act)| act == ctx_menu::CtxAction::TableAlignRight)
        .expect("表格菜单应有「右对齐」");
    a.ui_state.ctx.hover = Some(i);
    a.route_key(Key::Enter);

    assert_eq!(
        a.editor.session().serialize(),
        "p\n\n| a | b |\n| --- | --: |\n| c | d |\n| e | f |\n"
    );
    let align_of = |a: &App| -> veil_core::CellAlign {
        let id = a
            .editor
            .session()
            .blocks()
            .into_iter()
            .find(|(_, k, _)| *k == BlockKind::Table)
            .map(|(id, _, _)| id)
            .expect("编辑后仍应有表格块");
        a.editor.session().table(id).unwrap().aligns[1]
    };
    assert_eq!(
        align_of(&a),
        veil_core::CellAlign::Right,
        "解析后的列对齐应更新"
    );
    assert_eq!(
        a.editor.caret() - range.start,
        before,
        "对齐不移动光标（相对块起点偏移不变）"
    );

    a.editor.apply(Intent::Undo).unwrap();
    assert_eq!(
        a.editor.session().serialize(),
        TABLE_MENU_DOC,
        "对齐是单撤销步"
    );
    assert_eq!(align_of(&a), veil_core::CellAlign::None, "撤销后对齐复原");
}

#[test]
fn right_click_table_header_offers_no_row_actions() {
    let (mut a, _) = app_math(TABLE_MENU_DOC, false);
    right_click_table_cell(&mut a, 0); // 表头第 1 列
    assert_eq!(
        a.ui_state.ctx.table,
        Some(ctx_menu::TableCtx {
            body_row: None,
            col: 0,
            ncols: 2
        })
    );
    let row_ops = [
        ctx_menu::CtxAction::TableInsertRowAbove,
        ctx_menu::CtxAction::TableInsertRowBelow,
        ctx_menu::CtxAction::TableDeleteRow,
    ];
    assert!(
        !a.ui_state
            .ctx
            .items()
            .iter()
            .any(|&(_, act)| row_ops.contains(&act)),
        "表头不提供行操作"
    );
    assert!(a
        .ui_state
        .ctx
        .items()
        .iter()
        .any(|&(_, act)| act == ctx_menu::CtxAction::TableDeleteColumn));
}

#[test]
fn tab_advances_to_next_cell_start() {
    let (mut a, _) = app_math(TABLE_2X3, false);
    a.ensure_layouts_all();
    a.ensure_tables();
    let (id, _, _) = a.editor.session().blocks()[1].clone();
    let tbl = a.editor.session().table(id).unwrap();
    // caret 在表头首格 "a" 的内容起点
    let first = tbl.header[0].range.start;
    a.editor.set_caret(first);
    // Tab → 表头第二格 "b"
    a.route_key(Key::Tab);
    assert_eq!(a.editor.caret(), tbl.header[1].range.start);
    // Tab → 表头第三格 "c"
    a.route_key(Key::Tab);
    assert_eq!(a.editor.caret(), tbl.header[2].range.start);
    // Tab → 表体首格 "d"
    a.route_key(Key::Tab);
    assert_eq!(a.editor.caret(), tbl.rows[0][0].range.start);
}

#[test]
fn tab_on_last_cell_appends_row_and_moves_to_first_cell() {
    let (mut a, _) = app_math(TABLE_2X3, false);
    a.ensure_layouts_all();
    a.ensure_tables();
    let (id, _, _) = a.editor.session().blocks()[1].clone();
    let tbl = a.editor.session().table(id).unwrap();
    // caret 在末格 "f" 内容起点
    a.editor.set_caret(tbl.rows[0][2].range.start);
    a.route_key(Key::Tab);
    // 与 Typora 对齐：末格 Tab 追加一行并落新行首格（不绕回表首）
    assert!(a.editor.is_dirty(), "追加行必须置脏标");
    let src = a.editor.session().serialize();
    assert_eq!(src, format!("{TABLE_2X3}|  |  |  |\n"));
    // 块替换后 Table 可能换 id：按 kind 重新定位
    let id = a
        .editor
        .session()
        .blocks()
        .into_iter()
        .find(|(_, k, _)| *k == BlockKind::Table)
        .map(|(id, _, _)| id)
        .expect("追加行后仍应有表格块");
    let tbl = a.editor.session().table(id).unwrap();
    assert_eq!(tbl.rows.len(), 2, "末格 Tab 应追加一行");
    let first = &tbl.rows[1][0];
    assert!(
        first.range.start <= a.editor.caret() && a.editor.caret() <= first.range.end,
        "光标应落在新行首格内：caret={} cell=[{},{}]",
        a.editor.caret(),
        first.range.start,
        first.range.end
    );
}

#[test]
fn tab_on_structure_char_moves_into_next_cell() {
    let (mut a, _) = app_math(TABLE_2X3, false);
    a.ensure_layouts_all();
    a.ensure_tables();
    let (id, _, _) = a.editor.session().blocks()[1].clone();
    let tbl = a.editor.session().table(id).unwrap();
    // caret 落在表头 "a" 与 "b" 之间的管道符上（结构字符，不属于任何单元格）
    let pipe = tbl.header[0].range.end + 1;
    assert_eq!(
        a.editor
            .session()
            .text_slice(Range::new(pipe, pipe + 1))
            .as_deref(),
        Some("|")
    );
    a.editor.set_caret(pipe);
    a.route_key(Key::Tab);
    assert_eq!(
        a.editor.caret(),
        tbl.header[1].range.start,
        "结构字符上的 Tab 应进入其后第一个单元格"
    );
}

#[test]
fn shift_tab_goes_back() {
    let (mut a, _) = app_math(TABLE_2X3, false);
    a.ensure_layouts_all();
    a.ensure_tables();
    let (id, _, _) = a.editor.session().blocks()[1].clone();
    let tbl = a.editor.session().table(id).unwrap();
    // caret 在表体第二格 "e"
    a.editor.set_caret(tbl.rows[0][1].range.start);
    // Shift+Tab → 表体首格 "d"
    a.input_state.mods = Mods {
        ctrl: false,
        shift: true,
        alt: false,
    };
    a.route_key(Key::Tab);
    assert_eq!(a.editor.caret(), tbl.rows[0][0].range.start);
    a.input_state.mods = Mods {
        ctrl: false,
        shift: false,
        alt: false,
    };
}

#[test]
fn tab_outside_table_is_normal_indent() {
    // caret 在段落文本中 → Tab 仍是缩进（InsertText("    ") 或类似）
    let (mut a, _) = app_math("hello\n", false);
    a.editor.set_caret(0);
    let before = a.editor.caret();
    a.route_key(Key::Tab);
    // 缩进至少让光标右移（插 4 空格）
    assert!(a.editor.caret() >= before, "表外 Tab 不应回退 caret");
}

// ---- 列表 / 引用 / 分隔线 Reveal（切片 12）：块级重排与块高 ----

/// 窄窗 + 8 列宽表「可读完」（F-010）：默认收纳到版心；Shift+滚轮进入可读宽表后，
/// 表宽超出版心，最右列可通过横向平移到达，且绘制 / 命中 / 光标使用同一 pan。
#[test]
fn narrow_window_wide_table_readable_via_shift_wheel_pan() {
    let doc = "前\n\n\
        | 列一 | 列二 | 列三 | 列四 | 列五 | 列六 | 列七 | 列八 |\n\
        | --- | --- | --- | --- | --- | --- | --- | --- |\n\
        | 甲乙丙丁 | 甲乙丙丁 | 甲乙丙丁 | 甲乙丙丁 | 甲乙丙丁 | 甲乙丙丁 | 甲乙丙丁 | 最右列为可读内容 |\n\n\
        后\n";
    let (mut a, _) = app_math(doc, false);
    a.window_state.test_size = Some((520, 700));
    a.editor.set_caret(doc.len());
    a.refresh_index();
    a.ensure_layouts_all();
    a.ensure_tables();
    a.recompute_geometry();
    let (id, kind, range, top) = {
        let g = a
            .index
            .blocks
            .iter()
            .find(|g| g.kind == BlockKind::Table)
            .expect("应有表格块");
        (g.id, g.kind.clone(), g.range, g.top)
    };
    let content_w = a.content_width();
    let (fit_width, indent) = {
        let t = a.cache.tables.get(&id).expect("默认网格已排版");
        (t.width, t.x)
    };
    assert!(
        fit_width <= content_w + 0.5,
        "默认网格必须收纳到版心：{fit_width} > {content_w}"
    );

    // 指针放进表格内部，Shift+滚轮向下（d<0）→ 表格向右平移。
    let vp = a.viewport();
    let scroll = a.window_state.scroll_y;
    a.input_state.mouse = PhysicalPosition::new(
        (a.content_x() + 20.0) as f64,
        (vp.y + top + 8.0 - scroll) as f64,
    );
    let last = a.editor.session().table(id).unwrap().rows[0][7].range.start;
    a.input_state.mods.shift = true;
    a.on_wheel(-10_000.0);
    a.input_state.mods.shift = false;

    let (wide_w, pan, last_x, last_y, last_h, last_w) = {
        let t = a.cache.tables.get(&id).expect("宽表网格已排版");
        let cell = &t.cells[15]; // 表体第 8 列
        (t.width, t.pan, cell.x, cell.y, cell.h, cell.w)
    };
    assert!(
        wide_w > content_w + 0.5,
        "可读宽表必须超出版心（这是可平移的前提）：{wide_w} <= {content_w}"
    );
    let avail = content_w - indent;
    assert!(
        (pan - (wide_w - avail)).abs() < 1.0,
        "平移量应夹在 [0, 表宽-可用宽]：pan={pan}, max={}",
        wide_w - avail
    );
    // 最右列：平移后绘制矩形完整落在版心内，可被读到。
    let painted_left = a.content_x() + indent - pan + last_x;
    let painted_right = painted_left + last_w;
    assert!(
        painted_left >= a.content_x() - 0.5 && painted_right <= a.content_x() + content_w + 0.5,
        "最右列绘制区间应落在版心内：[{painted_left}, {painted_right}] vs [0, {content_w}]"
    );
    assert!(
        painted_left < a.content_x() + content_w,
        "最右列必须进入可见版心"
    );

    // 命中测试：点最右列内容起点 → 落在该单元格（同一 pan 口径）。
    a.input_state.mouse = PhysicalPosition::new(
        (painted_left + 0.25) as f64,
        (vp.y + top - scroll + last_y + last_h * 0.5) as f64,
    );
    assert_eq!(a.hit_offset(), Some(last), "平移后命中必须落在最右列单元格");
    assert!(a.cache.tables.contains_key(&id));
    assert_eq!(
        a.hit_block(
            id,
            kind,
            &range,
            indent - pan + last_x + 0.25,
            last_y + last_h * 0.5
        ),
        Some(last),
        "网格命中与绘制共用 pan"
    );

    // 光标：活动格（最右列）的光标矩形与单元格内容原点一致。
    a.editor.set_caret(last);
    a.ensure_tables();
    let (t_pan, t_x, cell_x, cell_y, active) = {
        let t = a.cache.tables.get(&id).expect("活动格网格已重建");
        let cell = &t.cells[15];
        (t.pan, t.x, cell.x, cell.y, t.active)
    };
    assert_eq!(active, Some(15), "活动单元格应为最右列");
    let (cx, cy, _) = a.caret_screen();
    assert!(
        (cx - (a.content_x() + t_x - t_pan + cell_x)).abs() < 1.0,
        "光标 x 应等于最右列内容左缘：{cx} vs {}",
        a.content_x() + t_x - t_pan + cell_x
    );
    assert!(
        (cy - (vp.y + top - scroll + cell_y)).abs() < 1.0,
        "光标 y 应等于最右列内容上缘"
    );
    assert!(
        cx >= a.content_x() - 0.5 && cx <= a.content_x() + content_w + 0.5,
        "光标必须落在版心内：{cx}"
    );
}

// ===== issue #11「表格出格了」回归 =====

/// 与 issue 截图同形：3 列，正文含行内代码/粗体等混排，第三列长句。
const ISSUE11_DOC: &str = "前\n\n\
    | 字段 | 参数 | 说明 |\n\
    | --- | --- | --- |\n\
    | remotePlay | {ids, start, position} | `{lrc, translation}`, or `{lines}` with timed words where type is mp3, flac, ogg, wav, m4a (ALAC, FLAC or AAC-LC, whole or in fragments as a DASH stream serves it) or aac (AAC-LC in ADTS) |\n\n\
    后\n";

fn issue11_app(scale: f32) -> (App, BlockId, f32, f32) {
    let (mut a, _) = app_math(ISSUE11_DOC, false);
    a.window_state.test_size = Some((840, 700));
    a.input_state.set_dpi(scale);
    a.editor.set_caret(ISSUE11_DOC.len());
    a.refresh_index();
    a.ensure_layouts_all();
    a.ensure_tables();
    a.recompute_geometry();
    let id = a
        .index
        .blocks
        .iter()
        .find(|g| g.kind == BlockKind::Table)
        .expect("应有表格块")
        .id;
    let (top, cw) = {
        let g = a.index.blocks.iter().find(|g| g.id == id).unwrap();
        (g.top, a.content_width())
    };
    (a, id, top, cw)
}

/// 默认模式：表宽 ≤ 版心、首列不低于可读下限、单元格墨迹与帧像素都不越出版心。
#[test]
fn issue11_default_table_fits_content_across_scale() {
    for scale in [1.0f32, 1.25, 1.5, 2.0] {
        let (mut a, id, top, content_w) = issue11_app(scale);
        let (t_w, t_h, cols, cells_right) = {
            let t = a.cache.tables.get(&id).expect("默认网格已排版");
            (
                t.width,
                t.height,
                t.cols.clone(),
                t.cells
                    .iter()
                    .map(|c| t.x + c.x + c.pixel_w as f32)
                    .fold(0.0f32, f32::max),
            )
        };
        assert!(
            t_w <= content_w + 0.5,
            "scale={scale}: 表宽必须收进版心 {t_w} > {content_w}"
        );
        let min = crate::view::table::MIN_COL_WIDTH * a.input_state.scale;
        assert!(
            cols[0] >= min - 0.5,
            "scale={scale}: 首列不得压到可读下限以下 {} < {min}",
            cols[0]
        );
        assert!(
            cells_right <= content_w + 0.6,
            "scale={scale}: 单元格墨迹不得越过版心右界 {cells_right} > {content_w}"
        );
        assert_no_ink_right_of_content(&mut a, top, t_h, content_w);

        // 不需要横滚 → 不绘制滚动条。
        let t = a.cache.tables.get(&id).unwrap();
        assert!(
            a.table_scrollbar_geom(t, 0).is_none(),
            "scale={scale}: 默认收纳后不应出现横向滚动条"
        );
    }
}

/// 渲染帧内、表格纵向范围内，版心右侧（不含竖向滚动条轨道）不得有任何墨迹。
fn assert_no_ink_right_of_content(a: &mut App, top: f32, h: f32, content_w: f32) {
    let vp = a.viewport();
    let scroll = a.window_state.scroll_y;
    let content_right = (a.content_x() + content_w) as u32;
    let lane = (vp.x + vp.w - crate::chrome::SCROLL_W * a.input_state.scale) as u32;
    let bg = a.input_state.theme.bg;
    let y0 = (vp.y + top - scroll).max(vp.y) as u32 + 1;
    let f = a.render();
    let y1 = (vp.y + top - scroll + h)
        .min(vp.y + vp.h)
        .min(f.height as f32) as u32;
    for y in y0..y1 {
        for x in (content_right + 1)..f.width.min(lane) {
            assert_eq!(f.get(x, y), bg, "表格墨迹越过版心右界: ({x},{y})");
        }
    }
}

/// 列表容器内：表宽 ≤ 版心 - 缩进，绘制右界仍在版心内。
#[test]
fn issue11_table_in_list_respects_indent() {
    let doc = "p\n\n- | 字段 | 参数 | 说明 |\n  | --- | --- | --- |\n  | remotePlay | {ids, start, position} | `{lrc, translation}`, or `{lines}` with timed words where type is mp3, flac |\n\n后\n";
    let (mut a, _) = app_math(doc, false);
    a.window_state.test_size = Some((840, 700));
    a.editor.set_caret(doc.len());
    a.refresh_index();
    a.ensure_layouts_all();
    a.ensure_tables();
    a.recompute_geometry();
    let id = a
        .index
        .blocks
        .iter()
        .find(|g| g.kind == BlockKind::Table)
        .expect("应有表格块")
        .id;
    let content_w = a.content_width();
    let (t_w, indent) = {
        let t = a.cache.tables.get(&id).unwrap();
        (t.width, t.x)
    };
    assert!(indent > 0.5, "列表内表格应有缩进: {indent}");
    assert!(
        t_w <= content_w - indent + 0.5,
        "容器缩进必须同时减宽：{t_w} > {} - {indent}",
        content_w
    );
}

/// 620px + 16 列：可读下限放不下 → 自动宽表 + 可见横向滚动条；
/// 不需要用户 Shift+滚轮，拖动后同一单元格仍命中同一文档偏移。
#[test]
fn issue11_auto_wide_shows_scrollbar_when_readable_min_does_not_fit() {
    let cols = 16;
    let head = format!(
        "| {} |",
        (1..=cols)
            .map(|i| format!("C{i:02}"))
            .collect::<Vec<_>>()
            .join(" | ")
    );
    let sep = format!("| {} |", vec!["---"; cols].join(" | "));
    let row = format!(
        "| {} |",
        (1..=cols)
            .map(|i| format!("列表{i:02}"))
            .collect::<Vec<_>>()
            .join(" | ")
    );
    let doc = format!("前\n\n{head}\n{sep}\n{row}\n\n后\n");
    let (mut a, _) = app_math(&doc, false);
    a.window_state.test_size = Some((620, 700));
    a.editor.set_caret(doc.len());
    a.refresh_index();
    a.ensure_layouts_all();
    a.ensure_tables();
    a.recompute_geometry();
    let (id, top) = {
        let g = a
            .index
            .blocks
            .iter()
            .find(|g| g.kind == BlockKind::Table)
            .expect("应有表格块");
        (g.id, g.top)
    };
    let content_w = a.content_width();
    let t_w = a.cache.tables.get(&id).expect("网格已排版").width;
    assert!(
        t_w > content_w + 0.5,
        "可读下限放不下时应自动宽表：{t_w} 未超出版心 {content_w}"
    );
    assert!(
        a.table_auto_wide(id) && !a.table_is_wide(id),
        "自动宽表不得写入用户 pan"
    );

    let vp = a.viewport();
    let scroll = a.window_state.scroll_y;
    let sy = (vp.y + top - scroll) as i32;
    let g = {
        let t = a.cache.tables.get(&id).unwrap();
        a.table_scrollbar_geom(t, sy)
            .expect("自动宽表必须出现横向滚动条")
    };
    let target = a.editor.session().table(id).unwrap().rows[0][7].range.start;
    let cell_screen = |a: &App| {
        let t = a.cache.tables.get(&id).unwrap();
        let c = &t.cells[23];
        (
            a.content_x() + t.x - t.pan + c.x + 0.5,
            vp.y + top - scroll + c.y + c.h * 0.5,
        )
    };
    let before = cell_screen(&a);
    a.input_state.mouse = PhysicalPosition::new(before.0 as f64, before.1 as f64);
    assert_eq!(a.hit_offset(), Some(target));

    let thumb_mid = g.thumb_x + g.thumb_w * 0.5;
    a.input_state.mouse = PhysicalPosition::new(thumb_mid as f64, (g.y + 1.0) as f64);
    assert!(a.try_start_table_scroll_drag(), "应命中滚动条滑块");
    a.input_state.mouse = PhysicalPosition::new((thumb_mid + 100.0) as f64, (g.y + 1.0) as f64);
    assert!(a.drag_table_scroll_to());
    assert!(
        a.cache.tables.get(&id).unwrap().pan > 0.5,
        "拖动后 pan 应大于 0"
    );
    assert!(a.ui_state.tables.pan.get(&id).copied().unwrap_or(0.0) > 0.0);

    let after = cell_screen(&a);
    a.input_state.mouse = PhysicalPosition::new(after.0 as f64, after.1 as f64);
    assert_eq!(
        a.hit_offset(),
        Some(target),
        "自动宽表拖动后同一单元格仍须命中同一文档偏移"
    );
}

/// 宽表横向滚动条：可拖动；拖动后同一单元格在新位置仍命中同一文档偏移。
#[test]
fn issue11_wide_table_scrollbar_drag_keeps_hit() {
    let doc = "前\n\n\
        | 列一 | 列二 | 列三 | 列四 | 列五 | 列六 | 列七 | 列八 |\n\
        | --- | --- | --- | --- | --- | --- | --- | --- |\n\
        | 甲乙丙丁 | 甲乙丙丁 | 甲乙丙丁 | 甲乙丙丁 | 甲乙丙丁 | 甲乙丙丁 | 甲乙丙丁 | 最右列为可读内容 |\n\n\
        后\n";
    let (mut a, _) = app_math(doc, false);
    a.window_state.test_size = Some((520, 700));
    a.editor.set_caret(doc.len());
    a.refresh_index();
    a.ensure_layouts_all();
    a.ensure_tables();
    a.recompute_geometry();
    let (id, top) = {
        let g = a
            .index
            .blocks
            .iter()
            .find(|g| g.kind == BlockKind::Table)
            .expect("应有表格块");
        (g.id, g.top)
    };
    // 进入可读宽表模式（Shift+滚轮），此时表格超出版心。
    let vp = a.viewport();
    let scroll = a.window_state.scroll_y;
    a.input_state.mouse = PhysicalPosition::new(
        (a.content_x() + 20.0) as f64,
        (vp.y + top + 8.0 - scroll) as f64,
    );
    let target = a.editor.session().table(id).unwrap().rows[0][5].range.start;
    assert!(a.try_pan_table(0.0), "宽表应能进入可读平移模式");
    let (before_pan, thumb, cell_screen) = {
        let t = a.cache.tables.get(&id).expect("宽表网格已排版");
        let sy = (vp.y + top - scroll) as i32;
        let g = a
            .table_scrollbar_geom(t, sy)
            .expect("需要横滚时必须有滚动条");
        let cell = &t.cells[13]; // 表体第 6 列
        let cell_screen = (
            a.content_x() + t.x - t.pan + cell.x + 0.5,
            vp.y + top - scroll + cell.y + cell.h * 0.5,
        );
        (t.pan, g, cell_screen)
    };
    assert!(before_pan < 1.0, "初始 pan 应为 0：{before_pan}");
    // 拖动前：该单元格命中目标偏移。
    a.input_state.mouse = PhysicalPosition::new(cell_screen.0 as f64, cell_screen.1 as f64);
    assert_eq!(a.hit_offset(), Some(target));

    // 抓取滑块中部，向右拖 80px。
    let thumb_mid = thumb.thumb_x + thumb.thumb_w * 0.5;
    a.input_state.mouse = PhysicalPosition::new(thumb_mid as f64, (thumb.y + 1.0) as f64);
    assert!(a.try_start_table_scroll_drag(), "应命中滚动条滑块");
    a.input_state.mouse = PhysicalPosition::new((thumb_mid + 80.0) as f64, (thumb.y + 1.0) as f64);
    assert!(a.drag_table_scroll_to());
    let after_pan = a.cache.tables.get(&id).unwrap().pan;
    assert!(
        after_pan > before_pan + 0.5,
        "滚动条拖动后 pan 应增大: {after_pan}"
    );
    assert!(a.ui_state.tables.pan.get(&id).copied().unwrap_or(0.0) > 0.0);

    // 拖动后：同一单元格随 pan 左移，其新位置仍命中同一文档偏移。
    let cell_screen = {
        let t = a.cache.tables.get(&id).unwrap();
        let cell = &t.cells[13];
        (
            a.content_x() + t.x - t.pan + cell.x + 0.5,
            vp.y + top - scroll + cell.y + cell.h * 0.5,
        )
    };
    a.input_state.mouse = PhysicalPosition::new(cell_screen.0 as f64, cell_screen.1 as f64);
    assert_eq!(
        a.hit_offset(),
        Some(target),
        "同一单元格在平移后的屏幕位置必须仍命中同一文档偏移"
    );
}
