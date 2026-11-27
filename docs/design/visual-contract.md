# 视觉契约 v1

- 版本：v1（2026-11-27）
- 状态：生效，后续所有视觉改动的唯一裁决依据
- 适用范围：屏幕渲染（`crates/veil-ui`）、HTML/PDF 导出（`crates/veil`）
- 数值依据文件：`crates/veil-ui/src/tokens.rs`、`crates/veil-ui/src/theme.rs`、`crates/veil-ui/src/view/style.rs`、`crates/veil-ui/src/view/layout.rs`、`crates/veil/src/platform/pdf_layout.rs`、`crates/veil-ui/src/theme_matrix_tests.rs`

本契约只写可执行规则，不写风格描述。凡与本文冲突的视觉实现，以本文为准；若要偏离，先改本文并记录理由，再改代码。

## 文首：四项决策

| 决策项 | 结论 | 理由 |
|---|---|---|
| Sepia | 不做 | 需先证明它是「同一张纸的第三种质感」而非第三套皮肤。 |
| 高对比度主题 | 保留，按本契约重做 | 它是系统集成入口，但现状是一套独立高饱和色，需纳入统一目标矩阵。 |
| H4–H6 | 单一基准比值表 + 通道夹取 | 屏幕与纸面各写一套会使导出版式与屏幕版式长期漂移，必须同源派生、按通道夹取。 |
| 应用内图标 | 不建图标系统，仅允许单色语义徽标，16/18 网格（执行条款见 §九） | 图标系统会引入第三套几何与色相，当前只需错误/警告等少量语义徽标，单色 + 固定网格即可。 |

---

## 一、色彩层级预算

**作用域**：正文视口（`theme.bg` 以上的文档内容区），不含 chrome 条带、浮层与模态。

**一屏非灰颜色上限**：

| 预算 | 上限 | 允许项 |
|---|---|---|
| 正文区彩色 | ≤ 3 类 | 链接/强调色（`theme.link` 与 `theme.accent` 同属一族，计 1 类）、危险色（`theme.danger`）、标记色（`theme.mark_bg`） |
| 代码块内彩色 | 独立预算 7 类 | 语法 token：`keyword` / `string` / `comment` / `number` / `function` / `type_name` / `punctuation`（`theme::SyntaxColors` 的全部字段） |

规则：

1. 代码块内的 7 类语法色不占用正文区的 3 类预算，两块预算互不叠加、互不挪用。
2. 灰阶、surface 底色、状态遮罩（`sel` / `hover`）、`kbd` 与表格网格不计入彩色预算。
3. **颜色只用于区分层级，不做装饰。** 任何「为了好看」而新增的彩色即为违规；新增彩色必须同时说明它区分的是哪两个层级。
4. 同一层级在同一屏内只允许一种颜色表达；`link` 与 `accent` 在浅色下同为 `[43,108,176]`，属同一族，不得视为两类。

## 二、灰阶与色相

**基调**：浅色为纸白基调，深色为炭灰基调，高对比度为纯黑基调。三者共用同一套中性灰阶，不各立一套。

现状（`crates/veil-ui/src/theme.rs`）：

| 角色 | 浅色 | 深色 | 高对比度 |
|---|---|---|---|
| `bg` | `[255,255,255]` | `[30,30,30]` | `[0,0,0]` |
| `chrome_bg` | `[248,248,248]` | `[38,38,38]` | `[15,15,15]` |
| `code_bg` | `[245,245,245]` | `[40,40,40]` | `[20,20,20]` |
| `table_head_bg` | `[246,248,250]` | `[42,42,42]` | `[30,30,30]` |
| `popup_bg` | `[255,255,255]` | `[45,45,45]` | `[10,10,10]` |
| `modal_bg` | `[249,250,252]` | `[49,49,49]` | `[20,20,20]` |
| `grid` | `[136,141,148]` | `[112,116,122]` | `[180,180,180]` |
| `chrome_border` | `[208,213,221]` | `[60,64,70]` | `[255,255,255]` |
| `thumb` | `[176,184,195]` | `[96,102,110]` | `[200,200,200]` |

**统一灰阶档位（L\* 明度，%）**：

```
核心档：0 / 2 / 4 / 8 / 12 / 16 / 24 / 32 / 48 / 56 / 68 / 76 / 84 / 88 / 92 / 96 / 100
补档：  20   （深色 surface 家族必需：modal 现值 L* ≈ 20.3，落在 16 与 24 之间无法归位）
```

规则：

