//! 编排层 (Orchestration) 入口 — 启动 veil.exe (轻量版)。
//! veil 发行版入口（约 9 MB）：Mermaid 走 mermaid.ink 远程 SVG + 本地 resvg 光栅化。
//! 发布构建不带控制台窗口（GUI 子系统）；debug 构建保留控制台以便读诊断输出。
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

fn main() {
    veil::run();
}
