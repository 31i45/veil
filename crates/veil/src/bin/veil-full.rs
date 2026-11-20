//! 编排层 (Orchestration) 入口 — 启动 veil-full.exe (完整版，QuickJS + mermaid.min.js)。
//! veil-full 发行版入口（约 14 MB）：内置 QuickJS + 完整 mermaid.js，
//! Mermaid 图表完全离线渲染（不依赖 mermaid.ink，也不需要联网）。
//! 其余能力与 veil.exe 完全一致（同一套 lib::run）。
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

fn main() {
    veil::run();
}
