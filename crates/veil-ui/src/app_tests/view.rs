//! 视图与导航测试：主题配置 / 大纲 / 查找替换 / 链接跳转与悬停 / 字数 / 专注模式。

use super::*;

#[test]
fn startup_applies_configured_mode() {
    let cfg = MemConfig::default();
    *cfg.text.borrow_mut() = Some("[theme]\nmode = \"dark\"\n".into());
    let a = app_theme(Box::new(cfg), Rc::new(Cell::new(Some(false))));
    assert_eq!(a.input_state.theme.mode, ThemeMode::Dark);
    assert!(a.input_state.theme.dark, "显式深色不受系统浅色影响");
    assert!(a.ui_state.chrome.notice.is_empty());
}

#[test]
fn startup_notices_invalid_config_and_uses_default() {
    let cfg = MemConfig::default();
    *cfg.text.borrow_mut() = Some("[theme]\nmode = \"auto\"\nfoo = \"bar\"\n".into());
    let a = app_theme(Box::new(cfg), Rc::new(Cell::new(None)));
    assert_eq!(a.input_state.theme.mode, ThemeMode::System);
    assert!(!a.input_state.theme.dark, "系统外观不可读按浅色");
    assert!(
        a.ui_state.chrome.notice.contains("2 处"),
        "提示无法识别条目数: {}",
        a.ui_state.chrome.notice
    );
}

#[test]
fn system_mode_follows_live_switch_and_clears_color_caches() {
    let sys = Rc::new(Cell::new(Some(false)));
    let mut a = app_theme(Box::new(MemConfig::default()), sys.clone());
    assert!(!a.input_state.theme.dark);
    a.ensure_layouts_all();
    let (id, kind, range) = a.editor.session().blocks()[0].clone();
    let caret = a.editor.caret();
    let base = a.base_key();
    let lk = a
        .cache
        .raster_key(id, &kind, range, base, caret, 0)
        .expect("首块应已按当前键排版");
    a.cache.rasters.insert(
        id,
        Memo {
            key: lk,
            value: (vec![0; 4], 1, 1),
        },
    );
    let ik = a.inline_key();
    a.cache.inline_math.insert(
        "x".into(),
        Memo {
            key: ik,
            value: MathImage {
                pixels: vec![0; 16],
                width: 2,
                height: 2,
                baseline: 0,
            },
        },
    );
    sys.set(Some(true));
    a.poll_system_theme(Instant::now());
    assert!(a.input_state.theme.dark, "系统切深色后调色板跟随");
    assert!(a.cache.rasters.is_empty(), "按旧前景色光栅化的文本须作废");
    assert!(a.cache.inline_math.is_empty(), "公式图须按新前景色重渲");
}

/// DEF-T05-007：设置面板把 Dark 改为 System（系统浅色）后，当前文档调色板完整重绘。
#[test]
fn settings_theme_switch_dark_to_system_repaints_palette() {
    let sys = Rc::new(Cell::new(Some(false)));
    let mut a = app_theme(Box::new(MemConfig::default()), sys);
    a.config.mode = ThemeMode::Dark;
    a.input_state.theme = crate::theme::Theme::of(ThemeMode::Dark, true);
    a.refresh_theme_visuals();
    a.ensure_layouts_all();
    let (id, kind, range) = a.editor.session().blocks()[0].clone();
    let old_base = a.base_key();
    let old_rev = a.input_state.revs.theme;
    a.config.mode = ThemeMode::System;
    a.refresh_settings_effects();
    assert!(!a.input_state.theme.dark, "System(浅色) 应切换为浅色调色板");
    assert!(
        a.input_state.revs.theme > old_rev,
        "主题代次必须递增，旧颜色缓存才会失效"
    );
    assert_ne!(a.base_key(), old_base, "基础键应随主题变化");
    a.ensure_layouts_all();
    let caret = a.editor.caret();
    let base = a.base_key();
    let lk = a
        .cache
        .raster_key(id, &kind, range, base, caret, 0)
        .expect("当前文档应按新主题重排");
    assert_eq!(a.cache.layouts[&id].key.base, base);
    let (px, _, _) = a
        .engine
        .rasterize(&a.cache.layouts[&id].value, a.input_state.theme.fg);
    let fg = a.input_state.theme.fg;
    assert!(
        px.chunks(4)
            .any(|p| p[3] > 0 && p[0] == fg[0] && p[1] == fg[1] && p[2] == fg[2]),
        "光栅应用新主题前景色重绘"
    );
    let _ = lk;
}

/// 字号/行高修改即时生效（无需重启），且进入基础键触发重排。
#[test]
fn settings_typography_change_applies_without_restart() {
    let mut a = app_theme(
        Box::new(MemConfig::default()),
        Rc::new(Cell::new(Some(false))),
    );
    a.ensure_layouts_all();
    let (id, _, _) = a.editor.session().blocks()[0].clone();
    let base0 = a.base_key();
    let h0 = a.cache.layouts[&id].height;
    a.config.doc_font_size = 24.0;
    a.config.doc_line_height = 36.0;
    a.config.doc_font_family = crate::theme::DocFontFamily::Serif;
    a.refresh_settings_effects();
    assert_eq!(a.engine.doc_font_size(), 24.0, "排版引擎立即采用新字号");
    assert_eq!(a.engine.doc_line_height(), 36.0, "排版引擎立即采用新行高");
    assert_eq!(a.engine.doc_font(), crate::theme::DocFontFamily::Serif);
    assert_ne!(a.base_key(), base0, "排版输入必须进入基础键");
    a.ensure_layouts_all();
    assert_ne!(
        a.cache.layouts[&id].height, h0,
        "布局按新字号/行高重排，块高随行高变化"
    );
}

#[test]
fn collapsed_frontmatter_bar_is_clickable() {
    // 4.1：折叠条保留鼠标入口——点击即落光标，光标进入后展开原文。
    let store = MemStore::default();
    let text = "---\ntitle: 演示\n---\n\n正文\n";
    let mut a = app(text, None, false, &store);
    a.editor.set_caret(text.find("正文").unwrap());
    a.ensure_layouts_all();
    a.render();
    let fm = block_id_of(&a, |k| matches!(k, BlockKind::FrontMatter(_)));
    let (fr, ftop) = {
        let g = a.index.blocks.iter().find(|g| g.id == fm).unwrap();
        (g.range, g.top)
    };
    let vp = a.viewport();
    let (x, y) = (a.content_x() + 12.0, vp.y + ftop + 4.0);
    click_at(&mut a, x, y);
    assert!(
        fr.start <= a.editor.caret() && a.editor.caret() <= fr.end,
        "点击折叠条应落光标到 Front Matter 内"
    );
    a.render();
    assert!(
        a.cache.layouts[&fm].layout_text().contains("title"),
        "光标进入后展开原文"
    );
    assert_eq!(doc_text(&a), text, "点击不得改写文档");
}

#[test]
fn generated_toc_entry_click_jumps_to_heading() {
    // 4.2：生成目录条目可点击跳转，源码 `[TOC]` 不被改写。
    let store = MemStore::default();
    let text = "[TOC]\n\n# 第一章\n\n正文\n\n## 小节\n\n更多\n";
    let mut a = app(text, None, false, &store);
    a.editor.set_caret(text.len()); // 光标离开标记块：渲染目录
    a.ensure_layouts_all();
    a.render();
    let toc_top = a
        .index
        .blocks
        .iter()
        .find(|g| g.toc_marker)
        .expect("应有 [TOC] 标记块")
        .top;
    let vp = a.viewport();
    let line_h = a.engine.doc_line_height() * a.input_state.scale;
    let (x, y) = (a.content_x() + 12.0, vp.y + toc_top + line_h * 1.5);
    // 第二条目录行 = 二级标题「小节」
    click_at(&mut a, x, y);
    assert_eq!(
        a.editor.caret(),
        text.find("## 小节").unwrap(),
        "应跳到对应标题块首"
    );
    assert_eq!(doc_text(&a), text, "跳转不得改写 `[TOC]` 源码");
}

#[test]
fn notice_expires_and_clears_code_copy_feedback() {
    // 4.3：notice 超时消退；复制按钮的就地反馈与提示同寿。
    let store = MemStore::default();
    let text = "```rust\nlet x = 1;\n```\n\n正文\n";
    let mut a = app(text, None, false, &store);
    a.ensure_layouts_all();
    let id = block_id_of(&a, |k| matches!(k, BlockKind::FencedCode(_)));
    a.copy_code_block(id);
    assert_eq!(a.window_state.code_copied, Some(id));
    let g = a.index.blocks.iter().find(|g| g.id == id).unwrap();
    assert!(
        a.code_header_layout(g).unwrap().copied,
        "按钮就地显示已复制"
    );
    let t0 = Instant::now();
    a.tick(t0);
    assert!(!a.ui_state.chrome.notice.is_empty());
    a.tick(t0 + Duration::from_secs(7));
    assert!(a.ui_state.chrome.notice.is_empty(), "提示应超时消退");
    assert_eq!(a.window_state.code_copied, None, "就地反馈同步复原");
}

#[test]
fn pointer_shape_follows_hover_target() {
    // 4.3：指针形状映射——正文文本 / 代码复制按钮（手型）。
    use winit::window::CursorIcon;
    let store = MemStore::default();
    let text = "正文\n\n```rust\nlet x = 1;\n```\n";
    let mut a = app(text, None, false, &store);
    a.ensure_layouts_all();
    let vp = a.viewport();
    a.input_state.mouse = PhysicalPosition::new((a.content_x() + 4.0) as f64, (vp.y + 4.0) as f64);
    a.apply_pointer();
    assert_eq!(a.window_state.pointer, Some(CursorIcon::Text));
    let g = a
        .index
        .blocks
        .iter()
        .find(|g| matches!(g.kind, BlockKind::FencedCode(_)));
    let h = a.code_header_layout(g.unwrap()).expect("代码头部");
    a.input_state.mouse = PhysicalPosition::new(
        (h.button.x + h.button.w / 2.0) as f64,
        (h.button.y + h.button.h / 2.0) as f64,
    );
    a.apply_pointer();
    assert_eq!(a.window_state.pointer, Some(CursorIcon::Pointer));
}

#[test]
fn explicit_mode_ignores_system_switch() {
    let cfg = MemConfig::default();
    *cfg.text.borrow_mut() = Some("[theme]\nmode = \"light\"".into());
    let sys = Rc::new(Cell::new(Some(false)));
    let mut a = app_theme(Box::new(cfg), sys.clone());
    sys.set(Some(true));
    a.poll_system_theme(Instant::now());
    assert_eq!(a.input_state.theme.mode, ThemeMode::Light);
    assert!(!a.input_state.theme.dark, "显式浅色不受系统切换影响");
}

#[test]
fn switching_theme_persists_config() {
    let cfg = MemConfig::default();
    let saves = cfg.saves.clone();
    let sys = Rc::new(Cell::new(Some(false)));
    let mut a = app_theme(Box::new(cfg), sys.clone());
    a.perform_menu(MenuAction::ThemeDark);
    assert!(a.input_state.theme.dark);
    assert_eq!(a.input_state.theme.mode, ThemeMode::Dark);
    assert!(a.ui_state.chrome.notice.contains("深色"));
    let saved = saves.borrow();
    assert_eq!(saved.len(), 1, "切换一次即落盘一次");
    assert!(saved[0].contains("mode = \"dark\""));
    drop(saved);
    // 切回跟随系统：立即采用当前系统外观（此处系统已切深色）
    sys.set(Some(true));
    a.perform_menu(MenuAction::ThemeSystem);
    assert_eq!(a.input_state.theme.mode, ThemeMode::System);
    assert!(a.input_state.theme.dark);
    assert!(saves.borrow()[1].contains("mode = \"system\""));
}

