//! Session 级集成测试：INSTANCE §3 不变量 + 定义文档反例的回归锚点。

use veil_core::{
    AnnotationKind, Bias, BlockId, BlockKind, Edit, EditError, InlineKind, InlineNode, Range,
    Session,
};

const SAMPLE: &str = "# Title\n\nfirst para\n\n```\ncode\n```\n\nlast para\n";

fn insert(at: usize, text: &str) -> Edit {
    Edit::Insert {
        at,
        text: text.to_owned(),
    }
}

fn replace(start: usize, end: usize, text: &str) -> Edit {
    Edit::Replace {
        range: Range::new(start, end),
        text: text.to_owned(),
    }
}

#[test]
fn roundtrip_identity_without_edits() {
    // 幂等不变量：打开→序列化 == 输入字节（INSTANCE §3）
    let s = Session::open(SAMPLE);
    assert_eq!(s.serialize(), SAMPLE);
    // 边界：空文档
    assert_eq!(Session::open("").serialize(), "");
}

#[test]
fn edit_middle_paragraph_dirty_is_local() {
    // 局部性不变量：改中间段落，脏集只含该段，前后块 id 不变
    let mut s = Session::open(SAMPLE);
    let before: Vec<_> = s.blocks().iter().map(|b| b.0).collect();
    let para_start = "# Title\n\n".len();
    let ev = s.apply(replace(para_start, para_start + 1, "X")).unwrap();
    let after = s.blocks();
    assert_eq!(ev.dirty_blocks.len(), 1);
    assert_eq!(ev.dirty_blocks[0], after[1].0); // 脏的是中间段落
    assert_eq!(before[0], after[0].0); // 前块 id 保留
    assert_eq!(before[2], after[2].0); // 代码块保留
    assert_eq!(before[3], after[3].0); // 尾段保留
    assert_eq!(s.serialize(), SAMPLE.replacen('f', "X", 1));
}

#[test]
fn fence_swallow_resync_regression() {
    // 反例 1 回归：文首输入 ``` 吞咽后方所有块，脏集必须覆盖它们
    let mut s = Session::open("a\n\nb\n\nc\n");
    assert_eq!(s.blocks().len(), 3);
    let ev = s.apply(insert(0, "```\n")).unwrap();
    let blocks = s.blocks();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].1, BlockKind::FencedCode);
    assert_eq!(ev.dirty_blocks, vec![blocks[0].0]);
    // 撤销后恢复原有三段（undo 路径同样走重同步）
    s.undo().unwrap();
    assert_eq!(s.blocks().len(), 3);
    assert_eq!(s.serialize(), "a\n\nb\n\nc\n");
}

#[test]
fn atx_interrupts_paragraph() {
    // 反例 1 变体：段落中插入 "# " 使段落分裂为 标题+段落
    let mut s = Session::open("foo\nbar\n");
    let ev = s.apply(insert(4, "# ")).unwrap();
    let blocks = s.blocks();
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].1, BlockKind::Paragraph);
    assert_eq!(blocks[1].1, BlockKind::AtxHeading(1));
    assert!(!ev.dirty_blocks.is_empty());
    assert_eq!(s.serialize(), "foo\n# bar\n");
}

#[test]
fn annotations_follow_edits_via_marks() {
    // 注解经 mark 间接寻址，文本编辑时自动跟随、无需重排
    let mut s = Session::open("hello world");
    let start = s.create_mark(0, Bias::Right).unwrap();
    let end = s.create_mark(5, Bias::Left).unwrap();
    let aid = s.annotate(AnnotationKind::Reveal, start, end);
    s.apply(insert(0, "> ")).unwrap();
    assert_eq!(
        s.annotations(),
        vec![(AnnotationKind::Reveal, Range::new(2, 7))]
    );
    s.remove_annotation(aid);
    assert!(s.annotations().is_empty());
}

#[test]
fn undo_redo_restores_text() {
    let mut s = Session::open("base");
    s.apply(insert(4, " one")).unwrap();
    s.apply(insert(8, " two")).unwrap();
    assert_eq!(s.serialize(), "base one two");
    s.undo().unwrap();
    assert_eq!(s.serialize(), "base one");
    s.undo().unwrap();
    assert_eq!(s.serialize(), "base");
    assert!(s.undo().is_none()); // 边界：栈空
    s.redo().unwrap();
    s.redo().unwrap();
    assert_eq!(s.serialize(), "base one two");
}

#[test]
fn consecutive_typing_merges_into_one_undo_step() {
    // 编辑器惯例：连续单字符输入合并为一步撤销，静默窗 500ms 内
    let mut s = Session::open("x");
    for (i, ch) in "abc".chars().enumerate() {
        s.apply(insert(1 + i, &ch.to_string())).unwrap();
    }
    assert_eq!(s.serialize(), "xabc");
    s.undo().unwrap();
    assert_eq!(s.serialize(), "x", "连续单字符输入应合并为一步");
    assert!(s.undo().is_none());
}

#[test]
fn newline_breaks_undo_group() {
    let mut s = Session::open("");
    s.apply(insert(0, "a")).unwrap();
    s.apply(insert(1, "\n")).unwrap();
    s.apply(insert(2, "b")).unwrap();
    s.undo().unwrap();
    assert_eq!(s.serialize(), "a\n");
    s.undo().unwrap();
    assert_eq!(s.serialize(), "a");
    s.undo().unwrap();
    assert_eq!(s.serialize(), "");
}

