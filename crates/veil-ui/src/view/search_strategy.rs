//! 查找策略层（P3-3）：匹配扫描、浮栏几何、命中判定与数据模型。
//!
//! 单文件编辑器语义：全文一次性扫描，不做跨文件搜索。本模块是纯计算层——只回答
//! 「哪里有命中 / 浮栏画在哪 / 点到谁」，不写 Frame、不产像素：
//! 显示截断与浮栏绘制在 `search_view`，交互在 interaction，帧合成顺序在 paint。
//! 打开期间文档只读（文本输入全部进入查询串），故匹配缓存仅在查询变化/打开时重建。
//!
//! 不负责：查询/替换串按宽度截断与显示宽度估算（`search_view`）、按钮底色/字形
//! 绘制、按键路由与文档真值。

use memchr::memmem::find_iter;
use regex::{Regex, RegexBuilder};
use std::borrow::Cow;
use veil_core::Range;

use crate::geometry::Rect;
use crate::theme::surface;
use crate::tokens::space;

/// 浮栏基准宽（乘 DPI scale）。
pub const BAR_W: f32 = 400.0;
/// 浮栏基准高。
pub const BAR_H: f32 = 36.0;
/// 栏内按钮基准宽（上一个/下一个/关闭/大小写）。
pub const BTN_W: f32 = 28.0;
/// 栏内按钮基准高。
pub const BTN_H: f32 = 28.0;
/// 相邻按钮间距（基准值）：4pt 网格。
pub const BTN_GAP: f32 = space::XS;
/// 按钮分组间距（基准值）：大小写组与翻页组之间。
pub const GROUP_GAP: f32 = space::S;
/// 栏内左右内边距基准值：与全部浮层同一 surface 令牌。
pub const PAD: f32 = surface::PADDING;
/// 浮栏与内容视口顶边的间距基准值：与全部浮层同一 surface 令牌。
pub const GAP: f32 = surface::CONTROL_GAP;
/// 计数区基准宽（右对齐「i/n」，并入输入区右侧）。
pub const COUNT_W: f32 = 56.0;
/// 替换行按钮基准宽（「替换」/「全部」）。
pub const REPLACE_BTN_W: f32 = 56.0;
/// 替换行按钮基准高。
pub const REPLACE_BTN_H: f32 = 28.0;
/// 查询行与替换行的分组间距（基准值）：取代行间分隔线。
pub const ROW_GAP: f32 = space::S;

/// 栏内命中目标。`Bar` = 栏内非按钮区域（吞掉点击，不落文档光标）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Hit {
    Prev,
    Next,
    Close,
    /// 查询文本框（点击后焦点回到查询）。
    QueryText,
    /// 替换文本框（点击后焦点切到替换）。
    ReplaceText,
    /// 单次替换当前命中并跳到下一处。
    ReplaceOne,
    /// 全部替换（单事务）。
    ReplaceAll,
    /// 「区分大小写」开关（点击即切换并重扫）。
    ToggleCase,
    /// 正则搜索开关（点击即切换并重扫）。
    ToggleRegex,
    Bar,
}

impl Hit {
    /// 过渡动画槽位（S6）：顺序与绘制侧按钮序列一致；非按钮目标不参与动效。
    pub(crate) fn anim_index(self) -> Option<usize> {
        Some(match self {
            Hit::Prev => 0,
            Hit::Next => 1,
            Hit::Close => 2,
            Hit::ToggleCase => 3,
            Hit::ToggleRegex => 4,
            Hit::ReplaceOne => 5,
            Hit::ReplaceAll => 6,
            Hit::QueryText | Hit::ReplaceText | Hit::Bar => return None,
        })
    }
}

/// 查找栏内当前输入焦点：查询串或替换串。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum Field {
    #[default]
    Query,
    Replace,
}

/// 浮栏几何：查询文本区、计数右缘与三个按钮。
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct BarLayout {
    pub(crate) bar: Rect,
    pub(crate) text: Rect,
    /// 「区分大小写」小按钮（查询框与计数之间）。
    pub(crate) case: Rect,
    /// 「正则模式」小按钮（大小写左侧）。
    pub(crate) regex: Rect,
    /// 计数文本右缘（右对齐基准）。
    pub(crate) count_right: f32,
    pub(crate) prev: Rect,
    pub(crate) next: Rect,
    pub(crate) close: Rect,
}

