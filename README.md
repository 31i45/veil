# Veil

![Veil](icons/veil.ico)

[![Version](https://img.shields.io/badge/version-1.0.0-blue.svg)](https://github.com/veil/veil/releases)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-stable-orange.svg)](https://www.rust-lang.org/)

[中文](README.zh-CN.md)

## Why Veil

Veil was born because **Typedown** — my favorite Markdown editor on Windows, bar none — takes **tens of seconds to minutes** to launch, every single time. A WinUI app that starts cold faster than you finish sipping your coffee should not exist, yet here we are.

So I built my own. Pure Rust. No webview, no Chromium, no Electron, no external runtime. Single-file exe. 150 ms cold start. **Single-user, single-document, purely offline — no collaboration, ever.**

## Architecture

Three crates, one-way dependency (`veil` → `veil-ui` → `veil-core`, no reverse edges permitted):

| Crate | Responsibility |
|---|---|
| `veil-core` | Pure logic kernel: document model, incremental parsing, edit transactions. Zero GUI, zero async runtime deps |
| `veil-ui` | View adapter: winit + softbuffer software rendering + cosmic-text layout. Chrome (menus/status/scrollbar) is drawn in-house — no egui, no GPU API (ADR-005) |
| `veil` | Product binary: platform seams (Windows FFI, config, clipboard, file watcher) and feature wiring |

> Before v1.0, `veil-egui` was renamed `veil-ui`. The crate had already been egui-free since ADR-005; the old name no longer matched the facts. Boundaries and responsibilities unchanged.

## What It Does

1. Text editing: insert / delete / undo / redo (transactional)
2. Incremental parsing and semantic tree (dirty-block local resync)
3. Cursor & selection: grapheme-boundary navigation; markers like `**bold**` auto-expand / collapse on caret enter/leave, caret visual position never jumps
4. File open / save: auto-save, external-modification notice + pre-overwrite backup
5. Clipboard: plain text, HTML→Markdown conversion, screenshot paste writes to `<doc>.assets/` (incremental names, no overwrite)
6. Block-level rendering: tables (grid mode + active-cell inline edit), math (pure Rust via latex-rust), inline images (relative-path PNG, broken link / remote URL degrades to alt text)
7. HTML export
8. Config & themes: top-level `mode` single source (`system` / `light` / `dark`, backward-compat with old `[theme] mode`), follows system appearance, deep-titlebar / Mica, switch persistent
9. In-house chrome: menu bar, status bar, scroll bar; system file dialogs (Open / Save As)
10. Writing loop: CJK-calibrated word count, Focus Mode & Typewriter Mode (two mutually-exclusive view toggles, not persisted, reset on launch), Ctrl+P Quick Open (recent + current-dir candidates), image paste / drag-drop insertion
11. Table keyboard editing: Tab / Shift+Tab navigate cells (last cell wraps back to first); Ctrl+Enter appends empty row at table tail

## What It Won't Do (v1.0 explicitly out of scope)

- wgpu / GPU-rendered view (ADR-011: deferred, triggered only if four conditions all trip)
- Collaborative editing / CRDT (**permanent exclusion — Veil is single-user single-document**)
- Plugin system
- Mobile
- Table visual editing (merge/split cells, column-width drag, alignment UI toggles); content editing + row/column keyboard ops supported
- Mermaid inline rendering
- Source-code edit mode (v1 deferred; reveal replaces it — markers expand on caret enter, collapse on leave — v2 re-evaluates file-toggle shape)
- Any networking: no sync, no update check, no telemetry

## Build / Run / Test

Requires: Rust stable (edition 2021), Windows 11 23H2+ baseline.

```powershell
cargo build --workspace
cargo test --workspace
cargo run -p veil -- <file.md>
```

Baseline: 910 passed / 0 failed / 12 ignored (2026-11-05), `cargo clippy --workspace --all-targets -- -D warnings` zero warnings.

## Red Numbers (targets vs measured)

Measured on release build, probe-serialized, 2026-11-05 keystroke attribution detail in `docs/review/keystroke-attribution.md`:

| Metric | Target | Measured |
|---|---|---|
| Cold start (release, `main` → first frame; acceptance口径) | ≤ 300 ms | 145–190 ms ✅ |
| Cold start (wall clock, process create → first frame; observation only) | ≤ 300 ms | 450–529 ms (440–470 ms before `main`, process-create overhead on this machine) |
| Keystroke→commit p95 on 100 K-word doc | ≤ 16 ms | 3.37 ms / 5.20 ms / 9.25 ms (5.0k / 10.5k / 20.0k blocks) ✅ |
| Resident memory, 100 K-word doc (private working set) | ≤ 150 MB | 40.76 MB |
| Installed size (single `veil.exe`) | ≤ 15 MiB | 5,856,256 B ≈ 5.59 MiB |
| `veil-core` unit + property tests | ≥ 80 | 209 |
| E2E assertions | ≥ 20 | 41 (3 e2e scenarios: open→save→reopen, table+image paste, find/replace→crash recovery→self-clean) ✅ |
| Markdown roundtrip (no edits: output == input bytes) | 100% | 100% |
| `veil-core` GUI/async runtime deps | 0 | 0 |
| `veil-ui` + `veil` direct deps | ≤ 8 | 8 |
| Workspace transitive deps | ≤ 200 | 141 |

## Same-machine Comparison (P2 public baseline, 2026-10-30)

Corpus & scripts reproducible: `bench/corpus/` (generator `tools/bench/gen_corpus.py`; pathological 10MB corpus auto-generated, not committed), `tools/bench/measure.ps1`. Methodology in [P2-benchmark.md](docs/review/P2-benchmark.md).

| Metric | Veil | MarkText | Ratio |
|---|---|---|---|
| Resident memory (process tree sum) | 21.51 MB | 365.01 MB | **17.0×** |
| Installed size | 5.46 MB | 488.95 MB | **89.6×** |
| Cold start (window-handle口径) | 30 ms | 30 ms | Tie (this口径 favors Electron; see §3 of the report) |

Dependency blacklist: `tokio`, `wasmtime`, any webview crate (`webview2-com`/`wry`, etc.). Introducing any of these is a violation.

## Docs

- `.skills/INSTANCE.md` — single source of truth: all proprietary facts, measured red numbers, decision register
- `.skills/` — ai-dev-skills methodology instantiation for this project
- `docs/adr/` — Architecture Decision Records:
  - ADR-001 rope selection
  - ADR-002 formula rendering path
  - ADR-003 caret & edit-point rules
  - ADR-004 platform seams
  - ADR-005 chrome drawn in-house + system dialogs
  - ADR-006 external file watcher
  - ADR-007 config & themes
  - ADR-008 dependency upgrade strategy
  - ADR-011 wgpu deferral (v1 defers, trigger-based)
- `.trae/specs/release-and-roadmap/v1.1-roadmap.md` — v1.1 detailed plan
- `.trae/specs/release-and-roadmap/v2.0-roadmap.md` — v2.0 detailed plan (wgpu deferred — four trigger conditions must all trip)

## License

MIT License · see [LICENSE](LICENSE).