#[test]
fn startup_applies_export_config_and_saves_all_keys() {
    let cfg = MemConfig::default();
    *cfg.text.borrow_mut() = Some(
        "mode = \"light\"\nexport_margin_mm = 12.5\nexport_page_numbers = false\nexport_toc = false\n"
            .into(),
    );
    let saves = cfg.saves.clone();
    let mut a = app_theme(Box::new(cfg), Rc::new(Cell::new(Some(false))));
    assert_eq!(a.config.export_margin_mm, 12.5);
    assert!(!a.config.export_page_numbers);
    assert!(!a.config.export_toc);
    a.perform_menu(MenuAction::ThemeDark);
    let saved = saves.borrow();
    assert!(saved[0].contains("export_margin_mm = 12.5"), "{}", saved[0]);
    assert!(saved[0].contains("export_page_numbers = false"));
    assert!(saved[0].contains("export_toc = false"));
}

#[test]
fn startup_applies_doc_typography_and_ignores_legacy_smooth_scroll() {
    let cfg = MemConfig::default();
    *cfg.text.borrow_mut() = Some(
        "doc_font_family = \"serif\"\ndoc_font_size = 18\ndoc_line_height = 27\nsmooth_scroll = true\n"
            .into(),
    );
    let mut a = app_theme(Box::new(cfg), Rc::new(Cell::new(Some(false))));
    // 旧配置的 smooth_scroll 静默忽略：滚动固定为瞬时。
    assert_eq!(a.config.doc_font_family, theme::DocFontFamily::Serif);
    assert_eq!(a.config.doc_font_size, 18.0);
    assert_eq!(a.config.doc_line_height, 27.0);
    // 正文块排版使用配置值（默认 16/22 → 18/27）
    let text = "正文一行";
    let h = a
        .engine
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
        .height;
    assert!(h >= 27.0 - 0.5, "正文块行高应使用配置值：{h}");
    // UI chrome 度量保持 UI 字阶（13/18），不受正文配置影响
    let (_, ui_h) = a.engine.measure("菜单");
    assert!((ui_h - 18.0).abs() < 0.5, "UI 行高应保持不变：{ui_h}");
}

#[test]
fn config_save_failure_keeps_theme_and_notices() {
    let sys = Rc::new(Cell::new(Some(false)));
    let mut a = app_theme(Box::new(FailSaveConfig), sys);
    a.perform_menu(MenuAction::ThemeDark);
    assert!(a.input_state.theme.dark, "保存失败不影响本次生效");
    assert!(
        a.ui_state.chrome.notice.contains("配置未保存"),
        "提示保存失败: {}",
        a.ui_state.chrome.notice
    );
}

/// DEF-T07-002：几何字段参与 apply；清除后按 OK 必须把移除结果写盘。
#[test]
fn settings_geometry_clear_is_applied_and_persisted_on_ok() {
    use crate::view::settings::{FieldKind, FieldValue, BTN_OK};
    let cfg = MemConfig::default();
    let saves = cfg.saves.clone();
    let mut a = app_theme(Box::new(cfg), Rc::new(Cell::new(Some(false))));
    a.config.window = Some(crate::theme::WindowGeom {
        width: 900,
        height: 700,
        x: -40,
        y: 30,
    });
    a.open_settings();

    // 清除按钮等价操作：表单值置 None。
    {
        let s = a.ui_state.settings.as_mut().expect("设置已打开");
        let idx = s
            .form
            .fields
            .iter()
            .position(|f| matches!(f.kind, FieldKind::WindowGeom))
            .expect("几何字段存在");
        s.form.fields[idx].value = FieldValue::Geom(None);
    }
    a.resolve_settings(BTN_OK);

    assert_eq!(a.config.window, None, "清除后 apply 必须移除几何");
    let saved = saves.borrow().last().cloned().expect("OK 必须写盘");
    let parsed = crate::theme::decode_config(saved.as_bytes()).expect("写盘 JSON 可解析");
    assert_eq!(parsed.config.window, None, "写盘内容不得复活旧几何");

    // 反向：恢复默认后再填入一组几何，OK 后写盘包含新值。
    a.config.window = None;
    a.open_settings();
    {
        let s = a.ui_state.settings.as_mut().expect("设置已打开");
        let idx = s
            .form
            .fields
            .iter()
            .position(|f| matches!(f.kind, FieldKind::WindowGeom))
            .expect("几何字段存在");
        s.form.fields[idx].value = FieldValue::Geom(Some((1000, 800, 5, 6)));
    }
    a.resolve_settings(BTN_OK);
    assert_eq!(
        a.config.window,
        Some(crate::theme::WindowGeom {
            width: 1000,
            height: 800,
            x: 5,
            y: 6
        })
    );
    let saved = saves.borrow().last().cloned().expect("OK 必须写盘");
    let parsed = crate::theme::decode_config(saved.as_bytes()).expect("写盘 JSON 可解析");
    assert_eq!(parsed.config.window, a.config.window, "新几何必须写盘");
}

/// DEF-T07-002 复发：本会话 resize 过的几何不得在清除/恢复默认后覆盖关闭写盘。
/// 清除或恢复默认 → OK 后，正常关闭不得写回 window_*；用户再次移动窗口后恢复持久化。
#[test]
fn clearing_geometry_suppresses_geometry_write_on_close() {
    use crate::view::settings::{FieldKind, FieldValue, BTN_OK, BTN_RESET_DEFAULTS};
    let cfg = MemConfig::default();
    let saves = cfg.saves.clone();
    let mut a = app_theme(Box::new(cfg), Rc::new(Cell::new(Some(false))));
    let old = crate::theme::WindowGeom {
        width: 700,
        height: 450,
        x: -40,
        y: 30,
    };
    a.config.window = Some(old);
    // 模拟本会话发生过 resize：关闭时原本会把当前几何写回。
    let resized = crate::theme::WindowGeom {
        width: 1050,
        height: 675,
        x: 0,
        y: 0,
    };
    a.window_state.geom = Some(resized);
    a.window_state.geom_dirty = true;

    // 清除 → OK：丢弃 resize 脏位并清空会话几何。
    a.open_settings();
    {
        let s = a.ui_state.settings.as_mut().expect("设置已打开");
        let idx = s
            .form
            .fields
            .iter()
            .position(|f| matches!(f.kind, FieldKind::WindowGeom))
            .expect("几何字段存在");
        s.form.fields[idx].value = FieldValue::Geom(None);
    }
    a.resolve_settings(BTN_OK);
    assert_eq!(a.config.window, None, "清除后 apply 必须移除几何");
    assert!(
        !a.window_state.geom_dirty,
        "清除后必须丢弃本会话 resize 脏位"
    );
    assert_eq!(a.window_state.geom, None, "清除后不得保留会话几何");

    a.window_state.exit_requested = true;
    assert!(a.finish_close());
    let saved = saves.borrow().last().cloned().expect("OK 已写盘");
    let parsed = crate::theme::decode_config(saved.as_bytes()).expect("写盘 JSON 可解析");
    assert_eq!(
        parsed.config.window, None,
        "正常关闭不得把本会话几何写回（清除结果必须跨重启生效）"
    );

    // 恢复默认 → OK：同样抑制关闭回写。
    a.config.window = Some(old);
    a.window_state.geom = Some(resized);
    a.window_state.geom_dirty = true;
    a.open_settings();
    a.resolve_settings(BTN_RESET_DEFAULTS);
    a.resolve_settings(BTN_OK);
    assert_eq!(a.config.window, None, "恢复默认后 apply 必须移除几何");
    assert!(!a.window_state.geom_dirty, "恢复默认后同样丢弃 resize 脏位");
    a.finish_close();
    let saved = saves.borrow().last().cloned().expect("OK 已写盘");
    let parsed = crate::theme::decode_config(saved.as_bytes()).expect("写盘 JSON 可解析");
    assert_eq!(
        parsed.config.window, None,
        "恢复默认后正常关闭不得写回旧几何"
    );

    // 用户随后主动移动窗口：恢复正常的几何持久化。
    a.on_window_moved(12, 34, false);
    assert!(a.window_state.geom_dirty, "主动移动窗口应重新置脏");
    a.finish_close();
    let saved = saves.borrow().last().cloned().expect("移动后关闭必须写盘");
    let parsed = crate::theme::decode_config(saved.as_bytes()).expect("写盘 JSON 可解析");
    assert!(
        parsed.config.window.is_some(),
        "用户再次移动窗口后必须正常保存新几何"
    );
}

/// DEF-T10-008：设置保存失败时不得再提示「设置已保存」。
#[test]
fn settings_save_failure_is_not_overwritten_by_success_notice() {
    use crate::view::settings::{FieldKind, FieldValue, BTN_OK};
    let mut a = app_theme(Box::new(FailSaveConfig), Rc::new(Cell::new(Some(false))));
    a.open_settings();
    {
        let s = a.ui_state.settings.as_mut().expect("设置已打开");
        let idx = s
            .form
            .fields
            .iter()
            .position(|f| matches!(f.kind, FieldKind::ThemeMode))
            .expect("主题字段存在");
        s.form.fields[idx].value = FieldValue::ThemeMode(ThemeMode::Dark);
    }
    a.resolve_settings(BTN_OK);
    assert_eq!(a.config.mode, ThemeMode::Dark, "本次操作仍然生效");
    assert!(
        a.ui_state.chrome.notice.contains("配置未保存"),
        "失败提示须保留：{}",
        a.ui_state.chrome.notice
    );
    assert!(
        !a.ui_state.chrome.notice.contains("设置已保存"),
        "不得假报保存成功：{}",
        a.ui_state.chrome.notice
    );
}

/// DEF-T10-008：远程图片开关保存失败同样不得假报已持久化。
#[test]
fn remote_image_toggle_save_failure_keeps_error_notice() {
    let mut a = app_theme(Box::new(FailSaveConfig), Rc::new(Cell::new(Some(false))));
    a.toggle_remote_images();
    assert!(a.config.allow_remote_images, "开关本身仍翻转");
    assert!(
        a.ui_state.chrome.notice.contains("配置未保存"),
        "失败提示不得被开关状态提示覆盖：{}",
        a.ui_state.chrome.notice
    );
    assert!(!a.ui_state.chrome.notice.contains("已允许远程图片"));
}

/// DEF-T11-E1-002：从设置面板切深色后，导出样式快照必须同步为深色。
#[test]
fn settings_theme_change_updates_export_style_immediately() {
    use crate::view::settings::{FieldKind, FieldValue, BTN_OK};
    let mut a = app_theme(
        Box::new(MemConfig::default()),
        Rc::new(Cell::new(Some(false))),
    );
    assert!(!a.doc.export_style.dark, "初始浅色");
    a.open_settings();
    {
        let s = a.ui_state.settings.as_mut().expect("设置已打开");
        let idx = s
            .form
            .fields
            .iter()
            .position(|f| matches!(f.kind, FieldKind::ThemeMode))
            .expect("主题字段存在");
        s.form.fields[idx].value = FieldValue::ThemeMode(ThemeMode::Dark);
    }
    a.resolve_settings(BTN_OK);
    assert!(a.input_state.theme.dark);
    assert_eq!(
        a.doc.export_style,
        ExportStyle::from_theme(&a.input_state.theme),
        "导出快照必须与当前主题一致"
    );
    assert!(a.doc.export_style.dark, "切深色后立即导出应为深色");
}