/// 浮栏布局：贴内容视口右上角（`right` 已扣除滚动条/大纲面板）。
/// 按钮自右向左排列、以间距分组；无竖分隔线，计数并入输入区右缘。
pub(crate) fn layout(right: f32, top: f32, scale: f32) -> BarLayout {
    let w = BAR_W * scale;
    let h = BAR_H * scale;
    let bar = Rect {
        x: right - w,
        y: top + GAP * scale,
        w,
        h,
    };
    let bw = BTN_W * scale;
    let bh = BTN_H * scale;
    let by = bar.y + (h - bh) / 2.0;
    let pad = PAD * scale;
    // 按钮自右向左：关闭 → 下一个 → 上一个 →（分组间距）→ 大小写 → 正则
    let close = Rect {
        x: bar.x + w - pad - bw,
        y: by,
        w: bw,
        h: bh,
    };
    let next = Rect {
        x: close.x - bw - BTN_GAP * scale,
        ..close
    };
    let prev = Rect {
        x: next.x - bw - BTN_GAP * scale,
        ..close
    };
    let case = Rect {
        x: prev.x - bw - GROUP_GAP * scale,
        ..close
    };
    let regex = Rect {
        x: case.x - bw - BTN_GAP * scale,
        ..close
    };
    let text_x = bar.x + pad;
    let text_right = regex.x - pad;
    let text = Rect {
        x: text_x,
        y: by,
        w: (text_right - text_x).max(0.0),
        h: bh,
    };
    BarLayout {
        bar,
        text,
        case,
        regex,
        // 计数右对齐于输入区右缘（不再独占独立槽位）。
        count_right: text_right,
        prev,
        next,
        close,
    }
}

/// 替换行几何（仅替换模式存在）：整行矩形、替换文本框与两个按钮。
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct ReplaceLayout {
    pub(crate) row: Rect,
    pub(crate) text: Rect,
    pub(crate) one: Rect,
    pub(crate) all: Rect,
}

/// 替换行：与查询行间隔 `ROW_GAP`（分组用间距，不画横线），按钮贴右缘。
pub(crate) fn replace_layout(l: &BarLayout, scale: f32) -> ReplaceLayout {
    let pad = PAD * scale;
    let bw = REPLACE_BTN_W * scale;
    let bh = REPLACE_BTN_H * scale;
    let row = Rect {
        x: l.bar.x,
        y: l.bar.y + l.bar.h + ROW_GAP * scale,
        w: l.bar.w,
        h: l.bar.h,
    };
    let by = row.y + (row.h - bh) / 2.0;
    let all = Rect {
        x: row.x + row.w - pad - bw,
        y: by,
        w: bw,
        h: bh,
    };
    let one = Rect {
        x: all.x - bw - BTN_GAP * scale,
        ..all
    };
    let text_x = row.x + pad;
    let text = Rect {
        x: text_x,
        y: by,
        w: (one.x - pad - text_x).max(0.0),
        h: bh,
    };
    ReplaceLayout {
        row,
        text,
        one,
        all,
    }
}