#[test]
fn new_edit_invalidates_redo() {
    let mut s = Session::open("a");
    s.apply(insert(1, "b")).unwrap();
    s.undo().unwrap();
    s.apply(insert(1, "c")).unwrap();
    assert!(s.redo().is_none());
    assert_eq!(s.serialize(), "ac");
}

#[test]
fn empty_edit_is_noop_not_recorded() {
    let mut s = Session::open("a");
    let ev = s.apply(replace(0, 0, "")).unwrap();
    assert!(ev.dirty_blocks.is_empty());
    assert!(s.undo().is_none()); // 空操作不进撤销栈
}

#[test]
fn line_col_is_one_based_and_counts_chars() {
    // 「ab\n中c\n」逐字节：a=0 b=1 \n=2 中=[3,6) c=6 \n=7，len=8
    let s = Session::open("ab\n中c\n");
    assert_eq!(s.line_col(0), Some((1, 1)));
    assert_eq!(s.line_col(2), Some((1, 3)), "行尾（换行符前）");
    assert_eq!(s.line_col(3), Some((2, 1)));
    assert_eq!(s.line_col(6), Some((2, 2)), "列按字符计数：中 为 1 列");
    assert_eq!(s.line_col(7), Some((2, 3)));
    assert_eq!(s.line_col(8), Some((3, 1)), "尾换行后的空行");
    assert_eq!(s.line_col(4), None, "非字符边界");
    assert_eq!(s.line_col(9), None, "越界");
}

#[test]
fn line_col_of_empty_document() {
    let s = Session::open("");
    assert_eq!(s.line_col(0), Some((1, 1)));
}

#[test]
fn error_out_of_range() {
    let mut s = Session::open("abc");
    assert_eq!(s.apply(insert(4, "x")), Err(EditError::OutOfRange)); // max+1
    assert_eq!(s.apply(insert(3, "x")).unwrap().dirty_blocks.len(), 1); // max 合法
}

#[test]
fn error_not_char_boundary() {
    let mut s = Session::open("é"); // 2 字节
    assert_eq!(s.apply(insert(1, "x")), Err(EditError::NotCharBoundary));
    assert_eq!(
        s.create_mark(1, Bias::Left),
        Err(EditError::NotCharBoundary)
    );
    assert_eq!(s.serialize(), "é"); // 失败路径不产生副作用
}

// ===== reparse 向前扩展边界（编辑落在块间空白 / 文档末尾） =====

#[test]
fn edit_into_empty_doc_creates_block() {
    // 空文档键入第一个字符：必须产生段落块
    let mut s = Session::open("");
    let ev = s.apply(insert(0, "a")).unwrap();
    let blocks = s.blocks();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].1, BlockKind::Paragraph);
    assert_eq!((blocks[0].2.start, blocks[0].2.end), (0, 1));
    assert_eq!(ev.dirty_blocks, vec![blocks[0].0]);
}

#[test]
fn append_after_last_block_grows_paragraph() {
    // 文档无尾空行时在末尾续写：段落向下生长，而不是块结构定格
    let mut s = Session::open("a\n");
    s.apply(insert(2, "b")).unwrap(); // "a\nb"
    let blocks = s.blocks();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].1, BlockKind::Paragraph);
    assert_eq!((blocks[0].2.start, blocks[0].2.end), (0, 3));
}

#[test]
fn insert_into_blank_gap_merges_paragraph_upward() {
    // 段落间隙的空白行被填入文字：与上文合并为同一段落（GFM 连续行规则）
    let mut s = Session::open("a\n\nb\n");
    s.apply(insert(2, "x")).unwrap(); // "a\nx\nb\n"
    let blocks = s.blocks();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].1, BlockKind::Paragraph);
    assert_eq!((blocks[0].2.start, blocks[0].2.end), (0, 6));
}

#[test]
fn insert_into_leading_gap_shifts_block() {
    // 文首空白区填入文字：新内容必须进入块结构
    let mut s = Session::open("\na\n");
    s.apply(insert(0, "x")).unwrap(); // "x\na\n"
    let blocks = s.blocks();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].1, BlockKind::Paragraph);
    assert_eq!(blocks[0].2.start, 0);
}

#[test]
fn insert_fence_into_gap_swallows_forward() {
    // 间隙空白行变为围栏：吞咽后方内容（反例 1 的空白区变体）
    let mut s = Session::open("a\n\nb\n");
    s.apply(insert(2, "```\n")).unwrap(); // "a\n```\n\nb\n"
    let blocks = s.blocks();
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].1, BlockKind::Paragraph);
    assert_eq!(blocks[1].1, BlockKind::FencedCode);
}

#[test]
fn gap_fill_with_remaining_blank_forms_new_paragraph() {
    // 多重空白间隙只填中间一行：不并段，新生成独立段落
    let mut s = Session::open("a\n\n\nb\n");
    s.apply(insert(3, "x")).unwrap(); // "a\n\nx\nb\n"
    let blocks = s.blocks();
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].1, BlockKind::Paragraph);
    assert_eq!((blocks[0].2.start, blocks[0].2.end), (0, 2));
    assert_eq!(blocks[1].1, BlockKind::Paragraph);
    assert_eq!((blocks[1].2.start, blocks[1].2.end), (3, 7));
}

