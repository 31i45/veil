//! 渲染与排版测试：视口感性排版 / present 管线 / 启动门控 / memo 缓存 / 性能探针 / toggle 投影。

use super::*;

#[test]
fn first_edit_lays_out_far_fewer_blocks_than_document() {
    let store = MemStore::default();
    let mut a = app(&stress_doc(), None, false, &store);
    assert!(a.index.blocks.len() >= 5005);
    let dirty = a.editor.apply(Intent::InsertText("a".into())).unwrap();
    a.after_edit(dirty);
    assert!(
        a.cache.layouts.len() < 100,
        "首次编辑只应排版视口内块，实际 {} / {} 块",
        a.cache.layouts.len(),
        a.index.blocks.len()
    );
}

// ---- 0a：超大单块布局保护 ----

#[test]
fn oversized_single_block_is_capped_with_explicit_degrade() {
    let text = "a".repeat(100_000);
    let store = MemStore::default();
    let mut a = app(&text, None, false, &store);
    assert_eq!(a.index.blocks.len(), 1, "单段应解析为单块");
    a.ensure_layouts_visible();
    let id = a.index.blocks[0].id;
    let l = &a.cache.layouts[&id];
    assert!(l.truncated, "超限块必须标记降级");
    assert_eq!(l.full_len, text.len(), "须记录原块长度");
    assert!(
        l.layout_text().len() <= MAX_BLOCK_BYTES,
        "排版文本须限制在上限内，实际 {}",
        l.layout_text().len()
    );
    assert!(l.height.is_finite() && l.height > 0.0);
    assert!(a.index.total_h.is_finite(), "文档总高须有限");
    // 源码真值不受影响：文末编辑后仍完整往返（不丢字）
    a.editor.apply(Intent::MoveDocEnd).unwrap();
    let dirty = a.editor.apply(Intent::InsertText("b".into())).unwrap();
    a.after_edit(dirty);
    a.paint();
    assert_eq!(a.editor.session().serialize(), format!("{text}b"));
    // 降级不得让光标跑出文档/视口：文末光标应在视口内，滚动不被估算高度带飞
    let vp = a.viewport();
    let (_, cy, ch) = a.caret_screen();
    assert!(
        cy >= vp.y - 1.0 && cy + ch <= vp.y + vp.h + 1.0,
        "文末光标须在视口内：cy={cy}, ch={ch}, 视口={}..{}",
        vp.y,
        vp.y + vp.h
    );
}

#[test]
fn oversized_block_tail_is_painted_after_scroll() {
    let text = "a".repeat(100_000);
    let store = MemStore::default();
    let mut a = app(&text, None, false, &store);
    a.ensure_layouts_visible();
    // 复刻真实滚轮路径：每个事件都走 on_wheel + 重绘，验证多次滚动后不出现「空白」
    let step = -3.0 * BASE_LINE_HEIGHT * a.input_state.scale;
    for _ in 0..300 {
        a.on_wheel(step);
        let _ = a.render();
    }
    let (doc_h, scroll) = (a.doc_height(), a.window_state.scroll_y);
    let vp = a.viewport();
    let bg = a.input_state.theme.bg;
    let bgpix = ((bg[0] as u32) << 16) | ((bg[1] as u32) << 8) | bg[2] as u32;
    let (sy, bh, notice_h) = {
        let g = &a.index.blocks[0];
        (
            vp.y + g.top - scroll,
            a.block_height(g),
            BASE_LINE_HEIGHT * a.input_state.scale,
        )
    };
    let frame = a.render();
    let lo = (vp.y as usize + 1) * frame.width as usize;
    let hi = ((vp.y + vp.h) as usize).min(frame.height as usize) * frame.width as usize;
    let nonbg = frame.pixels[lo..hi].iter().filter(|p| **p != bgpix).count();
    // 降级提示行：块尾单独一行，必须真的有像素（否则「明确降级」名存实亡）
    let nlo = ((sy + bh - notice_h).max(0.0) as usize).min(frame.height as usize);
    let nhi = ((sy + bh) as usize).min(frame.height as usize);
    let notice_px = if nlo < nhi {
        frame.pixels[nlo * frame.width as usize..nhi * frame.width as usize]
            .iter()
            .filter(|p| **p != bgpix)
            .count()
    } else {
        0
    };
    println!(
        "vp={}x{} doc_h={doc_h} block_h={bh} scroll={scroll} nonbg={nonbg} notice_px={notice_px}",
        vp.w, vp.h
    );
    assert!(nonbg > 1000, "滚动到底后仍须有文本像素");
    assert!(notice_px > 100, "块尾降级提示须可见，实际 {notice_px} 像素");
}

/// UTF-8 回归：中文围栏代码块在排版与逐屏滚动时都不得因字节下标非字符边界而 panic。
#[test]
fn cjk_fenced_code_scroll_to_end_does_not_panic() {
    let store = MemStore::default();
    let text = cjk_fence_long_doc();
    assert!(text.chars().count() >= 4000, "文档须 ≥4000 字");
    let mut a = app(&text, Some("doc/cjk.md"), false, &store);
    // 全量排版即触发中文围栏代码块的高亮（回归点：曾在此 panic）
    a.ensure_layouts_all();
    let code_id = a
        .index
        .blocks
        .iter()
        .find(|g| matches!(&g.kind, BlockKind::FencedCode(Some(lang)) if lang == "markdown"))
        .map(|g| g.id)
        .expect("文档须含 markdown 围栏代码块");
    assert!(
        a.cache.layouts.contains_key(&code_id),
        "中文代码块布局须存在"
    );
    a.recompute_geometry();
    let code_h0 = {
        let g = a.index.blocks.iter().find(|g| g.id == code_id).unwrap();
        a.block_height(g)
    };
    let vp = a.viewport();
    let max = chrome::max_scroll(vp, a.doc_height());
    assert!(max > 0.0, "长文档须可滚动");
    let mut covered = false;
    let mut y = 0.0f32;
    loop {
        a.window_state.scroll_y = y;
        let _ = a.render(); // 每屏触发布局/绘制
                            // 用当前帧几何判定可见性（块高在滚动中可能从实高回退估算值）
        let (code_top, code_h) = {
            let g = a.index.blocks.iter().find(|g| g.id == code_id).unwrap();
            (g.top, a.block_height(g))
        };
        let sy = vp.y + code_top - a.window_state.scroll_y;
        if sy < vp.y + vp.h && sy + code_h > vp.y {
            covered = true;
            assert!(
                a.cache.layouts.contains_key(&code_id),
                "中文代码块进入视口时布局必须存在"
            );
        }
        if a.window_state.scroll_y >= max {
            break;
        }
        y = (y + vp.h.max(1.0)).min(max);
    }
    assert!(covered, "滚动须覆盖到中文代码块所在块（块高 {code_h0}）");
}

/// 端到端源码真值（阶段 2）：CRLF + BOM 文档回车、输入、保存后，磁盘字节仍是
/// CRLF + BOM，不混入 LF、不产生孤立 `\r`——无隐式规范化。
#[test]
fn crlf_bom_edit_save_preserves_bytes() {
    let store = MemStore::default();
    let mut a = app("甲\r\n\r\n乙\r\n", Some("doc/a.md"), true, &store);
    a.paint();
    a.perform_action(Action::Doc(Intent::MoveDocEnd));
    a.route_key(Key::Enter);
    a.route_key(Key::Text("新".into()));
    a.save_now();
    let bytes = store.files.borrow()[&PathBuf::from("doc/a.md")].clone();
    assert_eq!(&bytes[..3], &[0xEF, 0xBB, 0xBF], "BOM 须保留");
    let text = String::from_utf8(bytes[3..].to_vec()).expect("UTF-8");
    assert_eq!(text, "甲\r\n\r\n乙\r\n\r\n新", "换行风格须保持 CRLF");
    // 结构校验：每个 `\n` 前必是 `\r`、每个 `\r` 后必是 `\n`（无孤立回车/LF 混入）
    let b = text.as_bytes();
    for (i, &c) in b.iter().enumerate() {
        if c == b'\n' {
            assert_eq!(
                b.get(i - 1),
                Some(&b'\r'),
                "第 {i} 字节的 \\n 前缺 \\r（混入 LF）"
            );
        }
        if c == b'\r' {
            assert_eq!(
                b.get(i + 1),
                Some(&b'\n'),
                "第 {i} 字节的 \\r 后缺 \\n（孤立回车）"
            );
        }
    }
}

/// 2c 源码真值 · 全角/半角：编辑与保存绝不转换全角/半角字符。
/// 全角拉丁/数字/标点（ＡＢＣ １２３ ！）与半角片假名（ｱｲｳ）必须原样往返。
#[test]
fn fullwidth_halfwidth_preserved_through_edit_save() {
    let store = MemStore::default();
    let doc = "全角 ＡＢＣ １２３ ！\n\n半角 ｱｲｳ abc\n";
    let mut a = app(doc, Some("doc/fw.md"), false, &store);
    a.paint();
    a.perform_action(Action::Doc(Intent::MoveDocEnd));
    a.route_key(Key::Text("尾".into()));
    a.perform_action(Action::Doc(Intent::MoveDocStart));
    a.route_key(Key::Text("首".into()));
    a.save_now();
    let bytes = store.files.borrow()[&PathBuf::from("doc/fw.md")].clone();
    let text = String::from_utf8(bytes).expect("UTF-8");
    assert_eq!(text, format!("首{doc}尾"), "全角/半角字符不得被转换");
    assert!(text.contains("ＡＢＣ １２３ ！"));
    assert!(text.contains("ｱｲｳ"));
}

/// 2c 源码真值 · 尾随空格：行尾两空格（Markdown 硬换行）与行尾 tab 是语义字节，
/// 编辑与保存不得修剪。
#[test]
fn trailing_whitespace_preserved_through_edit_save() {
    let store = MemStore::default();
    let doc = "硬换行  \n次行\ttab 尾\t\n\n末段  \n";
    let mut a = app(doc, Some("doc/ts.md"), false, &store);
    a.paint();
    a.perform_action(Action::Doc(Intent::MoveDocEnd));
    a.route_key(Key::Text("x".into()));
    a.save_now();
    let bytes = store.files.borrow()[&PathBuf::from("doc/ts.md")].clone();
    let text = String::from_utf8(bytes).expect("UTF-8");
    assert_eq!(text, format!("{doc}x"), "尾随空格/tab 不得被修剪");
}

