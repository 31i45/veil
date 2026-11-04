//! chrome 模型/绘制/缓存测试（原 chrome.rs 内联测试平移，labels 访问适配封装）。

use super::*;
use crate::document::ExternalState;
use crate::theme::Theme;
use crate::view::{Frame, LayoutEngine};

fn vp() -> Viewport {
    viewport(900, 700, 1.0)
}

#[test]
fn viewport_excludes_menu_and_status() {
    let v = vp();
    assert_eq!(v.y, MENU_H);
    assert_eq!(v.h, 700.0 - MENU_H - STATUS_H);
    assert_eq!(v.w, 900.0);
}

#[test]
fn menu_bar_hit_and_toggle_zones() {
    let rects = menu_bar_rects(1.0);
    assert_eq!(rects.len(), MENUS.len());
    assert_eq!(rects.len(), 3, "文件/编辑/视图");
    let (_, x0, w0) = rects[0];
    let (_, x1, _) = rects[1];
    let (_, x2, _) = rects[2];
    assert_eq!(hit_menu_bar(x0 + 1.0, 10.0, 1.0), Some(0));
    assert_eq!(hit_menu_bar(x1 + 1.0, 10.0, 1.0), Some(1));
    assert_eq!(hit_menu_bar(x2 + 1.0, 10.0, 1.0), Some(2));
    assert_eq!(
        hit_menu_bar(x0 + w0 - 0.1, 10.0, 1.0),
        Some(0),
        "右缘闭区间外"
    );
    assert_eq!(hit_menu_bar(x2 + 1000.0, 10.0, 1.0), None);
    assert_eq!(
        hit_menu_bar(x0 + 1.0, MENU_H + 1.0, 1.0),
        None,
        "菜单栏以下"
    );
}

#[test]
fn popup_hit_maps_items_and_contains_padding() {
    let p = popup_layout(0, 1.0, &[]);
    assert!(p.h > 0.0);
    assert_eq!(p.y, MENU_H);
    for (i, ix, iy, iw, ih) in &p.items {
        assert_eq!(
            hit_popup_item(ix + iw / 2.0, iy + ih / 2.0, 0, 1.0, &[]),
            Some(*i)
        );
    }
    assert_eq!(
        hit_popup_item(p.x + 1.0, p.y + 1.0, 0, 1.0, &[]),
        None,
        "内边距非项区"
    );
    assert!(popup_contains(p.x + 1.0, p.y + 1.0, 0, 1.0, &[]));
    assert!(!popup_contains(p.x - 1.0, p.y + 1.0, 0, 1.0, &[]));
    assert_eq!(action_at(0, 0, &[]), Some(MenuAction::New));
    assert_eq!(action_at(0, 1, &[]), Some(MenuAction::Open));
    assert_eq!(action_at(0, 99, &[]), None);
}

/// 0b 动态菜单：最近文件插在「退出」之前，布局/命中/分发三者一致。
#[test]
fn file_menu_injects_recent_before_quit() {
    let recent = vec!["a.md".to_string(), "b.md".to_string()];
    let static_len = MENUS[0].items.len();
    let rows = menu_rows(0, &recent);
    assert_eq!(rows.len(), static_len + 2, "静态项 + 两个最近文件");
    assert_eq!(rows[static_len - 1].2, MenuAction::OpenRecent(0));
    assert_eq!(rows[static_len].2, MenuAction::OpenRecent(1));
    assert_eq!(rows[static_len + 1].2, MenuAction::Quit, "退出仍在最后");
    // 分发与布局同源
    assert_eq!(
        action_at(0, static_len - 1, &recent),
        Some(MenuAction::OpenRecent(0))
    );
    assert_eq!(
        action_at(0, static_len + 1, &recent),
        Some(MenuAction::Quit)
    );
    assert_eq!(action_at(0, static_len + 2, &recent), None);
    // 高度随项数增长；项命中与下标一致
    let p0 = popup_layout(0, 1.0, &[]);
    let p1 = popup_layout(0, 1.0, &recent);
    assert_eq!(p1.h - p0.h, 2.0 * ITEM_H);
    for (i, ix, iy, iw, ih) in &p1.items {
        assert_eq!(
            hit_popup_item(ix + iw / 2.0, iy + ih / 2.0, 0, 1.0, &recent),
            Some(*i)
        );
    }
    // 其它菜单不受最近文件影响
    assert_eq!(menu_rows(1, &recent).len(), MENUS[1].items.len());
    // 空最近列表：零动态项
    assert_eq!(menu_rows(0, &[]).len(), static_len);
}

