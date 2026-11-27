//! 主题矩阵测试：light / dark / high_contrast × 六类绘制元素（kbd、mark、行内代码、
//! 表头与表格网格、选区、图片占位）。
//!
//! 目的：把这几类元素的取色钉在 [`Theme`] 字段上。上一轮修过一个真实缺陷——`<kbd>` /
//! `<mark>` / 图片占位在绘制层写死浅色值，深色主题下变成眩光块。本模块不复制调色板
//! 常量表，而是锁定三件事：
//! 1. 深色主题的对应字段与浅色主题不同（防「把浅色值写死」回归）；
//! 2. 三个主题的同一字段两两可区分（防两套主题合并成同一色）；
//! 3. 高对比度主题满足可读性底线（WCAG 相对亮度比，阈值写在各断言消息里）。
//!
//! 每个元素的字段来源见各测试顶部的注释（指向真正的绘制调用点）。

use crate::theme::Theme;

/// 矩阵列：(主题名, 主题构造器)。
type Mode = (&'static str, fn() -> Theme);

/// 同一字段在三主题下的取值行：(字段名, light, dark, high_contrast)。
type FieldRow = (&'static str, [u8; 4], [u8; 4], [u8; 4]);

/// 矩阵列：三个主题的构造器（顺序固定，便于失败时报出列名）。
const MODES: [Mode; 3] = [
    ("light", Theme::light),
    ("dark", Theme::dark),
    ("high_contrast", Theme::high_contrast),
];

/// 4 字节色（含 alpha）→ 不透明 RGB（对比度只看 RGB 分量）。
fn rgb(c: [u8; 4]) -> [u8; 3] {
    [c[0], c[1], c[2]]
}

/// 3 字节色 → 补全不透明 alpha，便于与 4 字节字段一起比较。
fn opaque(c: [u8; 3]) -> [u8; 4] {
    [c[0], c[1], c[2], 255]
}

/// 半透明色 `top` src-over 到不透明底 `bottom` 上的实际呈现色。
fn over(top: [u8; 4], bottom: [u8; 3]) -> [u8; 3] {
    let a = top[3] as f32 / 255.0;
    let ch = |t: u8, b: u8| (t as f32 * a + b as f32 * (1.0 - a)).round() as u8;
    [
        ch(top[0], bottom[0]),
        ch(top[1], bottom[1]),
        ch(top[2], bottom[2]),
    ]
}

/// WCAG 2.x 相对亮度（sRGB → 线性）。
fn luminance(rgb: [u8; 3]) -> f32 {
    let ch = |v: u8| {
        let u = v as f32 / 255.0;
        if u <= 0.039_28 {
            u / 12.92
        } else {
            ((u + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * ch(rgb[0]) + 0.7152 * ch(rgb[1]) + 0.0722 * ch(rgb[2])
}

/// WCAG 对比度 `(L1+0.05)/(L2+0.05)`，取值 1.0（同色）~ 21.0（黑白）。
fn contrast(a: [u8; 3], b: [u8; 3]) -> f32 {
    let (l1, l2) = (luminance(a), luminance(b));
    let (hi, lo) = if l1 > l2 { (l1, l2) } else { (l2, l1) };
    (hi + 0.05) / (lo + 0.05)
}

/// `<kbd>` 键帽：`paint.rs` 的 `UnderKind::KbdFrame` 取 `theme.kbd_border`（外框）+
/// `theme.kbd_bg`（内缩 1px 底）。
#[test]
fn kbd_uses_theme_fields_in_all_three_modes() {
    let (light, dark, hc) = (Theme::light(), Theme::dark(), Theme::high_contrast());
    // 深色不得复用浅色值：写死 [245,245,245] 是上一轮眩光块的根因。
    assert_ne!(dark.kbd_bg, light.kbd_bg, "深色 kbd 底色必须不同于浅色");
    assert_ne!(
        dark.kbd_border, light.kbd_border,
        "深色 kbd 描边必须不同于浅色"
    );
    // 深色底必须真的暗：明度差 ≥ 0.3（亮度 0=黑、1=白）。
    assert!(
        luminance(rgb(light.kbd_bg)) - luminance(rgb(dark.kbd_bg)) >= 0.3,
        "浅色 kbd 底应显著亮于深色: light={:?} dark={:?}",
        light.kbd_bg,
        dark.kbd_bg
    );
    // 高对比度主题自成一套，不与 light/dark 复用。
    assert_ne!(
        hc.kbd_bg, light.kbd_bg,
        "high_contrast kbd 底不得复用 light"
    );
    assert_ne!(hc.kbd_bg, dark.kbd_bg, "high_contrast kbd 底不得复用 dark");
    assert_ne!(
        hc.kbd_border, light.kbd_border,
        "high_contrast kbd 描边不得复用 light"
    );
    assert_ne!(
        hc.kbd_border, dark.kbd_border,
        "high_contrast kbd 描边不得复用 dark"
    );
    // 键帽文字（fg）压在 kbd 底上必须可读（正文标准 ≥ 4.5:1）。
    for (name, make) in MODES {
        let t = make();
        assert!(
            contrast(t.fg, rgb(t.kbd_bg)) >= 4.5,
            "{name}: kbd 文字/底色对比度 {:.2} 低于 4.5",
            contrast(t.fg, rgb(t.kbd_bg))
        );
    }
}

/// `<mark>` 高亮：`paint.rs` 的 `UnderKind::MarkBg` 取 `theme.mark_bg`（带 alpha，
/// 实际呈现色是它与正文底 `theme.bg` 的 src-over 结果）。
#[test]
fn mark_follows_theme_and_stays_readable() {
    let (light, dark, hc) = (Theme::light(), Theme::dark(), Theme::high_contrast());
    assert_ne!(dark.mark_bg, light.mark_bg, "深色 mark 底必须不同于浅色");
    assert_ne!(
        hc.mark_bg, light.mark_bg,
        "high_contrast mark 底不得复用 light"
    );
    assert_ne!(
        hc.mark_bg, dark.mark_bg,
        "high_contrast mark 底不得复用 dark"
    );
    // 合成到各自正文底后，深色必须比浅色暗：防「深色主题沿用黄底」。
    let light_mark = over(light.mark_bg, rgb(light.bg));
    let dark_mark = over(dark.mark_bg, rgb(dark.bg));
    assert!(
        luminance(light_mark) - luminance(dark_mark) >= 0.3,
        "浅色 mark 呈现色应显著亮于深色: light={light_mark:?} dark={dark_mark:?}"
    );
    // 高亮上的正文必须可读（≥ 4.5:1），三主题逐一校验。
    for (name, make) in MODES {
        let t = make();
        let shown = over(t.mark_bg, rgb(t.bg));
        assert!(
            contrast(t.fg, shown) >= 4.5,
            "{name}: mark 上的正文对比度 {:.2} 低于 4.5（呈现色 {shown:?}）",
            contrast(t.fg, shown)
        );
    }
}

/// 行内代码：`paint.rs` 的 `UnderKind::InlineCodeBg` 取 `theme.code_bg`。
#[test]
fn inline_code_follows_theme_in_all_three_modes() {
    let (light, dark, hc) = (Theme::light(), Theme::dark(), Theme::high_contrast());
    assert_ne!(dark.code_bg, light.code_bg, "深色行内代码底必须不同于浅色");
    assert_ne!(
        hc.code_bg, light.code_bg,
        "high_contrast 行内代码底不得复用 light"
    );
    assert_ne!(
        hc.code_bg, dark.code_bg,
        "high_contrast 行内代码底不得复用 dark"
    );
    for (name, make) in MODES {
        let t = make();
        // 代码底色必须与页面底可区分（否则行内代码没有「块」的视觉）。
        assert_ne!(
            t.code_bg, t.bg,
            "{name}: 行内代码底必须与页面底不同（浅色 245≠255，深色 40≠30）"
        );
        assert!(
            contrast(t.fg, rgb(t.code_bg)) >= 4.5,
            "{name}: 代码文字/底色对比度 {:.2} 低于 4.5",
            contrast(t.fg, rgb(t.code_bg))
        );
    }
}

/// 表头与表格网格：`paint_table.rs` 取 `theme.table_head_bg`（表头底）与
/// `theme.grid`（单元格描边/分隔线）。
#[test]
fn table_head_and_grid_follow_theme_in_all_three_modes() {
    let (light, dark, hc) = (Theme::light(), Theme::dark(), Theme::high_contrast());
    assert_ne!(
        dark.table_head_bg, light.table_head_bg,
        "深色表头底必须不同于浅色"
    );
    assert_ne!(dark.grid, light.grid, "深色表格网格线必须不同于浅色");
    assert_ne!(
        hc.table_head_bg, light.table_head_bg,
        "high_contrast 表头底不得复用 light"
    );
    assert_ne!(
        hc.table_head_bg, dark.table_head_bg,
        "high_contrast 表头底不得复用 dark"
    );
    assert_ne!(hc.grid, light.grid, "high_contrast 网格线不得复用 light");
    assert_ne!(hc.grid, dark.grid, "high_contrast 网格线不得复用 dark");
    for (name, make) in MODES {
        let t = make();
        // 表头底要比页面底「抬起」一档：可区分但不必高对比。
        assert_ne!(
            t.table_head_bg, t.bg,
            "{name}: 表头底必须与页面底不同（抬起的 surface 层级）"
        );
        // 网格线不是文字，按非文本元件标准：与页面底 ≥ 3:1。
        assert!(
            contrast(rgb(t.grid), rgb(t.bg)) >= 3.0,
            "{name}: 网格线/页面底对比度 {:.2} 低于 3.0",
            contrast(rgb(t.grid), rgb(t.bg))
        );
        // 网格线压在表头上也要看得见（≥ 1.5:1，避免表头内线条糊平）。
        assert!(
            contrast(rgb(t.grid), rgb(t.table_head_bg)) >= 1.5,
            "{name}: 网格线/表头底对比度 {:.2} 低于 1.5",
            contrast(rgb(t.grid), rgb(t.table_head_bg))
        );
    }
}

/// 选区：`paint.rs` 用 `theme.sel`（带 alpha）覆盖 `theme.bg`。
/// 半透明是设计意图（保留底纹），故只断言合成后的文字可读性与主题差异性。
#[test]
fn selection_follows_theme_and_keeps_text_readable() {
    let (light, dark, hc) = (Theme::light(), Theme::dark(), Theme::high_contrast());
    assert_ne!(dark.sel, light.sel, "深色选区必须不同于浅色");
    assert_ne!(hc.sel, light.sel, "high_contrast 选区不得复用 light");
    assert_ne!(hc.sel, dark.sel, "high_contrast 选区不得复用 dark");
    for (name, make) in MODES {
        let t = make();
        // 合法共享的「透明」情形只有：alpha 未满（半透明遮罩），由 over() 还原呈现色。
        assert!(
            t.sel[3] < 255,
            "{name}: 选区应为半透明遮罩（保留正文底纹），实际 alpha={}",
            t.sel[3]
        );
        let shown = over(t.sel, rgb(t.bg));
        assert!(
            contrast(t.fg, shown) >= 4.5,
            "{name}: 选区上的正文对比度 {:.2} 低于 4.5（呈现色 {shown:?}）",
            contrast(t.fg, shown)
        );
    }
}

/// 半透明色 alpha 必须来自契约 §5 的阶梯（禁止残留 80/110/120/170/180/210 裸数字）。
#[test]
fn semi_transparent_alphas_come_from_ladder() {
    use crate::tokens::alpha::AlphaStep;
    use crate::tokens::{state, surface};

    let ladder: Vec<u8> = AlphaStep::ALL.iter().map(|s| s.byte()).collect();
    for (name, make) in MODES {
        let t = make();
        let fields = [
            ("sel", t.sel[3]),
            ("mark_bg", t.mark_bg[3]),
            ("scrim", surface::scrim_alpha(t.dark)),
            ("focus_bg", state::focus_bg(&t)[3]),
        ];
        for (field, a) in fields {
            assert!(
                ladder.contains(&a),
                "{name}: {field} alpha {a} 不在阶梯 {ladder:?}"
            );
        }
    }

    // 收敛后的具体档位（真机复评对照表）。
    let light = Theme::light();
    let dark = Theme::dark();
    let hc = Theme::high_contrast();
    assert_eq!(light.sel[3], 71, "浅色选区 → 28% 档");
    assert_eq!(dark.sel[3], 102, "深色选区 → 40% 档");
    assert_eq!(hc.sel[3], 179, "高对比度选区 → 70% 档");
    assert_eq!(light.mark_bg[3], 179, "浅色 mark → 70% 档");
    assert_eq!(dark.mark_bg[3], 179, "深色 mark → 70% 档");
    assert_eq!(surface::SCRIM_ALPHA_LIGHT, 71, "浅色遮罩 → 28% 档");
    assert_eq!(surface::SCRIM_ALPHA_DARK, 102, "深色遮罩 → 40% 档");
    assert_eq!(state::focus_bg(&light)[3], 179, "焦点底 → 70% 档");
}

/// 专注/拖放压暗层：`paint.rs` 的 `FOCUS_DIM_ALPHA` 与 `DROP_DIM_ALPHA` 以主题底色
/// 半透明覆盖正文（`paint.rs` 的专注模式分支、`paint_overlay.rs::paint_drop_hint`）。
///
/// 压暗属于「内容压制态」：被压暗正文按次要文字档 ≥ 3:1 验收，不适用正文 7:1。
/// 真实缺陷来源：专注压暗原用 alpha 160（62.7%），浅色主题下压暗后正文仅约 2.2:1，
/// 低于可读下限；且该值不在契约 §5 阶梯内。此处把两个压暗 alpha 一并钉在阶梯上并
/// 断言合成后对比度。
#[test]
fn dim_overlay_keeps_text_above_secondary_contrast() {
    use crate::paint::{DROP_DIM_ALPHA, FOCUS_DIM_ALPHA};
    use crate::tokens::alpha::AlphaStep;

    let ladder: Vec<u8> = AlphaStep::ALL.iter().map(|s| s.byte()).collect();
    let dims = [("focus", FOCUS_DIM_ALPHA), ("drop", DROP_DIM_ALPHA)];
    for (dim_name, alpha) in dims {
        assert!(
            ladder.contains(&alpha),
            "{dim_name} 压暗 alpha {alpha} 不在契约 §5 阶梯 {ladder:?}"
        );
        for (name, make) in MODES {
            let t = make();
            let bg = rgb(t.bg);
            // 压暗层：主题底色 + 阶梯 alpha，src-over 到已绘制的文字像素上。
            let dim = [bg[0], bg[1], bg[2], alpha];
            for (fg_name, fg) in [("fg", t.fg), ("text", t.text)] {
                let shown = over(dim, fg);
                let ratio = contrast(shown, bg);
                assert!(
                    ratio >= 3.0,
                    "{name}/{dim_name}/{fg_name}: 被压暗正文对比度 {ratio:.2} 低于 3.0（呈现色 {shown:?}，alpha={alpha}）"
                );
            }
        }
    }
}

/// 同区间多状态（src-over）合成结果唯一：3 组具体组合，期望色逐字节钉死。
///
/// 顺序沿用绘制层实际层序：不透明行内代码底 → 半透明 mark 底；
/// 不透明搜索命中底 → 半透明选区；半透明选区 → 页面底。
#[test]
fn content_state_composition_is_deterministic() {
    type Case = (fn() -> Theme, [u8; 3], [u8; 3], [u8; 3]);
    let cases: [Case; 3] = [
        // (主题, mark over code_bg, sel over hover, sel over bg)
        (
            Theme::light,
            [252, 246, 180],
            [176, 201, 236],
            [195, 215, 242],
        ),
        (Theme::dark, [98, 79, 12], [61, 91, 136], [46, 70, 106]),
        (
            Theme::high_contrast,
            [90, 75, 0],
            [15, 71, 194],
            [0, 56, 179],
        ),
    ];
    for (make, mark_on_code, sel_on_hit, sel_on_bg) in cases {
        let t = make();
        // 1) mark_bg（半透明）覆盖行内代码底（不透明）：rank code_bg < mark_bg。
        assert_eq!(
            over(t.mark_bg, rgb(t.code_bg)),
            mark_on_code,
            "mark/code_bg 合成色不符（dark={}）",
            t.dark
        );
        // 2) 选区（半透明）覆盖搜索命中（hover 不透明）：搜索命中 < 当前命中/选区。
        assert_eq!(
            over(t.sel, over(t.hover, rgb(t.bg))),
            sel_on_hit,
            "选区/搜索命中合成色不符（dark={}）",
            t.dark
        );
        // 3) 选区直接压页面底。
        assert_eq!(
            over(t.sel, rgb(t.bg)),
            sel_on_bg,
            "选区/页面底合成色不符（dark={}）",
            t.dark
        );
    }
}

/// 图片占位块：`paint.rs` 的 `UnderKind::ImagePlaceholder` 取 `theme.chrome_border`
/// （深色下浅灰块等同亮斑，所以必须来自主题）。
#[test]
fn image_placeholder_follows_theme_in_all_three_modes() {
    let (light, dark, hc) = (Theme::light(), Theme::dark(), Theme::high_contrast());
    assert_ne!(
        dark.chrome_border, light.chrome_border,
        "深色图片占位描边必须不同于浅色"
    );
    assert_ne!(
        hc.chrome_border, light.chrome_border,
        "high_contrast 图片占位描边不得复用 light"
    );
    assert_ne!(
        hc.chrome_border, dark.chrome_border,
        "high_contrast 图片占位描边不得复用 dark"
    );
    for (name, make) in MODES {
        let t = make();
        // 占位块只需「可见但不喧宾夺主」：与页面底 ≥ 1.4:1（低于非文本元件的 3:1，
        // 因为占位块是空态提示而非可交互控件）。
        assert!(
            contrast(rgb(t.chrome_border), rgb(t.bg)) >= 1.4,
            "{name}: 图片占位/页面底对比度 {:.2} 低于 1.4",
            contrast(rgb(t.chrome_border), rgb(t.bg))
        );
    }
    // 深色占位的亮度必须低（浅色 [208,213,221] 直接搬进深色就是亮斑）。
    assert!(
        luminance(rgb(dark.chrome_border)) < 0.2,
        "深色图片占位亮度应低（防止浅色值写死），实际 {:.3}",
        luminance(rgb(dark.chrome_border))
    );
}

/// 六类元素的八个主题字段两两可区分（light / dark / high_contrast 不共用同一色）。
///
/// 合法共享只发生在「本就应无色/透明」的情形（如 `accent_text` 在 light/dark
/// 都是纯白 —— 那是强调色填充上的反色文字，不属于本矩阵元素）。本矩阵八个字段
/// 都承载可见色，因此任何两个主题共用同一值都视为「主题没真正生效」。
#[test]
fn element_fields_are_pairwise_distinct_across_modes() {
    let (l, d, h) = (Theme::light(), Theme::dark(), Theme::high_contrast());
    let fields: [FieldRow; 8] = [
        ("kbd_bg", l.kbd_bg, d.kbd_bg, h.kbd_bg),
        (
            "kbd_border",
            opaque(l.kbd_border),
            opaque(d.kbd_border),
            opaque(h.kbd_border),
        ),
        ("mark_bg", l.mark_bg, d.mark_bg, h.mark_bg),
        ("code_bg", l.code_bg, d.code_bg, h.code_bg),
        (
            "table_head_bg",
            l.table_head_bg,
            d.table_head_bg,
            h.table_head_bg,
        ),
        ("grid", l.grid, d.grid, h.grid),
        ("sel", l.sel, d.sel, h.sel),
        (
            "chrome_border",
            l.chrome_border,
            d.chrome_border,
            h.chrome_border,
        ),
    ];
    for (name, light, dark, hc) in fields {
        assert_ne!(light, dark, "{name}: light 与 dark 不得共用同一色");
        assert_ne!(light, hc, "{name}: light 与 high_contrast 不得共用同一色");
        assert_ne!(dark, hc, "{name}: dark 与 high_contrast 不得共用同一色");
    }
}

/// 高对比度主题可读性底线：正文/菜单文字按 WCAG AAA ≥ 7:1，次要文字与链接/危险色
/// ≥ 4.5:1，强调色上的反色文字 ≥ 4.5:1。
#[test]
fn high_contrast_meets_readability_floor() {
    let hc = Theme::high_contrast();
    let bg = rgb(hc.bg);
    assert!(
        contrast(hc.fg, bg) >= 7.0,
        "高对比度正文对比度 {:.2} 低于 AAA 7:1",
        contrast(hc.fg, bg)
    );
    assert!(
        contrast(hc.text, bg) >= 7.0,
        "高对比度菜单文字对比度 {:.2} 低于 AAA 7:1",
        contrast(hc.text, bg)
    );
    assert!(
        contrast(hc.muted, bg) >= 4.5,
        "高对比度次要文字对比度 {:.2} 低于 4.5:1",
        contrast(hc.muted, bg)
    );
    assert!(
        contrast(hc.link, bg) >= 4.5,
        "高对比度链接对比度 {:.2} 低于 4.5:1",
        contrast(hc.link, bg)
    );
    assert!(
        contrast(hc.danger, bg) >= 4.5,
        "高对比度危险色对比度 {:.2} 低于 4.5:1",
        contrast(hc.danger, bg)
    );
    assert!(
        contrast(hc.accent_text, hc.accent) >= 4.5,
        "高对比度强调色反色文字对比度 {:.2} 低于 4.5:1",
        contrast(hc.accent_text, hc.accent)
    );
    assert!(
        contrast(hc.fg, rgb(hc.code_bg)) >= 7.0,
        "高对比度代码文字对比度 {:.2} 低于 AAA 7:1",
        contrast(hc.fg, rgb(hc.code_bg))
    );
}

/// 语法高亮 7 个 token 在各自代码块底色（`theme.code_bg`）上的可读性门禁。
///
/// 真实缺陷来源：浅色色板原先直接采用 Nord 的极光色，而那是为深底（#2E3440）设计的
/// 前景色——落到近白 code_bg [245,245,245] 上时 keyword 2.47:1、number 2.60:1、
/// function 1.83:1、type 1.91:1，语法高亮反而让代码更难读。`nord_light.tmTheme`
/// 已按「保持色相族、只降明度」重推导为浅底方案；深色侧则修掉了 punctuation 的
/// 2.44:1。本测试把三主题 × 7 token 全部钉在 ≥ 4.5:1，防止再退回深底色板。
#[test]
fn syntax_tokens_meet_contrast_on_code_bg() {
    for (name, make) in MODES {
        let t = make();
        let bg = rgb(t.code_bg);
        let s = t.syntax;
        let tokens: [(&str, [u8; 3]); 7] = [
            ("keyword", s.keyword),
            ("string", s.string),
            ("comment", s.comment),
            ("number", s.number),
            ("function", s.function),
            ("type_name", s.type_name),
            ("punctuation", s.punctuation),
        ];
        for (token, color) in tokens {
            let ratio = contrast(color, bg);
            assert!(
                ratio >= 4.5,
                "{name}: 语法 token {token} #{:02X}{:02X}{:02X} 在 code_bg {bg:?} 上对比度 {ratio:.2} 低于 4.5",
                color[0],
                color[1],
                color[2]
            );
        }
    }
}
