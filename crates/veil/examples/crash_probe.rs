//! 崩溃取证自检样本（不进入发布包）：安装 hook、记录两条事件后强制 panic。
//!
//! 运行：cargo run -p veil --release --example crash_probe
//! 预期：进程非零退出，`data_dir()/logs/`（默认 `%APPDATA%\Veil\logs`，
//! 可用 `VEIL_HOME` 覆盖）出现 `veil-crash-*.txt`，hook 会把路径打到 stderr。

fn main() {
    veil::diag::install();
    veil_ui::diag::record("probe", "crash probe start");
    veil_ui::diag::record("probe", r"about to panic at C:\Users\alice\secret\a.md");
    match veil::diag::log_dir() {
        Some(dir) => eprintln!("log dir: {}", dir.display()),
        None => eprintln!("log dir: <unavailable>"),
    }
    panic!("veil crash probe");
}
