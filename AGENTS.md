# Working on External Gateway

This repository contains the standalone Teamarness Package `dev.teamarness.external-gateway`. Read this file before changing its contract, Backend, UI, persisted data, dependencies, or build output.

## The system this Package joins

Teamarness is a local-first, peer-to-peer team workspace. A team can use one desktop application while Packages provide separate product capabilities.

The Teamarness Core Runtime owns authority and behavior shared by Packages:

- Installation identity;
- Workspace creation and selection;
- canonical Workspace membership;
- Package lifecycle; and
- implementations of declared Semantic Capabilities.

The Teamarness Shell owns desktop navigation and Package UI mounting. The Package Host admits Package bytes, starts Package Backends, routes declared requests, supplies Workspace context and data paths, and supervises Backend processes.

This Package owns its product domain. That includes its request schemas, domain rules, validation, records, migrations, projections, policy, Backend behavior, and UI. Keep product-specific behavior here rather than expecting a change to the Teamarness Core Runtime, Shell, or Package Host.

## Runtime path

A normal request follows this path:

```text
Package UI
  -> mount-scoped PackageClient
  -> Teamarness Package Host
  -> supervised Package Backend
  -> Package-owned behavior or data
  -> response through the same declared contract
```

`ui-contract.json` defines the requests that may cross this path. Treat it as the shared boundary between the UI and Backend. Change the contract before implementing a new request on either side.

This Package consumes only `teamarness.workspace-context` 1.0, `teamarness.package-ports` 3.0 and `teamarness.membership` 1.0. It has eager activation, main and settings views, closed typed configuration requests and `external-gateway.status.watch`. It provides no Ports and declares no known consumed Ports. It supports the targets `aarch64-apple-darwin` and `x86_64-unknown-linux-gnu`. The source manifest declares a placeholder Backend for each, and a native-only build assembles a Package Directory that declares only the target it built (`scripts/targets.mjs`). The Backend requires process 1.1 and serves bounded authenticated IPv4 HTTP and confirmed NDJSON; the UI provides main/settings views, bounded catalog controls, one-time tokens and secret-free onboarding.

## Project map

| Path | Responsibility |
|---|---|
| `teamarness.package.json` | Declares Package identity, compatibility, Backend artifacts, UI artifacts, the UI contract, assets, and required capabilities. |
| `ui-contract.json` | Contains closed JSON schemas for every UI request, response, subscription request, and event. |
| `backend/src/lib.rs` | Owns domain behavior, lifecycle handling, validation, and Package data. |
| `backend/src/main.rs` | Starts the Backend through the vendored SDK. Keep it thin. |
| `backend/tests/` | Exercises Backend behavior through the SDK test harness, with no protocol frames. |
| `ui/src/index.ts` | Mounts the Package UI and communicates through the supplied `PackageClient`. |
| `ui/src/styles.ts` | Holds this Package's CSS, adopted into the mount and removed on unmount. |
| `ui/src/contract.generated.ts` | Generated request and response types. Regenerate it; never edit it. |
| `ui/src/index.test.ts` | Exercises UI behavior with the UI SDK's fake Package client. |
| `scripts/build-package.mjs` | Runs `npm run build`: selects the native target, then builds, assembles and validates it. |
| `scripts/targets.mjs` | Lists the supported Backend targets, parses the shared `--target <triple>` argument, reads the native target from `rustc -vV` (or `$RUSTC`), and refuses a non-native or unsupported target. |
| `scripts/assemble-package.mjs` | Assembles final Backend, UI, contract, and manifest bytes for one native target into `dist/`. Imported by `build-package.mjs` and its tests; it has no command line of its own. |
| `scripts/validate-dist.mjs` | Applies Package Admission's rules to `dist/` for the native target, smoke-launches the Backend, then smoke-tests its picker and CLI entry modes. |
| `scripts/verify-vendor.mjs` | Fails when a vendored SDK file no longer matches recorded provenance. |
| `scripts/headless/external-gateway-setup` | Headless answers-file setup (Bash plus `python3 -I`). It drives existing Host operator commands and Gateway requests, read-first and with no retries. The build copies it to `dist/external-gateway-setup`, beside the Package Directory and never inside it. `scripts/headless-setup.test.mjs` drives it against `scripts/headless/fake-host.py`. |
| `vendor/teamarness/` | Contains the generated copies of the Package Process Protocol and SDK source used by this project. |
| `teamarness-authoring.json` | Records template, protocol, UI host, SDK provenance, and request namespace information. |
| `Cargo.lock` and `package-lock.json` | Lock this project's Rust and JavaScript dependency graphs. Commit both. |
| `dist/` | Generated Package Directory output. Never edit it by hand. |

