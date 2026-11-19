# Direct dependency notices

This project is MIT OR Apache-2.0. Vendored Teamarness trees retain their original LICENSE and source notices; `teamarness-authoring.json` pins their source revision/digests.

G-owned direct Rust dependencies use exact versions in Cargo.toml and Cargo.lock: base64, bytes, getrandom, http, http-body-util, hyper, hyper-util, jsonschema, mime, rustix, semver, serde, serde_json, sha2, subtle, tempfile and tokio. Their registry packages carry authoritative license metadata and complete upstream notices. The build does not copy or alter their source. Rustix 1.1.4 is Apache-2.0 OR Apache-2.0 WITH LLVM-exception OR MIT; the LLVM exception text is part of its upstream crate. This safe filesystem dependency is G-only and explicitly approved; it does not alter SDK provenance.

The npm workspace retains exact TypeScript, Vite, Vitest and ESLint toolchain pins and the relative vendored UI SDK. package-lock.json records the resolved graph. No model SDK, TLS server, MCP library or extra Provider implementation is included.

## Native CLI ordinary transport

Pinned Hyper 1.11.1 additionally enables its HTTP/1 client; the existing pinned Tokio 1.53.1 adds signal handling. Hyper adds locked MIT-licensed `try-lock` 0.2.5 and `want` 0.3.2. Serde JSON enables raw-value validation to preserve original numeric bytes. Existing pinned jsonschema validates shared HTTP envelopes, not Provider authorization. No additional HTTP stack is introduced. Effective-user home lookup uses the macOS system utility `/usr/bin/dscacheutil`, or on Linux `/usr/bin/getent passwd`, through a bounded fixed-argument OS adapter, avoiding unsafe passwd FFI or another directory dependency; it is a system runtime dependency, not an interpreter or bundled artifact. The supported targets are `aarch64-apple-darwin` and `x86_64-unknown-linux-gnu`. Existing rustix pins/features and SDK provenance remain unchanged.


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

## Native chooser (Package 1.1.0)

The chooser and its dependencies are macOS-only: RFD, objc2 and objc2-app-kit are
`[target.'cfg(target_os = "macos")'.dependencies]` in `backend/Cargo.toml`. Root
workspace pins and Cargo.lock are unchanged, and the Linux Backend compiles none of
them. Exact approved direct pins are RFD `=0.16.1` (default features off), objc2
`=0.6.5`, and objc2-app-kit `=0.3.2` (default features off, `std` and
`NSApplication`). The latter explicit dependency initializes AppKit on the helper's
OS main thread before synchronous RFD. No async dialog, portal, interpreter,
network stack or extra artifact is introduced. RFD itself selects additional
AppKit features needed by its native panel; our explicit features do not remove
those transitive requirements.

Reviewed Cargo metadata, upstream manifests and retained registry license files:
RFD, objc2/objc2-encode, objc2-foundation and block2 are MIT;
objc2-app-kit/core-foundation and dispatch2 are Zlib OR Apache-2.0 OR MIT;
raw-window-handle is MIT OR Apache-2.0 OR Zlib; log is MIT OR Apache-2.0.
These are the ten newly compiled native dependencies (including the three direct
pins). Cargo.lock deliberately also resolves target-conditional futures-task/util
(MIT OR Apache-2.0), slab (MIT), wasm-bindgen-futures/web-sys (MIT OR Apache-2.0),
and windows-sys 0.60.2/windows-targets 0.53.5 plus eight architecture support
packages (MIT OR Apache-2.0). They do not add supported targets. Existing locked
versions are retained; js-sys gains its newly enabled futures-util dependency.
Other crates retain their upstream LICENSE/LICENCE notices in registry source;
objc2 family crate archives instead reference their repository-level LICENSE.md.
Its exact release-tag text was obtained/reviewed for objc2-0.6.5,
objc2-encode-4.1.0, block2-0.6.2, dispatch2-0.3.1 and frameworks-0.3.2;
the latter tag is shared by AppKit/Foundation/CoreFoundation. All five literal
texts have SHA-256 `7f976f7e9cb2d87df7230606feb932c3f21ac0e664045a775b600046ff850c54`.
That text retains its Apple SDK licensing discussion rather than asserting a new
license clearance. Required notices are embedded in
`backend/resources/THIRD_PARTY_NOTICES.json` with per-component source URLs, tags,
registry versions, authors and literal-text checksums. The static resource also
retains encountered existing production notices, including rustix LLVM exception
and Teamarness notices (114 native normal dependency components across both
supported targets, 83 distinct literal notices). The component list is the union
of `cargo tree -e normal` for `aarch64-apple-darwin` (113 components) and
`x86_64-unknown-linux-gnu` (104 components): ten chooser components are macOS-only,
and `linux-raw-sys` 0.12.1, rustix's Linux backend, is Linux-only. Its three
license files are byte-identical to rustix's stored texts and are referenced by
SHA-256; its COPYRIGHT names linux-raw-sys, so that text is stored literally. A
given binary may not contain every listed component. Missing split-crate notices use the identical repository
release source, not invented text. No general notice generator/updater was added.
Installed users retrieve the complete resource via offline `tmx --licenses`;
final assembly compares those bytes in explicit and basename CLI modes. Thus
source-free delivery retains notices without adding another artifact category;
vendored Teamarness notices are unchanged. No license or digest check is waived.
Final `otool -L` (macOS) or `ldd` (Linux) inspection records system library
dependencies separately from native/manual acquired-artifact execution qualification.

The resource is a JSON-escaped UTF-8 string so literal upstream CRLF/trailing
spaces are retained without whitespace-check exceptions. The 297,656 decoded
bytes have SHA-256 `6bee8926c87f2657779ce17a234f958c7c57440e60bf97e572e659f38dfc4fe7`;
per-notice original byte hashes remain inside the decoded provenance. Exact raw
RFD source notice is also an independent test fixture. Native tests pin original
CRLF/trailing spaces; assembly compares a separate JavaScript JSON decode against
the Rust CLI output and the encoded resource against the executable bytes.

Encoded resource SHA-256: `596018d981d7fe5f0ed1d9ab26778c2c68382d932a7874932cdcc0b24f326770`.
