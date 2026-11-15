# External Gateway Package

A standalone Personal Package for explicitly selected, Person-authorized Workspace reads. It needs no Agent Binding, model endpoint or model credentials. The Host derives the current Person and owns Port routing. This repository owns durable exact tool policy and independently revocable tokens.

## Current delivery

The Backend implements typed configuration/status, shared exact read policy, verifier-only tokens and bounded authenticated IPv4 HTTP. Main and standard settings views provide Workspace status, exact Package/tool selection, independent token controls and secret-free external-agent onboarding. The generic HTTP client is automated against the SDK capability seam; real-Host qualification and Don-owned native/external-agent observations remain pending. No real-Host or manual pass is claimed.

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

Read [AGENTS.md](AGENTS.md) before editing. `ui-contract.json` defines all closed UI operations; regenerate `ui/src/contract.generated.ts` with `npm run contract-types --workspace generated-package-ui`. `docs/http-api.md` records durable state and HTTP contracts and budgets. Do not edit the generated types or vendored SDK.

One shared policy applies to every valid token. Package enable alone selects no tools. Every ordinary read must be explicitly selected against its observed major, Provider and schema fingerprint. Its guide is included automatically. Added tools remain off; changed schemas/Providers/majors require review. Commands, streams and Conversations are refused before Provider dispatch. A live guide never grants authority. Removing a Package retains its data; disable and revoke all first when retained setup is unwanted.

## Provenance and dependencies

Foundation template 2 was generated from clean Host SDK revision `2ad3badb3e99845a23c699d18c656af9f14568a0`. `teamarness-authoring.json` records `sdkRevisionDirty:false`, Package Process 1.0, UI host 2 and four exact vendor digests. `npm run check:vendor` independently recomputes them. There is no in-place vendor upgrade. No H or vendor changes were made to implement gateway behavior.

All Rust/npm direct versions are exact. Cargo/npm lockfiles belong to this project. G-only rustix 1.1.4 (std/fs/process, default features off) was explicitly approved for safe descriptor-relative no-follow file handling and UID checks, using the already qualified Host lock version. Source notices and dual MIT/Apache licenses remain with vendored trees. New third-party crates retain their upstream licenses; see `docs/dependencies.md`.

## Security boundary

Configuration uses private owned files, bounded serialized writes and durable replacement. Only domain-separated SHA-256 token verifiers are stored. A bearer appears only in a successful create/regenerate response. Lost responses require regeneration. Do not store bearer values, private profiles or model credentials in this repository or evidence.

Native Packages run as the current OS user. This is not a sandbox or protection against hostile same-user native code. Local commits and builds do not establish installation, publication or manual usability.

## Configure and onboard

Open the standard Package settings view (or Settings in this UI). Enable the listener; its numeric URL appears only after bind and durable commit. Change the saved port explicitly if it is occupied. Package rows group contracts by Provider on each bounded page. Enable reviewed displayed contracts, then choose each ordinary read separately. New tools are off. A guide is included automatically; missing guides, Conversations, commands and streams cannot be enabled. Review changed schema/Provider/major choices explicitly. Unavailable saved choices can be removed with Disable Package; Host-only unavailable selections have an exact removal action.

Create a labeled token and transfer its one-time display to your outside client's local secret mechanism. Dismiss it when done; leaving settings or unmounting clears it. Lost create/regenerate responses require regeneration. All tokens share current tool scope; labels do not identify Principals. Already-authorized reads may finish after disable/revoke/regenerate.

The Overview renders [generic onboarding](docs/onboarding.md), the same Markdown served by authenticated `/v1/onboarding`. Save this guidance without a credential. Discover identifiers, complete schemas and fresh guide bytes before calling. No Backlog-specific client adapter or model setup is needed. The 12-operation bound counts outstanding gateway SDK operations, not simultaneous Provider executions; timeouts do not require a gateway restart or prove execution stopped. At most one delayed retry is permitted only for retryable ordinary reads when current Provider guidance permits; never automatically replay catalog/auth/policy/schema refusals or uncertain outcomes.

Refresh or page the catalog explicitly; only settings polls its displayed page, with one five-second-bounded refresh at a time and no accumulating all-Provider catalog. Closing settings cancels polling and pending observations, not Backend service. Setup and tokens survive restart/reinstall. Use **Disable and revoke all tokens** before removal when retained access is unwanted.