The root Cargo and npm workspaces belong to this Package project. This project must build without a Teamarness checkout path.

## Design rules

### Start with the contract

For each product operation:

1. Give it a lowercase dotted name in `ui-contract.json`.
2. Define separate, closed request and response schemas.
3. Add Backend behavior and tests.
4. Add UI behavior and tests.
5. Build and inspect the final Package Directory.

Use IDs for stable domain choices and labels for presentation. Reject unknown fields where the contract allows. Version persisted schemas and plan migrations before changing stored records.

Do not add a request that bypasses the product model merely because it is convenient for the UI. Prefer a small operation that states user intent and lets the Backend enforce invariants.

### Keep domain behavior in the Backend

The Backend should own:

- domain rules and calculations;
- validation at trust boundaries;
- random seed or identifier creation when those values affect product meaning;
- persisted records and schema migrations;
- save-time verification and regeneration; and
- background work that must outlive a visible UI mount.

Prefer deterministic pure functions for rules that can be expressed that way. Test them with independent examples, invalid inputs, and boundary values.

When generated output includes a seed, revision, mode, or other value that is meant to affect the result, test the result itself. Use fixed inputs at the pure behavior seam and prove both directions: equal inputs produce equal output, and selected unequal inputs produce unequal output. A test is wrong if only metadata changes while the meaningful output stays fixed.


The Backend receives its Package Data Directory at activation, as `Activation::data_directory()`. Use that supplied path. Do not hard-code an application-data path, inspect Teamarness databases, or store Package records in the project directory.

A Package Data Directory belongs to one Package Identity, Workspace, and Installation. Data is not global across Workspaces. Teamarness does not inspect, migrate, back up, delete, synchronize, or share Package-owned files for you.

Keep lifecycle operations bounded. The vendored SDK handles the Package Process Protocol and control-frame machinery: build a `Router`, register typed handlers, and let the activation hook produce the state those handlers share. Do not hand-write protocol frames.

### Validate before you build

`npm run validate` applies Package Admission's manifest and UI contract rules to this project and prints the host's verdict, with the rule that failed and where. Run it after every contract or manifest change, before building. `npm run build` assembles `dist/` and then runs the same rules against the assembled Package Directory, with a Backend smoke launch.

### Blocking work and cancellation

Backend handlers run on an async runtime. Anything that blocks — a file read or write, SQLite, hashing, a directory walk — goes inside `ctx.blocking(...)`, which moves it off that runtime and hands the closure a cancellation token.

The host cancels a request, and the SDK cancels its token, when the caller cancels, when the request's deadline passes, and when the Package is suspended or disposed. Long work checks `cancel.is_cancelled()` and returns early. A handler that blocks the runtime freezes every other request, including the host's health checks.

### Errors

A Package error code is namespaced and stable: `external_gateway.storage_failed`, not "error" and not a sentence. It reaches the Package UI unchanged, with `retryable` and bounded `details`, so the UI can tell "try again" from "this cannot work".

Mark an error `retryable` only when repeating the same request may succeed. Invalid input is not retryable; a failed write may be.

The SDK owns the codes that are not about this product: `package.not_active`, `package.unknown_operation`, `package.unknown_subscription`, `package.invalid_payload`, `package.blocking_task_failed`, and `cancelled`. Do not redefine them.

### Testing the Backend without frames

`backend/tests/` drives the Backend through `teamarness_package_sdk::testing::{HarnessBuilder, Harness}`: activate, request, subscribe, suspend, recover, dispose, shutdown — with a real temporary Package Data Directory and no hand-built frames. `assert_router_matches_contract` proves the router and `ui-contract.json` declare the same operations, so a contract change without a handler fails the tests.

### Typed UI requests