#[test]
fn menu_definitions_cover_shortcuts() {
    let file: Vec<_> = MENUS[0].items.iter().map(|(_, _, a)| *a).collect();
    assert_eq!(
        file,
        vec![
            MenuAction::New,
            MenuAction::Open,
            MenuAction::QuickOpen,
            MenuAction::Save,
            MenuAction::SaveAs,
            MenuAction::Export,
            MenuAction::Print,
            MenuAction::Quit
        ]
    );
    let edit: Vec<_> = MENUS[1].items.iter().map(|(_, _, a)| *a).collect();
    assert_eq!(
        edit,
        vec![
            MenuAction::Undo,
            MenuAction::Redo,
            MenuAction::Cut,
            MenuAction::Copy,
            MenuAction::Paste,
            MenuAction::SelectAll,
            MenuAction::ToggleBold,
            MenuAction::ToggleItalic,
            MenuAction::InsertLink,
            MenuAction::Find,
            MenuAction::Replace,
            MenuAction::InsertImage
        ]
    );
}

#[test]
fn scroll_thumb_none_without_overflow() {
    assert_eq!(scroll_thumb(vp(), 600.0, 0.0, 1.0), None);
    assert_eq!(scroll_thumb(vp(), 100.0, 0.0, 1.0), None);
}

#[test]
fn scroll_thumb_returns_none_for_degenerate_viewport() {
    // 退化视口（模拟最小化后 vp.h=1.0）应直接返回 None 而不是在 clamp(min>max) 处 panic
    let tiny = Viewport {
        x: 0.0,
        y: 30.0,
        w: 800.0,
        h: 1.0,
    };
    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        scroll_thumb(tiny, 1000.0, 0.0, 1.5)
    }));
    assert!(res.is_ok(), "退化为视口不得 panic");
    assert_eq!(res.unwrap(), None);

    // 回归：正常视口仍能返回 Some
    let normal = Viewport {
        x: 0.0,
        y: 30.0,
        w: 800.0,
        h: 600.0,
    };
    assert!(scroll_thumb(normal, 3000.0, 0.0, 1.5).is_some());
}

#[test]
fn scroll_thumb_position_and_mapping_roundtrip() {
    let v = vp();
    let doc = 3000.0;
    let (_, h) = scroll_thumb(v, doc, 0.0, 1.0).unwrap();
    assert!(h >= THUMB_MIN_H && h <= v.h);
    let (y0, _) = scroll_thumb(v, doc, 0.0, 1.0).unwrap();
    assert_eq!(y0, v.y);
    let (y1, _) = scroll_thumb(v, doc, max_scroll(v, doc), 1.0).unwrap();
    assert!((y1 - (v.y + v.h - h)).abs() < 0.01, "底部贴视口底");
    // 中位：滑块顶边 → 滚动量再回算一致
    let mid = (y0 + y1) / 2.0;
    let s = scroll_from_thumb(v, doc, mid, 1.0);
    let (ym, _) = scroll_thumb(v, doc, s, 1.0).unwrap();
    assert!((ym - mid).abs() < 0.01);
    assert!((s - max_scroll(v, doc) / 2.0).abs() < 0.5);
}