/// 块索引派生的有序编号必须真正传入排版（非激活态显示 2./3.）。
#[test]
fn ordered_list_derived_numbers_reach_painted_layout() {
    let store = MemStore::default();
    let mut a = app("1. a\n1. b\n1. c", None, false, &store);
    a.perform_action(Action::Doc(Intent::MoveDocStart));
    a.ensure_layouts_visible();
    let texts: Vec<String> = a
        .index
        .blocks
        .iter()
        .map(|g| a.cache.layouts[&g.id].layout_text().to_string())
        .collect();
    assert_eq!(texts, ["1. a", "2. b", "3. c"], "派生编号须传入排版层");
}

/// 2c 源码真值 · 列表标记重排：有序列表回车只给新行递增标记，
/// 既有项编号（含故意乱序的 1./1./3.）绝不重排。
#[test]
fn ordered_list_markers_not_renumbered() {
    let store = MemStore::default();
    let doc = "1. a\n1. b\n3. c"; // 无尾换行：MoveDocEnd 落在「3. c」行尾
    let mut a = app(doc, Some("doc/ol.md"), false, &store);
    a.paint();
    a.perform_action(Action::Doc(Intent::MoveDocEnd));
    a.route_key(Key::Enter); // 3. c 行尾回车 → 新行 4.
    a.route_key(Key::Text("d".into()));
    a.save_now();
    let bytes = store.files.borrow()[&PathBuf::from("doc/ol.md")].clone();
    let text = String::from_utf8(bytes).expect("UTF-8");
    assert_eq!(
        text, "1. a\n1. b\n3. c\n4. d",
        "新行递增为 4.，既有 1./1./3. 标记不得重排"
    );
}

/// 真机缺陷回归（2026-10-30 ①③ + IME 候选框停在左上角）：
/// Enter 产生的新行/新段在提交前不成块，光标/preedit/IME 候选框必须锚在
/// 文本将出现的位置——软换行锚同段下一视觉行，段落分隔锚新段顶边；
/// 提交前后光标 y 不得跳变。旧行为：钳回上一段末尾 / 落视口原点。
#[test]
fn caret_before_commit_anchors_where_text_will_land() {
    let store = MemStore::default();
    let mut a = app("段一内容\n\n段二内容", None, false, &store);
    a.paint();
    a.perform_action(Action::Doc(Intent::MoveDocEnd));
    let margin = MARGIN * a.input_state.scale;
    let vp_y = a.viewport().y;

    // 软换行（Enter×1）：同段下一视觉行（块底），x 回行首
    a.route_key(Key::Enter);
    let (cx1, cy1, _) = a.caret_screen();
    let (top1, h1) = {
        let g = &a.index.blocks[1];
        (g.top, a.block_height(g))
    };
    assert_eq!(
        (cx1, cy1),
        (margin, vp_y + top1 + h1),
        "软换行光标须在同段下一行起始"
    );

    // 段落分隔（Enter×2）：新段顶边 = 块底 + 段距 + 1 个空行行高（B5）
    a.route_key(Key::Enter);
    let (cx2, cy2, _) = a.caret_screen();
    assert_eq!(
        (cx2, cy2),
        (
            margin,
            vp_y + top1 + h1 + (PARA_GAP + BASE_LINE_HEIGHT) * a.input_state.scale
        ),
        "新段光标须在段落将出现的位置"
    );

    // 提交「nihao」：落字行与 Enter2 光标同 y，preedit 所见即所得（无跳变）
    a.route_key(Key::Text("nihao".into()));
    let (_, cy3, _) = a.caret_screen();
    assert_eq!(cy2, cy3, "提交前后光标 y 不得跳变");
}

/// I1（CRLF 文档）：Enter 插入 `\r\n` 后光标必须锚到同段下一行；
/// 旧实现只把单独 `\n` 计作尾换行，CRLF 落入 caret_xy 的行尾钳制 → 光标贴在上一行。
#[test]
fn crlf_enter_caret_anchors_next_line() {
    let store = MemStore::default();
    let mut a = app("段一内容\r\n\r\n段二内容", None, false, &store);
    a.paint();
    a.perform_action(Action::Doc(Intent::MoveDocEnd));
    a.route_key(Key::Enter);
    let (_, cy, _) = a.caret_screen();
    let (top, h) = {
        let g = &a.index.blocks[1];
        (g.top, a.block_height(g))
    };
    let vp_y = a.viewport().y;
    assert_eq!(
        cy,
        vp_y + top + h,
        "CRLF 回车后光标须落在同段下一行，而非上一行行尾"
    );
}

/// 文档中部块间空隙：Enter×2 后光标锚到下一块顶边（新段将出现的位置），
/// 不得落到视口原点（旧兜底把 IME 候选框扔到左上角）。
#[test]
fn caret_in_mid_doc_gap_anchors_to_next_block_top() {
    let store = MemStore::default();
    let mut a = app("甲\n\n乙\n\n丙", None, false, &store);
    a.paint();
    // 光标移到「乙」末尾，Enter×2 产生尚未成块的空段
    let end_yi = a.index.blocks[1].range.end;
    assert!(a.editor.set_caret(end_yi));
    a.route_key(Key::Enter);
    a.route_key(Key::Enter);
    a.paint();
    let margin = MARGIN * a.input_state.scale;
    let (cx, cy, _) = a.caret_screen();
    let next_top = a.index.blocks[2].top; // 「丙」当前顶边 = 新段将出现的位置
    let vp_y = a.viewport().y;
    assert_eq!(
        (cx, cy),
        (margin, vp_y + next_top - a.window_state.scroll_y),
        "空隙光标须锚在下一块顶边"
    );
}

/// 真机缺陷复现（2026-10-30）：新段落光标定位与段落实际位置严重不一致。
/// 不变量：光标块 = 末块；光标屏幕 y 落在该块纵向跨度内；可见区间无未排版块（paint 不留空洞）。
#[test]
fn new_paragraph_caret_matches_block_position() {
    let store = MemStore::default();
    let mut a = app("段一内容\n\n段二内容", None, false, &store);
    a.paint();
    a.perform_action(Action::Doc(Intent::MoveDocEnd));
    a.route_key(Key::Enter);
    a.route_key(Key::Enter);
    a.route_key(Key::Text("n".into()));
    a.paint();
    let ci = a.caret_block_index().expect("光标须落在某块");
    let last = a.index.blocks.len() - 1;
    let (_, cy, ch) = a.caret_screen();
    let (top, bh) = {
        let g = &a.index.blocks[ci];
        (g.top, a.block_height(g))
    };
    let vp = a.viewport();
    let (sy0, sy1) = (
        vp.y + top - a.window_state.scroll_y,
        vp.y + top + bh - a.window_state.scroll_y,
    );
    println!(
        "blocks={} ci={ci} last={last} caret={} cy={cy} ch={ch} 块屏幕区间={sy0}..{sy1} scroll={}",
        a.index.blocks.len(),
        a.editor.caret(),
        a.window_state.scroll_y
    );
    assert_eq!(ci, last, "文末新段光标须在末块");
    assert!(
        cy >= sy0 - 1.0 && cy + ch <= sy1 + 1.0,
        "光标须在所属块纵向跨度内：cy={cy} ch={ch} 块区间={sy0}..{sy1}"
    );
}

/// 真机缺陷复现（冷启动链路）：中高图片块从未排版（est 22px 欠估 23×），
/// 打开后直接跳文末回车新段——scroll_to_caret 依赖的 total_h 欠估，
/// 光标/新段位置必须与收敛后几何一致，可见区间无空洞。
#[test]
fn new_paragraph_caret_with_cold_images_below() {
    let para = "这是一段足够长的中文段落，用来产生多行折行的真实排版效果。".repeat(6);
    let mut parts: Vec<String> = Vec::new();
    for i in 0..10 {
        parts.push(para.clone());
        parts.push(format!("![p{i}](p{i}.png)"));
    }
    let text = parts.join("\n\n");
    let files: Vec<(String, u32, u32)> = (0..10)
        .map(|i| (format!("doc/p{i}.png"), 600, 500))
        .collect();
    let files_ref: Vec<(&str, u32, u32)> =
        files.iter().map(|(p, w, h)| (p.as_str(), *w, *h)).collect();
    let (mut a, _calls, _files) = app_with_images(&text, Some("doc/note.md"), &files_ref);
    // 不预热：直接跳文末（复刻用户打开文档后 Ctrl+End/点击末尾）
    a.perform_action(Action::Doc(Intent::MoveDocEnd));
    a.paint();
    a.route_key(Key::Enter);
    a.route_key(Key::Enter);
    a.route_key(Key::Text("n".into()));
    a.paint();
    let vp = a.viewport();
    let ci = a.caret_block_index().expect("光标须落在某块");
    let last = a.index.blocks.len() - 1;
    let (_, cy, ch) = a.caret_screen();
    let (top, bh) = {
        let g = &a.index.blocks[ci];
        (g.top, a.block_height(g))
    };
    let scroll = a.window_state.scroll_y;
    let (sy0, sy1) = (vp.y + top - scroll, vp.y + top + bh - scroll);
    let base = a.base_key();
    let caret = a.editor.caret();
    let vr = a.visible_index_range();
    let missing: Vec<usize> = a
        .index
        .blocks
        .get(vr.clone())
        .unwrap_or(&[])
        .iter()
        .enumerate()
        .filter(|(_, g)| {
            a.cache
                .layout(g.id, &g.kind, g.range, base, caret, a.toc_layout_epoch(g))
                .is_none()
                && g.kind != BlockKind::Math
                && !(g.kind == BlockKind::Table && a.cache.table(g.id, a.table_key(g.id)).is_some())
        })
        .map(|(i, _)| vr.start + i)
        .collect();
    println!(
        "blocks={} ci={ci} last={last} cy={cy} ch={ch} 块屏幕区间={sy0}..{sy1} scroll={scroll} missing={missing:?} vr={vr:?} total_h={} vp={}..{}",
        a.index.blocks.len(),
        a.index.total_h,
        vp.y,
        vp.y + vp.h
    );
    assert_eq!(ci, last, "文末新段光标须在末块");
    assert!(
        missing.is_empty(),
        "可见区间存在未排版块（绘制为空洞）: {missing:?}"
    );
    assert!(
        cy >= sy0 - 1.0 && cy + ch <= sy1 + 1.0,
        "光标须在所属块纵向跨度内：cy={cy} ch={ch} 块区间={sy0}..{sy1}"
    );
    assert!(
        cy >= vp.y - 1.0 && cy + ch <= vp.y + vp.h + 1.0,
        "光标须在视口内可见：cy={cy} ch={ch} 视口={}..{}",
        vp.y,
        vp.y + vp.h
    );
}

