# Veil

![Veil](icons/veil.ico)

[![CI](https://github.com/31i45/veil/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/31i45/veil/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/31i45/veil?label=release)](https://github.com/31i45/veil/releases)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-stable%20%C2%B7%20edition%202021-orange.svg)](https://www.rust-lang.org/)
[![Platform](https://img.shields.io/badge/platform-Windows%2011%20%C2%B7%20x64-0078d4.svg)](https://github.com/31i45/veil/releases)
[![Portable](https://img.shields.io/badge/single%20exe%20%C2%B7%20no%20installer%20%C2%B7%20no%20bg%20networking-2ea44f.svg)](https://github.com/31i45/veil/releases)

[中文](README.zh-CN.md)

## Why Veil

Veil was born because **Typedown** — my favorite Markdown editor on Windows, bar none — takes **tens of seconds to minutes** to launch, every single time. A WinUI app that starts cold faster than you finish sipping your coffee should not exist, yet here we are.

So I built my own. Pure Rust. No webview, no Chromium, no Electron, no external runtime. Single-file exe. In-app cold start ≈150 ms (the first open-and-edit adds ≈160 ms; measurements, commands and dates are in [Red Numbers](#red-numbers-targets-vs-measured)). **Single-user, single-document, zero background networking — no collaboration, ever.**

## Download & Run

Grab `veil.exe` from [Releases](https://github.com/31i45/veil/releases). There is no installer, no runtime dependency, and no admin rights required.

1. Put `veil.exe` in a folder you intend to keep (for example `%LOCALAPPDATA%\Veil`). Windows stores file associations by path, so moving the exe later breaks them.
2. Double-click it, or run `veil.exe path\to\note.md`.

## Open .md Files with Veil

Veil stays portable: startup never touches the registry. Association is opt-in through one explicit switch:

```powershell
veil.exe /register     # write per-user ProgID + embedded document icon, add Veil to "Open with", fill empty .md entries
veil.exe /unregister   # undo only what /register created and still owns
```

Both the app icon (white page + veiled V) and the `.md` document icon (folded page + M↓) are original designs made for Veil; no third-party icon set — including Typedown's — is used.

`/register` never steals an existing default: it fills empty slots only and never rewrites `UserChoice`. To make double-clicking `.md` open in Veil, pick Veil under **Settings → Apps → Default apps** (Windows forbids programs from setting themselves as the default). If you move `veil.exe` afterwards, run `/unregister` from the old path and `/register` from the new one — or associate manually:

1. Right-click any `.md` file and choose **Open with** → **Choose another app**.
2. Choose **More apps** → **Look for another app on this PC**, then select your `veil.exe`.
3. Tick **Always use this app to open .md files**.

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
6. Block-level rendering: tables (grid mode + active-cell inline edit; the active cell shows Markdown source, other cells render bold / italic / strikethrough / inline code / links with markers hidden), math (pure Rust via latex-rust), inline images (relative-path PNG; broken links degrade to alt text; remote http/https images load automatically, can be disabled globally via **View → Block Remote Images**, and on failure degrade to an alt placeholder with a one-time status-bar notice)
7. HTML export: a single self-contained file with no external assets; inline CSS follows the current light/dark theme (background, body text, muted text, link colour, code background, borders), and dark pages declare `color-scheme: dark`
8. Config & themes: top-level `mode` single source (`system` / `light` / `dark`, backward-compat with old `[theme] mode`), follows system appearance, deep-titlebar / Mica, switch persistent
9. In-house chrome: menu bar (↑/↓ move, Enter run, Esc close — main and context menus alike), status bar, scroll bar; system file dialogs (Open / Save As)
10. Writing loop: CJK-calibrated word count, Focus Mode & Typewriter Mode (two mutually-exclusive view toggles, not persisted, reset on launch), Ctrl+P Quick Open (recent + current-dir candidates), Ctrl+Shift+P print, image paste / drag-drop insertion
11. Table editing: Tab / Shift+Tab navigate cells (Tab in the last cell appends a row and moves to the new row's first cell); Ctrl+Enter appends an empty row at the table tail; the right-click menu inserts/deletes rows and columns and sets column alignment (left / center / right, separator row only); each structural change is one undo step
12. Fenced code blocks: syntax highlighting via `syntect` (fancy-regex, pure Rust) over its default grammar set (75 syntaxes); 17 of the 19 legacy canonical labels are covered (`toml` / `dockerfile` fall back to plain text). The syntax set loads lazily on the first code block, and per-line `ParseState` caching means a one-character edit reparses only the affected line. Each block's top-right corner shows its language label (canonical name for supported languages, raw info string for unknown ones, hidden when absent) and a copy button that copies the code verbatim (no fence or language marker; the document, undo stack and dirty flag stay untouched). Keyboard/menu: `Ctrl+Shift+C` copies the code block at the caret, and the block's right-click menu starts with "Copy Code Block"
13. Source mode: Ctrl+/ shows the full Markdown source (tables and math blocks included); toggling keeps caret and scroll position; print and export always use the rendered view
14. PDF export: Ctrl+Shift+E or File → Export PDF writes `<stem>.pdf` beside the document (name collisions become ` (2)`, ` (3)`); optional TOC page, page numbers and margin (default 20 mm) via `config.toml`. Native PDF output (multi-level bookmarks, CJK font subsetting, heading/table/image/math layout) with fallback to the system "Microsoft Print to PDF". Unsaved drafts first ask for an output location (cancelling aborts).
15. Opening links: `Ctrl+click` an http/https/mailto link (a mouse gesture, not a new shortcut) to hand it to the system default browser or mail client; the caret does not move. The link context menu starts with "Open in Browser", followed by "Copy Link Address". Protocols outside the allowlist only show a notice; aside from on-demand remote-image fetches, Veil itself makes no network requests
16. GFM footnotes: `[^id]` references and `[^id]: content` definitions. In rendered view, references show as a link-coloured `[n]`, while definition blocks hide the marker and show "number + content + ↩"; numbers follow first-reference order in the document. `Ctrl+click` a reference to jump to its definition, and `Ctrl+click` the trailing `↩` to jump back to the first reference. Source mode (`Ctrl+/`) shows the raw markup. HTML export emits `<sup>` references and a footnote list at the end (with backlinks)
17. In-document anchors: `Ctrl+click` `[text](#slug)` to jump to the matching heading; exported headings carry `id="<slug>"`; duplicate headings are de-duplicated as `name`, `name-1` (the same `heading_slug` / `heading_slugs` rules used by export)
18. Line navigation: `Alt+↑` / `Alt+↓` move the current line (one undo step); `PgUp` / `PgDn` page the viewport while keeping the target column; `Ctrl+Backspace` / `Ctrl+Delete` delete by word and never cross a line break
19. Find & replace: a "Match case" toggle (off by default, kept for the session); a regex mode (`.*` button or `Alt+R`, kept for the session) with line-anchored `^`/`$`, capture-group replacement (`$1` / `${name}`) and an inline "invalid regex" state; `Enter` replaces the current match and advances, `Ctrl+Enter` replaces all
20. Readable measure: above 1600 px window width the body column is capped at ~40 full-width characters and centred; H4–H6 indent by level; heading spacing is wider than paragraph spacing
21. Visible trust: the status bar shows the file name plus the last saved time (or an unsaved marker when the buffer is dirty); a failed CLI open raises a visible error box and exits with code 1
22. Caret as focus: solid when the window is focused, hollow when it is not — no timers, no idle wake-ups
23. State transitions: hover/press highlights in the menu bar, menus, outline, Quick Open, context menu, find bar and dialog buttons cross-fade over 120 ms (ease-out cubic); the fade advances only while it runs, so the idle path still schedules no timers and never wakes the process. Fades are skipped entirely when Windows "Show animations" is off
24. Remote images (v1.4): http/https images load automatically; **View → Block Remote Images** disables them globally; a failed fetch degrades to an alt placeholder with a one-time status-bar notice. Requests never carry cookies, `Referer` or credentials, the User-Agent is fixed at `Veil/<version>`, connect/read timeout is 10 s, the response body is capped at 20 MiB, at most 3 redirects are followed with https→http downgrades refused, and localhost / loopback / private / link-local addresses are rejected. Export and PDF never include remote resources
25. Editing feel (v1.5): auto-pairing for `()` `[]` `{}` `"` `'` with wrap-selection and paired backspace; the 600 ms caret blink pauses during IME preedit and printing; `Ctrl+,` opens a settings modal (10 items, applied immediately)
26. Outline & TOC: a `[TOC]` line renders as a nested ordered list with heading anchors and refreshes as you edit
27. Image workflow (v1.6 + this round): alt text is shown as a caption; standalone images support `<img align="left|center|right">` and explicit `width`/`height`; click a standalone image to open the zoom preview (wheel pans, `Ctrl+wheel` or `+`/`-` zooms, `0` fits, arrow keys / page keys pan, `Esc` or the mask closes; `Ctrl+click` still places the caret and reveals source)
28. Reading & navigation: a collapsed Front Matter card expands on click; heading/TOC anchors jump; interactive chrome shows a pointer cursor
29. Print & PDF (this round): in-app print preview (page turn / zoom / `Esc`), PDF link annotations, and layout that matches the rendered view
30. Inline HTML folding: block-level `<details>` expands and collapses by clicking its summary; `<details>` inside quotes, callouts and list items folds the same way (view-only, never rewrites document bytes, expands when the caret enters the body); semantics are identical across screen / print / PDF / HTML export
31. Version history (`Ctrl+Shift+H`): every overwrite archives the previous disk version first, and a dirty buffer is captured every 30 minutes; the panel lists snapshots with a read-only preview, restores one (a single `Ctrl+Z` undoes it), deletes a snapshot or clears the list
32. Outline panel (`Ctrl+Shift+O`): headings of the current document as a click-to-jump list, refreshed as you edit, with an explicit empty state when the document has no headings
33. View controls: zoom 50%–300% in 10% steps via `Ctrl+=` / `Ctrl+-` / `Ctrl+0` or `Ctrl+wheel` (trackpad pinch included; session-only, not persisted), and `F11` full screen
34. Windows: `Ctrl+Shift+N` opens an independent window; the current window keeps its document, caret and dirty state

## What It Won't Do (still out of scope)

- wgpu / GPU-rendered view (ADR-011: deferred, triggered only if four conditions all trip)
- Collaborative editing / CRDT (**permanent exclusion — Veil is single-user single-document**)
- Spell checking
- Plugin system
- Mobile / PWA (desktop exe only)
- Table visual editing beyond the delivered subset (merge/split cells); content editing, row/column insert/delete, column alignment and column-width drag are supported
- Mermaid rendering itself: Mermaid fenced blocks stay as code (a backend-neutral renderer interface is reserved; no rendering runtime is bundled)
- A separate rich-text document model: the text stays Markdown, and Ctrl+/ is a view toggle over the same document
- Background networking: no sync, no update check, no telemetry; the only outbound requests are made on demand when rendering remote images in a document, and can be disabled globally via **View → Block Remote Images**. External links are handed to the system default program; Export and PDF never include remote resources.
- Competitor-parity items deliberately declined: a separate whole-word toggle (the regex mode's `\b` covers it), `Ctrl+Shift+K` delete-line (`Alt+↑` / `Alt+↓` already covers line operations), and a separate H5/H6 type scale (indentation expresses level). Auto-pairing of brackets/quotes shipped in v1.5 and is no longer on this list

## Build / Run / Test

Requires: Rust stable (edition 2021). Acceptance environment: Windows 11 23H2+. Mica acrylic degrades to a solid colour where it is unavailable. Windows 10 has not been recorded on real hardware and is not claimed as verified.

Interface text ships in Chinese (zh-CN) only: the i18n layer is a single-locale table with no English entries, so English labels quoted in this README are translations of the shipped Chinese strings.

```powershell
cargo build --workspace
cargo test --workspace
cargo run -p veil -- <file.md>
```

Baseline: 1797 passed / 0 failed / 14 ignored (2026-11-17, re-verified after this round); `cargo clippy --workspace --all-targets -- -D warnings` zero warnings.

## Red Numbers (targets vs measured)

Measured on release build, probe-serialized; performance rows re-verified 2026-11-09, test and size rows 2026-11-17. Commands: `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo run --release -p veil --example bench_typing`, `cargo test -p veil-ui --release --lib -- --ignored --nocapture --test-threads=1 perf_keystroke_path` (and `perf_keystroke_10k_blocks`), `(Get-Item target\release\veil.exe).Length`, and `cargo tree -e normal --workspace --prefix none` for dependency counts. Keystroke attribution detail in `docs/review/keystroke-attribution.md`.

| Metric | Target | Measured |
|---|---|---|
| Cold start (release, `main` → first frame; acceptance definition; 5 runs) | ≤ 300 ms | 145–190 ms (min–max; 2026-11-05, `VEIL_BOOT_LOG` segmented probe) ✅ |
| Cold start (in-process open + first full layout + first edit; p50 / max over 5 runs) | observation only | 290.0 ms (app 131.8 ms + first edit 158.2 ms; 130.1 ms after warm-up; 2026-11-09, `cargo test -p veil-ui --release --lib -- --ignored --nocapture --test-threads=1 perf_cold_first_edit`). Same machine only: repeats vary between 290 and 580 ms because of the OS file cache |
| Cold start (bench harness, process create → open → first layout; p50 / max) | observation only | 576.46 / 599.98 ms (2026-11-09, `cargo run --release -p veil --example bench_typing`; baseline 520.87 / 533.85 → +10.7% / +12.4%, regression gate ≤ 20%) |
| Cold start (wall clock, process create → first frame; observation only) | ≤ 300 ms | 450–529 ms (440–470 ms before `main`, process-create overhead on this machine; 2026-10-31) |
| Cold start (window handle; P2 comparison definition) | — | 30 ms (2026-10-30, `tools/bench/measure.ps1`) |
| Keystroke→commit p95 on 100 K-word doc | ≤ 16 ms | 1.297 ms / 1.796 ms (`perf_keystroke_path` 5,005 blocks / `perf_keystroke_10k_blocks` 10,500 blocks; same machine, re-measured 2026-11-15; previous record 6.781 / 12.379) ✅ |
| Resident memory, 100 K-word doc (private working set) | ≤ 150 MB | 40.76 MB (2026-11-05) |
| Installed size — code + data (non-icon) | ≤ 6 MiB (Phase A baseline; superseded by the Wave 3/4 PDF export gate allowance) | 9,880,899 B (exe minus 89,277 B icons: `veil.ico` 46,688 + `veil-doc.ico` 42,589; 2026-11-17) |
| Installed size — total single-file `veil.exe` | ≤ 10 MiB = 10,485,760 B (Wave 4 authorization; every increment must be recorded) | 9,970,176 B ≈ 9.51 MiB (local build, pinned toolchain; +2,208,768 B vs v1.7.0 7,761,408 B; margin 515,584 B). The published v1.7.0 asset built on CI is 7,763,456 B |
| `veil-core` unit + property tests | ≥ 80 | 343 |
| E2E assertions | ≥ 20 | 41 (3 e2e scenarios: open→save→reopen, table+image paste, find/replace→crash recovery→self-clean) ✅ |
| Markdown roundtrip (no edits: output == input bytes; byte comparison) | 100% | 100% |
| CommonMark 0.31.2 conformance (by test cases; dimension A / C / D) | — | A 652/652 · C 652/652 · D 652/652 ✅ |
| CommonMark 0.31.2 conformance (dimension B) | — | B_EXPORT_EXACT 629/652 · B_CANON 630/652 (not full) |
| GFM extension conformance (Tables / TaskList / Strikethrough; by test cases; dimension A / C / D) | — | A 23/23 · C 23/23 · D 23/23 ✅; B_CANON 12/23 (11 autolink cases observed only) |
| `veil-core` GUI/async runtime deps | 0 | 0 |
| `veil-ui` + `veil` external direct deps (`cargo tree --depth 1`, distinct crate names) | ≤ 8 (pre-PDF baseline; superseded by the Wave 3 authorization for `pdf-writer` + `subsetter` + `ttf-parser`, and by v1.8's `regex`) | 17 (8 in `veil-ui` + 9 in `veil`) |
| Workspace transitive deps (distinct crate names, normal edges incl. the three workspace members) | ≤ 200 | 125 |

## Same-machine Comparison (P2 public baseline, 2026-10-30)

Corpus & scripts reproducible: generate the corpus with `tools/bench/gen_corpus.py`, then measure with `tools/bench/measure.ps1`; the corpus and results stay out of the repository (the pathological 10 MB file is generated on demand). Methodology in [P2-benchmark.md](docs/review/P2-benchmark.md).

| Metric | Veil | MarkText | Ratio |
|---|---|---|---|
| Resident memory (process tree sum) | 21.51 MB | 365.01 MB | **17.0×** |
| Installed size | 5.46 MB | 488.95 MB | **89.6×** |
| Cold start (window-handle definition) | 30 ms | 30 ms | Tie (this definition favors Electron; see §3 of the report) |

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
  - ADR-009 state model (single source of truth, orthogonal state machines, causal derivation)
  - ADR-010 collaboration position model (v2 design, frozen at v1 — Veil stays local-only)
  - ADR-011 wgpu deferral (v1 defers, trigger-based)
  - ADR-012 syntax highlighting migrates from the in-house state machine to `syntect` (fancy-regex) with a per-line incremental cache
  - ADR-013 table layout stays in-house (no `comfy-table`)
  - ADR-014 Quick Open fuzzy matching stays hand-written (no `fuzzy-matcher`/`nucleo`)
- `.trae/specs/release-and-roadmap/v1.1-roadmap.md` — v1.1 detailed plan
- `.trae/specs/release-and-roadmap/v1.2-roadmap.md` — v1.2 candidate pool (typography settings, PDF bookmarks, font subsetting)
- `.trae/specs/release-and-roadmap/v2.0-roadmap.md` — v2.0 detailed plan (wgpu deferred — four trigger conditions must all trip)

## License

MIT License · see [LICENSE](LICENSE).