/// DEF-T10-E1-001：最小化期间的 Moved（-32000 哨兵）不得污染持久化几何。
#[test]
fn minimized_window_move_does_not_overwrite_geometry() {
    let store = MemStore::default();
    let mut a = app("abc\n", None, false, &store);
    let original = crate::theme::WindowGeom {
        width: 900,
        height: 700,
        x: 300,
        y: 220,
    };
    a.window_state.geom = Some(original);
    a.window_state.geom_dirty = false;
    a.on_window_moved(-32000, -32000, true);
    assert_eq!(a.window_state.geom, Some(original), "最小化不写坐标");
    assert!(!a.window_state.geom_dirty, "最小化不置脏");
    // 恢复后的正常移动仍记录（尺寸沿用已存值）
    a.on_window_moved(40, 50, false);
    assert_eq!(
        a.window_state.geom,
        Some(crate::theme::WindowGeom {
            width: 900,
            height: 700,
            x: 40,
            y: 50
        })
    );
    assert!(a.window_state.geom_dirty);
}

/// DEF-T07-003：模态/浮层打开时 Ctrl+滚轮不得缩放背景文档。
#[test]
fn overlay_blocks_background_zoom() {
    let store = MemStore::default();
    let mut a = app("abc\n", None, false, &store);
    let base = a.input_state.user_zoom;
    assert!(!a.overlay_blocks_background_wheel());

    a.open_settings();
    assert!(a.overlay_blocks_background_wheel(), "设置打开必须拦截滚轮");
    a.resolve_settings(crate::view::settings::BTN_CANCEL);
    assert!(!a.overlay_blocks_background_wheel());

    a.open_search();
    assert!(a.overlay_blocks_background_wheel(), "查找打开必须拦截滚轮");
    a.close_search();

    let spec = crate::view::modal::ModalSpec::new("T", "M", Vec::new());
    a.ui_state.modal = Some(crate::app::ActiveModal {
        state: crate::view::modal::ModalState::new(&spec),
        spec,
        intent: None,
        purpose: crate::app::ModalPurpose::Unsaved,
    });
    assert!(a.overlay_blocks_background_wheel(), "模态打开必须拦截滚轮");
    a.ui_state.modal = None;
    assert_eq!(a.input_state.user_zoom, base, "守卫本身不得改变缩放");
}

/// DEF-T07-004：窗口失焦必须结束列宽拖动会话。
#[test]
fn window_blur_ends_column_drag() {
    let store = MemStore::default();
    let mut a = app("| A | B |\n| --- | --- |\n| 1 | 2 |\n", None, false, &store);
    a.ui_state.tables.drag = Some((BlockId(1), 0, 10.0, 80.0));
    a.on_window_focus(false);
    assert!(a.ui_state.tables.drag.is_none(), "失焦后拖动会话必须清空");
    assert!(!a.window_state.visible);

    a.on_window_focus(true);
    assert!(a.ui_state.tables.drag.is_none(), "回焦不得复活拖动");
}

/// DEF-T67-E1-005：识别图片命中，并区分正常/缺失图片。
#[test]
fn image_context_detects_missing_local_image() {
    let store = MemStore::default();
    let dir = std::env::temp_dir().join("veil-task06-image-missing");
    let doc = dir.join("a.md");
    let a = app(
        "![图](gone.png)\n\n![图](also-gone.png)\n",
        Some(doc.to_str().unwrap()),
        false,
        &store,
    );
    // 两个图片都指向不存在的本地文件 → missing=true，且能取到源路径。
    assert_eq!(a.image_at(2), Some(("gone.png".to_string(), true)));
    assert_eq!(a.image_at(0), Some(("gone.png".to_string(), true)));

    let plain = app("无图片\n", None, false, &store);
    assert!(plain.image_at(0).is_none());
}

#[test]
fn outline_toggle_lists_visible_headings_and_shrinks_content_width() {
    let store = MemStore::default();
    let (mut a, _) = app_dialogs(
        "# 一\n\n正文\n\n## **二** and `c`\n\n###\n",
        None,
        false,
        &store,
        None,
        None,
    );
    let w0 = a.content_width();
    a.toggle_outline();
    assert!(a.ui_state.outline.open, "切换后面板开启");
    assert!(
        (w0 - a.content_width() - outline::PANEL_W * a.input_state.scale).abs() < 0.01,
        "版心宽须减去面板宽"
    );
    assert_eq!(a.ui_state.outline.entries.len(), 2, "空标题不生成条目");
    assert_eq!(a.ui_state.outline.entries[0].level, 1);
    assert_eq!(a.ui_state.outline.entries[0].text, "一");
    assert_eq!(a.ui_state.outline.entries[1].level, 2);
    assert_eq!(
        a.ui_state.outline.entries[1].text, "二 and c",
        "inline 标记须剥除"
    );

    // 开启状态下编辑：条目随块结构重建
    let dirty = a
        .editor
        .apply(Intent::InsertText("# 新章\n\n".into()))
        .unwrap();
    a.after_edit(dirty);
    assert_eq!(a.ui_state.outline.entries.len(), 3);
    assert_eq!(a.ui_state.outline.entries[0].text, "新章");

    // 悬停：标题区不命中、首行命中
    let panel = a.outline_rect().expect("面板开启");
    a.input_state.mouse = PhysicalPosition::new(
        (panel.x + 10.0) as f64,
        (panel.y + outline::HEAD_H * a.input_state.scale + 5.0) as f64,
    );
    a.update_hover();
    assert_eq!(a.ui_state.outline.hover, Some(0));
    a.input_state.mouse = PhysicalPosition::new((panel.x + 10.0) as f64, (panel.y + 5.0) as f64);
    a.update_hover();
    assert_eq!(a.ui_state.outline.hover, None);

    // 菜单动作与再次切换：条目清空、版心恢复
    assert!(!a.perform_menu(MenuAction::ToggleOutline));
    assert!(!a.ui_state.outline.open);
    assert!(a.ui_state.outline.entries.is_empty());
    assert!((a.content_width() - w0).abs() < 0.01);
}

#[test]
fn outline_click_jumps_caret_and_scrolls_heading_to_top() {
    let mut text = String::from("# 开头\n");
    for i in 0..40 {
        text.push_str(&format!("\n段落 {i}\n"));
    }
    text.push_str("\n# 结尾\n");
    let store = MemStore::default();
    let (mut a, _) = app_dialogs(&text, None, false, &store, None, None);
    a.toggle_outline();
    let want_start = text.find("# 结尾").unwrap();
    let panel = a.outline_rect().expect("面板开启");
    let rows = outline::visible_rows(
        panel,
        a.ui_state.outline.entries.len(),
        0.0,
        a.input_state.scale,
    );
    let row = &rows[1];
    click_at(&mut a, row.x + 4.0, row.y + row.h / 2.0);
    assert_eq!(a.editor.caret(), want_start, "光标落在标题块起点");
    assert!(a.window_state.scroll_y > 0.0, "远处标题跳到视口顶");
    assert_eq!(a.ui_state.outline.hover, None, "点击不残留悬停");
}

#[test]
fn outline_wheel_scrolls_panel_only_when_pointer_inside() {
    let mut text = String::from("# 目录\n");
    for i in 0..40 {
        text.push_str(&format!("\n## 节 {i}\n\n正文\n"));
    }
    let store = MemStore::default();
    let (mut a, _) = app_dialogs(&text, None, false, &store, None, None);
    a.toggle_outline();
    let panel = a.outline_rect().expect("面板开启");
    assert!(
        outline::max_scroll(panel, a.ui_state.outline.entries.len(), a.input_state.scale) > 0.0,
        "条数须超出面板高度"
    );
    let doc_y = a.window_state.scroll_y;
    a.input_state.mouse = PhysicalPosition::new(
        (panel.x + 20.0) as f64,
        (panel.y + outline::HEAD_H * a.input_state.scale + 40.0) as f64,
    );
    a.on_wheel(-60.0);
    assert!(a.ui_state.outline.scroll > 0.0, "指针在面板内滚动面板");
    assert_eq!(a.window_state.scroll_y, doc_y, "面板滚动不影响正文");

    // 面板外：滚正文，面板偏移不变
    let panel_scroll = a.ui_state.outline.scroll;
    a.input_state.mouse = PhysicalPosition::new(100.0, (panel.y + 120.0) as f64);
    a.on_wheel(-60.0);
    assert!(a.window_state.scroll_y > doc_y, "面板外滚动正文");
    assert_eq!(a.ui_state.outline.scroll, panel_scroll);
}

#[test]
fn outline_panel_paints_only_when_open() {
    let store = MemStore::default();
    let (mut a, _) = app_dialogs("# 一\n\n正文\n", None, false, &store, None, None);
    a.toggle_outline();
    let (w, h) = a.win_size();
    let p = a.outline_rect().expect("面板开启");
    let mut f = Frame::new(w, h, a.input_state.theme.bg);
    a.paint_outline(&mut f);
    assert_eq!(
        f.get((p.x + 4.0) as u32, (p.y + 4.0) as u32),
        a.input_state.theme.popup_bg,
        "标题区底色（卡片式面板与浮层同色）"
    );
    assert_eq!(
        f.get((p.x - 30.0) as u32, (p.y + 4.0) as u32),
        a.input_state.theme.bg,
        "面板左侧（投影范围外）仍为文档底色"
    );
    // 关闭后面板不再绘制
    a.toggle_outline();
    let mut g = Frame::new(w, h, a.input_state.theme.bg);
    a.paint_outline(&mut g);
    assert_eq!(
        g.get((p.x + 4.0) as u32, (p.y + 4.0) as u32),
        a.input_state.theme.bg
    );
}

// ===== 文档内查找栏（P3-3） =====

#[test]
fn search_open_finds_all_and_walks_matches() {
    let store = MemStore::default();
    let mut a = app("alpha beta\nGamma alpha\n", None, false, &store);
    a.ensure_layouts_all();
    a.open_search();
    assert!(a.ui_state.search.open);
    assert!(a.ui_state.search.query.is_empty());
    a.search_input("alpha");
    // 大小写不敏感 + 全文扫描：alpha(0..5)、alpha(17..22)
    assert_eq!(
        a.ui_state.search.matches,
        vec![Range::new(0, 5), Range::new(17, 22)]
    );
    assert_eq!(a.editor.caret(), 0, "查询变化后跳到首个命中");
    assert_eq!(a.ui_state.search.label(), "1/2");

    a.search_next();
    assert_eq!(a.ui_state.search.current, 1);
    assert_eq!(a.editor.caret(), 17, "下一个命中");
    a.search_next();
    assert_eq!(a.editor.caret(), 0, "末尾循环回首个");
    a.search_prev();
    assert_eq!(a.editor.caret(), 17, "首部循环到末个");
    assert_eq!(a.ui_state.search.label(), "2/2");

    a.search_backspace();
    assert_eq!(a.ui_state.search.query, "alph");
    assert_eq!(a.editor.caret(), 0, "退格后重算并回首个");
    a.close_search();
    assert!(!a.ui_state.search.open);
    assert_eq!(a.editor.caret(), 0, "关闭时光标停在当前命中");
}