The UI calls `typedPackageClient<PackageContract>(context.client)`, whose types come from `ui-contract.json` through `teamarness-contract-types`. Change the contract first, then run `npm run contract-types --workspace generated-package-ui` to regenerate `ui/src/contract.generated.ts` and commit it. `npm run typecheck` fails when the committed types are stale, and the UI cannot name an operation or build a payload the contract does not describe.

Adopt Package CSS with `adoptPackageStyles(context.root, ...)` and drop it in the unmount. Present a failure with `describePackageError(error)`, and ignore an aborted request with `isAbortError(error)`.

### Keep the UI mount-scoped

Export `mount(context)` from the ESM entrypoint and return an idempotent unmount function.

Use `context.root` for rendered content and `context.client` for Package requests. Clean up listeners, subscriptions, timers, and DOM nodes during unmount. Present loading and request failures explicitly.

The UI should own interaction and presentation, not durable product authority. Do not calculate canonical records in the UI. The UI runs in a sandboxed frame, so Tauri is unreachable, browser storage and the network are unavailable, and the Shell-owned DOM is out of reach; `context.client` is the only way out.

Use semantic controls and accessible names. Mark purely decorative content outside the accessibility tree. Test behavior through user-visible state and PackageClient requests rather than internal function calls.

### Treat P2P and capabilities as explicit dependencies

Teamarness is a P2P application, but that does not make Package records synchronized. Do not open direct peer connections or assume access to Core Runtime transport, identities, keys, membership state, databases, or secrets.

A Package uses Teamarness-owned authority only through versioned Semantic Capabilities declared in `teamarness.package.json` and granted by the host. Add a capability only when the product requires it. Define the behavior and failure cases before changing the manifest.

Package source, trust, Workspace enablement, synchronization, sharing, networking, and collaboration are separate decisions. A local registration provides none of them automatically.

## Artifact rules

`npm run build` creates `dist/dev.teamarness.external-gateway/`. That Package Directory is the unit Teamarness validates and runs. The assembler writes final sizes and lowercase SHA-256 digests after copying the final files.

Do not edit `dist/` after assembly. Any byte change makes the manifest declaration stale.

The assembler copies one Backend executable, for the native target, `ui/dist/index.js`, and `ui-contract.json`. The manifest declares no assets. Keep the UI inside the single ESM bundle unless the product needs a different artifact shape.

If you add UI assets, another Backend, or another target platform, update all of the following together:

- `teamarness.package.json`;
- `scripts/assemble-package.mjs`;
- `scripts/targets.mjs` (`supportedTargets`), for another target platform;
- build dependencies and commands; and
- tests that verify final paths, permissions, sizes, and digests.

Do not add this project to the Teamarness Cargo or npm workspaces. Do not edit Teamarness application Package metadata for an installation-local Package.

## Vendored SDK policy

The source under `vendor/teamarness/` was copied from the Teamarness checkout that created this project. It makes the project independent of that checkout.

Treat vendored SDK files as external infrastructure. Do not modify them to implement Package behavior or to work around a contract mistake. Change Package code first. If an SDK defect blocks correct implementation, report the exact defect and the smallest upstream change needed.

`teamarness-authoring.json` records which checkout the copy came from — `sdkRevision`, `sdkRevisionDirty` — and a digest of every vendored tree. `npm run check:vendor` recomputes those digests as part of `npm test`, so a failure there means a vendored file was edited: restore it rather than weakening the check. From a Teamarness checkout, `npm run create:package -- --vendor-report <project>` reports which trees differ from that checkout, and changes nothing.

There is no supported in-place SDK upgrade yet. Teamarness SDK fixes do not arrive in this repository automatically. Preserve source notices, license metadata, relative dependency paths, and `teamarness-authoring.json` provenance.

## Security model

A Package Backend runs with the current user's ambient filesystem, network, and process authority. Process supervision is a recovery boundary, not a sandbox. Avoid unnecessary dependencies and operating-system access. Validate paths and untrusted input.

A Package UI runs in a sandboxed frame with an opaque origin. Tauri is unreachable, the Shell's DOM and storage are unreachable, and the frame's own policy denies network requests, nested frames, form navigation, and inline code. Treat the mount root and PackageClient as the supported interface. Frame isolation separates realms, not threads: a UI that blocks its thread freezes the Shell.

