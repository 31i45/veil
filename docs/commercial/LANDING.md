# Veil

**A single-file Markdown-first IDE for Windows. Offline. Zero telemetry. Your mind, your words, your data.**

## Why Veil

Veil is for writers, thinkers, researchers, and engineers who want distraction-free writing without giving up the power of Markdown. Unlike every other writing app, Veil runs entirely offline — no cloud, no telemetry, no background networking calls, no silent updates. Your words stay where you put them.

## Core

- **Markdown, evolved** — CommonMark 0.31.2 + GFM extensions (Tables / TaskList / Strikethrough), live incremental syntax highlighting
- **Single portable exe** — ≈13.3 MiB, no installer required, no external DLLs. Optional per-user installer available
- **Mermaid diagrams rendered in-process** — No external network calls for graph rendering
- **Snappy** — Cold start 145–190 ms (main → first frame; 5-run median), keystroke p95 2.6 ms (21,000 blocks, 300 samples), 100k-char 150% DPI scroll peak 28.83 MB
- **Zero telemetry** — No analytics, no crash reports, no phone-home. Ever
- **MIT licensed** — Source open at [github.com/31i45/veil](https://github.com/31i45/veil)

## Tech

- Rust 1.99.1 · Edition 2021
- Pure Windows 10/11 x64 · winit + softbuffer software rendering · QuickJS embedded runtime
- veil-core (cross-platform logic) · veil-ui (rendering) · veil-mermaid-offline (QuickJS + mermaid.js) · veil (product binary)

## Get it

[github.com/31i45/veil/releases](https://github.com/31i45/veil/releases)

SHA-256 verification script is in the root README. Currently signed with a self-signed development certificate `CN=Veil Development (Long-term)` (SHA-1 thumbprint `86C8793A44EF01EE1633BCF769244894326B4526`). Trusted production signing is planned — see the repository's Trust & Verification Status section.

## Privacy

Veil never phones home. No background networking. No silent updates. No telemetry. Ever.

## FAQ?

See [LICENSE-FAQ.md](./LICENSE-FAQ.md). Technical questions? File an issue on GitHub.