#[test]
fn case_toggle_click_rescans_and_updates_label() {
    let store = MemStore::default();
    let mut a = app("Foo foo", None, false, &store);
    a.ensure_layouts_all();
    a.open_search();
    a.search_input("foo");
    assert_eq!(a.ui_state.search.label(), "1/2", "默认不区分大小写");
    let l = a.search_rect().expect("查找栏开启");
    let (x, y) = (l.case.x + l.case.w / 2.0, l.case.y + l.case.h / 2.0);
    click_at(&mut a, x, y);
    assert!(a.ui_state.search.case_sensitive);
    assert_eq!(a.ui_state.search.label(), "1/1", "开启后 n/m 立即重算");
    assert_eq!(a.editor.caret(), 4, "跳到区分大小写后的首个命中");
    click_at(&mut a, x, y);
    assert!(!a.ui_state.search.case_sensitive);
    assert_eq!(a.ui_state.search.label(), "1/2");
    assert_eq!(a.editor.caret(), 0);
}

#[test]
fn search_is_read_only_for_document() {
    let store = MemStore::default();
    let mut a = app("abc\n", None, false, &store);
    a.ensure_layouts_all();
    let before = doc_text(&a);
    a.open_search();
    a.route_key(Key::Text("q".into()));
    assert_eq!(a.ui_state.search.query, "q");
    a.route_key(Key::Backspace);
    a.route_key(Key::Text("a".into()));
    assert_eq!(a.ui_state.search.query, "a");
    assert_eq!(doc_text(&a), before, "查找期间文档只读");
    assert!(!a.editor.is_dirty(), "查找不置脏");
    a.route_key(Key::Escape);
    assert!(!a.ui_state.search.open);
}

#[test]
fn search_keys_handle_enter_shift_enter_and_empty_result() {
    let store = MemStore::default();
    let mut a = app("x x x\n", None, false, &store);
    a.ensure_layouts_all();
    a.open_search();
    a.route_key(Key::Text("x".into()));
    assert_eq!(a.editor.caret(), 0);
    a.route_key(Key::Enter);
    assert_eq!(a.ui_state.search.current, 1);
    a.input_state.mods.shift = true;
    a.route_key(Key::Enter);
    assert_eq!(a.ui_state.search.current, 0, "Shift+Enter 走上一个");
    a.input_state.mods.shift = false;
    a.route_key(Key::Text("z".into())); // 查询「xz」：无命中
    assert!(a.ui_state.search.matches.is_empty());
    assert_eq!(a.ui_state.search.label(), "0/0");
    assert_eq!(a.editor.caret(), 0, "无命中不移动光标");
}

#[test]
fn mode_and_text_sink_derive_from_one_priority() {
    let store = MemStore::default();
    let mut a = app("abc\n", None, false, &store);
    assert_eq!(a.mode(), Mode::Document);
    assert_eq!(a.text_sink(), TextSink::Doc);

    a.ui_state.chrome.open = Some(0);
    assert_eq!(a.mode(), Mode::Menu, "菜单优先于查找栏");
    assert_eq!(a.text_sink(), TextSink::Doc);

    a.open_search(); // 既有行为：打开查找栏先关菜单
    assert_eq!(a.mode(), Mode::Search);
    assert_eq!(a.text_sink(), TextSink::Query);

    a.close_search();
    assert_eq!(a.mode(), Mode::Document);
}

#[test]
fn ime_commit_follows_text_sink() {
    let store = MemStore::default();
    let mut a = app("abc\n", None, false, &store);
    a.ensure_layouts_all();

    // 菜单打开：提交被吞掉，文档与查询串都不变
    a.ui_state.chrome.open = Some(0);
    a.ime_commit("菜".into());
    assert_eq!(doc_text(&a), "abc\n");
    assert!(a.ui_state.search.query.is_empty());

    // 查找栏打开：组合串提交进入查询串，文档只读
    a.open_search();
    a.ime_preedit("zh".into(), None);
    assert!(a.editor.preedit().is_some(), "组合串唯一存储在 Editor");
    a.ime_commit("中".into());
    assert_eq!(a.ui_state.search.query, "中");
    assert_eq!(doc_text(&a), "abc\n", "查找期间文档只读");
    assert!(a.editor.preedit().is_none(), "提交后组合串清除");

    // 关闭查找栏：提交回到文档
    a.close_search();
    a.ime_commit("文".into());
    assert_eq!(doc_text(&a), "文abc\n", "文档提交生效");
}

#[test]
fn ime_commit_is_separate_undo_step_after_typing() {
    let store = MemStore::default();
    let mut a = app("", None, false, &store);
    a.ensure_layouts_all();
    for ch in ["a", "b", "c"] {
        let dirty = a.editor.apply(Intent::InsertText(ch.into())).unwrap();
        a.after_edit(dirty);
    }
    assert_eq!(a.editor.session().undo_depth(), 1, "连续键入合成一步");

    a.ime_preedit("zhongzhong".into(), Some((0, 10)));
    a.ime_commit("中".into());
    assert_eq!(doc_text(&a), "abc中");
    assert_eq!(
        a.editor.session().undo_depth(),
        2,
        "上屏必须另起一步，不得并入键入"
    );

    let dirty = a.editor.apply(Intent::Undo).unwrap();
    a.after_edit(dirty);
    assert_eq!(doc_text(&a), "abc", "一次撤销精确回到上屏前");
    let dirty = a.editor.apply(Intent::Undo).unwrap();
    a.after_edit(dirty);
    assert_eq!(doc_text(&a), "", "第二次撤销才回退键入");
}

#[test]
fn ime_multichar_commit_is_single_undo_step() {
    let store = MemStore::default();
    let mut a = app("abc", None, false, &store);
    a.ensure_layouts_all();
    a.editor.apply(Intent::MoveDocEnd).unwrap();

    a.ime_preedit("zhongwen".into(), Some((0, 8)));
    a.ime_commit("中文".into());
    assert_eq!(doc_text(&a), "abc中文");

    let dirty = a.editor.apply(Intent::Undo).unwrap();
    a.after_edit(dirty);
    assert_eq!(doc_text(&a), "abc", "多字符上屏也是一步可撤销");
}

#[test]
fn empty_ime_commit_is_noop_with_clean_document() {
    let store = MemStore::default();
    let mut a = app("abc\n", None, false, &store);
    a.ensure_layouts_all();
    a.editor.apply(Intent::MoveDocEnd).unwrap();
    let depth = a.editor.session().undo_depth();

    a.ime_commit(String::new());

    assert_eq!(doc_text(&a), "abc\n");
    assert!(!a.editor.is_dirty(), "空提交不得置脏");
    assert_eq!(a.editor.session().undo_depth(), depth, "空提交不进撤销栈");
    assert!(a.editor.preedit().is_none());
}

#[test]
fn empty_ime_commit_keeps_selection() {
    let store = MemStore::default();
    let mut a = app("abc\n", None, false, &store);
    a.ensure_layouts_all();
    a.editor.apply(Intent::SelectAll).unwrap();
    let sel = a.editor.selection().expect("全选后应有选区");

    a.ime_commit(String::new());

    assert_eq!(doc_text(&a), "abc\n", "空提交不得删除选区内容");
    assert_eq!(a.editor.selection(), Some(sel), "选区保持");
    assert!(!a.editor.is_dirty(), "空提交不得置脏");
}

#[test]
fn ime_preedit_cleared_when_commit_lands_in_menu_or_history() {
    let store = MemStore::default();
    let mut a = app("abc\n", None, false, &store);
    a.ensure_layouts_all();

    // 菜单：preedit 不挂载，提交被吞掉
    a.ui_state.chrome.open = Some(0);
    a.ime_preedit("zh".into(), None);
    assert!(a.editor.preedit().is_none(), "菜单打开时不挂组合串");
    a.editor.set_preedit("zh".into(), None); // 模拟切菜单前已存在的组合串
    a.ime_commit("中".into());
    assert!(a.editor.preedit().is_none(), "提交落入菜单后清除组合态");
    assert_eq!(doc_text(&a), "abc\n", "菜单吞掉上屏");

    // 版本历史：同样不挂载、不留残影
    a.ui_state.chrome.open = None;
    a.ui_state.history.open = true;
    a.ime_preedit("zh".into(), None);
    assert!(a.editor.preedit().is_none(), "历史预览不挂组合串");
    a.editor.set_preedit("zh".into(), None);
    a.ime_commit("中".into());
    assert!(a.editor.preedit().is_none(), "提交落入历史后清除组合态");
    assert_eq!(doc_text(&a), "abc\n", "历史预览保持只读");

    // IME Disabled 也必须收掉组合态
    a.ui_state.history.open = false;
    a.ime_preedit("zh".into(), None);
    assert!(a.editor.preedit().is_some());
    a.ime_disabled();
    assert!(a.editor.preedit().is_none());
}

#[test]
fn ime_preedit_cleared_when_modal_opens() {
    let store = MemStore::default();
    let mut a = app("abc\n", None, false, &store);
    a.editor.set_preedit("zh".into(), None);
    assert!(a.editor.preedit().is_some());

    let spec = crate::view::modal::ModalSpec::new("T", "M", Vec::new());
    a.ui_state.modal = Some(crate::app::ActiveModal {
        state: crate::view::modal::ModalState::new(&spec),
        spec,
        intent: None,
        purpose: crate::app::ModalPurpose::Unsaved,
    });
    a.sync_ime_for_modal();

    assert!(a.editor.preedit().is_none(), "模态打开必须清除组合态");
    a.ime_commit("中".into());
    assert_eq!(doc_text(&a), "abc\n", "模态吞掉上屏");
}

#[test]
fn settings_excludes_ime_text_from_document() {
    let store = MemStore::default();
    let mut a = app("abc\n", None, false, &store);
    a.editor.set_preedit("zh".into(), None);

    a.open_settings();
    assert!(a.editor.preedit().is_none(), "设置打开必须清除组合态");
    a.ime_commit("2".into());

    assert_eq!(doc_text(&a), "abc\n", "设置打开时 IME 上屏不得写入正文");
    assert!(a.ui_state.settings.is_some());
}

#[test]
fn search_bar_clicks_run_buttons_and_swallow_text_area() {
    let store = MemStore::default();
    let mut a = app("x x x\n", None, false, &store);
    a.ensure_layouts_all();
    a.open_search();
    a.search_input("x");
    let l = a.search_rect().expect("查找栏开启");
    let center = |r: crate::geometry::Rect| (r.x + r.w / 2.0, r.y + r.h / 2.0);
    let (nx, ny) = center(l.next);
    click_at(&mut a, nx, ny);
    assert_eq!(a.ui_state.search.current, 1, "下一个按钮");
    let (px, py) = center(l.prev);
    click_at(&mut a, px, py);
    assert_eq!(a.ui_state.search.current, 0, "上一个按钮");
    let caret = a.editor.caret();
    let (tx, ty) = center(l.text);
    click_at(&mut a, tx, ty);
    assert_eq!(a.editor.caret(), caret, "栏内空白吞掉点击，不落文档光标");
    let (cx, cy) = center(l.close);
    click_at(&mut a, cx, cy);
    assert!(!a.ui_state.search.open, "关闭按钮");
}

