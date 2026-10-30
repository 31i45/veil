#!/usr/bin/env python3
"""Veil 公开基准语料生成器（阶段 2 / P2）。

生成 `bench/corpus/` 下的基准文档，供 Veil 与对照编辑器（MarkText 等）测量：
- 冷启动（进程创建 → 首帧可交互）
- 10 万字文档击键延迟（Veil 走进程内 perf 探针；对手走 UI 自动化或人工）
- 常驻内存（启动 2s 后 PrivateMemorySize64）
- 安装产物体积

用法：
    python tools/bench/gen_corpus.py [--out bench/corpus]

语料设计原则：
1. 中文为主体（目标用户场景），中英混排、行内代码/链接/强调穿插；
2. 结构覆盖：标题 / 段落 / 列表 / 引用 / 代码围栏 / 表格 / 公式 / 图片语法；
3. 规模分档：small（1 节）/ medium（100 节）/ large（1000 节，约 10 万字中文）。
   large 必须让 veil-core 产生 5,000+ 顶层块，与 perf 探针 fixture 同量级。
"""
from __future__ import annotations

import argparse
import pathlib
import random

SEED = 20261030
rng = random.Random(SEED)

CN_SENT = [
    "写作时最重要的是不被工具打断思路。",
    "当文档增长到十万字量级，编辑器仍然保持跟手才算合格。",
    "本地优先意味着你的数据始终掌握在自己手中。",
    "源码真值要求往返字节一致，不做任何隐式规范化。",
    "好的工具是透明的，你几乎感觉不到它的存在。",
    "渲染可以失之毫厘，但背后的字节流必须分毫不差。",
]
EN_SENT = [
    "Performance is a feature, not an afterthought.",
    "Local-first software keeps your words on your disk.",
    "The editor should never make you wait.",
]
MIX_SENT = [
    "在 `editor.rs` 里按下 `Ctrl+S` 触发**原子写**，中间文件是 `recovery.dat.tmp`。",
    "点击 [Veil 主页](https://example.com) 或 `https://example.com/docs` 查看文档。",
    "行内公式 $E=mc^2$ 与图片 `![示意](assets/pic.png)` 各占一席之地。",
    "**加粗**与*斜体*和 ~~删除线~~ 以及 `code` 混排同一行。",
]


def paragraph(n_sent: int = 4) -> str:
    parts = []
    for _ in range(n_sent):
        r = rng.random()
        if r < 0.55:
            parts.append(rng.choice(CN_SENT))
        elif r < 0.75:
            parts.append(rng.choice(EN_SENT))
        else:
            parts.append(rng.choice(MIX_SENT))
    return "".join(parts)


def section(idx: int) -> str:
    lines = [f"## 第 {idx} 节 标题", ""]
    lines.append(paragraph(4))
    lines.append("")
    lines.append(f"> 引用段落 {idx}：{rng.choice(CN_SENT)}")
    lines.append("  > 懒续行 " + rng.choice(EN_SENT))
    lines.append("")
    lines.append(f"- 列表项 {idx}-1：" + rng.choice(CN_SENT))
    lines.append(f"- 列表项 {idx}-2：" + rng.choice(MIX_SENT))
    lines.append(f"  - 子项 {idx}-3")
    lines.append("")
    lines.append("```rust")
    lines.append(f'fn section_{idx}() {{ println!("hello {idx}"); }}')
    lines.append("```")
    lines.append("")
    lines.append("| 列 A | 列 B | 列 C |")
    lines.append("| --- | ---: | :---: |")
    lines.append(f"| {idx} | 值 {idx} | **{idx}** |")
    lines.append("")
    lines.append("$$ \\int_0^1 x^2 \\, dx = \\frac{1}{3} $$")
    lines.append("")
    return "\n".join(lines)


def build(n_sections: int) -> str:
    head = "# Veil 公开基准语料\n\n> 由 `tools/bench/gen_corpus.py` 生成，SEED=20261030，公开可复现。\n\n"
    return head + "\n".join(section(i) for i in range(1, n_sections + 1))


def build_english(n_sections: int) -> str:
    """纯英文对照组：排除 CJK 排版差异，测「编辑器骨架」本身的开销。"""
    paras = [
        " ".join(rng.choice(EN_SENT) for _ in range(4)),
        " ".join(rng.choice(EN_SENT) for _ in range(3)),
    ]
    parts = ["# Veil Benchmark Corpus (English)\n"]
    for i in range(1, n_sections + 1):
        parts.append(f"## Section {i}\n")
        parts.append(rng.choice(paras) + "\n")
        parts.append(f"- Item {i}-a: {rng.choice(EN_SENT)}\n- Item {i}-b: {rng.choice(EN_SENT)}\n")
        parts.append("```rust\n" + f'fn sec_{i}() {{ println!("{i}"); }}' + "\n```\n")
    return "\n".join(parts)


def build_pathological(target_bytes: int = 10 * 1024 * 1024) -> str:
    """病态单块：单段落连续文本无换行，直到达到目标字节数（默认 10MB）。
    用于压测「超大单块三上限 + 降级」路径（0a 切片的保护线）。"""
    unit = "The quick brown fox jumps over the lazy dog. "
    reps = target_bytes // len(unit) + 1
    return "# Pathological single block\n\n" + unit * reps + "\n"


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default="bench/corpus", help="输出目录")
    args = ap.parse_args()
    out = pathlib.Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    for name, n in [("small.md", 1), ("medium.md", 100), ("large.md", 1000)]:
        p = out / name
        p.write_text(build(n), encoding="utf-8", newline="\n")
        print(f"{p}  sections={n}  bytes={p.stat().st_size}")
    # 对照语料：纯英文（同节数）+ 病态单块（10MB）
    en = out / "large-en.md"
    en.write_text(build_english(1000), encoding="utf-8", newline="\n")
    print(f"{en}  sections=1000  bytes={en.stat().st_size}")
    patho = out / "pathological-10mb.md"
    patho.write_text(build_pathological(), encoding="utf-8", newline="\n")
    print(f"{patho}  single-paragraph  bytes={patho.stat().st_size}")


if __name__ == "__main__":
    main()
