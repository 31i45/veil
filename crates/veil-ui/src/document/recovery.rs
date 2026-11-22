//! 崩溃恢复草稿（0b）：单槽恢复文件的编解码纯逻辑，零 IO、零 GUI。
//!
//! 设计要点：
//! - 恢复文件只承载「最后一个未保存状态」：路径（草稿为空）、BOM、完整内容。
//! - 内容含任意 UTF-8 字节，放文件末尾「读到 EOF」，头部行格式只含 ASCII 元数据。
//! - 路径用十进制长度前缀（不假设其不含任何特殊字符）；非 UTF-8 路径经 lossy 转换，
//!   无法表示时视为草稿（恢复内容不丢，仅丢失关联路径）。
//! - 解码对头部严格（魔数/字段缺一不可），对内容原样保留——宁可拒绝也不猜。

use std::path::{Path, PathBuf};

/// 恢复文件魔数（首行，含换行）。v2 增加内容长度与校验和，用于识别截断/损坏；
/// v1 仅作升级期兼容读取（旧文件由旧版原子写产生，无法自证长度）。
pub(crate) const MAGIC: &[u8] = b"VEIL-RECOVERY-2\n";
const LEGACY_MAGIC: &[u8] = b"VEIL-RECOVERY-1\n";

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

/// 编码恢复文件（v2）：
/// `MAGIC + bom 行 + path_len 行 + content_len 行 + checksum 行 + path 字节 + 换行 + 内容字节`。
pub(crate) fn encode_recovery(path: Option<&Path>, bom: bool, text: &str) -> Vec<u8> {
    let p = path
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut out = Vec::with_capacity(text.len() + p.len() + 96);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(if bom { b"bom=1\n" } else { b"bom=0\n" });
    out.extend_from_slice(format!("path_len={}\n", p.len()).as_bytes());
    out.extend_from_slice(format!("content_len={}\n", text.len()).as_bytes());
    out.extend_from_slice(format!("checksum={:016x}\n", fnv64(text.as_bytes())).as_bytes());
    out.extend_from_slice(p.as_bytes());
    out.push(b'\n');
    out.extend_from_slice(text.as_bytes());
    out
}

/// 解码恢复文件。头部损坏、长度不符或校验失败 → Err（调用方忽略该文件，正常打开文档）。
pub(crate) fn decode_recovery(data: &[u8]) -> Result<Recovered, String> {
    if let Some(rest) = data.strip_prefix(MAGIC) {
        decode_v2(rest)
    } else if let Some(rest) = data.strip_prefix(LEGACY_MAGIC) {
        decode_legacy(rest)
    } else {
        Err("非恢复文件（魔数不符）".into())
    }
}

/// v2：内容长度与 FNV-1a 校验和共同证明完整；任一不符都拒绝，绝不把截断当短文本。
fn decode_v2(mut rest: &[u8]) -> Result<Recovered, String> {
    let (bom_line, r) = parse_line(rest)?;
    let bom = parse_bom(bom_line)?;
    let (path_line, r) = parse_line(r)?;
    let path_len = parse_field(path_line, b"path_len=", "path_len")?;
    let (len_line, r) = parse_line(r)?;
    let content_len = parse_field(len_line, b"content_len=", "content_len")?;
    let (sum_line, r) = parse_line(r)?;
    let sum_hex = sum_line
        .strip_prefix(b"checksum=")
        .ok_or_else(|| "checksum 字段非法".to_string())?;
    let checksum = std::str::from_utf8(sum_hex)
        .ok()
        .and_then(|s| u64::from_str_radix(s, 16).ok())
        .ok_or_else(|| "checksum 字段非法".to_string())?;
    rest = r;
    if rest.len() < path_len + 1 {
        return Err("路径字节不完整".into());
    }
    let (path_bytes, rest) = rest.split_at(path_len);
    if rest.first() != Some(&b'\n') {
        return Err("路径后缺换行".into());
    }
    let content = rest.get(1..).unwrap_or_default();
    if content.len() != content_len {
        return Err(format!(
            "内容长度不符（声明 {content_len}，实际 {}，可能被截断）",
            content.len()
        ));
    }
    if fnv64(content) != checksum {
        return Err("内容校验失败（可能损坏）".into());
    }
    let path_str = std::str::from_utf8(path_bytes).map_err(|_| "路径非 UTF-8".to_string())?;
    let text = String::from_utf8(content.to_vec()).map_err(|_| "内容非 UTF-8".to_string())?;
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

/// v1 兼容读取：旧版恢复文件没有长度/校验字段，只能做头部严格校验。
fn decode_legacy(rest: &[u8]) -> Result<Recovered, String> {
    let (bom_line, rest) = parse_line(rest)?;
    let bom = parse_bom(bom_line)?;
    let (len_line, rest) = parse_line(rest)?;
    let path_len = parse_field(len_line, b"path_len=", "path_len")?;
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

fn parse_bom(line: &[u8]) -> Result<bool, String> {
    match line {
        b"bom=0" => Ok(false),
        b"bom=1" => Ok(true),
        _ => Err("bom 字段非法".into()),
    }
}

fn parse_field(line: &[u8], prefix: &[u8], name: &str) -> Result<usize, String> {
    line.strip_prefix(prefix)
        .and_then(|d| std::str::from_utf8(d).ok())
        .and_then(|d| d.parse::<usize>().ok())
        .ok_or_else(|| format!("{name} 字段非法"))
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
        let data = encode_recovery(
            Some(Path::new("D:\\docs\\中文 a.md")),
            true,
            "中\n文\r\n内容",
        );
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
    fn rejects_content_truncated_at_char_boundary() {
        let data = encode_recovery(Some(Path::new("a.md")), false, "未保存正文");
        // 头部完整，只截掉内容最后一个字符（3 字节，仍落在字符边界）
        let cut = data.len() - 3;
        assert!(decode_recovery(&data[..cut]).is_err(), "截断内容必须拒绝");
        // 截到只剩一个完整字符：也不得当作合法短文本
        let prefix =
            "VEIL-RECOVERY-2\nbom=0\npath_len=0\ncontent_len=9\nchecksum=0000000000000000\n\n未";
        assert!(decode_recovery(prefix.as_bytes()).is_err());
    }

    #[test]
    fn rejects_content_checksum_mismatch() {
        let mut data = encode_recovery(None, false, "abcdef");
        let last = data.len() - 1;
        data[last] ^= 0x01; // 等长篡改：长度对但校验不符
        assert!(decode_recovery(&data).is_err(), "校验失败必须拒绝");
    }

    #[test]
    fn legacy_v1_files_still_open() {
        let legacy = b"VEIL-RECOVERY-1\nbom=0\npath_len=0\n\n\xe6\x97\xa7\xe7\xa8\xbf";
        let r = decode_recovery(legacy).expect("旧版恢复文件升级期仍可读取");
        assert_eq!(r.text, "旧稿");
        assert_eq!(r.path, None);
    }

    #[test]
    fn fnv64_is_stable_and_sensitive() {
        assert_eq!(fnv64(b"abc"), fnv64(b"abc"));
        assert_ne!(fnv64(b"abc"), fnv64(b"abd"));
        assert_ne!(fnv64(b""), fnv64(b"\0"));
    }
}
