# paint 硬编码审计（Task 4.1 · 第二次）

> 审计范围：`crates/veil-ui/src/` 下所有绘制/布局相关文件
> 审计时间：2026-11-24
> 基线：commit `50bb0ab`
> 口径：与 `tokens::*` / `theme::*` / `palette::*` / `state::*` 不同步的数值字面量

## 一、颜色硬编码（paint_under_decor、遮罩、预览）

### 1.1 `paint.rs::paint_under_decor` — 行内装饰底色
| 位置 | 字面量 | 建议 |
|---|---|---|
| `paint.rs:300` MarkBg | `[255u8, 247, 153, 180]` | **应改为 token 消费** — Markdown 高亮底色，语义颜色，迁入 `tokens::palette` 或 `state` 模块 |
| `paint.rs:304` KbdFrame 外框 | `[160u8, 160, 160, 255]` | **应改为 token 消费** — 与 `chrome_border` 同是 UI 描边色，迁入 palette |
| `paint.rs:312` KbdFrame 内底 | `[245u8, 245, 245, 255]` | **应改为 token 消费** — 与 `code_bg` 同是浅灰底，可迁入 palette |
| `paint.rs:317` ImagePlaceholder 外框 | `[190u8, 190, 190, 255]` | **有理由保留** — 纯 UI 占位提示色（非语义色），且与 `chrome_border` 有意不同（更粗/更脏的灰），迁入 palette 收益不高 |
| `paint.rs:325` ImagePlaceholder 内底 | `[235u8, 235, 235, 255]` | **有理由保留** — 同上，占位语义 |
| `paint.rs:340` StyleBg | `[*r, *g, *b, 255]` | **有理由保留** — 来自 HTML `<span style>` 内联样式颜色，必须透传 |

### 1.2 `view::image_preview` — 图片放大预览遮罩
| 位置 | 字面量 | 建议 |
|---|---|---|
| `image_preview.rs:42` 全窗遮罩 | `[0, 0, 0, 220]` | **有理由保留** — 与模态 `scrim_alpha` 同源但不同用途（图片预览需要更深遮罩让图像凸显），语义自洽 |
| `image_preview.rs:62` 图影 | `[0, 0, 0, 160]` | **有理由保留** — 预览特有视觉效果，非通用浮层 |
| `image_preview.rs:79` 标题前景 | `[240, 240, 240]` | **应改为 token 消费** — 预览标题应消费 `theme.text` 或派生值（深主题不适用，但预览只在有图时出现，一般是全黑遮罩） |
| `image_preview.rs:83` 提示前景 | `[170, 170, 170]` | **应改为 token 消费** — 同上，消费 `theme.muted` |

### 1.3 `modal.rs` — 模态遮罩
| 位置 | 字面量 | 建议 |
|---|---|---|
| `modal.rs:350` | `[0, 0, 0, surface::scrim_alpha(...)]` | ✅ 已改为 token 消费 |

### 1.4 `palette::details_card_bg` — Details 卡片底色
| 位置 | 字面量 | 建议 |
|---|---|---|
| `tokens.rs:246` expanded 浅主题 | `[250, 250, 250, 255]` | **有理由保留** — Details 卡片在浅主题需要与 popup 有最小色差，且这是 palette 模块内部的硬编码（palette 本身就是值模块），合理 |
| `tokens.rs:248` collapsed 浅主题 | `[252, 252, 252, 255]` | 同上 |

### 1.5 `paint.rs::dim` 专注模式
| 位置 | 字面量 | 建议 |
|---|---|---|
| `paint.rs:1513` | `[bg[0], bg[1], bg[2], FOCUS_DIM_ALPHA]` | ✅ 底色已消费 `palette::surface_base`；alpha `FOCUS_DIM_ALPHA = 160` 为文件内 `const`，属语义常量而非散落硬编码 |

## 二、几何硬编码（半径、间距、线宽）

### 2.1 菜单栏/状态栏基本几何 — `chrome/mod.rs`
| 位置 | 字面量 | 建议 |
|---|---|---|
| `MENU_H = 30.0` | 基准高度 | **有理由保留** — chrome 基本几何量，已是 `surface::*` 之外的顶层基本量（不属于浮层几何家族） |
| `STATUS_H = 26.0` | 基准高度 | 同上 |
| `SCROLL_W = 12.0` | 基准宽度 | 同上 |
| `THUMB_MIN_H = 24.0` | 最小高度 | ✅ 已在 `surface::scrollbar_min_thumb` 有镜像 |

### 2.2 菜单布局 — `chrome/mod.rs`
| 位置 | 字面量 | 建议 |
|---|---|---|
| `menu_bar_rects` 内 `pad = 14.0 * scale` | 菜单标题左右留白 | **有理由保留** — 菜单栏是 chrome 顶层，不走浮层 surface 令牌；值固定且为 4 的倍数附近（14 ≈ 12 + 2），合理 |
| `menu_bar_rects` 内 `8.0 * scale` | 起始 x | **有理由保留** — 同上，可迁入 `space::S` 但当前合理 |
| `popup_layout` 内 `gap = 24.0 * scale` | 快捷键列宽 | **有理由保留** — 可迁入 `space::XL`，但语义独立（文本测量配合） |
| `popup_layout` 内 `top_pad = 4.0 * scale` | 菜单顶部内边距 | ✅ 已和 `space::XS` 同值 |
| `popup_layout` 内 `POPUP_MAX_W = 420.0` / `POPUP_MIN_W = 150.0` | 菜单宽限 | **有理由保留** — 逻辑边界常量，非视觉 token |

