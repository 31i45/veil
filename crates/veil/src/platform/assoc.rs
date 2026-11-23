//! `.md` 文件关联（C2 / F1 / D7②）：仅 HKCU、仅显式命令、可精确撤销。
//!
//! 三条硬约束（D7②）：
//! 1. **不抢默认应用**：只填「空位」，任何已存在的他人值一律不改写；`UserChoice` 从不触碰
//!    （它不可脚本化，默认应用交给系统设置）。
//! 2. **不静默注册**：注册表只在 `/register`、`/unregister` 分支执行，启动路径不碰。
//! 3. **可撤销**：登记自己创建过什么（`Software\Veil\Assoc`），撤销时只删自己创建、
//!    且当前值仍等于自己写入值的项。
//!
//! 图标来自 exe 的第二组资源（资源 ID 2），`DefaultIcon` 用负数资源 ID `-2` 引用，
//! 不分发外置 `.ico`。写完调 `SHChangeNotify(SHCNE_ASSOCCHANGED)` 让资源管理器刷新。

// 非 Windows 构建下注册表常量与格式化函数只被单元测试使用
#![cfg_attr(not(windows), allow(dead_code))]

use veil_ui::i18n::{t, tf, Key as I18nKey};

/// 命令行动作：仅这两个开关进入关联分支，其余参数仍按「文档路径」处理。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssocCommand {
    Register,
    Unregister,
}

/// 解析 `/register`、`/unregister`（大小写不敏感；`-` 前缀同义）。
/// 纯函数：不接受裸词，避免把名为 `register` 的文件误当命令。
pub fn parse_command(arg: &str) -> Option<AssocCommand> {
    let rest = arg
        .trim()
        .strip_prefix('/')
        .or_else(|| arg.trim().strip_prefix('-'))?;
    match rest.to_ascii_lowercase().as_str() {
        "register" => Some(AssocCommand::Register),
        "unregister" => Some(AssocCommand::Unregister),
        _ => None,
    }
}

/// ProgID：本程序独占的命名空间，注册与撤销都以它为准。
pub const PROG_ID: &str = "Veil.Markdown";

/// 本程序写入状态的私有键：记录「哪几项是新建的」，撤销据此判定。
#[cfg(windows)]
const STATE_KEY: &str = r"Software\Veil\Assoc";
#[cfg(windows)]
const PROG_ID_KEY: &str = r"Software\Classes\Veil.Markdown";
#[cfg(windows)]
const MD_KEY: &str = r"Software\Classes\.md";
#[cfg(windows)]
const MD_OPEN_WITH_KEY: &str = r"Software\Classes\.md\OpenWithProgids";
/// 资源管理器为 `.md` 单独维护的「打开方式」列表；旧版本注册可能只留下这里的项。
#[cfg(windows)]
const MD_EXPLORER_OPEN_WITH_KEY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\.md\OpenWithProgids";
#[cfg(windows)]
const APP_ICON_KEY: &str = r"Software\Classes\Applications\veil.exe\DefaultIcon";

/// `DefaultIcon` 值：`"<exe>",-2` —— 负数表示「exe 内资源 ID 2」（文档图标）。
pub fn icon_ref(exe: &str) -> String {
    format!("\"{exe}\",-2")
}

/// `shell\open\command` 值：`"<exe>" "%1"`。
pub fn open_command(exe: &str) -> String {
    format!("\"{exe}\" \"%1\"")
}

/// `/register` 的「只填空位」判定：值不存在或为空白才允许写入。
/// 与注册表类型判断分离，便于在无注册表环境单测。
fn value_vacant(existing: Option<&str>) -> bool {
    existing.is_none_or(|v| v.trim().is_empty())
}

/// `/unregister` 的「仍属本程序」判定：当前值与本次写入记录逐字一致才允许删除。
/// 记录缺失（非本程序写入）或被他人改写时返回 false。
fn value_owned_by_us(current: Option<&str>, recorded: Option<&str>) -> bool {
    matches!((current, recorded), (Some(c), Some(r)) if c == r)
}

/// 执行关联动作；成功返回给用户看的结果说明，失败返回可读原因。
#[cfg(windows)]
pub fn run(cmd: AssocCommand) -> Result<String, String> {
    let exe =
        std::env::current_exe().map_err(|e| tf(I18nKey::AssocLocateExe, &[&e.to_string()]))?;
    let exe = exe
        .to_str()
        .ok_or_else(|| t(I18nKey::AssocExeNotUtf8).to_owned())?;
    match cmd {
        AssocCommand::Register => register(exe),
        AssocCommand::Unregister => unregister(exe),
    }
}