/// 栏内命中：`None` = 栏外（调用方按普通点击处理）。
/// 替换模式下整张卡片（含两行之间的分组间距）都算栏内。
pub(crate) fn hit(l: &BarLayout, scale: f32, replace_open: bool, x: f32, y: f32) -> Option<Hit> {
    if replace_open {
        let r = replace_layout(l, scale);
        let card = Rect {
            x: l.bar.x,
            y: l.bar.y,
            w: l.bar.w,
            h: r.row.y + r.row.h - l.bar.y,
        };
        if !card.contains(x, y) {
            return None;
        }
        if r.all.contains(x, y) {
            return Some(Hit::ReplaceAll);
        }
        if r.one.contains(x, y) {
            return Some(Hit::ReplaceOne);
        }
        if r.text.contains(x, y) {
            return Some(Hit::ReplaceText);
        }
        if l.close.contains(x, y) {
            return Some(Hit::Close);
        }
        if l.next.contains(x, y) {
            return Some(Hit::Next);
        }
        if l.prev.contains(x, y) {
            return Some(Hit::Prev);
        }
        if l.case.contains(x, y) {
            return Some(Hit::ToggleCase);
        }
        if l.regex.contains(x, y) {
            return Some(Hit::ToggleRegex);
        }
        if l.text.contains(x, y) {
            return Some(Hit::QueryText);
        }
        return Some(Hit::Bar);
    }
    if !l.bar.contains(x, y) {
        return None;
    }
    if l.close.contains(x, y) {
        Some(Hit::Close)
    } else if l.next.contains(x, y) {
        Some(Hit::Next)
    } else if l.prev.contains(x, y) {
        Some(Hit::Prev)
    } else if l.case.contains(x, y) {
        Some(Hit::ToggleCase)
    } else if l.regex.contains(x, y) {
        Some(Hit::ToggleRegex)
    } else if l.text.contains(x, y) {
        Some(Hit::QueryText)
    } else {
        Some(Hit::Bar)
    }
}

/// 逐字符小写化，且要求每字符字节长度不变：满足时小写串与原串偏移一一对应，
/// 可直接复用 `match_indices` 的字节位置（大小写不敏感）。
/// 任一字符长度变化（如 KELVIN SIGN `K` 3 字节 → `k` 1 字节）即放弃折叠，
/// 由调用方回退大小写敏感匹配，避免偏移错位。
fn fold_case(s: &str) -> Option<String> {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        let mut it = c.to_lowercase();
        let lower = it.next()?;
        if it.next().is_some() || lower.len_utf8() != c.len_utf8() {
            return None;
        }
        out.push(lower);
    }
    Some(out)
}

/// 全文扫描全部非重叠命中（`match_indices` 语义：`aaa` 中找 `aa` 只命中一处）。
/// memchr `memmem::find_iter` 与 std `match_indices` 返回相同的非重叠字节偏移，
/// 实测快 3–10×（5000 块 2 MB 文档精确匹配）。
///
/// `case_sensitive = false`（默认）走小写折叠路径，与旧实现逐字节一致；折叠会使
/// 字节偏移失效时回退大小写敏感——两条路径都保证返回原文字节偏移。
/// `case_sensitive = true` 直接在原文上扫描，不做任何折叠。
pub(crate) fn find_all(hay: &str, needle: &str, case_sensitive: bool) -> Vec<Range> {
    if hay.is_empty() || needle.is_empty() {
        return Vec::new();
    }
    // 大小写折叠路径需要拥有的 bytes（临时 String）；大小写敏感/折叠失败路径
    // 直接用原文 bytes 的借用（零分配）。折叠失败时回退原文大小写敏感扫描，
    // 与旧实现逐字节等价。
    let (hay_bytes, needle_bytes): (Cow<[u8]>, Cow<[u8]>) = if case_sensitive {
        (Cow::Borrowed(hay.as_bytes()), Cow::Borrowed(needle.as_bytes()))
    } else if let (Some(h), Some(n)) = (fold_case(hay), fold_case(needle)) {
        (Cow::Owned(h.into_bytes()), Cow::Owned(n.into_bytes()))
    } else {
        (Cow::Borrowed(hay.as_bytes()), Cow::Borrowed(needle.as_bytes()))
    };
    find_iter(&hay_bytes, &needle_bytes)
        .map(|i| Range::new(i, i + needle_bytes.len()))
        .collect()
}

/// 构造正则：`case_sensitive = false`（默认）时大小写不敏感；`multi_line` 恒开，
/// 使 `^`/`$` 按行锚定（Markdown 场景「所有标题行 `^#{1,6} `」依赖它）。
/// `.` 默认不跨行，符合逐行查找直觉。
fn build_regex(pattern: &str, case_sensitive: bool) -> Result<Regex, regex::Error> {
    RegexBuilder::new(pattern)
        .case_insensitive(!case_sensitive)
        .multi_line(true)
        .build()
}

