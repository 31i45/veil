//! 标题 slug（锚点 id）生成：纯文本 → 稳定、可去重的 ASCII 风格 slug。
//!
//! 规则（与 GitHub 风格 slugger 对齐，但只保留任务要求的字符集）：
//! - ASCII 字母转小写；中文/其他 Unicode 字母数字原样保留；
//! - 空白序列与 `-` 合并为单个 `-`；
//! - 去除 ASCII 标点（`!"#$%&'()*+,./:;<=>?@[\]^`{|}~`），非 ASCII 标点同样丢弃；
//! - 保留字母/数字/`_`；首尾 `-` 去除；全被过滤时返回空串。
//!
//! 「为什么」这样定：slug 是锚点标识而非展示文本，越保守越不会在不同平台
//! 产生歧义；空白与连字符统一为 `-` 既能保留词边界，又让 `Foo  Bar` 与
//! `Foo-Bar` 收敛到同一形态。

use std::collections::{HashMap, HashSet};

/// 单段文本 → slug。允许返回空串（纯符号标题），由调用方决定是否参与去重。
pub fn heading_slug(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut dash_pending = false;
    for c in text.chars() {
        if c.is_whitespace() || c == '-' {
            dash_pending = true;
            continue;
        }
        // 只保留字母/数字/`_`；ASCII 标点与 Unicode 标点/符号一并丢弃
        if !(c.is_alphanumeric() || c == '_') {
            continue;
        }
        // 首部连字符不落地（去掉前导 `-`）；中间连续空白/连字符合并为一个 `-`
        if dash_pending && !out.is_empty() {
            out.push('-');
        }
        dash_pending = false;
        if c.is_ascii() {
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// 文档级 slug 去重：重名依次为 `名称`、`名称-1`、`名称-2`……，并保证结果全局唯一
/// （若 `名称-1` 恰好是另一标题的自然 slug，则继续递增到第一个未占用的名字）。
/// 空 slug 同样参与去重（首个为空串，重复者得到 `-1`、`-2`），不会 panic。
pub fn heading_slugs<'a>(texts: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut used: HashSet<String> = HashSet::new();
    // 每个 base 已用到的最大后缀序号；只对发生碰撞的 base 建键，常见文档零额外分配。
    let mut next_suffix: HashMap<String, usize> = HashMap::new();
    let mut out = Vec::new();
    for text in texts {
        let base = heading_slug(text);
        let mut slug = base.clone();
        while used.contains(&slug) {
            let n = next_suffix.entry(base.clone()).or_insert(0);
            *n += 1;
            slug = format!("{base}-{n}");
        }
        used.insert(slug.clone());
        out.push(slug);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_lowercase_and_spaces() {
        assert_eq!(heading_slug("Hello World"), "hello-world");
        assert_eq!(heading_slug("Hello   World"), "hello-world");
        assert_eq!(heading_slug("Hello--World"), "hello-world");
        assert_eq!(heading_slug("  Hello\tWorld  "), "hello-world");
    }

    #[test]
    fn punctuation_is_dropped() {
        // 标点直接消失（不产生分隔），空白才产生 `-`
        assert_eq!(heading_slug("Hello, World!"), "hello-world");
        assert_eq!(heading_slug("a.b:c;d"), "abcd");
        assert_eq!(heading_slug("`code` [link](x)"), "code-linkx");
    }

    #[test]
    fn chinese_and_underscore_kept() {
        assert_eq!(heading_slug("中文 标题"), "中文-标题");
        assert_eq!(heading_slug("Hello_World"), "hello_world");
        assert_eq!(heading_slug("A1 中文B2"), "a1-中文b2");
    }

    #[test]
    fn leading_trailing_dashes_trimmed() {
        assert_eq!(heading_slug("-Hello-"), "hello");
        assert_eq!(heading_slug("---"), "");
        assert_eq!(heading_slug("- a - b -"), "a-b");
    }

    #[test]
    fn symbol_only_and_empty_yield_empty() {
        assert_eq!(heading_slug(""), "");
        assert_eq!(heading_slug("!!! ??? ..."), "");
        assert_eq!(heading_slug("。、！"), ""); // 非 ASCII 标点同样丢弃
    }

    #[test]
    fn duplicate_slugs_increment_suffix() {
        let got = heading_slugs(["A", "A", "A", "B"].into_iter());
        assert_eq!(got, ["a", "a-1", "a-2", "b"]);
    }

    #[test]
    fn empty_slugs_participate_without_panic() {
        let got = heading_slugs(["", "", "!!!", "x"].into_iter());
        assert_eq!(got, ["", "-1", "-2", "x"]);
    }

    #[test]
    fn collision_with_natural_suffix_keeps_uniqueness() {
        // 自然 slug `a-1` 先占位，第二个 `A` 必须跳到 `a-2`
        let got = heading_slugs(["A", "A-1", "A"].into_iter());
        assert_eq!(got, ["a", "a-1", "a-2"]);
        // 反向顺序同理：`a` 重复两次占掉 a-1 后，自然 `a-1` 需再让位
        let got = heading_slugs(["A", "A", "A-1"].into_iter());
        assert_eq!(got, ["a", "a-1", "a-1-1"]);
    }
}