/// 非 Windows 兜底：本仓库只发布 Windows 构建，其余平台明确拒绝而不是假装成功。
#[cfg(not(windows))]
pub fn run(_cmd: AssocCommand) -> Result<String, String> {
    Err(t(I18nKey::AssocUnsupported).into())
}

#[cfg(windows)]
mod win {
    #![allow(non_snake_case)]

    use std::ffi::c_void;
    use std::ptr::{null, null_mut};

    use super::{
        icon_ref, open_command, value_owned_by_us, value_vacant, APP_ICON_KEY,
        MD_EXPLORER_OPEN_WITH_KEY, MD_KEY, MD_OPEN_WITH_KEY, PROG_ID, PROG_ID_KEY, STATE_KEY,
    };
    use crate::platform::wide;
    use veil_ui::i18n::{t, tf, Key as I18nKey};

    /// HKEY_CURRENT_USER = (LONG)0x80000001 按指针宽度符号扩展（64 位下高位全 1）。
    const HKEY_CURRENT_USER: isize = 0x8000_0001u32 as i32 as isize;
    const KEY_ALL_ACCESS: u32 = 0x000F_003F;
    const REG_SZ: u32 = 1;
    const REG_EXPAND_SZ: u32 = 2;
    const REG_NONE: u32 = 0;
    const REG_CREATED_NEW_KEY: u32 = 1;
    const ERROR_SUCCESS: i32 = 0;
    const ERROR_FILE_NOT_FOUND: i32 = 2;

    /// SHCNE_ASSOCCHANGED：文件关联变化，让资源管理器重读图标与关联。
    const SHCNE_ASSOCCHANGED: i32 = 0x0800_0000;
    /// SHCNF_IDLIST：item1/item2 为 ITEMIDLIST（此处传空，表示「全部」）。
    const SHCNF_IDLIST: u32 = 0;

    #[link(name = "advapi32")]
    extern "system" {
        fn RegCreateKeyExW(
            hkey: isize,
            sub: *const u16,
            reserved: u32,
            class: *const u16,
            options: u32,
            sam: u32,
            sa: *const c_void,
            out: *mut isize,
            disposition: *mut u32,
        ) -> i32;
        fn RegOpenKeyExW(
            hkey: isize,
            sub: *const u16,
            options: u32,
            sam: u32,
            out: *mut isize,
        ) -> i32;
        fn RegSetValueExW(
            hkey: isize,
            name: *const u16,
            reserved: u32,
            ty: u32,
            data: *const u8,
            cb: u32,
        ) -> i32;
        fn RegQueryValueExW(
            hkey: isize,
            name: *const u16,
            reserved: *mut u32,
            ty: *mut u32,
            data: *mut u8,
            cb: *mut u32,
        ) -> i32;
        fn RegDeleteValueW(hkey: isize, name: *const u16) -> i32;
        fn RegDeleteKeyW(hkey: isize, sub: *const u16) -> i32;
        fn RegDeleteTreeW(hkey: isize, sub: *const u16) -> i32;
        fn RegCloseKey(hkey: isize) -> i32;
    }

    #[link(name = "shell32")]
    extern "system" {
        fn SHChangeNotify(event: i32, flags: u32, item1: *const c_void, item2: *const c_void);
    }

    /// 已打开的注册表键：`Drop` 保证任何提前返回都不泄漏句柄。
    struct Key(isize);

    impl Drop for Key {
        fn drop(&mut self) {
            // SAFETY: 句柄由本模块的 RegCreateKeyExW/RegOpenKeyExW 产出且只关闭一次
            unsafe {
                RegCloseKey(self.0);
            }
        }
    }

    fn err(ctx: &str, code: i32) -> String {
        tf(
            I18nKey::AssocErrWithCode,
            &[
                ctx,
                &std::io::Error::from_raw_os_error(code).to_string(),
                &code.to_string(),
            ],
        )
    }

    fn name_ptr(name: Option<&str>) -> Vec<u16> {
        // 空 Vec 表示「默认值」：调用方只在 Some 时取指针
        name.map(wide).unwrap_or_default()
    }