1. 后续新增或调整的**中性色必须从上述档位取值**，禁止新增手填 RGB 字面量。
2. 现有数值按 L\* 就近登记到档位（浅色：`bg`→100、`chrome_bg`→97.6、`code_bg`→96.5；深色：`bg`→12、`chrome_bg`→15.2、`code_bg`→16.1、`popup_bg`→18.5、`modal_bg`→20.3）。逐项落表的迁移在 W3 执行；本契约生效后**只禁止新增未登记字面量**，不要求一次性重排现值。
3. 色相只允许两类：严格中性（R=G=B），或单一的冷调偏移族（蓝分量最高）。禁止暖偏中性色与多彩中性色。
4. 冷调偏移以现状最大值为上限：`thumb` 浅色 `B−R = 19`；`chrome_border` 浅色 `B−R = 13`；`grid` 浅色 `B−R = 12`；`modal_bg` 浅色 `B−R = 3`；`table_head_bg` 浅色 `B−R = 4`。新增中性色不得超过 `B−R = 19`。
5. 层级分离：`modal_bg ≠ popup_bg ≠ chrome_bg`，两两可被人眼区分（见 §七 的分离口径）。

## 三、字号与行高比值表

### 3.1 基准字号（`tokens::typography`）

| 通道 | 字号 | 行高 | 行高/字号 | 现状来源 |
|---|---|---|---|---|
| 正文 | 16.0 | 22.0 | 1.375 | `BODY_SIZE` / `BODY_LINE_HEIGHT` |
| UI（chrome / 浮层 / 模态正文） | 13.0 | 18.0 | 1.385 | `UI_SIZE` / `UI_LINE_HEIGHT` |
| 代码 | 14.0 | 19.25 | 1.375 | `CODE_SIZE`；行高按 `CODE_SCALE = 14/16 = 0.875` 由正文同比派生（22 × 0.875） |
| 模态标题 | 15.0 | 22.0 | 1.467 | `DIALOG_TITLE_SIZE` / `surface::DIALOG_TITLE_LINE_HEIGHT` |

规则：行高只能由字号乘上述固定比值得到，禁止逐块另设行距。

### 3.2 标题基准比值表（唯一基准，冻结）

标题一律以**正文字号为基准**乘比值，不占 `typography` 的 UI 字阶。

| 级别 | 基准比值 | 纸面字号（× 11.0） | 屏幕夹取上限 | 屏幕比值 | 屏幕字号（× 16.0） |
|---|---|---|---|---|---|
| H1 | 2.00 | 22.0 | 1.6 | 1.6 | 25.6 |
| H2 | 1.64 | 18.0 | 1.4 | 1.4 | 22.4 |
| H3 | 1.41 | 15.5 | 1.2 | 1.2 | 19.2 |
| H4 | 1.27 | 14.0 | 1.1 | 1.1 | 17.6 |
| H5 | 1.18 | 13.0 | 1.1 | 1.1 | 17.6 |
| H6 | 1.14 | 12.5 | 1.1 | 1.1 | 17.6 |

**唯一基准比值表 + 通道夹取规则**：

```
paper_ratio(level)  = 基准比值(level)                      // 纸面：全额使用
screen_ratio(level) = min( 基准比值(level), cap(level) )     // 屏幕：按级别上限夹取
cap = [1.6, 1.4, 1.2, 1.1, 1.1, 1.1]（H1..H6）
```

- 纸面保留六档细分（`crates/veil/src/platform/pdf_layout.rs` 的 `heading_size`）。
- 屏幕把 H1–H6 逐级夹到 `cap`（`crates/veil-ui/src/view/layout.rs` 的 `block_scale`，SetextHeading(1)/(2) 与 AtxHeading(1)/(2) 同级）。
- **两处必须从同一张基准表派生，禁止各写一套。** 现状两侧仍是字面量，属实现欠账：后续改为共享基准表 + 各自夹取，结果必须与本表逐格一致。

## 四、线宽语义族

线宽只有三档，全部承载语义，不做自由取值。

| 档 | 值（1.0× 逻辑 px） | 令牌 | 唯一语义 |
|---|---|---|---|
| hairline | 1.0 | `tokens::stroke::HAIRLINE` | 分隔线、表格网格、边框、`kbd` 描边、文字装饰线（下划/上划/删除/虚线） |
| caret | 2.0 | `tokens::surface::CARET_WIDTH` | 文本插入光标 |
| rail | 3.0 | `tokens::doc::QUOTE_RAIL` | 引用块左竖条、详情竖条 |

规则：

