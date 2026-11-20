//! 编排层 (Orchestration) 入口 — 启动唯一发行产物 veil.exe。
//! Mermaid 在进程内离线渲染（QuickJS + 完整 mermaid.js），不依赖网络。
//! 发布构建不带控制台窗口（GUI 子系统）；debug 构建保留控制台以便读诊断输出。
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

fn main() {
    veil::run();
}
