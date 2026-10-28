#!/usr/bin/env python3
"""GFM 扩展语料生成器：从 cmark-gfm spec.txt 抽取表格/任务列表/删除线/自动链接(扩展)四节，
写入 crates/veil-core/tests/conformance/gfm/。

用法: python tools/gen_gfm_corpus.py <spec.txt>
"""
import sys
import os
import re

SECTIONS = [
    ("tables", "Tables (extension)"),
    ("task_list_items", "Task list items (extension)"),
    ("strikethrough", "Strikethrough (extension)"),
    ("autolinks_ext", "Autolinks (extension)"),
]


def parse_examples(text):
    """扫描 ```` 围栏 example 块，返回 (markdown, html, flag) 列表。"""
    examples = []
    fence_re = re.compile(r"^`{4,}\s+example(?:[ \t]+(\S+))?[ \t]*$")
    close_re = re.compile(r"^`{4,}[ \t]*$")
    lines = text.split("\n")
    i = 0
    while i < len(lines):
        m = fence_re.match(lines[i])
        if not m:
            i += 1
            continue
        flag = m.group(1) or ""
        md = []
        i += 1
        while i < len(lines) and lines[i] != ".":
            md.append(lines[i])
            i += 1
        if i >= len(lines):  # 缺 '.' 分隔符：非完整 example
            break
        i += 1
        html = []
        while i < len(lines) and not close_re.match(lines[i]):
            html.append(lines[i])
            i += 1
        if i >= len(lines):
            break
        i += 1
        examples.append(("\n".join(md), "\n".join(html), flag))
    return examples


def main(spec_path, out_dir):
    with open(spec_path, "r", encoding="utf-8") as f:
        text = f.read()
    os.makedirs(out_dir, exist_ok=True)
    rows = []
    for key, heading in SECTIONS:
        # 找到该节标题行，收集其后到下一个 '^## ' 之前的全部 example
        hm = re.search(rf"^## {re.escape(heading)}\s*$", text, re.M)
        assert hm, f"missing heading {heading}"
        section_end = text.find("\n## ", hm.end())
        if section_end < 0:
            section_end = len(text)
        section_text = text[hm.end():section_end]
        exs = parse_examples(section_text)
        print(f"{key}: {len(exs)} examples")
        for idx, (md, html, _flag) in enumerate(exs, 1):
            gid = f"{key}-{idx:03d}"
            with open(os.path.join(out_dir, gid + ".md"), "w", encoding="utf-8", newline="") as f:
                f.write(md)
            with open(os.path.join(out_dir, gid + ".html"), "w", encoding="utf-8", newline="") as f:
                f.write(html)
            rows.append((gid, key, idx))
    with open(os.path.join(out_dir, "index.tsv"), "w", encoding="utf-8", newline="") as f:
        for gid, key, idx in rows:
            f.write(f"{gid}\t{key}\t{idx}\n")
    print(f"total: {len(rows)} examples -> {out_dir}")


if __name__ == "__main__":
    spec = sys.argv[1] if len(sys.argv) > 1 else os.path.join(os.environ.get("TEMP", "."), "veil-gfm", "spec-jsd.txt")
    out = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "crates", "veil-core", "tests", "conformance", "gfm")
    main(spec, os.path.abspath(out))
