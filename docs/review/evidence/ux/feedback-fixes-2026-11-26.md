# 用户反馈缺口修复记录（#7 / #8 / 源码空行）

> 日期：2026-11-26
> 版本：3.0.2
> 范围：仅记录本轮已落地并有自动化证据的三项；真机目检项单独标注。

## 1. 导出 HTML 图片太小（issue #8）

**原因**：`crates/veil/src/export.rs` 的导出 CSS 给正文图加了硬限高：

```css
img{max-width:100%;height:auto;max-height:3.2em}
p>img:only-child,figure img{max-height:9.6em}
```

3.2em 约两行、9.6em 约六行，图片文档导出后几乎不可读。这是刻意与编辑视图对齐的设计取舍，
不是渲染 bug，但对「导出即阅读」的场景明显过小。

**修改**：正文图改为 `img{max-width:100%;height:auto}`，删除 `p>img:only-child,figure img{max-height:9.6em}`；
表格单元格 `td img,th img{max-height:2.4em}` 保留（避免单图撑爆单元格）。

**验证**：`crates/veil/src/export.rs` 断言改为「必须无 max-height、必须有 max-width:100%、表格仍限高」。

## 2. 勾选框选中比未选中大（issue #7）

**核实**：当前 HEAD 的几何从未随勾选变化——`view/layout.rs` 的 `task_box_side = 0.9 × 行高`，
`x/y/side` 两态一致，`layout_inline_tests` 已有同矩形断言。差异来自画法：勾选态整块填充强调色，
实心块在视觉上比细描边更重，看起来「更大」。

**修改**：`paint.rs::draw_task_box` 勾选态不再整块填充，改为「同尺寸、同描边宽度、换描边色 + 叠对勾」；
两态内部都填所在块底色。新增像素测试 `task_box_center_stays_background_when_checked`
断言中心像素两态均为底色、描边色只由勾选态决定。

**结论**：不是布局 bug；旧构建若有真实矩形差异请附版本号，当前 HEAD 由上述测试锁定。

## 3. 源码模式无法显示空行

**原因**：纯文本行视图由 `Config.source_plain_lines` 控制，默认 `false`，只有开设置或
`VEIL_SOURCE_PLAIN=1` 才生效；默认源码模式仍走块布局，空行被块间距吸收，无法停光标。

**修改**：默认值改为 `true`（`theme.rs`）。设置开关保留，显式关闭仍回到旧块视图；
`VEIL_SOURCE_PLAIN=0` 也仍可强制关闭。

**验证**：设置页开关测试改为「默认为开 → Space 关闭 → Enter 打开 → 恢复默认回到开」；
配置解析测试改为「空配置默认开、`false` 解析为关、非法字面量保留默认并计入 ignored」；
7 个锁定旧块视图的源码模式测试显式 `config.source_plain_lines = false`，语义更清楚。

## 4. 测试

- `cargo test -p veil-ui`：1437 passed / 0 failed / 12 ignored。
- `cargo test -p veil`：除既有环回 HTTP 并行 flake 外通过（单独重跑通过）。
- 导出图片的「按版心宽度显示」仍建议真机打开导出文件目检一次；未执行时按 NOT RUN 记录。