/// 正则全文扫描：返回全部非重叠、非零宽命中的字节区间。
///
/// 零宽命中（如 `^`、`a*`）被过滤：它们无法高亮、替换时会在同一位置反复插入，
/// 对编辑器查找没有实际价值。Rust regex 为线性时间匹配，无回溯爆炸风险，
/// 恶意模式最坏只是慢，不会挂死。
pub(crate) fn find_all_regex(
    hay: &str,
    pattern: &str,
    case_sensitive: bool,
) -> Result<Vec<Range>, regex::Error> {
    if hay.is_empty() || pattern.is_empty() {
        return Ok(Vec::new());
    }
    let re = build_regex(pattern, case_sensitive)?;
    Ok(re
        .find_iter(hay)
        .filter(|m| m.start() != m.end())
        .map(|m| Range::new(m.start(), m.end()))
        .collect())
}

/// 全部替换的逐命中替换文本：支持 `$1`/`${name}` 捕获组展开。
/// 返回顺序与 `find_all_regex` 一致（起点升序、互不重叠、跳过零宽）。
pub(crate) fn expand_replacements(
    hay: &str,
    pattern: &str,
    case_sensitive: bool,
    template: &str,
) -> Result<Vec<(Range, String)>, regex::Error> {
    if hay.is_empty() || pattern.is_empty() {
        return Ok(Vec::new());
    }
    let re = build_regex(pattern, case_sensitive)?;
    let mut out = Vec::new();
    for caps in re.captures_iter(hay) {
        let Some(m) = caps.get(0) else { continue };
        if m.start() == m.end() {
            continue;
        }
        let mut dst = String::new();
        caps.expand(template, &mut dst);
        out.push((Range::new(m.start(), m.end()), dst));
    }
    Ok(out)
}

/// 单次替换的展开文本：定位起点恰为 `target.start` 的那一处捕获并按模板展开。
/// 扫描到起点超过目标即提前结束（命中按起点升序）。
pub(crate) fn expand_one(
    hay: &str,
    pattern: &str,
    case_sensitive: bool,
    target: Range,
    template: &str,
) -> Option<String> {
    let re = build_regex(pattern, case_sensitive).ok()?;
    for caps in re.captures_iter(hay) {
        let m = caps.get(0)?;
        if m.start() == target.start && m.end() == target.end {
            let mut dst = String::new();
            caps.expand(template, &mut dst);
            return Some(dst);
        }
        if m.start() > target.start {
            break;
        }
    }
    None
}

/// 查找栏状态：查询串、命中区间缓存、当前命中下标、按钮悬停与替换行。
#[derive(Default)]
pub(crate) struct Search {
    pub(crate) open: bool,
    /// 替换模式（Ctrl+H 打开）：浮栏多出一行替换输入与两个替换按钮。
    pub(crate) replace_open: bool,
    pub(crate) query: String,
    /// 替换串（仅替换模式可见；关闭/换文档时清空）。
    pub(crate) replace: String,
    /// 输入焦点：决定按键与 IME 落到查询串还是替换串。
    pub(crate) focus: Field,
    /// 区分大小写开关（会话内保持：`clear()` 不重置，默认 false = 旧行为）。
    pub(crate) case_sensitive: bool,
    /// 正则模式开关（会话内保持：`clear()` 不重置，默认 false = 逐字面量匹配）。
    /// 开启后查询串按正则解析，替换串支持 `$1`/`${name}` 捕获组。
    pub(crate) regex: bool,
    /// 当前查询串作为正则是否非法（仅在正则模式下可能为 true）。
    /// 非法时无命中、计数区显示「正则错误」，并阻止替换按钮启用。
    pub(crate) regex_error: bool,
    /// 全文命中区间（按起点升序，非重叠），绘制高亮与跳转同源。
    pub(crate) matches: Vec<Range>,
    /// 当前命中下标（`matches` 内）。
    pub(crate) current: usize,
    pub(crate) hover: Option<Hit>,
    /// 鼠标按下中的按钮（active 态）；松开或关闭时清空。
    pub(crate) pressed: Option<Hit>,
    /// 悬停过渡（S6）：按钮 hover 底色淡入/淡出（active 按下色不参与）。
    pub(crate) fade: crate::anim::Fade,
}

