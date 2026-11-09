//! 快速打开浮层（阶段 1）：候选列表、模糊过滤与浮层几何。
//!
//! 纯逻辑零 GUI：候选由 App 从最近文件 + 当前目录 Markdown 文件派生，本文件只做
//! 去重/过滤/排序与「画在哪 / 点到谁」的纯函数；绘制在根级 paint，交互在 interaction。
//! 打开期间文档只读（文本输入全部进入查询串）。

use std::path::{Path, PathBuf};

use crate::geometry::{Rect, BASE_FONT_SIZE};

/// 浮层基准宽（乘 DPI scale）。
pub const PANEL_W: f32 = 520.0;
/// 输入行基准高。
pub const INPUT_H: f32 = 40.0;
/// 候选行基准高：与全部浮层同一 surface 令牌。
pub const ROW_H: f32 = crate::theme::surface::ROW_HEIGHT;
/// 浮层同时可见的候选行数。
pub const MAX_VISIBLE: usize = 10;
/// 浮层左右内边距基准值：与全部浮层同一 surface 令牌。
pub const PAD: f32 = crate::theme::surface::PADDING;
/// 候选上限：巨型目录也只入列有限条目。
pub const CANDIDATE_CAP: usize = 500;

/// 单个候选：路径 + 展示名 + 来源说明。
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct QuickItem {
    pub(crate) path: PathBuf,
    /// 展示名：文件名（无文件名回退全路径）。
    pub(crate) label: String,
    /// 次要说明：父目录（仅展示）。
    pub(crate) hint: String,
    /// 是否来自最近文件（同分时优先）。
    pub(crate) recent: bool,
}

/// 浮层状态：开关、查询、候选与过滤结果。
#[derive(Default)]
pub(crate) struct QuickOpen {
    pub(crate) open: bool,
    pub(crate) query: String,
    pub(crate) items: Vec<QuickItem>,
    /// 过滤结果（`items` 下标，按得分降序）。
    pub(crate) filtered: Vec<usize>,
    /// 当前选中项在 `filtered` 中的位置。
    pub(crate) selected: usize,
    /// 首个可见候选在 `filtered` 中的位置。
    pub(crate) top: usize,
    /// 悬停行（`filtered` 位置）。
    pub(crate) hover: Option<usize>,
}

impl QuickOpen {
    /// 关闭时清空全部列表态（开关由调用方置位）。
    pub(crate) fn clear(&mut self) {
        self.query.clear();
        self.items.clear();
        self.filtered.clear();
        self.selected = 0;
        self.top = 0;
        self.hover = None;
    }

    /// 查询变化后重算过滤并回到首项。
    pub(crate) fn refilter(&mut self) {
        self.filtered = filter(&self.items, &self.query);
        self.selected = 0;
        self.top = 0;
        self.hover = None;
    }

    pub(crate) fn current(&self) -> Option<&QuickItem> {
        self.filtered
            .get(self.selected)
            .and_then(|&i| self.items.get(i))
    }

    /// 上下移动选中项（钳位不循环），并保证其可见。
    pub(crate) fn step(&mut self, delta: i32) {
        let n = self.filtered.len();
        if n == 0 {
            self.selected = 0;
            return;
        }
        self.selected = (self.selected as i32 + delta).clamp(0, n as i32 - 1) as usize;
        self.ensure_visible();
    }

    /// 选中项移出可见窗口时滚动列表。
    pub(crate) fn ensure_visible(&mut self) {
        if self.selected < self.top {
            self.top = self.selected;
        } else if self.selected >= self.top + MAX_VISIBLE {
            self.top = self.selected + 1 - MAX_VISIBLE;
        }
        self.top = self.top.min(self.filtered.len().saturating_sub(1));
    }

    /// 可见候选在 `filtered` 中的位置区间。
    pub(crate) fn visible(&self) -> std::ops::Range<usize> {
        let end = (self.top + MAX_VISIBLE).min(self.filtered.len());
        self.top..end
    }
}

