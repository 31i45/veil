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
#[link(name = "kernel32")]
extern "system" {
    fn GetLocalTime(time: *mut SystemTime);
}

/// 系统时钟：本地时间 `HH:MM`（两位补零）。
pub struct SystemClock;

impl Clock for SystemClock {
    fn local_hhmm(&self) -> String {
        #[cfg(windows)]
        {
            let mut t = SystemTime {
                year: 0,
                month: 0,
                day_of_week: 0,
                day: 0,
                hour: 0,
                minute: 0,
                second: 0,
                milliseconds: 0,
            };
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
}
