# Veil

![Veil](icons/veil.ico)

[![Version](https://img.shields.io/badge/version-1.2.0-blue.svg)](https://github.com/31i45/veil/releases)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-stable-orange.svg)](https://www.rust-lang.org/)

[中文](README.zh-CN.md)

## Why Veil

Veil was born because **Typedown** — my favorite Markdown editor on Windows, bar none — takes **tens of seconds to minutes** to launch, every single time. A WinUI app that starts cold faster than you finish sipping your coffee should not exist, yet here we are.

So I built my own. Pure Rust. No webview, no Chromium, no Electron, no external runtime. Single-file exe. In-app cold start ≈150 ms (the first open-and-edit adds ≈160 ms; measurements, commands and dates are in [Red Numbers](#red-numbers-targets-vs-measured)). **Single-user, single-document, purely offline — no collaboration, ever.**

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
18. Line navigation: `Alt+↑` / `Alt+↓` move the current line (one undo step); `PgUp` / `PgDn` page the viewport while keeping the target column; `Ctrl+Backspace` / `Ctrl+Delete` delete by word and never cross a line break
19. Find & replace: a "Match case" toggle (off by default, kept for the session); `Enter` replaces the current match and advances, `Ctrl+Enter` replaces all
20. Readable measure: above 1600 px window width the body column is capped at ~40 full-width characters and centred; H4–H6 indent by level; heading spacing is wider than paragraph spacing
21. Visible trust: the status bar shows "Saved · HH:MM" (or "Unsaved"); a failed CLI open raises a visible error box and exits with code 1
22. Caret as focus: solid when the window is focused, hollow when it is not — no timers, no idle wake-ups

## What It Won't Do (v1.2 explicitly out of scope)

- wgpu / GPU-rendered view (ADR-011: deferred, triggered only if four conditions all trip)
- Collaborative editing / CRDT (**permanent exclusion — Veil is single-user single-document**)
- Spell checking
- Plugin system
- Mobile / PWA (desktop exe only)
- Table visual editing beyond the delivered subset (merge/split cells, column-width drag); content editing, row/column insert/delete and column alignment are supported
- Mermaid rendering (Mermaid fenced blocks stay as code, not rendered)
- A separate rich-text document model: the text stays Markdown, and Ctrl+/ is a view toggle over the same document
- Any networking: no sync, no update check, no telemetry; external links are handed to the system default program, and Veil itself makes no network requests
- Competitor-parity items deliberately declined: regex find (the `regex` crate added 1,198,592 B to the exe), auto-pairing of brackets/quotes, a whole-word toggle, `Ctrl+Shift+K` delete-line (`Alt+↑` / `Alt+↓` already covers line operations), ≤120 ms state-transition animations (they would break the zero-timer / no-idle-wake policy), and a separate H5/H6 type scale (indentation expresses level)

## Build / Run / Test

Requires: Rust stable (edition 2021). Acceptance environment: Windows 11 23H2+. Mica acrylic degrades to a solid colour where it is unavailable. Windows 10 has not been recorded on real hardware and is not claimed as verified.

```powershell
cargo build --workspace
cargo test --workspace
cargo run -p veil -- <file.md>
```

Baseline: 1132 passed / 0 failed / 13 ignored (2026-11-09); `cargo clippy --workspace --all-targets -- -D warnings` zero warnings.

## Red Numbers (targets vs measured)

Measured on release build, probe-serialized, re-verified 2026-11-09. Commands: `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo run --release -p veil --example bench_typing`, `cargo test -p veil-ui --release --lib -- --ignored --nocapture --test-threads=1 perf_keystroke_path` (and `perf_keystroke_10k_blocks`), and `(Get-Item target\release\veil.exe).Length`. Keystroke attribution detail in `docs/review/keystroke-attribution.md`.

| Metric | Target | Measured |
|---|---|---|
| Cold start (release, `main` → first frame; acceptance口径; 5 runs) | ≤ 300 ms | 145–190 ms (min–max; 2026-11-05, `VEIL_BOOT_LOG` segmented probe) ✅ |
| Cold start (in-process open + first full layout + first edit; p50 / max over 5 runs) | observation only | 290.0 ms (app 131.8 ms + first edit 158.2 ms; 130.1 ms after warm-up; 2026-11-09, `cargo test -p veil-ui --release --lib -- --ignored --nocapture --test-threads=1 perf_cold_first_edit`). Same machine only: repeats vary between 290 and 580 ms because of the OS file cache |
| Cold start (bench harness, process create → open → first layout; p50 / max) | observation only | 576.46 / 599.98 ms (2026-11-09, `cargo run --release -p veil --example bench_typing`; baseline 520.87 / 533.85 → +10.7% / +12.4%, regression gate ≤ 20%) |
| Cold start (wall clock, process create → first frame; observation only) | ≤ 300 ms | 450–529 ms (440–470 ms before `main`, process-create overhead on this machine; 2026-10-31) |
| Cold start (window handle; P2 comparison口径) | — | 30 ms (2026-10-30, `tools/bench/measure.ps1`) |
| Keystroke→commit p95 on 100 K-word doc | ≤ 16 ms | 6.781 ms / 12.379 ms (`perf_keystroke_path` 5,005 blocks / `perf_keystroke_10k_blocks` 10,500 blocks; baseline 6.677 / 12.488; 2026-11-09) ✅ |
| Resident memory, 100 K-word doc (private working set) | ≤ 150 MB | 40.76 MB (2026-11-05) |
| Installed size (single-file `veil.exe`; no install directory or separate unpacked payload) | ≤ 6 MiB = 6,291,456 B | 6,043,136 B ≈ 5.76 MiB (2026-11-09); margin 248,320 B |
| `veil-core` unit + property tests | ≥ 80 | 209 |
| E2E assertions | ≥ 20 | 41 (3 e2e scenarios: open→save→reopen, table+image paste, find/replace→crash recovery→self-clean) ✅ |
| Markdown roundtrip (no edits: output == input bytes; byte comparison) | 100% | 100% |
| CommonMark 0.31.2 conformance (by test cases; dimension A / C / D) | — | A 652/652 · C 652/652 · D 652/652 ✅ |
| CommonMark 0.31.2 conformance (dimension B) | — | B_EXPORT_EXACT 629/652 · B_CANON 630/652 (not full) |
| GFM extension conformance (Tables / TaskList / Strikethrough; by test cases; dimension A / C / D) | — | A 23/23 · C 23/23 · D 23/23 ✅; B_CANON 12/23 (11 autolink cases observed only) |
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

- `docs/review/v1-2-implementation-log.md` — v1.2 implementation record: measured numbers, probes, decisions
- `docs/review/keystroke-attribution.md` — keystroke-to-commit latency attribution
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
- `.trae/specs/release-and-roadmap/v1.2-roadmap.md` — v1.2 candidate pool (typography settings, PDF bookmarks, font subsetting)
- `.trae/specs/release-and-roadmap/v2.0-roadmap.md` — v2.0 detailed plan (wgpu deferred — four trigger conditions must all trip)

## License

MIT License · see [LICENSE](LICENSE).