#[test]
fn winit_key_chain_opens_search_and_types_into_query() {
    let store = MemStore::default();
    let mut a = app("alpha beta alpha\n", None, false, &store);
    a.ensure_layouts_all();

    // Ctrl+F：Windows 的 event.text 是剥离 Ctrl 的 "f"，必须落到无修饰键分支
    let key = map_key_parts(
        Some("f"),
        WKey::Character("f".into()),
        Mods {
            ctrl: true,
            shift: false,
            alt: false,
        },
    )
    .expect("Ctrl+F 恒可映射");
    assert_eq!(key, Key::F, "Ctrl 组合不得走文本分支");
    a.input_state.mods.ctrl = true;
    a.route_key(key);
    a.input_state.mods.ctrl = false;
    assert!(a.ui_state.search.open, "Ctrl+F 打开查找栏");
    assert_eq!(a.mode(), Mode::Search);

    // 普通字母：winit 文本路径 → 进查询串，文档保持只读
    let key = map_key_parts(Some("a"), WKey::Character("a".into()), Mods::default())
        .expect("字母恒可映射");
    assert_eq!(key, Key::Text("a".into()));
    a.route_key(key);
    assert_eq!(a.ui_state.search.query, "a");
    assert_eq!(doc_text(&a), "alpha beta alpha\n", "查找期间文档只读");
}

#[test]
fn ctrl_h_first_opens_then_closes_then_reopens() {
    let store = MemStore::default();
    let mut a = app("a x a\n", None, false, &store);
    a.ensure_layouts_all();

    // 首次 Ctrl+H：打开 search+replace，focus 在 Query（首字不得落进替换框）
    a.input_state.mods.ctrl = true;
    a.route_key(Key::H);
    a.input_state.mods.ctrl = false;
    assert!(a.ui_state.search.open && a.ui_state.search.replace_open);
    assert_eq!(
        a.ui_state.search.focus,
        search::Field::Query,
        "首次 Ctrl+H 先聚焦查询，首字不得落进替换框"
    );
    a.route_key(Key::Text("a".into()));
    assert_eq!(a.ui_state.search.matches.len(), 2, "查询即时出命中");

    // 第二次 Ctrl+H：replace_open 已是 true → 关闭整个 search 面板（toggle 语义）
    a.input_state.mods.ctrl = true;
    a.route_key(Key::H);
    a.input_state.mods.ctrl = false;
    assert!(
        !a.ui_state.search.open,
        "已打开 replace 时再按 Ctrl+H 关闭整个面板"
    );

    // 第三次 Ctrl+H：search 已关 → 重新打开 search+replace，focus 又到 Query
    a.input_state.mods.ctrl = true;
    a.route_key(Key::H);
    a.input_state.mods.ctrl = false;
    assert!(a.ui_state.search.open && a.ui_state.search.replace_open);
    assert_eq!(
        a.ui_state.search.focus,
        search::Field::Query,
        "重新打开仍先聚焦查询"
    );

    // Search 模式下 Ctrl+F 同样是 toggle：search 已开 → 关闭
    a.input_state.mods.ctrl = true;
    a.route_key(Key::F);
    a.input_state.mods.ctrl = false;
    assert!(
        !a.ui_state.search.open,
        "Search 模式下 Ctrl+F 也按 toggle 关闭"
    );
}

#[test]
fn right_click_on_link_text_opens_link_menu() {
    // 真机走查发现：折叠态链接被整体命中为节点尾（range.end 不含），
    // 右键落在“链接”文字上却只出 Block 菜单，「复制链接地址」不可达。
    let (mut a, _) = app_math("[链接](https://example.com)\n", false);
    assert!(a.editor.set_caret(0));
    a.ensure_layouts_all();
    let id = a.editor.session().blocks()[0].0;
    let top = a
        .index
        .blocks
        .iter()
        .find(|g| g.id == id)
        .expect("正文块应存在")
        .top;
    let scale = a.input_state.scale;
    let vp = a.viewport();
    let (lx, ly, _) = {
        let l = &a.cache.layouts[&id];
        a.engine.caret_xy(l, 1)
    };
    a.input_state.mouse = PhysicalPosition::new(
        (MARGIN * scale + lx + 2.0) as f64,
        (vp.y + top - a.window_state.scroll_y + ly + 5.0) as f64,
    );
    a.on_right_click();
    assert!(a.ui_state.ctx.open, "应打开右键菜单");
    assert!(
        a.ui_state.ctx.link.is_some(),
        "链接文字上右键应出「复制链接地址」项集"
    );
}

// ---- 链接打开：Ctrl+左键 / 右键菜单（协议白名单 + 失败提示） ----

/// DEF-T67-E1-004：危险协议链接右键仍出 Link 专用项；点击打开有可见提示且不调 opener。
#[test]
fn right_click_dangerous_link_shows_link_items_and_visible_hint() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let mut a = link_app("[x](javascript:alert(1))\n", &calls, Ok(()));
    let (x, y) = link_point(&mut a);
    a.input_state.mouse = PhysicalPosition::new(x as f64, y as f64);
    a.on_right_click();
    assert!(a.ui_state.ctx.open);
    assert_eq!(
        a.ui_state.ctx.link.as_deref(),
        Some("javascript:alert(1)"),
        "危险协议也必须进入 Link 上下文"
    );

    let items = a.ui_state.ctx.items().to_vec();
    let open_i = items
        .iter()
        .position(|&(_, act)| act == ctx_menu::CtxAction::OpenLink)
        .expect("必须提供打开链接项");
    assert!(items
        .iter()
        .any(|&(_, act)| act == ctx_menu::CtxAction::CopyLink));

    let vp = a.viewport();
    let l = ctx_menu::layout(
        a.ui_state.ctx.items(),
        a.ui_state.ctx.x,
        a.ui_state.ctx.y,
        vp.x + vp.w,
        vp.y + vp.h,
        a.input_state.scale,
    );
    let iy = l.y
        + l.rows
            .iter()
            .find(|&&(i, _)| i == open_i)
            .map(|&(_, off)| off)
            .expect("打开项可点击")
        + 1.0;
    click_at(&mut a, l.x + 2.0, iy);

    assert!(calls.borrow().is_empty(), "危险协议不得交给系统 opener");
    assert!(
        a.ui_state.chrome.notice.contains("无法打开链接"),
        "状态栏必须给出可见提示：{:?}",
        a.ui_state.chrome.notice
    );
}

#[test]
fn ctrl_click_footnote_ref_jumps_to_definition_without_editing() {
    let text = "正文[^a]引用\n\n[^a]: 定义内容\n";
    let mut a = app_with(text, None, false, &MemStore::default(), no_url());
    a.ensure_layouts_all();
    let def_start = a
        .editor
        .session()
        .footnote_definition("a")
        .expect("定义应已注册")
        .start;
    let bytes0 = a.editor.session().serialize();
    let undo0 = a.editor.session().undo_depth();
    let ref_start = first_ref_start(&a, 0);

    ctrl_click_doc(&mut a, ref_start);

    assert_eq!(a.editor.caret(), def_start, "光标落到定义内容起点");
    assert_eq!(a.editor.session().serialize(), bytes0, "文档字节不变");
    assert_eq!(a.editor.session().undo_depth(), undo0, "不产生撤销步");
}

#[test]
fn ctrl_click_footnote_back_marker_jumps_to_first_reference() {
    let text = "甲[^a] 乙[^a]\n\n[^a]: 尾注\n";
    let mut a = app_with(text, None, false, &MemStore::default(), no_url());
    a.ensure_layouts_all();
    let first_ref = first_ref_start(&a, 0);
    let def_idx = a
        .index
        .blocks
        .iter()
        .position(|g| g.kind == BlockKind::FootnoteDef)
        .expect("应有定义块");
    let lit = {
        let id = a.index.blocks[def_idx].id;
        a.cache.layouts[&id].layout_text().to_string()
    };
    assert!(lit.contains('↩'), "定义块末应渲染返回标记: {lit:?}");
    // `↩` 插在块末正文之后（零宽），其锚点即块 range.end。
    let back = a.index.blocks[def_idx].range.end;
    ctrl_click_doc(&mut a, back);

    assert_eq!(a.editor.caret(), first_ref, "回到文档顺序首个引用");
    assert_eq!(a.editor.session().serialize(), text, "跳转不改文档");
}

#[test]
fn ctrl_click_anchor_link_jumps_to_heading() {
    let text = "# 目标标题\n\n[去](#目标标题)\n";
    let mut a = app_with(text, None, false, &MemStore::default(), no_url());
    a.editor.set_caret(a.editor.session().text_len());

    ctrl_click_doc(&mut a, text.find('去').expect("应找到链接文字"));

    assert_eq!(a.editor.caret(), 0, "跳到标题块起点");
    assert_eq!(a.editor.session().serialize(), text, "跳转不改文档");
}

#[test]
fn ctrl_click_anchor_duplicate_heading_matches_suffix_slug() {
    let text = "# A\n\n# A\n\n[x](#a-1)\n";
    let mut a = app_with(text, None, false, &MemStore::default(), no_url());
    let heads = heading_starts(&a);
    assert_eq!(heads.len(), 2, "两个同名标题均应参与去重编号");
    a.editor.set_caret(a.editor.session().text_len());

    ctrl_click_doc(&mut a, text.find('x').expect("应找到链接文字"));

    assert_eq!(a.editor.caret(), heads[1], "#a-1 命中第二个同名标题");
}

#[test]
fn ctrl_click_undefined_footnote_keeps_caret_and_reports() {
    let text = "x[^nope]\n";
    let mut a = app_with(text, None, false, &MemStore::default(), no_url());
    let caret0 = a.editor.session().text_len();
    a.editor.set_caret(caret0);

    ctrl_click_doc(&mut a, text.find("[^nope]").expect("引用起点"));

    assert_eq!(a.editor.caret(), caret0, "光标不变");
    assert_eq!(
        a.ui_state.chrome.notice,
        "未找到脚注定义；下一步：检查脚注名称是否与引用一致"
    );
}

#[test]
fn ctrl_click_missing_anchor_keeps_caret_and_reports() {
    let text = "[x](#nope)\n";
    let mut a = app_with(text, None, false, &MemStore::default(), no_url());
    let caret0 = a.editor.session().text_len();
    a.editor.set_caret(caret0);

    ctrl_click_doc(&mut a, text.find('x').expect("应找到链接文字"));

    assert_eq!(a.editor.caret(), caret0, "光标不变");
    assert_eq!(
        a.ui_state.chrome.notice,
        "未找到锚点；下一步：检查锚点名称是否与标题一致"
    );
}

#[test]
fn plain_click_footnote_ref_does_not_jump() {
    let text = "正文[^a]引用\n\n[^a]: 定义内容\n";
    let mut a = app_with(text, None, false, &MemStore::default(), no_url());
    let def_start = a
        .editor
        .session()
        .footnote_definition("a")
        .expect("定义应已注册")
        .start;
    a.editor.set_caret(a.editor.session().text_len());
    let ref_start = first_ref_start(&a, 0);

    let (x, y) = point_at_doc(&mut a, ref_start);
    click_at(&mut a, x, y);

    assert_ne!(a.editor.caret(), def_start, "无 Ctrl 不得跳转");
    assert!(
        (ref_start..=ref_start + 4).contains(&a.editor.caret()),
        "普通单击落在脚注引用标记区: {}",
        a.editor.caret()
    );
    assert!(a.ui_state.chrome.notice.is_empty(), "不得产生跳转提示");
}

#[test]
fn url_whitelist_accepts_only_web_and_mail_schemes() {
    for ok in ["https://example.com", "  HTTP://a.b", "MAILTO:x@y.z"] {
        assert!(url_is_openable(ok), "应放行: {ok:?}");
    }
    for bad in [
        "file:///C:/x",
        "javascript:alert(1)",
        "a.md",
        "#anchor",
        "/abs",
    ] {
        assert!(!url_is_openable(bad), "应拒绝: {bad:?}");
    }
}

