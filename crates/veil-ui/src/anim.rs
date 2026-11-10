//! 状态过渡动效（S6 / A9）：悬停高亮的透明度过渡。
//!
//! 约束（设计台账 S6）：状态反馈 ≤120ms、单缓动（ease-out cubic）、只改透明度/位置，
//! 且可整体关闭（系统「显示动画」关闭时立即到位）。
//! 时间只作为输入：`tick` 由事件循环推进，绘制侧只读缓存后的 alpha（像素仍是纯函数）。

use std::time::{Duration, Instant};

use crate::tokens::motion;

/// 过渡时长：状态反馈须跟手，超过 120ms 即显得迟钝（令牌单源，S1）。
pub(crate) const DURATION: Duration = motion::FAST;
/// 动画帧间隔（≈60fps）：仅在有过渡进行时驱动重绘。
pub(crate) const FRAME: Duration = Duration::from_millis(16);

/// 过渡槽：目标项、起始不透明度（0..=1）与起始时刻。
#[derive(Clone, Copy, Debug)]
struct Slot {
    idx: usize,
    from: f32,
    start: Instant,
}

/// 单帧可读的过渡结果：当前项与正在淡出的上一项（均为不透明度 0..=255）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct FadeView {
    pub(crate) cur: Option<(usize, u8)>,
    pub(crate) prev: Option<(usize, u8)>,
}

impl FadeView {
    /// 指定条目本帧应使用的不透明度；None = 本帧不画（完全透明）。
    /// 同一项可能同时出现在两槽（快速往返），取较大者避免闪断。
    pub(crate) fn alpha_of(&self, idx: usize) -> Option<u8> {
        let mut a: Option<u8> = None;
        for (i, v) in [self.cur, self.prev].into_iter().flatten() {
            if i == idx {
                a = Some(a.map_or(v, |x| x.max(v)));
            }
        }
        a.filter(|v| *v > 0)
    }
}

/// 单表面悬停过渡：同一时刻只有一个目标项，离开的上一项淡出。
///
/// 关闭（`enabled == false`）时 `view` 直接以满不透明度返回目标项，
/// 绘制像素与无动效版本完全一致（测试与系统关闭动画共用这条路径）。
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Fade {
    enabled: bool,
    cur: Option<Slot>,
    prev: Option<Slot>,
    cur_alpha: u8,
    prev_alpha: u8,
    running: bool,
}

impl Fade {
    pub(crate) fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
        if !on {
            self.cur = None;
            self.prev = None;
            self.cur_alpha = 0;
            self.prev_alpha = 0;
            self.running = false;
        }
    }

    /// 推进到 `now` 并把目标切到 `target`（None = 指针离开所有项）。
    /// 目标变化时以当前不透明度为起点续算，快速往返不跳变。
    pub(crate) fn tick(&mut self, now: Instant, target: Option<usize>) {
        if !self.enabled {
            self.cur = target.map(|idx| Slot {
                idx,
                from: 1.0,
                start: now,
            });
            self.prev = None;
            self.cur_alpha = if target.is_some() { 255 } else { 0 };
            self.prev_alpha = 0;
            self.running = false;
            return;
        }
        let changed = match (self.cur, target) {
            (Some(s), Some(t)) => s.idx != t,
            (None, None) => false,
            _ => true,
        };
        if changed {
            // 上一项从「此刻的实际不透明度」开始淡出
            self.prev = self.cur.map(|s| Slot {
                idx: s.idx,
                from: alpha_in(&s, now),
                start: now,
            });
            self.cur = target.map(|idx| Slot {
                idx,
                from: 0.0,
                start: now,
            });
        }
        if self.prev.is_some_and(|s| progress(&s, now) >= 1.0) {
            self.prev = None;
        }
        self.cur_alpha = self.cur.map_or(0, |s| to_u8(alpha_in(&s, now)));
        self.prev_alpha = self
            .prev
            .map_or(0, |s| to_u8(s.from * (1.0 - motion::ease_out_cubic(progress(&s, now)))));
        self.running = (self.cur.is_some() && self.cur_alpha < 255)
            || (self.prev.is_some() && self.prev_alpha > 0);
    }

    /// 绘制侧读值：关闭时目标项恒为满不透明度（与无动效版本逐像素一致）。
    pub(crate) fn view(&self, target: Option<usize>) -> FadeView {
        if !self.enabled {
            return FadeView {
                cur: target.map(|i| (i, 255)),
                prev: None,
            };
        }
        FadeView {
            cur: self.cur.map(|s| (s.idx, self.cur_alpha)),
            prev: self.prev.map(|s| (s.idx, self.prev_alpha)),
        }
    }

    /// 是否仍在过渡：事件循环据此按帧调度重绘。
    pub(crate) fn running(&self) -> bool {
        self.running
    }
}

