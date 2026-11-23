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

[github.com/31i45/veil](https://github.com/31i45/veil). All releases are signed with the development certificate `CN=Veil Development (Long-term)` (SHA-1 thumbprint `86C8793A44EF01EE1633BCF769244894326B4526`).

### Signed / Trust status?

Current releases use a **self-signed development certificate**. Other machines will show an unknown-publisher warning. Full details and planned production signing path (SignPath Foundation — free OV for qualifying open-source, Azure Artifact Signing — US$9.99/month, commercial OV/EV — US$129+/year) are documented in the repository's Trust & Verification Status section and [evidence/renewal/production-signing-options.md](../../evidence/renewal/production-signing-options.md).

### Does Veil phone home?

**No.** Zero telemetry. No crash reports. No analytics. No silent updates. No background networking calls. Ever. See [PRIVACY.md](../../PRIVACY.md).

### Windows SmartScreen warning?

Because Veil currently uses a self-signed certificate, SmartScreen may warn or block downloads. A trusted production certificate will resolve this — the process is being actively evaluated.

### Who maintains Veil?

One person. All releases, all commits, all signing. No corporate involvement, no venture capital, no product managers.