#[test]
fn normalize_link_url_adds_http_for_bare_www() {
    assert_eq!(
        normalize_link_url("www.example.com/a"),
        "http://www.example.com/a"
    );
    assert_eq!(
        normalize_link_url("  WWW.Example.com  "),
        "http://WWW.Example.com"
    );
    assert_eq!(normalize_link_url("https://a.b"), "https://a.b");
    assert_eq!(normalize_link_url("mailto:x@y.z"), "mailto:x@y.z");
    // `wwwx.com` 不是 www 前缀，保持原样（随后仍会被白名单拒绝）
    assert_eq!(normalize_link_url("wwwx.com"), "wwwx.com");
    // 裸邮箱补 mailto（GFM 自动链接）
    assert_eq!(normalize_link_url("foo@bar.baz"), "mailto:foo@bar.baz");
    assert_eq!(normalize_link_url("mailto:x@y.z"), "mailto:x@y.z");
    assert_eq!(normalize_link_url("foo@bar"), "foo@bar");
    assert!(url_is_openable(&normalize_link_url("www.a.com")));
}

#[test]
fn link_hover_shows_normalized_url_and_clears_on_leave() {
    let store = MemStore::default();
    let mut a = app("[站点](www.example.com)\n", None, false, &store);
    let (x, y) = point_at_doc(&mut a, 1);
    a.input_state.mouse = PhysicalPosition::new(x as f64, y as f64);
    a.update_hover();
    let (url, _, _) = a.ui_state.hover_link.clone().expect("链接文字上应出提示");
    assert_eq!(url, "http://www.example.com", "浮层显示归一化目标");
    // 移到菜单栏（正文区外）→ 立即清除，无延迟
    a.input_state.mouse = PhysicalPosition::new(x as f64, 2.0);
    a.update_hover();
    assert!(a.ui_state.hover_link.is_none());
}

/// DEF-T05-010：切换文档时链接 tooltip 必须复位，不残留旧 URL。
#[test]
fn switching_document_clears_link_tooltip() {
    let store = MemStore::default();
    store
        .files
        .borrow_mut()
        .insert(PathBuf::from("b.md"), "新文档\n".as_bytes().to_vec());
    let mut a = app("[站点](www.example.com)\n", None, false, &store);
    let (x, y) = point_at_doc(&mut a, 1);
    a.input_state.mouse = PhysicalPosition::new(x as f64, y as f64);
    a.update_hover();
    assert!(a.ui_state.hover_link.is_some(), "前置：悬停应出提示");
    a.open_path(PathBuf::from("b.md"));
    assert!(
        a.ui_state.hover_link.is_none(),
        "换文档后不得残留旧文档链接提示"
    );
}

#[test]
fn link_hover_hits_table_cell_link() {
    let store = MemStore::default();
    let mut a = app(
        "p\n\n| [站点](www.example.com) | b |\n| --- | --- |\n| c | d |\n",
        None,
        false,
        &store,
    );
    a.ensure_layouts_all();
    a.ensure_tables();
    let (id, top) = {
        let g = a
            .index
            .blocks
            .iter()
            .find(|g| g.kind == BlockKind::Table)
            .expect("应有表格块");
        (g.id, g.top)
    };
    let cell = &a.cache.tables[&id].cells[0];
    let vp = a.viewport();
    let x = a.content_x() + cell.x + 2.0;
    let y = vp.y + top - a.window_state.scroll_y + cell.y + cell.h / 2.0;
    a.input_state.mouse = PhysicalPosition::new(x as f64, y as f64);
    a.update_hover();
    assert_eq!(
        a.ui_state.hover_link.as_ref().map(|t| t.0.as_str()),
        Some("http://www.example.com"),
        "表格单元格内链接同样触发"
    );
    // 移到表格外空白（首段）→ 清除
    a.input_state.mouse = PhysicalPosition::new(x as f64, (vp.y + 1.0) as f64);
    a.update_hover();
    assert!(a.ui_state.hover_link.is_none());
}

#[test]
fn link_hover_ignores_code_and_plain_text() {
    let store = MemStore::default();
    // 行内代码里的 URL 不是 Link 节点
    let mut a = app("`https://example.com`\n", None, false, &store);
    let (x, y) = point_at_doc(&mut a, 1);
    a.input_state.mouse = PhysicalPosition::new(x as f64, y as f64);
    a.update_hover();
    assert!(a.ui_state.hover_link.is_none(), "行内代码不应触发");

    // 代码块里的 URL 同样不触发
    let mut b = app("```\nhttps://example.com\n```\n", None, false, &store);
    let (x, y) = point_at_doc(&mut b, 5);
    b.input_state.mouse = PhysicalPosition::new(x as f64, y as f64);
    b.update_hover();
    assert!(b.ui_state.hover_link.is_none(), "代码块不应触发");

    // 普通文本（无链接节点）
    let mut c = app("普通文本\n", None, false, &store);
    let (x, y) = point_at_doc(&mut c, 1);
    c.input_state.mouse = PhysicalPosition::new(x as f64, y as f64);
    c.update_hover();
    assert!(c.ui_state.hover_link.is_none(), "普通文本不应触发");
}

#[test]
fn link_tooltip_truncates_long_url_and_clamps_to_viewport() {
    let vp = chrome::Viewport {
        x: 0.0,
        y: 20.0,
        w: 240.0,
        h: 180.0,
    };
    let long = "https://example.com/very/long/path/that/never/ends";
    let max = crate::paint::link_tip_max_text_w(vp, 1.0);
    let (text, tw) = crate::paint::truncate_link_tip(long, max, 1.0);
    assert!(text.ends_with('…'), "超宽 URL 尾部省略");
    assert!(tw <= max + 0.01, "截断后宽度受视口约束");
    let r = crate::paint::link_tip_rect(tw + 12.0, 24.0, 238.0, 22.0, vp, 1.0);
    assert!(r.x >= vp.x && r.x + r.w <= vp.x + vp.w + 0.01, "水平不越界");
    assert!(r.y >= vp.y && r.y + r.h <= vp.y + vp.h + 0.01, "垂直不越界");
}

#[test]
fn ctrl_click_link_opens_without_touching_document() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let mut a = link_app(LINK_DOC, &calls, Ok(()));
    let (x, y) = link_point(&mut a);
    let caret0 = a.editor.caret();
    let bytes0 = a.editor.session().serialize();
    let undo0 = a.editor.session().undo_depth();

    a.input_state.mods.ctrl = true;
    assert!(!click_at(&mut a, x, y), "打开链接不触发退出");

    assert_eq!(*calls.borrow(), vec!["https://example.com".to_string()]);
    assert_eq!(a.editor.caret(), caret0, "Ctrl+单击不移动光标");
    assert_eq!(a.editor.selection(), None, "Ctrl+单击不改变选区");
    assert_eq!(a.editor.session().serialize(), bytes0, "文档字节不变");
    assert_eq!(a.editor.session().undo_depth(), undo0, "不产生撤销步");
    assert!(
        a.window_state.last_click.is_none(),
        "不进入点击计数/拖选路径"
    );
}

#[test]
fn plain_click_link_moves_caret_and_does_not_open() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let mut a = link_app(LINK_DOC, &calls, Ok(()));
    let (x, y) = link_point(&mut a);
    let doc_len = a.editor.session().text_len();
    a.editor.set_caret(doc_len);

    click_at(&mut a, x, y);

    assert!(calls.borrow().is_empty(), "未按 Ctrl 不打开链接");
    assert_ne!(a.editor.caret(), doc_len, "未按 Ctrl 时回落光标定位");
}

#[test]
fn ctrl_click_rejects_non_whitelisted_scheme() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let mut a = link_app("[x](file:///C:/Windows/notepad.exe)\n", &calls, Ok(()));
    let (x, y) = link_point(&mut a);
    a.input_state.mods.ctrl = true;
    click_at(&mut a, x, y);

    assert!(calls.borrow().is_empty(), "白名单外不得调用平台实现");
    assert!(
        a.ui_state.chrome.notice.contains("无法打开链接"),
        "状态栏须提示：{:?}",
        a.ui_state.chrome.notice
    );

    // 应用内锚点：不交给系统 shell，也不弹提示（锚点跳转属另一任务）
    a.ui_state.chrome.notice.clear();
    a.open_link("#section");
    assert!(calls.borrow().is_empty(), "锚点不进入 opener");
    assert!(a.ui_state.chrome.notice.is_empty(), "锚点静默忽略");
}

#[test]
fn ctrl_click_reports_open_failure_in_status_bar() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let mut a = link_app(LINK_DOC, &calls, Err("模拟打开失败".into()));
    let (x, y) = link_point(&mut a);
    a.input_state.mods.ctrl = true;
    click_at(&mut a, x, y);

    assert_eq!(calls.borrow().len(), 1, "白名单内应已交给平台实现");
    assert!(
        a.ui_state.chrome.notice.contains("模拟打开失败"),
        "失败原因须可见：{:?}",
        a.ui_state.chrome.notice
    );
}

#[test]
fn link_menu_offers_open_first_and_executes_it() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let mut a = link_app(LINK_DOC, &calls, Ok(()));
    let (x, y) = link_point(&mut a);
    a.input_state.mouse = PhysicalPosition::new(x as f64, y as f64);
    a.on_right_click();
    assert!(a.ui_state.ctx.open, "链接上右键应打开菜单");

    let items = a.ui_state.ctx.items().to_vec();
    let open_i = items
        .iter()
        .position(|&(_, act)| act == ctx_menu::CtxAction::OpenLink)
        .expect("应提供「在浏览器中打开」");
    let copy_i = items
        .iter()
        .position(|&(_, act)| act == ctx_menu::CtxAction::CopyLink)
        .expect("应保留「复制链接地址」");
    assert!(open_i < copy_i, "打开项须排在复制项之前");

    // 点击首项：走与 Ctrl+左键相同的打开路径
    let vp = a.viewport();
    let l = ctx_menu::layout(
        a.ui_state.ctx.items(),
        a.ui_state.ctx.x,
        a.ui_state.ctx.y,
        vp.x + vp.w,
        vp.y + vp.h,
        a.input_state.scale,
    );
    let iy = l.y
        + l.rows
            .iter()
            .find(|&&(i, _)| i == open_i)
            .map(|&(_, off)| off)
            .expect("打开项应在可点行中")
        + 1.0;
    click_at(&mut a, l.x + 2.0, iy);
    assert_eq!(*calls.borrow(), vec!["https://example.com".to_string()]);
    assert!(!a.ui_state.ctx.open, "点击后菜单关闭");
}

#[test]
fn opening_search_closes_context_menu_so_keys_are_not_swallowed() {
    let store = MemStore::default();
    let mut a = app("abc\n", None, false, &store);
    a.ui_state
        .ctx
        .open_at(10.0, 20.0, ctx_menu::CtxKind::Block, None, None);
    assert_eq!(a.mode(), Mode::Menu, "右键菜单打开时按键归菜单");
    a.open_search();
    assert!(!a.ui_state.ctx.open, "查找栏打开必须关掉右键菜单");
    assert_eq!(a.mode(), Mode::Search, "否则栏可见但按键被 Menu 模式吞掉");
    a.route_key(Key::Text("a".into()));
    assert_eq!(a.ui_state.search.query, "a");
}