#[test]
fn insert_newline_in_first_para_preserves_later_block_ids() {
    // 局部性不变量（含行平移）：首段内插换行，后块平移但 id 保留
    let mut s = Session::open("aaa\n\nb\n\nc\n");
    let before: Vec<_> = s.blocks().iter().map(|b| b.0).collect();
    let ev = s.apply(insert(3, "\nx")).unwrap(); // "aaa\nx\n\nb\n\nc\n"
    let after = s.blocks();
    assert_eq!(after.len(), 3);
    assert_eq!(ev.dirty_blocks.len(), 1);
    assert_eq!(ev.dirty_blocks[0], after[0].0); // 仅首段脏
    assert_eq!(after[1].0, before[1]); // b 块 id 保留
    assert_eq!(after[2].0, before[2]); // c 块 id 保留
    assert_eq!((after[1].2.start, after[1].2.end), (7, 9));
}

// ===== inline 节点的 Session 级行为 =====

#[test]
fn block_inlines_absolute_offsets() {
    let s = Session::open("# T\n\ntext *em* and `code`\n");
    let blocks = s.blocks();
    assert!(s.block_inlines(blocks[0].0).unwrap().is_empty()); // "# T" 无标记
    let ns = s.block_inlines(blocks[1].0).unwrap();
    let got: Vec<_> = ns
        .iter()
        .map(|n| {
            (
                n.kind,
                (n.range.start, n.range.end),
                (n.content.start, n.content.end),
            )
        })
        .collect();
    assert_eq!(
        got,
        vec![
            (InlineKind::Emph, (10, 14), (11, 13)),
            (InlineKind::Code, (19, 25), (20, 24)),
        ]
    );
}

#[test]
fn fenced_code_block_has_no_inlines() {
    let s = Session::open("```\n**not bold**\n```\n");
    let blocks = s.blocks();
    assert_eq!(blocks[0].1, BlockKind::FencedCode);
    assert_eq!(s.block_inlines(blocks[0].0).unwrap(), Vec::new());
}

#[test]
fn unknown_block_id_returns_none() {
    let s = Session::open("a\n");
    assert_eq!(s.block_inlines(BlockId(9999)), None);
}

#[test]
fn inlines_recomputed_after_edit_shifts_offsets() {
    let mut s = Session::open("para *em*\n");
    s.apply(insert(0, ">> ")).unwrap();
    assert_eq!(s.serialize(), ">> para *em*\n");
    let blocks = s.blocks();
    let ns = s.block_inlines(blocks[0].0).unwrap();
    assert_eq!((ns[0].range.start, ns[0].range.end), (8, 12)); // 整体平移 +3
}

#[test]
fn deleting_marker_removes_inline_node() {
    let mut s = Session::open("a **b**\n");
    s.apply(Edit::Delete {
        range: Range::new(2, 4),
    })
    .unwrap(); // 删掉开标记 `**`
    assert_eq!(s.serialize(), "a b**\n");
    let blocks = s.blocks();
    assert!(s.block_inlines(blocks[0].0).unwrap().is_empty());
}

#[test]
fn insert_inside_emph_grows_content() {
    let mut s = Session::open("*ab*\n");
    s.apply(insert(2, "x")).unwrap();
    assert_eq!(s.serialize(), "*axb*\n");
    let blocks = s.blocks();
    let ns = s.block_inlines(blocks[0].0).unwrap();
    assert_eq!((ns[0].content.start, ns[0].content.end), (1, 4)); // "axb"
}

// ===== undo/redo 与 mark/annotation 交叉 =====

#[test]
fn undo_restores_annotation_range() {
    let mut s = Session::open("hello world");
    let m0 = s.create_mark(0, Bias::Right).unwrap();
    let m1 = s.create_mark(5, Bias::Left).unwrap();
    s.annotate(AnnotationKind::Reveal, m0, m1);
    s.apply(insert(2, "X")).unwrap();
    assert_eq!(
        s.annotations(),
        vec![(AnnotationKind::Reveal, Range::new(0, 6))]
    );
    s.undo().unwrap();
    assert_eq!(
        s.annotations(),
        vec![(AnnotationKind::Reveal, Range::new(0, 5))]
    );
    assert_eq!(s.serialize(), "hello world");
}

#[test]
fn undo_marks_outside_region_restore_interior_collapses() {
    // 删除区间外的 mark 撤销后复原；区间内的 mark 坍缩不可逆（原位置信息已丢失）
    let mut s = Session::open("hello world");
    let m0 = s.create_mark(0, Bias::Left).unwrap();
    let m1 = s.create_mark(5, Bias::Right).unwrap(); // 将落入删除区间
    let m2 = s.create_mark(11, Bias::Right).unwrap();
    s.apply(Edit::Delete {
        range: Range::new(2, 6),
    })
    .unwrap(); // "heworld"
    assert_eq!(s.mark_offset(m1), Some(2));
    s.undo().unwrap();
    assert_eq!(s.mark_offset(m0), Some(0));
    assert_eq!(s.mark_offset(m1), Some(6)); // 坍缩后停在插入物之后，不回到 5
    assert_eq!(s.mark_offset(m2), Some(11));
}