Review both Backend and UI dependencies before shipping or registering the Package.

## Working method for agents

Before editing:

1. Read `teamarness.package.json`, `ui-contract.json`, and `teamarness-authoring.json`.
2. Read the relevant Backend and UI tests.
3. Read the local SDK README for any interface you will use.
4. State the product behavior, ownership boundary, files to change, and tests to add.
5. Confirm whether the artifact shape or declared capabilities must change.

While editing:

- work only in this Package project;
- keep the contract, Backend, UI, and tests aligned;
- write a failing behavior test before production behavior when the seam is testable;
- use real temporary directories for persistence tests;
- keep lockfile changes deliberate; and
- do not weaken schemas, tests, permissions, or digest checks to make a build pass.

Before reporting completion, run:

```sh
npm run validate
npm run typecheck
npm run lint
npm test
npm run build
```

`npm run lint` already runs `cargo fmt --all --check` and `cargo clippy`.

Then inspect `dist/dev.teamarness.external-gateway/` and report:

- which behavior changed;
- which contract operations changed;
- which persisted schema or migration changed;
- whether capabilities or artifact shape changed;
- the tests and checks that passed; and
- any remaining runtime or manual verification.

Do not claim synchronization, team sharing, sandboxing, another target platform, or compatibility beyond what the manifest and tests prove.

## Gateway policy and storage

`backend/src/policy.rs` is the only authorization owner. UI and eventual HTTP callers must use it, never maintain an independent grant cache. `catalog.rs` uses major-3 Person discovery and route preconditions. Queries are never retried. Metadata pages are consumed incrementally; complete documents are bounded and released rather than collected into an all-Provider catalog.

`config.rs` reads version-1 and owns managed version-2 state under the supplied Data Directory. Descriptor-relative no-follow opens and ownership checks use exact rustix 1.1.4 with default features off and std/fs/process features. This G-only dependency was explicitly approved to meet the accepted storage contract; the version and checksum came from the qualified Host lockfile. No SDK source is patched. Keep the rustix license files in the dependency registry/distribution and preserve all vendored notices.

`tokens.rs` uses OS entropy, domain-separated SHA-256 and fixed-size constant-time verifier comparison. Never log bearer values or persist them in server Policy/configuration. The separate `client_store.rs` intentionally persists private client bearer material under the effective OS user’s home; keep that custody distinct from verifier-only server state. Token labels are not principals. All tokens share exact tool policy. All four metadata-derived shapes, including Conversations, require exact explicit selection. Package enable alone selects none; upgrade alone grants no commands or streams. Commands act as the Person, never replay automatically, and preserve authoritative uncertainty plus complete Provider-owned details within 4,096 bytes/depth 8. Gateway failures after possible command dispatch are uncertain and nonretryable. A schema field named description remains schema data; only schema annotation keywords are removed from fingerprints.

One owned policy transaction outlives cancelled UI observers. Failed authorization writes reread storage, keep access closed and retain only the intersection of old and requested authority until a successful durable action reconciles state. A later unrelated mutation must not resurrect a requested revocation or publish a failed enable. Invalid configuration is never overwritten. File fault tests use real files at write/fsync/rename/directory-fsync boundaries.

Phase 4 implements bounded HTTP, first-port bind/persist/publication and listener lifecycle. Twelve slots count outstanding local SDK operations, including guides; every SDK terminal releases a slot. Observer loss alone does not. No Host settlement tracking or quarantine. Phase 5 implements complete settings and shared onboarding presentation. `docs/onboarding.md` is bundled into UI and Backend; never add credentials to it. Catalog UI retains only one summary/tool page and has at most one refresh. Null-provider package changes are disable-only for exact currently unavailable contracts; saved Provider identities are presentation/grouping, never authorization. Final real-Host qualification and Don-owned M1–M5 remain pending.

