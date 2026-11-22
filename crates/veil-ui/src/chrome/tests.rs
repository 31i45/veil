//! chrome 模型/绘制/缓存测试（原 chrome.rs 内联测试平移，labels 访问适配封装）。

use super::*;
use crate::document::ExternalState;
use crate::i18n::t;
use crate::theme::Theme;
use crate::tokens::palette;
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
fn action_enabled_only_gates_known_noop_items() {
    let none = MenuFlags::default();
    assert!(
        !action_enabled(MenuAction::Separator, none),
        "分隔线不可执行"
    );
    assert!(
        !action_enabled(MenuAction::Save, none),
        "无改动无草稿：保存禁用"
    );
    assert!(
        !action_enabled(MenuAction::Undo, none),
        "无撤销步：撤销禁用"
    );
    assert!(
        !action_enabled(MenuAction::Redo, none),
        "无重做步：重做禁用"
    );
    // 空文档没有可导出内容：导出/打印禁用（产品语义变化，见 Task 6.4）。
    assert!(
        !action_enabled(MenuAction::Export, none),
        "空文档禁用导出 HTML"
    );
    assert!(
        !action_enabled(MenuAction::ExportPdf, none),
        "空文档禁用导出 PDF"
    );
    assert!(!action_enabled(MenuAction::Print, none), "空文档禁用打印");
    // 其余项恒可用（含剪贴板等不可静态判断的项）
    assert!(action_enabled(MenuAction::Copy, none));
    assert!(action_enabled(MenuAction::Open, none));

    let all = MenuFlags {
        can_save: true,
        can_undo: true,
        can_redo: true,
        ..none
    };
    assert!(action_enabled(MenuAction::Save, all));
    assert!(action_enabled(MenuAction::Undo, all));
    assert!(action_enabled(MenuAction::Redo, all));
    // 正文非空即可导出/打印，与是否有未保存修改无关。
    let with_text = MenuFlags {
        can_export: true,
        ..none
    };
    assert!(action_enabled(MenuAction::Export, with_text));
    assert!(action_enabled(MenuAction::ExportPdf, with_text));
    assert!(action_enabled(MenuAction::Print, with_text));
}