#[test]
fn redo_reapplies_annotation_range() {
    let mut s = Session::open("hello world");
    let m0 = s.create_mark(0, Bias::Right).unwrap();
    let m1 = s.create_mark(5, Bias::Left).unwrap();
    s.annotate(AnnotationKind::Reveal, m0, m1);
    s.apply(insert(2, "X")).unwrap();
    s.undo().unwrap();
    s.redo().unwrap();
    assert_eq!(
        s.annotations(),
        vec![(AnnotationKind::Reveal, Range::new(0, 6))]
    );
    assert_eq!(s.serialize(), "heXllo world");
}

#[test]
fn annotation_on_deleted_region_collapses_to_empty() {
    // 被删内容坍缩后注解区间为空（session.rs annotations() 文档语义）
    let mut s = Session::open("hello world");
    let a = s.create_mark(2, Bias::Right).unwrap();
    let b = s.create_mark(6, Bias::Left).unwrap();
    s.annotate(AnnotationKind::Reveal, a, b);
    s.apply(Edit::Delete {
        range: Range::new(0, 11),
    })
    .unwrap();
    assert_eq!(
        s.annotations(),
        vec![(AnnotationKind::Reveal, Range::new(0, 0))]
    );
}

#[test]
fn annotations_never_enter_serialized_source() {
    // 注解不持久化不变量（INSTANCE §3）：含 Reveal/缓存句柄时序列化恒等于源
    let mut s = Session::open("a **b**\n");
    let m0 = s.create_mark(2, Bias::Right).unwrap();
    let m1 = s.create_mark(7, Bias::Left).unwrap();
    s.annotate(AnnotationKind::Reveal, m0, m1);
    s.annotate(AnnotationKind::BlockRenderCacheId(42), m0, m1);
    assert_eq!(s.serialize(), "a **b**\n");
}

#[test]
fn undo_returns_dirty_events() {
    let mut s = Session::open("a\n\nb\n");
    s.apply(insert(0, "x")).unwrap();
    let ev = s.undo().unwrap();
    assert!(!ev.dirty_blocks.is_empty());
    assert_eq!(s.serialize(), "a\n\nb\n");
}

#[test]
fn redo_restores_fence_swallow_structure() {
    let mut s = Session::open("a\n\nb\n");
    s.apply(insert(0, "```\n")).unwrap();
    s.undo().unwrap();
    assert_eq!(s.blocks().len(), 2);
    s.redo().unwrap();
    let blocks = s.blocks();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].1, BlockKind::FencedCode);
}

// ===== inline × 编辑交叉 =====

#[test]
fn edit_inside_link_url_grows_url_range() {
    let mut s = Session::open("[t](abc)\n");
    s.apply(insert(5, "X")).unwrap(); // "[t](aXbc)"
    let blocks = s.blocks();
    let ns = s.block_inlines(blocks[0].0).unwrap();
    assert_eq!(ns.len(), 1);
    assert_eq!(ns[0].kind, InlineKind::Link);
    let u = ns[0].url.unwrap();
    assert_eq!((u.start, u.end), (4, 8));
}

#[test]
fn typing_closing_marker_creates_emph() {
    let mut s = Session::open("*a\n");
    let id0 = s.blocks()[0].0;
    assert!(s.block_inlines(id0).unwrap().is_empty());
    let ev = s.apply(insert(2, "*")).unwrap(); // "*a*\n"
    let blocks = s.blocks();
    assert!(ev.dirty_blocks.contains(&blocks[0].0));
    let ns = s.block_inlines(blocks[0].0).unwrap();
    assert_eq!(ns.len(), 1);
    assert_eq!(ns[0].kind, InlineKind::Emph);
}

#[test]
fn deleting_strong_opener_half_yields_emph() {
    // "**a**" 删掉开标记一半 → "*a**"：v0 语义下成 Emph（闭标记吃掉 `**` 的第一半）
    let mut s = Session::open("**a**\n");
    s.apply(Edit::Delete {
        range: Range::new(1, 2),
    })
    .unwrap();
    assert_eq!(s.serialize(), "*a**\n");
    let blocks = s.blocks();
    let ns = s.block_inlines(blocks[0].0).unwrap();
    assert_eq!(ns.len(), 1);
    assert_eq!(ns[0].kind, InlineKind::Emph);
    assert_eq!((ns[0].content.start, ns[0].content.end), (1, 2));
}

#[test]
fn appending_closing_strong_creates_node() {
    let mut s = Session::open("**a");
    s.apply(insert(3, "**")).unwrap(); // "**a**"
    let blocks = s.blocks();
    let ns = s.block_inlines(blocks[0].0).unwrap();
    assert_eq!(ns.len(), 1);
    assert_eq!(ns[0].kind, InlineKind::Strong);
    assert_eq!((ns[0].range.start, ns[0].range.end), (0, 5));
}