#[test]
fn scroll_from_thumb_clamps() {
    let v = vp();
    let doc = 2000.0;
    assert_eq!(scroll_from_thumb(v, doc, -100.0, 1.0), 0.0);
    assert_eq!(
        scroll_from_thumb(v, doc, v.y + v.h + 100.0, 1.0),
        max_scroll(v, doc)
    );
    assert_eq!(scroll_from_thumb(v, 100.0, 50.0, 1.0), 0.0, "无溢出恒 0");
}

#[test]
fn page_scroll_steps_and_clamps() {
    let v = vp();
    let doc = 5000.0;
    assert!(page_scroll(v, doc, 0.0, 1) > 0.0);
    assert_eq!(page_scroll(v, doc, 0.0, -1), 0.0);
    assert_eq!(
        page_scroll(v, doc, max_scroll(v, doc), 1),
        max_scroll(v, doc)
    );
    assert_eq!(page_scroll(v, 100.0, 0.0, 1), 0.0, "无溢出不滚动");
}

#[test]
fn scrollbar_hit_requires_overflow_and_right_edge() {
    let v = vp();
    let x = v.x + v.w - SCROLL_W / 2.0;
    assert!(scrollbar_hit(v, 2000.0, x, v.y + 10.0, 1.0));
    assert!(!scrollbar_hit(v, 600.0, x, v.y + 10.0, 1.0), "不溢出不出现");
    assert!(
        !scrollbar_hit(v, 2000.0, x - SCROLL_W, v.y + 10.0, 1.0),
        "轨道左侧"
    );
    assert!(!scrollbar_hit(v, 2000.0, x, v.y - 1.0, 1.0), "视口上方");
}

#[test]
fn status_line_variants() {
    assert_eq!(
        status_line(None, false, ExternalState::None, 0, 1, 1),
        ("未命名草稿".into(), "0 字 · 行 1，列 1".into())
    );
    assert_eq!(
        status_line(
            Some(Path::new("dir/a.md")),
            true,
            ExternalState::BackedUp,
            1234,
            12,
            3
        ),
        (
            "dir/a.md *（外部修改已备份）".into(),
            "1234 字 · 行 12，列 3".into()
        )
    );
    assert_eq!(
        status_line(Some(Path::new("b.md")), false, ExternalState::None, 0, 2, 7).0,
        "b.md"
    );
}

#[test]
fn status_line_marks_external_states() {
    let p = Some(Path::new("a.md"));
    assert_eq!(
        status_line(p, false, ExternalState::Changed, 0, 1, 1).0,
        "a.md（外部修改）"
    );
    assert_eq!(
        status_line(p, true, ExternalState::Missing, 0, 1, 1).0,
        "a.md *（文件缺失）"
    );
}

#[test]
fn ellipsize_middle_fits_and_keeps_head_tail() {
    // S1：长路径左文超窄可用宽 → 中段省略、头尾保留、宽度达标；宽余充足原样返回
    let mut engine = LayoutEngine::new();
    let long = format!("D:/{}", "很长的目录名/".repeat(12)) + "最终文档名.md *";
    let avail = 200.0;
    let fitted = super::paint::ellipsize_middle(&mut engine, &long, avail);
    assert!(fitted.contains('…'), "超宽应中段省略: {fitted}");
    assert!(fitted.len() < long.len());
    assert!(
        super::paint::measure_w(&mut engine, &fitted) <= avail + 1.0,
        "省略后必须放下"
    );
    // 结构不变量：省略号前后分别是原文前缀与后缀（头尾保留，不依赖具体字宽）
    let head_part = fitted.split('…').next().unwrap();
    let tail_part = fitted.rsplit('…').next().unwrap();
    assert!(long.starts_with(head_part), "头段必须是原文前缀: {fitted}");
    assert!(long.ends_with(tail_part), "尾段必须是原文后缀: {fitted}");
    let short = "a.md";
    assert_eq!(
        super::paint::ellipsize_middle(&mut engine, short, 100_000.0),
        short,
        "宽余充足不省略"
    );
}

