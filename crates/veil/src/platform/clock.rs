//! 本地时钟（信任显性化）：手写 Win32 `GetLocalTime`，零新增依赖。
//! veil-ui 只依赖 `Clock` trait；本模块是全仓唯一接触系统时间的位置，
//! 因此 headless 测试可用固定时钟替代，不依赖真实时间。

use veil_ui::Clock;

#[cfg(windows)]
#[repr(C)]
struct SystemTime {
    year: u16,
    month: u16,
    day_of_week: u16,
    day: u16,
    hour: u16,
    minute: u16,
    second: u16,
    milliseconds: u16,
}

#[cfg(windows)]
#[repr(C)]
struct FileTime {
    low: u32,
    high: u32,
}

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn GetLocalTime(time: *mut SystemTime);
    fn FileTimeToSystemTime(file_time: *const FileTime, system_time: *mut SystemTime) -> i32;
    fn SystemTimeToTzSpecificLocalTime(
        tz: *const core::ffi::c_void,
        universal: *const SystemTime,
        local: *mut SystemTime,
    ) -> i32;
}

#[cfg(windows)]
fn zero_system_time() -> SystemTime {
    SystemTime {
        year: 0,
        month: 0,
        day_of_week: 0,
        day: 0,
        hour: 0,
        minute: 0,
        second: 0,
        milliseconds: 0,
    }
}

/// 系统时钟：本地时间 `HH:MM`（两位补零）。
pub struct SystemClock;

impl Clock for SystemClock {
    fn local_hhmm(&self) -> String {
        #[cfg(windows)]
        {
            let mut t = zero_system_time();
            // SAFETY: GetLocalTime 只写调用方提供的 SYSTEMTIME；指针非空且布局与 Win32 一致
            unsafe { GetLocalTime(&mut t) };
            format!("{:02}:{:02}", t.hour, t.minute)
        }
        #[cfg(not(windows))]
        {
            // 非 Windows 无本地时区来源（本仓库仅发布 Windows 构建）：零时刻占位
            "00:00".to_string()
        }
    }

    /// 快照时间戳 → 本地 `YYYY-MM-DD HH:MM:SS`：
    /// Unix 毫秒先转 FILETIME，再转 SYSTEMTIME，最后经系统时区转换（含 DST）。
    fn local_datetime(&self, ts_ms: u64) -> String {
        #[cfg(windows)]
        {
            // Unix epoch → FILETIME：1970-01-01 距 1601-01-01 为 11644473600 秒
            let ticks = (ts_ms + 11_644_473_600_000).saturating_mul(10_000);
            let ft = FileTime {
                low: ticks as u32,
                high: (ticks >> 32) as u32,
            };
            let mut utc = zero_system_time();
            let mut local = zero_system_time();
            // SAFETY: 两个转换函数只写调用方提供的 SYSTEMTIME/FILETIME；
            // tz 传 NULL 表示使用当前系统时区。
            let ok = unsafe {
                FileTimeToSystemTime(&ft, &mut utc) != 0
                    && SystemTimeToTzSpecificLocalTime(core::ptr::null(), &utc, &mut local) != 0
            };
            if ok {
                return format!(
                    "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
                    local.year, local.month, local.day, local.hour, local.minute, local.second
                );
            }
        }
        veil_ui::format_unix_ms_utc(ts_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 格式契约：恒为两位补零的 `HH:MM`（状态栏直接拼接，不做二次格式化）。
    #[test]
    fn clock_formats_hhmm() {
        let s = SystemClock.local_hhmm();
        assert_eq!(s.len(), 5, "{s}");
        let (h, m) = s.split_once(':').expect("含冒号分隔");
        assert_eq!((h.len(), m.len()), (2, 2), "{s}");
        assert!(h.chars().all(|c| c.is_ascii_digit()), "{s}");
        assert!(m.chars().all(|c| c.is_ascii_digit()), "{s}");
    }

    /// 快照时间格式契约：`YYYY-MM-DD HH:MM:SS`（固定 19 字符，本地时区）。
    #[test]
    fn clock_formats_local_datetime() {
        let s = SystemClock.local_datetime(1_700_000_000_000);
        assert_eq!(s.len(), 19, "{s}");
        assert_eq!(&s[4..5], "-");
        assert_eq!(&s[7..8], "-");
        assert_eq!(&s[10..11], " ");
        assert_eq!(&s[13..14], ":");
        assert_eq!(&s[16..17], ":");
        assert!(
            s.chars().filter(|c| c.is_ascii_digit()).count() == 14,
            "{s}"
        );
    }
}
