# ADR-004: 平台接缝机制 —— 条件编译同名模块

## 状态：已采纳

## 背景

INSTANCE §7 识别出五个平台差异点：窗口材质（Windows Mica）、系统主题跟随、剪贴板、文件监听、IME。需要决定：条件编译 vs trait 对象（运行时分发）vs 适配器层。

## 决策

采用**条件编译 + 同名同签名模块**，落点 `crates/veil/src/platform/`：

```
platform/
  mod.rs          // #[cfg(target_os = "...")] mod imp; pub use imp::*;
  windows.rs    // pub fn apply_mica(...) 等同名函数
  macos.rs      // （v1 不实现，占位即编译错误，强制显式决策）
  linux.rs      // 同上
```

规则：上层调用 `platform::apply_mica(...)` 等统一入口，**零 `#[cfg]` 出现在 platform/ 之外**；各平台实现函数签名逐字一致。

## 后果

- 好：零运行时开销；v1 只写 Windows 实现时，macos.rs/linux.rs 保持「未实现即编译失败」状态，防止「伪跨平台」；签名一致性由编译器强制。
- 坏：新增平台时必须修改 platform/ 目录（可接受，且正是接缝设计的意图——平台扩展收敛在唯一目录）。

## 替代方案

- trait 对象运行时分发：引入不必要的动态分发与对象安全约束；平台差异是编译期已知事实，运行时分发无收益，否决。
- 各调用点就地 `#[cfg]`：平台分支散布全代码库，违反「上层零平台分支」不变量，否决。

## 备注

`veil-core` 不在接缝范围内——它零平台代码，纯逻辑（INSTANCE §7）。IME 的自绘 preedit 路径是 winit 事件直收，不属于平台差异（winit 已抽象跨平台 IME），故不进入 platform/ 模块，而是 veil-egui 的输入管线职责。
