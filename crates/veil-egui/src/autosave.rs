//! 自动保存策略：纯逻辑、零 GUI 依赖（headless 可测）。
//!
//! 触发条件四则同时成立：有未保存修改、文档有关联文件、非 IME 组合期、静默期已满。
//! 静默期以「上次内容编辑或上次自动保存尝试」为起点（失败尝试同样推迟下次，
//! 避免磁盘只读/被锁时忙轮询）。
//!
//! 冲突语义见 shell：自动保存绝不覆盖被外部修改的文件，只写冲突副本并挂起。

use std::time::{Duration, Instant};

/// 默认静默期：击键停止 2s 后落盘。足够避开连续输入，又把崩溃丢失窗口压到最小。
const DEFAULT_IDLE: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug)]
pub struct Autosave {
    idle: Duration,
}

impl Default for Autosave {
    fn default() -> Self {
        Self::new(DEFAULT_IDLE)
    }
}

impl Autosave {
    pub fn new(idle: Duration) -> Self {
        Self { idle }
    }

    pub fn idle(&self) -> Duration {
        self.idle
    }

    /// 是否应立刻自动保存。`idle_for` = 距上次内容编辑/自动保存尝试的时长。
    /// `hold` = 冲突挂起（自动保存曾撞上外部修改，等待用户显式保存）。
    pub fn due(
        &self,
        dirty: bool,
        has_path: bool,
        ime_active: bool,
        hold: bool,
        idle_for: Duration,
    ) -> bool {
        dirty && has_path && !ime_active && !hold && idle_for >= self.idle
    }

    /// 下次判定时刻；无待保存内容（`last_activity` 为空）时无需调度。
    pub fn deadline(&self, last_activity: Option<Instant>) -> Option<Instant> {
        last_activity.map(|t| t + self.idle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn due_only_when_all_conditions_hold() {
        let a = Autosave::new(Duration::from_secs(2));
        let ok = Duration::from_secs(2);
        assert!(a.due(true, true, false, false, ok));
        assert!(!a.due(false, true, false, false, ok), "干净文档不保存");
        assert!(!a.due(true, false, false, false, ok), "草稿无路径不保存");
        assert!(!a.due(true, true, true, false, ok), "IME 组合期不保存");
        assert!(!a.due(true, true, false, true, ok), "冲突挂起时不保存");
        assert!(
            !a.due(true, true, false, false, ok - Duration::from_millis(1)),
            "静默期未满不保存"
        );
    }

    #[test]
    fn deadline_is_last_activity_plus_idle() {
        let a = Autosave::default();
        assert_eq!(a.deadline(None), None);
        let t = Instant::now();
        assert_eq!(a.deadline(Some(t)), Some(t + a.idle()));
    }

    #[test]
    fn default_idle_is_two_seconds() {
        assert_eq!(Autosave::default().idle(), Duration::from_secs(2));
    }
}
