//! 崩溃恢复草稿（0b）：单槽恢复文件的编解码纯逻辑，零 IO、零 GUI。
//!
//! 设计要点：
//! - 恢复文件只承载「最后一个未保存状态」：路径（草稿为空）、BOM、完整内容。
//! - 内容含任意 UTF-8 字节，放文件末尾「读到 EOF」，头部行格式只含 ASCII 元数据。
//! - 路径用十进制长度前缀（不假设其不含任何特殊字符）；非 UTF-8 路径经 lossy 转换，
//!   无法表示时视为草稿（恢复内容不丢，仅丢失关联路径）。
//! - 解码对头部严格（魔数/字段缺一不可），对内容原样保留——宁可拒绝也不猜。

use std::path::{Path, PathBuf};

/// 恢复文件魔数（首行，含换行）。版本升级时改魔数，旧文件解码拒绝而非误读。
pub(crate) const MAGIC: &[u8] = b"VEIL-RECOVERY-1\n";

/// 恢复出的文档快照。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Recovered {
    /// 关联文件路径；None = 无路径草稿。
    pub path: Option<PathBuf>,
    /// 保存时是否还原 BOM。
    pub bom: bool,
    /// 文档内容（不含 BOM）。
    pub text: String,
}

/// 编码恢复文件：`MAGIC + bom 行 + path_len 行 + path 字节 + 换行 + 内容字节`。
pub(crate) fn encode_recovery(path: Option<&Path>, bom: bool, text: &str) -> Vec<u8> {
    let p = path
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut out = Vec::with_capacity(text.len() + p.len() + 64);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(if bom { b"bom=1\n" } else { b"bom=0\n" });
    out.extend_from_slice(format!("path_len={}\n", p.len()).as_bytes());
    out.extend_from_slice(p.as_bytes());
    out.push(b'\n');
    out.extend_from_slice(text.as_bytes());
    out
}

/// 解码恢复文件。头部损坏/字段非法 → Err（调用方忽略该文件，正常打开文档）。
pub(crate) fn decode_recovery(data: &[u8]) -> Result<Recovered, String> {
    let rest = data
        .strip_prefix(MAGIC)
        .ok_or_else(|| "非恢复文件（魔数不符）".to_string())?;
    let (bom, rest) = parse_line(rest)?;
    let bom = match bom {
        b"bom=0" => false,
        b"bom=1" => true,
        _ => return Err("bom 字段非法".into()),
    };
    let (len_line, rest) = parse_line(rest)?;
    let path_len = len_line
        .strip_prefix(b"path_len=")
        .and_then(|d| std::str::from_utf8(d).ok())
        .and_then(|d| d.parse::<usize>().ok())
        .ok_or_else(|| "path_len 字段非法".to_string())?;
    if rest.len() < path_len + 1 {
        return Err("路径字节不完整".into());
    }
    let (path_bytes, rest) = rest.split_at(path_len);
    if rest.first() != Some(&b'\n') {
        return Err("路径后缺换行".into());
    }
    let path_str = std::str::from_utf8(path_bytes).map_err(|_| "路径非 UTF-8".to_string())?;
    let text = String::from_utf8(rest[1..].to_vec()).map_err(|_| "内容非 UTF-8".to_string())?;
    Ok(Recovered {
        path: if path_len == 0 {
            None
        } else {
            Some(PathBuf::from(path_str))
        },
        bom,
        text,
    })
}

/// 取一行（到 `\n` 为止，不含），返回 (行, 剩余)。
fn parse_line(data: &[u8]) -> Result<(&[u8], &[u8]), String> {
    let i = data
        .iter()
        .position(|&b| b == b'\n')
        .ok_or_else(|| "头部行不完整".to_string())?;
    Ok((&data[..i], &data[i + 1..]))
}

/// FNV-1a 64：恢复写入去重只需稳定分散（同布局指纹强度即可）。
pub(crate) fn fnv64(data: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for &b in data {
        h = (h ^ b as u64).wrapping_mul(0x0100_0000_01b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_with_path_and_bom() {
        let data = encode_recovery(Some(Path::new("D:\\docs\\中文 a.md")), true, "中\n文\r\n内容");
        let r = decode_recovery(&data).expect("自编码应可解码");
        assert_eq!(r.path, Some(PathBuf::from("D:\\docs\\中文 a.md")));
        assert!(r.bom);
        assert_eq!(r.text, "中\n文\r\n内容");
    }

    #[test]
    fn roundtrip_draft_without_path() {
        let data = encode_recovery(None, false, "");
        let r = decode_recovery(&data).unwrap();
        assert_eq!(r.path, None);
        assert!(!r.bom);
        assert_eq!(r.text, "");
    }

    #[test]
    fn content_may_contain_header_like_lines() {
        // 内容里出现与头部同形的行不得干扰解码（内容按 EOF 读取）
        let text = "bom=1\npath_len=3\nabc\nVEIL-RECOVERY-1\n";
        let r = decode_recovery(&encode_recovery(None, false, text)).unwrap();
        assert_eq!(r.text, text);
    }

    #[test]
    fn rejects_corrupt_headers() {
        assert!(decode_recovery(b"").is_err());
        assert!(decode_recovery(b"VEIL-RECOVERY-0\nbom=0\npath_len=0\n\n").is_err());
        assert!(decode_recovery(b"VEIL-RECOVERY-1\n").is_err());
        assert!(decode_recovery(b"VEIL-RECOVERY-1\nbom=2\npath_len=0\n\n").is_err());
        assert!(decode_recovery(b"VEIL-RECOVERY-1\nbom=0\npath_len=x\n\n").is_err());
        // 声明长度超过实际
        assert!(decode_recovery(b"VEIL-RECOVERY-1\nbom=0\npath_len=9\nab\n").is_err());
        // 路径后无换行
        assert!(decode_recovery(b"VEIL-RECOVERY-1\nbom=0\npath_len=2\nab").is_err());
        // 内容非 UTF-8
        let mut bad = b"VEIL-RECOVERY-1\nbom=0\npath_len=0\n\n".to_vec();
        bad.extend_from_slice(&[0xff, 0xfe]);
        assert!(decode_recovery(&bad).is_err());
    }

    #[test]
    fn fnv64_is_stable_and_sensitive() {
        assert_eq!(fnv64(b"abc"), fnv64(b"abc"));
        assert_ne!(fnv64(b"abc"), fnv64(b"abd"));
        assert_ne!(fnv64(b""), fnv64(b"\0"));
    }
}
