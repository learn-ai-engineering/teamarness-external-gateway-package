# External Gateway Package

A standalone Personal Package for explicitly selected, Person-authorized Workspace Ports. It needs no Agent Binding, model endpoint or model credentials. The Host derives the current Person and owns Port routing. This repository owns durable exact tool policy and independently revocable tokens.

## Current delivery

The Backend implements typed configuration/status, shared exact-tool policy, verifier-only tokens and bounded authenticated IPv4 HTTP. Main and standard settings views provide Workspace status, exact Package/tool selection, independent token controls and secret-free external-agent onboarding. The generic HTTP client is automated against the SDK capability seam; real-Host qualification and Don-owned native/external-agent observations remain pending. No real-Host or manual pass is claimed.

Native ordinary/streaming CLI access is also embedded in the same executable; see [CLI usage and client custody](docs/cli.md). Recognized managed command/skill preview, publication and recovery modules are now implemented and filesystem-tested. App-led setup, activation refresh wiring and native selection are implemented; installed/native qualification and Don-owned M1–M5 observations remain pending.

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

One shared policy applies to every valid token. Package enable alone selects no tools. Every operation must be explicitly selected against its observed major, Provider and schema fingerprint. Its guide is included automatically. Added tools remain off; changed schemas/Providers/majors require review. All four shapes are metadata-derived; Conversations is selectable, not enabled by default. Calls use /v1/call and confirmed streams use /v1/subscribe. A live guide never grants authority. Removing a Package retains its data; disable and revoke all first when retained setup is unwanted.

## Provenance and dependencies

Foundation template 2 received an authorized one-time canonical four-tree refresh generated from clean committed Host SDK revision `46a95356572edc38b3381bb349c9d66ede918fd0`. `teamarness-authoring.json` records `sdkRevisionDirty:false`, Package Process 1.1, UI host 3 and four exact vendor digests. `npm run check:vendor` independently recomputes them. There is no general in-place updater. Vendor source was not hand-patched. Confirmed streams require Provider process 1.1 support; legacy setup is refused clearly, without dropping ordinary compatible calls.

All Rust/npm direct versions are exact. Cargo/npm lockfiles belong to this project. G-only rustix 1.1.4 (std/fs/process, default features off) was explicitly approved for safe descriptor-relative no-follow file handling and UID checks, using the already qualified Host lock version. Source notices and dual MIT/Apache licenses remain with vendored trees. New third-party crates retain their upstream licenses; see `docs/dependencies.md`.

## Security boundary

Configuration uses private owned files, bounded serialized writes and durable replacement. The server stores only domain-separated SHA-256 token verifiers. Manual create/regenerate returns a one-time bearer; lost responses require regeneration. The separate machine-local ClientStore format 1 intentionally stores client bearer material in private owned files under the effective OS user’s home, outside the Package Data Directory. These are plaintext OS-user credentials, not encrypted or hostile-same-user protected. The typed Backend setup coordinator publishes credentials without bearer copying; native picker and both mount views use that coordinator. Do not store bearer values, private profiles or model credentials in this repository or evidence.

Native Packages run as the current OS user. This is not a sandbox or protection against hostile same-user native code. Local commits and builds do not establish installation, publication or manual usability.

## Configure and onboard

Main and Shell Configuration share the flat Gateway, Packages and Agent setup controls. Configuration adds token management below the Shell-owned header/tabs; no imitation tabs or editable-port control are rendered. The main gear requests only this Package’s own Configuration through optional mount navigation; older Hosts show a disabled gear with an update explanation. Enable the listener; its numeric URL appears only after bind and durable commit. If the saved port is occupied, dismissing the error changes presentation only: free that same port and explicitly retry. No fallback port is selected. Package rows group contracts by Provider on each bounded page. Enable reviewed displayed contracts before expanding their tools, then choose each exact tool separately (Read, Write, Streaming query or Event). New tools are off. A guide is included automatically; missing guides cannot be enabled. Review changed schema/Provider/major choices explicitly. Disabling a Package requires confirmation and removes all its saved tool choices; re-enabling does not restore them. Unavailable saved choices can be removed with Disable Package; Host-only unavailable selections have an exact removal action.