/// 候选集：最近文件（原顺序）在前，其后是目录内去重后的 Markdown 文件（按名排序）。
/// 总条目不超过 `cap`；重复路径只保留首次出现（最近文件优先）。
pub(crate) fn candidates(recent: &[PathBuf], dir_files: &[PathBuf], cap: usize) -> Vec<QuickItem> {
    let mut out: Vec<QuickItem> = Vec::with_capacity(recent.len() + 8);
    let mut seen: Vec<PathBuf> = Vec::new();
    for p in recent {
        if seen.contains(p) {
            continue;
        }
        seen.push(p.clone());
        out.push(item(p.clone(), true));
    }
    let mut files: Vec<&PathBuf> = dir_files
        .iter()
        .filter(|p| is_markdown(p) && !seen.contains(p))
        .collect();
    files.sort_by_key(|p| file_name(p).to_lowercase());
    for p in files {
        if out.len() >= cap {
            break;
        }
        out.push(item((*p).clone(), false));
    }
    out
}

/// 是否 Markdown 家族扩展名（大小写不敏感）。
pub(crate) fn is_markdown(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            matches!(
                e.to_ascii_lowercase().as_str(),
                "md" | "markdown" | "mdown" | "mkd" | "mkdn"
            )
        })
        .unwrap_or(false)
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn item(path: PathBuf, recent: bool) -> QuickItem {
    let label = file_name(&path);
    let hint = path
        .parent()
        .map(|d| d.display().to_string())
        .unwrap_or_default();
    QuickItem {
        path,
        label,
        hint,
        recent,
    }
}

/// 模糊匹配得分：查询字符按顺序出现在候选文本（大小写不敏感）即命中。
/// 连续匹配、词首（起点或分隔符后）匹配加分；文本越长扣分越多。
/// `None` = 不命中。空查询恒 0 分（保留候选原顺序）。
pub(crate) fn score(query: &str, text: &str) -> Option<i32> {
    let q: Vec<char> = query.to_lowercase().chars().collect();
    if q.is_empty() {
        return Some(0);
    }
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let mut from = 0usize;
    let mut last: Option<usize> = None;
    let mut points = 0i32;
    for qc in &q {
        let mut pos = from;
        loop {
            if pos >= t.len() {
                return None;
            }
            if t[pos] == *qc {
                break;
            }
            pos += 1;
        }
        points += 1;
        if last == Some(pos.wrapping_sub(1)) {
            points += 12; // 连续匹配（权重高于词首，连续串优先于分散串）
        }
        if pos == 0 || matches!(t[pos - 1], ' ' | '-' | '_' | '/' | '\\' | '.') {
            points += 8; // 词首匹配
        }
        last = Some(pos);
        from = pos + 1;
    }
    Some(points * 10 - text.chars().count() as i32)
}

/// 候选匹配得分：文件名优先，文件名不命中时退回全路径；最近文件有同分优先加成。
fn match_score(query: &str, item: &QuickItem) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let base = score(query, &item.label).or_else(|| score(query, &item.path.to_string_lossy()))?;
    Some(base + if item.recent { 10 } else { 0 })
}

/// 过滤候选并排序：得分降序；同分保持原顺序（最近文件在前）。
pub(crate) fn filter(items: &[QuickItem], query: &str) -> Vec<usize> {
    let q = query.trim();
    let mut hits: Vec<(i32, usize)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, it)| match_score(q, it).map(|s| (s, i)))
        .collect();
    hits.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    hits.into_iter().map(|(_, i)| i).collect()
}

/// 浮层几何：面板、输入行与固定数量的候选行槽位。
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct Layout {
    pub(crate) panel: Rect,
    pub(crate) input: Rect,
    pub(crate) rows: Vec<Rect>,
}

