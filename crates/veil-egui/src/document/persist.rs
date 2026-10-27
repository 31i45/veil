//! 持久化状态（ADR-009 决策三）：事实维度 + 单一状态投影。
//!
//! 审计结论（2026-10-26 实现时验证）：原设计的「Save × Disk 二维之积」不足以行为等价地
//! 表达现状——`hold`（自动保存是否挂起）与 `backup`（是否已写冲突副本及其登记版本）
//! 在「自动保存撞上外部修改」与「手工保存覆盖后」两条路径下不可互相推导，
//! 故保留三个独立事实 + 一个时间输入，全部收进本结构，禁止散落在 `App` 顶层：
//!
//! - `pending`：检测到但尚未备份的外部版本（存在即提示「外部修改」）
//! - `backup`：已写冲突副本；`Written(Some(v))` 登记版本用于去重，
//!   `Written(None)` 表示登记已随保存作废、但「已备份」提示保留至下次无冲突保存
//! - `missing`：磁盘缺失/不可读（可与其他事实共存，优先级见 [`Persist::status`]）
//! - `hold`：自动保存挂起（外部冲突未解决，只有手工保存/外部回退可解除）
//! - `activity`：最近内容变更时刻（自动保存调度的输入，非真值）

use std::time::Instant;

/// 外部变更对外呈现的单一状态（状态栏/标题共用）。
/// 优先级：`Changed` > `BackedUp` > `Missing`。
/// 新的待备份版本必须盖过旧版本的「已备份」提示，否则用户会误以为当前磁盘版本已有副本。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ExternalState {
    #[default]
    None,
    /// 检测到外部内容变化，尚未备份（保存覆盖前会先备份）。
    Changed,
    /// 已把外部版本写成冲突副本。
    BackedUp,
    /// 磁盘文件被删除或不可读。
    Missing,
}

/// 冲突副本登记：`Written(None)` 只保留提示，不再有可用于去重的版本。
#[derive(Clone, PartialEq, Eq, Debug, Default)]
enum Backup {
    #[default]
    None,
    Written(Option<Vec<u8>>),
}

#[derive(Default, Debug)]
pub(crate) struct Persist {
    hold: bool,
    pending: Option<Vec<u8>>,
    missing: bool,
    backup: Backup,
    activity: Option<Instant>,
}

impl Persist {
    /// 提示优先级的唯一实现点。
    pub(crate) fn status(&self) -> ExternalState {
        if self.pending.is_some() {
            ExternalState::Changed
        } else if matches!(self.backup, Backup::Written(_)) {
            ExternalState::BackedUp
        } else if self.missing {
            ExternalState::Missing
        } else {
            ExternalState::None
        }
    }

    pub(crate) fn hold(&self) -> bool {
        self.hold
    }

    pub(crate) fn set_hold(&mut self, hold: bool) {
        self.hold = hold;
    }

    pub(crate) fn pending(&self) -> Option<&[u8]> {
        self.pending.as_deref()
    }

    /// 检测到外部新版本：置待备份并挂起自动保存（绝不静默覆盖）。
    pub(crate) fn begin_external(&mut self, disk: Vec<u8>) {
        self.pending = Some(disk);
        self.hold = true;
    }

    /// 磁盘回到基线：清待备份，返回此前是否存在（用于决定是否解除挂起）。
    pub(crate) fn clear_pending(&mut self) -> bool {
        self.pending.take().is_some()
    }

    pub(crate) fn missing(&self) -> bool {
        self.missing
    }

    pub(crate) fn set_missing(&mut self, missing: bool) {
        self.missing = missing;
    }

    pub(crate) fn backed_up(&self) -> bool {
        matches!(self.backup, Backup::Written(_))
    }

    /// 已备份的外部版本（去重登记）；`None` 表示无登记。
    pub(crate) fn registry(&self) -> Option<&[u8]> {
        match &self.backup {
            Backup::Written(v) => v.as_deref(),
            Backup::None => None,
        }
    }

