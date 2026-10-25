//! Spike（A5 风险验证）：winit 直收 Ime::Preedit / Ime::Commit，验证自绘 preedit 可行性。
//!
//! 验证点：
//!   1. 开启 `set_ime_allowed(true)` 后，微软拼音等 IME 的 preedit 串与光标字节区间
//!      是否以 `Ime::Preedit(String, Option<(usize, usize)>)` 原样送达；
//!   2. `set_ime_cursor_area` 能否让 IME 候选框跟随我们设定的「光标」位置
//!      （本 demo 无文本渲染，用假光标：每次 Commit 后右移 12px 换行折返）；
//!   3. Commit 事件的最终文本是否正确送达。
//!
//! 人工验证步骤：运行后点击窗口取得焦点，切换到中文输入法打字若干，
//! 观察控制台 PREEDIT/COMMIT 输出及候选框位置，最后关闭窗口结束进程。

use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize};
use winit::event::{Ime, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

struct App {
    window: Option<Window>,
    /// 假光标位置（逻辑像素），模拟编辑区 caret
    caret: LogicalPosition<f64>,
}

impl App {
    fn apply_caret(&self) {
        if let Some(w) = &self.window {
            w.set_ime_cursor_area(self.caret, LogicalSize::new(2.0, 20.0));
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let attrs = Window::default_attributes()
            .with_title("veil IME spike - 点击本窗口后用中文输入法打字")
            .with_inner_size(LogicalSize::new(640.0, 360.0));
        let window = event_loop.create_window(attrs).expect("create window");
        window.set_ime_allowed(true);
        self.window = Some(window);
        self.apply_caret();
        println!("window created, IME allowed, cursor area set at {:?}", self.caret);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                println!("close requested, exit");
                event_loop.exit();
            }
            WindowEvent::Ime(Ime::Enabled) => println!("IME: Enabled"),
            WindowEvent::Ime(Ime::Disabled) => println!("IME: Disabled"),
            WindowEvent::Ime(Ime::Preedit(text, cursor)) => {
                println!("IME: Preedit text={:?} cursor_byte_range={:?}", text, cursor);
            }
            WindowEvent::Ime(Ime::Commit(text)) => {
                println!("IME: Commit text={:?}", text);
                // 假光标前进：模拟「提交后 caret 移动，候选框须跟随」
                self.caret.x += 12.0 * text.chars().count().max(1) as f64;
                if self.caret.x > 600.0 {
                    self.caret.x = 20.0;
                    self.caret.y += 24.0;
                }
                self.apply_caret();
                println!("  caret moved to {:?}", self.caret);
            }
            _ => {}
        }
    }
}

fn main() {
    let event_loop = EventLoop::new().expect("event loop");
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = App {
        window: None,
        caret: LogicalPosition::new(20.0, 20.0),
    };
    event_loop.run_app(&mut app).expect("run app");
}