/// 浮层布局：水平居中、纵向约 12% 处；候选行数取 `count`（至少 1 行放空态提示）。
pub(crate) fn layout(x: f32, y: f32, w: f32, h: f32, scale: f32, count: usize) -> Layout {
    let pw = (PANEL_W * scale)
        .min((w - 2.0 * PAD * scale).max(0.0))
        .max(160.0 * scale);
    let visible = count.clamp(1, MAX_VISIBLE);
    let rows_h = visible as f32 * ROW_H * scale;
    let ph = INPUT_H * scale + rows_h;
    let px = x + (w - pw) / 2.0;
    let py = (y + h * 0.12).clamp(
        y + PAD * scale,
        (y + h - ph - PAD * scale).max(y + PAD * scale),
    );
    let panel = Rect {
        x: px,
        y: py,
        w: pw,
        h: ph,
    };
    let input = Rect {
        x: px,
        y: py,
        w: pw,
        h: INPUT_H * scale,
    };
    let rows = (0..visible)
        .map(|i| Rect {
            x: px,
            y: py + INPUT_H * scale + i as f32 * ROW_H * scale,
            w: pw,
            h: ROW_H * scale,
        })
        .collect();
    Layout { panel, input, rows }
}

/// 候选行命中：返回 `filtered` 中的位置（无候选/面板外恒 None）。
pub(crate) fn hit(l: &Layout, top: usize, count: usize, x: f32, y: f32) -> Option<usize> {
    let slot = l.rows.iter().position(|r| r.contains(x, y))?;
    (top + slot < count).then_some(top + slot)
}

/// 近似文本宽度：CJK 全角按字号、ASCII 按 0.6 字号（只服务截断/布局，不参与光栅）。
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