`membership.rs` owns one activation-scoped membership subscription and sequential typed roster observations. Start unknown/closed, read current, subscribe after its durable epoch, and revalidate before listener publication. Events invalidate admission before waiting on policy ownership. Membership eligibility, lifecycle-active state and persisted desired-enabled are independent; a late observation or write cannot undo manual suspension. Confirmed removal is latched for the activation and remains absent on later canonical snapshots after restart. Monitoring failures are unavailable, not removal; a valid later notification may recover a live stream, while a terminated stream needs explicit lifecycle recovery/reload. Never reconnect on a timer, infer identity from transition digests, replay calls, or patch the SDK. Snapshot work remains singly owned through the existing SDK terminal/deadline. Final disposal cancels/joins the watcher; stop retires every route/connection through the existing five-second drain. This gate does not promise idle shutdown for unsignaled Person presentation errors or retract delivered bytes.

C3 recovery discards only an abandoned owned 0600 regular `state.tmp` during a serialized write after validating committed state; never promote its contents or reclaim on open/read. One live Backend generation per Data Directory and cancellation-surviving policy write ownership are required. HTTP alone wraps opaque Host cursors in canonical unpadded base64url (2,731 ASCII characters / 2,048 decoded UTF-8 bytes); UI/Host cursors and the 4,096-byte target limit remain unchanged.

Final gateway local admission, not SDK writer enqueue, is the accepted revocation boundary. An admitted call may wait inside the SDK, hand off and finish after a later revoke/regenerate/permission change succeeds. Later admissions use current policy. Do not add call revocation barriers, pending states, call cancellation or extra SDK machinery; use the external agent’s stop control to stop its work. CR-002 remains valid historical evidence against the superseded enqueue requirement, not a falsified finding.

Corrected full-access Phase 2 refreshes the four canonical vendor trees once from committed H SDK `8f3a64367988c8860527ce3fbcc9a676d2646c1d` via the existing generator. This authorized generated snapshot is not permission to hand-patch vendor files, add a general updater, or depend on a Host checkout. Process 1.1 is required; capabilities, configVersion 1, bearer format, read fingerprints, artifact shape and approved rustix pins remain unchanged.

`streams.rs` owns eight pending/active subscriptions separately from twelve calls. Confirmed SDK methods precede HTTP 200/ready. Legacy/unconfirmed setup refuses clearly, never fabricates ready. Policy owns token-generation/exact-tool registration and every output admission. Idle permission/lifecycle loss signals streams before drain; failed storage closes all. Pending observer loss retains capacity through SDK terminal and late success closes. Use recv_with_credit with one waiting frame plus one writer-owned frame, 256 KiB original values plus 4 KiB framing, five-second pending-write flush and 15-second heartbeat. No owner is held over socket/credit/cleanup waits; no detached handles, durable replay, automatic reconnect or model wakeup. Already-admitted bounded bytes may drain. Provider Markdown stays unchanged; structured runtime.guide remains callable. Native Epic/Collection prerequisites and Provider versions/conflicts stay Provider-owned. Final real-Host qualification and Don-owned corrected-plan M1–M4 remain pending.

`cli/` owns interaction major 1, raw-byte JSON/NDJSON and numeric-loopback Hyper client access. `cli/stream.rs` serializes one observation, validated ready and terminal selection; descriptor-level nonblocking stdout has one bounded pending line, a five-second write budget and no detached writer. Subscriptions require readiness-registered pipe/terminal stdout; regular/non-pollable output refuses before network (exit 2, stderr JSON/hint), never silently falling back to blocking disk I/O. Continuous mode has no CLI lifetime cap; the 45-second valid-frame liveness clock pauses under local output backpressure. `client_store.rs` owns private format-1 connection files, origin conflicts and cross-process publication. Production home lookup is a bounded (2 s) fixed OS account query: macOS `/usr/bin/dscacheutil`, Linux `/usr/bin/getent passwd <euid>` with an empty environment. Neither reads HOME or accepts a public path flag; any failure is `cli.home_unavailable` with no fallback (the Backend maps it to `external_gateway.home_unavailable`, which fails activation). Tests inject physical homes through constructors or the unassembled fixture example. CLI help/version/licenses/skill do not resolve home, start Host stdio, listener or AppKit. `setup.rs` owns recoverable managed access coordination; Policy alone owns authorization. `backend/build.rs` embeds the Package manifest version; Cargo crate version, CLI major and store format are independent.


