# v1.2 真机走查记录（2026-11-09）

构建：`target\release\veil.exe`，6,043,136 B。环境：Windows，DPI 150%，窗口客户区宽度按 1440 / 1920 / 2560 设置。
采集：`tools/walkthrough.ps1`（PrintWindow 不可用时回退屏幕捕获）。

## 截图与结论

| 文件 | 场景 | 结论 |
|---|---|---|
| `1440-window.png` | 1440 px | 未达 1600 px 阈值，列宽上限不启用，行为与改动前一致 |
| `1920-window.png` | 1920 px | 进入上限：正文列约 40 全角当量，左右留白对称 |
| `2560-window.png` | 2560 px | 列宽保持上限，留白增大；H4/H5/H6 递增缩进可辨；标题段距明显大于正文段距 |
| `alt-up.png` | `Alt+↑` | 第 2 行移到第 1 行前，光标随行，单 undo 单元 |
| `alt-down.png` | `Alt+↓` | 第 1 行移到第 2 行后，**画面即时刷新**（见下方缺陷） |
| `undo-after-move.png` | `Ctrl+Z` | 恢复原顺序，布局同步刷新 |
| `page-up.png` | `PgUp` | 视口上翻一页，光标落新视口内（行 3、列 13） |
| `page-down.png` | `PgDn` | 视口下翻一页，光标落新视口内 |
| `ctrl-delete.png` | `Ctrl+Delete` | 行首整词删除（`line01` 删除剩 ` alpha`），不跨行 |
| `find-case-toggle.png` | `Ctrl+F` + 输入 | 查找栏含 `Aa` 开关与 `n/m` 计数 |
| `replace-enter.png` | `Ctrl+H` → 查询 → `Tab` → 替换 → `Enter` | 命中 1/1，`line05` → `LINE05X` 并前进；纯键盘可完成 |

## 走查中发现并修复的缺陷

`Alt+↓` 首次实测时文档已改但画面滞后，直到下一次编辑才刷新。根因：行移动只置换行内容，块 id 与行区间均不变，而布局缓存键不含内容指纹，命中旧布局。修复：行移动后递增会话代次（`interaction.rs`），并补回归用例 `line_move_refreshes_layout_without_further_edit`。修复后 `alt-down.png` 即时刷新。

## 采集偏差说明

截图中窗口左右边缘出现的竖排碎片是**窗口外的桌面内容**：该环境下 `PrintWindow` 返回空，脚本回退为屏幕区域捕获，区域含窗口边框外数像素。放大左上角可见圆角边框把碎片隔在窗外，非应用绘制缺陷。