/// 按可用宽度截断（超出补尾部 `…`）。
pub(crate) fn fit_label(text: &str, max_w: f32, scale: f32) -> String {
    if text_width(text, scale) <= max_w {
        return text.to_string();
    }
    let avail = (max_w - BASE_FONT_SIZE * scale).max(0.0);
    let mut out = String::new();
    let mut w = 0.0;
    for c in text.chars() {
        let cw = if c.is_ascii() {
            BASE_FONT_SIZE * 0.6 * scale
        } else {
            BASE_FONT_SIZE * scale
        };
        if w + cw > avail {
            break;
        }
        w += cw;
        out.push(c);
    }
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn candidates_put_recent_first_and_dedup_dir_files() {
        let recent = vec![p("D:\\notes\\b.md"), p("D:\\notes\\a.md")];
        let dir = vec![
            p("D:\\notes\\a.md"),
            p("D:\\notes\\c.md"),
            p("D:\\notes\\x.png"),
            p("D:\\notes\\C.md"),
            p("D:\\notes\\b.md"),
        ];
        let items = candidates(&recent, &dir, CANDIDATE_CAP);
        let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
        // 最近文件原顺序在前；目录内去重后按名排序（大小写不敏感，同键保持原序）
        assert_eq!(labels, vec!["b.md", "a.md", "c.md", "C.md"]);
        assert!(items[0].recent && items[1].recent && !items[2].recent);
    }

    #[test]
    fn candidates_respect_cap_and_markdown_filter() {
        let recent: Vec<PathBuf> = Vec::new();
        let dir: Vec<PathBuf> = (0..20).map(|i| p(&format!("D:\\d\\{i:02}.md"))).collect();
        assert_eq!(candidates(&recent, &dir, 5).len(), 5);
        assert!(!is_markdown(Path::new("a.txt")));
        assert!(is_markdown(Path::new("a.MD")));
        assert!(is_markdown(Path::new("a.markdown")));
    }

    #[test]
    fn score_prefers_consecutive_and_word_start() {
        // 「abc」在「abc.md」中连续且词首，应高于「a-b-c.md」中的分散匹配
        let a = score("abc", "abc.md").unwrap();
        let b = score("abc", "a-b-c.md").unwrap();
        assert!(a > b, "连续匹配得分更高: {a} vs {b}");
        // 大小写不敏感 + 中文子序列
        assert!(score("md", "README.MD").is_some());
        assert!(score("中文", "我的中文笔记.md").is_some());
        // 顺序不匹配/缺字符即 None
        assert_eq!(score("cb", "abc"), None);
        assert_eq!(score("z", "abc"), None);
        assert_eq!(score("", "abc"), Some(0));
    }

    #[test]
    fn filter_ranks_best_match_and_keeps_recent_on_ties() {
        let items = vec![
            item(p("D:\\d\\a.md"), true),
            item(p("D:\\d\\abc.md"), false),
            item(p("D:\\d\\zeta.md"), false),
        ];
        let idx = filter(&items, "abc");
        assert_eq!(idx[0], 1, "文件名精确前缀优先于最近加成");
        assert_eq!(idx.len(), 1, "不命中者不出现");
        // 空查询：最近文件优先
        let all = filter(&items, "");
        assert_eq!(all, vec![0, 1, 2]);
    }

    #[test]
    fn step_clamps_and_scrolls_selection_into_view() {
        let items: Vec<QuickItem> = (0..30)
            .map(|i| item(p(&format!("f{i}.md")), false))
            .collect();
        let mut q = QuickOpen {
            items,
            ..QuickOpen::default()
        };
        q.refilter();
        assert_eq!(q.filtered.len(), 30);
        q.step(-1);
        assert_eq!(q.selected, 0, "顶部不越界");
        q.step(MAX_VISIBLE as i32);
        assert_eq!(q.selected, MAX_VISIBLE);
        assert_eq!(q.top, 1, "选中项保持在可见窗口内");
        // 直接跳到底部附近再保证可见
        q.selected = 29;
        q.ensure_visible();
        assert_eq!(q.top, 30 - MAX_VISIBLE);
        assert_eq!(q.visible(), q.top..30);
    }

    #[test]
    fn layout_centers_and_hit_maps_visible_rows() {
        let l = layout(0.0, 30.0, 1000.0, 600.0, 1.0, 12);
        assert!(l.panel.x >= 0.0 && l.panel.x + l.panel.w <= 1000.0);
        assert!(
            (l.panel.x + l.panel.w / 2.0 - 500.0).abs() < 0.01,
            "水平居中"
        );
        assert_eq!(l.rows.len(), MAX_VISIBLE);
        assert_eq!(l.input.y, l.panel.y);
        assert_eq!(l.rows[0].y, l.panel.y + INPUT_H);
        for (slot, r) in l.rows.iter().enumerate() {
            assert_eq!(hit(&l, 2, 12, r.x + 1.0, r.y + r.h / 2.0), Some(2 + slot));
        }
        // 输入行不算候选；越界与末项之外不命中
        assert_eq!(hit(&l, 2, 12, l.input.x + 2.0, l.input.y + 2.0), None);
        assert_eq!(
            hit(
                &l,
                0,
                3,
                l.rows[MAX_VISIBLE - 1].x + 1.0,
                l.rows[MAX_VISIBLE - 1].y + 1.0
            ),
            None
        );
    }

    #[test]
    fn empty_result_still_draws_one_row() {
        let l = layout(0.0, 0.0, 800.0, 600.0, 1.0, 0);
        assert_eq!(l.rows.len(), 1);
        assert_eq!(hit(&l, 0, 0, l.rows[0].x + 1.0, l.rows[0].y + 1.0), None);
    }

    #[test]
    fn fit_label_truncates_with_ellipsis() {
        assert_eq!(fit_label("短名.md", 200.0, 1.0), "短名.md");
        let long = "一个非常非常非常非常非常长的文件名.md";
        let out = fit_label(long, 60.0, 1.0);
        assert!(out.ends_with('…'));
        assert!(out.chars().count() < long.chars().count());
    }
}