    pub(crate) fn mark_backed_up(&mut self, version: Vec<u8>) {
        self.backup = Backup::Written(Some(version));
    }

    /// 保存成功且磁盘已是本版内容：登记作废，但「已备份」提示保留。
    pub(crate) fn keep_notice_only(&mut self) {
        if self.backed_up() {
            self.backup = Backup::Written(None);
        }
    }

    /// 无冲突保存：提示与登记一并清除。
    pub(crate) fn clear_backup(&mut self) {
        self.backup = Backup::None;
    }

    /// 打开/另存为新文档：全部外部状态归零。
    pub(crate) fn reset(&mut self) {
        self.hold = false;
        self.pending = None;
        self.missing = false;
        self.backup = Backup::None;
        self.activity = None;
    }

    pub(crate) fn activity(&self) -> Option<Instant> {
        self.activity
    }

    pub(crate) fn note_activity(&mut self, now: Instant) {
        self.activity = Some(now);
    }

    pub(crate) fn clear_activity(&mut self) {
        self.activity = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn status_priority_is_changed_backedup_missing() {
        let mut p = Persist::default();
        assert_eq!(p.status(), ExternalState::None);

        p.set_missing(true);
        assert_eq!(p.status(), ExternalState::Missing);

        p.mark_backed_up(b"v1".to_vec());
        assert_eq!(p.status(), ExternalState::BackedUp, "已备份盖过缺失");

        p.begin_external(b"v2".to_vec());
        assert_eq!(p.status(), ExternalState::Changed, "待备份盖过已备份");

        p.clear_pending();
        assert_eq!(p.status(), ExternalState::BackedUp);
    }

    #[test]
    fn begin_external_pends_and_holds() {
        let mut p = Persist::default();
        p.begin_external(b"disk".to_vec());
        assert_eq!(p.pending(), Some(b"disk".as_slice()));
        assert!(p.hold(), "外部新版本必须挂起自动保存");
        assert_eq!(p.status(), ExternalState::Changed);
    }

    #[test]
    fn clear_pending_reports_whether_there_was_one() {
        let mut p = Persist::default();
        assert!(!p.clear_pending());
        p.begin_external(b"x".to_vec());
        assert!(p.clear_pending());
        assert!(p.pending().is_none());
        // 解除挂起由调用方按「无已备份提示」决定
        assert!(p.hold(), "clear_pending 本身不改挂起");
    }

    #[test]
    fn keep_notice_only_clears_registry_but_keeps_backed_up() {
        let mut p = Persist::default();
        p.mark_backed_up(b"v1".to_vec());
        assert_eq!(p.registry(), Some(b"v1".as_slice()));
        p.keep_notice_only();
        assert_eq!(p.registry(), None, "登记作废：同版本不再重复比对");
        assert!(p.backed_up());
        assert_eq!(p.status(), ExternalState::BackedUp);
        p.clear_backup();
        assert_eq!(p.status(), ExternalState::None);
    }

    #[test]
    fn reset_zeroes_every_fact() {
        let mut p = Persist::default();
        p.begin_external(b"x".to_vec());
        p.mark_backed_up(b"x".to_vec());
        p.set_missing(true);
        p.note_activity(Instant::now());
        p.reset();
        assert!(!p.hold());
        assert!(p.pending().is_none());
        assert!(!p.missing());
        assert!(!p.backed_up());
        assert!(p.activity().is_none());
        assert_eq!(p.status(), ExternalState::None);
    }

    #[test]
    fn activity_is_a_scheduling_input() {
        let mut p = Persist::default();
        assert!(p.activity().is_none());
        let t = Instant::now();
        p.note_activity(t);
        assert_eq!(p.activity(), Some(t));
        p.clear_activity();
        assert!(p.activity().is_none());
        // 时刻本身不被视为「状态」：note 后经过时间由调用方用 now - activity 判定
        p.note_activity(t);
        assert_eq!(
            p.activity().unwrap() + Duration::from_secs(2),
            t + Duration::from_secs(2)
        );
    }
}