#[test]
fn paint_caches_status_fit_across_frames() {
    // S1：超宽场景产生省略缓存；同参数重绘命中缓存不增长（避免每帧二分重排）
    let mut engine = LayoutEngine::new();
    let mut frame = Frame::new(320, 200, [255, 255, 255, 255]);
    let theme = Theme::light();
    let mut chrome = Chrome::default();
    let long_path = Path::new("D:/").join("很长的目录名".repeat(20)).join("文件.md");
    let v = viewport(320, 200, 1.0);
    for _ in 0..2 {
        chrome.paint(
            &mut frame,
            &mut engine,
            v,
            1.0,
            &theme,
            Some(&long_path),
            true,
            ExternalState::None,
            0,
            1,
            1,
            2000.0,
            0.0,
            &[],
            MenuFlags::default(),
        );
    }
    let n = chrome.status_fit.len();
    assert!(n >= 1, "超宽场景应产生省略缓存");
    chrome.paint(
        &mut frame,
        &mut engine,
        v,
        1.0,
        &theme,
        Some(&long_path),
        true,
        ExternalState::None,
        0,
        1,
        1,
        2000.0,
        0.0,
        &[],
        MenuFlags::default(),
    );
    assert_eq!(chrome.status_fit.len(), n, "同参数重绘命中缓存不增长");
}

#[test]
fn paint_draws_menu_status_and_popup() {
    let mut engine = LayoutEngine::new();
    let mut frame = Frame::new(300, 200, [255, 255, 255, 255]);
    let theme = Theme::light();
    let mut chrome = Chrome {
        open: Some(0),
        ..Default::default()
    };
    let v = viewport(300, 200, 1.0);
    chrome.paint(
        &mut frame,
        &mut engine,
        v,
        1.0,
        &theme,
        None,
        false,
        ExternalState::None,
        0,
        1,
        1,
        2000.0,
        0.0,
        &[],
        MenuFlags::default(),
    );
    assert_eq!(frame.get(0, 0), theme.chrome_bg, "菜单栏底色");
    let status_y = 200 - STATUS_H as i32;
    assert_eq!(
        frame.get(0, (status_y + 5) as u32),
        theme.chrome_bg,
        "状态栏底色"
    );
    let p = popup_layout(0, 1.0, &[]);
    assert_eq!(
        frame.get((p.x + p.w / 2.0) as u32, (p.y + p.h / 2.0) as u32),
        theme.popup_bg,
        "弹窗底色"
    );
    assert!(!chrome.labels.is_empty());
    // 缓存复用：同帧重复绘制不增长
    let n = chrome.labels.len();
    chrome.paint(
        &mut frame,
        &mut engine,
        v,
        1.0,
        &theme,
        None,
        false,
        ExternalState::None,
        0,
        1,
        1,
        2000.0,
        0.0,
        &[],
        MenuFlags::default(),
    );
    assert_eq!(chrome.labels.len(), n);
}

#[test]
fn toggle_rows_report_and_paint_checked_state() {
    let flags = MenuFlags {
        focus: true,
        ..Default::default()
    };
    assert!(action_checked(MenuAction::ToggleFocus, flags));
    assert!(!action_checked(MenuAction::ToggleTypewriter, flags));
    assert!(!action_checked(MenuAction::Save, flags), "非开关项无「当前」");
    assert!(!action_checked(
        MenuAction::ToggleOutline,
        MenuFlags::default()
    ), "大纲面板不显示任何状态标识");
    assert!(action_checked(
        MenuAction::ToggleFullscreen,
        MenuFlags {
            fullscreen: true,
            ..Default::default()
        }
    ));

    // 像素级：视图菜单「专注模式」行开启后多出右侧「当前」像素
    let theme = Theme::light();
    let v = viewport(300, 300, 1.0);
    let p = popup_layout(2, 1.0, &[]);
    let (_, ix, iy, iw, ih) = p.items[8];
    let shot = |on: bool| {
        let mut engine = LayoutEngine::new();
        let mut frame = Frame::new(300, 300, [255, 255, 255, 255]);
        let mut chrome = Chrome {
            open: Some(2),
            ..Default::default()
        };
        chrome.paint(
            &mut frame,
            &mut engine,
            v,
            1.0,
            &theme,
            None,
            false,
            ExternalState::None,
            0,
            1,
            1,
            3000.0,
            0.0,
            &[],
            MenuFlags {
                focus: on,
                ..Default::default()
            },
        );
        let mut px = Vec::new();
        for y in iy as u32..(iy + ih) as u32 {
            for x in ix as u32..(ix + iw) as u32 {
                px.push(frame.get(x, y));
            }
        }
        px
    };
    assert_ne!(shot(false), shot(true), "开启态须在行右侧画出「当前」提示");
}

