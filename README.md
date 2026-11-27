# Veil

![Veil](icons/veil.ico)

[![Release](https://img.shields.io/github/v/release/31i45/veil?label=release&style=flat-square)](https://github.com/31i45/veil/releases)
[![CI](https://img.shields.io/github/actions/workflow/status/31i45/veil/ci.yml?branch=main&label=CI&style=flat-square)](https://github.com/31i45/veil/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg?style=flat-square)](LICENSE)
[![Platform](https://img.shields.io/badge/platform-Windows%2010%2F11%20%C2%B7%20x64-0078d4.svg?style=flat-square)](https://github.com/31i45/veil/releases)
[![Rust](https://img.shields.io/badge/rust-stable%20%C2%B7%20edition%202021-orange.svg?style=flat-square)](https://www.rust-lang.org/)
[![Build](https://img.shields.io/badge/build-portable%20exe%20%C2%B7%20optional%20installer%20%C2%B7%20offline%20mermaid%20%C2%B7%20no%20telemetry-2ea44f.svg?style=flat-square)](https://github.com/31i45/veil/releases)
[![Stars](https://img.shields.io/github/stars/31i45/veil?style=flat-square)](https://github.com/31i45/veil/stargazers)
[![Forks](https://img.shields.io/github/forks/31i45/veil?style=flat-square)](https://github.com/31i45/veil/network/members)
[![Downloads](https://img.shields.io/github/downloads/31i45/veil/total?style=flat-square)](https://github.com/31i45/veil/releases)

[中文](README.zh-CN.md)

## Why Veil

Veil was born because **Typedown** — my favorite Markdown editor on Windows, bar none — takes **tens of seconds to minutes** to launch, every single time. A WinUI app that starts cold faster than you finish sipping your coffee should not exist, yet here we are.

So I built my own. Pure Rust. No webview, no Chromium, no Electron, no separate application runtime. Single-file exe. Historical performance figures remain attached to their builds, dates and measurement definitions in [Historical Measurements](#historical-measurements-targets-vs-results); they are not current v3.0.2 measurements. **Single-user, single-document; no background sync, telemetry or update checks — no collaboration, ever.**

## Download & Run

The **only official download source** is [github.com/31i45/veil/releases](https://github.com/31i45/veil/releases). Choose either the portable `veil.exe` or the optional per-user Inno Setup installer `veil-setup-<version>.exe`. Neither form requires a separate application runtime or administrator rights; both use Windows system components and system DLLs. Do not use mirrored or re-uploaded binaries.

Download your chosen artifact and its SHA-256 sidecar from the **same Release**: `veil.exe` with `veil.exe.sha256`, or `veil-setup-<version>.exe` with its matching sidecar. Place each pair in the same folder. The PowerShell example below verifies the portable exe; substitute the installer and sidecar names to verify the setup package:

```powershell
$exe = Join-Path $PWD 'veil.exe'
$sidecar = Join-Path $PWD 'veil.exe.sha256'
$expected = ((Get-Content -LiteralPath $sidecar -Raw).Trim() -split '\s+')[0]
$actual = (Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash
if ($actual -ne $expected) { throw "SHA-256 mismatch: expected $expected, got $actual" }
"SHA-256 verified: $actual"
```

Never compare an artifact with a sidecar from another Release. The current portable `veil.exe` and `veil-setup-<version>.exe` are signed with the **self-signed development certificate `CN=Veil Development (Long-term)`** (SHA-1 thumbprint `86C8793A44EF01EE1633BCF769244894326B4526`; NotAfter `2031-11-23T05:06:11Z`) and an RFC3161 timestamp, strictly to exercise the signing pipeline. Its explicitly approved trust is limited to the current user's root store on this development machine; it is **not a cross-machine or production trust anchor**, so other machines do not inherit that trust and may report an unknown publisher. An unsigned build or a newly established certificate may trigger SmartScreen. A matching SHA-256 verifies file integrity, not publisher reputation. If Windows warns, check the official source, digest, and publisher details, and stop if anything differs. Do not disable SmartScreen or other Windows security features.

### Portable exe

1. Put `veil.exe` in a folder you intend to keep (for example `%LOCALAPPDATA%\Veil`). Windows stores file associations by path, so moving the exe later breaks them.
2. Double-click it, or run `veil.exe path\to\note.md`.

**Upgrade / removal:** replace the old `veil.exe` with the new one. To remove the portable app, run `veil.exe /unregister` first if you used `/register`, then delete the exe. Replacing or deleting the program does not remove configuration, sessions, version history or logs under `%APPDATA%\Veil`; back up or delete that folder yourself as needed.

### Optional per-user installer

Run `veil-setup-<version>.exe` to install Veil to `%LOCALAPPDATA%\Programs\Veil`, without administrator rights or a UAC prompt. A Start Menu shortcut is created by default; a desktop shortcut is optional and not selected by default.

- **Upgrade:** run the same-version or newer setup directly over the existing installation; no prior uninstall is needed.
- **Uninstall:** use the standard Windows **Settings → Apps → Installed apps** (**Apps & features**) entry. Program files and shortcuts are removed.
- **Data:** both overwrite upgrades and uninstall preserve configuration, sessions, version history and logs under `%APPDATA%\Veil`. Back up or delete that folder yourself as needed.
- **File associations:** the installer does not register or manage file associations. During uninstall, Veil attempts `veil.exe /unregister` once before program files are deleted, cleaning associations you may have registered manually with `/register`.

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
> One application binary: `veil.exe`; the optional installer packages this same exe. Mermaid renders in-process through `veil-mermaid-offline` (QuickJS + the full `mermaid.js`), so diagram source never leaves the machine. There is no second application binary and no `veil-full.exe`: the engine is a default feature and is initialised lazily on the first diagram request, so a document without diagrams pays nothing for it.

## What It Does

1. Text editing: insert / delete / undo / redo (transactional)
2. Incremental parsing and semantic tree (dirty-block local resync)
3. Cursor & selection: grapheme-boundary navigation; markers like `**bold**` auto-expand / collapse on caret enter/leave, caret visual position never jumps; links use the theme link colour (light `#2B6CB0` / dark `#7FB4F5`), distinct from body text
4. File open / save: auto-save, external-modification notice + pre-overwrite backup
5. Clipboard: plain text, HTML→Markdown conversion, screenshot paste writes to `<doc>.assets/` (incremental names, no overwrite)
6. Block-level rendering: tables (grid mode + active-cell inline edit; the active cell shows Markdown source, other cells render bold / italic / strikethrough / inline code / links with markers hidden), math (pure Rust via latex-rust), inline images (relative-path PNG; broken links degrade to alt text; remote http/https images are disabled by default; **View → Allow Remote Images** is a persistent opt-in, and once enabled, subsequently opened documents can fetch their remote images on demand without another prompt; on failure they degrade to an alt placeholder with a one-time status-bar notice)
7. HTML export: a single self-contained file with no external assets; inline CSS follows the current light/dark theme (background, body text, muted text, link colour, code background, borders), and dark pages declare `color-scheme: dark`
8. Config & themes: top-level `mode` single source (`system` / `light` / `dark`, backward-compat with old `[theme] mode`), follows system appearance, deep-titlebar / Mica, switch persistent
9. In-house chrome: menu bar (↑/↓ move, Enter run, Esc close — main and context menus alike), status bar, scroll bar; system file dialogs (Open / Save As)
10. Writing loop: CJK-calibrated word count, Focus Mode (a view toggle, not persisted, reset on launch), Ctrl+P Quick Open (recent + current-dir candidates), Ctrl+Shift+P command palette, Ctrl+Alt+P print, image paste / drag-drop insertion
11. Table editing: Tab / Shift+Tab navigate cells (Tab in the last cell appends a row and moves to the new row's first cell); Ctrl+Enter appends an empty row at the table tail; the right-click menu inserts/deletes rows and columns and sets column alignment (left / center / right, separator row only); each structural change is one undo step
12. Fenced code blocks: syntax highlighting via `syntect` (fancy-regex, pure Rust) over its default grammar set (75 syntaxes); 17 of the 19 legacy canonical labels are covered (`toml` / `dockerfile` fall back to plain text). The syntax set loads lazily on the first code block, and per-line `ParseState` caching means a one-character edit reparses only the affected line. Each block's top-right corner shows its language label (canonical name for supported languages, raw info string for unknown ones, hidden when absent) and a copy button that copies the code verbatim (no fence or language marker; the document, undo stack and dirty flag stay untouched). Keyboard/menu: `Ctrl+Shift+C` copies the code block at the caret, and the block's right-click menu starts with "Copy Code Block"
13. Source mode: Ctrl+/ shows the full Markdown source (tables and math blocks included); toggling keeps caret and scroll position; print and export always use Rendered mode
14. PDF export: Ctrl+Shift+E or File → Export PDF writes `<stem>.pdf` to a location you confirm (name collisions become ` (2)`, ` (3)`); optional TOC page, page numbers and page margin (default 20 mm) via `config.toml`. This is the only PDF output path: native vector output (multi-level bookmarks, CJK font subsetting, heading/table/image/math layout), searchable and copyable. If native export fails (missing CJK fonts, unwritable target, locked file, oversized output), Veil keeps any existing target untouched and shows an actionable error — it never opens the print dialog and never writes a bitmap PDF. To preview the final layout, open the exported PDF in your system PDF viewer. Unsaved drafts first ask for an output location (cancelling aborts).
15. Opening links: `Ctrl+click` an http/https/mailto link (a mouse gesture, not a new shortcut) to hand it to the system default browser or mail client; the caret does not move. The link context menu starts with "Open in Browser", followed by "Copy Link Address". Protocols outside the allowlist only show a notice; aside from on-demand remote-image fetches, Veil itself makes no network requests
16. GFM footnotes: `[^id]` references and `[^id]: content` definitions. In Rendered mode, references show as a link-coloured `[n]`, while definition blocks hide the marker and show "number + content + ↩"; numbers follow first-reference order in the document. `Ctrl+click` a reference to jump to its definition, and `Ctrl+click` the trailing `↩` to jump back to the first reference. Source mode (`Ctrl+/`) shows the raw markup. HTML export emits `<sup>` references and a footnote list at the end (with backlinks)
17. In-document anchors: `Ctrl+click` `[text](#slug)` to jump to the matching heading; exported headings carry `id="<slug>"`; duplicate headings are de-duplicated as `name`, `name-1` (the same `heading_slug` / `heading_slugs` rules used by export)
18. Line navigation: `Alt+↑` / `Alt+↓` move the current line (one undo step); `PgUp` / `PgDn` page the viewport while keeping the target column; `Ctrl+Backspace` / `Ctrl+Delete` delete by word and never cross a line break
19. Find & replace: a "Match case" toggle (off by default, kept for the session); a regex mode (`.*` button or `Alt+R`, kept for the session) with line-anchored `^`/`$`, capture-group replacement (`$1` / `${name}`) and an inline "invalid regex" state; `Enter` replaces the current match and advances, `Ctrl+Enter` replaces all
20. Readable measure: above 1600 px window width the body column is capped at ~40 full-width characters and centred; H4–H6 indent by level; heading spacing is wider than paragraph spacing
21. Visible trust: the status bar degrades by priority (file name > save state > external change > source mode) and shows the file name plus the last saved time or an unsaved marker; error notices stay until you act or a newer notice arrives; a failed CLI open raises a visible error box and exits with code 1
22. Caret as focus: solid when the window is focused, hollow when it is not — no timers, no idle wake-ups
23. State transitions: hover/press highlights in the menu bar, menus, outline, Quick Open, context menu, find bar and dialog buttons cross-fade over 120 ms (ease-out cubic); the fade advances only while it runs, so the idle path still schedules no timers and never wakes the process. Fades are skipped entirely when Windows "Show animations" is off
24. Remote images (v1.4): http/https images are disabled by default; the persistent **View → Allow Remote Images** setting enables them globally, so later documents containing remote images can fetch them on demand without another prompt; a failed fetch degrades to an alt placeholder with a one-time status-bar notice. Requests never carry cookies, `Referer` or credentials, the User-Agent is fixed at `Veil/<version>`, connect/read timeout is 10 s, the response body is capped at 20 MiB, at most 3 redirects are followed with https→http downgrades refused, and localhost / loopback / private / link-local addresses are rejected. Export and PDF never include remote resources
25. Editing feel (v1.5): auto-pairing for `()` `[]` `{}` `"` `'` with wrap-selection and paired backspace; the 600 ms caret blink pauses during IME preedit and printing; `Ctrl+,` opens the settings panel (13 rows: 12 editable settings plus the read-only window geometry, applied immediately)
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
- Background networking: no sync, no update check, no telemetry, and Mermaid never touches the network. Ordinary remote images remain an independent capability under the persistent **View → Allow Remote Images** setting: once enabled, later documents containing them can make on-demand requests; blocking them never affects Mermaid. External links and `mermaid.live` are only reached through an explicit user action. Export and PDF never include remote resources.
- Competitor-parity items deliberately declined: a separate whole-word toggle (the regex mode's `\b` covers it), `Ctrl+Shift+K` delete-line (`Alt+↑` / `Alt+↓` already covers line operations), and a separate H5/H6 type scale (indentation expresses level). Auto-pairing of brackets/quotes shipped in v1.5 and is no longer on this list

## Known Limitations (rendering & export)

Veil does not claim that every Markdown or Mermaid construct renders identically on all five surfaces (Rendered mode, Source mode, HTML, PDF, print). The following are known limits, not "coming soon" promises:

- **Mermaid readability**: `gantt` task and section labels are raised from Mermaid's 11 px default to 14 px through the public `gantt.fontSize` / `sectionFontSize` options, set by the in-process renderer. Axis tick labels and internal layout metrics stay as Mermaid produces them. **Limit:** `c4context` stereotype text is fixed at about 12 px by the upstream c4 renderer; there is no safe knob to enlarge it without overflowing boxes measured at that size, so it is recorded as a **known degradation**, not silently restyled.
- **Dark Mermaid contrast**: every one of the 22 corpus diagram types was reviewed visually, with PNG/SVG evidence under `docs/qa/evidence/mermaid-dark-2026-11-27/` (dark bitmaps are composited over the real `#1e1e1e` theme background before inspection). Automated assertions check that each type renders, differs from light, rasterises, and is not fully transparent; `mindmap`/`gantt`/`flowchart-v2`/`journey`/`radar`/`c4context`/`pie`/`packet` also get WCAG contrast assertions. `gitGraph` branch labels measured 3.35:1; they are now fixed by adding a thin outline to the label text only under `aria-roledescription="gitGraph"` (branch colours and pill background unchanged), and re-rasterised sampling gives 5.04:1 (≥4.5, AA). `c4context` stereotype text is fixed at about 12 px upstream, the only remaining known degradation. In the light theme, `kanban`/`journey` section labels are overridden to dark text and have no corresponding dark override.
- **PDF / print tables**: cell text carries bold, italic, inline-code background, link colour/underline and link annotations, and long cells paginate instead of being truncated. Tables break only at row boundaries and repeat the header row at the top of every continuation page; an individual row is split only when it is taller than a full page (the header is still repeated). Splitting uses a fixed line-height estimate, so exceptionally tall rows may spill onto more pages than a browser would use. A display formula taller than a page is likewise scaled proportionally into one page's usable height (print: frame height × 80%; PDF: content height − 12pt), with no content dropped and no hard cut across pages; block height, painting and hit-testing share the same scaled geometry.
- **Wide tables in narrow windows**: columns are compressed back into the body column by default so the table never overflows. To read every column, hover the table and use `Shift+wheel` to enter readable-wide mode and pan horizontally (a thin indicator sits under the table); painting, hit-testing, caret placement and column dragging share the same pan offset. Print and export always fit the body column.
- **Chinese line breaking in PDF / print**: a simplified kinsoku pass keeps closing punctuation off line starts and opening brackets off line ends. It is not full UAX #14 line breaking with punctuation squeezing, so line breaks can still differ from Rendered mode (cosmic-text).
- **Remote images**: Rendered mode may display them on demand (when the global policy blocks them, right-click the image and choose **Allow Remote Images (global)**; confirming re-lays out and retries immediately, with no trip to Settings), but HTML export drops remote `src` values (self-contained output) and PDF/print degrade them to alt text or `[图片]`; export and PDF never download remote resources.
- **Image sampling**: Mermaid bitmaps are restored to CSS size and clamped to the body column before drawing, but scaled with nearest-neighbour sampling; very large diagrams are limited by the body column width rather than re-rasterised per zoom level.
- **Details**: collapsed in Rendered mode by design; HTML, PDF and print always expand them. This is intentional, not a cross-surface defect.

Beyond rendering and export, this section also records verification gaps and standing product decisions:

- **Manual items (assisted flow; the user only performs the physical action)**: two required on-site checks plus one optional smoke check now run from one command that prompts step by step, captures screenshots/files automatically and writes the verdict table: `tools/verify/assisted/{dpi,print,ime}.ps1`, documented in `tools/verify/assisted/README.md`. Current status: system DPI 100/125/200 **NOT RUN** (150% has a real capture with script `status=PASS`); physical-printer output is an **optional smoke check, not a gate** (NOT RUN; Print-to-PDF already covers pagination correctness; sample, driver list and six-item checklist are generated automatically); real Chinese IME six-step walkthrough **NOT RUN** (the code-side anchor and preedit path have unit tests). No unexecuted item counts as a pass. Other verification gaps: remote GitHub Actions and a Linux desktop; a real network-capture run for offline attribution; systems without any CJK font together with read-only export targets. High contrast has one real system-switch PASS (228 ms) plus forced high-contrast re-verification screenshots (see `docs/qa/release-facts-2026-11-27.md`); the look under extreme themes still needs human confirmation. Accessibility support was removed; see the Accessibility section below.
- **Line-breaking dependency: `unicode-linebreak` is a NO-GO (recorded decision, not a to-do)**: the existing line-breaking pass already covers 99%+ of real text, so pulling in `unicode-linebreak` was evaluated and rejected — the gain did not justify the dependency. This is a decision record; it is not an upcoming feature.
- **Fonts come from the system; nothing is bundled**: body text is laid out with the installed fonts enumerated via `fontdb`; no font file ships in the repository or inside the binary, so font choice never affects the `veil.exe` size. Any installed family can be selected under **Settings → Body Font Family**; when the chosen face lacks a glyph (for example a Latin-only face used for Chinese), it falls back to a system CJK font instead of showing tofu.
- **Source mode layouts**: Source mode has two layouts over the same bytes. By default it is "block layout + visible markers": a blank line between blocks is not independently addressable, and Up/Down jumps straight to the neighbouring block (the blank line can still be deleted with Backspace at the start of the next paragraph, and `Ctrl+Z` restores it). Enable **Settings → Source mode: plain-text line view** (off by default) to lay every physical line, blank lines included, on its own addressable row; Up/Down then stops on blank lines. Saving bytes are identical in both layouts.

## Accessibility

Veil **does not provide screen-reader / UIA support**. The AccessKit/UIA layer was permanently removed in commit `3a6d72d` (`refactor(ui)!: 永久移除无障碍（AccessKit）能力`): the UI exposes no accessibility tree, so Narrator, NVDA and similar tools cannot read the body, menus or dialogs.

This is an explicit non-goal, not a "coming soon" promise; do not choose Veil if accessibility is a hard requirement. The historical scope note in [docs/review/accessibility-scope-and-limits.md](docs/review/accessibility-scope-and-limits.md) is archived only — the capability it describes no longer exists.

## Build / Run / Test

Requires: Rust stable (edition 2021). Acceptance environment: Windows 11 23H2+. Windows 10 is also reported working by community testers and is expected to work, but it is not part of the formal acceptance matrix. Mica acrylic degrades to a solid colour where it is unavailable.

**Platform boundary:** Veil is a **Windows-only desktop application**. The `veil.exe` UI, Windows integration and release artifact are not claimed to build or run on Linux or macOS. Only the platform-independent `veil-core` crate has Linux CI coverage: the release workflow's `core-linux` job runs `cargo check -p veil-core` and `cargo test -p veil-core` on Ubuntu. A green Linux job validates the core only; it is not a Linux application release or support claim.

Interface language follows the Windows display language by default: a Chinese system gets zh-CN, an English system gets en-US, and every other language falls back to en-US. You can also choose Chinese or English explicitly under **Settings → Interface Language**; the choice is saved and takes effect after a restart, and **Automatic** returns to following the system. Every interface string ships in both Chinese and English, so no label falls back to Chinese on an English system. `VEIL_UI_LANG=zh-CN|en-US` exists only for QA; it overrides **Automatic** but not a manual choice in Settings. Document text, exported content, and text inside diagrams are not translated with the interface.

```powershell
cargo build --workspace
cargo test --workspace
cargo run -p veil -- <file.md>

# release product: one veil.exe, offline Mermaid included by default
cargo build --release --bin veil
cargo test -p veil-mermaid-offline
```

Historical release gate (v3.0.0-era records, **not current v3.0.2 acceptance**): serial `cargo test --workspace --release -- --test-threads=1` passed at commit `0d229eb` (`evidence/release/release-record.json`, `tests.status=passed`); the recorded `veil-ui` lib breakdown is **1281 passed / 11 ignored**. Earlier, commit `97d3a60` recorded 1919 passed / 0 failed / 15 ignored (the remote-SVG mock uses a 20 ms wall-clock deadline, so parallel runs can produce false timeouts). The same historical record lists `cargo test -p veil --lib` as 221 passed / 1 ignored with the default feature set and `cargo clippy --workspace --all-targets -- -D warnings` at zero warnings. Current v3.0.2 release evidence was not rebuilt or rerun in this wording review.

`#[ignore]` probes are no longer an unexplained gap. The 18 remaining ignored tests (13 release-optimised performance probes, 4 pathological / huge-block probes, 1 CJK-PDF evidence generator) plus one promoted diagnostic are listed in `docs/qa/ignored-tests-2026-11-27.md` with why each is ignored and how it is judged. Run them all against a release build with one command:

```powershell
pwsh -NoProfile -File tools/verify/probe-gate.ps1
```

The gate is deliberately thin: it builds the release test binaries, runs every ignored probe once (serially), tees the output to `docs/qa/probe-gate-<date>.md`, and exits with cargo's status. **Each probe asserts its own regression ceiling or structural invariant inside the test** — single observations are sampled 3 times and asserted on p50, and diagnostic/attribution probes must show a non-trivial breakdown rather than only printing. There is no external baseline JSON, metric-regex parsing, p50/p95 policy, or noise floor. **FAIL means a probe's own in-test assertion failed.** SKIP is reserved for missing environment (for example no system CJK font) and is never reported as PASS. `mindmap_viewbox_debug` is now a normal assertion; its SVG dump requires `VEIL_MINDMAP_DUMP=1`.

Release CI runs on `v*` tag pushes and through `workflow_dispatch`. Manual dispatch defaults to `dry_run=true` and runs Linux core check/test plus the Windows build, tests, clippy, artifact boundary, smoke, optional Authenticode/RFC3161 signing and post-signing SHA-256 validation without creating or uploading a GitHub Release. Only a `v*` tag push can publish. Smoke failures are never hidden with `continue-on-error`.

## Tier-0 Gate: Real Launch Smoke

Green tests do not prove that the exe users double-click actually opens. Before publishing, run the real launch smoke against the release binary:

```powershell
pwsh -File tools/smoke_launch.ps1 -ExePath target/release/veil.exe -ExpectedTitlePattern '^veil' -MaxLaunchMs 1000
```

The window budget is **1 second**: `-MaxLaunchMs` (default 1000) measures process start → a titled top-level window being found through Win32 `EnumWindows` / `GetWindowText`, and a slower window fails with `stage=launch-too-slow` even when the window eventually opens, the title is correct, and the exit code is 0. The script does not use `Process.MainWindowHandle` to infer window or desktop availability. `-TimeoutSeconds` (default 5) only bounds probing so that "slow" can be told apart from "never appeared"; it never relaxes the 1 s budget. The script also verifies the title is non-empty (and matches `-ExpectedTitlePattern` when given), treats `panicked at` / `buffer_mut` / `Must set size of surface` in stderr as a failure even if a window appeared, then sends `WM_CLOSE` and checks the exit code. Failures exit non-zero with a structured `SMOKE FAIL ... stage=process-exited|window-timeout|launch-too-slow|stderr-panic|...` line; every result reports `launch_ms`, `max_ms`, `session`, `target_session`, `user_interactive`, `desktop` and the window probe source. The child always receives a fresh temporary `VEIL_HOME` and no document argument, so smoke cannot restore a real user document. The script refuses to run when another `veil` process is already running and never kills processes it did not start.

**A smoke failure blocks the release**, regardless of unit tests, clippy, size gates, or screenshots, and the full interactive smoke must be run locally against the final binary before publishing. The release workflow runs the same script after tests/boundary gates and before upload, never with `continue-on-error`. On Session 0 or when `UserInteractive=false`, the only acceptable reduced result is the explicit `stage=pass-degraded`: the process stays alive through the settle interval, stderr has no panic signature, its own top-level windows accept `WM_CLOSE`, and it exits with code 0. That result deliberately does **not** assert window visibility, title, or the 1 s budget; authoritative visual launch evidence remains local. A missing close target, close timeout, non-zero exit or panic still fails the job—forced cleanup is never counted as graceful termination.

## Historical Measurements (targets vs results)

**These are historical records, not a v3.0.2 release PASS.** This wording review did not rerun startup, keystroke, Working Set/RSS, Private Bytes, or artifact-size measurements for current v3.0.2; those current measurements are **NOT RUN**. Each result below belongs only to its stated date, build and commit. The current artifact size and SHA-256 must come from the matching Release and its bundled `veil.exe.sha256`, not from a historical figure quoted here. Reproduction commands: `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo run --release -p veil --example bench_typing`, `cargo test -p veil-ui --release --lib -- --ignored --nocapture --test-threads=1 perf_20k_blocks`, `cargo build --release -p veil` + `(Get-Item target\release\veil.exe).Length`, and `cargo tree -p veil --edges normal --prefix none` for dependency counts. Keystroke attribution detail is in `docs/review/keystroke-attribution.md`.

Conformance dimensions are distinct: **A** compares the top-level Markdown-generated block sequence; **B_EXPORT_EXACT** compares exported HTML exactly; **B_CANON** compares canonicalized HTML; **C** compares inline-node counts with the reference; **D** compares preorder inline-forest signatures. A/C/D passage does not imply exact HTML compatibility. CommonMark's 652 official examples and GFM's separate 23 examples are different corpora and are never combined into one compatibility score.

| Metric | Target | Historical result |
|---|---|---|
| Cold start (release, `main` → first frame; acceptance definition; 5 runs) | ≤ 300 ms | 145–190 ms (**5-run min–max, not median**; 2026-11-05, pre-v3 build; exact commit not preserved in the public summary; `VEIL_BOOT_LOG` segmented probe) |
| Cold start (in-process open + first full layout + first edit; p50 / max over 5 runs) | observation only | 405.7 ms (app 188.9 ms + first edit 216.8 ms; 2026-11-18, `cargo test -p veil-ui --release --lib -- --ignored --nocapture --test-threads=1 perf_cold_first_edit`). Same machine only: repeats vary between 290 and 580 ms because of the OS file cache |
| Cold start (bench harness, process create → open → first layout; p50 / max) | observation only | 576.46 / 599.98 ms (2026-11-09, `cargo run --release -p veil --example bench_typing`; baseline 520.87 / 533.85 → +10.7% / +12.4%, regression gate ≤ 20%) |
| Cold start (wall clock, process create → first frame; observation only) | ≤ 300 ms | 450–529 ms (440–470 ms before `main`, process-create overhead on this machine; 2026-10-31) |
| Cold start (window handle; P2 comparison definition) | — | 30 ms (2026-10-30, `tools/bench/measure.ps1`; a window-handle proxy, not first-frame or interactive latency) |
| Keystroke→commit p95 on 100 K-word doc | ≤ 16 ms | p50 2.231–2.268 ms · p95 2.556–2.615 ms · max ≤3.468 ms (`perf_20k_blocks` 21,000 blocks / 300 samples, 2 runs, historical build 2026-11-21 commit `0d229eb`; worst p95 2.615 ms ≤ 16 ms. Earlier 10k record: 0.973 / 1.566 ms) |
| Memory, 100 K-word doc (**Working Set / RSS**) | ≤ 150 MB | 40.76 MB (single observation after scrolling 20 pages; 2026-11-05, pre-v3 build; exact commit not preserved in the public summary; **not Private Bytes**) |
| Private Bytes, current v3.0.2 | — | **NOT RUN** in this wording review; do not substitute the Working Set/RSS value above |
| Single-file `veil.exe` size | ≤ 14 MiB = 14,680,064 B | Current v3.0.2 size **NOT RUN** here; use the matching Release and sidecar. Historical builds are not interchangeable (single product: offline Mermaid engine + full `mermaid.js` + resvg, lazy-initialised) |
| `veil-core` unit + property tests | ≥ 80 | 347 (historical summary, not a v3.0.2 rerun) |
| E2E assertions | ≥ 20 | 41 (historical summary: open→save→reopen, table+image paste, find/replace→crash recovery→self-clean) |
| Markdown roundtrip (no edits: output == input bytes; byte comparison) | 100% | 100% (historical summary, not a v3.0.2 rerun) |
| CommonMark 0.31.2 — 652 official examples, A / C / D | — | Historical public summary (2026-10-28, pre-v3): A 652/652 · C 652/652 · D 652/652 |
| CommonMark 0.31.2 — same 652 examples, B | — | Historical public summary (2026-10-28, pre-v3): B_EXPORT_EXACT 629/652 · B_CANON 630/652 (**not full exact/canonical HTML compatibility**) |
| GFM — separate 23-example corpus (Tables 8 / TaskList 2 / Strikethrough 2 / Autolinks 11) | — | Historical public summary (2026-10-28, pre-v3): A/C/D 23/23; B_CANON 12/23; the 11 autolink cases were observation-only in that summary. **Not a claim of full GFM compatibility** |
| `veil-core` GUI/async runtime deps | 0 | 0 (historical summary) |
| `veil-ui` + `veil` external direct deps (`cargo tree --depth 1`, distinct crate names) | ≤ 8 (pre-PDF baseline; superseded by the Wave 3 authorization for `pdf-writer` + `subsetter` + `ttf-parser`, and by v1.8's `regex`) | 17 (8 in `veil-ui` + 9 in `veil`; historical summary) |
| Workspace transitive deps (distinct crate names, normal edges incl. the three workspace members) | ≤ 200 | 125 (2026-11-18, `cargo tree -p veil --edges normal --prefix none`) |

## Same-machine Comparison (P2 public baseline, 2026-10-30)

Historical comparison of **Veil 0.1.0 and MarkText 0.23.0**, on the same machine, with the same generated P2 corpus and five-run medians. Generate the corpus with `tools/bench/gen_corpus.py`, then measure with `tools/bench/measure.ps1`; the corpus and results stay out of the repository (the pathological 10 MB file is generated on demand). Methodology and limitations are in [P2-benchmark.md](docs/review/P2-benchmark.md).

| Metric | Veil 0.1.0 | MarkText 0.23.0 | Comparison |
|---|---|---|---|
| **Private Bytes** (process-tree sum of `PrivateMemorySize64`; five-run median) | 21.51 MB | 365.01 MB | **17.0×** under this same metric and corpus |
| Artifact / installation footprint (**different definitions**) | 5.46 MiB, one `veil.exe` | 488.95 MB, total installation directory | **No ratio**: a single exe and an installation directory are not the same artifact |
| Cold start (process create → main window handle; five-run median) | 30 ms | 30 ms | Tie for this proxy only; **not first-frame or time-to-interactive equivalence** |

These are historical P2 measurements, not a current v3.0.2 comparison. In particular, the 40.76 MB Working Set/RSS observation above must not be compared with this table's Private Bytes values. For the current artifact, use its matching Release and `veil.exe.sha256`.

Dependency blacklist: `tokio`, `wasmtime`, any webview crate (`webview2-com`/`wry`, etc.). Introducing any of these is a violation.

## Terminology

One term per concept in the UI, in `config.toml` documentation and in this README. UI labels use Title Case; running prose uses lower case.

| Term | Meaning | Where it appears |
|---|---|---|
| **Rendered mode** | The default styled view of the Markdown document. | View → theme entries; the mode you return to with `Ctrl+/` |
| **Source mode** | The raw Markdown view of the same document (`Ctrl+/`); a view toggle, not a second document model. | View → Source Mode; status bar |
| **Body column** | The on-screen text column capped at a readable width and centred. Its presets and custom margin are screen-only. | Settings → Body Column; Settings → Custom Body Margin |
| **Page margin** | The physical margin of PDF/print output, in millimetres. Independent of the body column. | Settings → PDF Export Margin |
| **Interface language** | The language of menus, dialogs and notices; choosing a new one takes effect after a restart. Document text is never translated. | Settings → Interface Language |

## Settings Surface (audit)

Baseline: 2026-11-25, source version 3.0.2. The 17 independent top-level keys are listed once each. `doc_font_face` is folded into `doc_font_family`: it is the "explicit installed family" representation of the same single settings row, not an extra user-facing option.

| # | Key | UI label | Overlap? | Decision | Reason |
|---:|---|---|---|---|---|
| 1 | `mode` | Theme Mode | No | Keep | `system` / `light` / `dark` are three independent appearance choices; none is derivable from another. |
| 2 | `ui_language` | Interface Language | No | Keep | Independent of document content; restart-only by design. |
| 3 | `doc_font_family` (+ `doc_font_face`) | Body Font Family | Two representations of one row | Keep | A preset enum and an explicit installed family are mutually exclusive; dropping either loses installed-font selection. The panel already exposes one row. |
| 4 | `doc_font_size` | Body Font Size (10–32) | No | Keep | Independent typography value with its own clamp. |
| 5 | `doc_line_height` | (config file only) | No | Keep | Independent typography value; not exposed in the panel and not derived from the font size at read time. |
| 6 | `doc_margin` | Body Column | No | Keep | Screen body-column preset; unrelated to the PDF page margin. |
| 7 | `doc_margin_px` | Custom Body Margin | Parameter of `doc_margin = custom` | Keep | Only read in the Custom preset, but it is an independent number, not derivable from the four fixed presets. |
| 8 | `allow_remote_images` | Allow Remote Images | Same key also appears in the View menu | Keep | One persisted state with two entry points; not a duplicate key. |
| 9 | `source_plain_lines` | Source mode: plain-text line view | Modifies Source mode only | Keep | Optional line geometry inside Source mode; keeping it preserves the default block-layout source view. |
| 10 | `tab_width` | Tab Width (2 or 4) | No | Keep | Independent editing preference. |
| 11 | `export_margin_mm` | PDF Export Margin (mm, 0–50) | Not the same as `doc_margin` | Keep | Page margin vs. on-screen body column: different unit, surface and purpose. |
| 12 | `export_page_numbers` | Show Page Numbers in PDF | No | Keep | Independent PDF option. |
| 13 | `export_toc` | Include TOC Page in PDF | No | Keep | Independent PDF option. |
| 14 | `window_width` | Last Window Geometry (read-only) | One atomic geometry record | Keep | Four keys are read and written together; a partial record is rejected. |
| 15 | `window_height` | Last Window Geometry (read-only) | One atomic geometry record | Keep | Same record as row 14. |
| 16 | `window_x` | Last Window Geometry (read-only) | One atomic geometry record | Keep | Same record as row 14. |
| 17 | `window_y` | Last Window Geometry (read-only) | One atomic geometry record | Keep | Same record as row 14. |

**Result: 17 keep / 0 merge / 0 delete.** No semantically duplicated or strictly derivable key was found this round, so no capability was removed. The only true duplication (`doc_font_face` vs. `doc_font_family`) is already collapsed into one settings row at the UI layer while both representations remain necessary on disk.

## Document Status & Wording Register

Status baseline for this wording review: **source version 3.0.2, 2026-11-24**. This register prevents historical evidence from being read as current acceptance.

| Material | Status and permitted reading |
|---|---|
| `README.md` / `README.zh-CN.md` | Current feature and default documentation. Measurement and release figures above are historical; current v3.0.2 performance and release re-acceptance were not rerun in this review. |
| `docs/commercial/LANDING.md` / `LANDING.zh-CN.md` | Current public summary without unverified size or performance numbers; capability limits link back to these READMEs. |
| `docs/commercial/RELEASE-ARTICLE-3.0.zh-CN.md` | Historical v3.0.1 snapshot (2026-11-23), not current release evidence. |
| `PRIVACY.md` / `SUPPORT.md` | Current data and support boundaries: local reports are not automatically uploaded, remote-image opt-in persists, and Check for Updates only opens the Release page. |
| `SECURITY.md` | Security policy owned by separate security work; unchanged in this wording review. |
| `docs/review/evaluation/EVALUATION-FINAL-v3.0.md` | Historical v3.0.1 engineering-gate evaluation only; not evidence of aesthetics, reputation, adoption, revenue, or market status. |
| `evidence/release/v3-build.md` / `release-record.json` | Historical v3.0.0 build records; not valid for v3.0.2 size, SHA-256, or release PASS. The JSON remains unchanged. |
| `docs/review/P2-benchmark.md` | Historical 2026-10-30 Veil 0.1.0 / MarkText 0.23.0 comparison; retain its machine, corpus, run-count, metric and artifact definitions. |

## Docs

- `docs/review/v1-2-implementation-log.md` — v1.2 implementation record: measured numbers, probes, decisions
- `docs/review/keystroke-attribution.md` — keystroke-to-commit latency attribution
- `docs/qa/ignored-tests-2026-11-27.md` — every `#[ignore]` probe: file, type, why ignored, how judged, gate coverage
- `docs/qa/probe-gate-<date>.md` — generated probe-gate archive (command, timestamp, per-binary `test result` lines and output tail; verdict is cargo's exit code)
- `docs/qa/perf-distribution-2026-11-27.md` — min/p50/p95/max distributions for key paths, including 5000 headings and the 5000-snapshot history panel
- `docs/qa/last-mile-closeout-2026-11-28.md` — last-mile closeout: all six task states, evidence locations, open manual items and the single command
- `tools/verify/assisted/README.md` — assisted flows for DPI / physical print / real IME (the user only performs the physical action)
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
- `docs/roadmap/v1.1-roadmap.md` — v1.1 detailed plan
- `docs/roadmap/v1.2-roadmap.md` — v1.2 candidate pool (typography settings, PDF bookmarks, font subsetting)
- `docs/roadmap/v2.0-roadmap.md` — v2.0 detailed plan (wgpu deferred — four trigger conditions must all trip)

## Repository Structure

| Path | Purpose |
|---|---|
| `crates/` | Source code: `veil-core` (pure logic), `veil-ui` (rendering + interaction), `veil-mermaid-offline` (QuickJS + mermaid.js in-process), `veil` (product binary + platform seams) |
| `tools/` | Release, smoke, installer, sbom, walkthrough scripts — all PowerShell except the Python bench helpers |
| `icons/` | Application icon (`veil.ico`) and document-type icon (`veil-doc.ico`) |
| `docs/` | ADRs (`adr/`), public roadmaps (`roadmap/`: v1.1/v1.2/v2.0), strategy docs; `review/` baselines (G0/G1/G2/P0/P1/P2 + `evaluation/` subdirectory with 5 evaluation reports moved from root in final-polish) — retained as historical records, some describe superseded approaches |
| `evidence/` | **~343 MB** (gitignored): build artifact verification + audit trail. `render/` (237 MB, historical PNG render captures) + `ux/` (70 MB, interaction probes) = 89% of volume. Only summary records (release-record, signing-verification, sbom) are committed; raw screenshots/logs are local-only. B-grade cleanup policy in place |
| `.trae/` | Local-only development workspace (gitignored) — historical agent specs; never published |
| `.github/workflows/` | `ci.yml` (PR + main branch gates) and `release.yml` (`v*` tag publish + manual dry-run dispatch) |

## Privacy, Security & Support

Veil is distributed free of charge, under the MIT license. By design it contains **no telemetry and no background update checks**. **Help → Check for Updates** only opens the official Release page in the system browser; it is not an in-process update request. Remote images are off by default under a persistent setting: once enabled, subsequently opened documents containing them can make on-demand requests without another prompt. External links and mermaid.live require explicit user actions. Crash and diagnostic reports are generated locally and are not automatically uploaded; they leave the machine only when you choose to submit them. Trusted production signing and real SmartScreen behaviour remain **NOT RUN**; this commit's remote GitHub Actions evidence was not inspected in this review. A scoped local packet capture is recorded in [PRIVACY.md](PRIVACY.md) and is not a blanket zero-egress claim.

- [PRIVACY.md](PRIVACY.md) — what is and is not collected, local data locations, network capabilities and defaults, remote images, diagnostic reports, how to delete data
- [SECURITY.md](SECURITY.md) — supported versions (3.x), private vulnerability reporting, report contents and scope
- [SUPPORT.md](SUPPORT.md) — official source, SHA-256 verification, troubleshooting, what to include in an issue
- [THIRD-PARTY-LICENSES.md](THIRD-PARTY-LICENSES.md) — third-party components and license texts shipped with the release

## Trust & Verification Status

Veil is currently signed with the self-signed development certificate `CN=Veil Development (Long-term)` (SHA-1 thumbprint `86C8793A44EF01EE1633BCF769244894326B4526`; NotAfter `2031-11-23T05:06:11Z`) with an RFC 3161 timestamp. This certificate is trusted only on the current development machine; other machines will show an unknown-publisher warning and SmartScreen may warn or block.

The following external verifications remain **NOT RUN** — they require actions beyond this repository:

- **Trusted production signing**: Three candidate paths have been evaluated and documented in detail in the local-only file `evidence/renewal/production-signing-options.md` (not published in this repository): SignPath Foundation (free OV for qualifying open-source), Azure Artifact Signing (US$9.99/month), and commercial OV/EV (US$129+/year). All require identity verification and/or payment that only you can complete personally. No production certificate has been applied for or purchased.
- **Real SmartScreen with MotW downloads**: Requires a trusted production certificate above. Microsoft's SmartScreen reputation system accumulates over time; even with a trusted signature, the first release may still warn.
- **This commit's remote GitHub Actions evidence**: Not inspected in this wording review (**NOT RUN here**). The workflow files have been statically verified and include a `workflow_dispatch` entry with `dry_run=true` default; no current CI result is inferred from that static check.
- **Cross-machine trust revocation drill**: Depends on having a real CRL/AIA infrastructure that comes only with a trusted public CA or enterprise PKI. Self-signed certificates have no public revocation mechanism.

### Recommended next steps (when you are ready)

1. Read the four-path comparison in the local-only `evidence/renewal/production-signing-options.md` §7 ("Veil 推荐路线与下一步清单").
2. Determine whether SignPath (OSS foundation, free), Azure Public Trust (US$9.99/month, US/CA identity), or a commercial OV certificate fits your constraints.
3. Complete the identity verification / purchase yourself — the certificate and private key must never enter this repository or be accessible to any agent.
4. Once you have a production certificate, add its Base64 PFX and password as GitHub Repository Secrets (`VEIL_SIGNING_CERTIFICATE_BASE64`, `VEIL_SIGNING_CERTIFICATE_PASSWORD`) and configure the timestamp URL as `VEIL_SIGNING_TIMESTAMP_URL`.
5. Push a `v*` tag to publish a signed release, then download that release from a fresh Windows VM to verify publisher identity, SmartScreen behavior, and establish initial reputation.

## License

MIT License · see [LICENSE](LICENSE).
