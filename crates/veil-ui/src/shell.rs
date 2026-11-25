//! 进程级窗口管理器：每个 `App` 仍只负责一个窗口，`Shell` 按 `WindowId` 路由事件。

use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::Receiver;
use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::WindowId;

use crate::app::{App, Doc, Ports};
use crate::i18n::{t, tf, Key};

// 直接编译产品的同一错误提示入口；测试不复制 MessageBoxW 产品逻辑。
// 只复用其中的产品提示入口，其余平台函数不在本测试目标调用。
#[cfg(all(test, windows))]
#[allow(dead_code)]
// 引入外部 product_error 模块：仅复用提示入口，其余平台函数本测试目标不调用（该文件属 veil crate，不在本次可改范围）
#[path = "../../veil/src/platform/error.rs"]
mod product_error;

#[cfg(all(test, windows))]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 单窗口事件产生的进程级结果。
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(crate) struct RouteOutcome {
    pub(crate) new_window: bool,
    pub(crate) closed: bool,
    pub(crate) exit_process: bool,
}

/// 每个窗口的运行时状态。
pub(crate) struct WindowEntry {
    pub(crate) app: App,
}

pub(crate) struct Shell {
    windows: HashMap<WindowId, WindowEntry>,
    make_ports: Box<dyn FnMut() -> Ports>,
    pending: VecDeque<App>,
    /// 非首窗创建失败的用户可见上报回调：产品层据此弹错误框（不只写 stderr）。
    on_error: Box<dyn Fn(&str)>,
    /// 首窗创建失败的致命错误详情：`run` 据此返回 Err，让产品层弹可见错误框而非静默退出。
    startup_error: Option<String>,
    /// 行为测试专用：只替换已有真实窗口后的下一次创建，首窗仍走完整 winit/softbuffer 路径。
    #[cfg(test)]
    fail_non_initial_create: Option<String>,
}

impl Shell {
    fn new(
        doc: Doc,
        mut make_ports: Box<dyn FnMut() -> Ports>,
        on_error: Box<dyn Fn(&str)>,
    ) -> Self {
        let initial = App::new(doc, make_ports());
        Self {
            windows: HashMap::new(),
            make_ports,
            pending: VecDeque::from([initial]),
            on_error,
            startup_error: None,
            #[cfg(test)]
            fail_non_initial_create: None,
        }
    }

    fn open_doc(&mut self, event_loop: &ActiveEventLoop, mut doc: Doc) {
        // 只有进程的首个窗口允许消费启动恢复槽；其余窗口都是显式文档。
        doc.resume = false;
        let app = App::new(doc, (self.make_ports)());
        self.create_window(event_loop, app);
    }

    fn create_window(&mut self, event_loop: &ActiveEventLoop, mut app: App) {
        #[cfg(test)]
        let id = if !self.windows.is_empty() && self.pending.is_empty() {
            match self.fail_non_initial_create.take() {
                Some(detail) => {
                    app.window_state.startup_error = Some(detail);
                    None
                }
                None => app.create_window(event_loop),
            }
        } else {
            app.create_window(event_loop)
        };
        #[cfg(not(test))]
        let id = app.create_window(event_loop);
        if let Some(id) = id {
            self.windows.insert(id, WindowEntry { app });
            return;
        }
        let detail = app.window_state.take_startup_error();
        // 只有进程的首个窗口失败才是致命启动失败；已有窗口时失败必须让用户看见。
        let fatal = if self.windows.is_empty() && self.pending.is_empty() {
            self.fail_initial_window(detail)
        } else {
            self.report_create_failure(detail, false)
        };
        if fatal {
            event_loop.exit();
        }
    }

    /// 窗口创建失败的统一处置：返回 true 表示已记录致命启动错误、调用方应退出事件循环。
    /// 首窗沿用旧行为（记录 → `finish` 返回 Err）；非首窗写入 stderr 并经注入回调
    /// 弹出可见错误，现有文档与窗口保持不变。
    fn report_create_failure(&mut self, detail: Option<String>, initial: bool) -> bool {
        if initial {
            if self.startup_error.is_none() {
                self.startup_error =
                    Some(detail.unwrap_or_else(|| t(Key::FatalWindowCreate).to_string()));
            }
            return true;
        }
        let message = non_initial_failure_text(detail);
        eprintln!("{message}");
        (self.on_error)(&message);
        false
    }

