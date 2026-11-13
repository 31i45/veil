//! 进程级窗口管理器：每个 `App` 仍只负责一个窗口，`Shell` 按 `WindowId` 路由事件。

use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::Receiver;
use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::WindowId;

use crate::app::{App, Doc, Ports};
use crate::i18n::{tf, Key};

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
}

impl Shell {
    fn new(doc: Doc, mut make_ports: Box<dyn FnMut() -> Ports>) -> Self {
        let initial = App::new(doc, make_ports());
        Self {
            windows: HashMap::new(),
            make_ports,
            pending: VecDeque::from([initial]),
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
        } else if self.windows.is_empty() && self.pending.is_empty() {
            event_loop.exit();
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
        Self {
            windows: HashMap::new(),
            make_ports: Box::new(|| unreachable!("测试不创建额外窗口")),
            pending: VecDeque::new(),
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

/// 启动多窗口事件循环。每个窗口调用一次端口工厂，避免共享不可克隆的 trait object。
pub fn run<F>(doc: Doc, make_ports: F, requests: Option<Receiver<Doc>>) -> Result<(), String>
where
    F: FnMut() -> Ports + 'static,
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
    let mut shell = Shell::new(doc, Box::new(make_ports));
    event_loop
        .run_app(&mut shell)
        .map_err(|e| tf(Key::ErrEventLoopRun, &[&e.to_string()]))
}