#[test]
fn menu_bar_hit_and_toggle_zones() {
    let rects = menu_bar_rects(1.0);
    assert_eq!(rects.len(), MENUS.len());
    assert_eq!(rects.len(), 4, "文件/编辑/视图/帮助");
    let (_, x0, w0) = rects[0];
    let (_, x1, _) = rects[1];
    let (_, x2, _) = rects[2];
    let (_, x3, _) = rects[3];
    assert_eq!(hit_menu_bar(x0 + 1.0, 10.0, 1.0), Some(0));
    assert_eq!(hit_menu_bar(x1 + 1.0, 10.0, 1.0), Some(1));
    assert_eq!(hit_menu_bar(x2 + 1.0, 10.0, 1.0), Some(2));
    assert_eq!(hit_menu_bar(x3 + 1.0, 10.0, 1.0), Some(3));
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
fn help_menu_items_hit_map_to_help_actions() {
    let (_, x, w) = menu_bar_rects(1.0)[3];
    assert_eq!(hit_menu_bar(x + w / 2.0, 10.0, 1.0), Some(3));
    assert_eq!(t(MENUS[3].title), "帮助");
    let p = popup_layout(3, 1.0, &[], 1000.0);
    for (item, expected) in [
        (0, MenuAction::About),
        (1, MenuAction::ExportDiagnostics),
        (2, MenuAction::CheckUpdate),
    ] {
        let (idx, ix, iy, iw, ih) = p.items[item];
        assert_eq!(
            hit_popup_item(ix + iw / 2.0, iy + ih / 2.0, 3, 1.0, &[], 1000.0),
            Some(idx)
        );
        assert_eq!(action_at(3, idx, &[]), Some(expected));
        assert!(
            action_enabled(expected, MenuFlags::default()),
            "{expected:?} 应始终可用"
        );
        assert!(!action_checked(expected, MenuFlags::default()));
        assert_eq!(disabled_reason(expected, MenuFlags::default()), None);
    }
}

#[test]
fn popup_hit_maps_items_and_contains_padding() {
    let p = popup_layout(0, 1.0, &[], 1000.0);
    assert!(p.h > 0.0);
    assert_eq!(p.y, MENU_H);
    for (i, ix, iy, iw, ih) in &p.items {
        assert_eq!(
            hit_popup_item(ix + iw / 2.0, iy + ih / 2.0, 0, 1.0, &[], 1000.0),
            Some(*i)
        );
    }
    assert_eq!(
        hit_popup_item(p.x + 1.0, p.y + 1.0, 0, 1.0, &[], 1000.0),
        None,
        "内边距非项区"
    );
    assert!(popup_contains(p.x + 1.0, p.y + 1.0, 0, 1.0, &[], 1000.0));
    assert!(!popup_contains(p.x - 1.0, p.y + 1.0, 0, 1.0, &[], 1000.0));
    assert_eq!(action_at(0, 0, &[]), Some(MenuAction::New));
    assert_eq!(action_at(0, 1, &[]), Some(MenuAction::NewWindow));
    assert_eq!(action_at(0, 2, &[]), Some(MenuAction::Open));
    assert_eq!(action_at(0, 99, &[]), None);
}

/// 0b 动态菜单：最近文件插在「退出」之前，布局/命中/分发三者一致。
#[test]
fn file_menu_injects_recent_before_quit() {
    let recent = vec!["a.md".to_string(), "b.md".to_string()];
    let static_len = MENUS[0].items.len();
    let rows = menu_rows(0, &recent);
    // 静态项 + 两个最近文件 + 新增的一条分隔线（最近文件块与「退出」之间）
    assert_eq!(rows.len(), static_len + 3, "静态项 + 两个最近文件 + 分隔线");
    let first = rows
        .iter()
        .position(|(_, _, a)| *a == MenuAction::OpenRecent(0))
        .expect("应含第一个最近文件");
    assert_eq!(
        rows[first - 1].2,
        MenuAction::Separator,
        "最近文件块上方有分隔线"
    );
    assert_eq!(
        rows[first + 2].2,
        MenuAction::Separator,
        "最近文件块下方有分隔线"
    );
    assert_eq!(rows[first + 3].2, MenuAction::Quit, "退出仍在最后");
    // 分发与布局同源
    assert_eq!(
        action_at(0, first, &recent),
        Some(MenuAction::OpenRecent(0))
    );
    assert_eq!(
        action_at(0, first + 1, &recent),
        Some(MenuAction::OpenRecent(1))
    );
    assert_eq!(action_at(0, rows.len(), &recent), None);
    // 高度随项数增长：两项行高 + 一个分组间距（无分隔线）
    let p0 = popup_layout(0, 1.0, &[], 1000.0);
    let p1 = popup_layout(0, 1.0, &recent, 1000.0);
    assert_eq!(
        p1.h - p0.h,
        2.0 * crate::theme::surface::ROW_HEIGHT + crate::tokens::space::S
    );
    for (i, ix, iy, iw, ih) in &p1.items {
        assert_eq!(
            hit_popup_item(ix + iw / 2.0, iy + ih / 2.0, 0, 1.0, &recent, 1000.0),
            Some(*i)
        );
    }
    // 其它菜单不受最近文件影响
    assert_eq!(menu_rows(1, &recent).len(), MENUS[1].items.len());
    // 空最近列表：零动态项
    assert_eq!(menu_rows(0, &[]).len(), static_len);
}

#[test]
fn popup_width_capped_for_long_recent_label() {
    // 模拟超长最近文件标签：菜单宽度不应超过 420px（1.0×）
    let long = "运维量化核心：标签体系与记录模板终极配置.md · Desktop".to_string();
    let p = popup_layout(0, 1.0, &[long], 1000.0);
    assert!(p.w <= 420.0, "宽菜单应被钳制到 420px，实际 {:.1}", p.w);
    // 命中仍落在完整行高内：取最近文件第一项的 rect
    let recent_first = p
        .items
        .iter()
        .find(|(i, _, _, _, _)| {
            matches!(
                action_at(0, *i, &["x.md".to_string()]),
                Some(MenuAction::OpenRecent(0))
            )
        })
        .copied();
    assert!(recent_first.is_some(), "应能命中最近文件项");
    let (_, _, _, _, ih) = recent_first.unwrap();
    assert!(ih >= 28.0 * 1.0, "命中行高不应被截断影响");
}

#[test]
fn popup_label_truncated_when_too_long() {
    let long = "运维量化核心：标签体系与记录模板终极配置.md · Desktop".to_string();
    let recent = vec![long.clone()];
    let mut engine = LayoutEngine::new();
    let mut frame = Frame::new(800, 400, [255, 255, 255, 255]);
    let theme = Theme::light();
    let mut chrome = Chrome {
        open: Some(0),
        ..Default::default()
    };
    chrome.paint(
        &mut frame,
        &mut engine,
        viewport(800, 400, 1.0),
        1.0,
        &theme,
        None,
        false,
        ExternalState::None,
        None,
        0,
        1,
        1,
        0.0,
        0.0,
        &recent,
        MenuFlags::default(),
    );
    // 像素断言：菜单宽度被钳制，右缘不超出窗口
    let p = popup_layout(0, 1.0, &recent, 800.0);
    assert!(p.w <= 420.0, "菜单宽度应 ≤ 420px，实际 {:.1}", p.w);
    assert!(p.x + p.w <= 800.0, "菜单右缘不应超出窗口");
    // 菜单外（x=430）应为背景色，不会有标签像素溢出
    let text4 = [theme.text[0], theme.text[1], theme.text[2], 255];
    let outside = frame.get(430, (p.y + p.h / 2.0) as u32);
    assert_ne!(outside, text4, "长标签不应溢出到菜单外");
}

#[test]
fn menu_definitions_cover_shortcuts() {
    let file: Vec<_> = MENUS[0].items.iter().map(|(_, _, a)| *a).collect();
    assert_eq!(
        file,
        vec![
            MenuAction::New,
            MenuAction::NewWindow,
            MenuAction::Open,
            MenuAction::QuickOpen,
            MenuAction::Separator,
            MenuAction::Save,
            MenuAction::SaveAs,
            MenuAction::Separator,
            MenuAction::Export,
            MenuAction::ExportPdf,
            MenuAction::Print,
            MenuAction::VersionHistory,
            MenuAction::Separator,
            MenuAction::Quit
        ]
    );
    let edit: Vec<_> = MENUS[1].items.iter().map(|(_, _, a)| *a).collect();
    assert_eq!(
        edit,
        vec![
            MenuAction::Undo,
            MenuAction::Redo,
            MenuAction::Separator,
            MenuAction::Cut,
            MenuAction::Copy,
            MenuAction::Paste,
            MenuAction::SelectAll,
            MenuAction::Separator,
            MenuAction::ToggleBold,
            MenuAction::ToggleItalic,
            MenuAction::InsertLink,
            MenuAction::Separator,
            MenuAction::Find,
            MenuAction::Replace,
            MenuAction::Separator,
            MenuAction::InsertImage
        ]
    );
    let view: Vec<_> = MENUS[2].items.iter().map(|(_, _, a)| *a).collect();
    assert_eq!(
        view,
        vec![
            MenuAction::ToggleOutline,
            MenuAction::Separator,
            MenuAction::ZoomIn,
            MenuAction::ZoomOut,
            MenuAction::ZoomReset,
            MenuAction::Separator,
            MenuAction::ToggleFullscreen,
            MenuAction::Separator,
            MenuAction::ThemeSystem,
            MenuAction::ThemeLight,
            MenuAction::ThemeDark,
            MenuAction::Separator,
            MenuAction::ToggleSourceMode,
            MenuAction::ToggleFocus,
            MenuAction::Separator,
            MenuAction::ToggleRemoteImages,
            MenuAction::Separator,
            MenuAction::Settings,
        ]
    );
    let help: Vec<_> = MENUS[3].items.iter().map(|(_, _, a)| *a).collect();
    assert_eq!(
        help,
        vec![
            MenuAction::About,
            MenuAction::ExportDiagnostics,
            MenuAction::CheckUpdate,
        ]
    );
}

fn expected_input_action(action: MenuAction) -> crate::input::Action {
    use crate::editor::Intent;
    use crate::input::Action;
    match action {
        MenuAction::New => Action::New,
        MenuAction::NewWindow => Action::NewWindow,
        MenuAction::Open => Action::Open,
        MenuAction::QuickOpen => Action::QuickOpen,
        MenuAction::Save => Action::Save,
        MenuAction::SaveAs => Action::SaveAs,
        MenuAction::Export => Action::Export,
        MenuAction::ExportPdf => Action::ExportPdf,
        MenuAction::Print => Action::Print,
        MenuAction::VersionHistory => Action::VersionHistory,
        MenuAction::Undo => Action::Doc(Intent::Undo),
        MenuAction::Redo => Action::Doc(Intent::Redo),
        MenuAction::Cut => Action::Cut,
        MenuAction::Copy => Action::Copy,
        MenuAction::Paste => Action::Paste,
        MenuAction::SelectAll => Action::Doc(Intent::SelectAll),
        MenuAction::ToggleOutline => Action::OpenOutline,
        MenuAction::Find => Action::Find,
        MenuAction::Replace => Action::Replace,
        MenuAction::InsertImage => Action::InsertImage,
        MenuAction::ToggleBold => Action::Doc(Intent::ToggleWrap("**")),
        MenuAction::ToggleItalic => Action::Doc(Intent::ToggleWrap("*")),
        MenuAction::InsertLink => Action::Doc(Intent::InsertLink),
        MenuAction::ToggleSourceMode => Action::OpenSourceMode,
        MenuAction::ZoomIn => Action::ZoomIn,
        MenuAction::ZoomOut => Action::ZoomOut,
        MenuAction::ZoomReset => Action::ZoomReset,
        MenuAction::ToggleFullscreen => Action::ToggleFullscreen,
        MenuAction::Settings => Action::OpenSettings,
        other => panic!("{other:?} 不应有需要键盘映射的菜单快捷键"),
    }
}

fn dispatch_shortcut(shortcut: &str) -> crate::input::Action {
    use crate::input::Mods;
    use winit::keyboard::{Key as WKey, KeyCode, NamedKey, PhysicalKey};

    let mut mods = Mods::default();
    let mut parts = shortcut.split('+').peekable();
    while let Some(part) = parts.peek().copied() {
        match part {
            "Ctrl" => mods.ctrl = true,
            "Shift" => mods.shift = true,
            "Alt" => mods.alt = true,
            _ => break,
        }
        parts.next();
    }
    let name = parts.next().expect("快捷键含主键");
    assert!(parts.next().is_none(), "快捷键格式无效: {shortcut}");
    let logical = if name == "F11" {
        WKey::Named(NamedKey::F11)
    } else {
        WKey::Character(name.into())
    };
    let key = if mods.ctrl && !mods.alt {
        crate::interaction::ctrl_punctuation(&logical, PhysicalKey::Code(KeyCode::KeyA))
            .or_else(|| crate::interaction::map_key_parts(None, logical, mods))
    } else {
        crate::interaction::map_key_parts(None, logical, mods)
    }
    .unwrap_or_else(|| panic!("winit 映射未识别菜单快捷键: {shortcut}"));
    match crate::mode::route(crate::mode::Mode::Document, key, mods, false) {
        crate::mode::Cmd::Edit(action) => action,
        other => panic!("菜单快捷键未分发为真实动作: {shortcut} → {other:?}"),
    }
}

#[test]
fn menu_shortcuts_match_winit_mapping_and_real_dispatch() {
    let mut with_shortcuts = 0;
    for menu in MENUS {
        for (label, shortcut, action) in menu.items {
            if *action == MenuAction::Separator {
                assert_eq!((*label, *shortcut), (Key::Empty, Key::Empty));
                continue;
            }
            assert!(!t(*label).is_empty(), "菜单标签不得为空: {action:?}");
            if *shortcut == Key::Empty {
                assert!(matches!(
                    action,
                    MenuAction::Quit
                        | MenuAction::ThemeSystem
                        | MenuAction::ThemeLight
                        | MenuAction::ThemeDark
                        | MenuAction::ToggleFocus
                        | MenuAction::ToggleRemoteImages
                        | MenuAction::About
                        | MenuAction::ExportDiagnostics
                        | MenuAction::CheckUpdate
                ));
                continue;
            }
            with_shortcuts += 1;
            assert_eq!(
                dispatch_shortcut(t(*shortcut)),
                expected_input_action(*action),
                "菜单快捷键文案与真实输入分发不一致: {} / {action:?}",
                t(*shortcut)
            );
        }
    }
    assert_eq!(with_shortcuts, 29, "全部 29 个显示快捷键都必须经过真实分发");
}

/// 命令矩阵（Task 3.3）：动作 → 快捷键 的唯一对照表。
/// 断言：主菜单每项都在这张表里、同动作只有一个主入口、提示键与真实分发一致。
#[test]
fn menu_command_matrix_maps_each_action_to_its_real_shortcut() {
    let matrix: &[(MenuAction, Key)] = &[
        (MenuAction::New, Key::ShortcutNew),
        (MenuAction::NewWindow, Key::ShortcutNewWindow),
        (MenuAction::Open, Key::ShortcutOpen),
        (MenuAction::QuickOpen, Key::ShortcutQuickOpen),
        (MenuAction::Save, Key::ShortcutSave),
        (MenuAction::SaveAs, Key::ShortcutSaveAs),
        (MenuAction::Export, Key::ShortcutExportHtml),
        (MenuAction::ExportPdf, Key::ShortcutExportPdf),
        (MenuAction::Print, Key::ShortcutPrint),
        (MenuAction::VersionHistory, Key::ShortcutVersionHistory),
        (MenuAction::Undo, Key::ShortcutUndo),
        (MenuAction::Redo, Key::ShortcutRedo),
        (MenuAction::Cut, Key::ShortcutCut),
        (MenuAction::Copy, Key::ShortcutCopy),
        (MenuAction::Paste, Key::ShortcutPaste),
        (MenuAction::SelectAll, Key::ShortcutSelectAll),
        (MenuAction::ToggleBold, Key::ShortcutBold),
        (MenuAction::ToggleItalic, Key::ShortcutItalic),
        (MenuAction::InsertLink, Key::ShortcutLink),
        (MenuAction::Find, Key::ShortcutFind),
        (MenuAction::Replace, Key::ShortcutReplace),
        (MenuAction::InsertImage, Key::ShortcutInsertImage),
        (MenuAction::ToggleOutline, Key::ShortcutOutline),
        (MenuAction::ZoomIn, Key::ShortcutZoomIn),
        (MenuAction::ZoomOut, Key::ShortcutZoomOut),
        (MenuAction::ZoomReset, Key::ShortcutZoomReset),
        (MenuAction::ToggleFullscreen, Key::ShortcutFullscreen),
        (MenuAction::ToggleSourceMode, Key::ShortcutSourceMode),
        (MenuAction::Settings, Key::ShortcutSettings),
        // 以下动作没有真实按键路由：明确不显示快捷键，不编造。
        (MenuAction::Quit, Key::Empty),
        (MenuAction::ThemeSystem, Key::Empty),
        (MenuAction::ThemeLight, Key::Empty),
        (MenuAction::ThemeDark, Key::Empty),
        (MenuAction::ToggleFocus, Key::Empty),
        (MenuAction::ToggleRemoteImages, Key::Empty),
        (MenuAction::About, Key::Empty),
        (MenuAction::ExportDiagnostics, Key::Empty),
        (MenuAction::CheckUpdate, Key::Empty),
    ];

    let mut seen = Vec::new();
    let mut shown = 0;
    for menu in MENUS {
        for (label, shortcut, action) in menu.items {
            if *action == MenuAction::Separator {
                continue;
            }
            assert!(
                !seen.contains(action),
                "动作 {action:?} 在主菜单出现两次，必须只有一个主入口"
            );
            seen.push(*action);
            let want = matrix
                .iter()
                .find(|(a, _)| a == action)
                .unwrap_or_else(|| panic!("命令矩阵缺少主菜单动作 {action:?}"))
                .1;
            assert_eq!(
                *shortcut,
                want,
                "{action:?} 的快捷键提示与命令矩阵不一致: {}",
                t(*shortcut)
            );
            assert!(!t(*label).is_empty(), "菜单标签不得为空: {action:?}");
            if want != Key::Empty {
                shown += 1;
                assert_eq!(
                    dispatch_shortcut(t(*shortcut)),
                    expected_input_action(*action),
                    "{action:?} 的提示键必须能真实分发"
                );
            }
        }
    }
    assert_eq!(seen.len(), 38, "主菜单应覆盖 38 个静态动作");
    assert_eq!(shown, 29, "矩阵内显示快捷键的动作数与既有红线一致");

    // 最近文件是动态项：出现在菜单里但没有（也不应有）快捷键提示。
    let rows = menu_rows(0, &["a.md".to_string()]);
    let (_, hint, action) = rows
        .iter()
        .find(|(_, _, a)| matches!(a, MenuAction::OpenRecent(_)))
        .expect("文件菜单应含最近文件");
    assert_eq!(*action, MenuAction::OpenRecent(0));
    assert_eq!(*hint, "", "最近文件没有可显示的快捷键");
}

/// 禁用项必须解释原因（Task 3.2）：与 `action_enabled` 同源，
/// 不允许出现「被置灰但没有任何原因」的非分隔线动作。
#[test]
fn disabled_menu_actions_always_explain_why() {
    let none = MenuFlags::default();
    let all = MenuFlags {
        can_save: true,
        can_export: true,
        can_undo: true,
        can_redo: true,
        ..MenuFlags::default()
    };
    let mut reasons = Vec::new();
    for menu in MENUS {
        for (_, _, action) in menu.items {
            if *action == MenuAction::Separator {
                assert_eq!(disabled_reason(*action, none), None, "分隔线不是禁用项");
                continue;
            }
            if action_enabled(*action, none) {
                assert_eq!(
                    disabled_reason(*action, none),
                    None,
                    "{action:?} 可用时不得带禁用原因"
                );
            } else {
                let reason = disabled_reason(*action, none)
                    .unwrap_or_else(|| panic!("{action:?} 被置灰却没有原因"));
                assert!(!t(reason).is_empty(), "{action:?} 的原因文案为空");
                reasons.push(reason);
            }
            assert!(
                action_enabled(*action, all),
                "{action:?} 在全部能力可用时应恢复可执行"
            );
        }
    }
    // 保存 / 导出三件套 / 撤销 / 重做：默认态共 6 项禁用，原因只来自 4 个词条。
    assert_eq!(reasons.len(), 6, "默认态应有 6 个禁用项: {reasons:?}");
    for want in [
        Key::DisabledNoChanges,
        Key::DisabledEmptyDocument,
        Key::DisabledNothingToUndo,
        Key::DisabledNothingToRedo,
    ] {
        assert!(reasons.contains(&want), "缺少禁用原因词条 {want:?}");
        assert!(!t(want).is_empty());
    }
    assert_eq!(
        reasons
            .iter()
            .filter(|r| **r == Key::DisabledEmptyDocument)
            .count(),
        3,
        "导出 HTML / 导出 PDF / 打印共享同一空文档原因"
    );
}

#[test]
fn dialog_commands_use_ellipsis_consistently() {
    for menu in MENUS {
        for (label, _, action) in menu.items {
            let should = matches!(
                action,
                MenuAction::Open
                    | MenuAction::SaveAs
                    | MenuAction::Export
                    | MenuAction::ExportPdf
                    | MenuAction::Print
                    | MenuAction::InsertImage
                    | MenuAction::Settings
                    | MenuAction::ExportDiagnostics
            );
            assert_eq!(
                t(*label).ends_with('…'),
                should,
                "省略号与是否打开后续对话框不一致: {action:?}"
            );
        }
    }
}

#[test]
fn print_entry_label_has_no_export_wording() {
    // 「打印…」只负责系统打印对话框；PDF 导出由上方独立的「导出 PDF」承载
    let print = MENUS[0]
        .items
        .iter()
        .find(|(_, _, a)| *a == MenuAction::Print)
        .expect("文件菜单应含打印项");
    assert_eq!(t(print.0), "打印…");
    assert_eq!(t(print.1), "Ctrl+Shift+P");
    assert!(
        !MENUS[0]
            .items
            .iter()
            .any(|(l, _, _)| t(*l).contains("打印") && t(*l).contains("导出 PDF")),
        "打印项不得再兼提导出 PDF"
    );
}

#[test]
fn view_menu_exposes_source_mode_toggle() {
    let item = MENUS[2]
        .items
        .iter()
        .find(|(_, _, a)| *a == MenuAction::ToggleSourceMode)
        .expect("视图菜单应含源码模式");
    assert_eq!(t(item.0), "源码模式");
    assert_eq!(t(item.1), "Ctrl+/");
    // 紧随其后的是专注模式，再往后回到分隔线
    let idx = MENUS[2]
        .items
        .iter()
        .position(|(_, _, a)| *a == MenuAction::ToggleSourceMode)
        .unwrap();
    assert_eq!(MENUS[2].items[idx + 1].2, MenuAction::ToggleFocus);
    assert_eq!(MENUS[2].items[idx + 2].2, MenuAction::Separator);
}

#[test]
fn remote_images_menu_uses_settings_wording() {
    let item = MENUS[2]
        .items
        .iter()
        .find(|(_, _, a)| *a == MenuAction::ToggleRemoteImages)
        .expect("视图菜单应含远程图片开关");
    assert_eq!(
        t(item.0),
        t(crate::i18n::Key::SettingsAllowRemoteImages),
        "菜单与设置必须使用同一文案"
    );
    assert_eq!(t(item.0), "允许远程图片");
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
        status_line(None, false, ExternalState::None, false, None, 0, 1, 1),
        ("未命名草稿".into(), "0 字 · 行 1，列 1".into())
    );
    assert_eq!(
        status_line(
            Some(Path::new("dir/a.md")),
            true,
            ExternalState::BackedUp,
            false,
            None,
            1234,
            12,
            3
        ),
        (
            // 脏态只用「未保存」表达（状态栏不再叠加 `*`，避免双重信号）
            "a.md · dir（外部修改已备份） · 未保存".into(),
            "1234 字 · 行 12，列 3".into()
        )
    );
    assert_eq!(
        status_line(
            Some(Path::new("b.md")),
            false,
            ExternalState::None,
            false,
            None,
            0,
            2,
            7
        )
        .0,
        "b.md"
    );
}

