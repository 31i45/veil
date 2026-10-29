#!/usr/bin/env python3
"""抓真实 Markdown 文档用于 G1 前的块结构对照（§5.5：100 篇真实文档块结构零意外）。
来源：raw.githubusercontent.com，全部公开仓库。

用法: python tools/fetch_real_corpus.py  # 下载到 crates/veil-core/tests/conformance/real/
并发 10 线程，跳过已存在文件。
"""
import os
import sys
import urllib.request
import urllib.error
from concurrent.futures import ThreadPoolExecutor, as_completed

# 固定仓库 + 文件路径表。优先挑 docs/ 目录（真实文写作风格），其次 README。
# 覆盖 Rust / TypeScript / Python / Go / C / 中文项目，尽量混。
SOURCES = [
    # rust-lang 组织
    ("rust-lang/rust", "README.md"),
    ("rust-lang/rustc-dev-guide", "README.md"),
    ("rust-lang/rustc-dev-guide", "src/introduction.md"),
    ("rust-lang/rustc-dev-guide", "src/high-level-overview.md"),
    ("rust-lang/rustc-dev-guide", "src/part-1-intro/01-why-rustc-exists.md"),
    ("rust-lang/rustc-dev-guide", "src/part-2-building/02-building-overview.md"),
    ("rust-lang/book", "src/README.md"),
    ("rust-lang/book", "src/ch00-00-introduction.md"),
    ("rust-lang/book", "src/ch03-common-programming-concepts.md"),
    ("rust-lang/book", "src/ch04-understanding-ownership.md"),
    ("rust-lang/book", "src/ch05-structs.md"),
    ("rust-lang/book", "src/ch10-00-generics.md"),
    ("rust-lang/book", "src/ch11-00-testing.md"),
    ("rust-lang/book", "src/ch13-00-functional-features.md"),
    ("rust-lang/book", "src/ch15-00-smart-pointers.md"),
    ("rust-lang/book", "src/ch16-00-fearless-concurrency.md"),
    ("rust-lang/book", "src/ch17-00-oop.md"),
    ("rust-lang/book", "src/ch18-00-patterns.md"),
    ("rust-lang/book", "src/ch19-00-advanced-features.md"),
    ("rust-lang/std-dev-guide", "src/README.md"),
    ("rust-lang/std-dev-guide", "src/collections/overview.md"),
    ("rust-lang/std-dev-guide", "src/building/building.md"),
    ("rust-lang/rustfmt", "README.md"),
    ("rust-lang/clippy", "README.md"),
    ("rust-lang/clippy", "doc/adding_lints.md"),
    ("rust-lang/clippy", "doc/clip_dev/introduction.md"),
    ("rust-lang/cargo", "README.md"),
    ("rust-lang/cargo", "src/bin/cargo/README.md"),

    # tokio / async
    ("tokio-rs/tokio", "README.md"),
    ("tokio-rs/tokio", "docs/getting-started.md"),
    ("tokio-rs/axum", "README.md"),
    ("tokio-rs/axum", "docs/overview.md"),
    ("tokio-rs/axum", "docs/faq.md"),

    # serde
    ("serde-rs/serde", "README.md"),
    ("serde-rs/serde", "docs/README.md"),

    # egui / rerun
    ("emilk/egui", "README.md"),
    ("emilk/egui", "crates/egui/README.md"),

    # 其他高星 Rust
    ("sharkdp/bat", "README.md"),
    ("sharkdp/fd", "README.md"),
    ("sharkdp/hyperfine", "README.md"),
    ("BurntSushi/ripgrep", "README.md"),
    ("BurntSushi/ripgrep", "doc/FAQ.md"),
    ("rust-lang/regex", "README.md"),
    ("unicode-org/icu4x", "README.md"),
    ("unicode-org/icu4x", "docs/introduction.md"),

    # TypeScript / JS
    ("microsoft/TypeScript", "README.md"),
    ("microsoft/TypeScript", "CONTRIBUTING.md"),
    ("microsoft/playwright", "README.md"),
    ("nodejs/node", "README.md"),
    ("nodejs/node", "CONTRIBUTING.md"),
    ("vercel/next.js", "README.md"),
    ("vercel/swr", "README.md"),
    ("facebook/react", "README.md"),
    ("facebook/react-native", "README.md"),
    ("vuejs/core", "README.md"),
    ("vitejs/vite", "README.md"),
    ("eslint/eslint", "README.md"),
    ("prettier/prettier", "README.md"),

    # Python
    ("python/cpython", "README.rst", "cpython.md"),
    ("python/cpython", "CONTRIBUTING.md"),
    ("python/mypy", "README.md"),
    ("python/pip", "README.md"),
    ("pypa/setuptools", "README.md"),
    ("pallets/flask", "README.md"),
    ("pallets/flask", "docs/quickstart.md"),
    ("pallets/flask", "docs/api.rst", "flask-api.md"),

    # Go
    ("golang/go", "README.md"),
    ("golang/go", "doc/contribute.md"),
    ("golang/go", "doc/codewalk/network.md"),
    ("golang/go", "src/net/http/README.md"),
    ("uber-go/zap", "README.md"),

    # C / C++
    ("torvalds/linux", "README.md"),
    ("torvalds/linux", "Documentation/core-api/kernel-api.rst", "kernel-api.md"),
    ("llvm/llvm-project", "README.md"),

    # 中文 / 大型项目
    ("nlohmann/json", "README.md"),
    ("opencv/opencv", "README.md"),
    ("opencv/opencv", "doc/tutorials/README.md"),
    ("grpc/grpc", "README.md"),
    ("kubernetes/kubernetes", "README.md"),
    ("microsoft/vscode", "README.md"),
    ("microsoft/vscode", "CONTRIBUTING.md"),
    ("microsoft/vscode", "product.md"),
    ("microsoft/vscode-docs", "api/extension-guides/overview.md"),
    ("microsoft/vscode-docs", "api/references/vscode-api.md"),

    # docs 仓库专用
    ("github/docs", "content/en/README.md"),
    ("github/docs", "content/en/get-started.md"),
    ("github/docs", "content/en/account-and-profile/setting-up-and-managing-your-github-profile/personalizing-your-profile.md"),
    ("github/docs", "content/en/actions/learn-github-actions/understanding-github-actions.md"),
    ("github/docs", "content/en/actions/writing-workflows/quickstart.md"),
    ("github/docs", "content/en/copilot/overview-of-github-copilot-features.md"),

    # 混合内容（含表格、图片、长列表）
    ("github/docs", "content/en/issues/planning-and-tracking-with-projects/automating-your-project/built-in-automations.md"),
    ("github/docs", "content/en/organizations/managing-user-access-to-your-organization/roles-in-an-organization.md"),

    # 中文 Markdown 密集：Golang 中文文档
    ("polaris1119/golang.en.v5", "README.md"),
    ("polaris1119/golang.en.v5", "ch01.md"),
    ("polaris1119/golang.en.v5", "ch02.md"),
    ("polaris1119/golang.en.v5", "ch03.md"),

    # 含长表格项目
    ("rust-lang/rust", "compiler/rustc/HACKING.md"),
    ("rust-lang/cargo", "src/doc/man/cargo.md"),
    ("emilk/egui", "CHANGELOG.md"),
    ("BurntSushi/ripgrep", "CHANGELOG.md"),
]