#[test]
fn search_hover_targets_buttons_only() {
    let store = MemStore::default();
    let mut a = app("x\n", None, false, &store);
    a.open_search();
    let l = a.search_rect().unwrap();
    a.input_state.mouse = PhysicalPosition::new(
        (l.next.x + l.next.w / 2.0) as f64,
        (l.next.y + l.next.h / 2.0) as f64,
    );
    a.update_hover();
    assert_eq!(a.ui_state.search.hover, Some(search::Hit::Next));
    a.input_state.mouse = PhysicalPosition::new((l.text.x + 2.0) as f64, (l.text.y + 2.0) as f64);
    a.update_hover();
    assert_eq!(
        a.ui_state.search.hover,
        Some(search::Hit::QueryText),
        "文本区只是焦点目标，不画按钮悬停底色"
    );
    a.close_search();
    a.update_hover();
    assert_eq!(a.ui_state.search.hover, None);
}

#[test]
fn menu_find_opens_empty_search_bar() {
    let store = MemStore::default();
    let (mut a, _) = app_dialogs("x\n", None, false, &store, None, None);
    a.perform_menu(MenuAction::Find);
    assert!(a.ui_state.search.open);
    assert!(a.ui_state.search.query.is_empty());
}

#[test]
fn search_bar_has_one_boundary_and_no_internal_dividers() {
    let store = MemStore::default();
    let mut a = app("alpha beta\n", None, false, &store);
    a.open_replace();
    a.search_input("a");
    let (w, h) = a.win_size();
    let l = a.search_rect().unwrap();
    let rl = search::replace_layout(&l, a.input_state.scale);
    let mut f = Frame::new(w, h, a.input_state.theme.bg);
    a.paint_search(&mut f);
    let border = [
        a.input_state.theme.chrome_border[0],
        a.input_state.theme.chrome_border[1],
        a.input_state.theme.chrome_border[2],
    ];
    // 按钮之间只有间距：分距列不得出现边框色竖线
    let gap_x = (l.prev.x - search::GROUP_GAP / 2.0).round() as u32;
    for y in (l.bar.y + 2.0) as u32..(l.bar.y + l.bar.h - 2.0) as u32 {
        let px = f.get(gap_x, y);
        assert_ne!([px[0], px[1], px[2]], border, "按钮间不得有竖分隔线");
    }
    // 两行之间只有间距：空档整行不得出现边框色横线
    let gap_y = (rl.row.y - search::ROW_GAP / 2.0).round() as u32;
    for x in (l.bar.x + 2.0) as u32..(l.bar.x + l.bar.w - 2.0) as u32 {
        let px = f.get(x, gap_y);
        assert_ne!(
            [px[0], px[1], px[2]],
            border,
            "查询行与替换行之间不得有横线"
        );
    }
}

#[test]
fn search_bar_focus_uses_quiet_field_tint() {
    let store = MemStore::default();
    let mut a = app("x\n", None, false, &store);
    a.open_search();
    let (w, h) = a.win_size();
    let l = a.search_rect().unwrap();
    let mut f = Frame::new(w, h, a.input_state.theme.bg);
    a.paint_search(&mut f);
    // 输入区右端为空档（无计数、无字形）：应只剩焦点底色
    let x = (l.text.x + l.text.w - 6.0) as u32;
    let y = (l.text.y + l.text.h / 2.0) as u32;
    let mut t = Frame::new(1, 1, a.input_state.theme.popup_bg);
    t.fill_rect(
        0,
        0,
        1,
        1,
        crate::tokens::state::focus_bg(&a.input_state.theme),
    );
    assert_eq!(f.get(x, y), t.get(0, 0), "焦点用输入区安静底色");
}

#[test]
fn search_bar_no_match_uses_danger_and_disabled_colors() {
    let store = MemStore::default();
    let mut a = app("alpha\n", None, false, &store);
    a.open_search();
    a.search_input("zzz");
    assert!(a.ui_state.search.no_matches());
    let (w, h) = a.win_size();
    let l = a.search_rect().unwrap();
    let mut f = Frame::new(w, h, a.input_state.theme.bg);
    a.paint_search(&mut f);
    let theme = a.input_state.theme;
    let wanted = crate::tokens::state::empty_fg(&theme);
    let mut found = false;
    for x in (l.count_right - search::COUNT_W) as u32..l.count_right as u32 {
        for y in l.text.y as u32..(l.text.y + l.text.h) as u32 {
            let px = f.get(x, y);
            if [px[0], px[1], px[2]] == wanted {
                found = true;
            }
        }
    }
    assert!(found, "无命中计数使用 danger 色");
    // 翻页按钮字形应为禁用前景（置灰），不出现正文色
    let text = theme.text;
    let bg = theme.popup_bg;
    let (mut has_ink, mut has_text) = (false, false);
    for x in l.prev.x as u32..(l.prev.x + l.prev.w) as u32 {
        for y in l.prev.y as u32..(l.prev.y + l.prev.h) as u32 {
            let px = f.get(x, y);
            let rgb = [px[0], px[1], px[2]];
            if rgb != [bg[0], bg[1], bg[2]] {
                has_ink = true;
            }
            has_text |= rgb == text;
        }
    }
    assert!(has_ink, "无命中时翻页按钮仍绘制（置灰）");
    assert!(!has_text, "禁用按钮不得使用正文色");
}

#[test]
fn search_match_ticks_mark_scrollbar_positions() {
    let store = MemStore::default();
    let mut a = app("alpha\nalpha\nalpha\n", None, false, &store);
    a.open_search();
    a.search_input("alpha");
    assert_eq!(a.ui_state.search.matches.len(), 3);
    let (w, h) = a.win_size();
    let vp = a.viewport();
    let mut f = Frame::new(w, h, a.input_state.theme.bg);
    a.paint_search_ticks(&mut f, vp);
    let scale = a.input_state.scale;
    let lane = (chrome::SCROLL_W * scale).ceil() as u32;
    let x = (w as f32 - lane as f32 + 3.0 * scale).round() as u32;
    let tick_h = (2.0 * scale).round().max(1.0) as u32;
    let total = a.editor.session().text_len().max(1);
    let m = a.ui_state.search.matches[0];
    let y = (vp.y + (m.start as f32 / total as f32) * vp.h - tick_h as f32 / 2.0).round() as u32;
    let accent = a.input_state.theme.accent;
    assert_eq!(
        f.get(x + 1, y),
        [accent[0], accent[1], accent[2], 255],
        "当前命中在滚动条上打 accent 刻度"
    );
    // 关闭后不再打刻度
    a.close_search();
    let mut g = Frame::new(w, h, a.input_state.theme.bg);
    a.paint_search_ticks(&mut g, vp);
    assert_eq!(g.get(x + 1, y), a.input_state.theme.bg);
}

#[test]
fn search_bar_paints_overlay_and_clears_on_close() {
    let store = MemStore::default();
    let mut a = app("x\n", None, false, &store);
    a.open_search();
    let (w, h) = a.win_size();
    let l = a.search_rect().unwrap();
    let mut f = Frame::new(w, h, a.input_state.theme.bg);
    a.paint_search(&mut f);
    assert_eq!(
        f.get((l.bar.x + 3.0) as u32, (l.bar.y + 3.0) as u32),
        a.input_state.theme.popup_bg,
        "浮栏不透明底色（与全部浮层同色）"
    );
    a.close_search();
    let mut g = Frame::new(w, h, a.input_state.theme.bg);
    a.paint_search(&mut g);
    assert_eq!(
        g.get((l.bar.x + 3.0) as u32, (l.bar.y + 3.0) as u32),
        a.input_state.theme.bg,
        "关闭后不再绘制"
    );
}

#[test]
fn search_highlights_use_selection_and_hover_colors() {
    let store = MemStore::default();
    let mut a = app("alpha beta alpha\n", None, false, &store);
    a.ensure_layouts_all();
    a.open_search();
    a.search_input("alpha");
    let (id, range) = (a.index.blocks[0].id, a.index.blocks[0].range);
    // 像素取样：命中矩形中心（布局缓存跨关闭仍有效）
    let pixel = |f: &Frame, a: &App, m: &Range| {
        let (rx, ry, rw, rh) = a
            .engine
            .selection_rects(&a.cache.layouts[&id], m.start, m.end)[0];
        f.get(
            (MARGIN + rx + rw / 2.0) as u32,
            (10.0 + ry + rh / 2.0) as u32,
        )
    };
    let mut f = Frame::new(420, 120, a.input_state.theme.bg);
    a.paint_match_highlights(&mut f, &a.cache.layouts[&id], &range, 10);
    // 半透明色经 src-over 混合：以同底色单像素填充作为期望值
    let blend = |c: [u8; 4]| {
        let mut t = Frame::new(1, 1, a.input_state.theme.bg);
        t.fill_rect(0, 0, 1, 1, c);
        t.get(0, 0)
    };
    assert_eq!(
        pixel(&f, &a, &Range::new(0, 5)),
        blend(a.input_state.theme.sel),
        "当前命中用选区色"
    );
    assert_eq!(
        pixel(&f, &a, &Range::new(11, 16)),
        blend(a.input_state.theme.hover),
        "其余命中用 hover 色"
    );
    // 关闭后不再绘制命中高亮
    a.close_search();
    let mut g = Frame::new(420, 120, a.input_state.theme.bg);
    a.paint_match_highlights(&mut g, &a.cache.layouts[&id], &range, 10);
    assert_eq!(pixel(&g, &a, &Range::new(0, 5)), a.input_state.theme.bg);
}

/// max 峰值定位探针（默认忽略）：拆分 apply / after_edit / paint 耗时，
/// 并采样最大帧的各环节占比，定位 66ms 峰值来源。
/// cargo test -p veil-ui --release -- --ignored --nocapture perf_peak_breakdown
#[test]
#[ignore = "性能探针，不参与常规门禁"]
fn perf_peak_breakdown() {
    let mut text = String::with_capacity(500_000);
    for i in 0..1500 {
        text.push_str(&format!(
            "## Section {i}\n\nParagraph {i} with **bold** text, `code`, and a [link](https://example.com/{i}).\n\n- item A {i}\n- item B {i}\n\n```rust\nfn f{i}() -> u32 {{ {i} }}\n```\n\n> quote {i}\n\n---\n\n"
        ));
    }
    let store = MemStore::default();
    let mut a = app(&text, None, false, &store);
    a.editor.apply(Intent::MoveDocStart).unwrap();
    for _ in 0..5 {
        a.editor.apply(Intent::MoveRight).unwrap();
    }
    a.paint();

    let mut total = Vec::with_capacity(500);
    let mut apply_ms = Vec::with_capacity(500);
    let mut after_ms = Vec::with_capacity(500);
    let mut paint_ms = Vec::with_capacity(500);
    for _ in 0..500 {
        let t0 = Instant::now();
        let dirty = a.editor.apply(Intent::InsertText("a".into())).unwrap();
        let t1 = Instant::now();
        a.after_edit(dirty);
        let t2 = Instant::now();
        a.paint();
        let t3 = Instant::now();
        apply_ms.push((t1 - t0).as_secs_f64() * 1e3);
        after_ms.push((t2 - t1).as_secs_f64() * 1e3);
        paint_ms.push((t3 - t2).as_secs_f64() * 1e3);
        total.push((t3 - t0).as_secs_f64() * 1e3);
    }
    // 找最大帧，输出其各环节耗时
    let (max_idx, _) = total
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
        .unwrap();
    // 统计 paint > 5ms 的次数
    let paint_spikes = paint_ms.iter().filter(|&&x| x > 5.0).count();
    let after_spikes = after_ms.iter().filter(|&&x| x > 5.0).count();
    println!("peak_total_ms={:.3}", total[max_idx]);
    println!("peak_apply_ms={:.3}", apply_ms[max_idx]);
    println!("peak_after_edit_ms={:.3}", after_ms[max_idx]);
    println!("peak_paint_ms={:.3}", paint_ms[max_idx]);
    println!("paint_spikes_gt5ms={paint_spikes}/500");
    println!("after_edit_spikes_gt5ms={after_spikes}/500");
}