#[test]
fn status_line_shows_saved_time_only_when_clean() {
    let p = Some(Path::new("a.md"));
    assert_eq!(
        status_line(p, false, ExternalState::None, false, Some("14:32"), 0, 1, 1).0,
        "a.md · 已保存 14:32"
    );
    // 源码模式后缀顺序不变，保存态追加在最后
    assert_eq!(
        status_line(p, false, ExternalState::None, true, Some("09:05"), 0, 1, 1).0,
        "a.md（源码模式） · 已保存 09:05"
    );
    // 脏态优先：即便已有保存时刻也不得显示「已保存」
    assert_eq!(
        status_line(p, true, ExternalState::None, false, Some("14:32"), 0, 1, 1).0,
        "a.md · 未保存"
    );
}

#[test]
fn status_line_marks_source_mode() {
    let p = Some(Path::new("a.md"));
    assert_eq!(
        status_line(p, false, ExternalState::None, true, None, 0, 1, 1).0,
        "a.md（源码模式）"
    );
    // 与外部状态共存时追加在最后，关闭时逐字节等于旧输出
    assert_eq!(
        status_line(p, true, ExternalState::Changed, true, None, 0, 1, 1).0,
        "a.md（外部修改）（源码模式） · 未保存"
    );
    assert_eq!(
        status_line(p, true, ExternalState::Changed, false, None, 0, 1, 1).0,
        "a.md（外部修改） · 未保存"
    );
}

