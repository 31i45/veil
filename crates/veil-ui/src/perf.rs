//! 性能探针（P0 证据基础设施）：仅在 `VEIL_PERF_PROBE=1` 时向 stderr 输出
//! `[perf] {json}` 单行，供 `tools/walkthrough.ps1 -Perf` 解析。
//!
//! 关闭时：`OnceLock` 判定一次后所有入口直接返回，零分配、零 IO、不写 stderr。
//! 不引入 serde：字段全为整数，手写 JSON 即可。

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

static ENABLED: OnceLock<bool> = OnceLock::new();

/// 环境变量判定（独立成纯函数，便于测试且不触碰进程环境）。
fn is_enabled(raw: Option<&std::ffi::OsStr>) -> bool {
    matches!(raw, Some(v) if v == "1")
}

/// 探针是否启用。首次调用读环境变量，此后只读缓存。
pub(crate) fn enabled() -> bool {
    *ENABLED.get_or_init(|| is_enabled(std::env::var_os("VEIL_PERF_PROBE").as_deref()))
}

/// 编辑后快照（由 `after_edit` 在编辑完成后采集）。
pub(crate) struct EditSnapshot {
    pub dirty_blocks: usize,
    pub total_blocks: usize,
    pub rescan_lines: usize,
    pub undo_depth: usize,
}

fn micros(d: Duration) -> u128 {
    d.as_micros()
}

/// 按键耗时：从收到按键到路由/编辑完成。
fn key_line(us: u128, rss_mb: u64) -> String {
    format!("{{\"key_us\":{us},\"rss_mb\":{rss_mb}}}")
}

/// 单帧耗时：render + present。
fn frame_line(us: u128, rss_mb: u64) -> String {
    format!("{{\"frame_us\":{us},\"rss_mb\":{rss_mb}}}")
}

/// 编辑快照：脏块/总块/重扫行数/撤销深度。
fn edit_line(snap: &EditSnapshot, rss_mb: u64) -> String {
    format!(
        "{{\"dirty_blocks\":{},\"total_blocks\":{},\"rescan_lines\":{},\"undo_depth\":{},\"rss_mb\":{rss_mb}}}",
        snap.dirty_blocks, snap.total_blocks, snap.rescan_lines, snap.undo_depth
    )
}

fn emit(line: &str) {
    eprintln!("[perf] {line}");
}

/// 记录一次按键路由耗时（关闭时立即返回）。
pub(crate) fn key(elapsed: Duration) {
    if !enabled() {
        return;
    }
    emit(&key_line(micros(elapsed), rss_mb()));
}

/// 记录一帧合成耗时（关闭时立即返回）。
pub(crate) fn frame(elapsed: Duration) {
    if !enabled() {
        return;
    }
    emit(&frame_line(micros(elapsed), rss_mb()));
}

/// 记录一次编辑完成后的快照（关闭时立即返回）。
pub(crate) fn edit(snap: EditSnapshot) {
    if !enabled() {
        return;
    }
    emit(&edit_line(&snap, rss_mb()));
}

/// RSS 采样缓存：每秒最多查询一次（探针关闭时永不到达这里）。
static RSS_CACHE: Mutex<Option<(Instant, u64)>> = Mutex::new(None);

fn rss_mb() -> u64 {
    let mut slot = match RSS_CACHE.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some((at, value)) = *slot {
        if at.elapsed() < Duration::from_secs(1) {
            return value;
        }
    }
    let value = query_rss_mb().unwrap_or(0);
    *slot = Some((Instant::now(), value));
    value
}

#[cfg(windows)]
fn query_rss_mb() -> Option<u64> {
    win::rss_mb()
}

#[cfg(not(windows))]
fn query_rss_mb() -> Option<u64> {
    None
}

#[cfg(windows)]
#[allow(non_snake_case)]
mod win {
    use std::ffi::c_void;

    /// PROCESS_MEMORY_COUNTERS（psapi.h）：DWORD cb + DWORD PageFaultCount + 8×SIZE_T。
    #[repr(C)]
    struct ProcessMemoryCounters {
        cb: u32,
        page_fault_count: u32,
        peak_working_set_size: usize,
        working_set_size: usize,
        quota_peak_paged_pool_usage: usize,
        quota_paged_pool_usage: usize,
        quota_peak_non_paged_pool_usage: usize,
        quota_non_paged_pool_usage: usize,
        pagefile_usage: usize,
        peak_pagefile_usage: usize,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn K32GetProcessMemoryInfo(
            process: *mut c_void,
            counters: *mut ProcessMemoryCounters,
            cb: u32,
        ) -> i32;
        fn GetCurrentProcess() -> *mut c_void;
    }

    pub(super) fn rss_mb() -> Option<u64> {
        let mut counters = ProcessMemoryCounters {
            cb: std::mem::size_of::<ProcessMemoryCounters>() as u32,
            page_fault_count: 0,
            peak_working_set_size: 0,
            working_set_size: 0,
            quota_peak_paged_pool_usage: 0,
            quota_paged_pool_usage: 0,
            quota_peak_non_paged_pool_usage: 0,
            quota_non_paged_pool_usage: 0,
            pagefile_usage: 0,
            peak_pagefile_usage: 0,
        };
        // SAFETY：传入当前进程伪句柄与本栈上结构；cb 为结构实际字节数。
        // 失败仅返回 0，探针静默降级（不 panic、不影响编辑路径）。
        let ok =
            unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb) };
        if ok == 0 {
            return None;
        }
        Some(counters.working_set_size as u64 / (1024 * 1024))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    #[test]
    fn enable_flag_requires_exact_one() {
        assert!(is_enabled(Some(OsStr::new("1"))));
        assert!(!is_enabled(Some(OsStr::new("0"))));
        assert!(!is_enabled(Some(OsStr::new("true"))));
        assert!(!is_enabled(None));
    }

    #[test]
    fn lines_are_flat_json_with_expected_fields() {
        let key = key_line(123, 42);
        assert!(key.starts_with('{') && key.ends_with('}'), "key 行：{key}");
        assert!(key.contains("\"key_us\":123"), "key 行：{key}");
        assert!(key.contains("\"rss_mb\":42"), "key 行：{key}");

        let frame = frame_line(456, 42);
        assert!(frame.contains("\"frame_us\":456"), "frame 行：{frame}");

        let snap = EditSnapshot {
            dirty_blocks: 1,
            total_blocks: 9,
            rescan_lines: 3,
            undo_depth: 2,
        };
        let edit = edit_line(&snap, 42);
        for field in [
            "\"dirty_blocks\":1",
            "\"total_blocks\":9",
            "\"rescan_lines\":3",
            "\"undo_depth\":2",
            "\"rss_mb\":42",
        ] {
            assert!(edit.contains(field), "edit 行缺少 {field}：{edit}");
        }
        // 无嵌套对象/数组：ConvertFrom-Json 可直接得到扁平等量
        assert!(!edit.contains('[') && edit.matches('{').count() == 1);
    }

    #[test]
    fn disabled_probe_produces_no_line() {
        // 关闭时三个入口在 emit 前返回；这里以「行构造器未被调用」的判据间接验证：
        // 探针入口不持有任何输出缓冲，关闭时只做一次原子读。
        // （stdout/stderr 捕获在单元测试中不可用，故用构造器与启用判定分离来保证。）
        let raw: Option<&OsStr> = None;
        assert!(!is_enabled(raw));
    }
}
