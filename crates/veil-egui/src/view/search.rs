//! 文档内查找栏（P3-3）：匹配扫描状态与浮栏几何。
//!
//! 单文件编辑器语义：全文一次性扫描，不做跨文件搜索。纯逻辑零 GUI——本文件只
//! 承载状态容器与「画在哪 / 点到谁」的纯函数；绘制在根级 paint，交互在 interaction。
//! 打开期间文档只读（文本输入全部进入查询串），故匹配缓存仅在查询变化/打开时重建。

use veil_core::Range;

use crate::geometry::BASE_FONT_SIZE;

/// 浮栏基准宽（乘 DPI scale）。
pub const BAR_W: f32 = 360.0;
/// 浮栏基准高。
pub const BAR_H: f32 = 36.0;
/// 栏内按钮基准宽（上一个/下一个/关闭）。
pub const BTN_W: f32 = 30.0;
/// 栏内左右内边距基准值。
pub const PAD: f32 = 8.0;
/// 浮栏与内容视口顶边的间距基准值。
pub const GAP: f32 = 8.0;
/// 计数区基准宽（右对齐「i/n」）。
pub const COUNT_W: f32 = 56.0;
/// 替换行按钮基准宽（「替换」/「全部」）。
pub const REPLACE_BTN_W: f32 = 56.0;

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct Rect {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) w: f32,
    pub(crate) h: f32,
}

impl Rect {
    pub(crate) fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

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
    Bar,
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
    /// 计数文本右缘（右对齐基准）。
    pub(crate) count_right: f32,
    pub(crate) prev: Rect,
    pub(crate) next: Rect,
    pub(crate) close: Rect,
}