1. 每档只能用于其语义，不得跨用（例：不得用 2px 画分隔线，不得用 1px 画引用竖条）。
2. 禁止新增自由线宽；任何新线条必须从 1 / 2 / 3 中选一，并写明语义。
3. 缩放后必须保底 1 物理像素（现状在绘制点以 `.max(1.0)` 保证，见 `crates/veil-ui/src/paint.rs`），且须吸附到整数物理像素（`tokens` 的像素对齐测试）。

## 五、alpha 阶梯

所有半透明色的 alpha 分量只能取下列离散档（百分数 → alpha 字节 = `round(p × 255 / 100)`）：

| 档位 | alpha 字节 | 语义用途 |
|---|---|---|
| 8% | 20 | 最轻底色偏移（极浅 tint） |
| 12% | 31 | 悬停/焦点的最轻档 |
| 20% | 51 | 选中底色轻档 |
| 28% | 71 | 选中底色、遮罩最低档 |
| 40% | 102 | 遮罩、深色选区 |
| 55% | 140 | 半透明强调、mark 底轻档 |
| 70% | 179 | mark 底、高存在感半透明 |
| 100% | 255 | 不透明 |

规则：

1. 所有 `[u8; 4]` 的 alpha 必须来自上表，禁止任意值。
2. 当前散落值收敛目标（就近取档，收敛后须复跑 §七 门禁）：

| 现值 | 出处 | 百分比 | 收敛档 |
|---|---|---|---|
| 80 | `sel`（浅色）、`SCRIM_ALPHA_LIGHT` | 31.4% | 28% |
| 110 | `sel`（深色） | 43.1% | 40% |
| 120 | `SCRIM_ALPHA_DARK` | 47.1% | 40% |
| 170 | `state::focus_bg` | 66.7% | 70% |
| 180 | `mark_bg`（浅色） | 70.6% | 70% |
| 210 | `mark_bg`（深色） | 82.4% | 70%（待复验；若 §七 对比度不足则取 100%） |

3. 阴影层的 alpha（`surface::SHADOW_LAYERS`）是叠层累积值，不属本阶梯，保持现状。

## 六、装饰白名单与冲突裁决

### 6.1 白名单（`crates/veil-ui/src/view/style.rs` 的 `Style` 全部 15 个样式位）

| 字段 | 类型 | 层 | 语义 | 屏幕 | 导出 |
|---|---|---|---|---|---|
| `bold` | bool | 字形 | 加粗 | 绘制 | 输出 |
| `italic` | bool | 字形 | 斜体 | 绘制 | 输出 |
| `mono` | bool | 字形 | 等宽族 | 绘制 | 输出 |
| `link` | bool | 前景 | 链接色（不自动加下划线） | 绘制 | 输出 |
| `strikethrough` | bool | 装饰线 | 删除线（x-height 中点） | 绘制 | 输出 |
| `color` | `Option<[u8;3]>` | 前景 | 显式前景色（语法高亮 / HTML color） | 绘制 | 输出 |
| `size_factor` | `Option<f32>` | 几何 | sub/sup 缩字（0.75） | 生效 | 输出 |
| `baseline_shift` | `Option<f32>` | 几何 | sub/sup 基线偏移（±1em） | 生效 | 输出 |
| `mark_bg` | bool | 底层 | mark 底色（取 `theme.mark_bg`） | 绘制 | 输出 |
| `dotted_underline` | bool | 装饰线 | abbr 虚线下划线 | 绘制 | 输出 |
| `kbd_frame` | bool | 底层 | kbd 底 + 1px 描边 | 绘制 | 输出 |
| `code_bg` | bool | 底层 | 行内代码圆角底色 | 绘制 | 输出 |
| `bg` | `Option<[u8;3]>` | 底层 | 作者 `background-color` | **待裁决**（见 6.3） | 保留 |
| `underline` | bool | 装饰线 | 实线下划线 | 绘制 | 输出 |
| `overline` | bool | 装饰线 | 上划线 | 绘制 | 输出 |

白名单外不得新增样式位；任何新装饰必须先进本表再实现。

### 6.2 叠加确定顺序（back → front，唯一顺序）

1. 几何解析：`size_factor` → `baseline_shift`（在 shaping 前生效，不产生绘制）
2. `code_bg`
3. `mark_bg`
4. `kbd_frame`（先 `kbd_bg` 填充，再 1px 描边）
5. `bg`（作者底色；按 6.3 处置）
6. 字形光栅：`mono` → `italic` → `bold`（族选择/字形选择，可叠加）
7. `strikethrough`
8. `dotted_underline`
9. `underline`
10. `overline`

