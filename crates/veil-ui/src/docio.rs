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
        .map_err(|_| "文件不是有效的 UTF-8 编码".to_string())
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
}