    fn open_key(sub: &str) -> Result<Option<Key>, String> {
        let sub = wide(sub);
        let mut h: isize = 0;
        // SAFETY: sub 为 NUL 结尾 UTF-16；h 在返回后归 Key 所有
        let rc =
            unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, sub.as_ptr(), 0, KEY_ALL_ACCESS, &mut h) };
        match rc {
            ERROR_SUCCESS => Ok(Some(Key(h))),
            ERROR_FILE_NOT_FOUND => Ok(None),
            _ => Err(err(t(I18nKey::AssocOpenKeyFailed), rc)),
        }
    }

    fn create_key(sub: &str) -> Result<(Key, bool), String> {
        let sub = wide(sub);
        let mut h: isize = 0;
        let mut disp = 0u32;
        // SAFETY: 同 open_key；无安全属性（sa = null）即当前用户默认 ACL
        let rc = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                sub.as_ptr(),
                0,
                null(),
                0,
                KEY_ALL_ACCESS,
                null(),
                &mut h,
                &mut disp,
            )
        };
        if rc != ERROR_SUCCESS {
            return Err(err(t(I18nKey::AssocCreateKeyFailed), rc));
        }
        Ok((Key(h), disp == REG_CREATED_NEW_KEY))
    }

    fn set_sz(key: &Key, name: Option<&str>, value: &str) -> Result<(), String> {
        let name = name_ptr(name);
        let data = wide(value);
        // SAFETY: 两个缓冲在调用期间存活且 NUL 结尾；cb 按字节计（含终止符）
        let rc = unsafe {
            RegSetValueExW(
                key.0,
                if name.is_empty() {
                    null()
                } else {
                    name.as_ptr()
                },
                0,
                REG_SZ,
                data.as_ptr().cast::<u8>(),
                (data.len() * 2) as u32,
            )
        };
        if rc != ERROR_SUCCESS {
            return Err(err(t(I18nKey::AssocWriteValueFailed), rc));
        }
        Ok(())
    }

    /// 写零长度 `REG_NONE` 值：`OpenWithProgids` 只以「值存在」为标记。
    fn set_empty(key: &Key, name: &str) -> Result<(), String> {
        let name = wide(name);
        let empty: [u8; 0] = [];
        // SAFETY: name 为 NUL 结尾 UTF-16；cb = 0 时 data 不被解引用
        let rc = unsafe { RegSetValueExW(key.0, name.as_ptr(), 0, REG_NONE, empty.as_ptr(), 0) };
        if rc != ERROR_SUCCESS {
            return Err(err(t(I18nKey::AssocWriteValueFailed), rc));
        }
        Ok(())
    }

    /// 读原始字节 + 类型；值不存在返回 `None`（键不存在同）。
    fn read_raw(sub: &str, name: Option<&str>) -> Result<Option<(u32, Vec<u8>)>, String> {
        let Some(key) = open_key(sub)? else {
            return Ok(None);
        };
        let name = name_ptr(name);
        let name_ptr = if name.is_empty() {
            null()
        } else {
            name.as_ptr()
        };
        let mut ty = 0u32;
        let mut cb = 0u32;
        // SAFETY: 第一次调用只取类型与长度，data = null
        let rc =
            unsafe { RegQueryValueExW(key.0, name_ptr, null_mut(), &mut ty, null_mut(), &mut cb) };
        if rc == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        if rc != ERROR_SUCCESS {
            return Err(err(t(I18nKey::AssocReadValueFailed), rc));
        }
        let mut buf = vec![0u8; cb as usize];
        // SAFETY: buf 容量恰为 cb，由系统写入不超过该长度
        let rc = unsafe {
            RegQueryValueExW(
                key.0,
                name_ptr,
                null_mut(),
                &mut ty,
                buf.as_mut_ptr(),
                &mut cb,
            )
        };
        if rc != ERROR_SUCCESS {
            return Err(err(t(I18nKey::AssocReadValueFailed), rc));
        }
        buf.truncate(cb as usize);
        Ok(Some((ty, buf)))
    }

    /// 注册表字符串值 → `Option<String>`：仅 `REG_SZ` / `REG_EXPAND_SZ` 视为字符串。
    fn decode_sz(v: &Option<(u32, Vec<u8>)>) -> Option<String> {
        let (ty, data) = v.as_ref()?;
        if *ty != REG_SZ && *ty != REG_EXPAND_SZ {
            return None;
        }
        let units: Vec<u16> = data
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .take_while(|&u| u != 0)
            .collect();
        Some(String::from_utf16_lossy(&units))
    }

    fn read_sz(sub: &str, name: Option<&str>) -> Result<Option<String>, String> {
        Ok(decode_sz(&read_raw(sub, name)?))
    }

    /// 值是否空缺（不存在、零长度或纯空白）。非字符串类型视为已占用，不覆盖。
    fn is_vacant(sub: &str, name: Option<&str>) -> Result<bool, String> {
        let Some((ty, data)) = read_raw(sub, name)? else {
            return Ok(true);
        };
        if ty != REG_SZ && ty != REG_EXPAND_SZ {
            return Ok(false);
        }
        Ok(value_vacant(decode_sz(&Some((ty, data))).as_deref()))
    }

    fn delete_value(sub: &str, name: Option<&str>) -> Result<(), String> {
        let Some(key) = open_key(sub)? else {
            return Ok(());
        };
        let name = name_ptr(name);
        // SAFETY: 同上；名字缺省时删除「默认值」
        let rc = unsafe {
            RegDeleteValueW(
                key.0,
                if name.is_empty() {
                    null()
                } else {
                    name.as_ptr()
                },
            )
        };
        if rc == ERROR_SUCCESS || rc == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            Err(err(t(I18nKey::AssocDeleteValueFailed), rc))
        }
    }

    fn delete_tree(sub: &str) -> Result<(), String> {
        let sub = wide(sub);
        // SAFETY: sub 为 NUL 结尾 UTF-16，键不存在视为已完成
        let rc = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, sub.as_ptr()) };
        if rc == ERROR_SUCCESS || rc == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            Err(err(t(I18nKey::AssocDeleteKeyFailed), rc))
        }
    }

    /// 仅删除「确已为空」的键：有子键或子值时 Windows 会拒绝，别人写的内容不受影响。
    /// 失败（非空/不存在/无权限）一律忽略，不阻塞撤销。
    fn delete_key_if_empty(sub: &str) {
        let sub = wide(sub);
        // SAFETY: sub 为 NUL 结尾 UTF-16
        unsafe {
            RegDeleteKeyW(HKEY_CURRENT_USER, sub.as_ptr());
        }
    }

    fn set_state(name: &str, value: &str) -> Result<(), String> {
        let (key, _) = create_key(STATE_KEY)?;
        set_sz(&key, Some(name), value)
    }

    fn notify_shell() {
        // SAFETY: 空 ITEMIDLIST 表示「全部关联」，无所有权语义
        unsafe {
            SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, null(), null());
        }
    }

    pub fn register(exe: &str) -> Result<String, String> {
        let icon = icon_ref(exe);
        let command = open_command(exe);
        let description = t(I18nKey::AssocMarkdownDescription).to_owned();
        let icon_key = format!("{PROG_ID_KEY}\\DefaultIcon");
        let cmd_key = format!("{PROG_ID_KEY}\\shell\\open\\command");
        let mut notes: Vec<String> = Vec::new();
        let mut kept: Vec<&str> = Vec::new();

        // 1) 本程序命名空间：只填空缺值。他人已写入的描述/图标/命令一律不改写，
        //    并把保留项如实告知用户（DEF-T10-005）。
        if is_vacant(PROG_ID_KEY, None)? {
            let (prog, _) = create_key(PROG_ID_KEY)?;
            set_sz(&prog, None, &description)?;
            set_state("ProgIdDesc", &description)?;
        } else {
            kept.push(t(I18nKey::AssocProgIdDescription));
        }
        if is_vacant(&icon_key, None)? {
            let (key, _) = create_key(&icon_key)?;
            set_sz(&key, None, &icon)?;
            set_state("ProgIdIcon", &icon)?;
        } else {
            kept.push("DefaultIcon");
        }
        if is_vacant(&cmd_key, None)? {
            let (key, _) = create_key(&cmd_key)?;
            set_sz(&key, None, &command)?;
            set_state("ProgIdCmd", &command)?;
        } else {
            kept.push(t(I18nKey::AssocOpenCommand));
        }
        if !kept.is_empty() {
            notes.push(tf(I18nKey::AssocRegisterKept, &[&kept.join("、")]));
        }

        // 2) `.md` 的默认值：只填空位，绝不改写他人关联（不抢默认应用）
        match read_sz(MD_KEY, None)? {
            None => {
                let (md, _) = create_key(MD_KEY)?;
                set_sz(&md, None, PROG_ID)?;
                set_state("MdDefault", PROG_ID)?;
                notes.push(t(I18nKey::AssocDefaultSet).to_owned());
            }
            Some(v) if v.trim().is_empty() => {
                let (md, _) = create_key(MD_KEY)?;
                set_sz(&md, None, PROG_ID)?;
                set_state("MdDefault", PROG_ID)?;
                notes.push(t(I18nKey::AssocDefaultSet).to_owned());
            }
            Some(v) if v == PROG_ID => {}
            Some(_) => notes.push(t(I18nKey::AssocDefaultOther).to_owned()),
        }

        // 3) 「打开方式」列表：只加 Veil；已存在的项不重写。
        //    撤销时按 ProgID 清理，无需所有权记录。
        let (open_with, _) = create_key(MD_OPEN_WITH_KEY)?;
        if read_raw(MD_OPEN_WITH_KEY, Some(PROG_ID))?.is_none() {
            set_empty(&open_with, PROG_ID)?;
        }

        // 4) 直接关联到 veil.exe 的旧关联（资源管理器里显示「V」的根因）：只补空位
        if is_vacant(APP_ICON_KEY, None)? {
            let (app_icon, _) = create_key(APP_ICON_KEY)?;
            set_sz(&app_icon, None, &icon)?;
            set_state("AppIcon", &icon)?;
        } else if read_sz(APP_ICON_KEY, None)?.as_deref() != Some(icon.as_str()) {
            notes.push(t(I18nKey::AssocIconKept).to_owned());
        }

        notify_shell();

        let mut text = t(I18nKey::AssocRegisteredBody).to_owned();
        if !notes.is_empty() {
            text.push('\n');
            for n in &notes {
                text.push_str("\n- ");
                text.push_str(n);
            }
        }
        text.push_str(t(I18nKey::AssocManualDefault));
        Ok(text)
    }

    pub fn unregister(_exe: &str) -> Result<String, String> {
        let mut kept: Vec<&str> = Vec::new();

        // 1) 已关联到 veil.exe 的图标：只删本程序记录过、且当前值未被改写的项
        let recorded_icon = read_sz(STATE_KEY, Some("AppIcon"))?;
        let current_icon = read_sz(APP_ICON_KEY, None)?;
        if value_owned_by_us(current_icon.as_deref(), recorded_icon.as_deref()) {
            delete_value(APP_ICON_KEY, None)?;
            delete_key_if_empty(APP_ICON_KEY);
        } else if current_icon.is_some() {
            kept.push(t(I18nKey::AssocAppIcon));
        }

        // 2) 「打开方式」列表：Veil.Markdown 是本程序独占的 ProgID，凡存在即清——
        //    旧版本注册可能没留下所有权记录，但该项仍只属于 Veil；其他 ProgID 与
        //    UserChoice（用户默认应用）一律不碰。两处列表并存，都需要清理。
        for key in [MD_OPEN_WITH_KEY, MD_EXPLORER_OPEN_WITH_KEY] {
            if read_raw(key, Some(PROG_ID))?.is_some() {
                delete_value(key, Some(PROG_ID))?;
            }
        }

        // 3) `.md` 默认值：只删本程序写入且仍指向本程序的值
        let recorded_md = read_sz(STATE_KEY, Some("MdDefault"))?;
        let current_md = read_sz(MD_KEY, None)?;
        if recorded_md.as_deref() == Some(PROG_ID) && current_md.as_deref() == Some(PROG_ID) {
            delete_value(MD_KEY, None)?;
        } else if current_md.as_deref() == Some(PROG_ID) {
            kept.push(t(I18nKey::AssocMarkdownDefault));
        }

        // 4) 本程序命名空间：逐值核对所有权，只删自己写入且未被改写的值；
        //    绝不整树删除（他人可能在同一 ProgID 下追加内容）。
        let icon_key = format!("{PROG_ID_KEY}\\DefaultIcon");
        let cmd_key = format!("{PROG_ID_KEY}\\shell\\open\\command");
        for (sub, state, label) in [
            (
                PROG_ID_KEY,
                "ProgIdDesc",
                t(I18nKey::AssocProgIdDescription),
            ),
            (icon_key.as_str(), "ProgIdIcon", "DefaultIcon"),
            (cmd_key.as_str(), "ProgIdCmd", t(I18nKey::AssocOpenCommand)),
        ] {
            let current = read_sz(sub, None)?;
            let recorded = read_sz(STATE_KEY, Some(state))?;
            if value_owned_by_us(current.as_deref(), recorded.as_deref()) {
                delete_value(sub, None)?;
            } else if current.is_some() {
                kept.push(label);
            }
        }
        // 只删已成空的键（有子键/子值时 Windows 会拒绝，他方内容原样保留）。
        delete_key_if_empty(&cmd_key);
        delete_key_if_empty(&format!("{PROG_ID_KEY}\\shell\\open"));
        delete_key_if_empty(&format!("{PROG_ID_KEY}\\shell"));
        delete_key_if_empty(&icon_key);
        delete_key_if_empty(PROG_ID_KEY);

        delete_tree(STATE_KEY)?;
        delete_key_if_empty(r"Software\Veil");
        notify_shell();
        let mut text = t(I18nKey::AssocUnregisteredBody).to_owned();
        if !kept.is_empty() {
            text.push_str("\n\n");
            text.push_str(&tf(I18nKey::AssocUnregisterKept, &[&kept.join("、")]));
        }
        Ok(text)
    }
}