顺序依据现状 `crates/veil-ui/src/view/layout_engine.rs`（`decorations_under` 依次压入 code_bg → mark_bg → kbd_frame → bg；`strikes` 先于 `decorations_over`；`decorations_over` 依次为 dotted_underline → underline → overline）与 `crates/veil-ui/src/paint.rs`（strikes 先画，decorations_over 后画）。

冲突裁决：

| 情形 | 裁决 |
|---|---|
| `color` 与 `link` 同时为真 | `color` 优先；`link` 仅在 `color` 为 `None` 时改前景色 |
| `color` / `link` 均缺省 | 用 `theme.fg`（表格内用 `theme.text`） |
| `mark_bg` 与 `code_bg` 同时为真 | 两者都画，按 2→3 顺序；`mark_bg` 覆盖 `code_bg` |
| `kbd_frame` 与任意底层同时为真 | `kbd_frame` 覆盖其下所有底层 |
| `underline` 与 `dotted_underline` 同区间 | 实线后画（第 9 步）覆盖虚线（第 8 步），两线同基线位；禁止调换顺序 |
| `overline` / `underline` / `strikethrough` 同时 | 三条线各自绘制，互不抑制，位置固定（上划线在字顶、下划线在基线下 1px、删除线在 x-height 中点） |
| 装饰线与 `size_factor` / `baseline_shift` | 装饰线随字形度量走，不另立偏移 |
| 任意叠加 | 不得产生 §四 之外的线宽，不得产生 §五 之外的 alpha |

### 6.3 `Style.bg` 的唯一处置规定

- **规定**：仅导出侧保留（HTML / PDF 输出 `background-color`），屏幕侧不绘制、不参与主题取色、不进入 §七 对比度矩阵。
- **现状**：`layout_engine.rs` 仍把 `StyleBg` 压在最上层绘制，与本规定不一致。
- **标注**：该项列为**待 W3 裁决项**。若 W3 裁定屏幕保留，则 `bg` 必须补 alpha 阶梯取值，并纳入 §七 的正文可读性校验（前景/背景 ≥ 7:1，否则降级为 mark 级 4.5:1 且不得与 `mark_bg` 叠加使用）。

## 七、对比度目标矩阵

**测量口径**：

- 对比度：WCAG 2.x 相对亮度比 `(L1+0.05)/(L2+0.05)`，sRGB→线性用 `c ≤ 0.03928 ? c/12.92 : ((c+0.055)/1.055)^2.4`，系数 `0.2126/0.7152/0.0722`（`crates/veil-ui/src/theme_matrix_tests.rs` 已按此实现）。
- 半透明色先做 src-over 合成到实际背景，再测合成结果。
- 层级分离：OKLab 欧氏色差 ΔE_OK（含明度与色度三轴），CIELAB ΔE 2 ≈ ΔE_OK 0.02。现状已实现（`crates/veil-ui/src/tokens.rs` 的 `oklab` / `de_ok`，门禁 `surface_hierarchy_is_perceptually_separated`：card–popup ≥ 0.02，modal 与 popup/card 之一 ≥ 0.01）。若该实现将来被回退为仅比明度差的 ΔL 版本，则按**待修正项**处理，不得以 ΔL 冒充感知色差。

| 类别 | 目标 | 现状 / 门禁 |
|---|---|---|
| 正文文字 / 底色 | ≥ 7:1 | 浅色约 16.7:1、深色约 12.2:1，实测达标；高对比度已有 7:1 断言（`high_contrast_meets_readability_floor`），浅/深色仅经 4.5 档间接校验，**需补 7:1 断言** |
| 次要文字（`muted`）/ 底色 | ≥ 4.5:1 | 高对比度已断言，浅/深色**待补** |
| 语法 token / `code_bg` | ≥ 4.5:1 | 已加入 3 主题 × 7 token 门禁（`theme_matrix_tests` 的 `syntax_tokens_meet_contrast_on_code_bg`）；该门禁缺失时按**待补**处理 |
| 链接 / 底色 | ≥ 4.5:1 | 高对比度已断言，浅/深色**待补** |
| mark 上正文 | ≥ 4.5:1 | 已断言通过 |
| `kbd` 文字 / `kbd_bg` | ≥ 4.5:1 | 已断言通过 |
| 表格线（`grid`）/ 底色 | ≥ 3:1 | 浅色约 3.4:1、深色约 3.4:1，已通过 |
| 滚动条拇指（`thumb`）/ 底色 | ≥ 3:1 | 浅色约 2.2:1、深色约 2.7:1，**未达标，待修正** |
| 边框（`chrome_border`）/ 底色 | ≥ 3:1 | 浅色约 1.5:1，**未达标，待修正** |
| 装饰性占位（图片占位）/ 底色 | ≥ 1.4:1 | 已断言通过（1.4 口径） |