Create a labeled token and transfer its one-time display to your outside client's local secret mechanism. Dismiss it when done; leaving settings or unmounting clears it. Lost create/regenerate responses require regeneration. All tokens share current tool scope; labels do not identify Principals. Calls that passed final gateway local admission may still be sent to the Host and finish after disable/revoke/regenerate succeeds, including commands waiting inside the SDK. Later admissions use the new policy. Use the external agent's own stop control to stop its work; revocation is not cancellation.

Both views include expandable [generic onboarding](docs/onboarding.md), the same Markdown served by authenticated `/v1/onboarding`. Save this guidance without a credential. Discover identifiers, complete schemas and fresh guide bytes before calling. No Backlog-specific client adapter or model setup is needed. The 12-operation bound counts outstanding gateway SDK operations, not simultaneous Provider executions; timeouts do not require a gateway restart or prove execution stopped. At most one delayed retry is permitted only for retryable ordinary reads when current Provider guidance permits; never automatically replay catalog/auth/policy/schema refusals or uncertain outcomes.

Refresh or page the catalog explicitly; each mounted view polls its displayed page, with one five-second-bounded refresh at a time and no accumulating all-Provider catalog. Each admitted manual refresh shows progress and a fresh result; the refresh control stays disabled until the active read settles. Background polling does not reopen a dismissed success. Closing a view cancels polling and pending observations, not Backend service. Setup and tokens survive restart/reinstall. Use **Disable and revoke all tokens** before removal when retained access is unwanted.

## Membership and listener eligibility

The Package also requires existing `teamarness.membership` 1.0. One Backend-owned event watcher observes canonical membership independently of HTTP requests or UI visibility. It starts closed, subscribes from the initial roster epoch and revalidates before publishing a listener. Membership loss stops accepts and retires existing connections within five seconds after stop begins, including manifest/onboarding and cached-schema routes. It preserves desired state, saved port, tokens and selections. Confirmed removal stays stopped across recovery/restart/reload/reinstall; there is no promise of re-inviting a causally removed identity.

Monitoring failure is visibly unavailable, not confirmed removal. A usable stream may recover only on a valid later notification plus a fresh roster; a terminated stream needs explicit Package lifecycle recovery/reload. Recovery never overrides manual suspension or desired-disabled state. Already-authorized SDK calls may finish internally without delivering a response; no calls are replayed and no Provider-native cessation is promised. This watcher covers membership and its observation channel, not arbitrary Person presentation/storage failures that emit no membership signal. Real-Host correction qualification, M1–M5 (including membership in M4) and independent Verify remain pending.

## Writes and live streams

All active tokens gain every explicitly enabled operation. Writes act as the local Workspace Person; response loss can follow a committed write. Never replay a command automatically. Complete bounded Provider errors preserve versions/conflicts; follow the current guide to reconcile. Existing read fingerprints and manual tokens are preserved; version-1 policy migrates to version 2 only on the first managed authority commit. Upgrade alone grants no command or stream.

One authenticated NDJSON connection owns each confirmed subscription. Ready means Provider and Host registration completed. Wait for ready before the initial read. Values remain arbitrary JSON; EOF without end means interruption. Reconnect explicitly and reconcile: there is no durable replay, automatic reconnect or model-wakeup promise. Eight pending/active stream slots are separate from twelve call slots. Output and credit are bounded, quiet streams heartbeat after 15 seconds, and pending output has five seconds to flush. Policy/lifecycle loss ends affected idle streams; already-admitted bounded bytes may drain.

This phase is gateway-tested, not real-Host or outside-agent acceptance. Corrected-plan M1–M4 and independent Verify remain pending. Previous milestone M1–M5 is separate historical ownership.


## Managed access coordination (Backend)

Closed `setup.preview`, `setup.apply`, `setup.status` and `setup.watch` operations
coordinate connect, repair, rotate, disconnect, selected skill installation and
independent `install-cli`. The latter rejects folder selections, uses only the
recognized command installer's existing journal, and never reads/locks/publishes
ClientStore connection records, creates an origin, issues credentials or changes
Policy. Invalid private credential history is preserved; validated unfinished
credential work still reserves the coordinator. CLI operation feedback is
in-memory, not a new credential Journal/Completed form. No uninstall is provided.
Preview is read-only, accepts only an opaque folder selection, expires in ten
minutes and is limited to four plans. Apply rechecks consent and returns an
owned operation ID; cancelling a UI observer does not cancel accepted work.
The native folder picker supplies a bounded mount-scoped selection lease; both views use the same coordinator controls.