impl Search {
    /// 清空查询、替换串与命中（打开/关闭/换文档时）。`replace_open` 由调用方决定。
    pub(crate) fn clear(&mut self) {
        self.query.clear();
        self.replace.clear();
        self.focus = Field::Query;
        self.matches.clear();
        self.current = 0;
        self.hover = None;
        self.pressed = None;
        self.regex_error = false;
    }

    /// 无命中空态：查询非空但没有任何命中。
    pub(crate) fn no_matches(&self) -> bool {
        !self.query.is_empty() && self.matches.is_empty()
    }

    /// 查询变化后重算匹配并回到首个命中。
    /// 正则模式下非法模式不产生命中，只置 `regex_error` 供 UI 表达。
    pub(crate) fn rescan(&mut self, hay: &str) {
        self.regex_error = false;
        self.matches = if self.regex {
            match find_all_regex(hay, &self.query, self.case_sensitive) {
                Ok(m) => m,
                Err(_) => {
                    self.regex_error = true;
                    Vec::new()
                }
            }
        } else {
            find_all(hay, &self.query, self.case_sensitive)
        };
        self.current = 0;
    }

    /// 命中集合重建后定位文档顺序的下一处（起点不早于 `from`，末尾回绕）。
    /// 供「替换当前并前进」使用：`from` = 旧命中起点 + 替换串字节长度。
    pub(crate) fn select_after(&mut self, from: usize) -> Option<Range> {
        if self.matches.is_empty() {
            self.current = 0;
            return None;
        }
        self.current = self
            .matches
            .iter()
            .position(|m| m.start >= from)
            .unwrap_or(0);
        self.current_match()
    }

    pub(crate) fn current_match(&self) -> Option<Range> {
        self.matches.get(self.current).copied()
    }

    /// 步进到下一个/上一个命中（循环）。返回新当前命中；无命中 None。
    pub(crate) fn step(&mut self, forward: bool) -> Option<Range> {
        let n = self.matches.len();
        if n == 0 {
            return None;
        }
        self.current = if forward {
            (self.current + 1) % n
        } else {
            (self.current + n - 1) % n
        };
        self.current_match()
    }