#[test]
fn paint_uses_dark_palette() {
    let mut engine = LayoutEngine::new();
    let mut frame = Frame::new(300, 200, [1, 2, 3, 255]);
    let theme = Theme::dark();
    let mut chrome = Chrome::default();
    chrome.paint(
        &mut frame,
        &mut engine,
        viewport(300, 200, 1.0),
        1.0,
        &theme,
        None,
        false,
        ExternalState::None,
        0,
        1,
        1,
        10.0,
        0.0,
        &[],
        MenuFlags::default(),
    );
    assert_eq!(frame.get(0, 0), theme.chrome_bg, "深色菜单栏");
    assert_eq!(theme.chrome_bg, [38, 38, 38, 255]);
}

#[test]
fn theme_menu_marks_current_mode() {
    assert_eq!(MENUS.len(), 3);
    assert_eq!(action_at(2, 0, &[]), Some(MenuAction::ToggleOutline));
    // 3c-2：视图缩放进视图菜单（大纲面板之后、主题之前）
    assert_eq!(action_at(2, 1, &[]), Some(MenuAction::ZoomIn));
    assert_eq!(action_at(2, 2, &[]), Some(MenuAction::ZoomOut));
    assert_eq!(action_at(2, 3, &[]), Some(MenuAction::ZoomReset));
    assert_eq!(action_at(2, 4, &[]), Some(MenuAction::ToggleFullscreen));
    assert_eq!(action_at(2, 5, &[]), Some(MenuAction::ThemeSystem));
    assert_eq!(action_at(2, 6, &[]), Some(MenuAction::ThemeLight));
    assert_eq!(action_at(2, 7, &[]), Some(MenuAction::ThemeDark));
    assert_eq!(action_at(2, 8, &[]), Some(MenuAction::ToggleFocus));
    assert_eq!(action_at(2, 9, &[]), Some(MenuAction::ToggleTypewriter));
    assert!(theme_action_matches(
        MenuAction::ThemeSystem,
        ThemeMode::System
    ));
    assert!(theme_action_matches(MenuAction::ThemeDark, ThemeMode::Dark));
    assert!(!theme_action_matches(
        MenuAction::ThemeDark,
        ThemeMode::Light
    ));
    assert!(!theme_action_matches(MenuAction::Save, ThemeMode::Dark));
}

#[test]
fn scrollbar_thumb_painted_when_overflowing() {
    let mut engine = LayoutEngine::new();
    let mut frame = Frame::new(300, 200, [255, 255, 255, 255]);
    let theme = Theme::light();
    let mut chrome = Chrome::default();
    let v = viewport(300, 200, 1.0);
    chrome.paint(
        &mut frame,
        &mut engine,
        v,
        1.0,
        &theme,
        None,
        false,
        ExternalState::None,
        0,
        1,
        1,
        3000.0,
        0.0,
        &[],
        MenuFlags::default(),
    );
    let x = (300.0 - SCROLL_W + 2.0) as u32;
    let y = (v.y + 2.0) as u32;
    assert_eq!(frame.get(x, y), theme.thumb);
}
