//! 会话状态（0b）：最近文件列表的编解码与更新策略，零 IO、零 GUI。
//!
//! 行格式存储（不伪装 TOML，解析容错优先）：每行 `recent=<路径>`，顺序即新旧
//! （首行 = 最近一次打开 = 会话恢复目标）。非法行/空路径静默忽略，最多保留
//! [`RECENT_CAP`] 条。路径不含换行符（Windows/Unix 均不允许），行格式安全。

use std::path::PathBuf;

#[cfg(test)]
use std::path::Path;

/// 最近文件上限：菜单可读性与存储量的平衡点。
pub(crate) const RECENT_CAP: usize = 10;

/// 解析会话文件 → 最近列表（保序去重、截断；坏行忽略）。文件首行即最新。
pub(crate) fn parse_session(text: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for line in text.lines() {
        let Some(p) = line.strip_prefix("recent=") else {
            continue;
        };
        if p.is_empty() {
            continue;
        }
        let path = PathBuf::from(p);
        if !out.contains(&path) {
            out.push(path);
        }
    }
    out.truncate(RECENT_CAP);
    out
}

/// 序列化最近列表。
pub(crate) fn encode_session(recent: &[PathBuf]) -> String {
    let mut out = String::new();
    for p in recent {
        out.push_str("recent=");
        out.push_str(&p.to_string_lossy());
        out.push('\n');
    }
    out
}

/// 记录一次打开：去重后插到头部，截断到上限。
pub(crate) fn touch_recent(recent: &mut Vec<PathBuf>, path: PathBuf) {
    recent.retain(|p| *p != path);
    recent.insert(0, path);
    recent.truncate(RECENT_CAP);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touch_dedups_and_caps() {
        let mut r = Vec::new();
        for i in 0..12 {
            touch_recent(&mut r, PathBuf::from(format!("f{i}.md")));
        }
        assert_eq!(r.len(), RECENT_CAP);
        assert_eq!(r[0], PathBuf::from("f11.md"));
        // 重复打开提到最前
        touch_recent(&mut r, PathBuf::from("f5.md"));
        assert_eq!(r[0], PathBuf::from("f5.md"));
        assert_eq!(r.len(), RECENT_CAP, "去重后不膨胀");
        assert_eq!(r.iter().filter(|p| p.as_path() == Path::new("f5.md")).count(), 1);
    }

    #[test]
    fn session_roundtrip_and_fault_tolerance() {
        let recent = vec![PathBuf::from("D:\\a.md"), PathBuf::from("相对/b.md")];
        let text = encode_session(&recent);
        assert_eq!(parse_session(&text), recent);
        // 坏行/空值/未知键不影响其余行；重复路径按首次出现保序
        let messy = "garbage\nrecent=\nrecent=D:\\x.md\nrecent=D:\\y.md\nrecent=D:\\x.md\nfoo=bar\n";
        assert_eq!(
            parse_session(messy),
            vec![PathBuf::from("D:\\x.md"), PathBuf::from("D:\\y.md")]
        );
        assert_eq!(parse_session(""), Vec::<PathBuf>::new());
    }

    #[test]
    fn parse_caps_at_ten() {
        let text: String = (0..20).map(|i| format!("recent=f{i}.md\n")).collect();
        assert_eq!(parse_session(&text).len(), RECENT_CAP);
    }
}