#[cfg(windows)]
use win::{register, unregister};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_both_commands_case_insensitively() {
        let cases = [
            ("/register", AssocCommand::Register),
            ("/REGISTER", AssocCommand::Register),
            ("-Register", AssocCommand::Register),
            (" /unregister ", AssocCommand::Unregister),
        ];
        for (s, want) in cases {
            assert_eq!(parse_command(s), Some(want), "{s}");
        }
    }

    /// 文件路径与未知开关不得被当成命令（`register` 裸词也不行）。
    #[test]
    fn ignores_paths_and_unknown_switches() {
        for s in ["notes.md", "register", "/register.md", "/reg", "-u", ""] {
            assert_eq!(parse_command(s), None, "{s}");
        }
    }

    /// 负数资源 ID 是「exe 内资源 2」的唯一正确写法（正数会按图标序号解释）。
    #[test]
    fn icon_ref_uses_negative_resource_id() {
        assert_eq!(icon_ref(r"D:\Apps\veil.exe"), r#""D:\Apps\veil.exe",-2"#);
    }

    #[test]
    fn open_command_quotes_exe_and_placeholder() {
        assert_eq!(
            open_command(r"D:\Apps\veil.exe"),
            r#""D:\Apps\veil.exe" "%1""#
        );
    }

    /// 路径含空格/中文时保留引号，命令才能被 shell 正确切分。
    #[test]
    fn keeps_quotes_for_paths_with_spaces() {
        let exe = r"D:\我的 工具\Veil\veil.exe";
        assert!(icon_ref(exe).starts_with('"'));
        assert!(open_command(exe).ends_with("\" \"%1\""));
        assert!(open_command(exe).contains("我的 工具"));
    }

    /// `/register` 只填空位：已存在的非空值一律不改写（DEF-T10-005）。
    #[test]
    fn register_only_fills_vacant_values() {
        assert!(value_vacant(None), "不存在算空缺");
        assert!(value_vacant(Some("")), "零长度算空缺");
        assert!(value_vacant(Some("   ")), "纯空白算空缺");
        for occupied in ["Veil.Markdown", "Other.App", "\"D:\\a.exe\",-2"] {
            assert!(!value_vacant(Some(occupied)), "非空值不得覆盖: {occupied}");
        }
    }

    /// `/unregister` 只删本次创建且仍属 Veil 的值（DEF-T10-006）。
    #[test]
    fn unregister_only_deletes_owned_values() {
        assert!(value_owned_by_us(Some("v"), Some("v")), "值未变才可删");
        assert!(!value_owned_by_us(Some("v2"), Some("v")), "他人改写须保留");
        assert!(!value_owned_by_us(Some("v"), None), "无写入记录须保留");
        assert!(
            !value_owned_by_us(None, Some("v")),
            "值已被删除不报错但也不删"
        );
        assert!(!value_owned_by_us(None, None));
    }

    /// 键路径必须是相对 HKCU 的路径（不以反斜杠开头），且 ProgID 一致。
    #[cfg(windows)]
    #[test]
    fn registry_paths_are_relative_and_consistent() {
        for key in [
            PROG_ID_KEY,
            MD_KEY,
            MD_OPEN_WITH_KEY,
            MD_EXPLORER_OPEN_WITH_KEY,
            APP_ICON_KEY,
            STATE_KEY,
        ] {
            assert!(!key.starts_with('\\'), "{key}");
            assert!(!key.ends_with('\\'), "{key}");
        }
        assert!(PROG_ID_KEY.ends_with(PROG_ID));
    }
}