/// 线性进度 0..=1（超出时长即收敛，不重复动画）。
fn progress(slot: &Slot, now: Instant) -> f32 {
    let elapsed = now.saturating_duration_since(slot.start).as_secs_f32();
    (elapsed / DURATION.as_secs_f32()).clamp(0.0, 1.0)
}

/// 入场曲线：从起始值补到 1（单缓动 ease-out cubic）。
fn alpha_in(slot: &Slot, now: Instant) -> f32 {
    slot.from + (1.0 - slot.from) * motion::ease_out_cubic(progress(slot, now))
}

fn to_u8(a: f32) -> u8 {
    (a.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_snaps_to_target_every_frame() {
        let t0 = Instant::now();
        let mut f = Fade::default();
        f.tick(t0, Some(3));
        assert_eq!(f.view(Some(3)).alpha_of(3), Some(255));
        assert!(!f.running());
        f.tick(t0 + DURATION, None);
        assert_eq!(f.view(None).alpha_of(3), None);
    }

    #[test]
    fn enabled_eases_in_over_duration() {
        let t0 = Instant::now();
        let mut f = Fade::default();
        f.set_enabled(true);
        f.tick(t0, Some(1));
        assert_eq!(f.view(Some(1)).alpha_of(1), None); // 起点完全透明
        assert!(f.running());
        f.tick(t0 + DURATION / 2, Some(1));
        let mid = f.view(Some(1)).alpha_of(1).expect("半程应可见");
        assert!(mid > 150 && mid < 255, "ease-out 半程应过半: {mid}");
        f.tick(t0 + DURATION, Some(1));
        assert_eq!(f.view(Some(1)).alpha_of(1), Some(255));
        assert!(!f.running());
    }

    #[test]
    fn switching_target_continues_from_current_alpha() {
        let t0 = Instant::now();
        let mut f = Fade::default();
        f.set_enabled(true);
        f.tick(t0, Some(0));
        f.tick(t0 + DURATION / 2, Some(0));
        let a0 = f.view(Some(0)).alpha_of(0).expect("首项半程可见");
        // 切到第二项：首项从当前值淡出，第二项从 0 淡入
        f.tick(t0 + DURATION / 2, Some(1));
        let v = f.view(Some(1));
        assert_eq!(v.alpha_of(0), Some(a0));
        assert_eq!(v.alpha_of(1), None);
        assert!(f.running());
        // 再过一个完整时长：上一项清空，新项到位
        f.tick(t0 + DURATION * 3 / 2, Some(1));
        let v = f.view(Some(1));
        assert_eq!(v.alpha_of(0), None);
        assert_eq!(v.alpha_of(1), Some(255));
        assert!(!f.running());
    }

    #[test]
    fn leaving_fades_out_previous_item() {
        let t0 = Instant::now();
        let mut f = Fade::default();
        f.set_enabled(true);
        f.tick(t0, Some(4));
        f.tick(t0 + DURATION, Some(4));
        f.tick(t0 + DURATION, None);
        let a = f.view(None).alpha_of(4).expect("离开瞬间仍是满不透明度");
        assert_eq!(a, 255);
        f.tick(t0 + DURATION + DURATION / 2, None);
        let mid = f.view(None).alpha_of(4).expect("半程淡出中");
        assert!(mid > 0 && mid < 255, "淡出半程: {mid}");
        f.tick(t0 + DURATION * 2, None);
        assert_eq!(f.view(None).alpha_of(4), None);
        assert!(!f.running());
    }
}
