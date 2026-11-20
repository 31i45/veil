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

/// 单窗口事件产生的进程级结果。
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(crate) struct RouteOutcome {
    pub(crate) new_window: bool,
    pub(crate) closed: bool,
    pub(crate) exit_process: bool,
}

pub(crate) struct Shell {
    windows: HashMap<WindowId, App>,
    make_ports: Box<dyn FnMut() -> Ports>,
    pending: VecDeque<App>,
    /// 非首窗创建失败的用户可见上报回调：产品层据此弹错误框（不只写 stderr）。
    on_error: Box<dyn Fn(&str)>,
    /// 首窗创建失败的致命错误详情：`run` 据此返回 Err，让产品层弹可见错误框而非静默退出。
    startup_error: Option<String>,
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
        }
    }

    fn open_doc(&mut self, event_loop: &ActiveEventLoop, mut doc: Doc) {
        // 只有进程的首个窗口允许消费启动恢复槽；其余窗口都是显式文档。
        doc.resume = false;
        let app = App::new(doc, (self.make_ports)());
        self.create_window(event_loop, app);
    }

    fn create_window(&mut self, event_loop: &ActiveEventLoop, mut app: App) {
        if let Some(id) = app.create_window(event_loop) {
            self.windows.insert(id, app);
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
        let Some(app) = self.windows.get_mut(&id) else {
            return RouteOutcome::default();
        };
        app.handle_window_event(event);
        let new_window = std::mem::take(&mut app.window_state.new_window_requested);
        let closed = destroyed || app.finish_close();
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
        }
    }

    #[cfg(test)]
    pub(crate) fn insert_for_test(&mut self, id: WindowId, app: App) {
        self.windows.insert(id, app);
    }

    #[cfg(test)]
    pub(crate) fn get_for_test(&self, id: WindowId) -> Option<&App> {
        self.windows.get(&id)
    }
}

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
        for app in self.windows.values_mut() {
            if let Some(at) = app.next_wake() {
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
    format!("{base}\n\n请关闭一个窗口后重试；现有文档不会被关闭。")
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
}