### 2.3 滚动条 — `chrome/paint.rs`
| 位置 | 字面量 | 建议 |
|---|---|---|
| `tw = (SCROLL_W - space::XS) * scale` | 滑块宽 | ✅ 已消费 `space::XS` |
| `x + 2.0 * scale` | 轨道内缩 | **应改为 token 消费** — 可迁入 `surface` 或 `space`（比如 space::XS / 2 但要取整） |
| 滑块圆角 `tw / 2.0` | 胶囊形 | **有理由保留** — 胶囊 = 宽的一半，是几何派生而非硬编码值 |

### 2.4 Modal — `view/modal.rs`
| 位置 | 字面量 | 建议 |
|---|---|---|
| `BUTTON_PAD_X = 12.0` | 按钮文字左右留白 | **有理由保留** — 纯按钮排版常量；12 不在 4 倍数刻度但语义独立（按钮舒适留白） |
| `paint_error_icon` 内 `w*0.11` / `h*0.42` / `h*0.22` / `h*0.10` | 徽标内感叹号几何 | **有理由保留** — 几何派生（按卡片图标大小等比），非散落硬编码 |
| `focus_ring` 内 `radius-1.0` | 焦点环内缩 | **有理由保留** — 几何派生 |

### 2.5 大纲面板 — `paint_overlay.rs::paint_outline`
| 位置 | 字面量 | 建议 |
|---|---|---|
| `row.h - 12.0 * scale` | 当前项指示条高度扣减 | **有理由保留** — 行高的派生量，12 = UI_SIZE - 1（或类似） |
| `4.0 * scale` | 指示条左内缩 | ✅ 已和 `space::XS` 同值 |
| `3.0 * scale` | 指示条宽 | **有理由保留** — 近 space::XS/2，合理 |
| `12.0 * scale` | 时间标签上边距 | **应改为 token 消费** — 可迁入 `space::M` |
| `14.0 * scale` | 时间标签下边距 | **有理由保留** — 近似 space::M |
| 滚动条 `6.0 * scale` / `3.0 * scale` | 大纲轨道 | **应改为 token 消费** — 消费 surface::SCROLLBAR_MIN_THUMB |

### 2.6 代码块头部 — `paint.rs::paint_code_header`
| 位置 | 字面量 | 建议 |
|---|---|---|
| `(3.0 * scale).max(1.0)` | 按钮圆角 | **应改为 token 消费** — 可迁入 `surface::CORNER_RADIUS` 或按钮专用 |
| `8.0 * scale` | 按钮与标签间距 | ✅ 已和 `space::S` 同值 |

### 2.7 链接浮层 — `paint.rs`
| 位置 | 字面量 | 建议 |
|---|---|---|
| `LINK_TIP_EDGE = 8.0` | 浮层边留白 | ✅ 已和 `space::S` 同值 |
| `LINK_TIP_GAP = 24.0` | 链接锚到浮层间距 | **应改为 token 消费** — 可迁入 `space::XL` |

### 2.8 Callout 徽标 — `paint.rs`
| 位置 | 字面量 | 建议 |
|---|---|---|
| `12.0 * scale` | 徽标大小 | **应改为 token 消费** — 可迁入 `surface` 模块（与 modal icon 独立） |
| `2.0 * scale` | 徽标左偏移 | **应改为 token 消费** — 可迁入 `space::XS` |
| `2.5 * scale` | 徽标圆角 | **有理由保留** — 几何派生（约 size/5） |

### 2.9 Details 折叠箭头 — `paint.rs`
| 位置 | 字面量 | 建议 |
|---|---|---|
| `6.0 * scale` | 箭头边长 | **有理由保留** — 近 space::XS + 2，合理 |

### 2.10 专注模式压暗 — `paint.rs`
| 位置 | 字面量 | 建议 |
|---|---|---|
| `FOCUS_DIM_ALPHA = 160` | 压暗强度 | **应改为 token 消费** — 迁入 `state` 模块（专注模式属于交互态） |

### 2.11 拖放压暗 — `paint.rs`
| 位置 | 字面量 | 建议 |
|---|---|---|
| `DROP_DIM_ALPHA = 120` | 压暗强度 | **应改为 token 消费** — 同上，迁入 `state` |

## 三、Chrome 几何顶层常量 — `crates/veil/src/chrome.rs`
该文件已不存在（Chrome 模块已迁入 `crates/veil-ui/src/chrome/`），跳过。

## 四、审计汇总

| 类别 | 总数 | 应改 token | 有理由保留 |
|---|---|---|---|
| 颜色 | 14 | 5 | 9 |
| 几何 | 31 | 7 | 24 |
| **合计** | **45** | **12** | **33** |

### 优先迁移项（3 个批次）

**批次 A（低风险·本轮可做）：**
1. `paint.rs::paint_under_decor` MarkBg/KbdFrame 外框/内底 → `tokens::palette`
2. `paint.rs::FOCUS_DIM_ALPHA` / `DROP_DIM_ALPHA` → `tokens::state`（专注/拖放属于交互态）
3. `image_preview.rs` 标题/提示前景 → `theme.text` / `theme.muted`

**批次 B（几何收敛·后续）：**
1. `paint.rs` 链接浮层几何 → `tokens::space`
2. `paint.rs` Callout 徽标几何 → `tokens::surface`
3. 滚动条内缩 → `tokens::surface`

**批次 C（架构整理·后续）：**
1. `chrome/mod.rs` 菜单起始 x 等 → `tokens::space`
2. `modal.rs::BUTTON_PAD_X` → `tokens::surface::dialog_*`