/// 浮栏布局：贴内容视口右上角（`right` 已扣除滚动条/大纲面板），按钮自右向左排列。
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
    let close = Rect {
        x: bar.x + w - bw,
        y: bar.y,
        w: bw,
        h,
    };
    let next = Rect {
        x: close.x - bw,
        ..close
    };
    let prev = Rect {
        x: next.x - bw,
        ..close
    };
    let text_x = bar.x + PAD * scale;
    let count_right = prev.x - PAD * scale;
    let text = Rect {
        x: text_x,
        y: bar.y,
        w: (count_right - COUNT_W * scale - text_x).max(0.0),
        h,
    };
    BarLayout {
        bar,
        text,
        count_right,
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

/// 替换行：紧贴查询行下方，按钮贴右缘（与查询行按钮同一右对齐线）。
pub(crate) fn replace_layout(l: &BarLayout, scale: f32) -> ReplaceLayout {
    let bw = REPLACE_BTN_W * scale;
    let row = Rect {
        x: l.bar.x,
        y: l.bar.y + l.bar.h,
        w: l.bar.w,
        h: l.bar.h,
    };
    let all = Rect {
        x: row.x + row.w - bw,
        y: row.y,
        w: bw,
        h: row.h,
    };
    let one = Rect {
        x: all.x - bw,
        ..all
    };
    let text_x = row.x + PAD * scale;
    let text = Rect {
        x: text_x,
        y: row.y,
        w: (one.x - PAD * scale - text_x).max(0.0),
        h: row.h,
    };
    ReplaceLayout {
        row,
        text,
        one,
        all,
    }
}

/// 栏内命中：`None` = 栏外（调用方按普通点击处理）。替换模式下先命中替换行。
pub(crate) fn hit(l: &BarLayout, scale: f32, replace_open: bool, x: f32, y: f32) -> Option<Hit> {
    if replace_open {
        let r = replace_layout(l, scale);
        if r.all.contains(x, y) {
            return Some(Hit::ReplaceAll);
        }
        if r.one.contains(x, y) {
            return Some(Hit::ReplaceOne);
        }
        if r.text.contains(x, y) {
            return Some(Hit::ReplaceText);
        }
        if r.row.contains(x, y) {
            return Some(Hit::Bar);
        }
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
    } else if l.text.contains(x, y) {
        Some(Hit::QueryText)
    } else {
        Some(Hit::Bar)
    }
}

/// chrome 近似文本宽度：CJK 全角按字号、ASCII 按 0.6 字号估算。
/// 只用于截断与 IME 候选框定位，不参与文本光栅（与 outline 同一取舍）。
pub(crate) fn text_width(text: &str, scale: f32) -> f32 {
    text.chars()
        .map(|c| {
            if c.is_ascii() {
                BASE_FONT_SIZE * 0.6 * scale
            } else {
                BASE_FONT_SIZE * scale
            }
        })
        .sum()
}

/// 查询串按可用宽度截断：保留尾部（正在输入的一端），超出补前导 `…`。
pub(crate) fn fit_query(text: &str, max_w: f32, scale: f32) -> String {
    if text_width(text, scale) <= max_w {
        return text.to_string();
    }
    let avail = (max_w - BASE_FONT_SIZE * scale).max(0.0);
    let mut tail: Vec<char> = Vec::new();
    let mut w = 0.0;
    for c in text.chars().rev() {
        let cw = if c.is_ascii() {
            BASE_FONT_SIZE * 0.6 * scale
        } else {
            BASE_FONT_SIZE * scale
        };
        if w + cw > avail {
            break;
        }
        w += cw;
        tail.push(c);
    }
    tail.reverse();
    let mut out = String::from("…");
    out.extend(tail);
    out
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
/// 默认大小写不敏感；折叠会使字节偏移失效时回退大小写敏感——两条路径都保证
/// 返回的区间是原文字节偏移。
pub(crate) fn find_all(hay: &str, needle: &str) -> Vec<Range> {
    if hay.is_empty() || needle.is_empty() {
        return Vec::new();
    }
    let (h, n) = match (fold_case(hay), fold_case(needle)) {
        (Some(h), Some(n)) => (h, n),
        _ => (hay.to_string(), needle.to_string()),
    };
    h.match_indices(n.as_str())
        .map(|(i, _)| Range::new(i, i + needle.len()))
        .collect()
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
    /// 全文命中区间（按起点升序，非重叠），绘制高亮与跳转同源。
    pub(crate) matches: Vec<Range>,
    /// 当前命中下标（`matches` 内）。
    pub(crate) current: usize,
    pub(crate) hover: Option<Hit>,
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
    }

    /// 查询变化后重算匹配并回到首个命中。
    pub(crate) fn rescan(&mut self, hay: &str) {
        self.matches = find_all(hay, &self.query);
        self.current = 0;
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

    /// 计数标签：空查询空串；无命中 `0/0`；否则 `i/n`（1 起）。
    pub(crate) fn label(&self) -> String {
        if self.query.is_empty() {
            String::new()
        } else if self.matches.is_empty() {
            "0/0".into()
        } else {
            format!("{}/{}", self.current + 1, self.matches.len())
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
            find_all("Foo foo", "foo"),
            vec![Range::new(0, 3), Range::new(4, 7)]
        );
        // 换行与多字节混排：「中A中a」中=3 字节
        // 「中A」= 中(3)+A(1)、「中a」= 中(3)+a(1)：大小写不敏感各命中一处
        assert_eq!(
            find_all("中A中a", "中a"),
            vec![Range::new(0, 4), Range::new(4, 8)]
        );
    }

    #[test]
    fn find_all_reports_cjk_and_utf8_offsets() {
        assert_eq!(
            find_all("中文中文", "中文"),
            vec![Range::new(0, 6), Range::new(6, 12)]
        );
    }

    #[test]
    fn empty_needle_or_hay_yields_no_matches() {
        assert!(find_all("abc", "").is_empty());
        assert!(find_all("", "a").is_empty());
    }

    #[test]
    fn overlapping_occurrences_are_not_reported() {
        // match_indices 非重叠语义（与正则全局匹配一致）
        assert_eq!(find_all("aaa", "aa"), vec![Range::new(0, 2)]);
    }

    #[test]
    fn case_fold_offset_shift_falls_back_to_case_sensitive() {
        // KELVIN SIGN（U+212A）小写为 k 但字节长度 3 → 1：折叠偏移不可复用
        assert!(fold_case("\u{212A}").is_none());
        assert!(
            find_all("\u{212A}", "k").is_empty(),
            "回退大小写敏感，不命中"
        );
        // 混合串仍按原串大小写敏感扫描：仅 ASCII 「k」命中（字节 3）
        assert_eq!(find_all("\u{212A}k", "k"), vec![Range::new(3, 4)]);
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
        // 按钮自右向左：关闭 > 下一个 > 上一个
        assert_eq!(l.close.x + l.close.w, l.bar.x + l.bar.w);
        assert_eq!(l.next.x + l.next.w, l.close.x);
        assert_eq!(l.prev.x + l.prev.w, l.next.x);
        let center = |r: Rect| (r.x + r.w / 2.0, r.y + r.h / 2.0);
        let (cx, cy) = center(l.close);
        assert_eq!(hit(&l, 1.0, false, cx, cy), Some(Hit::Close));
        let (nx, ny) = center(l.next);
        assert_eq!(hit(&l, 1.0, false, nx, ny), Some(Hit::Next));
        let (px, py) = center(l.prev);
        assert_eq!(hit(&l, 1.0, false, px, py), Some(Hit::Prev));
        assert_eq!(
            hit(&l, 1.0, false, l.text.x + 2.0, cy),
            Some(Hit::QueryText),
            "文本区点击聚焦查询"
        );
        assert_eq!(hit(&l, 1.0, false, l.bar.x - 1.0, cy), None, "栏外不命中");
        // 查询文本区在计数区左侧
        assert!(l.text.x + l.text.w <= l.count_right - COUNT_W + 0.01);
    }

    #[test]
    fn replace_row_sits_below_find_row_and_maps_hits() {
        let l = layout(800.0, 30.0, 1.0);
        let r = replace_layout(&l, 1.0);
        assert_eq!(r.row.y, l.bar.y + l.bar.h);
        assert_eq!(r.row.x, l.bar.x);
        assert_eq!(r.row.w, l.bar.w);
        assert_eq!(r.all.x + r.all.w, l.bar.x + l.bar.w, "按钮贴右缘");
        assert_eq!(r.one.x + r.one.w, r.all.x);
        assert!(r.text.x + r.text.w <= r.one.x - PAD + 0.01);
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
    }

    #[test]
    fn fit_query_keeps_short_and_ellipsizes_head() {
        assert_eq!(fit_query("abc", 200.0, 1.0), "abc");
        let long = "一个很长很长很长很长的查询字符串";
        let out = fit_query(long, 60.0, 1.0);
        assert!(out.starts_with('…'), "保留尾部须加前导省略号: {out}");
        assert!(out.chars().count() < long.chars().count());
        // 极窄区域仍返回省略号（不 panic）
        assert_eq!(fit_query(long, 1.0, 1.0), "…");
    }

    #[test]
    fn clear_resets_query_replace_and_focus() {
        let mut s = Search {
            open: true,
            replace_open: true,
            query: "a".into(),
            replace: "b".into(),
            focus: Field::Replace,
            matches: vec![Range::new(0, 1)],
            current: 1,
            hover: Some(Hit::Next),
        };
        s.clear();
        assert!(s.query.is_empty());
        assert!(s.replace.is_empty(), "替换串随关闭/换文档清空");
        assert_eq!(s.focus, Field::Query);
        assert!(s.matches.is_empty());
        assert_eq!(s.current, 0);
        assert_eq!(s.hover, None);
    }
}
