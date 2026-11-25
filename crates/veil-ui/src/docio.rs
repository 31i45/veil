//! 文件字节 ↔ 文档文本编解码：纯逻辑零 IO（IO 由 FileStore trait 注入）。
//! 不变量：打开 → 不编辑 → 保存，输出字节 == 输入字节（INSTANCE §3 幂等不变量）。
//! - BOM：读入剥离、保存还原（`bom` 标记随行），否则往返不再字节一致。
//! - CRLF：内核原样保留 `\r\n`，本层不做任何换行归一化。
//! - 非法 UTF-8：拒绝打开并给可读错误，不用 lossy 替换（lossy 会静默破坏字节一致）。

const BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

/// 文件字节 → (文本, 是否带 BOM)。非法 UTF-8 返回 Err。
pub fn decode(bytes: &[u8]) -> Result<(String, bool), String> {
    let (bom, rest) = match bytes.strip_prefix(&BOM) {
        Some(r) => (true, r),
        None => (false, bytes),
    };
    // String::from_utf8 逐字节校验：任何非法序列（含孤立代理字节、截断多字节）都拒绝
    String::from_utf8(rest.to_vec())
        .map(|t| (t, bom))
        .map_err(|_| crate::i18n::t(crate::i18n::Key::ErrNotUtf8).to_string())
}