    /// 首窗（且无其他窗口/待建窗口）创建失败：记录可读错误并请求退出事件循环。
    /// 返回 true 表示错误已记录为致命启动失败，调用方应退出事件循环。
    fn fail_initial_window(&mut self, app_error: Option<String>) -> bool {
        if !self.windows.is_empty() || !self.pending.is_empty() {
            return false;
        }
        self.report_create_failure(app_error, true)
    }

    /// 汇总事件循环结果：正常结束后若记录过致命启动错误，返回该错误（不吞掉）。
    fn finish(&mut self, result: Result<(), String>) -> Result<(), String> {
        match result {
            Ok(()) => match self.startup_error.take() {
                Some(error) => Err(error),
                None => Ok(()),
            },
            Err(e) => Err(e),
        }
    }

    pub(crate) fn route_event(&mut self, id: WindowId, event: WindowEvent) -> RouteOutcome {
        let destroyed = matches!(event, WindowEvent::Destroyed);
        let Some(entry) = self.windows.get_mut(&id) else {
            return RouteOutcome::default();
        };

        entry.app.handle_window_event(event);
        let new_window = std::mem::take(&mut entry.app.window_state.new_window_requested);
        let closed = destroyed || entry.app.finish_close();
        if closed {
            self.windows.remove(&id);
        }
        RouteOutcome {
            new_window: new_window && !closed,
            closed,
            exit_process: self.windows.is_empty() && self.pending.is_empty(),
        }
    }

    #[cfg(test)]
    pub(crate) fn empty_for_test() -> Self {
        Self::empty_for_test_with_sink(|_| {})
    }