def fetch_one(repo: str, path: str, name: str, out_dir: str) -> tuple:
    """下载单个源，返回 (safe_name, repo, path, ok)。已存在则跳过。"""
    safe_name = f"{repo.replace('/', '_').replace('.', '_')}__{name.replace('.', '_')}"
    out_path = os.path.join(out_dir, safe_name)
    if os.path.exists(out_path) and os.path.getsize(out_path) > 0:
        return (safe_name, repo, path, True, "skip")
    url = f"https://raw.githubusercontent.com/{repo}/master/{path}"
    alt_url = f"https://raw.githubusercontent.com/{repo}/main/{path}"
    for u in (url, alt_url):
        try:
            req = urllib.request.Request(
                u,
                headers={"User-Agent": "veil-real-corpus/0.1", "Accept": "*/*"},
            )
            with urllib.request.urlopen(req, timeout=20) as resp:
                data = resp.read()
            with open(out_path, "wb") as f:
                f.write(data)
            return (safe_name, repo, path, True, "ok")
        except Exception as e:
            last_err = str(e)
    return (safe_name, repo, path, False, last_err)


def main():
    out_dir = os.path.join(
        os.path.dirname(os.path.abspath(__file__)),
        "..", "crates", "veil-core", "tests", "conformance", "real",
    )
    out_dir = os.path.abspath(out_dir)
    os.makedirs(out_dir, exist_ok=True)
    index_path = os.path.join(out_dir, "index.tsv")

    tasks = []
    for src in SOURCES:
        repo = src[0]
        path = src[1]
        name = src[2] if len(src) > 2 else os.path.basename(path)
        tasks.append((repo, path, name))

    total = len(tasks)
    rows = []
    ok_count = 0
    skip_count = 0
    fail_count = 0

    with ThreadPoolExecutor(max_workers=10) as pool:
        futures = {pool.submit(fetch_one, *t, out_dir): t for t in tasks}
        for fut in as_completed(futures):
            repo, path, name = futures[fut]
            safe_name, r, p, ok, tag = fut.result()
            if ok:
                if tag == "skip":
                    skip_count += 1
                else:
                    ok_count += 1
                rows.append((safe_name, r, p))
                print(f"  [{ok_count+skip_count}/{total}] {'SKIP' if tag=='skip' else 'OK  '} {repo}/{path}", flush=True)
            else:
                fail_count += 1
                print(f"  [{ok_count+skip_count+fail_count}/{total}] FAIL {repo}/{path}: {tag}", file=sys.stderr, flush=True)

    with open(index_path, "w", encoding="utf-8", newline="") as f:
        for name, repo, path in rows:
            f.write(f"{name}\t{repo}\t{path}\n")
    print(f"\n下载完成：ok={ok_count} skip={skip_count} fail={fail_count} total={total} -> {out_dir}")


if __name__ == "__main__":
    main()
