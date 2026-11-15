# External Gateway Package

A standalone Personal Package for explicitly selected, Person-authorized Workspace reads. It needs no Agent Binding, model endpoint or model credentials. The Host derives the current Person and owns Port routing. This repository owns durable exact tool policy and independently revocable tokens.

## Current delivery

Phase 3 implements typed configuration/status operations, incremental major-3 catalog discovery, shared exact tool selections and verifier-only credentials. It starts no HTTP listener. Main/settings currently show only a mount-scoped status view. The HTTP contract, full settings and real-Host qualification follow in phases 4–6; no end-to-end or manual pass is claimed.

## Build and check

Use Rust 1.99.1 and Node 24.20.0 from this standalone project:

```sh
npm ci
npm run validate
npm run check:vendor
npm run typecheck
npm run lint
npm test
npm run build
git diff --check
```

The build performs locked release compilation, UI assembly, final manifest/digest validation and a Backend smoke handshake. `npm test` includes vendor integrity, exact direct pins, contract/generated-type checks, locked workspace Rust tests, UI tests and assembler tests. A smoke handshake is not an HTTP or real-Host proof.

The assembled Package Directory is `dist/dev.teamarness.external-gateway/`. Never edit it. `dist/`, `target/`, and `node_modules/` are ignored. No Teamarness checkout or sibling Package is a build dependency.

## Contracts and source ownership

Read [AGENTS.md](AGENTS.md) before editing. `ui-contract.json` defines all closed UI operations; regenerate `ui/src/contract.generated.ts` with `npm run contract-types --workspace generated-package-ui`. `docs/http-api.md` records durable state and planned HTTP contracts/budgets. Do not edit the generated types or vendored SDK.

One shared policy applies to every valid token. Package enable alone selects no tools. Every ordinary read must be explicitly selected against its observed major, Provider and schema fingerprint. Its guide is included automatically. Added tools remain off; changed schemas/Providers/majors require review. Commands, streams and Conversations are refused before Provider dispatch. A live guide never grants authority. Removing a Package retains its data; disable and revoke all first when retained setup is unwanted.

## Provenance and dependencies

Foundation template 2 was generated from clean Host SDK revision `2ad3badb3e99845a23c699d18c656af9f14568a0`. `teamarness-authoring.json` records `sdkRevisionDirty:false`, Package Process 1.0, UI host 2 and four exact vendor digests. `npm run check:vendor` independently recomputes them. There is no in-place vendor upgrade. No H or vendor changes were made to implement gateway behavior.

All Rust/npm direct versions are exact. Cargo/npm lockfiles belong to this project. G-only rustix 1.1.4 (std/fs/process, default features off) was explicitly approved for safe descriptor-relative no-follow file handling and UID checks, using the already qualified Host lock version. Source notices and dual MIT/Apache licenses remain with vendored trees. New third-party crates retain their upstream licenses; see `docs/dependencies.md`.

## Security boundary

Configuration uses private owned files, bounded serialized writes and durable replacement. Only domain-separated SHA-256 token verifiers are stored. A bearer appears only in a successful create/regenerate response. Lost responses require regeneration. Do not store bearer values, private profiles or model credentials in this repository or evidence.

Native Packages run as the current OS user. This is not a sandbox or protection against hostile same-user native code. Local commits and builds do not establish installation, publication or manual usability.