    #[cfg(test)]
    pub(crate) fn empty_for_test_with_sink<S>(sink: S) -> Self
    where
        S: Fn(&str) + 'static,
    {
        Self {
            windows: HashMap::new(),
            make_ports: Box::new(|| unreachable!("测试不创建额外窗口")),
            pending: VecDeque::new(),
            on_error: Box::new(sink),
            startup_error: None,
            #[cfg(test)]
            fail_non_initial_create: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn insert_for_test(&mut self, id: WindowId, app: App) {
        self.windows.insert(id, WindowEntry { app });
    }

    #[cfg(test)]
    pub(crate) fn get_for_test(&self, id: WindowId) -> Option<&App> {
        self.windows.get(&id).map(|e| &e.app)
    }
}

// —— ApplicationHandler —— //

impl ApplicationHandler<Doc> for Shell {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        while let Some(app) = self.pending.pop_front() {
            self.create_window(event_loop, app);
            if self.windows.is_empty() {
                break;
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let outcome = self.route_event(id, event);
        if outcome.new_window && !outcome.closed {
            self.open_doc(
                event_loop,
                Doc {
                    text: String::new(),
                    path: None,
                    bom: false,
                    resume: false,
                    open_error: None,
                },
            );
        }
        if outcome.exit_process && self.windows.is_empty() {
            event_loop.exit();
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, doc: Doc) {
        self.open_doc(event_loop, doc);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let mut next: Option<Instant> = None;
        for entry in self.windows.values_mut() {
            if let Some(at) = entry.app.next_wake() {
                next = Some(next.map_or(at, |old| old.min(at)));
            }
        }
        event_loop.set_control_flow(match next {
            Some(at) => ControlFlow::WaitUntil(at),
            None => ControlFlow::Wait,
        });
    }
}

/// 非首窗创建失败的可见文案：原始诊断（缺失时用既有兜底文案）+ 可执行恢复动作。
/// 不新增 i18n key（`FatalWindowCreate` 已存在），恢复动作直接写给用户。
fn non_initial_failure_text(detail: Option<String>) -> String {
    let base = detail.unwrap_or_else(|| t(Key::FatalWindowCreate).to_string());
    format!("{base}\n\n{}", t(Key::FatalWindowBusyNext))
}

/// 启动多窗口事件循环。每个窗口调用一次端口工厂，避免共享不可克隆的 trait object。
/// `on_error` 在非首窗创建失败时收到可读错误（含恢复动作），产品层据此弹出可见错误框。
pub fn run<F, E>(
    doc: Doc,
    make_ports: F,
    on_error: E,
    requests: Option<Receiver<Doc>>,
) -> Result<(), String>
where
    F: FnMut() -> Ports + 'static,
    E: Fn(&str) + 'static,
{
    run_impl(doc, make_ports, on_error, requests)
}

fn run_impl<F, E>(
    doc: Doc,
    make_ports: F,
    on_error: E,
    requests: Option<Receiver<Doc>>,
) -> Result<(), String>
where
    F: FnMut() -> Ports + 'static,
    E: Fn(&str) + 'static,
{
    let event_loop = EventLoop::<Doc>::with_user_event()
        .build()
        .map_err(|e| tf(Key::ErrEventLoopCreate, &[&e.to_string()]))?;
    event_loop.set_control_flow(ControlFlow::Wait);
    if let Some(requests) = requests {
        let proxy = event_loop.create_proxy();
        std::thread::Builder::new()
            .name("veil-ui-requests".into())
            .spawn(move || {
                while let Ok(doc) = requests.recv() {
                    if proxy.send_event(doc).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| tf(Key::ErrEventLoopCreate, &[&e.to_string()]))?;
    }
    let mut shell = Shell::new(doc, Box::new(make_ports), Box::new(on_error));
    let result = event_loop
        .run_app(&mut shell)
        .map_err(|e| tf(Key::ErrEventLoopRun, &[&e.to_string()]));
    shell.finish(result)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;

    /// 记录型上报回调：断言非首窗失败确实经产品层注入的通道变为用户可见错误。
    fn recording_sink() -> (Rc<RefCell<Vec<String>>>, impl Fn(&str) + 'static) {
        let reported: Rc<RefCell<Vec<String>>> = Rc::default();
        let sink = reported.clone();
        (reported, move |message: &str| {
            sink.borrow_mut().push(message.to_string())
        })
    }

    /// 首窗创建失败：错误被记录，收尾据此返回 Err（不再静默退出）。
    #[test]
    fn initial_window_failure_becomes_visible_error() {
        let mut shell = Shell::empty_for_test();
        assert!(shell.fail_initial_window(Some("创建窗口失败: 无显示设备".into())));
        assert_eq!(
            shell.startup_error.as_deref(),
            Some("创建窗口失败: 无显示设备")
        );
        assert_eq!(
            shell.finish(Ok(())),
            Err("创建窗口失败: 无显示设备".to_string())
        );
        assert_eq!(shell.finish(Ok(())), Ok(()), "错误只上报一次");
    }

    /// 应用未留下详情时给兜底文案，仍是可读错误而不是空字符串。
    #[test]
    fn initial_window_failure_without_detail_uses_fallback() {
        let mut shell = Shell::empty_for_test();
        assert!(shell.fail_initial_window(None));
        assert_eq!(
            shell.startup_error.as_deref(),
            Some(t(Key::FatalWindowCreate))
        );
    }

    /// 非首窗失败：必须经注入回调上报非空、含恢复动作的可见错误，且不记为致命启动错误。
    #[test]
    fn second_window_failure_reports_visible_error_with_recovery_action() {
        let (reported, sink) = recording_sink();
        let mut shell = Shell::empty_for_test_with_sink(sink);
        assert!(!shell.report_create_failure(Some("创建窗口失败: 无显示设备".into()), false));
        let messages = reported.borrow();
        assert_eq!(messages.len(), 1, "失败必须上报一次");
        assert!(
            messages[0].contains("创建窗口失败: 无显示设备"),
            "须保留原始诊断: {}",
            messages[0]
        );
        assert!(
            messages[0].contains("关闭一个窗口后重试"),
            "须给出可执行恢复动作: {}",
            messages[0]
        );
        assert_eq!(shell.finish(Ok(())), Ok(()), "非首窗失败不是致命启动错误");
    }

    /// 非首窗失败但应用没留下详情：仍给兜底文案与恢复动作，不能是空串。
    #[test]
    fn second_window_failure_without_detail_uses_fallback_text() {
        let (reported, sink) = recording_sink();
        let mut shell = Shell::empty_for_test_with_sink(sink);
        assert!(!shell.report_create_failure(None, false));
        let messages = reported.borrow();
        assert_eq!(messages.len(), 1);
        assert!(
            messages[0].contains(t(Key::FatalWindowCreate)),
            "{}",
            messages[0]
        );
        assert!(
            messages[0].contains("关闭一个窗口后重试"),
            "{}",
            messages[0]
        );
    }

    /// 首窗失败仍走 `finish` 返回 Err，不误调非首窗的可见上报回调。
    #[test]
    fn initial_window_failure_does_not_use_error_sink() {
        let (reported, sink) = recording_sink();
        let mut shell = Shell::empty_for_test_with_sink(sink);
        assert!(shell.fail_initial_window(Some("创建窗口失败: 无显示设备".into())));
        assert!(
            reported.borrow().is_empty(),
            "首窗失败由 finish 返回 Err 上报"
        );
        assert_eq!(
            shell.finish(Ok(())),
            Err("创建窗口失败: 无显示设备".to_string())
        );
    }

    /// 正常关闭（无致命启动错误）仍返回 Ok。
    #[test]
    fn normal_exit_stays_ok() {
        let mut shell = Shell::empty_for_test();
        assert_eq!(shell.finish(Ok(())), Ok(()));
    }

    /// 事件循环自身的错误不被启动错误覆盖。
    #[test]
    fn event_loop_error_is_preserved() {
        let mut shell = Shell::empty_for_test();
        shell.startup_error = Some("窗口错误".into());
        assert_eq!(
            shell.finish(Err("事件循环错误".into())),
            Err("事件循环错误".to_string())
        );
    }

    #[cfg(windows)]
    mod real_event_loop {
        use std::ffi::c_void;
        use std::path::PathBuf;
        use std::process::{Command, Stdio};
        use std::time::{Duration, Instant};

        use winit::platform::windows::EventLoopBuilderExtWindows;

        use super::*;
        use crate::input::{Key as InputKey, Mods};

        const CHILD_ENV: &str = "VEIL_REAL_SECOND_WINDOW_TEST_CHILD";
        const CHILD_TEST: &str =
            "shell::tests::real_event_loop::second_window_failure_event_loop_child";
        const INJECTED_DETAIL: &str = "创建窗口失败: 确定性测试注入：第二窗口创建失败";

        type Hwnd = *mut c_void;

        #[link(name = "user32")]
        extern "system" {
            fn EnumWindows(
                callback: Option<unsafe extern "system" fn(Hwnd, isize) -> i32>,
                data: isize,
            ) -> i32;
            fn EnumChildWindows(
                parent: Hwnd,
                callback: Option<unsafe extern "system" fn(Hwnd, isize) -> i32>,
                data: isize,
            ) -> i32;
            fn GetWindowThreadProcessId(window: Hwnd, pid: *mut u32) -> u32;
            fn IsWindowVisible(window: Hwnd) -> i32;
            fn GetWindowTextLengthW(window: Hwnd) -> i32;
            fn GetWindowTextW(window: Hwnd, text: *mut u16, capacity: i32) -> i32;
            fn GetDlgItem(window: Hwnd, id: i32) -> Hwnd;
            fn PostMessageW(window: Hwnd, message: u32, wparam: usize, lparam: isize) -> i32;
        }

        struct DialogSearch<'a> {
            pid: u32,
            expected_title: &'a str,
            found: Option<(Hwnd, String, String)>,
        }

        fn window_text(window: Hwnd) -> String {
            unsafe {
                let len = GetWindowTextLengthW(window);
                if len <= 0 {
                    return String::new();
                }
                let mut buffer = vec![0u16; len as usize + 1];
                let read = GetWindowTextW(window, buffer.as_mut_ptr(), buffer.len() as i32);
                String::from_utf16_lossy(&buffer[..read.max(0) as usize])
            }
        }

        unsafe extern "system" fn collect_child_text(window: Hwnd, data: isize) -> i32 {
            let texts = &mut *(data as *mut Vec<String>);
            let text = window_text(window);
            if !text.is_empty() {
                texts.push(text);
            }
            1
        }

        unsafe extern "system" fn find_product_dialog(window: Hwnd, data: isize) -> i32 {
            let search = &mut *(data as *mut DialogSearch<'_>);
            let mut pid = 0;
            GetWindowThreadProcessId(window, &mut pid);
            if pid != search.pid || IsWindowVisible(window) == 0 {
                return 1;
            }
            let title = window_text(window);
            if title != search.expected_title {
                return 1;
            }
            let mut children = Vec::new();
            EnumChildWindows(
                window,
                Some(collect_child_text),
                &mut children as *mut Vec<String> as isize,
            );
            let body = children.join("\n");
            search.found = Some((window, title, body));
            0
        }

        /// 父测试进程枚举真正的 MessageBoxW；只接受指定子进程且标题匹配的可见对话框。
        fn product_dialog(pid: u32, expected_title: &str) -> Option<(Hwnd, String, String)> {
            let mut search = DialogSearch {
                pid,
                expected_title,
                found: None,
            };
            unsafe {
                EnumWindows(
                    Some(find_product_dialog),
                    &mut search as *mut DialogSearch<'_> as isize,
                );
            }
            search.found
        }

        fn close_dialog(window: Hwnd) {
            unsafe {
                let ok = GetDlgItem(window, 1);
                if !ok.is_null() {
                    PostMessageW(ok, 0x00f5, 0, 0); // BM_CLICK
                } else {
                    PostMessageW(window, 0x0010, 0, 0); // WM_CLOSE
                }
            }
        }

        struct Harness {
            shell: Shell,
            started: Instant,
            original_text: String,
            original_path: PathBuf,
            attempted: bool,
            closed: bool,
        }

        impl ApplicationHandler<Doc> for Harness {
            fn resumed(&mut self, event_loop: &ActiveEventLoop) {
                self.shell.resumed(event_loop);
            }

            fn window_event(
                &mut self,
                event_loop: &ActiveEventLoop,
                id: WindowId,
                event: WindowEvent,
            ) {
                self.shell.window_event(event_loop, id, event);
            }

            fn user_event(&mut self, event_loop: &ActiveEventLoop, doc: Doc) {
                self.shell.user_event(event_loop, doc);
            }

            fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
                self.shell.about_to_wait(event_loop);
                if self.attempted {
                    return;
                }
                assert!(
                    self.started.elapsed() < Duration::from_secs(10),
                    "真实首窗未在 10 秒内进入可见状态"
                );
                let Some((&id, entry)) = self.shell.windows.iter().next() else {
                    event_loop.set_control_flow(ControlFlow::WaitUntil(
                        Instant::now() + Duration::from_millis(10),
                    ));
                    return;
                };
                let visible = entry
                    .app
                    .window_state
                    .window
                    .as_ref()
                    .is_some_and(|window| window.is_visible().unwrap_or(false));
                if !visible {
                    event_loop.set_control_flow(ControlFlow::WaitUntil(
                        Instant::now() + Duration::from_millis(10),
                    ));
                    return;
                }

                // 使用生产快捷键路由产生 NewWindow，再由真实 Shell.window_event 消费。
                // winit KeyEvent 含私有平台字段，不能安全伪造；这里不冒充 OS 键盘输入。
                let entry_mut = self.shell.windows.get_mut(&id).expect("首窗仍存在");
                entry_mut.app.input_state.mods = Mods {
                    ctrl: true,
                    shift: true,
                    alt: false,
                };
                entry_mut.app.route_key(InputKey::N);
                assert!(
                    entry_mut.app.window_state.new_window_requested,
                    "Ctrl+Shift+N 必须经过生产路由请求独立窗口"
                );
                // MessageBoxW 自带模态消息循环；先置位，防止其重入 about_to_wait 时重复触发。
                self.attempted = true;
                self.shell
                    .window_event(event_loop, id, WindowEvent::RedrawRequested);
                eprintln!("REAL_EVENT_LOOP stage=native-dialog-returned");

                assert_eq!(self.shell.windows.len(), 1, "失败不得新增或移除窗口");
                let app = self.shell.get_for_test(id).expect("原 WindowId 必须保留");
                assert_eq!(crate::app_tests::doc_text(app), self.original_text);
                assert_eq!(app.doc.path.as_deref(), Some(self.original_path.as_path()));
                assert!(!app.editor.is_dirty(), "原始文档脏状态不得改变");
                assert!(app
                    .window_state
                    .window
                    .as_ref()
                    .is_some_and(|window| window.is_visible().unwrap_or(false)));
                assert!(
                    self.shell.startup_error.is_none(),
                    "非首窗失败不得变成致命错误"
                );

                // 错误框关闭后仍走正常用户关闭路径，证明事件循环和进程没有致命退出。
                self.shell
                    .window_event(event_loop, id, WindowEvent::CloseRequested);
                self.closed = self.shell.windows.is_empty();
                eprintln!(
                    "REAL_EVENT_LOOP stage=original-window-close closed={}",
                    self.closed
                );
            }
        }

        /// 父进程独立捕获 stderr，避免把测试框架输出或人工观察当成诊断证据。
        #[test]
        fn second_window_failure_real_event_loop_and_native_prompt() {
            let exe = std::env::current_exe().expect("定位当前测试可执行文件");
            let mut child = Command::new(exe)
                .args(["--exact", CHILD_TEST, "--nocapture", "--test-threads=1"])
                .env(CHILD_ENV, "1")
                .env("RUST_BACKTRACE", "1")
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("启动真实事件循环测试子进程");
            let deadline = Instant::now() + Duration::from_secs(45);
            let expected_title = t(Key::FatalWindowCreate).to_string();
            let mut dialog = None;
            let timed_out = loop {
                if dialog.is_none() {
                    if let Some((window, title, body)) = product_dialog(child.id(), &expected_title)
                    {
                        close_dialog(window);
                        dialog = Some((title, body));
                    }
                }
                match child.try_wait().expect("轮询测试子进程") {
                    Some(_) => break false,
                    None if Instant::now() >= deadline => break true,
                    None => std::thread::sleep(Duration::from_millis(50)),
                }
            };
            if timed_out {
                child.kill().expect("超时后仅回收本测试子进程");
            }
            let output = child.wait_with_output().expect("收集子进程输出");
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(
                !timed_out,
                "真实事件循环子进程超时\nstdout:\n{stdout}\nstderr:\n{stderr}"
            );
            assert!(
                output.status.success(),
                "子进程失败\nstdout:\n{stdout}\nstderr:\n{stderr}"
            );
            assert!(
                stdout.contains("REAL_EVENT_LOOP_CHILD PASS"),
                "缺少子进程行为证据: {stdout}"
            );
            assert!(
                stderr.contains(INJECTED_DETAIL),
                "stderr 未保留原始诊断: {stderr}"
            );
            assert!(
                stderr.contains("请关闭一个窗口后重试"),
                "stderr 未保留恢复建议: {stderr}"
            );
            let (title, body) = dialog.expect("父进程未捕获可见的产品原生错误框");
            assert_eq!(title, expected_title);
            assert!(body.contains(INJECTED_DETAIL), "原生提示缺少诊断: {body}");
            assert!(
                body.contains("请关闭一个窗口后重试"),
                "原生提示缺少恢复建议: {body}"
            );
            assert!(
                body.contains("现有文档不会被关闭"),
                "原生提示缺少文档保持承诺: {body}"
            );
            println!(
                "REAL_EVENT_LOOP_DETERMINISTIC_INJECTION PASS native_prompt=visible_product_messagebox original_window=preserved original_document=preserved graceful_exit=0 stderr=captured"
            );
            println!("NATIVE_OS_FAILURE needs-user reason=未制造真实操作系统窗口资源故障");
        }

        /// 仅由父测试以独立进程运行；普通全量测试中的直接调用不创建窗口。
        #[test]
        fn second_window_failure_event_loop_child() {
            if std::env::var_os(CHILD_ENV).is_none() {
                return;
            }
            let original_path = PathBuf::from(r"D:\veil-test-only\original-document.md");
            let original_text = "# 原始文档\n\n第二窗口失败后这段文字必须保留。".to_string();
            let doc = Doc {
                text: original_text.clone(),
                path: Some(original_path.clone()),
                bom: false,
                resume: false,
                open_error: None,
            };
            let mut shell = Shell::new(
                doc,
                Box::new(|| {
                    crate::app_tests::ports(
                        &crate::app_tests::MemStore::default(),
                        crate::app_tests::no_url(),
                    )
                }),
                Box::new(super::super::product_error::show_window_create_error),
            );
            shell.fail_non_initial_create = Some(INJECTED_DETAIL.into());

            let mut builder = EventLoop::<Doc>::with_user_event();
            builder.with_any_thread(true);
            let event_loop = builder.build().expect("创建真实 winit 事件循环");
            event_loop.set_control_flow(ControlFlow::Wait);
            let mut harness = Harness {
                shell,
                started: Instant::now(),
                original_text,
                original_path,
                attempted: false,
                closed: false,
            };
            let result = event_loop
                .run_app(&mut harness)
                .map_err(|error| error.to_string());
            assert!(harness.attempted, "未触发第二窗口创建");
            assert!(harness.closed, "失败后原窗口未能正常关闭");
            assert_eq!(harness.shell.finish(result), Ok(()), "事件循环必须正常收尾");
            println!(
                "REAL_EVENT_LOOP_CHILD PASS native_prompt=returned original_window=visible original_document=unchanged graceful_close=true"
            );
        }
    }
}