/// 同缺陷的富文档变体：大图片块（est 22px vs 实高数百 px）+ CJK 段落，
/// 估算与真实高度大散度下 paint 两遍收敛仍须无空洞、光标贴块。
#[test]
fn new_paragraph_caret_matches_block_position_with_images() {
    let para = "这是一段足够长的中文段落，用来产生多行折行的真实排版效果。".repeat(6);
    let mut parts: Vec<String> = Vec::new();
    for i in 0..10 {
        parts.push(para.clone());
        if i % 2 == 0 {
            parts.push(format!("![p{i}](p{i}.png)"));
        }
    }
    let text = parts.join("\n\n");
    let files: Vec<(String, u32, u32)> = (0..5)
        .map(|i| (format!("doc/p{}.png", i * 2), 600, 500))
        .collect();
    let files_ref: Vec<(&str, u32, u32)> =
        files.iter().map(|(p, w, h)| (p.as_str(), *w, *h)).collect();
    let (mut a, _calls, _files) = app_with_images(&text, Some("doc/note.md"), &files_ref);
    a.paint();
    a.perform_action(Action::Doc(Intent::MoveDocEnd));
    a.route_key(Key::Enter);
    a.route_key(Key::Enter);
    a.route_key(Key::Text("n".into()));
    a.paint();
    let vp = a.viewport();
    let ci = a.caret_block_index().expect("光标须落在某块");
    let last = a.index.blocks.len() - 1;
    let (_, cy, ch) = a.caret_screen();
    let (top, bh) = {
        let g = &a.index.blocks[ci];
        (g.top, a.block_height(g))
    };
    let scroll = a.window_state.scroll_y;
    let (sy0, sy1) = (vp.y + top - scroll, vp.y + top + bh - scroll);
    let base = a.base_key();
    let caret = a.editor.caret();
    let vr = a.visible_index_range();
    let missing: Vec<usize> = a
        .index
        .blocks
        .get(vr.clone())
        .unwrap_or(&[])
        .iter()
        .enumerate()
        .filter(|(_, g)| {
            a.cache
                .layout(g.id, &g.kind, g.range, base, caret, a.toc_layout_epoch(g))
                .is_none()
                && g.kind != BlockKind::Math
                && !(g.kind == BlockKind::Table && a.cache.table(g.id, a.table_key(g.id)).is_some())
        })
        .map(|(i, _)| vr.start + i)
        .collect();
    println!(
        "blocks={} ci={ci} last={last} cy={cy} ch={ch} 块屏幕区间={sy0}..{sy1} scroll={scroll} missing={missing:?} vr={vr:?} total_h={}",
        a.index.blocks.len(),
        a.index.total_h
    );
    assert!(
        missing.is_empty(),
        "可见区间存在未排版块（绘制为空洞）: {missing:?}"
    );
    assert_eq!(ci, last, "文末新段光标须在末块");
    assert!(
        cy >= sy0 - 1.0 && cy + ch <= sy1 + 1.0,
        "光标须在所属块纵向跨度内：cy={cy} ch={ch} 块区间={sy0}..{sy1}"
    );
}

/// 同缺陷的高文档变体：CJK 长段（估算高度高估）+ 滚动态下末尾新段，
/// 验证 paint 两遍收敛后可见区间无空洞、几何单调、光标贴块。
#[test]
fn new_paragraph_caret_matches_block_position_tall_cjk() {
    let para = "这是一段足够长的中文段落，用来产生多行折行的真实排版效果。".repeat(8);
    let text = (0..12)
        .map(|_| para.clone())
        .collect::<Vec<_>>()
        .join("\n\n");
    let store = MemStore::default();
    let mut a = app(&text, None, false, &store);
    a.paint();
    a.perform_action(Action::Doc(Intent::MoveDocEnd));
    a.route_key(Key::Enter);
    a.route_key(Key::Enter);
    a.route_key(Key::Text("n".into()));
    a.paint();
    let vp = a.viewport();
    let ci = a.caret_block_index().expect("光标须落在某块");
    let last = a.index.blocks.len() - 1;
    let (_, cy, ch) = a.caret_screen();
    let (top, bh) = {
        let g = &a.index.blocks[ci];
        (g.top, a.block_height(g))
    };
    let scroll = a.window_state.scroll_y;
    let (sy0, sy1) = (vp.y + top - scroll, vp.y + top + bh - scroll);
    // 可见区间空洞检查：paint 已跑过两遍收敛，可见块必须全部有命中布局
    let base = a.base_key();
    let caret = a.editor.caret();
    let vr = a.visible_index_range();
    let missing: Vec<usize> = a
        .index
        .blocks
        .get(vr.clone())
        .unwrap_or(&[])
        .iter()
        .enumerate()
        .filter(|(_, g)| {
            a.cache
                .layout(g.id, &g.kind, g.range, base, caret, a.toc_layout_epoch(g))
                .is_none()
                && g.kind != BlockKind::Math
                && !(g.kind == BlockKind::Table && a.cache.table(g.id, a.table_key(g.id)).is_some())
        })
        .map(|(i, _)| vr.start + i)
        .collect();
    // 几何单调性：top 单调不减
    let mut tops_ok = true;
    for w in a.index.blocks.windows(2) {
        if w[1].top < w[0].top {
            tops_ok = false;
        }
    }
    println!(
        "blocks={} ci={ci} last={last} cy={cy} ch={ch} 块屏幕区间={sy0}..{sy1} scroll={scroll} missing={missing:?} vr={vr:?} total_h={}",
        a.index.blocks.len(),
        a.index.total_h
    );
    assert!(tops_ok, "块 top 必须单调不减");
    assert!(
        missing.is_empty(),
        "可见区间存在未排版块（绘制为空洞）: {missing:?}"
    );
    assert_eq!(ci, last, "文末新段光标须在末块");
    assert!(
        cy >= sy0 - 1.0 && cy + ch <= sy1 + 1.0,
        "光标须在所属块纵向跨度内：cy={cy} ch={ch} 块区间={sy0}..{sy1}"
    );
}

#[test]
fn oversize_area_cap_keeps_height_and_raster_in_sync() {
    let text = "a".repeat(100_000);
    let store = MemStore::default();
    let mut a = app(&text, None, false, &store);
    // 抬高缩放：内容变窄、行高变大，触发「光栅面积」上限而非字节/行数上限
    a.input_state.scale = 1.5;
    a.ensure_layouts_visible();
    let id = a.index.blocks[0].id;
    let (bh, truncated) = {
        let l = &a.cache.layouts[&id];
        (l.height, l.truncated)
    };
    assert!(truncated, "面积超限也须标记降级");
    a.paint();
    let g = &a.index.blocks[0];
    let key = a
        .cache
        .raster_key(
            g.id,
            &g.kind,
            g.range,
            a.base_key(),
            a.editor.caret(),
            a.toc_layout_epoch(g),
        )
        .expect("布局已缓存，光栅键应可用");
    let rh = a.cache.raster(g.id, key).expect("绘制后应有光栅").2;
    assert!(
        rh as f32 <= bh.ceil() + 1.0,
        "光栅高度 {rh} 不得超过块高 {bh}（否则块尾留白）"
    );
}

#[test]
fn oversized_line_count_is_capped() {
    let text = (0..2000).map(|i| format!("line {i}\n")).collect::<String>();
    assert!(text.len() < MAX_BLOCK_BYTES, "本用例只触发行数上限");
    let store = MemStore::default();
    let mut a = app(&text, None, false, &store);
    a.ensure_layouts_visible();
    let id = a.index.blocks[0].id;
    let l = &a.cache.layouts[&id];
    assert!(l.truncated, "行数超限也应降级");
    assert!(
        l.layout_text().lines().count() <= 513,
        "硬行数须受限，实际 {}",
        l.layout_text().lines().count()
    );
    assert_eq!(l.full_len, text.trim_end_matches('\n').len());
}

#[test]
fn block_below_cap_is_not_truncated() {
    let text = "a".repeat(10_000);
    let store = MemStore::default();
    let mut a = app(&text, None, false, &store);
    a.ensure_layouts_visible();
    let id = a.index.blocks[0].id;
    let l = &a.cache.layouts[&id];
    assert!(!l.truncated, "上限内块不得降级");
    assert_eq!(l.layout_text(), text, "上限内块须完整排版");
    assert_eq!(l.full_len, text.len());
}

#[test]
fn geometry_index_is_monotonic_and_consistent() {
    let store = MemStore::default();
    let mut a = app(&stress_doc(), None, false, &store);
    let dirty = a.editor.apply(Intent::InsertText("a".into())).unwrap();
    a.after_edit(dirty);
    let mut prev = f32::NEG_INFINITY;
    for g in &a.index.blocks {
        assert!(g.height > 0.0, "块高须为正: {:?}", g.id);
        assert!(g.top >= prev, "top 须单调不减: {:?}", g.id);
        prev = g.top;
    }
    let want = expected_doc_height(&a);
    assert!(
        (a.index.total_h - want).abs() < 1e-3,
        "total_h 应与累计一致: {} vs {want}",
        a.index.total_h
    );
}

#[test]
fn ensure_block_lays_out_far_block_with_real_height() {
    let store = MemStore::default();
    let mut a = app(&stress_doc(), None, false, &store);
    let id = a.index.blocks.last().unwrap().id;
    let g = geom(&a, id);
    let (kind, range) = (g.kind.clone(), g.range);
    let toc = a.toc_layout_epoch(g);
    assert!(
        a.cache
            .layout(id, &kind, range, a.base_key(), a.editor.caret(), toc)
            .is_none(),
        "远块初始不应已排版"
    );
    a.ensure_block(id);
    let h = a.cache.layouts.get(&id).expect("按需排版应写入布局").height;
    assert!(h > 0.0);
    assert_eq!(a.block_height(geom(&a, id)), h);
}