    /// 计数标签：空查询空串；非法正则「正则错误」；无命中 `0/0`；否则 `i/n`（1 起）。
    pub(crate) fn label(&self) -> String {
        if self.query.is_empty() {
            String::new()
        } else if self.regex_error {
            crate::i18n::t(crate::i18n::Key::SearchRegexError).to_string()
        } else if self.matches.is_empty() {
            crate::i18n::tf(crate::i18n::Key::SearchCount, &["0", "0"])
        } else {
            crate::i18n::tf(
                crate::i18n::Key::SearchCount,
                &[
                    &(self.current + 1).to_string(),
                    &self.matches.len().to_string(),
                ],
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_all_is_case_insensitive_with_byte_offsets() {
        // 「Foo foo」F=[0,1) o=[1,2) o=[2,3) 空格 [3,4) f=[4,5)...
        assert_eq!(
            find_all("Foo foo", "foo", false),
            vec![Range::new(0, 3), Range::new(4, 7)]
        );
        // 换行与多字节混排：「中A中a」中=3 字节
        // 「中A」= 中(3)+A(1)、「中a」= 中(3)+a(1)：大小写不敏感各命中一处
        assert_eq!(
            find_all("中A中a", "中a", false),
            vec![Range::new(0, 4), Range::new(4, 8)]
        );
    }

    #[test]
    fn find_all_case_sensitive_restricts_hit_set() {
        // 关：Fold 后三处；开：只认小写 foo（一处）
        assert_eq!(
            find_all("Foo foo FOO", "foo", false),
            vec![Range::new(0, 3), Range::new(4, 7), Range::new(8, 11)]
        );
        assert_eq!(find_all("Foo foo FOO", "foo", true), vec![Range::new(4, 7)]);
        // CJK 与大写混排：开时「中A」不再命中「中a」，仅小写一处
        assert_eq!(
            find_all("中A中a", "中a", false),
            vec![Range::new(0, 4), Range::new(4, 8)]
        );
        assert_eq!(find_all("中A中a", "中a", true), vec![Range::new(4, 8)]);
    }

    #[test]
    fn case_sensitive_defaults_off_and_clear_keeps_it() {
        let mut s = Search::default();
        assert!(!s.case_sensitive, "默认关闭 = 改动前的逐字节行为");
        s.case_sensitive = true;
        s.matches = vec![Range::new(0, 1)];
        s.clear();
        assert!(s.case_sensitive, "会话内保持：clear 不重置开关");
        assert!(s.matches.is_empty());
    }

    #[test]
    fn find_all_reports_cjk_and_utf8_offsets() {
        assert_eq!(
            find_all("中文中文", "中文", false),
            vec![Range::new(0, 6), Range::new(6, 12)]
        );
    }

    #[test]
    fn regex_find_all_matches_line_anchors() {
        // 行锚定（multi_line 恒开）：找出全部 ATX 标题行首的 `#`/`##`
        let hay = "# 标题\n正文\n## 二级\n";
        assert_eq!(
            find_all_regex(hay, r"^#{1,2} ", false).unwrap(),
            vec![Range::new(0, 2), Range::new(16, 19)]
        );
    }

    #[test]
    fn regex_respects_case_toggle_and_reports_invalid_pattern() {
        assert_eq!(
            find_all_regex("Foo foo", "foo", false).unwrap(),
            vec![Range::new(0, 3), Range::new(4, 7)]
        );
        assert_eq!(
            find_all_regex("Foo foo", "foo", true).unwrap(),
            vec![Range::new(4, 7)]
        );
        assert!(find_all_regex("abc", "(", false).is_err(), "非法模式须报错");
    }

    #[test]
    fn regex_zero_width_matches_are_skipped() {
        // `^` 只产生零宽命中 → 全部过滤；`b*` 只保留非零宽的 "b"
        assert!(find_all_regex("abc", "^", false).unwrap().is_empty());
        assert_eq!(
            find_all_regex("abc", "b*", false).unwrap(),
            vec![Range::new(1, 2)]
        );
    }

    #[test]
    fn regex_replacement_expands_capture_groups() {
        let hay = "**粗** 与 **斜**";
        let edits = expand_replacements(hay, r"\*\*([^*]+)\*\*", false, "$1").unwrap();
        assert_eq!(
            edits,
            vec![
                (Range::new(0, 7), "粗".to_string()),
                (Range::new(12, 19), "斜".to_string()),
            ]
        );
        // 单次替换复用同一展开口径
        assert_eq!(
            expand_one(hay, r"\*\*([^*]+)\*\*", false, Range::new(12, 19), "<$1>"),
            Some("<斜>".to_string())
        );
        assert_eq!(
            expand_one(hay, r"\*\*([^*]+)\*\*", false, Range::new(99, 100), "$1"),
            None
        );
    }

    #[test]
    fn regex_error_sets_label_and_toggle_survives_clear() {
        let mut s = Search {
            query: "[".into(),
            regex: true,
            ..Default::default()
        };
        s.rescan("abc");
        assert!(s.regex_error);
        assert!(s.matches.is_empty());
        assert_eq!(
            s.label(),
            crate::i18n::t(crate::i18n::Key::SearchRegexError)
        );
        // 修好模式后错误态立刻消失
        s.query = "^a".into();
        s.rescan("abc");
        assert!(!s.regex_error);
        assert_eq!(s.matches, vec![Range::new(0, 1)]);
        // 开关与会话同寿命：clear 不重置
        s.clear();
        assert!(s.regex);
        assert!(!s.regex_error);
    }

    #[test]
    fn empty_needle_or_hay_yields_no_matches() {
        assert!(find_all("abc", "", false).is_empty());
        assert!(find_all("", "a", false).is_empty());
    }

    #[test]
    fn overlapping_occurrences_are_not_reported() {
        // match_indices 非重叠语义（与正则全局匹配一致）
        assert_eq!(find_all("aaa", "aa", false), vec![Range::new(0, 2)]);
    }

    #[test]
    fn case_fold_offset_shift_falls_back_to_case_sensitive() {
        // KELVIN SIGN（U+212A）小写为 k 但字节长度 3 → 1：折叠偏移不可复用
        assert!(fold_case("\u{212A}").is_none());
        assert!(
            find_all("\u{212A}", "k", false).is_empty(),
            "回退大小写敏感，不命中"
        );
        // 混合串仍按原串大小写敏感扫描：仅 ASCII 「k」命中（字节 3）
        assert_eq!(find_all("\u{212A}k", "k", false), vec![Range::new(3, 4)]);
    }

    #[test]
    fn replace_then_rescan_selects_next_hit() {
        // 模拟 Enter 替换当前：原文「ab ab」，把首处 ab 换成 a（1 字节）后重扫。
        let hay = "ab ab";
        let mut s = Search {
            query: "ab".into(),
            case_sensitive: true,
            ..Search::default()
        };
        s.rescan(hay);
        let m = s.current_match().expect("初始命中");
        let from = m.start + 1; // 替换串「a」的字节长度
        let after = format!("{}a{}", &hay[..m.start], &hay[m.end..]);
        s.rescan(&after);
        assert_eq!(
            s.select_after(from),
            Some(Range::new(2, 4)),
            "前进到文档顺序的下一处（光标落点）"
        );
        assert_eq!(s.label(), "1/1", "替换后 n/m 同步");
    }

    #[test]
    fn select_after_wraps_and_handles_empty() {
        let mut s = Search {
            query: "x".into(),
            matches: vec![Range::new(0, 1), Range::new(4, 5)],
            ..Search::default()
        };
        assert_eq!(s.select_after(1), Some(Range::new(4, 5)));
        assert_eq!(s.current, 1);
        assert_eq!(s.label(), "2/2");
        assert_eq!(s.select_after(5), Some(Range::new(0, 1)), "末尾回绕首处");
        s.matches.clear();
        assert_eq!(s.select_after(0), None);
        assert_eq!(s.current, 0);
    }

    #[test]
    fn step_wraps_in_both_directions() {
        let mut s = Search {
            matches: vec![Range::new(0, 1), Range::new(2, 3), Range::new(4, 5)],
            ..Search::default()
        };
        assert_eq!(s.current_match(), Some(Range::new(0, 1)));
        assert_eq!(s.step(true), Some(Range::new(2, 3)));
        assert_eq!(s.step(true), Some(Range::new(4, 5)));
        assert_eq!(s.step(true), Some(Range::new(0, 1)), "末尾回到首个");
        assert_eq!(s.step(false), Some(Range::new(4, 5)), "首部回到末个");
        let mut empty = Search::default();
        assert_eq!(empty.step(true), None);
    }

    #[test]
    fn label_reports_empty_zero_and_position() {
        let mut s = Search::default();
        assert_eq!(s.label(), "");
        s.query = "x".into();
        assert_eq!(s.label(), "0/0");
        s.matches = vec![Range::new(0, 1), Range::new(2, 3)];
        assert_eq!(s.label(), "1/2");
        s.current = 1;
        assert_eq!(s.label(), "2/2");
    }

    #[test]
    fn layout_anchors_to_top_right_and_hit_maps_buttons() {
        let l = layout(800.0, 30.0, 1.0);
        assert_eq!(l.bar.x, 800.0 - BAR_W);
        assert_eq!(l.bar.y, 30.0 + GAP);
        assert_eq!(l.bar.x + l.bar.w, 800.0);
        // 按钮自右向左：关闭 > 下一个 > 上一个 >（分组间距）> 大小写 > 正则
        assert_eq!(l.close.x + l.close.w, l.bar.x + l.bar.w - PAD);
        assert_eq!(l.next.x + l.next.w, l.close.x - BTN_GAP);
        assert_eq!(l.prev.x + l.prev.w, l.next.x - BTN_GAP);
        assert_eq!(l.case.x + l.case.w, l.prev.x - GROUP_GAP);
        assert_eq!(l.regex.x + l.regex.w, l.case.x - BTN_GAP);
        assert!(l.close.y > l.bar.y, "按钮在行内垂直居中");
        assert_eq!(l.text.y, l.close.y);
        assert_eq!(l.text.h, BTN_H);
        let center = |r: Rect| (r.x + r.w / 2.0, r.y + r.h / 2.0);
        let (cx, cy) = center(l.close);
        assert_eq!(hit(&l, 1.0, false, cx, cy), Some(Hit::Close));
        let (nx, ny) = center(l.next);
        assert_eq!(hit(&l, 1.0, false, nx, ny), Some(Hit::Next));
        let (px, py) = center(l.prev);
        assert_eq!(hit(&l, 1.0, false, px, py), Some(Hit::Prev));
        let (kx, ky) = center(l.case);
        assert_eq!(hit(&l, 1.0, false, kx, ky), Some(Hit::ToggleCase));
        let (rx, ry) = center(l.regex);
        assert_eq!(hit(&l, 1.0, false, rx, ry), Some(Hit::ToggleRegex));
        assert_eq!(
            hit(&l, 1.0, false, l.text.x + 2.0, cy),
            Some(Hit::QueryText),
            "文本区点击聚焦查询"
        );
        assert_eq!(hit(&l, 1.0, false, l.bar.x - 1.0, cy), None, "栏外不命中");
        // 查询文本区 → 正则 → Aa → 计数并入输入区右缘，自左向右不重叠
        assert_eq!(l.text.x + l.text.w, l.regex.x - PAD);
        assert_eq!(l.count_right, l.regex.x - PAD);
        assert!(
            l.text.x + l.text.w - COUNT_W >= l.text.x,
            "计数预留在输入区内"
        );
        assert!((l.text.w - COUNT_W) > 0.0, "输入区放得下计数");
    }

    #[test]
    fn replace_row_sits_below_find_row_and_maps_hits() {
        let l = layout(800.0, 30.0, 1.0);
        let r = replace_layout(&l, 1.0);
        assert_eq!(r.row.y, l.bar.y + l.bar.h + ROW_GAP);
        assert_eq!(r.row.x, l.bar.x);
        assert_eq!(r.row.w, l.bar.w);
        assert_eq!(
            r.all.x + r.all.w,
            l.bar.x + l.bar.w - PAD,
            "按钮贴右缘内边距"
        );
        assert_eq!(r.one.x + r.one.w, r.all.x - BTN_GAP);
        assert_eq!(r.text.x + r.text.w, r.one.x - PAD);
        let center = |q: Rect| (q.x + q.w / 2.0, q.y + q.h / 2.0);
        let (ox, oy) = center(r.one);
        assert_eq!(hit(&l, 1.0, true, ox, oy), Some(Hit::ReplaceOne));
        let (ax, ay) = center(r.all);
        assert_eq!(hit(&l, 1.0, true, ax, ay), Some(Hit::ReplaceAll));
        let (tx, ty) = center(r.text);
        assert_eq!(hit(&l, 1.0, true, tx, ty), Some(Hit::ReplaceText));
        assert_eq!(
            hit(&l, 1.0, false, tx, ty),
            None,
            "非替换模式下替换行不存在"
        );
        // 旧按钮命中不受替换行影响
        let (cx, cy) = center(l.close);
        assert_eq!(hit(&l, 1.0, true, cx, cy), Some(Hit::Close));
        // 两行之间的分组间距属于卡片内部：吞掉点击
        assert_eq!(
            hit(
                &l,
                1.0,
                true,
                l.bar.x + 4.0,
                l.bar.y + l.bar.h + ROW_GAP / 2.0
            ),
            Some(Hit::Bar)
        );
    }

    #[test]
    fn clear_resets_query_replace_and_focus() {
        let mut s = Search {
            open: true,
            replace_open: true,
            query: "a".into(),
            replace: "b".into(),
            focus: Field::Replace,
            case_sensitive: true,
            matches: vec![Range::new(0, 1)],
            current: 1,
            hover: Some(Hit::Next),
            pressed: Some(Hit::Next),
            ..Default::default()
        };
        s.clear();
        assert!(s.query.is_empty());
        assert!(s.replace.is_empty(), "替换串随关闭/换文档清空");
        assert_eq!(s.focus, Field::Query);
        assert!(s.case_sensitive, "大小写开关会话内保持，clear 不重置");
        assert!(s.matches.is_empty());
        assert_eq!(s.current, 0);
        assert_eq!(s.hover, None);
        assert_eq!(s.pressed, None);
    }
}
