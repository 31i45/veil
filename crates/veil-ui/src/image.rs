//! 文档内图片加载抽象：实现由产品层（veil）注入（同 Clipboard/FileStore 注入模式）。
//!
//! 视图层只认「路径 → 位图」这一件事：路径解析（相对文档目录）与缓存/失败记忆在
//! shell；解码格式（v0 PNG）属产品层实现。加载失败返回 Err → 视图降级显示 alt 文本，
//! 不崩溃、不每帧重试。

use std::path::Path;

/// 已解码位图（straight alpha RGBA8，行主序，长度 = width × height × 4）。
pub struct ImageData {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// 按本地绝对/相对路径加载图片（相对路径已由 shell 按文档目录解析）。
pub trait ImageLoader {
    fn load(&mut self, path: &Path) -> Result<ImageData, String>;
}

/// 粘贴图片落盘抽象（切片 14）：实现由产品层注入（同 ImageLoader 模式）。
///
/// 约定：实现只接受 `doc` 的所在目录作为写入根，落盘到 `<stem>.assets/` 子目录；
/// 文件名由实现生成且必须 sanitize（防路径穿越 `../`），重名须避让不得覆盖既有资产。
/// 返回值是插入 Markdown 用的相对 URL（正斜杠，如 `notes.assets/image-1.png`）。
/// 草稿无路径时调用方不调用本方法；失败返回 Err → 粘贴不动文档并报错。
pub trait AssetStore {
    fn save_pasted_image(&mut self, doc: &Path, image: &ImageData) -> Result<String, String>;
}

/// 图片扩展名判定（拖拽分流，阶段 1）：按小写扩展名认 PNG/JPEG/WebP 等常见位图。
/// 只做「像不像图片」，不承诺可解码——真正能否解码由 `ImageLoader` 实现决定，
/// 解码失败时调用方提示并保持文档不变。
pub(crate) fn is_image_path(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "png" | "jpg" | "jpeg" | "jfif" | "bmp" | "gif" | "webp" | "tif" | "tiff" | "ico"
    )
}

#[cfg(test)]
mod tests {
    use super::is_image_path;
    use std::path::Path;

    #[test]
    fn image_extensions_are_case_insensitive() {
        for p in [
            "a.png", "a.PNG", "a.jpg", "a.JPEG", "a.bmp", "a.gif", "a.webp", "a.TiFf", "a.ico",
        ] {
            assert!(is_image_path(Path::new(p)), "{p} 应识别为图片");
        }
        for p in ["a.md", "a.txt", "a", "a.png.txt", "a.js"] {
            assert!(!is_image_path(Path::new(p)), "{p} 不应识别为图片");
        }
    }
}