规则：

1. 高对比度主题同样进入本矩阵，按同口径重做，不得以「高饱和」豁免任何一行。
2. 同一元素在多主题下逐一测，不得只测浅色。
3. 表格线压表头、正文压各类 surface 等叠加场景，测合成后的呈现色，不测声明色。
4. 现状未达标的行（`thumb`、`chrome_border`）在修正前不得被新代码当作「已达标的参考值」引用。

## 八、冻结量

以下三项为冻结值：任何改动都必须重跑全部门禁（`cargo test --workspace` 的视觉用例，含 `theme_matrix_tests`、`tokens` 像素对齐与字号测试、`pdf_layout` 标题字号测试、`docs/qa/golden/html` 字节比对），并在提交说明中写明重跑结果。

| 冻结项 | 值 | 位置 |
|---|---|---|
| `code_bg` 浅色 | `[245,245,245,255]` | `crates/veil-ui/src/theme.rs`（`Theme::light`） |
| `code_bg` 深色 | `[40,40,40,255]` | `crates/veil-ui/src/theme.rs`（`Theme::dark`） |
| `QUOTE_RAIL` | `3.0` | `crates/veil-ui/src/tokens.rs`（`doc::QUOTE_RAIL`） |
| 标题比值表 | §3.2 全表（基准比值 H1–H6 = 2.00 / 1.64 / 1.41 / 1.27 / 1.18 / 1.14，屏幕夹取上限 = 1.6 / 1.4 / 1.2 / 1.1 / 1.1 / 1.1） | `docs/design/visual-contract.md` §3.2（实现位于 `view/layout.rs`、`platform/pdf_layout.rs`） |

冻结解释：

- `code_bg` 两值与页面底（浅色 255、深色 30）的差异是「行内代码成块可辨」的最低条件，不得为统一灰阶而收窄。
- `QUOTE_RAIL = 3.0` 是 §四 rail 档的唯一实现，不得并入 hairline 或 caret。
- 标题比值表是 §3.2 那张；改一格即视为改契约，必须先修订本文版本号，再改两侧实现并重跑门禁。

## 九、图标规则

文首「应用内图标 = 不建图标系统，仅单色语义徽标，16/18 网格」的可执行条款：

1. **不引入图标资产**：不得引入图标字体（icon font）、第三方图标集（Lucide / Heroicons / Fluent 等）、SVG 图标包或位图图标文件；不得为图标新增依赖。应用内一切图形只能由现有像素原语（圆角矩形、矩形、线段）绘制。
2. **只允许语义徽标**：图标只能表达状态语义（错误 / 警告 / 信息），不得用于装饰、按钮配图或替代文字标签；徽标旁必须保留文字说明，不得出现「只有图标没有文字」的信息。
3. **单色线稿**：一个徽标只允许一种语义色——取 `theme.danger` / `theme.muted` / `theme.fg` 之一，内部笔画可用其反色；禁止多色、渐变、投影与纯装饰填充。
   - 现状：`crates/veil-ui/src/view/modal.rs::paint_error_icon` 是「danger 实心圆 + 反色感叹号」，属单色语义徽标但**不是线稿**，列为待收敛项；收敛前不得再新增第二种徽标形态。
4. **尺寸只允许 16 / 18**：两种逻辑像素边长（1.0× 基准，随 scale 线性放大）。18 用于与模态标题同高的徽标（`surface::DIALOG_ICON_SIZE = 18`，且 `≥ typography::DIALOG_TITLE_SIZE`）；16 预留给与 UI 字号同高的行内位置（当前无实现）。禁止其他尺寸，也禁止非方形网格。
5. **线宽只取 §四 三档**：默认 hairline（1.0，`tokens::stroke::HAIRLINE`）；需要强调笔画时最多 caret（2.0，`surface::CARET_WIDTH`）；rail（3.0）不得用于图标。禁止为图标新增自由线宽。
6. **图标不得驱动布局**：徽标尺寸与间距只允许 `surface::DIALOG_ICON_SIZE` 与 `surface::DIALOG_ICON_GAP` 两个令牌；不得用图标宽度决定行高、按钮宽或栅格，图标必须装入其所在行/控件的既有几何。

