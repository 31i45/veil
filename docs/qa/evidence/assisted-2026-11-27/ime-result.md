# 真机 IME 六步走查结果（2026-11-27）

> 本文件由 `tools/verify/assisted/ime.ps1` 自动生成。
> 纪律：未真实执行时记 **NOT RUN**，不写「预期通过」。

- 时间：2026-11-27 23:59:12 +08:00
- 机器：DESKTOP-K0DU5LK
- 语料：`D:\VSCode\Veil\docs\qa\syntax-matrix.md`
- 结论：**NOT RUN**
- 说明：调用方指定 -Preflight：未启动应用、未进行真实输入。

## 六步结果

| 步骤 | 用户操作 | 用户观察 | 截图 | 判定 |
|---|---|---|---|---|
| 1 | 段落中部输入 nihao | 未执行 | — | NOT RUN |
| 2 | 空格上屏「你好」 | 未执行 | — | NOT RUN |
| 3 | 输入 zhongguo 选第 2 候选 | 未执行 | — | NOT RUN |
| 4 | 行首/行尾/表格单元格重复 1–3 | 未执行 | — | NOT RUN |
| 5 | 输入中按 Esc 取消组合 | 未执行 | — | NOT RUN |
| 6 | 窗口失焦再回焦 | 未执行 | — | NOT RUN |

## 用户只需执行的那一条命令

```powershell
pwsh -NoProfile -File tools/verify/assisted/ime.ps1
```

执行前把系统输入法切到「微软拼音」，并按提示在行首/行尾/表格单元格重复；脚本会在每个回车后自动截图。