#[test]
fn status_line_marks_external_states() {
    let p = Some(Path::new("a.md"));
    assert_eq!(
        status_line(p, false, ExternalState::Changed, false, None, 0, 1, 1).0,
        "a.md（外部修改）"
    );
    assert_eq!(
        status_line(p, true, ExternalState::Missing, false, None, 0, 1, 1).0,
        "a.md（文件缺失） · 未保存"
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
    let long_path = Path::new("D:/")
        .join("很长的目录名".repeat(20))
        .join("文件.md");
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
            None,
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
        None,
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
    let mut frame = Frame::new(300, 260, [255, 255, 255, 255]);
    let theme = Theme::light();
    let mut chrome = Chrome {
        open: Some(0),
        ..Default::default()
    };
    let v = viewport(300, 260, 1.0);
    chrome.paint(
        &mut frame,
        &mut engine,
        v,
        1.0,
        &theme,
        None,
        false,
        ExternalState::None,
        None,
        0,
        1,
        1,
        2000.0,
        0.0,
        &[],
        MenuFlags::default(),
    );
    assert_eq!(frame.get(0, 0), palette::surface_card(&theme), "菜单栏底色");
    let status_y = 260 - STATUS_H as i32;
    assert_eq!(
        frame.get(299, (status_y + 5) as u32),
        palette::surface_card(&theme),
        "状态栏底色（采样点避开弹层投影）"
    );
    let p = popup_layout(0, 1.0, &[], 1000.0);
    assert_eq!(
        frame.get((p.x + p.w / 2.0) as u32, (p.y + p.h / 2.0) as u32),
        palette::surface_popup(&theme),
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
        None,
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
    assert!(
        !action_checked(MenuAction::Save, flags),
        "非开关项无「当前」"
    );
    assert!(
        !action_checked(MenuAction::ToggleOutline, MenuFlags::default()),
        "大纲面板不显示任何状态标识"
    );
    assert!(action_checked(
        MenuAction::ToggleFullscreen,
        MenuFlags {
            fullscreen: true,
            ..Default::default()
        }
    ));
    // 远程图片与设置同极性：勾选 = 允许（不再是反相的「阻止」）。
    assert!(action_checked(
        MenuAction::ToggleRemoteImages,
        MenuFlags {
            allow_remote_images: true,
            ..Default::default()
        }
    ));
    assert!(!action_checked(
        MenuAction::ToggleRemoteImages,
        MenuFlags::default()
    ));
    // 源码模式只认自己的开关位，不与其它模式串扰
    assert!(action_checked(
        MenuAction::ToggleSourceMode,
        MenuFlags {
            source: true,
            ..Default::default()
        }
    ));
    assert!(!action_checked(
        MenuAction::ToggleSourceMode,
        MenuFlags::default()
    ));
    assert!(!action_checked(
        MenuAction::ToggleFocus,
        MenuFlags {
            source: true,
            ..Default::default()
        }
    ));
    assert!(!action_checked(
        MenuAction::ToggleSourceMode,
        MenuFlags {
            focus: true,
            ..Default::default()
        }
    ));

    // 像素级：视图菜单「专注模式」行开启后多出右侧「当前」像素
    // 帧高需容纳源码模式项后的整张弹窗（10 行）
    let theme = Theme::light();
    let v = viewport(300, 400, 1.0);
    let p = popup_layout(2, 1.0, &[], 1000.0);
    let (_, ix, iy, iw, ih) = p.items[9];
    let shot = |on: bool| {
        let mut engine = LayoutEngine::new();
        let mut frame = Frame::new(300, 400, [255, 255, 255, 255]);
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
            None,
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
        None,
        0,
        1,
        1,
        10.0,
        0.0,
        &[],
        MenuFlags::default(),
    );
    assert_eq!(frame.get(0, 0), palette::surface_card(&theme), "深色菜单栏");
    assert_eq!(palette::surface_card(&theme), [38, 38, 38, 255]);
}

#[test]
fn theme_menu_marks_current_mode() {
    assert_eq!(MENUS.len(), 4);
    // 分隔线不参与动作序：去掉后仍是「大纲 → 缩放组 → 全屏 → 主题组 → 模式组」
    let view: Vec<MenuAction> = MENUS[2]
        .items
        .iter()
        .map(|(_, _, a)| *a)
        .filter(|a| *a != MenuAction::Separator)
        .collect();
    assert_eq!(
        view,
        vec![
            MenuAction::ToggleOutline,
            MenuAction::ZoomIn,
            MenuAction::ZoomOut,
            MenuAction::ZoomReset,
            MenuAction::ToggleFullscreen,
            MenuAction::ThemeSystem,
            MenuAction::ThemeLight,
            MenuAction::ThemeDark,
            MenuAction::ToggleSourceMode,
            MenuAction::ToggleFocus,
            MenuAction::ToggleRemoteImages,
            MenuAction::Settings,
        ]
    );
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
    let theme = Theme::light();
    let mut chrome = Chrome::default();
    let v = viewport(300, 200, 1.0);
    let mut draw = |chrome: &mut Chrome| {
        let mut frame = Frame::new(300, 200, [255, 255, 255, 255]);
        chrome.paint(
            &mut frame,
            &mut engine,
            v,
            1.0,
            &theme,
            None,
            false,
            ExternalState::None,
            None,
            0,
            1,
            1,
            3000.0,
            0.0,
            &[],
            MenuFlags::default(),
        );
        frame
    };
    // 滑块中线取样：胶囊右侧圆角处会落在轨迹上，必须取几何中心
    let x = (300.0 - SCROLL_W + 2.0 + 6.0) as u32;
    let y = (v.y + 10.0) as u32;

    let idle = draw(&mut chrome).get(x, y);
    assert_ne!(idle, palette::surface_card(&theme), "溢出时滑块必须可见");
    assert_ne!(idle, theme.thumb, "常态滑块为半透明，悬停才满不透明");

    chrome.scroll_hover = true;
    assert_eq!(draw(&mut chrome).get(x, y), theme.thumb, "悬停滑块加亮");
}

// ---- Task 1：状态栏优先级 / 通知降级 / 禁用原因呈现 ----

#[test]
fn status_tier_drops_low_priority_segments_first() {
    // 全量放得下：保持完整
    assert_eq!(pick_status_tier(300.0, 260.0, 500.0), StatusTier::Full);
    // 放不下全量但丢源码模式后放得下
    assert_eq!(pick_status_tier(500.0, 400.0, 450.0), StatusTier::NoSource);
    // 两者都放不下：降到「文件名 + 保存态」，由绘制层收缩文件名
    assert_eq!(
        pick_status_tier(500.0, 400.0, 300.0),
        StatusTier::Core,
        "优先级决策只允许先丢源码模式、再丢外部变更"
    );
    // Core 也必须能返回（不允许返回「无内容」）
    assert_eq!(pick_status_tier(999.0, 999.0, 0.0), StatusTier::Core);
}

#[test]
fn status_parts_keep_file_identity_and_save_state() {
    let parts = status_parts(
        Some(Path::new("dir/a.md")),
        true,
        ExternalState::Changed,
        true,
        Some("14:32"),
    );
    assert_eq!(parts.full(), "a.md · dir（外部修改）（源码模式） · 未保存");
    assert_eq!(parts.without_source(), "a.md · dir（外部修改） · 未保存");
    assert_eq!(parts.core(), "a.md · dir · 未保存");
    assert!(!parts.full().contains(" *"), "状态栏不得再有重复脏标");
}

/// 700 px 窗口 + 深目录长文件名 + 脏态 + 外部变更 + 源码模式：
/// 文件名与「未保存」必须同时可读，先被省略的是源码模式。
#[test]
fn status_layout_700px_long_path_keeps_filename_and_unsaved() {
    let mut engine = LayoutEngine::new();
    let mut chrome = Chrome::default();
    let long_name =
        "2026-11-21-ux-baseline-ji-shen-mu-lu-chang-wen-jian-ming-ce-shi-yong-li-abcdefghijklmnop.md";
    let path = Path::new("D:/资料归档-2026/第二层目录名称同样很长-用于验证路径截断/third-level-directory-with-a-rather-long-name/第四层")
        .join(long_name);
    let fit = chrome.layout_status(
        &mut engine,
        Some(&path),
        true,
        ExternalState::Changed,
        true,
        Some("14:32"),
        1234,
        5,
        23,
        700.0,
        1.0,
    );
    assert!(
        fit.left.contains("未保存"),
        "保存态不得被省略: {}",
        fit.left
    );
    assert!(
        fit.left.starts_with("2026-11-21-ux-baseline"),
        "文件名开头应保留: {}",
        fit.left
    );
    assert!(
        !fit.left.contains("源码模式"),
        "最低优先级的源码模式应先被丢弃: {}",
        fit.left
    );
    // 左右互不重叠：左文 + 间距 + 右文不超过右缘
    let left_x = crate::tokens::space::M;
    let right_edge = 700.0 - SCROLL_W - crate::tokens::space::M;
    let gap = crate::tokens::space::M;
    assert!(
        left_x + fit.left_w + gap + fit.right_w <= right_edge + 0.5,
        "左右状态文本重叠: left={} right={}",
        fit.left_w,
        fit.right_w
    );
    assert!(
        fit.right.contains("行 5"),
        "宽裕时右文保持完整: {}",
        fit.right
    );
}

/// 900 px 窗口 + 深目录长文件名 + 脏态 + 外部变更 + 源码模式：
/// 文件名与「未保存」仍须可读，且状态栏通知在此宽度下不得被吞。
#[test]
fn status_layout_900px_long_path_keeps_filename_and_unsaved() {
    let mut engine = LayoutEngine::new();
    let mut chrome = Chrome::default();
    let long_name =
        "2026-11-21-ux-baseline-ji-shen-mu-lu-chang-wen-jian-ming-ce-shi-yong-li-abcdefghijklmnop.md";
    let path = Path::new("D:/资料归档-2026/第二层目录名称同样很长-用于验证路径截断/third-level-directory-with-a-rather-long-name/第四层")
        .join(long_name);
    let fit = chrome.layout_status(
        &mut engine,
        Some(&path),
        true,
        ExternalState::Changed,
        true,
        Some("14:32"),
        1234,
        5,
        23,
        900.0,
        1.0,
    );
    assert!(
        fit.left.contains("未保存"),
        "保存态不得被省略: {}",
        fit.left
    );
    assert!(
        fit.left.starts_with("2026-11-21-ux-baseline"),
        "文件名开头应保留: {}",
        fit.left
    );
    // 左右互不重叠：左文 + 间距 + 右文不超过右缘
    let left_x = crate::tokens::space::M;
    let right_edge = 900.0 - SCROLL_W - crate::tokens::space::M;
    let gap = crate::tokens::space::M;
    assert!(
        left_x + fit.left_w + gap + fit.right_w <= right_edge + 0.5,
        "左右状态文本重叠: left={} right={}",
        fit.left_w,
        fit.right_w
    );
    assert!(
        fit.right.contains("行 5"),
        "宽裕时右文保持完整: {}",
        fit.right
    );

    // 同一宽度下带通知绘制：通知仍须可见，且不得被绘制层清空。
    let notice = "文件监听不可用: 没有权限；重新打开该文件即可再次尝试";
    let without = status_paint(900, 700, None, &mut chrome, &mut engine);
    let with = status_paint(900, 700, Some(notice), &mut chrome, &mut engine);
    let mut diff = 0usize;
    for y in 0..700 {
        for x in 0..900 {
            if without.get(x, y) != with.get(x, y) {
                diff += 1;
            }
        }
    }
    assert!(diff > 0, "900px 下状态栏通知必须仍然可见");
    assert_eq!(chrome.notice, notice, "绘制不得吞掉通知文本");
}

fn status_paint(
    w: u32,
    h: u32,
    notice: Option<&str>,
    chrome: &mut Chrome,
    engine: &mut LayoutEngine,
) -> Frame {
    let mut frame = Frame::new(w, h, [255, 255, 255, 255]);
    let theme = Theme::light();
    if let Some(n) = notice {
        chrome.notify_error(n);
    }
    chrome.paint(
        &mut frame,
        engine,
        viewport(w, h, 1.0),
        1.0,
        &theme,
        Some(Path::new("a.md")),
        true,
        ExternalState::None,
        None,
        0,
        1,
        1,
        100.0,
        0.0,
        &[],
        MenuFlags::default(),
    );
    frame
}

/// 极窄窗口：中段/右区都容不下通知时也必须画出来（本次退回最小浮层），不得静默丢弃。
#[test]
fn notice_still_painted_in_extremely_narrow_window() {
    let notice = "保存失败：拒绝访问。 (os error 5)；可 Ctrl+S 重试，或 Ctrl+Shift+S 另存为";
    let mut engine = LayoutEngine::new();
    let mut chrome = Chrome::default();
    let without = status_paint(140, 160, None, &mut chrome, &mut engine);
    let with = status_paint(140, 160, Some(notice), &mut chrome, &mut engine);
    let mut diff = 0usize;
    for y in 0..160 {
        for x in 0..140 {
            if without.get(x, y) != with.get(x, y) {
                diff += 1;
            }
        }
    }
    assert!(diff > 0, "窄窗口下通知必须仍然可见（不得直接不画）");
    assert_eq!(chrome.notice, notice, "绘制不得吞掉通知文本");
}

/// 极窄窗浮层（复审）：布局是纯函数；所有行矩形都在客户区内，高度不足时
/// 减少行数但首行与末行都保留，而不只是断言「有像素差」。
#[test]
fn notice_overlay_layout_keeps_rows_in_client_area_and_first_last() {
    let notice = "已从崩溃恢复「recover-me.md」的未保存内容；恢复时间 12:44。当前内容尚未保存，按 Ctrl+S 保存（写回原文件所在目录「recovery」）";
    // 同一逻辑尺寸（140×160）在 100% 与 150% 下的物理像素。
    for (w, h, scale) in [(140u32, 160u32, 1.0f32), (210, 240, 1.5)] {
        let mut engine = LayoutEngine::new();
        let pad = crate::tokens::space::M * scale;
        let inner = w as f32 - 2.0 * pad - 2.0 * crate::tokens::space::S * scale;
        let lines = super::paint::wrap_notice(&mut engine, notice, inner);
        assert!(lines.len() > 4, "用例前提：窄窗下须折出多行: {lines:?}");

        let layout = super::paint::notice_overlay_layout(lines.len(), w as f32, h as f32, scale);
        let status_y = h as f32 - STATUS_H * scale;
        assert!(!layout.rows.is_empty(), "浮层至少保留一行");
        assert!(
            layout.rows.len() < lines.len(),
            "空间不足必须减少行数: {} / {}",
            layout.rows.len(),
            lines.len()
        );
        for &ry in &layout.rows {
            assert!(ry >= 0.0, "行矩形越出客户区顶部: {ry}");
            assert!(
                ry + layout.line_h <= h as f32 + 0.5,
                "行矩形越出客户区底部: {ry}"
            );
            assert!(
                ry + layout.line_h <= status_y + 0.5,
                "行矩形越过状态栏上沿: {ry}"
            );
        }
        assert!(
            layout.y >= 0.0 && layout.y + layout.h <= h as f32 + 0.5,
            "浮层卡片越出客户区: y={} h={}",
            layout.y,
            layout.h
        );
        assert!(
            layout.h <= status_y - MENU_H * scale + 0.5,
            "浮层不得压住菜单栏: h={} status_y={status_y}",
            layout.h
        );

        let shown = super::paint::notice_overlay_lines(&lines, layout.rows.len());
        assert_eq!(shown.len(), layout.rows.len());
        assert_eq!(shown.first(), lines.first(), "首行必须可见");
        assert_eq!(shown.last(), lines.last(), "末行必须可见");
    }
}

#[test]
fn notice_painted_in_narrow_window_too() {
    let notice = "已恢复「a.md」的未保存内容（尚未保存）";
    let mut engine = LayoutEngine::new();
    let mut chrome = Chrome::default();
    let without = status_paint(320, 200, None, &mut chrome, &mut engine);
    let with = status_paint(320, 200, Some(notice), &mut chrome, &mut engine);
    let mut diff = 0usize;
    for y in 0..200 {
        for x in 0..320 {
            if without.get(x, y) != with.get(x, y) {
                diff += 1;
            }
        }
    }
    assert!(diff > 0, "通知在中段不足时必须走降级路径而非消失");
}

/// R1：700px + 长文件名 + 脏态 + 长通知。左文压缩到「文件名 + 保存态」即止，
/// 「 · 未保存」必须保留；通知要么完整放进中段/右文区，要么升级为浮层，绝不省略。
#[test]
fn notice_keeps_unsaved_state_at_700px_long_name() {
    let mut engine = LayoutEngine::new();
    let mut chrome = Chrome::default();
    let long_name =
        "2026-11-21-ux-after-ji-shen-mu-lu-chang-wen-jian-ming-ce-shi-yong-li-abcdefghijklmnop.md";
    let path = Path::new("D:/资料归档-2026/第二层目录名称同样很长-用于验证路径截断/third-level-directory-with-a-rather-long-name/第四层")
        .join(long_name);
    let notice = "保存失败：拒绝访问。 (os error 5)；可 Ctrl+S 重试，或 Ctrl+Shift+S 另存为";
    let (fit, slot) = chrome.layout_status_notice(
        &mut engine,
        Some(&path),
        true,
        ExternalState::Changed,
        true,
        None,
        1234,
        5,
        23,
        700.0,
        1.0,
        Some(notice),
    );
    assert!(
        fit.left.ends_with(" · 未保存"),
        "保存态不得被通知挤掉: {}",
        fit.left
    );
    assert_eq!(
        slot,
        super::paint::NoticeSlot::Overlay,
        "全文放不进状态栏时改浮层"
    );
}

/// R2：禁用原因计入弹层宽度后，动作名整名可见、原因同时可见（上限仍为 420px）。
#[test]
fn disabled_reason_reserves_width_so_action_name_is_whole() {
    let mut engine = LayoutEngine::new();
    let pad = crate::theme::surface::padding(1.0);
    for (menu, action, label, reason) in [
        (
            0,
            MenuAction::Export,
            t(Key::MenuFileExportHtml),
            t(Key::DisabledEmptyDocument),
        ),
        (
            1,
            MenuAction::Undo,
            t(Key::MenuEditUndo),
            t(Key::DisabledNothingToUndo),
        ),
    ] {
        let p = popup_layout(menu, 1.0, &[], 1000.0);
        assert!(p.w <= 420.0, "菜单宽度上限不得突破: {:.1}", p.w);
        let inner = p.w - 2.0 * pad;
        assert!(
            super::paint::measure_w(&mut engine, label) <= inner,
            "动作名不得被截断: {label}"
        );
        let label_measured = super::paint::measure_w(&mut engine, label);
        let reason_measured = super::paint::measure_w(&mut engine, reason);
        assert!(
            label_measured + crate::tokens::space::XL + reason_measured <= inner + 1.0,
            "动作名与禁用原因必须同时容纳: {action:?} / {reason} (label={label_measured} reason={reason_measured} inner={inner} p.w={})",
            p.w
        );
        assert!(
            super::text_width(label, 1.0) + 24.0 + super::text_width(reason, 1.0) <= inner + 0.5,
            "布局宽度必须为原因列预留: {action:?}"
        );
    }
}

/// R3：崩溃恢复通知含对象/时间/保存去向，任何省略都不允许留在状态栏；
/// 700px 下必须走浮层，并按内宽折行完整呈现关键字段。
#[test]
fn recovery_notice_overlay_keeps_object_time_and_destination() {
    let mut engine = LayoutEngine::new();
    let mut chrome = Chrome::default();
    let long_name =
        "2026-11-21-ux-after-ji-shen-mu-lu-chang-wen-jian-ming-ce-shi-yong-li-abcdefghijklmnop.md";
    let path = Path::new("D:/资料归档-2026/第二层目录名称同样很长-用于验证路径截断/third-level-directory-with-a-rather-long-name/第四层")
        .join(long_name);
    let notice = "已从崩溃恢复「recover-me.md」的未保存内容；恢复时间 12:44。当前内容尚未保存，按 Ctrl+S 保存（写回原文件所在目录「recovery」）";
    let (fit, slot) = chrome.layout_status_notice(
        &mut engine,
        Some(&path),
        true,
        ExternalState::Changed,
        true,
        None,
        23,
        1,
        1,
        700.0,
        1.0,
        Some(notice),
    );
    assert_eq!(
        slot,
        super::paint::NoticeSlot::Overlay,
        "需要省略时必须改用浮层"
    );
    assert!(fit.left.ends_with(" · 未保存"));

    let pad = crate::tokens::space::M;
    let inner = 700.0 - 2.0 * pad - 2.0 * crate::tokens::space::S;
    let lines = super::paint::wrap_notice(&mut engine, notice, inner);
    assert!(lines.len() > 1, "长通知必须折行而非省略: {lines:?}");
    assert!(
        !lines.iter().any(|l| l.contains('…')),
        "浮层不得出现省略号: {lines:?}"
    );
    let joined: String = lines.concat();
    for needle in ["recover-me.md", "12:44", "写回原文件所在目录「recovery」"] {
        assert!(joined.contains(needle), "浮层缺少关键字段「{needle}」");
    }
    for line in &lines {
        assert!(
            super::paint::measure_w(&mut engine, line) <= inner + 1.0,
            "折行超宽: {line}"
        );
    }
}

/// 禁用项不能只置灰：右键提示列改为 muted 的禁用原因（长原因可截断）。
#[test]
fn disabled_menu_item_paints_reason_text() {
    let theme = Theme::light();
    let p = popup_layout(0, 1.0, &[], 500.0);
    let (_, ix, iy, iw, ih) = *p
        .items
        .iter()
        .find(|(i, _, _, _, _)| *i == 5)
        .expect("文件菜单第 6 行应为「保存」");
    let zone = |flags: MenuFlags| -> Vec<[u8; 4]> {
        let mut engine = LayoutEngine::new();
        let mut frame = Frame::new(500, 400, [255, 255, 255, 255]);
        let mut chrome = Chrome {
            open: Some(0),
            ..Default::default()
        };
        chrome.paint(
            &mut frame,
            &mut engine,
            viewport(500, 400, 1.0),
            1.0,
            &theme,
            None,
            false,
            ExternalState::None,
            None,
            0,
            1,
            1,
            500.0,
            0.0,
            &[],
            flags,
        );
        let mut out = Vec::new();
        for y in iy as u32..(iy + ih) as u32 {
            for x in (ix + iw * 0.5) as u32..(ix + iw) as u32 {
                out.push(frame.get(x, y));
            }
        }
        out
    };
    let disabled = zone(MenuFlags::default());
    let enabled = zone(MenuFlags {
        can_save: true,
        ..MenuFlags::default()
    });
    assert_eq!(
        disabled_reason(MenuAction::Save, MenuFlags::default()),
        Some(Key::DisabledNoChanges),
        "保存禁用原因与 action_enabled 同源"
    );
    assert_ne!(
        disabled, enabled,
        "禁用行的右侧必须画出原因文本，而不是与快捷键列相同的空白/文案"
    );
}