#[test]
fn markers_inserted_inside_code_stay_literal() {
    // Code 内容原样不嵌套：编辑注入标记后仍无子节点
    let mut s = Session::open("`x`\n");
    s.apply(insert(1, "**")).unwrap(); // "`**x`"
    let blocks = s.blocks();
    let ns = s.block_inlines(blocks[0].0).unwrap();
    assert_eq!(ns.len(), 1);
    assert_eq!(ns[0].kind, InlineKind::Code);
    assert!(ns[0].children.is_empty());
    assert_eq!((ns[0].content.start, ns[0].content.end), (1, 4));
}

#[test]
fn multibyte_insert_shifts_inline_offsets() {
    // 一切区间为字节偏移：插入 3 字节汉字，后续节点整体平移 +3
    let mut s = Session::open("[a](u)\n");
    s.apply(insert(0, "中")).unwrap();
    let blocks = s.blocks();
    let ns = s.block_inlines(blocks[0].0).unwrap();
    assert_eq!(ns.len(), 1);
    assert_eq!((ns[0].range.start, ns[0].range.end), (3, 9));
    let u = ns[0].url.unwrap();
    assert_eq!((u.start, u.end), (7, 8));
}

#[test]
fn deleting_blank_line_merges_paragraph_inline_scope() {
    // 两段落并段后，inline 解析覆盖合并后的整段
    let mut s = Session::open("*a*\n\n*b*\n");
    assert_eq!(s.blocks().len(), 2);
    s.apply(Edit::Delete {
        range: Range::new(4, 5),
    })
    .unwrap(); // "*a*\n*b*\n"
    let blocks = s.blocks();
    assert_eq!(blocks.len(), 1);
    let ns = s.block_inlines(blocks[0].0).unwrap();
    assert_eq!(ns.len(), 2);
    assert_eq!(ns[1].kind, InlineKind::Emph);
    assert_eq!((ns[1].range.start, ns[1].range.end), (4, 7));
}

// ===== 属性测试（确定性伪随机，零新增依赖） =====

/// xorshift64*：可复现的伪随机源。
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn boundaries(s: &str) -> Vec<usize> {
    s.char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(s.len()))
        .collect()
}

#[test]
fn prop_random_edits_roundtrip_and_undo() {
    // 幂等 + 撤销不变量：随机语料随机编辑，内核文本始终等于参考串；全撤销回到原文
    let tokens: [&str; 15] = [
        "a", "中", "*", "**", "`", "$", "[", "]", "(", ")", "\n", "# ", " ", "xy", "```\n",
    ];
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    for _case in 0..50 {
        let mut expected = String::new();
        for _ in 0..rng.below(40) + 1 {
            expected.push_str(tokens[rng.below(tokens.len())]);
        }
        let original = expected.clone();
        let mut s = Session::open(&original);
        for _ in 0..rng.below(20) + 1 {
            let bs = boundaries(&expected);
            let i = bs[rng.below(bs.len())];
            if rng.below(2) == 0 {
                let t = tokens[rng.below(tokens.len())];
                s.apply(insert(i, t)).unwrap();
                expected.insert_str(i, t);
            } else {
                let j = bs[rng.below(bs.len())];
                let (a, b) = (i.min(j), i.max(j));
                s.apply(Edit::Delete {
                    range: Range::new(a, b),
                })
                .unwrap();
                expected.replace_range(a..b, "");
            }
        }
        assert_eq!(s.serialize(), expected);
        while s.undo().is_some() {}
        assert_eq!(s.serialize(), original);
    }
}

#[test]
fn prop_inline_parse_never_panics_and_stays_in_bounds() {
    // B5 拒绝服务防线：随机标记语料不 panic；所有节点区间合法且含于块内
    let tokens: [&str; 12] = [
        "*", "**", "`", "$", "[", "](", ")", "\\*", "a", "中", "\n", " ",
    ];
    let mut rng = Rng(0xdead_beef_cafe_f00d);
    for _ in 0..200 {
        let mut doc = String::new();
        for _ in 0..rng.below(60) {
            doc.push_str(tokens[rng.below(tokens.len())]);
        }
        let s = Session::open(&doc);
        for (id, _, brange) in s.blocks() {
            for n in s.block_inlines(id).unwrap() {
                check_node(&doc, brange, &n);
            }
        }
    }
}

fn check_node(doc: &str, block: Range, n: &InlineNode) {
    assert!(n.range.start <= n.range.end && n.range.end <= doc.len());
    assert!(block.start <= n.range.start && n.range.end <= block.end);
    assert!(doc.is_char_boundary(n.range.start) && doc.is_char_boundary(n.range.end));
    assert!(n.content.start >= n.range.start && n.content.end <= n.range.end);
    if let Some(u) = n.url {
        assert!(u.start >= n.range.start && u.end <= n.range.end);
    }
    for c in &n.children {
        assert!(c.range.start >= n.content.start && c.range.end <= n.content.end);
        check_node(doc, block, c);
    }
}

