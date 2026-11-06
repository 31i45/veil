# Veil

![Veil](icons/veil.ico)

[![Version](https://img.shields.io/badge/version-1.0.0-blue.svg)](https://github.com/31i45/veil/releases)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-stable-orange.svg)](https://www.rust-lang.org/)

[中文](README.zh-CN.md)

## Why Veil

Veil was born because **Typedown** — my favorite Markdown editor on Windows, bar none — takes **tens of seconds to minutes** to launch, every single time. A WinUI app that starts cold faster than you finish sipping your coffee should not exist, yet here we are.

So I built my own. Pure Rust. No webview, no Chromium, no Electron, no external runtime. Single-file exe. 150 ms cold start. **Single-user, single-document, purely offline — no collaboration, ever.**

## Download & Run

Grab `veil.exe` from [Releases](https://github.com/31i45/veil/releases). There is no installer, no runtime dependency, and no admin rights required.

1. Put `veil.exe` in a folder you intend to keep (for example `%LOCALAPPDATA%\Veil`). Windows stores file associations by path, so moving the exe later breaks them.
2. Double-click it, or run `veil.exe path\to\note.md`.

## Open .md Files with Veil

Veil is a portable single-file app and does not write to the registry, so it cannot register itself as the `.md` handler. To make `.md` open in Veil:

1. Right-click any `.md` file and choose **Open with** → **Choose another app**.
2. Choose **More apps** → **Look for another app on this PC**, then select your `veil.exe`.
3. Tick **Always use this app to open .md files**.

If you move `veil.exe` afterwards, repeat these steps from the new location.

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
3. Cursor & selection: grapheme-boundary navigation; markers like `**bold**` auto-expand / collapse on caret enter/leave, caret visual position never jumps; links use the theme link colour (light `#2B6CB0` / dark `#7FB4F5`), distinct from body text
4. File open / save: auto-save, external-modification notice + pre-overwrite backup
5. Clipboard: plain text, HTML→Markdown conversion, screenshot paste writes to `<doc>.assets/` (incremental names, no overwrite)
6. Block-level rendering: tables (grid mode + active-cell inline edit; the active cell shows Markdown source, other cells render bold / italic / strikethrough / inline code / links with markers hidden), math (pure Rust via latex-rust), inline images (relative-path PNG, broken link / remote URL degrades to alt text)
7. HTML export: a single self-contained file with no external assets; inline CSS follows the current light/dark theme (background, body text, muted text, link colour, code background, borders), and dark pages declare `color-scheme: dark`
8. Config & themes: top-level `mode` single source (`system` / `light` / `dark`, backward-compat with old `[theme] mode`), follows system appearance, deep-titlebar / Mica, switch persistent
9. In-house chrome: menu bar (↑/↓ move, Enter run, Esc close — main and context menus alike), status bar, scroll bar; system file dialogs (Open / Save As)
10. Writing loop: CJK-calibrated word count, Focus Mode & Typewriter Mode (two mutually-exclusive view toggles, not persisted, reset on launch), Ctrl+P Quick Open (recent + current-dir candidates), Ctrl+Shift+P print, image paste / drag-drop insertion
11. Table editing: Tab / Shift+Tab navigate cells (last cell wraps back to first); Ctrl+Enter appends empty row at table tail; the right-click menu inserts/deletes rows and columns and sets column alignment (left / center / right, separator row only); each structural change is one undo step
12. Fenced code blocks: syntax highlighting for 16 languages (keywords / strings / comments / numbers / types), implemented as a pure-Rust line-state machine — no `syntect`
13. Source mode: Ctrl+/ shows the full Markdown source (tables and math blocks included); toggling keeps caret and scroll position; print and export always use the rendered view
14. PDF export: Ctrl+Shift+E or File → Export PDF writes `<stem>.pdf` beside the document (name collisions become ` (2)`, ` (3)`); optional TOC page, page numbers and margin (default 20 mm) via `config.toml`; relies on the system "Microsoft Print to PDF". Unsaved drafts first ask for an output location (cancelling aborts); a missing PDF printer falls back to the system print dialog with a notice. Known limits: no PDF bookmarks, no font subsetting
15. Opening links: `Ctrl+click` an http/https/mailto link (a mouse gesture, not a new shortcut) to hand it to the system default browser or mail client; the caret does not move. The link context menu starts with "Open in Browser", followed by "Copy Link Address". Protocols outside the allowlist only show a notice; Veil itself makes no network requests
16. GFM footnotes: `[^id]` references and `[^id]: content` definitions. In rendered view, references show as a link-coloured `[n]`, while definition blocks hide the marker and show "number + content + ↩"; numbers follow first-reference order in the document. `Ctrl+click` a reference to jump to its definition, and `Ctrl+click` the trailing `↩` to jump back to the first reference. Source mode (`Ctrl+/`) shows the raw markup. HTML export emits `<sup>` references and a footnote list at the end (with backlinks)
17. In-document anchors: `Ctrl+click` `[text](#slug)` to jump to the matching heading; exported headings carry `id="<slug>"`; duplicate headings are de-duplicated as `name`, `name-1` (the same `heading_slug` / `heading_slugs` rules used by export)

## What It Won't Do (v1.0 explicitly out of scope)

- wgpu / GPU-rendered view (ADR-011: deferred, triggered only if four conditions all trip)
- Collaborative editing / CRDT (**permanent exclusion — Veil is single-user single-document**)
- Plugin system
- Mobile
- Table visual editing beyond the delivered subset (merge/split cells, column-width drag); content editing, row/column insert/delete and column alignment are supported
- Mermaid inline rendering
- A separate rich-text document model: the text stays Markdown, and Ctrl+/ is a view toggle over the same document
- Any networking: no sync, no update check, no telemetry; external links are handed to the system default program, and Veil itself makes no network requests

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
| Installed size (single `veil.exe`) | ≤ 15 MiB | 5,985,280 B ≈ 5.71 MiB |
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
- `.trae/specs/release-and-roadmap/v1.2-roadmap.md` — v1.2 candidate pool (table visual editing, PDF bookmarks, font subsetting, typography settings)
- `.trae/specs/release-and-roadmap/v2.0-roadmap.md` — v2.0 detailed plan (wgpu deferred — four trigger conditions must all trip)

## License

MIT License · see [LICENSE](LICENSE).
