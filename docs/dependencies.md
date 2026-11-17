# Direct dependency notices

This project is MIT OR Apache-2.0. Vendored Teamarness trees retain their original LICENSE and source notices; `teamarness-authoring.json` pins their source revision/digests.

G-owned direct Rust dependencies use exact versions in Cargo.toml and Cargo.lock: base64, bytes, getrandom, http, http-body-util, hyper, hyper-util, jsonschema, mime, rustix, semver, serde, serde_json, sha2, subtle, tempfile and tokio. Their registry packages carry authoritative license metadata and complete upstream notices. The build does not copy or alter their source. Rustix 1.1.4 is Apache-2.0 OR Apache-2.0 WITH LLVM-exception OR MIT; the LLVM exception text is part of its upstream crate. This safe filesystem dependency is G-only and explicitly approved; it does not alter SDK provenance.

The npm workspace retains exact TypeScript, Vite, Vitest and ESLint toolchain pins and the relative vendored UI SDK. package-lock.json records the resolved graph. No model SDK, TLS server, MCP library or extra Provider implementation is included.

## Native CLI ordinary transport

Pinned Hyper 1.11.1 additionally enables its HTTP/1 client; the existing pinned Tokio 1.53.1 adds signal handling. Hyper adds locked MIT-licensed `try-lock` 0.2.5 and `want` 0.3.2. Serde JSON enables raw-value validation to preserve original numeric bytes. Existing pinned jsonschema validates shared HTTP envelopes, not Provider authorization. No additional HTTP stack is introduced. Effective-user home lookup uses the macOS system utility `/usr/bin/dscacheutil` through a bounded fixed-argument OS adapter, avoiding unsafe passwd FFI or another directory dependency; it is a system runtime dependency, not an interpreter or bundled artifact. Apple Silicon macOS remains the only declared target. Existing rustix pins/features and SDK provenance remain unchanged.


## Recognized managed outputs

Managed output publication reuses the approved rustix filesystem/UID/OS-lock
surface, getrandom OS entropy, SHA-256 and serde strict bounded receipts/journals.
The existing exact semver 1.0.28 pin now also supplies runtime Package-version
precedence comparison (build metadata is not a strictly newer release). No new
crate, lock resolution, interpreter, registry, capability or artifact category is
introduced. Skill Markdown is embedded in the same native executable; final
validation checks its complete bytes. Effective-user home resolution retains the
bounded D1 system adapter. Automated output tests inject private physical roots;
production-home setup is not part of build or smoke validation.
