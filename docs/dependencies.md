# Direct dependency notices

This project is MIT OR Apache-2.0. Vendored Teamarness trees retain their original LICENSE and source notices; `teamarness-authoring.json` pins their source revision/digests.

G-owned direct Rust dependencies use exact versions in Cargo.toml and Cargo.lock: base64, bytes, getrandom, http, http-body-util, hyper, hyper-util, jsonschema, mime, rustix, semver, serde, serde_json, sha2, subtle, tempfile and tokio. Their registry packages carry authoritative license metadata and complete upstream notices. The build does not copy or alter their source. Rustix 1.1.4 is Apache-2.0 OR Apache-2.0 WITH LLVM-exception OR MIT; the LLVM exception text is part of its upstream crate. This safe filesystem dependency is G-only and explicitly approved; it does not alter SDK provenance.

The npm workspace retains exact TypeScript, Vite, Vitest and ESLint toolchain pins and the relative vendored UI SDK. package-lock.json records the resolved graph. No model SDK, TLS server, MCP library or extra Provider implementation is included.
