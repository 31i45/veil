# 配置读写 round-trip 证据（Phase 3 / Task 6）

> 日期：2026-11-26
> HEAD：`71cb42a`；版本：`3.0.2`
> 范围：`config.toml` 的读取路径与写回路径语义一致性（不改产品行为）
> 结果：**4 个文件级 round-trip 测试全部通过，未发现行为漂移，无产品代码改动**

## 1. 复现

```powershell
cargo test -p veil-ui config_roundtrip
```

结果：

```text
running 4 tests
test theme::tests::config_roundtrip_all_fields_through_file ... ok
test theme::tests::config_roundtrip_edit_then_reload_preserves_semantics ... ok
test theme::tests::config_roundtrip_numeric_edges_through_file ... ok
test theme::tests::config_roundtrip_legacy_section_bom_crlf_keeps_semantics ... ok

test result: ok. 4 passed; 0 failed
```

## 2. 实际路径

| 环节 | 位置 | 说明 |
|---|---|---|
| 读取 | `crates/veil-ui/src/theme.rs` `parse_config` / `decode_config` | 手写解析；`collect_entries` 优先 `toml_edit` AST，失败回退逐行 key=value；段标题下的键拉平到顶层 |
| 写回 | `theme.rs` `Config::to_toml` / `to_toml_preserving` / `edit_existing_toml` | 新文件整文件生成；已有严格 TOML 且无表段时 `toml_edit` 原地编辑（保留注释/未知键/键序） |
| 产品落盘 | `crates/veil/src/platform/mod.rs` `FsConfig::load/save`（L142-176） | 读取 `read` + `from_utf8_lossy`；写入同目录 `.veil-tmp` + rename 原子写 |
| 产品编排 | `crates/veil-ui/src/app.rs` 启动 L1043-1057、`save_config` L1327-1356 | 启动 `decode_config`；保存先读最新文件合并，只覆盖本动作负责字段 |

## 3. 覆盖字段

共 15 个 `Config` 字段 + 4 个窗口键：

| 类别 | 键 |
|---|---|
| 主题 / 语言 | `mode`、`ui_language` |
| PDF 导出 | `export_margin_mm`、`export_page_numbers`、`export_toc` |
| 正文排版 | `doc_font_family`、`doc_font_face`、`doc_font_size`、`doc_line_height` |
| 版心 | `doc_margin`、`doc_margin_px` |
| 行为开关 | `allow_remote_images`、`source_plain_lines`、`tab_width` |
| 窗口几何 | `window_width`、`window_height`、`window_x`、`window_y`（成组） |
| 历史/已移除键 | `image_cache_limit_mb`、`smooth_scroll`、`mermaid_insert_url`、`mermaid_render`、`[theme]` 段标题 |

测试设计：

1. `config_roundtrip_all_fields_through_file`：逐个列出全部 15 字段（不用 `..Default`），构造 → 写临时 `config.toml` → `fs::read` + `decode_config` → 全字段 `PartialEq` 恒等，`ignored == 0`。新增字段会让该测试编译失败，强制纳入门禁。
2. `config_roundtrip_edit_then_reload_preserves_semantics`：手写配置（注释、行尾注释、未知键、内联表、已移除旧键）→ 修改 6 个字段 → `to_toml_preserving` 写盘 → 再读取语义等价；未知键/注释保留；再次保存**字节稳定**（幂等，不产生无意义 diff）。
3. `config_roundtrip_numeric_edges_through_file`：`export_margin_mm` ∈ {0, 12.5, 13.7, 20, 50}、`doc_margin_px` ∈ {4, 28.5, 200}、字号/行高 ∈ {(10,10),(16,22),(18,27),(32,96)} 全组合写盘再读取，逐位相等（含非二进制精确小数 13.7/28.5）。
4. `config_roundtrip_legacy_section_bom_crlf_keeps_semantics`：BOM + CRLF + `[theme]` 段读取语义正确（`ignored == 0`），保存回退整文件重生成后重新加载仍等价。

临时文件：`std::env::temp_dir()` + 进程 id + 纳秒时间戳 + 自增序号；`Drop` 删除，断言失败展开也清理。**未新增依赖**。

## 4. 发现的漂移与容错语义

**A. 行为漂移：无。** 全字段、数值边界（含 f32 文本 → f64 → f32 的二次舍入路径）、手写配置、BOM/CRLF/历史段四类场景均语义恒等。

**B. 读取端夹取 → 保存时规范化（既有口径，记录不改）。**
`doc_font_size` 夹 10..=32、`doc_line_height` 夹 字号..=3×字号、`doc_margin_px` 夹 4..=200、`tab_width` 非 2/4 回 4；越界**不**计入 `ignored`。用户下次保存会把夹取后的有效值写回，文件文本被规范化，但运行时语义不变。这是既有「越界夹取」设计，不在本任务硬改；不构成「静默改写」以外的行为变化。

**C. 已移除旧键的写回不对称（既有口径，记录不改）。**
读取端对 `smooth_scroll`、`mermaid_insert_url`、`mermaid_render`、`image_cache_limit_mb` 均静默忽略且不计数；写回端只剥离 `image_cache_limit_mb`，其余三个原样保留。语义稳定（保留键不回流为行为），故不改为统一剥离——统一删除属主动改写用户文件。

**D. `[theme]` 段/非严格 TOML 触发整文件重生成。**
含任何表段、重复键或非法值时 `to_toml_preserving` 回退 `to_toml()`：语义等价，但历史注释与段内未知键会丢失。该行为在 `edit_existing_toml` 注释中已声明（ADR-012 / S2），测试锁定为「语义不漂移」。

**E. `doc_font_face` 写出不校验读端接受域（边界，产品路径不可达）。**
读取端拒绝空串与 >64 字节字族（计 `ignored`）；写回端原样输出。设置 UI 的 `apply` 已限制长度并清空预设档切换，故产品路径构造不出不可读值；记录为边界而非修复。

**F. 非法 UTF-8 的处理分层。**
`decode_config` 本身拒绝非法 UTF-8，但产品 `FsConfig::load` 先 `from_utf8_lossy`，因此真实文件不会导致整份配置失效：坏字节变为替换符后逐值忽略并提示，启动仍用其余可用键。测试只覆盖合法 UTF-8 文件；该分层属既有容错，已记录。

## 5. 结论

配置读取与写回在语义层面**无漂移**；现有「未知键保留 / 缺省回填 / 越界夹取 / 历史段跳过」容错语义全部稳定且可解释。本轮只新增 4 个文件级 round-trip 测试作为回归门禁（`crates/veil-ui/src/theme.rs` 测试模块），**未修改任何产品代码**（未动 UI、解析器、布局、PDF、`app.rs`）。

后续若新增配置键，`config_roundtrip_all_fields_through_file` 会因结构体字面量缺字段而编译失败，必须先补 round-trip 断言。
