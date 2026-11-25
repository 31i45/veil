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

    /// 预留 getter：生产代码路径从不调用（自动保存完全由 deadline/activity 驱动），
    /// 仅被 cfg(test) 场景用来断言默认值。
    #[allow(dead_code)] // 仅 cfg(test) 使用：断言默认 idle 时长
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

    /// 大文件往返：10 万字量级文档（含单行约 1 MiB 段落与 1000 个围栏代码块）
    /// 编码后再解码，文本与字节都必须完全一致。只走编解码，不触发排版。
    #[test]
    fn large_document_roundtrip_is_byte_identical() {
        use crate::docio::{decode, encode};

        let mut text = String::with_capacity(2 * 1024 * 1024);
        text.push_str("# 大文档往返\n\n");
        for i in 0..1000 {
            text.push_str("```rust\nfn f");
            text.push_str(&i.to_string());
            text.push_str("() {}\n```\n\n");
        }
        // 单行约 1 MiB（UTF-8 三字节/字）：连同上面的围栏块，总字符数远超 10 万。
        text.push_str(&"长".repeat(350_000));
        text.push('\n');

        let start = Instant::now();
        for bom in [false, true] {
            let bytes = encode(&text, bom);
            let (decoded, has_bom) = decode(&bytes).expect("合法 UTF-8 必须可解码");
            assert_eq!(has_bom, bom);
            assert_eq!(decoded, text, "文本往返必须一致");
            assert_eq!(encode(&decoded, has_bom), bytes, "字节往返必须一致");
        }
        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_secs(5),
            "往返耗时异常（可能误触发排版）：{elapsed:?}"
        );
        eprintln!(
            "大文件往返：{} 字符 / {} 字节，耗时 {elapsed:?}",
            text.chars().count(),
            text.len()
        );
    }
}
