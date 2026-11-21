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

So I built my own. Pure Rust. No webview, no Chromium, no Electron, no external runtime. Single-file exe. In-app cold start ≈190 ms (the first open-and-edit adds ≈220 ms; measurements, commands and dates are in [Red Numbers](#red-numbers-targets-vs-measured)). **Single-user, single-document, zero background networking — no collaboration, ever.**

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

Four crates, one-way dependency (`veil` → `veil-ui` → `veil-core`; `veil-mermaid-offline` depends only on `veil-core`; no reverse edges permitted):

| Crate | Responsibility |
|---|---|
| `veil-core` | Pure logic kernel: document model, incremental parsing, edit transactions. Zero GUI, zero async runtime deps |
| `veil-ui` | View adapter: winit + softbuffer software rendering + cosmic-text layout. Chrome (menus/status/scrollbar) is drawn in-house — no egui, no GPU API (ADR-005) |
| `veil-mermaid-offline` | Mermaid engine: QuickJS + the pinned full `mermaid.js` + linkedom headless DOM; turns diagram source into SVG in-process, never touches the network |
| `veil` | Product binary: platform seams (Windows FFI, config, clipboard, file watcher) and feature wiring |

> Before v1.0, `veil-egui` was renamed `veil-ui`. The crate had already been egui-free since ADR-005; the old name no longer matched the facts. Boundaries and responsibilities unchanged.
>
> One release product: `veil.exe`. Mermaid renders in-process through `veil-mermaid-offline` (QuickJS + the full `mermaid.js`), so diagram source never leaves the machine. There is no second binary and no `veil-full.exe`: the engine is a default feature and is initialised lazily on the first diagram request, so a document without diagrams pays nothing for it.

## What It Does

1. Text editing: insert / delete / undo / redo (transactional)
2. Incremental parsing and semantic tree (dirty-block local resync)
3. Cursor & selection: grapheme-boundary navigation; markers like `**bold**` auto-expand / collapse on caret enter/leave, caret visual position never jumps; links use the theme link colour (light `#2B6CB0` / dark `#7FB4F5`), distinct from body text
4. File open / save: auto-save, external-modification notice + pre-overwrite backup
5. Clipboard: plain text, HTML→Markdown conversion, screenshot paste writes to `<doc>.assets/` (incremental names, no overwrite)
6. Block-level rendering: tables (grid mode + active-cell inline edit; the active cell shows Markdown source, other cells render bold / italic / strikethrough / inline code / links with markers hidden), math (pure Rust via latex-rust), inline images (relative-path PNG; broken links degrade to alt text; remote http/https images are disabled by default and only load after you check **View → Allow Remote Images**; on failure they degrade to an alt placeholder with a one-time status-bar notice)
7. HTML export: a single self-contained file with no external assets; inline CSS follows the current light/dark theme (background, body text, muted text, link colour, code background, borders), and dark pages declare `color-scheme: dark`
8. Config & themes: top-level `mode` single source (`system` / `light` / `dark`, backward-compat with old `[theme] mode`), follows system appearance, deep-titlebar / Mica, switch persistent
9. In-house chrome: menu bar (↑/↓ move, Enter run, Esc close — main and context menus alike), status bar, scroll bar; system file dialogs (Open / Save As)
10. Writing loop: CJK-calibrated word count, Focus Mode (a view toggle, not persisted, reset on launch), Ctrl+P Quick Open (recent + current-dir candidates), Ctrl+Shift+P print, image paste / drag-drop insertion
11. Table editing: Tab / Shift+Tab navigate cells (Tab in the last cell appends a row and moves to the new row's first cell); Ctrl+Enter appends an empty row at the table tail; the right-click menu inserts/deletes rows and columns and sets column alignment (left / center / right, separator row only); each structural change is one undo step
12. Fenced code blocks: syntax highlighting via `syntect` (fancy-regex, pure Rust) over its default grammar set (75 syntaxes); 17 of the 19 legacy canonical labels are covered (`toml` / `dockerfile` fall back to plain text). The syntax set loads lazily on the first code block, and per-line `ParseState` caching means a one-character edit reparses only the affected line. Each block's top-right corner shows its language label (canonical name for supported languages, raw info string for unknown ones, hidden when absent) and a copy button that copies the code verbatim (no fence or language marker; the document, undo stack and dirty flag stay untouched). Keyboard/menu: `Ctrl+Shift+C` copies the code block at the caret, and the block's right-click menu starts with "Copy Code Block"
13. Source mode: Ctrl+/ shows the full Markdown source (tables and math blocks included); toggling keeps caret and scroll position; print and export always use the rendered view
14. PDF export: Ctrl+Shift+E or File → Export PDF writes `<stem>.pdf` to a location you confirm (name collisions become ` (2)`, ` (3)`); optional TOC page, page numbers and margin (default 20 mm) via `config.toml`. This is the only PDF output path: native vector output (multi-level bookmarks, CJK font subsetting, heading/table/image/math layout), searchable and copyable. If native export fails (missing CJK fonts, unwritable target, locked file, oversized output), Veil keeps any existing target untouched and shows an actionable error — it never opens the print dialog and never writes a bitmap PDF. To preview the final layout, open the exported PDF in your system PDF viewer. Unsaved drafts first ask for an output location (cancelling aborts).
15. Opening links: `Ctrl+click` an http/https/mailto link (a mouse gesture, not a new shortcut) to hand it to the system default browser or mail client; the caret does not move. The link context menu starts with "Open in Browser", followed by "Copy Link Address". Protocols outside the allowlist only show a notice; aside from on-demand remote-image fetches, Veil itself makes no network requests
16. GFM footnotes: `[^id]` references and `[^id]: content` definitions. In rendered view, references show as a link-coloured `[n]`, while definition blocks hide the marker and show "number + content + ↩"; numbers follow first-reference order in the document. `Ctrl+click` a reference to jump to its definition, and `Ctrl+click` the trailing `↩` to jump back to the first reference. Source mode (`Ctrl+/`) shows the raw markup. HTML export emits `<sup>` references and a footnote list at the end (with backlinks)
17. In-document anchors: `Ctrl+click` `[text](#slug)` to jump to the matching heading; exported headings carry `id="<slug>"`; duplicate headings are de-duplicated as `name`, `name-1` (the same `heading_slug` / `heading_slugs` rules used by export)
18. Line navigation: `Alt+↑` / `Alt+↓` move the current line (one undo step); `PgUp` / `PgDn` page the viewport while keeping the target column; `Ctrl+Backspace` / `Ctrl+Delete` delete by word and never cross a line break
19. Find & replace: a "Match case" toggle (off by default, kept for the session); a regex mode (`.*` button or `Alt+R`, kept for the session) with line-anchored `^`/`$`, capture-group replacement (`$1` / `${name}`) and an inline "invalid regex" state; `Enter` replaces the current match and advances, `Ctrl+Enter` replaces all
20. Readable measure: above 1600 px window width the body column is capped at ~40 full-width characters and centred; H4–H6 indent by level; heading spacing is wider than paragraph spacing
21. Visible trust: the status bar degrades by priority (file name > save state > external change > source mode) and shows the file name plus the last saved time or an unsaved marker; error notices stay until you act or a newer notice arrives; a failed CLI open raises a visible error box and exits with code 1
22. Caret as focus: solid when the window is focused, hollow when it is not — no timers, no idle wake-ups
23. State transitions: hover/press highlights in the menu bar, menus, outline, Quick Open, context menu, find bar and dialog buttons cross-fade over 120 ms (ease-out cubic); the fade advances only while it runs, so the idle path still schedules no timers and never wakes the process. Fades are skipped entirely when Windows "Show animations" is off
24. Remote images (v1.4): http/https images are disabled by default; checking **View → Allow Remote Images** enables them globally; a failed fetch degrades to an alt placeholder with a one-time status-bar notice. Requests never carry cookies, `Referer` or credentials, the User-Agent is fixed at `Veil/<version>`, connect/read timeout is 10 s, the response body is capped at 20 MiB, at most 3 redirects are followed with https→http downgrades refused, and localhost / loopback / private / link-local addresses are rejected. Export and PDF never include remote resources
25. Editing feel (v1.5): auto-pairing for `()` `[]` `{}` `"` `'` with wrap-selection and paired backspace; the 600 ms caret blink pauses during IME preedit and printing; `Ctrl+,` opens a settings modal (9 items, applied immediately)
26. Outline & TOC: a `[TOC]` line renders as a nested ordered list with heading anchors and refreshes as you edit
27. Image workflow (v1.6 + this round): alt text is shown as a caption; standalone images support `<img align="left|center|right">` and explicit `width`/`height`; click a standalone image to open the zoom preview (wheel pans, `Ctrl+wheel` or `+`/`-` zooms, `0` fits, arrow keys / page keys pan, `Esc` or the mask closes; `Ctrl+click` still places the caret and reveals source)
28. Reading & navigation: a collapsed Front Matter card expands on click; heading/TOC anchors jump; interactive chrome shows a pointer cursor
29. Print & PDF (this round): File → Print… opens the system print dialog and prints through GDI, reusing the same `pdf_layout` pagination as PDF export (page range, copies and cancel are handled by the system dialog). PDF export remains the only vector output. There is no in-app print preview — export the PDF and review it in the system viewer.
30. Inline HTML folding: block-level `<details>` expands and collapses by clicking its summary; `<details>` inside quotes, callouts and list items folds the same way (view-only, never rewrites document bytes, expands when the caret enters the body); semantics are identical across screen / print / PDF / HTML export
31. Version history (`Ctrl+Shift+H`): every overwrite archives the previous disk version first, and a dirty buffer is captured every 30 minutes; the panel lists snapshots with a read-only preview, restores one (a single `Ctrl+Z` undoes it), deletes a snapshot or clears the list
32. Outline panel (`Ctrl+Shift+O`): headings of the current document as a click-to-jump list, refreshed as you edit, with an explicit empty state when the document has no headings
33. View controls: zoom 50%–300% in 10% steps via `Ctrl+=` / `Ctrl+-` / `Ctrl+0` or `Ctrl+wheel` (trackpad pinch included; session-only, not persisted), and `F11` full screen
34. Windows: `Ctrl+Shift+N` opens an independent window; the current window keeps its document, caret and dirty state
35. Mermaid diagrams: a `mermaid` fenced block stays a source card with its code and templates. Visible blocks render locally in-process (QuickJS + the full `mermaid.js`, every diagram type including mindmap) and re-render after a debounce when the source changes; the engine is created on the first diagram request, so a document without diagrams never pays for it. Rendering never touches the network — the source stays on your machine, and failures give a local, actionable error instead of falling back to a remote service. The source stays the single truth: the caret shows the fence with code background, monospace font and a copy button. The block's context menu offers **Copy source**, **Open in mermaid.live** (copies the source and opens the site; the URL carries no source and Veil uploads nothing) and **Insert image snapshot**, which writes `<doc>.assets/mermaid-N.png` and appends `![](relative-path)` after the block as one undo step. The legacy `mermaid_insert_url` key is silently ignored: a snapshot is always written to local assets, and no external image URL is ever inserted. A snapshot is an independent, possibly stale image — never a live bidirectional diagram, and the source is never deleted or hidden permanently

## What It Won't Do (still out of scope)

- wgpu / GPU-rendered view (ADR-011: deferred, triggered only if four conditions all trip)
- Collaborative editing / CRDT (**permanent exclusion — Veil is single-user single-document**)
- Spell checking
- Plugin system
- Mobile / PWA (desktop exe only)
- Table visual editing beyond the delivered subset (merge/split cells); content editing, row/column insert/delete, column alignment and column-width drag are supported
- A browser engine or WebView for diagrams: Mermaid is rendered in-process by `veil-mermaid-offline` (QuickJS + the full `mermaid.js`); no Chromium ships, and `foreignObject` output is disabled so resvg can rasterise labels
- A separate rich-text document model: the text stays Markdown, and Ctrl+/ is a view toggle over the same document
- Background networking: no sync, no update check, no telemetry, and Mermaid never touches the network. Ordinary remote images remain an independent, opt-in capability under **View → Allow Remote Images**; blocking them never affects Mermaid. External links and `mermaid.live` are only reached through an explicit user action. Export and PDF never include remote resources.
- Competitor-parity items deliberately declined: a separate whole-word toggle (the regex mode's `\b` covers it), `Ctrl+Shift+K` delete-line (`Alt+↑` / `Alt+↓` already covers line operations), and a separate H5/H6 type scale (indentation expresses level). Auto-pairing of brackets/quotes shipped in v1.5 and is no longer on this list

## Known Limitations (rendering & export)

Veil does not claim that every Markdown or Mermaid construct renders identically on all five surfaces (editing, source, HTML, PDF, print). The following are known limits, not "coming soon" promises:

- **Mermaid readability**: `gantt` task and section labels are raised from Mermaid's 11 px default to 14 px through the public `gantt.fontSize` / `sectionFontSize` options, set by the in-process renderer. Axis tick labels and internal layout metrics stay as Mermaid produces them. **Limit:** `c4context` stereotype text is fixed at about 12 px by the upstream c4 renderer; there is no safe knob to enlarge it without overflowing boxes measured at that size, so it is recorded as a **known degradation**, not silently restyled.
- **Dark Mermaid contrast**: automated WCAG-AA contrast assertions cover `mindmap`, `kanban`, `journey`, `gantt` and `c4context` (node text vs. node background). Other diagram types are checked for geometry, visible text and successful rasterisation only; per-type human visual review of dark themes remains **NOT RUN**. In `c4context`, relation labels inherit Mermaid's fixed `#444444` on dark themes, which can be low-contrast; overriding it with global CSS would leak into other diagram types, so that case stays unasserted and **NOT RUN**.
- **PDF / print tables**: cell text now carries bold, italic, inline-code background, link colour/underline and link annotations, and long cells paginate instead of being truncated. Row splitting uses a fixed line-height estimate, so exceptionally tall cells may spill onto more pages than a browser would use.
- **Chinese line breaking in PDF / print**: a simplified kinsoku pass keeps closing punctuation off line starts and opening brackets off line ends. It is not full UAX #14 line breaking with punctuation squeezing, so line breaks can still differ from the editing surface (cosmic-text).
- **Remote images**: the editing surface may display them on demand, but HTML export drops remote `src` values (self-contained output) and PDF/print degrade them to alt text or `[图片]`; export and PDF never download remote resources.
- **Image sampling**: Mermaid bitmaps are restored to CSS size and clamped to the text column before drawing, but scaled with nearest-neighbour sampling; very large diagrams are limited by the column width rather than re-rasterised per zoom level.
- **Details**: collapsed in the editing view by design; HTML, PDF and print always expand them. This is intentional, not a cross-surface defect.

## Build / Run / Test

Requires: Rust stable (edition 2021). Acceptance environment: Windows 11 23H2+. Mica acrylic degrades to a solid colour where it is unavailable. Windows 10 has not been recorded on real hardware and is not claimed as verified.

**Platform boundary:** Veil is a **Windows-only desktop application**. The `veil.exe` UI, Windows integration and release artifact are not claimed to build or run on Linux or macOS. Only the platform-independent `veil-core` crate has Linux CI coverage: the release workflow's `core-linux` job runs `cargo check -p veil-core` and `cargo test -p veil-core` on Ubuntu. A green Linux job validates the core only; it is not a Linux application release or support claim.

Interface text ships in Chinese (zh-CN) only: the i18n layer is a single-locale table with no English entries, so English labels quoted in this README are translations of the shipped Chinese strings. Veil does not detect the system display language and has no runtime language switch. Document text, exported content, and text inside diagrams are not translated with the interface.

```powershell
cargo build --workspace
cargo test --workspace
cargo run -p veil -- <file.md>

# release product: one veil.exe, offline Mermaid included by default
cargo build --release --bin veil
cargo test -p veil-mermaid-offline
```

Final: 1919 passed / 0 failed / 15 ignored (2026-11-21, commit `97d3a60`, serial `cargo test --workspace --release -- --test-threads=1`; the remote-SVG mock uses a 20 ms wall-clock deadline, so parallel runs can produce false timeouts). `cargo test -p veil --lib` is 221 passed / 1 ignored with the default feature set, which now includes the offline engine. `cargo clippy --workspace --all-targets -- -D warnings` zero warnings.

Release CI runs on `v*` tag pushes and through `workflow_dispatch`. Manual dispatch defaults to `dry_run=true` and runs Linux core check/test plus the Windows build, tests, clippy, artifact boundary, smoke and SHA-256 validation without creating or uploading a GitHub Release. Only a `v*` tag push can publish. Smoke failures are never hidden with `continue-on-error`.

## Tier-0 Gate: Real Launch Smoke

Green tests do not prove that the exe users double-click actually opens. Before publishing, run the real launch smoke against the release binary:

```powershell
pwsh -File tools/smoke_launch.ps1 -ExePath target/release/veil.exe -ExpectedTitlePattern '^veil' -MaxLaunchMs 1000
```

The window budget is **1 second**: `-MaxLaunchMs` (default 1000) measures process start → a titled top-level window being found through Win32 `EnumWindows` / `GetWindowText`, and a slower window fails with `stage=launch-too-slow` even when the window eventually opens, the title is correct, and the exit code is 0. The script does not use `Process.MainWindowHandle` to infer window or desktop availability. `-TimeoutSeconds` (default 5) only bounds probing so that "slow" can be told apart from "never appeared"; it never relaxes the 1 s budget. The script also verifies the title is non-empty (and matches `-ExpectedTitlePattern` when given), treats `panicked at` / `buffer_mut` / `Must set size of surface` in stderr as a failure even if a window appeared, then sends `WM_CLOSE` and checks the exit code. Failures exit non-zero with a structured `SMOKE FAIL ... stage=process-exited|window-timeout|launch-too-slow|stderr-panic|...` line; every result reports `launch_ms`, `max_ms`, `session`, `target_session`, `user_interactive`, `desktop` and the window probe source. The child always receives a fresh temporary `VEIL_HOME` and no document argument, so smoke cannot restore a real user document. The script refuses to run when another `veil` process is already running and never kills processes it did not start.

**A smoke failure blocks the release**, regardless of unit tests, clippy, size gates, or screenshots, and the full interactive smoke must be run locally against the final binary before publishing. The release workflow runs the same script after tests/boundary gates and before upload, never with `continue-on-error`. On Session 0 or when `UserInteractive=false`, the only acceptable reduced result is the explicit `stage=pass-degraded`: the process stays alive through the settle interval, stderr has no panic signature, its own top-level windows accept `WM_CLOSE`, and it exits with code 0. That result deliberately does **not** assert window visibility, title, or the 1 s budget; authoritative visual launch evidence remains local. A missing close target, close timeout, non-zero exit or panic still fails the job—forced cleanup is never counted as graceful termination.

## Red Numbers (targets vs measured)

Measured on release build, probe-serialized; final performance/size rows bound to the 2026-11-21 final rebuild (commit `345d8c6`, `veil.exe` 14,010,368 B, SHA-256 `17d8e081…beca5ee1`); earlier rows retain their original measurement dates. Commands: `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo run --release -p veil --example bench_typing`, `cargo test -p veil-ui --release --lib -- --ignored --nocapture --test-threads=1 perf_20k_blocks`, `cargo build --release -p veil` + `(Get-Item target\release\veil.exe).Length`, and `cargo tree -p veil --edges normal --prefix none` for dependency counts. Keystroke attribution detail in `docs/review/keystroke-attribution.md`.

| Metric | Target | Measured |
|---|---|---|
| Cold start (release, `main` → first frame; acceptance definition; 5 runs) | ≤ 300 ms | 145–190 ms (min–max; 2026-11-05, `VEIL_BOOT_LOG` segmented probe) ✅ |
| Cold start (in-process open + first full layout + first edit; p50 / max over 5 runs) | observation only | 405.7 ms (app 188.9 ms + first edit 216.8 ms; 2026-11-18, `cargo test -p veil-ui --release --lib -- --ignored --nocapture --test-threads=1 perf_cold_first_edit`). Same machine only: repeats vary between 290 and 580 ms because of the OS file cache |
| Cold start (bench harness, process create → open → first layout; p50 / max) | observation only | 576.46 / 599.98 ms (2026-11-09, `cargo run --release -p veil --example bench_typing`; baseline 520.87 / 533.85 → +10.7% / +12.4%, regression gate ≤ 20%) |
| Cold start (wall clock, process create → first frame; observation only) | ≤ 300 ms | 450–529 ms (440–470 ms before `main`, process-create overhead on this machine; 2026-10-31) |
| Cold start (window handle; P2 comparison definition) | — | 30 ms (2026-10-30, `tools/bench/measure.ps1`) |
| Keystroke→commit p95 on 100 K-word doc | ≤ 16 ms | p50 2.33–2.40 ms · p95 2.70–2.86 ms · max ≤3.19 ms (`perf_20k_blocks` 21,000 blocks / 300 samples, 2 runs, final exe 2026-11-21 commit `345d8c6`; worst p95 2.856 ms ≤ 16 ms. Earlier 10k record: 0.973 / 1.566 ms) ✅ |
| Resident memory, 100 K-word doc (private working set) | ≤ 150 MB | 40.76 MB (2026-11-05) |
| Installed size — total single-file `veil.exe` | ≤ 14 MiB = 14,680,064 B | 13.36 MiB = 14,010,368 B (2026-11-21 final clean rebuild, commit `345d8c6`, SHA-256 `17d8e081…beca5ee1`; single product: offline Mermaid engine + full `mermaid.js` + resvg, lazy-initialised) |
| `veil-core` unit + property tests | ≥ 80 | 347 |
| E2E assertions | ≥ 20 | 41 (3 e2e scenarios: open→save→reopen, table+image paste, find/replace→crash recovery→self-clean) ✅ |
| Markdown roundtrip (no edits: output == input bytes; byte comparison) | 100% | 100% |
| CommonMark 0.31.2 conformance (by test cases; dimension A / C / D) | — | A 652/652 · C 652/652 · D 652/652 ✅ |
| CommonMark 0.31.2 conformance (dimension B) | — | B_EXPORT_EXACT 629/652 · B_CANON 630/652 (not full) |
| GFM extension conformance (Tables / TaskList / Strikethrough; by test cases; dimension A / C / D) | — | A 23/23 · C 23/23 · D 23/23 ✅; B_CANON 12/23 (11 autolink cases observed only) |
| `veil-core` GUI/async runtime deps | 0 | 0 |
| `veil-ui` + `veil` external direct deps (`cargo tree --depth 1`, distinct crate names) | ≤ 8 (pre-PDF baseline; superseded by the Wave 3 authorization for `pdf-writer` + `subsetter` + `ttf-parser`, and by v1.8's `regex`) | 17 (8 in `veil-ui` + 9 in `veil`) |
| Workspace transitive deps (distinct crate names, normal edges incl. the three workspace members) | ≤ 200 | 125 (2026-11-18, `cargo tree -p veil --edges normal --prefix none`) |

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
