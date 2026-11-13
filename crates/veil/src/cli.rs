use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Launch {
    Default,
    Open(PathBuf),
    NewWindow(Option<PathBuf>),
}

pub fn parse<I, S>(args: I) -> Result<Launch, String>
where
    I: IntoIterator<Item = S>,
    S: Into<std::ffi::OsString>,
{
    let mut args: Vec<std::ffi::OsString> = args.into_iter().map(Into::into).collect();
    if args.first().is_some_and(|a| a == "--") {
        args.remove(0);
    }
    match args.as_slice() {
        [] => Ok(Launch::Default),
        [flag] if flag == "--new-window" => Ok(Launch::NewWindow(None)),
        [flag, path] if flag == "--new-window" && path != "--" => {
            Ok(Launch::NewWindow(Some(PathBuf::from(path))))
        }
        [path] if path != "--new-window" => Ok(Launch::Open(PathBuf::from(path))),
        _ => Err("用法: veil [--new-window] [路径]".into()),
    }
}

pub fn absolute_path(path: &Path) -> Result<PathBuf, String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| format!("无法确定当前目录: {e}"))?
            .join(path)
    };
    if absolute.as_os_str().is_empty() || absolute.to_string_lossy().contains('\0') {
        return Err("文档路径无效".into());
    }
    Ok(absolute)
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
        assert_eq!(parse(["--", "--new-window"]), Ok(Launch::NewWindow(None)));
        assert_eq!(
            parse(["--", "notes.md"]),
            Ok(Launch::Open(PathBuf::from("notes.md")))
        );
    }

    #[test]
    fn rejects_extra_or_incomplete_arguments() {
        assert!(parse(["a.md", "b.md"]).is_err());
        assert!(parse(["--new-window", "a.md", "b.md"]).is_err());
        assert!(parse(["--new-window", "--"]).is_err());
        assert!(parse(["--", "--", "a.md"]).is_err());
    }
}