/// after_edit 内部子步骤拆分探针（默认忽略）：定位 69ms 峰值具体来自哪个子步骤。
/// cargo test -p veil-ui --release -- --ignored --nocapture perf_after_edit_breakdown
#[test]
#[ignore = "性能探针，不参与常规门禁"]
fn perf_after_edit_breakdown() {
    let mut text = String::with_capacity(500_000);
    for i in 0..1500 {
        text.push_str(&format!(
            "## Section {i}\n\nParagraph {i} with **bold** text, `code`, and a [link](https://example.com/{i}).\n\n- item A {i}\n- item B {i}\n\n```rust\nfn f{i}() -> u32 {{ {i} }}\n```\n\n> quote {i}\n\n---\n\n"
        ));
    }
    let store = MemStore::default();
    let mut a = app(&text, None, false, &store);
    a.editor.apply(Intent::MoveDocStart).unwrap();
    for _ in 0..5 {
        a.editor.apply(Intent::MoveRight).unwrap();
    }
    a.paint();

    // 真实 render() 路径预热：首帧后派生缓存已填，后续循环按命中率路径测
    a.refresh_index();
    a.ensure_layouts_visible();
    a.recompute_geometry();
    println!("after_warmup_cache_layouts={}", a.cache.layouts.len());

    let n = 500;
    // 记录首次编辑前后可见块 ID，验证 ID 稳定性
    let before_ids: Vec<BlockId> = a
        .index
        .blocks
        .get(a.visible_index_range())
        .unwrap_or(&[])
        .iter()
        .map(|g| g.id)
        .collect();
    let mut refresh = Vec::with_capacity(n);
    let mut ensure_vis = Vec::with_capacity(n);
    let mut ensure_block = Vec::with_capacity(n);
    let mut ensure_tbl = Vec::with_capacity(n);
    let mut recompute = Vec::with_capacity(n);
    let mut scroll = Vec::with_capacity(n);
    let mut total = Vec::with_capacity(n);
    let mut dirty_count = Vec::with_capacity(n);
    let mut vis_range_len = Vec::with_capacity(n);
    let mut missing_layouts = Vec::with_capacity(n);
    let mut after_first_ids: Option<Vec<BlockId>> = None;
    for iter in 0..n {
        let dirty = a.editor.apply(Intent::InsertText("a".into())).unwrap();
        dirty_count.push(dirty.len());
        let t0 = Instant::now();
        let t1 = Instant::now();
        a.refresh_index();
        let t2 = Instant::now();
        let vr = a.visible_index_range();
        vis_range_len.push(vr.len());
        // 统计可见范围内缺少布局缓存的块数
        let base = a.base_key();
        let caret = a.editor.caret();
        let missing = a
            .index
            .blocks
            .get(vr.clone())
            .unwrap_or(&[])
            .iter()
            .filter(|g| {
                a.cache
                    .layout(g.id, &g.kind, g.range, base, caret, a.toc_layout_epoch(g))
                    .is_none()
            })
            .count();
        missing_layouts.push(missing);
        if iter < 5 {
            println!("iter{iter}_cache_size={}", a.cache.layouts.len());
        }
        a.ensure_layouts_visible();
        let t3 = Instant::now();
        let caret_id = a.caret_block_index().map(|i| a.index.blocks[i].id);
        if let Some(id) = caret_id {
            a.ensure_block(id);
        }
        let t4 = Instant::now();
        a.ensure_tables_visible();
        let t5 = Instant::now();
        a.recompute_geometry();
        let t6 = Instant::now();
        a.scroll_to_caret();
        let t7 = Instant::now();
        refresh.push((t2 - t1).as_secs_f64() * 1e3);
        ensure_vis.push((t3 - t2).as_secs_f64() * 1e3);
        ensure_block.push((t4 - t3).as_secs_f64() * 1e3);
        ensure_tbl.push((t5 - t4).as_secs_f64() * 1e3);
        recompute.push((t6 - t5).as_secs_f64() * 1e3);
        scroll.push((t7 - t6).as_secs_f64() * 1e3);
        total.push((t7 - t0).as_secs_f64() * 1e3);
        if iter == 0 {
            after_first_ids = Some(
                a.index
                    .blocks
                    .get(a.visible_index_range())
                    .unwrap_or(&[])
                    .iter()
                    .map(|g| g.id)
                    .collect(),
            );
        }
        // 模拟生产 paint 的视口稳定：两遍 ensure_layouts_visible + recompute
        a.ensure_layouts_visible();
        a.ensure_math_visible();
        a.ensure_tables_visible();
        a.recompute_geometry();
        if visible_layout_missing(&a) {
            a.ensure_layouts_visible();
            a.ensure_math_visible();
            a.ensure_tables_visible();
            a.recompute_geometry();
        }
    }
    let (max_idx, _) = total
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
        .unwrap();
    println!("peak_total_ms={:.3}", total[max_idx]);
    println!("peak_refresh_index_ms={:.3}", refresh[max_idx]);
    println!("peak_ensure_layouts_visible_ms={:.3}", ensure_vis[max_idx]);
    println!("peak_ensure_block_ms={:.3}", ensure_block[max_idx]);
    println!("peak_ensure_tables_visible_ms={:.3}", ensure_tbl[max_idx]);
    println!("peak_recompute_geometry_ms={:.3}", recompute[max_idx]);
    println!("peak_scroll_to_caret_ms={:.3}", scroll[max_idx]);
    println!("peak_dirty_count={}", dirty_count[max_idx]);
    println!("peak_visible_range_len={}", vis_range_len[max_idx]);
    let max_dirty = dirty_count.iter().max().unwrap();
    println!("max_dirty_count={max_dirty}");
    println!("peak_missing_layouts={}", missing_layouts[max_idx]);
    let max_missing = missing_layouts.iter().max().unwrap();
    println!("max_missing_layouts={max_missing}");
    let multi_missing = missing_layouts.iter().filter(|&&m| m > 1).count();
    println!("iters_with_multi_missing={multi_missing}/{n}");
    // 打印前 20 次的 missing 数，看规律
    for (i, m) in missing_layouts.iter().take(20).enumerate() {
        println!("iter{i}_missing={m}");
    }
    // 首次编辑后可见块 ID 与编辑前对比
    if let Some(after_ids) = after_first_ids {
        let changed = before_ids
            .iter()
            .zip(after_ids.iter())
            .filter(|(b, a)| b != a)
            .count();
        println!(
            "before_ids={:?}",
            before_ids.iter().map(|id| id.0).collect::<Vec<_>>()
        );
        println!(
            "after_ids={:?}",
            after_ids.iter().map(|id| id.0).collect::<Vec<_>>()
        );
        println!("ids_changed_in_visible_range={changed}");
    }
    // p95 of each
    let p95 = |v: &[f64]| {
        let mut s = v.to_vec();
        s.sort_by(|x, y| x.partial_cmp(y).unwrap());
        let i = ((s.len() as f64 * 0.95).ceil() as usize)
            .saturating_sub(1)
            .min(s.len() - 1);
        s[i]
    };
    println!("p95_refresh_index_ms={:.3}", p95(&refresh));
    println!("p95_ensure_layouts_visible_ms={:.3}", p95(&ensure_vis));
    println!("p95_recompute_geometry_ms={:.3}", p95(&recompute));
}

// ---- 阶段 1：中文口径字数 + 专注模式 ----

#[test]
fn word_count_tracks_edits_and_undo() {
    let store = MemStore::default();
    let mut a = app("你好世界", None, false, &store);
    assert_eq!(a.ui_state.writing.words, 4, "启动即按中文口径统计");
    a.perform_action(Action::Doc(Intent::InsertText(" hello".into())));
    assert_eq!(a.ui_state.writing.words, 5, "编辑后与文本同源重算");
    a.perform_menu(MenuAction::Undo);
    assert_eq!(a.ui_state.writing.words, 4, "撤销后回到旧值");
}

#[test]
fn line_move_refreshes_layout_without_further_edit() {
    let store = MemStore::default();
    let mut a = app("1one\n2two\n3three\n", None, false, &store);
    a.ensure_layouts_all();
    let text_of = |a: &crate::app::App| -> String {
        a.index
            .blocks
            .iter()
            .map(|g| a.cache.layouts[&g.id].layout_text())
            .collect()
    };
    let before = text_of(&a);
    assert!(before.find("1one") < before.find("2two"), "{before:?}");
    // 行移动保持块 id 与行区间不变，布局键必须仍能失配（回归：画面滞后到下次编辑）。
    a.perform_action(Action::Doc(Intent::MoveLineDown));
    let after = text_of(&a);
    assert!(
        after.find("2two") < after.find("1one"),
        "行移动后布局应立即反映新顺序，实际 {after:?}"
    );
}

#[test]
fn word_count_skips_markdown_markers() {
    let store = MemStore::default();
    let a = app("# 标题\n\n正文 **加粗** `code`\n", None, false, &store);
    // 标题(2) + 正文(2) + 加粗(2) + code(1)
    assert_eq!(a.ui_state.writing.words, 7);
}

#[test]
fn focus_mode_dims_only_non_caret_blocks() {
    let store = MemStore::default();
    let mut a = app("第一段\n\n第二段\n", None, false, &store);
    a.render(); // 先完成排版，几何才稳定
    let vp = a.viewport();
    let bg = a.input_state.theme.bg;
    let scroll = a.window_state.scroll_y;
    let (top0, h0) = {
        let g = &a.index.blocks[0];
        (g.top, g.height)
    };
    let (top1, h1) = {
        let g = &a.index.blocks[1];
        (g.top, g.height)
    };
    let (y0, y1) = (vp.y + top0 - scroll, vp.y + top1 - scroll);
    let frame = a.render();
    let p0 = first_non_bg(frame, y0 as i32, (y0 + h0) as i32, bg).expect("首段有内容");
    let p1 = first_non_bg(frame, y1 as i32, (y1 + h1) as i32, bg).expect("次段有内容");

    a.toggle_focus();
    assert!(a.ui_state.writing.focus);
    let frame = a.render();
    assert_eq!(frame.get(p0.0, p0.1), p0.2, "光标所在块不压暗");
    let dimmed = frame.get(p1.0, p1.1);
    assert_ne!(dimmed, p1.2, "非光标块被压暗");
    let lum = |p: [u8; 4]| p[0] as i32 + p[1] as i32 + p[2] as i32;
    assert!(lum(dimmed) > lum(p1.2), "压暗即向背景色靠拢");
    // 纯视觉开关：源码一个字节都不变
    assert_eq!(doc_text(&a), "第一段\n\n第二段\n");
}

// ---- 2e 端到端场景：打开→编辑→表格→图片→查找→保存→重开→恢复（红线 e2e 断言 ≥20）----