/// (文本, 是否带 BOM) → 文件字节。`decode ∘ encode` 为恒等。
pub fn encode(text: &str, bom: bool) -> Vec<u8> {
    let mut v = Vec::with_capacity(text.len() + if bom { 3 } else { 0 });
    if bom {
        v.extend_from_slice(&BOM);
    }
    v.extend_from_slice(text.as_bytes());
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

    #[test]
    fn plain_utf8_no_bom() {
        assert_eq!(decode(b"abc"), Ok(("abc".into(), false)));
    }

    /// BOM + 内容拼成测试字节串。
    fn bombed(s: &[u8]) -> Vec<u8> {
        let mut v = BOM.to_vec();
        v.extend_from_slice(s);
        v
    }

    #[test]
    fn bom_is_stripped_and_flagged() {
        assert_eq!(decode(&bombed(b"abc")), Ok(("abc".into(), true)));
    }

    #[test]
    fn invalid_utf8_is_rejected() {
        assert!(decode(&[0x61, 0xFF, 0x62]).is_err()); // 0xFF 非合法 UTF-8
    }

    #[test]
    fn empty_file_is_empty_text() {
        assert_eq!(decode(b""), Ok((String::new(), false)));
        assert_eq!(decode(&BOM), Ok((String::new(), true))); // 仅 BOM 也是合法空文档
    }

    #[test]
    fn crlf_passes_through_untouched() {
        assert_eq!(decode(b"a\r\nb"), Ok(("a\r\nb".into(), false)));
    }

    #[test]
    fn encode_writes_bom_prefix() {
        assert_eq!(encode("abc", false), b"abc".to_vec());
        assert_eq!(encode("abc", true), bombed(b"abc"));
    }

    #[test]
    fn roundtrip_identity() {
        // 幂等不变量的编解码半边：含 BOM、CRLF、多字节的语料逐字节往返
        for (text, bom) in [("中\r\nx", true), ("中\r\nx", false), ("", true)] {
            assert_eq!(decode(&encode(text, bom)), Ok((text.to_string(), bom)));
        }
    }

    // ---- 恢复路径矩阵（qq-task3）----
    //
    // 原子写（tmp + rename）、备份策略与外部修改检测的实现分布在 `veil` 产品层
    // （`Fs::save`）与本层 `App::save`（`document::doc_state`）。测试沿用现有
    // 「注入 FileStore 端口」的替身模式（见 `app_tests::MemStore` / `ports`），
    // 仅在需要真实文件系统语义（只读目标、rename 前中断）时定义局部替身，
    // 不为测试新增任何生产代码路径。

    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::rc::Rc;

    use crate::app::{App, Doc, FileStore};
    use crate::document::doc_state::SaveKind;
    use crate::document::ExternalState;
    use crate::editor::Intent;

    /// 复用既有装配（`app_tests::ports`）再把 store 端口替换为故障注入替身。
    /// `store` 由调用方持有（Rc 内部可变），可在 App 构造后模拟外部改写/故障翻转。
    fn app_with_store(store: Box<dyn FileStore>, text: &str, path: &Path) -> App {
        let mut p = crate::app_tests::ports(
            &crate::app_tests::MemStore::default(),
            crate::app_tests::no_url(),
        );
        p.store = store;
        App::new(
            Doc {
                text: text.into(),
                path: Some(path.to_path_buf()),
                bom: false,
                resume: false,
                open_error: None,
            },
            p,
        )
    }

    #[derive(Default)]
    struct StoreState {
        files: HashMap<PathBuf, Vec<u8>>,
        /// 冲突副本写入故障（文件名含 `.conflict-` 时生效）。
        fail_conflict: Option<String>,
        /// 主目标写入故障（其余路径生效）。
        fail_target: Option<String>,
    }

    /// 内存故障注入 store：可断言磁盘内容，也可指定「备份失败」「磁盘已满」等注入点。
    #[derive(Clone, Default)]
    struct FaultStore {
        state: Rc<RefCell<StoreState>>,
    }

    impl FaultStore {
        fn disk(&self, path: &str) -> Option<Vec<u8>> {
            self.state.borrow().files.get(Path::new(path)).cloned()
        }
        fn set_disk(&self, path: &str, bytes: &[u8]) {
            self.state
                .borrow_mut()
                .files
                .insert(PathBuf::from(path), bytes.to_vec());
        }
        fn fail_conflict_with(&self, msg: &str) {
            self.state.borrow_mut().fail_conflict = Some(msg.into());
        }
        fn fail_target_with(&self, msg: &str) {
            self.state.borrow_mut().fail_target = Some(msg.into());
        }
    }

    impl FileStore for FaultStore {
        fn load(&mut self, path: &Path) -> Result<Vec<u8>, String> {
            self.state
                .borrow()
                .files
                .get(path)
                .cloned()
                .ok_or_else(|| "文件不存在".to_string())
        }
        fn save(&mut self, path: &Path, data: &[u8]) -> Result<(), String> {
            let is_conflict = path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.contains(".conflict-"));
            let injected = {
                let st = self.state.borrow();
                if is_conflict {
                    st.fail_conflict.clone()
                } else {
                    st.fail_target.clone()
                }
            };
            if let Some(msg) = injected {
                return Err(msg);
            }
            self.state
                .borrow_mut()
                .files
                .insert(path.to_path_buf(), data.to_vec());
            Ok(())
        }
    }

    /// 真实文件系统 store：语义与产品层 `Fs::save` 对齐（同目录 `<path>.veil-tmp`
    /// 再 rename；失败不清理临时文件）。`CrashBeforeRename` 模拟「临时文件已写、
    /// rename 前进程被杀」。
    #[derive(Clone, Copy)]
    enum FsMode {
        Normal,
        CrashBeforeRename,
    }

    struct FsStore {
        mode: FsMode,
    }

    impl FileStore for FsStore {
        fn load(&mut self, path: &Path) -> Result<Vec<u8>, String> {
            std::fs::read(path).map_err(|e| format!("读取失败: {e}"))
        }
        fn save(&mut self, path: &Path, data: &[u8]) -> Result<(), String> {
            let mut tmp_os = path.as_os_str().to_owned();
            tmp_os.push(".veil-tmp");
            let tmp = PathBuf::from(tmp_os);
            std::fs::write(&tmp, data).map_err(|e| format!("写入失败: {e}"))?;
            if matches!(self.mode, FsMode::CrashBeforeRename) {
                // 目标保持原样：等价于 rename 前进程被强行终止。
                return Err("模拟：rename 前进程终止".into());
            }
            std::fs::rename(&tmp, path).map_err(|e| format!("写入失败: {e}"))
        }
    }

    /// 唯一临时目录，Drop 时清只读属性后递归删除。
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let dir = std::env::temp_dir().join(format!(
                "veil-recovery-{tag}-{}-{nanos}",
                std::process::id()
            ));
            std::fs::create_dir_all(&dir).expect("创建临时目录");
            Self(dir)
        }
        fn join(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = clear_readonly(&self.0);
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn clear_readonly(dir: &Path) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                clear_readonly(&entry.path())?;
            } else {
                let mut perms = entry.metadata()?.permissions();
                if perms.readonly() {
                    perms.set_readonly(false);
                    std::fs::set_permissions(entry.path(), perms)?;
                }
            }
        }
        Ok(())
    }

    /// 写入中断：临时文件已落盘、rename 前失败 → 原文件不被覆盖，临时文件不成为目标。
    #[test]
    fn recovery_interrupted_write_keeps_original_and_temp_never_replaces_target() {
        let dir = TempDir::new("interrupted");
        let target = dir.join("a.md");
        std::fs::write(&target, b"original").expect("预置原文件");

        let mut a = app_with_store(
            Box::new(FsStore {
                mode: FsMode::CrashBeforeRename,
            }),
            "original",
            &target,
        );
        a.editor.apply(Intent::InsertText("new".into())).unwrap();
        let pending = encode(&a.editor.session().serialize(), false);
        a.save_now();

        assert_eq!(
            std::fs::read(&target).unwrap(),
            b"original".to_vec(),
            "中断写入不得改动目标文件"
        );
        let tmp = target.with_extension("md.veil-tmp");
        assert_eq!(
            std::fs::read(&tmp).unwrap(),
            pending,
            "临时文件承载的候选版本不得冒充目标文件"
        );
        assert_ne!(
            std::fs::read(&target).unwrap(),
            std::fs::read(&tmp).unwrap(),
            "目标仍是旧内容，候选版本只在临时文件里"
        );
        assert!(a.editor.is_dirty(), "失败后必须保持脏态");
        assert!(a.ui_state.modal.is_some(), "手工保存失败须有可见错误出口");
    }

    /// 备份失败：主写入必须整体取消，磁盘上的外部版本不被覆盖、不产生半成品副本。
    #[test]
    fn recovery_backup_failure_cancels_save_and_keeps_external_version() {
        let store = FaultStore::default();
        store.set_disk("a.md", b"base");
        let mut a = app_with_store(Box::new(store.clone()), "base", Path::new("a.md"));
        a.editor.apply(Intent::InsertText("改".into())).unwrap();
        store.set_disk("a.md", b"external"); // 外部改写
        store.fail_conflict_with("模拟备份写失败");
        a.save_now();

        assert_eq!(
            store.disk("a.md"),
            Some(b"external".to_vec()),
            "设计：备份失败即取消保存，绝不无备份覆盖"
        );
        assert_eq!(
            store.disk("a.conflict-1.md"),
            None,
            "备份失败不得留下半成品副本"
        );
        assert!(a.editor.is_dirty(), "取消保存后修改仍在编辑器");
        assert!(!a.persist_state.persist.backed_up());
        let modal = a.ui_state.modal.as_ref().expect("须弹错误弹窗");
        assert!(
            modal.spec.message.contains("模拟备份写失败"),
            "错误原因须可见：{}",
            modal.spec.message
        );
    }

    /// 只读目标：rename 覆盖失败必须返回可读错误，原文件完好、脏态保留。
    ///
    /// 说明：当前生产实现 `veil/src/lib.rs` 的 `Fs::save` 在 rename 失败后**不清理**
    /// `.veil-tmp`（见 `evidence/verify/qq-task3/findings.md`）。本测试锁定数据安全
    /// 不变量（目标未被破坏），不把该残留行为当作期望值。
    #[test]
    fn recovery_read_only_target_fails_without_clobbering() {
        let dir = TempDir::new("readonly");
        let target = dir.join("a.md");
        std::fs::write(&target, b"original").expect("预置原文件");
        let mut perms = std::fs::metadata(&target).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&target, perms).unwrap();

        let mut a = app_with_store(
            Box::new(FsStore {
                mode: FsMode::Normal,
            }),
            "original",
            &target,
        );
        a.editor.apply(Intent::InsertText("new".into())).unwrap();
        a.save_now();

        assert_eq!(
            std::fs::read(&target).unwrap(),
            b"original".to_vec(),
            "只读目标不得被破坏"
        );
        assert!(a.editor.is_dirty(), "失败后必须保持脏态");
        let modal = a.ui_state.modal.as_ref().expect("须弹错误弹窗");
        assert!(
            modal.spec.message.contains("a.md"),
            "错误须指明目标文件：{}",
            modal.spec.message
        );
    }

    /// 磁盘满：写入返回可辨识的存储错误 → 原文件不动、脏态保留、原因可见。
    #[test]
    fn recovery_disk_full_returns_identifiable_error_without_data_loss() {
        let store = FaultStore::default();
        store.set_disk("a.md", b"original");
        store.fail_target_with("磁盘已满（StorageFull / os error 112）");
        let mut a = app_with_store(Box::new(store.clone()), "original", Path::new("a.md"));
        a.editor.apply(Intent::InsertText("x".into())).unwrap();
        a.save_now();

        assert_eq!(
            store.disk("a.md"),
            Some(b"original".to_vec()),
            "磁盘满时目标文件必须保持原样"
        );
        assert!(a.editor.is_dirty());
        let modal = a.ui_state.modal.as_ref().expect("须弹错误弹窗");
        assert!(
            modal.spec.message.contains("磁盘已满") && modal.spec.message.contains("StorageFull"),
            "错误须可辨识为存储耗尽：{}",
            modal.spec.message
        );
    }

    /// 外部修改冲突：自动保存拒绝覆盖（先备份外部版本并挂起），手工保存才覆盖。
    #[test]
    fn recovery_external_edit_blocks_autosave_then_manual_save() {
        let store = FaultStore::default();
        store.set_disk("a.md", b"base");
        let mut a = app_with_store(Box::new(store.clone()), "base", Path::new("a.md"));
        a.editor.apply(Intent::InsertText("改".into())).unwrap();
        store.set_disk("a.md", b"external"); // 打开后外部改写

        a.save(SaveKind::Auto);
        assert_eq!(
            store.disk("a.md"),
            Some(b"external".to_vec()),
            "自动保存绝不覆盖外部修改"
        );
        assert_eq!(
            store.disk("a.conflict-1.md"),
            Some(b"external".to_vec()),
            "外部版本须先落成冲突副本"
        );
        assert!(a.persist_state.persist.hold(), "自动保存须挂起");
        assert!(a.editor.is_dirty());
        assert_eq!(a.external_state(), ExternalState::BackedUp);

        // 手工保存：显式意图，允许在备份后覆盖；同一外部版本不重复备份。
        a.save_now();
        assert_eq!(
            store.disk("a.md"),
            Some(encode(&a.editor.session().serialize(), false)),
            "手工保存应写入当前内容"
        );
        assert_eq!(
            store.disk("a.conflict-1.md"),
            Some(b"external".to_vec()),
            "外部版本仍保留"
        );
        assert!(!a.editor.is_dirty());
        assert!(!a.persist_state.persist.hold(), "手工保存解除挂起");
    }
}
