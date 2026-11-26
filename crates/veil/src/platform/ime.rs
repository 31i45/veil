//! IME 组合窗/候选窗锚点纠正（Windows）：winit 0.30 把 `COMPOSITIONFORM` 的锚点
//! 写在光标矩形底部（`y + height`），候选窗却用顶部。这里用 Imm32 直接把两者
//! 都拉回光标区域左上角，避免系统组合窗低一行、候选窗与插入点错位。
//!
//! 约定：只做坐标纠正，不改变 Veil 的 preedit 自绘与提交逻辑；失败静默返回
//! （无 HIMC / 非输入法窗口属于正常状态，不弹窗、不写日志噪音）。

use veil_ui::ImeAnchor;

#[derive(Clone, Copy)]
#[repr(C)]
struct Point {
    x: i32,
    y: i32,
}

#[derive(Clone, Copy)]
#[repr(C)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[repr(C)]
struct CompositionForm {
    style: u32,
    current_pos: Point,
    area: Rect,
}

#[repr(C)]
struct CandidateForm {
    index: u32,
    style: u32,
    current_pos: Point,
    area: Rect,
}

type Himc = isize;

const CFS_POINT: u32 = 0x0002;
const CFS_EXCLUDE: u32 = 0x0080;

#[link(name = "imm32")]
extern "system" {
    fn ImmGetContext(hwnd: isize) -> Himc;
    fn ImmReleaseContext(hwnd: isize, himc: Himc) -> i32;
    fn ImmSetCompositionWindow(himc: Himc, form: *const CompositionForm) -> i32;
    fn ImmSetCandidateWindow(himc: Himc, form: *const CandidateForm) -> i32;
}

pub struct WinImeAnchor;

impl ImeAnchor for WinImeAnchor {
    fn correct(&mut self, hwnd: isize, x: f64, y: f64, w: f64, h: f64) {
        if hwnd == 0 {
            return;
        }
        let x = x.round() as i32;
        let y = y.round() as i32;
        let w = w.max(1.0).round() as i32;
        let h = h.max(1.0).round() as i32;
        let area = Rect {
            left: x,
            top: y,
            right: x + w,
            bottom: y + h,
        };
        unsafe {
            let himc = ImmGetContext(hwnd);
            if himc == 0 {
                return;
            }
            let composition = CompositionForm {
                style: CFS_POINT,
                current_pos: Point { x, y },
                area,
            };
            let candidate = CandidateForm {
                index: 0,
                style: CFS_EXCLUDE,
                current_pos: Point { x, y },
                area,
            };
            ImmSetCompositionWindow(himc, &composition);
            ImmSetCandidateWindow(himc, &candidate);
            ImmReleaseContext(hwnd, himc);
        }
    }
}