`managed_install.rs` owns recognized executable/skill publication safety shared by
`skill_install.rs`, not authorization. Preview is read-only; callers run apply and
recovery as owned blocking work. Supplied provenance comes from the current Host
Initialization (never a filesystem manifest). Automatic-refresh API admits only
intact same-owner strictly-newer compatible bytes; equal bytes/version is a no-op,
and transfer/same-version change/downgrade needs explicit preview. Unrecognized or
modified outputs and future/invalid receipts never become repair authority.
Private bounded old/new journals plus the shared nonwaiting OS installer lock
reconcile partial receipts; never promote temporary bytes or claim rollback.
`backend/resources/teamarness/SKILL.md` is embedded portable live guidance with one
literal absolute quoted command, no bearer/default/store path/Provider schema.
Selected folder identities are revalidated; no registry or previous-folder scan.
A pre-journal death can leave a private random staging orphan; retry never adopts
or reclaims it. `setup.rs` wires actual owning-Installation activation/reload refresh and explicit
coordinator consent. All automated Backend constructors must inject private
physical homes, including regression harnesses. No automated test may resolve,
read or install into the ordinary home. The unassembled `backend-fixture` Backend
entry requires canonical owned0700 `TEAMARNESS_FIXTURE_ROOT` before SDK stdio.
At SDK activation its home is the safe owned0700 child named lowercase hex SHA-256
of canonical lowercase Installation identity UTF-8 (not decoded key bytes); all
Workspaces of that Installation share it. Copied basename-tmx fixture CLI requires
`TEAMARNESS_FIXTURE_HOME` set to that exact existing child, never rederives it or
falls back. Picker dispatch precedes fixture environment checks. Production accepts
no override and never reads fixture environment. Both entries share fixed strict
pre-runtime mode selection, including the actual ordered Host launch arguments.

`setup.rs` retains four ten-minute read-only previews and one owned unfinished
operation. Its private client journal precedes recognized outputs, the narrow
Policy verifier/association commit and atomic credential+endpoint publication.
Never commit after a required output fails. Exact current committed recovery may
finish publication; never-committed proposals need explicit repair. Managed
revoked/disconnected tombstones prevent resurrection. Rotation replaces the same
slot/generation and raw manual regenerate refuses managed IDs. Disconnect revokes
before only matching local cleanup; uncertain revoke is not success. Port-only
repair preserves intact credentials. No second authorization owner, automatic
handover or grants/listener enablement is added. Lock order is ClientStore before
bounded Policy, never inverse; no socket/dialog/observer waits hold owners.
Lifecycle closes admission before Policy suspension and joins owned disk work.
Selected paths persist only in unfinished work; completed receipts are secret-free
and do not retain selection paths. Config v1 migrates only at managed authority
commit to v2, preserving manual authority/port/grants/desired. Invalid/future
state and unsupported downgrade remain untouched. CLI/store major 1, capabilities,
artifact shape and Package version are independent and unchanged by Phase 4.
Production picker/UI remains Phase 5; M1–M5 and real-Host qualification remain pending.

`picker.rs` owns one admitted helper child per Instance and at most four selected
mount-scoped leases. Its empty-input setup subscription establishes promptly;
there is no browsing lifetime limit. The one selected event ends native work,
not the selection lease: close/reselect/unmount/lifecycle invalidates selections
and unaccepted plans, while accepted apply remains independently owned. Child
kill/reap and lifecycle joins are explicit. On macOS `picker_helper.rs` dispatches
before Tokio, initializes AppKit and runs synchronous RFD on the OS main thread; its
fixed private request pipe is also the parent-EOF lifeline. Other targets have no
native chooser: `Picker::choose` refuses `choose-skill-folder` before capacity or
spawn with non-retryable `external_gateway.picker_unavailable`, and the helper
emits `picker_failed`. Keep 4-KiB path, 8-KiB frame/stderr and five-second
terminal/cleanup budgets, no debug leakage or arbitrary helper/script arguments.
`picker-fixture` is unassembled fault/lifeline evidence, never a claim of actual
AppKit or installed behavior. Package version 1.1.0 is manifest-owned; CLI/store
major 1, process 1.1, capabilities and artifact shape remain independent.
Native focus/installed M1–M5 remain Don-owned pending.
`cli::THIRD_PARTY_NOTICES_JSON` embeds literal dependency notices and source provenance;
`tmx --licenses` retrieves them offline before runtime/home/network/AppKit, as
`tmx --skill` prints the portable skill for manual installation. Preserve
upstream text/checksums when changing native dependencies; assembled checks compare
the entire resource in both CLI entry modes, without adding an asset category.

