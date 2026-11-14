use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Launch {
    Default,
    Open(PathBuf),
    NewWindow(Option<PathBuf>),
}

const USAGE: &str = "用法: veil [--new-window] [路径]";

/// 以 `-` 开头且不是终止符的参数：未知选项，不得回退为路径
/// （文件名以 `-` 开头时须用 `--` 显式终止选项解析）。
fn is_option(arg: &std::ffi::OsStr) -> bool {
    arg.to_string_lossy().starts_with('-')
}

fn unknown_option(arg: &std::ffi::OsStr) -> String {
    format!("未知参数：{}；{USAGE}", arg.to_string_lossy())
}

pub fn parse<I, S>(args: I) -> Result<Launch, String>
where
    I: IntoIterator<Item = S>,
    S: Into<std::ffi::OsString>,
{
    let mut args: Vec<std::ffi::OsString> = args.into_iter().map(Into::into).collect();
    // `--` 终止选项解析：其后全部按位置参数（文件名）处理（DEF-T10-001）。
    if args.first().is_some_and(|a| a == "--") {
        args.remove(0);
        return match args.as_slice() {
            [] => Ok(Launch::Default),
            [path] => Ok(Launch::Open(PathBuf::from(path))),
            _ => Err(USAGE.into()),
        };
    }
    match args.as_slice() {
        [] => Ok(Launch::Default),
        [flag] if flag == "--new-window" => Ok(Launch::NewWindow(None)),
        [flag, path] if flag == "--new-window" => {
            if path == "--" {
                Err(USAGE.into())
            } else if is_option(path) {
                Err(unknown_option(path))
            } else {
                Ok(Launch::NewWindow(Some(PathBuf::from(path))))
            }
        }
        [path] if is_option(path) => Err(unknown_option(path)),
        [path] => Ok(Launch::Open(PathBuf::from(path))),
        _ => Err(USAGE.into()),
    }
}

pub fn absolute_path(path: &Path) -> Result<PathBuf, String> {
    // 空参数必须在 join 之前拒绝：否则 `join("")` 会退化成当前目录
    // （DEF-T10-DEEP-001）。
    if path.as_os_str().is_empty() {
        return Err("文档路径无效".into());
    }
    if path.to_string_lossy().contains('\0') {
        return Err("文档路径无效".into());
    }
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| format!("无法确定当前目录: {e}"))?
            .join(path)
    };
    // 规范化 `.`/`..`：避免把 `D:\docs\..\notes.md` 原样交给 IPC/文件层；
    // 越过根目录（`..` 无法再回退）视为无效路径（DEF-T10-003）。
    let normalized = normalize_lexically(&absolute)?;
    if normalized.as_os_str().is_empty() {
        return Err("文档路径无效".into());
    }
    Ok(normalized)
}

/// 词法消解 `.`/`..`（不访问文件系统）；越过根目录时拒绝。
fn normalize_lexically(path: &Path) -> Result<PathBuf, String> {
    use std::path::Component;
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    return Err("文档路径越界".into());
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_launch_forms() {
        assert_eq!(parse(Vec::<String>::new()), Ok(Launch::Default));
        assert_eq!(
            parse(["notes.md"]),
            Ok(Launch::Open(PathBuf::from("notes.md")))
        );
        assert_eq!(parse(["--new-window"]), Ok(Launch::NewWindow(None)));
        assert_eq!(
            parse(["--new-window", "notes.md"]),
            Ok(Launch::NewWindow(Some(PathBuf::from("notes.md"))))
        );
        // `--` 终止选项解析：其后同名字符串必须按路径处理（DEF-T10-001）。
        assert_eq!(
            parse(["--", "--new-window"]),
            Ok(Launch::Open(PathBuf::from("--new-window")))
        );
        assert_eq!(
            parse(["--", "notes.md"]),
            Ok(Launch::Open(PathBuf::from("notes.md")))
        );
        assert_eq!(
            parse(["--", "-dash-name.md"]),
            Ok(Launch::Open(PathBuf::from("-dash-name.md")))
        );
    }

    #[test]
    fn rejects_extra_or_incomplete_arguments() {
        assert!(parse(["a.md", "b.md"]).is_err());
        assert!(parse(["--new-window", "a.md", "b.md"]).is_err());
        assert!(parse(["--new-window", "--"]).is_err());
        assert!(parse(["--", "--", "a.md"]).is_err());
    }

    #[test]
    fn rejects_unknown_options_instead_of_treating_them_as_paths() {
        // DEF-T10-002：未知选项必须明确报错，不得被当成文件名打开。
        for bad in [vec!["--bogus"], vec!["-x"], vec!["--new-window", "--bogus"]] {
            let err = parse(bad).unwrap_err();
            assert!(err.contains("未知参数"), "应报未知参数：{err}");
        }
    }

    #[test]
    fn rejects_explicit_empty_path_before_join() {
        // DEF-T10-DEEP-001：空参数不得回退到当前目录。
        assert!(absolute_path(Path::new("")).is_err());
        assert!(parse([""]) == Ok(Launch::Open(PathBuf::new())));
    }

    #[cfg(windows)]
    #[test]
    fn absolute_path_normalizes_dot_segments_and_keeps_clean_paths() {
        // DEF-T10-003：`..` 必须规范化后再交给 IPC/文件层。
        let got = absolute_path(Path::new(r"D:\docs\..\notes.md")).unwrap();
        assert_eq!(got, PathBuf::from(r"D:\notes.md"));
        let clean = absolute_path(Path::new(r"D:\docs\notes.md")).unwrap();
        assert_eq!(clean, PathBuf::from(r"D:\docs\notes.md"));
    }
}