#[test]
fn prop_marks_stay_valid_through_random_edits() {
    // Mark 稳定不变量的全性表述：任意编辑序列后，mark 始终落在合法字符边界内
    let tokens: [&str; 6] = ["a", "中", "*", "\n", " ", "`x`"];
    let mut rng = Rng(0x0f0f_0f0f_1234_5678);
    for _ in 0..30 {
        let mut expected = String::new();
        for _ in 0..rng.below(30) + 1 {
            expected.push_str(tokens[rng.below(tokens.len())]);
        }
        let mut s = Session::open(&expected);
        let mut marks = Vec::new();
        for (k, &b) in boundaries(&expected).iter().enumerate().step_by(3) {
            let bias = if k % 2 == 0 { Bias::Left } else { Bias::Right };
            marks.push(s.create_mark(b, bias).unwrap());
        }
        for _ in 0..rng.below(10) + 1 {
            let bs = boundaries(&expected);
            let i = bs[rng.below(bs.len())];
            let j = bs[rng.below(bs.len())];
            let (a, b) = (i.min(j), i.max(j));
            let t = tokens[rng.below(tokens.len())];
            s.apply(Edit::Replace {
                range: Range::new(a, b),
                text: t.to_owned(),
            })
            .unwrap();
            expected.replace_range(a..b, t);
            for &m in &marks {
                let off = s.mark_offset(m).unwrap();
                assert!(off <= expected.len() && expected.is_char_boundary(off));
            }
        }
    }
}

// ===== 视图层接缝（阶段二切片 1）：grapheme 导航 / 按需取片 / 长度 =====

#[test]
fn grapheme_navigation_multibyte() {
    // 「中」3 字节、「文」3 字节：逐 grapheme 步进，字节边界正确
    let s = Session::open("中文");
    assert_eq!(s.prev_grapheme(6), Some(3));
    assert_eq!(s.prev_grapheme(3), Some(0));
    assert_eq!(s.prev_grapheme(0), None); // 文首再左移为空
    assert_eq!(s.next_grapheme(0), Some(3));
    assert_eq!(s.next_grapheme(3), Some(6));
    assert_eq!(s.next_grapheme(6), None); // 文末再右移为空
}

#[test]
fn grapheme_navigation_emoji_cluster_is_one_step() {
    // ZWJ 序列「👨‍👩‍👧」= 18 字节、单 grapheme：一步跨过，不得停在簇内部
    let s = Session::open("👨\u{200d}👩\u{200d}👧");
    assert_eq!(s.next_grapheme(0), Some(18));
    assert_eq!(s.prev_grapheme(18), Some(0));
}

#[test]
fn grapheme_navigation_rejects_non_boundary_and_overrun() {
    let s = Session::open("中"); // len = 3
    assert_eq!(s.next_grapheme(1), None); // 非字符边界
    assert_eq!(s.prev_grapheme(2), None);
    assert_eq!(s.next_grapheme(4), None); // 越界
    assert_eq!(s.prev_grapheme(4), None);
}

#[test]
fn text_slice_bounds_and_content() {
    let s = Session::open("ab中文cd"); // 2+3+3+2 = 10 字节
    assert_eq!(s.text_slice(Range::new(0, 2)).as_deref(), Some("ab"));
    assert_eq!(s.text_slice(Range::new(2, 8)).as_deref(), Some("中文"));
    assert_eq!(s.text_slice(Range::new(3, 8)), None); // 起点非字符边界
    assert_eq!(s.text_slice(Range::new(0, 11)), None); // 越界（len=10）
    assert_eq!(s.text_slice(Range { start: 5, end: 2 }), None); // start > end（绕过 new 的断言）
    assert_eq!(s.text_slice(Range::new(10, 10)).as_deref(), Some("")); // 空区间合法
}

#[test]
fn text_len_tracks_edits() {
    let mut s = Session::open("ab");
    assert_eq!(s.text_len(), 2);
    s.apply(insert(2, "中")).unwrap();
    assert_eq!(s.text_len(), 5);
}

#[test]
fn grapheme_navigation_after_edit_uses_current_text() {
    // mark 变换后再导航：编辑后的文本必须立即可见（不得用旧快照）
    let mut s = Session::open("ab");
    s.apply(insert(1, "中")).unwrap(); // "a中b"，len=5
    assert_eq!(s.next_grapheme(0), Some(1));
    assert_eq!(s.next_grapheme(1), Some(4));
    assert_eq!(s.prev_grapheme(5), Some(4));
}

// ---- 数学块（切片 6）：Session 门面与增量重解析 ----

#[test]
fn session_math_source_roundtrip() {
    let s = Session::open("$$\nx^2\n$$\n");
    let (id, kind, range) = s.blocks()[0];
    assert_eq!(kind, BlockKind::Math);
    assert_eq!(range, Range::new(0, 10)); // 逐字节：3+4+3=10
    assert_eq!(s.math_source(id).as_deref(), Some("x^2"));
    assert_eq!(s.text_slice(range).as_deref(), Some("$$\nx^2\n$$\n"));
}

#[test]
fn edit_turns_paragraph_into_math_block() {
    // 段首插入 "$$\n"：未闭合吞咽至 EOF，全文成一个数学块
    let mut s = Session::open("x\n\ny\n");
    let ev = s.apply(insert(0, "$$\n")).unwrap();
    assert!(!ev.dirty_blocks.is_empty());
    let bs = s.blocks();
    assert_eq!(bs.len(), 1);
    assert_eq!(bs[0].1, BlockKind::Math);
    assert_eq!(bs[0].2, Range::new(0, 8)); // 逐字节：3+2+1+2=8
}