#[test]
fn memo_hit_equals_forced_relayout() {
    // 因果键命中的派生值必须与「清缓存后强制重排」逐块一致（ADR-009 决策四）
    let store = MemStore::default();
    let doc = "## 标题 **粗**\n\n段落 `code` 与 [链接](https://e.com)\n\n- 一\n- 二\n\n> 引用一\n> 引用二\n\n```rust\nfn f() {}\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n";
    let mut a = app(doc, None, false, &store);
    a.ensure_layouts_all();
    type Snap = Vec<(BlockId, f32, u64, usize, usize, String)>;
    let snapshot = |a: &App| -> Snap {
        a.index
            .blocks
            .iter()
            .map(|g| {
                let m = &a.cache.layouts[&g.id];
                (
                    g.id,
                    m.height,
                    m.key.reveal,
                    m.imgs.len(),
                    m.strikes.len(),
                    m.layout_text().to_string(),
                )
            })
            .collect()
    };
    let before = snapshot(&a);
    a.cache.layouts.clear(); // 命中路径失效：下次 ensure 必定全量重排
    a.ensure_layouts_all();
    assert_eq!(before, snapshot(&a), "命中值与重算值须逐块等价");
}

#[test]
fn render_is_idempotent_consecutive_frames_equal() {
    // ADR-009 决策五幂等不变量：同一输入连续两次 render 产出逐像素相同的帧缓冲
    let store = MemStore::default();
    let doc = "# 一 **粗**\n\n段落 `code` 与 [链接](https://e.com)\n\n- 列表一\n- 列表二\n\n> 引用一\n> 引用二\n\n```rust\nfn f() -> u32 { 42 }\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n";
    let mut a = app(doc, None, false, &store);
    a.paint(); // 第一次 render：填派生缓存（布局/光栅/chrome 标签）
    let first = a
        .window_state
        .frame
        .as_ref()
        .map(|f| f.pixels.clone())
        .expect("render 后 frame 必须存在");
    a.paint(); // 第二次 render：全部走缓存命中路径，输出须与第一次逐像素同
    let second = a
        .window_state
        .frame
        .as_ref()
        .map(|f| &f.pixels)
        .expect("同上");
    assert_eq!(
        &first[..],
        second.as_slice(),
        "连续两次 render 产出的 frame.pixels 必须逐像素相同"
    );
    let (w, h) = a.win_size();
    assert_eq!(
        first.len() as u32,
        w * h,
        "像素数 = 宽 × 高（Frame.pixels 为 Vec<u32>，每像素一个 u32）"
    );
}

#[test]
fn present_is_noop_without_surface_and_isolated_from_render() {
    // present 在无 surface（headless）时必须纯 no-op；render 的 frame 状态不变
    let store = MemStore::default();
    let doc = "# t\n\np\n";
    let mut a = app(doc, None, false, &store);
    a.paint();
    let frame_pixels_before = a
        .window_state
        .frame
        .as_ref()
        .map(|f| f.pixels.clone())
        .expect("render 产出 frame");
    assert!(
        !a.present(),
        "headless 无 surface 时 present 必须报告未上屏"
    ); // surface=None → no-op
    let frame_pixels_after = a
        .window_state
        .frame
        .as_ref()
        .map(|f| f.pixels.clone())
        .expect("present 后 frame 仍须存在");
    assert_eq!(
        frame_pixels_before, frame_pixels_after,
        "headless present 不得改变 frame 状态"
    );
}

#[test]
fn present_requires_matching_lengths() {
    // 真实失配路径（最小化后 1×1 帧 vs 旧 surface）依赖 winit/softbuffer 的真实窗口，
    // headless 下无法构造 Surface，故只能测纯判定层；present 仅在该判定为真时拷贝。
    assert!(can_present(100, 100), "长度相等应允许呈现");
    assert!(!can_present(4, 100), "帧短于 surface 应丢帧");
    assert!(!can_present(100, 4), "帧长于 surface 应丢帧");
}

#[test]
fn present_hidden_first_frame_resizes_surface_before_buffer_mut() {
    // 隐藏窗口首帧：surface 从未收到 Resized（surface_size=None），但帧/客户区已就绪。
    // 顺序必须是 resize → buffer_mut；FakeSink 在未 resize 取 buffer 时会 panic。
    let frame = test_frame(900, 700);
    let mut surface = FakeSink::new();
    let mut surface_size = None;
    assert!(present_frame(
        Some(&frame),
        false,
        Some((900, 700)),
        Some(&mut surface),
        &mut surface_size
    ));
    assert_eq!(
        surface.resizes,
        vec![(900, 700)],
        "首帧必须先 resize surface"
    );
    assert_eq!(surface.presents, 1, "resize 成功后只取一次 buffer");
    assert_eq!(
        surface_size,
        Some((900, 700)),
        "resize 成功后必须记录新尺寸，供下一次呈现判断"
    );
}

#[test]
fn present_skips_stale_frame_without_touching_surface() {
    // 渲染后窗口尺寸又变了：旧尺寸帧不得 present 到新客户区，
    // 且 stale 检查必须在 resize/buffer_mut 之前返回。
    let frame = test_frame(800, 600);
    let mut surface = FakeSink::new();
    let mut surface_size = None;
    assert!(!present_frame(
        Some(&frame),
        false,
        Some((1024, 768)),
        Some(&mut surface),
        &mut surface_size
    ));
    assert!(surface.resizes.is_empty(), "stale 帧不得 resize surface");
    assert_eq!(surface.presents, 0, "stale 帧不得取 buffer");
    assert_eq!(surface_size, None);
}

#[test]
fn present_resizes_again_after_client_size_changes() {
    // 首帧后 DPI/最大化改变客户区：旧尺寸帧会被 stale 防护丢弃；
    // 新尺寸帧到来时必须再次 resize 才能 buffer_mut。
    let frame = test_frame(1024, 768);
    let mut surface = FakeSink {
        size: Some((800, 600)),
        ..FakeSink::new()
    };
    let mut surface_size = Some((800, 600));
    assert!(present_frame(
        Some(&frame),
        false,
        Some((1024, 768)),
        Some(&mut surface),
        &mut surface_size
    ));
    assert_eq!(
        surface.resizes,
        vec![(1024, 768)],
        "尺寸变化后必须先 resize 到新帧尺寸"
    );
    assert_eq!(surface.presents, 1);
    assert_eq!(surface_size, Some((1024, 768)));
}

#[test]
fn present_resize_failure_abandons_frame_without_buffer_mut() {
    // resize 失败（如 size 超范围）时必须放弃本帧，绝不能继续 buffer_mut。
    let frame = test_frame(900, 700);
    let mut surface = FakeSink {
        resize_ok: false,
        ..FakeSink::new()
    };
    let mut surface_size = None;
    assert!(!present_frame(
        Some(&frame),
        false,
        Some((900, 700)),
        Some(&mut surface),
        &mut surface_size
    ));
    assert_eq!(surface.presents, 0, "resize 失败后不得取 buffer");
    assert_eq!(surface_size, None, "resize 失败不得记录尺寸");
}

#[test]
fn present_failure_is_reported_and_keeps_surface_size() {
    // present 本身失败（平台错误）应返回未呈现，但不改变已同步的尺寸记录。
    let frame = test_frame(900, 700);
    let mut surface = FakeSink {
        size: Some((900, 700)),
        present_ok: false,
        ..FakeSink::new()
    };
    let mut surface_size = Some((900, 700));
    assert!(!present_frame(
        Some(&frame),
        false,
        Some((900, 700)),
        Some(&mut surface),
        &mut surface_size
    ));
    assert_eq!(surface.presents, 1, "尺寸一致时应尝试一次 present");
    assert_eq!(surface_size, Some((900, 700)));
}

#[test]
fn present_skips_zero_sized_and_print_frames() {
    // 0×0 帧：surface.resize 只接受非零，必须在触碰 surface 前跳过。
    let zero = test_frame(0, 0);
    let mut surface = FakeSink::new();
    let mut surface_size = None;
    assert!(!present_frame(
        Some(&zero),
        false,
        Some((0, 0)),
        Some(&mut surface),
        &mut surface_size
    ));
    assert!(surface.resizes.is_empty());
    assert_eq!(surface.presents, 0);

    // 打印页帧尺寸 ≠ 客户区：不 present 到窗口。
    let page = test_frame(794, 1123);
    let mut page_surface = FakeSink::new();
    let mut page_size = None;
    assert!(!present_frame(
        Some(&page),
        true,
        Some((900, 700)),
        Some(&mut page_surface),
        &mut page_size
    ));
    assert!(page_surface.resizes.is_empty());
    assert_eq!(page_surface.presents, 0);
}

#[test]
fn surface_step_orders_zero_guard_before_stale_and_resize() {
    assert_eq!(
        surface_step((0, 600), Some((0, 600)), None),
        SurfaceStep::Skip,
        "0×0 帧必须先被防护，不得进入 resize"
    );
    assert_eq!(
        surface_step((800, 600), Some((0, 0)), None),
        SurfaceStep::Skip,
        "客户区 0×0（最小化）不可呈现"
    );
    assert_eq!(
        surface_step((800, 600), Some((1024, 768)), None),
        SurfaceStep::Skip,
        "帧/客户区不一致时不得触碰 surface"
    );
    assert_eq!(
        surface_step((800, 600), Some((800, 600)), None),
        SurfaceStep::Resize {
            width: 800,
            height: 600
        },
        "surface 未设尺寸时必须先 resize"
    );
    assert_eq!(
        surface_step((800, 600), Some((800, 600)), Some((1024, 768))),
        SurfaceStep::Resize {
            width: 800,
            height: 600
        },
        "surface 记录尺寸与帧不一致时必须先 resize"
    );
    assert_eq!(
        surface_step((800, 600), Some((800, 600)), Some((800, 600))),
        SurfaceStep::Draw,
        "尺寸一致才允许直接取 buffer"
    );
    assert_eq!(
        surface_step((800, 600), None, None),
        SurfaceStep::Resize {
            width: 800,
            height: 600
        },
        "无窗口（headless）不做 stale 检查，但仍需先设尺寸"
    );
}

