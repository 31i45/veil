# Veil

**A single-file Markdown editor for Windows. Local-first. Zero telemetry. Your mind, your words, your data.**

## Why Veil

Veil is for writers, thinkers, researchers, and engineers who want distraction-free writing while keeping Markdown source under their control. Veil keeps documents local by default: no cloud sync, telemetry, or background update checks. Remote images are a separate, persistent opt-in; once enabled, opening a document containing them causes on-demand requests. External links and mermaid.live open only through explicit user actions.

## Core

- **Markdown with documented limits** — CommonMark 0.31.2 core + a GFM subset (Tables / TaskList / Strikethrough / Autolinks), with live incremental syntax highlighting. This is not a claim of complete Markdown compatibility; separate test corpora, conformance dimensions and known limits are documented in the [root README](../../README.md#historical-measurements-targets-vs-results)
- **Single portable exe** — No separate application runtime or installer required; Windows system components and system DLLs are still used. Optional per-user installer available
- **Mermaid diagrams rendered in-process** — No external network calls for graph rendering
- **Evidence tied to builds** — Historical performance measurements retain their dates, builds and methods in the root README; this wording review did not rerun them for v3.0.2
- **Zero telemetry upload** — No analytics or telemetry upload. Crash and diagnostic reports are generated locally and are not uploaded automatically
- **MIT licensed** — Source open at [github.com/31i45/veil](https://github.com/31i45/veil)

## Tech

- Rust 1.99.1 · Edition 2021
- Pure Windows 10/11 x64 · winit + softbuffer software rendering · QuickJS embedded runtime
- veil-core (cross-platform logic) · veil-ui (rendering) · veil-mermaid-offline (QuickJS + mermaid.js) · veil (product binary)

## Get it

[github.com/31i45/veil/releases](https://github.com/31i45/veil/releases)

SHA-256 verification script is in the root README. Currently signed with a self-signed development certificate `CN=Veil Development (Long-term)` (SHA-1 thumbprint `86C8793A44EF01EE1633BCF769244894326B4526`). Trusted production signing is planned — see the repository's Trust & Verification Status section.

## Privacy

Veil has no telemetry, cloud sync, or background update checks. **Help → Check for Updates** only opens the official Release page in your browser. Remote images are off by default under a persistent setting: once enabled, subsequently opened documents containing them can trigger on-demand requests without another prompt. External links and mermaid.live open only through explicit user actions.

## FAQ?

See [LICENSE-FAQ.md](./LICENSE-FAQ.md). Technical questions? File an issue on GitHub.