`setup.inspect` separately observes current CLI recognition/candidate change,
Workspace authority/local publication, and pending/invalid credential recovery.
It is read-only, performs disk/hash work off the async runtime, and rejects raced
Policy revisions without hiding independently observed CLI facts. No bearer,
private record path, origin nonce or opaque identity is returned. Connected means
an exact current verifier/private-record pair and committed saved endpoint, not
a running listener or the latest operation result. Missing private credentials
require explicit replacement; intact credentials with an endpoint mismatch need
publication repair. Tombstones never mean connected. Inspection creates no
files, takes no mutation locks, and neither recovers nor reclaims temporary files.
Status/watch retain their existing operation-history shape; legacy bundled
preview/apply requests remain supported internally. Both views use one coalesced,
revision-guarded inspection chain, not a filesystem poll or a hash on every
progress event.

Policy remains the only authorization owner. A private client operation journal
precedes recognized outputs, verifier commit, and atomic credential/endpoint
publication. Results distinguish actual partial effects. Uncommitted proposals
need explicit repair; recovery may finish only a still-current committed
operation. Revoke/disable-all/supersession cannot recreate old authority.
Rotation replaces one managed generation in its existing token slot. Raw manual
regeneration refuses managed IDs; manual tokens otherwise work unchanged.
Disconnect revokes first, then removes only its matching local record; cleanup
pending is not complete. Deleting a local file alone never revokes access.

First managed authority commit durably migrates policy to configVersion 2,
preserving port, desired state, grants and manual verifiers/generations. Older
Backends cannot read it: downgrading or deleting policy is not rollback. Client
store/CLI major 1 remain independent. Activation/reload refreshes only an intact,
recognized compatible command owned by this Installation, strictly newer than
the installed version; equal bytes/version is unchanged. Consent, ownership
transfer, same-version changes and collisions retain explicit refusal rules.
See [CLI custody and recovery](docs/cli.md). All automated Backend/CLI setup
uses injected private physical homes; installed production-home M1–M5 are pending.

Agent setup in both views separates **CLI**, **Workspace connection** and the
optional **Agent skill**. Use **Install CLI** (or **Update CLI**) independently of
credentials; there is no uninstall. A recognized exceptional CLI replacement asks
for one short confirmation with human-readable versions. Modified or unrelated
outputs cannot be replaced here. Command path is available in a disclosure.

Use **Connect**, **Rotate credentials**, **Disconnect** or **Repair** as offered
by current connection facts. These named actions internally prepare and accept
one exact plan; there is no routine Preview/checkbox/Apply screen. Connect and
rotation require a running listener and a current issue-free CLI; setup never
starts the listener or grants tools. Stopping the listener does not erase an
intact connection. Missing credentials need explicit rotation; endpoint-only
repair retains intact credentials. Revoked/disconnected access is not connected.
Pending/invalid/conflicting state is shown separately from historical results.

The optional skill helps an agent discover and use live Workspace tools. With
the current CLI installed, **Install skill…** opens the native folder chooser;
confirming the folder installs only the skill, and Cancel installs nothing.
No folder is attached to credential actions. Connect, rotation and skill
installation refuse any implicit CLI change: use Install/Update CLI first.
Repair explicitly resumes previously accepted unfinished work. If it requires
command publication, one short version/output confirmation resumes that reserved
work; the separate CLI action cannot bypass it. Manual tokens
and explicit denial recovery remain in Configuration. Closing a view cancels
its picker and observations, not an accepted setup operation. See [native picker integration](docs/native-picker.md)
for protocol/lifecycle limits and the still-pending installed/native qualification.

Catalog refresh never sends a listener enable/port-change intent and does not
restart a live listener or close its streams. The Backend port-change operation
remains available to declared callers; after a committed change, managed setup
reports repair-required. Explicit port-only repair republishes the saved endpoint
without rotating an intact credential. Production revised A, packaged gear/focus,
acquired native setup and actual harness use remain Don-owned M1–M5; fixture tests
and canonical vendor comparison are not installed/manual acceptance.
