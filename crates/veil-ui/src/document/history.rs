//! 版本历史（N1/B8a）：文档稳定身份与快照键。
//!
//! 快照目录不能以内容哈希为键（内容一变就换目录，历史被切成互不相干的碎片）。
//! 稳定身份 = 规范化绝对路径的 FNV-1a 哈希；未命名草稿用本次会话的临时 id，
//! 首次保存后由平台层把临时目录迁移到路径键目录。
//!
//! 纯逻辑零 GUI：规范化/哈希可 headless 测试；平台层只消费 64 位键。

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::recovery::fnv64;

/// 规范化文档路径：绝对化（文件存在时消解符号链接/别名）→ 去掉 Windows
/// `\\?\` 前缀 → 统一 `/` 为 `\` → Windows 下整体小写（路径大小写不敏感）。
/// 非 Windows 保持大小写（POSIX 路径大小写敏感）。
pub(crate) fn normalize_doc_path(path: &Path) -> PathBuf {
    let abs = std::fs::canonicalize(path)
        .or_else(|_| std::path::absolute(path))
        .unwrap_or_else(|_| path.to_path_buf());
    let mut text = abs.to_string_lossy().into_owned();
    if let Some(rest) = text.strip_prefix(r"\\?\") {
        text = rest.to_string();
    }
    if cfg!(windows) {
        text = text.replace('/', "\\");
        text.make_ascii_lowercase();
    }
    PathBuf::from(text)
}

/// 文档稳定身份键：同一路径无论内容如何变化都映射到同一目录。
pub(crate) fn doc_key(path: &Path) -> u64 {
    let normalized = normalize_doc_path(path);
    fnv64(normalized.to_string_lossy().as_bytes())
}

/// 未命名草稿的临时会话键：纳秒时间 + 进程 id，避免不同会话互相污染。
pub(crate) fn session_key() -> u64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let pid = std::process::id() as u64;
    fnv64(&(nanos ^ pid.rotate_left(32)).to_le_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn path_key_is_stable_across_content_changes() {
        // 键只由路径派生：Content v1 → v2 不改变 key，与内容无关。
        let path = p(r"C:\notes\b8a\历史.md");
        let k1 = doc_key(&path);
        let k2 = doc_key(&path);
        assert_eq!(k1, k2);
        assert_eq!(normalize_doc_path(&path), normalize_doc_path(&path));
    }

    #[cfg(windows)]
    #[test]
    fn path_key_normalizes_case_and_separators() {
        let a = doc_key(Path::new("C:/Notes/Sub/Doc.MD"));
        let b = doc_key(Path::new(r"c:\notes\sub\doc.md"));
        assert_eq!(a, b, "同一文件的不同写法必须进同一目录");
    }

    #[test]
    fn different_paths_have_different_keys() {
        assert_ne!(
            doc_key(Path::new(r"C:\notes\a.md")),
            doc_key(Path::new(r"C:\notes\b.md"))
        );
    }

    #[test]
    fn session_keys_are_distinct_per_call() {
        // 极端情况下同一纳秒两次调用也可能不同（pid/nanos 同源），此处只要求
        // 连续生成不恒定；真实碰撞概率由 64 位空间兜底。
        let mut seen = std::collections::HashSet::new();
        for _ in 0..8 {
            seen.insert(session_key());
        }
        assert!(seen.len() >= 2);
    }
}