#[test]
fn startup_without_surface_reveals_instead_of_staying_hidden() {
    // 首帧兜底（Task 2.3）：呈现面不可用时也必须结束隐藏阶段，
    // 由主题底色窗口兜底，不能永久隐藏或静默退出。
    let store = MemStore::default();
    let mut a = app("# t\n\np\n", None, false, &store);
    a.window_state.visible = false;
    a.window_state.startup = Some(StartupGate::new(Instant::now()));
    a.try_show_first_frame();
    assert!(
        a.window_state.startup.is_none(),
        "surface 不可用时必须结束隐藏阶段并显示窗口"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Task 5 首帧显示状态机：窗口隐藏创建 → 首帧成功只显示一次；
// 呈现失败由 1 秒保护时限兜底显示；隐藏阶段尺寸变化重做首帧但不重复显示。
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn startup_gate_reveals_once_after_first_present() {
    let t0 = Instant::now();
    let mut gate = StartupGate::new(t0);
    assert!(!gate.revealed(), "创建时窗口必须保持隐藏");
    assert!(gate.on_present(true), "首帧成功后应请求显示窗口");
    assert!(gate.revealed());
    assert!(
        !gate.on_present(true),
        "后续首帧（尺寸变化重做）不得重复显示"
    );
    assert!(
        !gate.on_timeout(t0 + Duration::from_secs(5)),
        "已显示后超时不得再次触发显示"
    );
}

#[test]
fn startup_gate_failed_present_waits_for_timeout() {
    let t0 = Instant::now();
    let mut gate = StartupGate::new(t0);
    assert!(!gate.on_present(false), "呈现失败时不得显示窗口");
    assert!(
        !gate.on_timeout(t0 + Duration::from_millis(999)),
        "未到 1 秒保护时限不得显示"
    );
    assert!(!gate.revealed());
    assert!(
        gate.on_timeout(t0 + Duration::from_secs(1)),
        "保护时限到达必须兜底显示"
    );
    assert!(gate.revealed());
    assert!(!gate.on_timeout(t0 + Duration::from_secs(2)));
}

#[test]
fn startup_gate_deadline_is_one_second() {
    let t0 = Instant::now();
    let gate = StartupGate::new(t0);
    assert_eq!(gate.deadline(), t0 + Duration::from_secs(1));
}

#[test]
fn startup_gate_resize_retry_can_succeed_after_failure() {
    // 首帧失败（如客户区尚为 0×0）后，尺寸事件带来的重试成功 → 只显示一次。
    let t0 = Instant::now();
    let mut gate = StartupGate::new(t0);
    assert!(!gate.on_present(false));
    assert!(gate.on_present(true), "尺寸就绪后的重试应显示窗口");
    assert!(!gate.on_present(true), "之后的正常帧不得再次显示");
    assert!(!gate.on_timeout(t0 + Duration::from_secs(3)));
}

#[test]
fn headless_app_has_no_startup_gate_and_present_fails() {
    // 无窗口（headless）路径不登记首帧守卫：tick/next_wake 不引入额外唤醒。
    let store = MemStore::default();
    let mut a = app("# t\n\np\n", None, false, &store);
    assert!(a.window_state.startup.is_none());
    let _ = a.render();
    assert!(
        !a.present(),
        "无 surface 时 present 报告失败，由调用方决定兜底"
    );
}

#[test]
fn startup_gate_registers_deadline_in_next_wake() {
    // 接线测试（Review A F2）：隐藏首帧阶段的唯一唤醒源是 next_wake 登记的
    // 1 秒保护时限；若该接线回退，纯状态机测试仍会全绿而窗口可能永久不可见。
    let store = MemStore::default();
    let mut a = app("# t\n\np\n", None, false, &store);
    a.window_state.visible = false;
    let t0 = Instant::now();
    a.window_state.startup = Some(StartupGate::new(t0));
    let next = a
        .next_wake()
        .expect("首帧隐藏阶段必须登记兜底唤醒，否则窗口可能永久不可见");
    assert_eq!(
        next,
        t0 + Duration::from_secs(1),
        "startup 阶段 next_wake 必须包含 1s 保护时限"
    );
}

#[test]
fn startup_timeout_tick_settles_gate_once() {
    // 接线测试（Review A F2）：tick 在保护时限到达后必须结束隐藏阶段。
    // headless 无窗口，`set_visible` 安全跳过；真实 GUI 显示为 NOT RUN。
    let store = MemStore::default();
    let mut a = app("# t\n\np\n", None, false, &store);
    a.window_state.visible = false;
    let t0 = Instant::now();
    a.window_state.startup = Some(StartupGate::new(t0));
    a.tick(t0 + Duration::from_millis(999));
    assert!(
        a.window_state.startup.is_some(),
        "未到 1 秒保护时限必须保持隐藏"
    );
    a.tick(t0 + Duration::from_secs(1));
    assert!(
        a.window_state.startup.is_none(),
        "保护时限到达必须结束隐藏阶段（超时兜底显示）"
    );
    // 只显示一次：后续 tick 不得重新进入启动态或再次登记兜底唤醒。
    a.tick(t0 + Duration::from_secs(5));
    assert!(a.window_state.startup.is_none());
    assert!(
        a.next_wake().is_none(),
        "首帧结束后（不可见且无其它唤醒源）不得残留周期唤醒"
    );
}

#[test]
fn theme_switch_applies_current_theme_bg_to_window_effects() {
    // 接线测试（Review A F2）：apply_window_effects 必须以「当前主题」底色
    // 调用 WindowEffects::set_background；HWND 获取层之外的部分在此验证。
    let store = MemStore::default();
    let mut a = app("# t\n\np\n", None, false, &store);
    let rec = RecordingEffects::default();
    a.ports.effects = Box::new(rec.clone());
    let native = NativeWindow { hwnd: 1 };

    a.apply_window_effects_to(native);
    let light_rgb = window_erase_rgb(a.input_state.theme.bg);
    assert_eq!(
        rec.backgrounds.borrow().last(),
        Some(&light_rgb),
        "首次应用必须使用当前主题底色（RGB，无 alpha）"
    );

    a.set_theme_mode(ThemeMode::Dark);
    a.apply_window_effects_to(native);
    let dark_rgb = window_erase_rgb(a.input_state.theme.bg);
    assert_ne!(dark_rgb, light_rgb, "深色主题底色应与浅色不同");
    assert_eq!(
        rec.backgrounds.borrow().last(),
        Some(&dark_rgb),
        "主题切换后必须使用新主题底色更新窗口擦除色"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 性质级测试（ADR-009 S6 阶段 B）：验证 Memo 因果键对任意编辑序列的正确性。
// 两个性质：
//   1. memo_hit_equals_forced_after_any_sequence：任意编辑序列（0..50 次）+
//      主题/DPI 切换（0..20 次）后，memo 命中值恒等于 clear 后强制重算值
//   2. render_idempotent_after_any_sequence：任意编辑序列后连续两次 render
//      逐元素像素相等
// 使用 proptest（dev-dependency，只进测试二进制）：自动 shrinking 找最小反例，
// 发现边界 bug 后自动缩减为可钉死的永久单元测试。
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn heading_reveal_invalidates_after_block_shift_and_caret_leave() {
    let store = MemStore::default();
    let mut a = app("# 标题\n\n尾\n", None, false, &store);
    // caret 0 在标题块内 → 显示源码 `# 标题`
    let dirty = a.editor.apply(Intent::Newline).unwrap_or_default();
    a.after_edit(dirty); // 块整体下移，id 保留
    let dirty = a.editor.apply(Intent::MoveDocStart).unwrap_or_default();
    a.after_edit(dirty); // caret 回到块外 → 标记应消隐
    let stale = memo_matches_forced_after(&mut a);
    assert!(
        stale.is_empty(),
        "标题块迁移后不得残留旧 Reveal 布局: {stale:?}"
    );
    // caret 在块外（0）→ 隐藏 `#`
    let heading = a
        .index
        .blocks
        .iter()
        .find(|g| matches!(g.kind, BlockKind::AtxHeading(_)))
        .map(|g| g.id)
        .expect("文档含标题块");
    assert_eq!(a.cache.layouts[&heading].layout_text(), "标题");
}

#[test]
fn fenced_reveal_invalidates_after_block_shift_and_caret_leave() {
    let store = MemStore::default();
    let mut a = app("```\nx\n```\n\n尾\n", None, false, &store);
    let dirty = a.editor.apply(Intent::Newline).unwrap_or_default();
    a.after_edit(dirty);
    let dirty = a.editor.apply(Intent::MoveDocStart).unwrap_or_default();
    a.after_edit(dirty);
    let stale = memo_matches_forced_after(&mut a);
    assert!(
        stale.is_empty(),
        "围栏块迁移后不得残留旧 Reveal 布局: {stale:?}"
    );
}

/// 主题/DPI 切换变体（5 种）：主题模式切换（set_theme_mode 内部递增 revs.theme）
/// 与 scale 切换（base_key.scale_q 自然失配）。
fn apply_toggle(a: &mut App, kind: u8) {
    use crate::theme::ThemeMode;
    match kind % 5 {
        0 => a.set_theme_mode(ThemeMode::Light),
        1 => a.set_theme_mode(ThemeMode::Dark),
        2 => a.set_theme_mode(ThemeMode::System),
        3 => a.input_state.scale = 1.25,
        _ => a.input_state.scale = 1.75,
    }
}

proptest::prop_compose! {
    fn arbitrary_intent()(
        // InsertText 需要字符串：随机 1~5 个 a-z 字符（避免 markdown 语法复杂化解析）
        text in proptest::collection::vec(proptest::char::range('a', 'z'), 1..=5),
        // 选择哪个变体：0..=13 覆盖 14 个 Intent 变体
        var in 0u8..=13,
    ) -> Intent {
        let text: String = text.into_iter().collect();
        match var {
            0 => Intent::InsertText(text),
            1 => Intent::DeleteBackward,
            2 => Intent::DeleteForward,
            3 => Intent::Newline,
            4 => Intent::MoveLeft,
            5 => Intent::MoveRight,
            6 => Intent::MoveDocStart,
            7 => Intent::MoveDocEnd,
            8 => Intent::SelectLeft,
            9 => Intent::SelectRight,
            10 => Intent::SelectDocStart,
            11 => Intent::SelectDocEnd,
            12 => Intent::Undo,
            _ => Intent::Redo,
        }
    }
}

proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config {
        // 标准 100 组对性质测试足够；debug build 22s 跑通第一条，
        // 若边界 bug 需要更高覆盖可调大
        cases: 100,
        ..proptest::test_runner::Config::default()
    })]

    #[test]
    fn memo_hit_equals_forced_after_any_sequence(
        edits in proptest::collection::vec(arbitrary_intent(), 0..=50),
        toggles in proptest::collection::vec(0u8..=4, 0..=20),
    ) {
        // 性质一：同一 App 上先让 memo 自然命中并取快照，再清空派生缓存强制重排取第二份，
        // 两次的 alive BlockId 集合与逐块 (height, reveal, layout_text) 必须完全一致。
        // 用单 App 而非双 App 对比：撤销分组按真实时钟 500ms 静默窗合并（History::record），
        // 两份实例的耗时差会让「连续单字符删除」有时合并有时不合并，Undo 后文本分叉——
        // 那是测试对时钟的依赖，不是产品不变量。
        let base_doc = "# 标题\n\n段落 **粗** 与 `code` 和 [链接](https://e.com)\n\n- 列表一\n- 列表二\n\n> 引用一\n> 引用二\n\n```rust\nfn f() -> u32 { 42 }\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n";
        let store = MemStore::default();
        let mut a = app(base_doc, None, false, &store);

        for intent in &edits {
            let dirty = a.editor.apply(intent.clone()).unwrap_or_default();
            a.after_edit(dirty);
        }
        for t in &toggles {
            apply_toggle(&mut a, *t);
        }

        a.ensure_layouts_all();
        let snap_memo = snapshot_layouts(&a);

        a.cache.layouts.clear();
        a.cache.rasters.clear();
        a.cache.math_imgs.clear();
        a.cache.math_failed.clear();
        a.cache.inline_math.clear();
        a.cache.inline_failed.clear();
        a.cache.inline_images.clear();
        a.cache.image_failed.clear();
        a.cache.tables.clear();
        a.ensure_layouts_all();
        let snap_forced = snapshot_layouts(&a);

        let mut ids_memo: Vec<BlockId> = snap_memo.keys().copied().collect();
        let mut ids_forced: Vec<BlockId> = snap_forced.keys().copied().collect();
        ids_memo.sort();
        ids_forced.sort();
        proptest::prop_assert_eq!(
            ids_memo, ids_forced,
            "alive BlockId 集合不一致"
        );

        for id in snap_memo.keys() {
            if snap_memo.get(id) != snap_forced.get(id) {
                let g = a.index.blocks.iter().find(|g| g.id == *id);
                let src = g.map(|g| {
                    a
                        .editor
                        .session()
                        .text_slice(g.range)
                        .unwrap_or_default()
                });
                let g2 = a.index.blocks.iter().find(|g| g.id == *id);
                let src2 = g2.map(|g| {
                    a
                        .editor
                        .session()
                        .text_slice(g.range)
                        .unwrap_or_default()
                });
                let info = format!(
                    "id={id:?} memo:kind={:?} range={:?} 源文本={src:?} | forced:kind={:?} range={:?} 源文本={src2:?}",
                    g.map(|g| g.kind.clone()),
                    g.map(|g| g.range),
                    g2.map(|g| g.kind.clone()),
                    g2.map(|g| g.range)
                );
                proptest::prop_assert_eq!(
                    snap_memo.get(id),
                    snap_forced.get(id),
                    "memo 命中值 ≠ 强制重算值: {}",
                    info
                );
            }
        }
    }

    #[test]
    fn render_idempotent_after_any_sequence(
        edits in proptest::collection::vec(arbitrary_intent(), 0..=30),
    ) {
        // 性质二：任意编辑序列后连续两次 paint() 产出逐元素像素相同的帧缓冲。
        // 幂等性是渲染纯化（S4）的核心不变量。
        let base_doc = "# 标题\n\n段落 **粗** 与 `code` 和 [链接](https://e.com)\n\n- 列表一\n- 列表二\n\n> 引用一\n> 引用二\n\n```rust\nfn f() -> u32 { 42 }\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n";
        let store = MemStore::default();
        let mut a = app(base_doc, None, false, &store);

        for intent in &edits {
            let dirty = a.editor.apply(intent.clone()).unwrap_or_default();
            a.after_edit(dirty);
        }

        a.paint();
        let first = a.window_state.frame.as_ref().unwrap().pixels.clone();
        a.paint();
        let second = a.window_state.frame.as_ref().unwrap().pixels.clone();
        proptest::prop_assert_eq!(
            first, second,
            "render 幂等性被破坏"
        );
    }

    #[test]
    fn undo_to_initial_is_byte_identical_after_any_sequence(
        edits in proptest::collection::vec(arbitrary_intent(), 0..=50),
    ) {
        // 源码真值往返 fuzz（2c 切片）：任何编辑序列后逐步撤销到底，
        // 必须字节级回到初始文档。任何隐式规范化（全角/半角转换、尾随空格
        // 修剪、列表编号重排、换行风格改写）都不会进入撤销栈——一旦发生，
        // 撤销到底必残留差异而被本性质捕获。
        let seed = "首段 ＡＢＣ １２３ ！ ｱｲｳ  \n\n1. 甲\n1. 乙\n3. 丙\n\n> 引用  \n\n```\ncode 行  \n```\n\n末段 tab\t\n";
        let store = MemStore::default();
        let mut a = app(seed, None, false, &store);

        for intent in &edits {
            let dirty = a.editor.apply(intent.clone()).unwrap_or_default();
            a.after_edit(dirty);
        }
        // 逐步撤销到底（空栈 Undo 返回 None，循环必然终止）
        while a.editor.apply(Intent::Undo).is_some() {}
        let restored = a.editor.session().serialize();
        proptest::prop_assert_eq!(
            restored.as_str(),
            seed,
            "撤销到底后须字节级还原初始文档（隐式规范化检测）"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 持久化状态机性质测试（ADR-009 S6 阶段 B 续）：外部变更 × 保存 × 自动保存。
// 外部磁盘事件由不可控进程驱动，是全项目唯一「输入不可穷举」的状态机，故用 proptest
// 钉死六条用户可观察不变量。断言只观察 status()/hold() 与磁盘副作用，
// 不观察 pending/backup/missing 等内部事实（内部表示可变，契约不变）。
// ─────────────────────────────────────────────────────────────────────────────

fn persist_projection(a: &App, store: &MemStore) -> PersistProjection {
    (
        a.external_state(),
        a.persist_state.persist.hold(),
        a.persist_state.persist.pending().map(<[u8]>::to_vec),
        a.persist_state.persist.missing(),
        a.persist_state.persist.backed_up(),
        store.files.borrow().clone(),
        a.doc.baseline.clone(),
    )
}

proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config {
        cases: 100,
        ..proptest::test_runner::Config::default()
    })]

    #[test]
    fn persist_state_machine_invariants_hold_after_any_sequence(
        events in proptest::collection::vec(persist_event(), 1..=30),
    ) {
        let store = MemStore::default();
        let watch = FakeWatch::default();
        let base = "ab\n";
        disk(&store, "a.md", base.as_bytes());
        let mut a = app_watch(base, Some("a.md"), &store, &watch);
        let mut clock = Instant::now();
        let mut prev_was_poll = false;

        for (i, ev) in events.iter().enumerate() {
            let disk_before = store.files.borrow().get(Path::new("a.md")).cloned();
            let baseline_before = a.doc.baseline.clone();
            let conflicts_before = conflict_files(&store);
            let facts_before = persist_projection(&a, &store);
            apply_persist_event(&mut a, &store, &watch, &mut clock, ev);

            let p = &a.persist_state.persist;

            // I1：status 优先级投影自洽（Changed > BackedUp > Missing > None）
            match a.external_state() {
                ExternalState::Changed => proptest::prop_assert!(
                    p.pending().is_some(),
                    "step {} ({:?}): Changed 但无待备份版本", i, ev
                ),
                ExternalState::BackedUp => proptest::prop_assert!(
                    p.pending().is_none() && p.backed_up(),
                    "step {} ({:?}): BackedUp 但事实不符", i, ev
                ),
                ExternalState::Missing => proptest::prop_assert!(
                    p.pending().is_none() && !p.backed_up() && p.missing(),
                    "step {} ({:?}): Missing 但事实不符", i, ev
                ),
                ExternalState::None => proptest::prop_assert!(
                    p.pending().is_none() && !p.backed_up() && !p.missing(),
                    "step {} ({:?}): None 但仍有外部事实", i, ev
                ),
            }

            // I2：自动保存挂起必有原因（否则用户会看到「不保存」却无任何提示）
            if p.hold() {
                proptest::prop_assert!(
                    p.pending().is_some() || p.backed_up(),
                    "step {} ({:?}): 挂起自动保存但无外部状态原因", i, ev
                );
            }

            // I3：自动保存绝不覆盖外部版本（磁盘存在且≠基线时，磁盘必须原样保留）
            if matches!(ev, PersistEvent::AutoSaveTick) {
                if let Some(d) = &disk_before {
                    if *d != baseline_before {
                        let after = store.files.borrow().get(Path::new("a.md")).cloned();
                        proptest::prop_assert_eq!(
                            after, disk_before.clone(),
                            "step {}: 自动保存覆盖了外部版本", i
                        );
                    }
                }
            }

            // I4：手工保存成功 ⇒ 磁盘==基线、缺失态清除、挂起解除
            if matches!(ev, PersistEvent::ManualSave) {
                let disk_after = store.files.borrow().get(Path::new("a.md")).cloned();
                proptest::prop_assert_eq!(
                    disk_after.as_deref(), Some(a.doc.baseline.as_slice()),
                    "step {}: 手工保存成功后磁盘≠基线", i
                );
                proptest::prop_assert!(!p.missing(), "step {}: 保存成功但仍是缺失态", i);
                proptest::prop_assert!(!p.hold(), "step {}: 保存成功但自动保存仍挂起", i);
            }

            // I5：外部版本不丢失（被覆盖的外部字节必落冲突副本），且已写副本永不被改写
            if matches!(ev, PersistEvent::ManualSave) {
                let conflicts_after = conflict_files(&store);
                for (cp, cb) in &conflicts_before {
                    proptest::prop_assert!(
                        conflicts_after.iter().any(|(p2, b2)| p2 == cp && b2 == cb),
                        "step {}: 已写冲突副本被改写或丢失 {}", i, cp.display()
                    );
                }
                if let Some(d) = &disk_before {
                    if *d != baseline_before {
                        proptest::prop_assert!(
                            conflicts_after.iter().any(|(_, b)| b == d),
                            "step {}: 被覆盖的外部版本未落入任何冲突副本", i
                        );
                    }
                }
            }

            // I6：检测幂等（连续两次 Poll 且磁盘未变时，状态事实不变）
            if matches!(ev, PersistEvent::Poll) && prev_was_poll {
                let facts_after = persist_projection(&a, &store);
                proptest::prop_assert_eq!(
                    facts_after, facts_before,
                    "step {}: 连续轮询状态不幂等", i
                );
            }

            prev_was_poll = matches!(ev, PersistEvent::Poll);
        }
    }

    #[test]
    fn persist_replay_is_deterministic(
        events in proptest::collection::vec(persist_event(), 0..=20),
    ) {
        // 同一事件序列重放两次必须得到完全相同的可观察投影（含磁盘全量内容与基线）：
        // 持久化编排不得依赖 HashMap 顺序、时钟漂移或残留临时状态。
        let run = |events: &[PersistEvent]| {
            let store = MemStore::default();
            let watch = FakeWatch::default();
            let base = "ab\n";
            disk(&store, "a.md", base.as_bytes());
            let mut a = app_watch(base, Some("a.md"), &store, &watch);
            let mut clock = Instant::now();
            for ev in events {
                apply_persist_event(&mut a, &store, &watch, &mut clock, ev);
            }
            persist_projection(&a, &store)
        };
        proptest::prop_assert_eq!(run(&events), run(&events));
    }
}

#[test]
fn move_doc_end_ensures_caret_block_layout() {
    let store = MemStore::default();
    let mut a = app(&stress_doc(), None, false, &store);
    let dirty = a.editor.apply(Intent::MoveDocEnd).unwrap();
    a.after_edit(dirty);
    let id = a
        .caret_block_index()
        .map(|i| a.index.blocks[i].id)
        .expect("文末应落在某块内");
    assert!(a.cache.layouts.contains_key(&id), "光标块须已排版");
    let (x, y, h) = a.caret_screen();
    assert!(x.is_finite() && y.is_finite() && h > 0.0);
}

#[test]
fn estimated_height_positive_for_all_block_kinds() {
    let doc = "# h\n\np\n\n```\ncode\n```\n\n$$\nx\n$$\n\n| a | b |\n| --- | --- |\n| c | d |\n\n- li\n\n> q\n\n---\n";
    let store = MemStore::default();
    let a = app(doc, None, false, &store);
    assert!(a.index.blocks.len() >= 8, "覆盖全部块类型");
    for g in &a.index.blocks {
        assert!(g.est_h > 0.0, "估算高度须为正: {:?}", g.kind);
    }
}

/// 真实路径性能探针（默认忽略，不参与常规门禁）：
/// cargo test -p veil-ui --release -- --ignored --nocapture perf_keystroke_path
/// 度量 apply + after_edit + paint 的端到端延迟；bench_typing 只测单点 API，
/// 无法反映视图层真实装配路径，故以此探针作为 P1/P2 的验收依据。
#[test]
#[ignore = "性能探针，不参与常规门禁"]
fn perf_keystroke_path() {
    let text = stress_doc();
    let store = MemStore::default();
    let mut a = app(&text, None, false, &store);
    a.editor.apply(Intent::MoveDocStart).unwrap();
    for _ in 0..5 {
        a.editor.apply(Intent::MoveRight).unwrap();
    }
    a.paint(); // 预热：首帧含全量排版
    let mut samples = Vec::with_capacity(300);
    for _ in 0..300 {
        let t = Instant::now();
        let dirty = a.editor.apply(Intent::InsertText("a".into())).unwrap();
        a.after_edit(dirty);
        a.paint();
        samples.push(t.elapsed().as_secs_f64() * 1e3);
    }
    samples.sort_by(|x, y| x.partial_cmp(y).unwrap());
    let pick = |q: f64| {
        let i = ((samples.len() as f64 * q).ceil() as usize)
            .saturating_sub(1)
            .min(samples.len() - 1);
        samples[i]
    };
    println!("perf_blocks={}", a.index.blocks.len());
    println!("perf_keystroke_ms_p50={:.3}", pick(0.5));
    println!("perf_keystroke_ms_p95={:.3}", pick(0.95));
    println!("perf_keystroke_ms_max={:.3}", samples[samples.len() - 1]);
}

/// 分项归因探针（默认忽略）：三档文档规模下把每次击键拆成 5 段独立耗时。
/// 全部走真实代码路径：Editor::apply → Session::apply；refresh_index；脏块
/// ensure_block（真实缓存+排版路径）；LayoutEngine::rasterize；render 帧合成
///（headless 亦可运行，present 无 surface 时为 no-op，不纳入计时）。
/// cargo test -p veil-ui --release --lib -- --ignored --nocapture perf_keystroke_attribution
#[test]
#[ignore = "性能探针，不参与常规门禁"]
fn perf_keystroke_attribution() {
    const SAMPLES: usize = 300;
    println!(
        "# segments: core_apply_reparse=Editor::apply->Session::apply; \
         index_rebuild=App::refresh_index（含 recompute_geometry + refresh_stats）; \
         dirty_layout=App::ensure_block（真实缓存查询+排版）; \
         rasterize=LayoutEngine::rasterize（脏块）; \
         frame_compose=App::render（真实 headless 帧合成，present 无 surface 不测）"
    );
    println!(
        "doc_blocks,core_apply_reparse_p50_us,core_apply_reparse_p95_us,\
         index_rebuild_p50_us,index_rebuild_p95_us,\
         dirty_layout_p50_us,dirty_layout_p95_us,\
         rasterize_p50_us,rasterize_p95_us,\
         frame_compose_p50_us,frame_compose_p95_us,\
         total_p95_us,index_share_pct"
    );

    for iters in [286usize, 715, 1429, 2857, 5715] {
        let text = stress_doc_n(iters);
        let store = MemStore::default();
        let mut a = app(&text, None, false, &store);
        a.editor.apply(Intent::MoveDocStart).unwrap();
        for _ in 0..5 {
            a.editor.apply(Intent::MoveRight).unwrap();
        }
        a.paint(); // 预热：首帧完成可见区布局与光栅，排除字体首次加载成本

        let block_count = a.index.blocks.len();
        let mut core = Vec::with_capacity(SAMPLES);
        let mut index = Vec::with_capacity(SAMPLES);
        let mut layout = Vec::with_capacity(SAMPLES);
        let mut raster = Vec::with_capacity(SAMPLES);
        let mut compose = Vec::with_capacity(SAMPLES);

        for _ in 0..SAMPLES {
            let t = Instant::now();
            let dirty = a
                .editor
                .apply(Intent::InsertText("a".into()))
                .expect("插入点恒合法");
            core.push(t.elapsed().as_secs_f64() * 1e6);

            let t = Instant::now();
            a.refresh_index();
            index.push(t.elapsed().as_secs_f64() * 1e6);

            // 脏块排版：生产路径 ensure_block（命中则只比键，未命中才取词/行内并排版）
            let t = Instant::now();
            for &id in &dirty {
                a.ensure_block(id);
            }
            layout.push(t.elapsed().as_secs_f64() * 1e6);

            // 脏块光栅：与 paint.rs 的惰性光栅同一路径（布局键比对 + rasterize + 回填）
            let base = a.base_key();
            let caret = a.effective_caret();
            let fg = a.input_state.theme.fg;
            let t = Instant::now();
            for &id in &dirty {
                let Some(g) = a.index.blocks.iter().find(|g| g.id == id) else {
                    continue;
                };
                let kind = g.kind.clone();
                let range = g.range;
                let Some(lk) =
                    a.cache
                        .raster_key(id, &kind, range, base, caret, a.toc_layout_epoch(g))
                else {
                    continue;
                };
                if a.cache.raster(id, lk).is_none() {
                    let r = a.engine.rasterize(&a.cache.layouts[&id].value, fg);
                    a.cache.rasters.insert(id, Memo { key: lk, value: r });
                }
            }
            raster.push(t.elapsed().as_secs_f64() * 1e6);

            let t = Instant::now();
            let _ = a.render();
            compose.push(t.elapsed().as_secs_f64() * 1e6);
        }

        let pick = |v: &mut [f64], q: f64| {
            v.sort_by(|x, y| x.partial_cmp(y).unwrap());
            let i = ((v.len() as f64 * q).ceil() as usize)
                .saturating_sub(1)
                .min(v.len() - 1);
            v[i]
        };
        let (core_p50, core_p95) = (pick(&mut core, 0.5), pick(&mut core, 0.95));
        let (index_p50, index_p95) = (pick(&mut index, 0.5), pick(&mut index, 0.95));
        let (layout_p50, layout_p95) = (pick(&mut layout, 0.5), pick(&mut layout, 0.95));
        let (raster_p50, raster_p95) = (pick(&mut raster, 0.5), pick(&mut raster, 0.95));
        let (compose_p50, compose_p95) = (pick(&mut compose, 0.5), pick(&mut compose, 0.95));
        let total_p95 = core_p95 + index_p95 + layout_p95 + raster_p95 + compose_p95;
        println!(
            "{block_count},{core_p50:.2},{core_p95:.2},\
             {index_p50:.2},{index_p95:.2},\
             {layout_p50:.2},{layout_p95:.2},\
             {raster_p50:.2},{raster_p95:.2},\
             {compose_p50:.2},{compose_p95:.2},\
             {total_p95:.2},{:.1}",
            index_p95 / total_p95 * 100.0
        );
    }
}

/// `index_rebuild` 子段探针（默认忽略）：按生产顺序逐步调用 refresh_index 的 4 个真实步骤。
/// 生产 `refresh_index` 自身不含任何 `Instant::now()`；探针在外部逐步计时，不重复实现逻辑。
/// cargo test -p veil-ui --release --lib -- --ignored --nocapture perf_index_subsegments
#[test]
#[ignore = "性能探针，不参与常规门禁"]
fn perf_index_subsegments() {
    const SAMPLES: usize = 300;
    println!(
        "# index subsegments: blocks_enum=App::blocks_enum; geom_fill=App::geom_fill;\
         recompute=App::recompute_geometry; stats=App::refresh_stats"
    );
    println!(
        "doc_blocks,blocks_enum_p50_us,blocks_enum_p95_us,\
         geom_fill_p50_us,geom_fill_p95_us,\
         recompute_p50_us,recompute_p95_us,\
         stats_p50_us,stats_p95_us,\
         index_sum_p50_us,index_sum_p95_us,\
         blocks_enum_share_pct,geom_fill_share_pct,recompute_share_pct,stats_share_pct"
    );

    for iters in [286usize, 715, 1429, 2857, 5715] {
        let text = stress_doc_n(iters);
        let store = MemStore::default();
        let mut a = app(&text, None, false, &store);
        a.editor.apply(Intent::MoveDocStart).unwrap();
        for _ in 0..5 {
            a.editor.apply(Intent::MoveRight).unwrap();
        }
        a.paint();

        let block_count = a.index.blocks.len();
        let mut blocks_enum = Vec::with_capacity(SAMPLES);
        let mut geom_fill = Vec::with_capacity(SAMPLES);
        let mut recompute = Vec::with_capacity(SAMPLES);
        let mut stats = Vec::with_capacity(SAMPLES);

        for _ in 0..SAMPLES {
            // 编辑本身不计入 index 子段，只为制造真实脏状态
            let _dirty = a
                .editor
                .apply(Intent::InsertText("a".into()))
                .expect("插入点恒合法");

            let t = Instant::now();
            let blocks = a.blocks_enum();
            blocks_enum.push(t.elapsed().as_secs_f64() * 1e6);

            let t = Instant::now();
            a.geom_fill(blocks);
            geom_fill.push(t.elapsed().as_secs_f64() * 1e6);

            let t = Instant::now();
            a.recompute_geometry();
            recompute.push(t.elapsed().as_secs_f64() * 1e6);

            let t = Instant::now();
            a.refresh_stats();
            stats.push(t.elapsed().as_secs_f64() * 1e6);
        }

        let pick = |v: &mut [f64], q: f64| {
            v.sort_by(|x, y| x.partial_cmp(y).unwrap());
            let i = ((v.len() as f64 * q).ceil() as usize)
                .saturating_sub(1)
                .min(v.len() - 1);
            v[i]
        };
        let (be_p50, be_p95) = (pick(&mut blocks_enum, 0.5), pick(&mut blocks_enum, 0.95));
        let (gf_p50, gf_p95) = (pick(&mut geom_fill, 0.5), pick(&mut geom_fill, 0.95));
        let (rc_p50, rc_p95) = (pick(&mut recompute, 0.5), pick(&mut recompute, 0.95));
        let (st_p50, st_p95) = (pick(&mut stats, 0.5), pick(&mut stats, 0.95));
        let sum_p50 = be_p50 + gf_p50 + rc_p50 + st_p50;
        let sum_p95 = be_p95 + gf_p95 + rc_p95 + st_p95;
        println!(
            "{block_count},{be_p50:.2},{be_p95:.2},\
             {gf_p50:.2},{gf_p95:.2},\
             {rc_p50:.2},{rc_p95:.2},\
             {st_p50:.2},{st_p95:.2},\
             {sum_p50:.2},{sum_p95:.2},\
             {:.1},{:.1},{:.1},{:.1}",
            be_p95 / sum_p95 * 100.0,
            gf_p95 / sum_p95 * 100.0,
            rc_p95 / sum_p95 * 100.0,
            st_p95 / sum_p95 * 100.0
        );
    }
}

/// P1b 验收探针（默认忽略）：构造 5005 块文档 + 第一次编辑的端到端耗时。
/// cargo test -p veil-ui --release -- --ignored --nocapture perf_cold_first_edit
#[test]
#[ignore = "性能探针，不参与常规门禁"]
fn perf_cold_first_edit() {
    let text = stress_doc();
    let store = MemStore::default();
    let t = Instant::now();
    let mut a = app(&text, None, false, &store);
    let app_ms = t.elapsed().as_secs_f64() * 1e3;
    let t = Instant::now();
    let dirty = a.editor.apply(Intent::InsertText("a".into())).unwrap();
    a.after_edit(dirty);
    let edit_ms = t.elapsed().as_secs_f64() * 1e3;
    println!("perf_cold_first_edit_ms={:.3}", app_ms + edit_ms);
    println!("perf_cold_app_ms={app_ms:.3}");
    println!("perf_cold_edit_ms={edit_ms:.3}");
    println!("perf_layouts_after_first_edit={}", a.cache.layouts.len());
}

/// 对照探针（默认忽略）：先完成一次极小排版（等价真实窗口的首帧），
/// 再测同一「首编辑」路径，用于区分字体懒加载成本与编辑本身成本。
/// cargo test -p veil-ui --release -- --ignored --nocapture perf_cold_first_edit_prewarmed
#[test]
#[ignore = "性能探针，不参与常规门禁"]
fn perf_cold_first_edit_prewarmed() {
    let text = stress_doc();
    let store = MemStore::default();
    let mut a = app(&text, None, false, &store);
    let _ = a.engine.layout_preedit("a");
    let t = Instant::now();
    let dirty = a.editor.apply(Intent::InsertText("a".into())).unwrap();
    a.after_edit(dirty);
    let edit_ms = t.elapsed().as_secs_f64() * 1e3;
    println!("perf_cold_edit_prewarmed_ms={edit_ms:.3}");
    println!("perf_cold_edit_prewarmed_layouts={}", a.cache.layouts.len());
}

/// 真实生命周期的冷首编辑探针（默认忽略）：App 构造 → 首帧 paint（真实窗口启动必经）
/// → 用户第一次按键。与 `perf_cold_first_edit`（跳过首帧）对照，用于区分「用户可感知的
/// 首键延迟」与「字体/字形首次预热成本」。
/// cargo test -p veil-ui --release -- --ignored --nocapture perf_first_edit_after_first_frame
#[test]
#[ignore = "性能探针，不参与常规门禁"]
fn perf_first_edit_after_first_frame() {
    let text = stress_doc();
    let store = MemStore::default();
    let t = Instant::now();
    let mut a = app(&text, None, false, &store);
    let app_ms = t.elapsed().as_secs_f64() * 1e3;
    let t = Instant::now();
    a.paint();
    let first_frame_ms = t.elapsed().as_secs_f64() * 1e3;
    let t = Instant::now();
    let dirty = a.editor.apply(Intent::InsertText("a".into())).unwrap();
    a.after_edit(dirty);
    a.paint();
    let first_edit_ms = t.elapsed().as_secs_f64() * 1e3;
    println!("perf_first_edit_after_first_frame app_ms={app_ms:.3} first_frame_ms={first_frame_ms:.3} first_edit_ms={first_edit_ms:.3}");
    println!("perf_first_edit_layouts={}", a.cache.layouts.len());
}

/// 0a 验收探针（默认忽略）：10MB 单块连续滚轮 300 格后仍须有内容像素。
/// cargo test -p veil-ui --release -- --ignored --nocapture perf_huge_scroll
#[test]
#[ignore = "性能探针，不参与常规门禁"]
fn perf_huge_scroll() {
    let text = "a".repeat(10 * 1024 * 1024);
    let store = MemStore::default();
    let mut a = app(&text, None, false, &store);
    a.ensure_layouts_visible();
    let step = -3.0 * BASE_LINE_HEIGHT * a.input_state.scale;
    for _ in 0..300 {
        a.on_wheel(step);
        let _ = a.render();
    }
    let vp = a.viewport();
    let bg = a.input_state.theme.bg;
    let bgpix = ((bg[0] as u32) << 16) | ((bg[1] as u32) << 8) | bg[2] as u32;
    let (doc_h, scroll, block_h) = (
        a.doc_height(),
        a.window_state.scroll_y,
        a.block_height(&a.index.blocks[0]),
    );
    let frame = a.render();
    let lo = (vp.y as usize + 1) * frame.width as usize;
    let hi = ((vp.y + vp.h) as usize).min(frame.height as usize) * frame.width as usize;
    let nonbg = frame.pixels[lo..hi].iter().filter(|p| **p != bgpix).count();
    println!(
        "huge_scroll vp={}x{} doc_h={doc_h} block_h={block_h} scroll={scroll} nonbg={nonbg}",
        vp.w, vp.h
    );
    assert!(nonbg > 1000, "滚到 10MB 单块末尾仍须有内容");
}

/// 0a 验收探针（默认忽略）：1/2/10MB 单块只排前缀，不再整块成形。
/// cargo test -p veil-ui --release -- --ignored --nocapture perf_huge_block
#[test]
#[ignore = "性能探针，不参与常规门禁"]
fn perf_huge_block() {
    for mb in [1usize, 2, 10] {
        let text = "a".repeat(mb * 1024 * 1024);
        let store = MemStore::default();
        let t = Instant::now();
        let mut a = app(&text, None, false, &store);
        let app_ms = t.elapsed().as_secs_f64() * 1e3;
        let t = Instant::now();
        let dirty = a.editor.apply(Intent::InsertText("b".into())).unwrap();
        a.after_edit(dirty);
        a.paint();
        let edit_ms = t.elapsed().as_secs_f64() * 1e3;
        let t = Instant::now();
        a.editor.apply(Intent::MoveDocEnd).unwrap();
        let dirty = a.editor.apply(Intent::InsertText("c".into())).unwrap();
        a.after_edit(dirty);
        a.paint();
        let tail_ms = t.elapsed().as_secs_f64() * 1e3;
        let l = &a.cache.layouts[&a.index.blocks[0].id];
        println!(
            "huge_mb={mb} app_ms={app_ms:.3} edit_ms={edit_ms:.3} tail_ms={tail_ms:.3} prefix={} truncated={} height={:.1}",
            l.layout_text().len(),
            l.truncated,
            l.height
        );
        assert!(l.truncated);
        assert!(l.layout_text().len() <= MAX_BLOCK_BYTES);
        assert_eq!(a.editor.session().serialize().len(), text.len() + 2);
    }
}

// ---- 崩溃恢复 + 会话恢复 + 最近文件（0b）：故障注入 ----
// 核心验收：任意时刻杀进程，重启后可恢复到最后一次成功提交的恢复快照。