Onboarding Phase 7 adopts one canonical generated four-tree snapshot from committed
H `46a95356572edc38b3381bb349c9d66ede918fd0` (dirty:false), UI mount contract 3.
No vendor source is hand-patched. Preserve G's exact rustix/native dependencies and
locks; no Host checkout path is a build dependency. `controls.ts` owns the shared
main/Configuration presentation, while Policy alone owns authorization. Settings
adds tokens below Shell-owned header/tabs; only main renders the optional own-
Configuration gear. No internal view tabs or editable-port UI. Catalog refresh
never enables/restarts the listener. Bind dismissal is presentation-only; explicit
retry uses listener.set true only while stopped on the committed saved port.
Package disable requires confirmation. One summary/tool page and one active
refresh remain bounded; closing either view clears observations, picker leases,
secrets and timers, never an accepted setup apply. Manual M1–M5/native packaged
qualification remain pending, not inferred from generated provenance or UI tests.

Setup simplification Phase 1 adds internal `install-cli` and separate read-only
`setup.inspect`; production UI simplification remains a subsequent phase.
Command-only plans have no Policy revision/connection material and branch before
Policy snapshots, ClientStore connection reads/locks, or credential-derived
invalid flags. Valid unfinished credential work still reserves the one owner.
Use only ManagedInstaller's existing old/new journal for command publication and
activation recovery, including when private credential observation fails. Never
persist InstallCli in setup Journal/Completed, invoke credential recovery after
its failure, create/read an origin, issue tokens, migrate Policy or publish a
skill/connection in that path. Invalid credential history remains untouched.
Inspection reuses installer recognition independent of candidate verification;
all disk/hash work is blocking, with a final Policy revision guard and no owner
over disk. It creates/cleans/recovers nothing, takes no mutation lock and returns
no credential/private-record path/opaque identity. Connection truth matches the
current association/verifier/private record and saved port, never running state
or last operation history. Missing bearer needs explicit replacement, endpoint
mismatch needs publication repair, and tombstones remain denied. Status/watch
remain historical; no filesystem monitoring, new durable schema or uninstall.


Setup simplification Phase 2 uses `setup.inspect` for compact independent CLI,
current Workspace connection and optional skill presentation. `setup.ts` owns one
coalesced revision-guarded inspection chain on mount, relevant Policy changes and
terminal/action observations, never a hash per progress event or filesystem poll.
Routine named actions internally preview then accept one exact plan; no visible
Preview/checkbox/Apply ceremony or opaque provenance. Connect/Rotate/skill require
an issue-free current CLI and an unchanged command plan; they never hide command
publication or attach a folder to credential actions. Install/Update CLI itself
consents to the exact recognized output change; no second CLI confirmation is
shown. Collisions remain refusal. Progress, errors and independently dismissible
results belong above the controls in each CLI/connection/skill section. Required
command publication in a valid already accepted pending Repair retains its
targeted confirmation to avoid a recovery deadlock; only Repair can resume those
reserved outputs, never the independent CLI action. On macOS, Install skill…
opens the native chooser and selection immediately prepares/applies only the
skill, holding its lease through acceptance. On other targets the chooser
request refuses with `external_gateway.picker_unavailable`. Cancel and
stale/unmounted observations never apply; accepted work still outlives the view.
Current connection facts never come from operation receipts or listener running
state. Manual token one-time-secret cleanup and explicit denial recovery remain
available under storage failure. Catalog manual refresh visibly acknowledges each
admitted click and disables its stable control through the active read terminal;
polling preserves dismissal, identical notifications retain their dismissal node,
and errors are never hidden by a success dismissal. No uninstall, Backend/contract,
SDK, capability, dependency or durable-format change belongs to this UI phase.
Native/manual and real-Host qualification remain pending.
