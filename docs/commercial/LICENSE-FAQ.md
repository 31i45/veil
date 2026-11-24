# Veil — License & Trust FAQ

### What license is Veil under?

MIT. See [LICENSE](../../LICENSE). You can use Veil commercially, modify it, redistribute it, and even sublicense it under the MIT terms. No corporate entity or CLA — the entire project is the work of one maintainer.

### Why MIT?

MIT is the most permissive and most widely understood open-source license. Veil is a personal tool that grew into a product. The maintainer believes your words belong to you, and your tools should not hold them hostage. MIT keeps it simple: do whatever you want, just don't sue anyone.

### Where can I see the third-party license list?

Full third-party license inventory (Rust crates, Mermaid.js, linkedom, QuickJS bundled assets) is in the root [THIRD-PARTY-LICENSES.md](../../THIRD-PARTY-LICENSES.md). Updated on every release via `tools/third_party_notices.ps1`.

### Is Veil free for commercial use?

Yes. MIT permits commercial use without restrictions. You can ship Veil as part of a commercial product, charge money for it, or build on top of it.

### Where is the source code?

[github.com/31i45/veil](https://github.com/31i45/veil). The current portable `veil.exe` and per-user installer `veil-setup-<version>.exe` are signed with the development certificate `CN=Veil Development (Long-term)` (SHA-1 thumbprint `86C8793A44EF01EE1633BCF769244894326B4526`).

### Signed / Trust status?

The current portable exe and installer use a **self-signed development certificate** trusted only on the current development machine. Other machines will show an unknown-publisher warning. Trusted production signing and real SmartScreen validation remain **NOT RUN**. Candidate production signing paths (SignPath Foundation — free OV for qualifying open-source, Azure Artifact Signing — US$9.99/month, commercial OV/EV — US$129+/year) are documented in the repository's Trust & Verification Status section.

### Does Veil phone home?

Veil contains no telemetry or analytics and has no background update checks, but it is not accurate to say that it never makes network requests. Crash and diagnostic reports are generated locally and are not automatically uploaded; they leave the machine only when you choose to submit them. **Help → Check for Updates** only opens the official Release page in the system browser. Remote images are off by default; once the persistent setting is enabled, subsequently opened documents containing remote images are fetched on demand without another prompt. See [PRIVACY.md](../../PRIVACY.md).

### Windows SmartScreen warning?

Because Veil currently uses a self-signed certificate, SmartScreen may warn or block downloads. A trusted production certificate does not guarantee that SmartScreen will stop warning: its reputation system accumulates over time, and real Mark-of-the-Web download validation remains **NOT RUN**.

### Who maintains Veil?

One person. All releases, all commits, all signing. No corporate involvement, no venture capital, no product managers.