#[test]
fn edit_closing_math_block_reparse() {
    // 补上闭标记：数学块收缩，其后内容重新成段
    let mut s = Session::open("$$\nx\ny\n");
    let before = s.blocks();
    assert_eq!(before.len(), 1);
    assert_eq!(before[0].1, BlockKind::Math);
    s.apply(insert(5, "$$\n")).unwrap(); // "$$\nx\n$$\ny\n"
    let bs = s.blocks();
    assert_eq!(bs.len(), 2);
    assert_eq!(bs[0].1, BlockKind::Math);
    assert_eq!(bs[0].2, Range::new(0, 8)); // 3+2+3=8
    assert_eq!(bs[1].1, BlockKind::Paragraph);
    assert_eq!(bs[1].2, Range::new(8, 10)); // "y\n"
}

// ---- GFM 表格（切片 9）：Session 门面与增量重解析 ----
// 语料逐字节：`| a | b |\n| :- | -: |\n| c | d |\n`（长 32）
// 行起点：0 / 10 / 22；单元格内容区间：a[2,3) b[6,7) c[24,25) d[28,29)

#[test]
fn session_table_exposes_aligns_cells_and_ranges() {
    let doc = "| a | b |\n| :- | -: |\n| c | d |\n";
    let s = Session::open(doc);
    let (id, kind, range) = s.blocks()[0];
    assert_eq!(kind, BlockKind::Table);
    assert_eq!(range, Range::new(0, 32));
    let t = s.table(id).unwrap();
    assert_eq!(
        t.aligns,
        vec![veil_core::CellAlign::Left, veil_core::CellAlign::Right]
    );
    assert_eq!(
        t.header.iter().map(|c| c.text.as_str()).collect::<Vec<_>>(),
        ["a", "b"]
    );
    assert_eq!(t.header[0].range, Range::new(2, 3));
    assert_eq!(t.header[1].range, Range::new(6, 7));
    assert_eq!(t.rows.len(), 1);
    assert_eq!(
        t.rows[0]
            .iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>(),
        ["c", "d"]
    );
    assert_eq!(t.rows[0][0].range, Range::new(24, 25));
    assert_eq!(t.rows[0][1].range, Range::new(28, 29));
    assert_eq!(s.block_inlines(id), Some(Vec::new())); // 表格块不解析 inline
}

#[test]
fn table_cell_visible_text_strips_inline_markers() {
    // 逐字节：`| **a** | `c` |\n| --- | --- |\n`
    // 「**a**」内容区间 [2,7)、可见文本 "a"；「`c`」[10,13)、可见文本 "c"
    let s = Session::open("| **a** | `c` |\n| --- | --- |\n");
    let (id, kind, _) = s.blocks()[0];
    assert_eq!(kind, BlockKind::Table);
    let t = s.table(id).unwrap();
    assert_eq!(t.header[0].text, "a");
    assert_eq!(t.header[0].range, Range::new(2, 7));
    assert_eq!(t.header[1].text, "c");
    assert_eq!(t.header[1].range, Range::new(10, 13));
}

#[test]
fn table_escaped_pipe_renders_literal() {
    // `| a \| b | c |`：`\|` 不切分，且可见文本还原为字面 `|`
    let s = Session::open("| a \\| b | c |\n| --- | --- |\n");
    let (id, kind, _) = s.blocks()[0];
    assert_eq!(kind, BlockKind::Table);
    let t = s.table(id).unwrap();
    assert_eq!(t.header[0].text, "a | b");
    assert_eq!(t.header[1].text, "c");
}

#[test]
fn table_body_pads_missing_cells_and_truncates_extra() {
    // 表体 `| c |` 缺第二格 → 补空；`| e | f | g |` 多格 → 截断
    let s = Session::open("| a | b |\n| --- | --- |\n| c |\n| e | f | g |\n");
    let (id, kind, _) = s.blocks()[0];
    assert_eq!(kind, BlockKind::Table);
    let t = s.table(id).unwrap();
    assert_eq!(t.rows[0].len(), 2);
    assert_eq!(t.rows[0][1].text, "");
    assert_eq!(t.rows[0][1].range, Range::new(29, 29)); // 行尾（`| c |\n` 的 trim 末位）
    assert_eq!(
        t.rows[1]
            .iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>(),
        ["e", "f"]
    );
}

#[test]
fn edit_table_cell_reparses_in_place() {
    let mut s = Session::open("| a | b |\n| :- | -: |\n| c | d |\n");
    let before_range = s.blocks()[0].2;
    let ev = s.apply(replace(24, 25, "C")).unwrap(); // c → C
    assert_eq!(ev.dirty_blocks.len(), 1);
    let bs = s.blocks();
    assert_eq!(bs.len(), 1);
    assert_eq!(bs[0].1, BlockKind::Table);
    assert_eq!(bs[0].2, before_range);
    let t = s.table(bs[0].0).unwrap();
    assert_eq!(t.rows[0][0].text, "C");
    assert_eq!(s.serialize(), "| a | b |\n| :- | -: |\n| C | d |\n");
}

#[test]
fn removing_delimiter_row_collapses_table_into_paragraph() {
    let mut s = Session::open("| a | b |\n| :- | -: |\n| c | d |\n");
    // 分隔行 [10,22) → "plain\n"：表头不再有分隔行，整块退化为段落
    s.apply(replace(10, 22, "plain\n")).unwrap();
    let bs = s.blocks();
    assert_eq!(bs.len(), 1);
    assert_eq!(bs[0].1, BlockKind::Paragraph);
    assert_eq!(bs[0].2, Range::new(0, 26)); // 32 - 12 + 6
    assert_eq!(s.table(bs[0].0), None);
}

#[test]
fn inserting_delimiter_row_turns_paragraph_into_table() {
    let mut s = Session::open("| a |\nplain\n");
    assert_eq!(s.blocks()[0].1, BlockKind::Paragraph);
    s.apply(replace(6, 11, "| --- |")).unwrap(); // 第二行变分隔行
    let bs = s.blocks();
    assert_eq!(bs.len(), 1);
    assert_eq!(bs[0].1, BlockKind::Table);
    assert_eq!(bs[0].2, Range::new(0, 14));
    assert_eq!(s.table(bs[0].0).unwrap().header[0].text, "a");
}

#[test]
fn table_none_for_non_table_or_unknown_id() {
    let s = Session::open("para\n");
    let (id, kind, _) = s.blocks()[0];
    assert_eq!(kind, BlockKind::Paragraph);
    assert_eq!(s.table(id), None);
    assert_eq!(s.table(BlockId(999)), None);
}

// ---- 列表 / 引用 / 分隔线（切片 12）：Session 门面与增量重解析 ----

#[test]
fn session_list_item_range_and_inline_scope() {
    // 「1. **b**\n」：块区间 [0,9)，inline 仅解析标记后内容（strong [3,8)）
    let s = Session::open("1. **b**\n");
    let (id, kind, range) = s.blocks()[0];
    assert_eq!(
        kind,
        BlockKind::ListItem {
            ordered: true,
            depth: 0
        }
    );
    assert_eq!(range, Range::new(0, 9));
    let ns = s.block_inlines(id).unwrap();
    assert_eq!(ns.len(), 1);
    assert_eq!(ns[0].range, Range::new(3, 8));
    assert_eq!(ns[0].content, Range::new(5, 6));
}

#[test]
fn edit_turns_paragraph_into_list_item_and_back() {
    let mut s = Session::open("a\n");
    let ev = s.apply(insert(0, "- ")).unwrap();
    assert_eq!(ev.dirty_blocks.len(), 1);
    let bs = s.blocks();
    assert_eq!(bs.len(), 1);
    assert_eq!(
        bs[0].1,
        BlockKind::ListItem {
            ordered: false,
            depth: 0
        }
    );
    assert_eq!(bs[0].2, Range::new(0, 4)); // "- a\n"
                                           // 删标记回到段落
    s.apply(replace(0, 2, "")).unwrap();
    let bs = s.blocks();
    assert_eq!(bs[0].1, BlockKind::Paragraph);
    assert_eq!(bs[0].2, Range::new(0, 2));
}

#[test]
fn blockquote_splits_on_blank_line_edit() {
    // 「> a\n> b\n」8 字节：一个引用块
    let mut s = Session::open("> a\n> b\n");
    let bs = s.blocks();
    assert_eq!(bs.len(), 1);
    assert_eq!(bs[0].1, BlockKind::BlockQuote);
    assert_eq!(bs[0].2, Range::new(0, 8));
    // 行间插入空行 → 两块引用
    s.apply(insert(4, "\n")).unwrap();
    let bs = s.blocks();
    assert_eq!(bs.len(), 2);
    assert_eq!(bs[0].1, BlockKind::BlockQuote);
    assert_eq!(bs[0].2, Range::new(0, 4));
    assert_eq!(bs[1].1, BlockKind::BlockQuote);
    assert_eq!(bs[1].2, Range::new(5, 9));
}

#[test]
fn thematic_break_recognized_after_paragraph_and_editable() {
    let mut s = Session::open("a\n\n---\n");
    let bs = s.blocks();
    assert_eq!(bs.len(), 2);
    assert_eq!(bs[1].1, BlockKind::ThematicBreak);
    assert_eq!(bs[1].2, Range::new(3, 7));
    // 删去一条横线 → 只剩 `--`，不再成立（退化段落）
    s.apply(replace(5, 6, "")).unwrap();
    let bs = s.blocks();
    assert_eq!(bs.len(), 2);
    assert_eq!(bs[1].1, BlockKind::Paragraph);
    assert_eq!(s.serialize(), "a\n\n--\n");
}

#[test]
fn indent_edit_changes_list_depth_kind() {
    // 缩进 2 空格 → depth 1，块种类变化必须被重解析感知（脏块 id 更新）
    let mut s = Session::open("- a\n");
    let before_id = s.blocks()[0].0;
    let ev = s.apply(insert(0, "  ")).unwrap();
    assert_eq!(ev.dirty_blocks.len(), 1);
    let bs = s.blocks();
    assert_eq!(
        bs[0].1,
        BlockKind::ListItem {
            ordered: false,
            depth: 1
        }
    );
    assert_ne!(bs[0].0, before_id);
}
