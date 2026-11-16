# External Gateway HTTP v1 reference

The Backend serves HTTP/1.1 on `127.0.0.1:<saved-port>`. It starts disabled. All routes require one `Authorization: Bearer <BEARER>` header. The UI uses the Package bridge, never HTTP.

## Authenticated routes

- `GET /v1`: manifest and Workspace identity.
- `GET /v1/onboarding`: generic instructions with a bearer placeholder.
- `GET /v1/contracts`: enabled contract pages.
- `GET /v1/contracts/{contract}/{major}`: complete projected schemas.
- `GET /v1/contracts/{contract}/{major}/guide`: live Provider guide.
- `POST /v1/subscribe`: selected event or streaming query, with the same closed selection body.
- `POST /v1/call`: closed `{contract, major, port, request}` body, `{result: <original JSON value>}` response.

Ordinary queries, commands, streaming queries, and events from any guided Provider are selectable, including Conversations. Each exact tool requires explicit selection; Package enable alone selects none. The Host selection, Provider, major and canonical schema fingerprint must still match. Description-only edits preserve selections. Added tools stay off. Every active token shares the current selections but is independently revocable. The Host derives the current Person; clients cannot choose an actor, Provider or Workspace. A call that passed final local admission may still be sent to the Host and finish after a later policy change succeeds, even if it was waiting inside the SDK. Revocation does not cancel or roll back an admitted command.

`/v1/call` accepts only response queries and commands. `/v1/subscribe` accepts only streaming queries and events. Exact authorization precedes shape checks. Events require `request:null`, which is not forwarded to the event SDK method. Streaming queries carry their declared request JSON. Unknown fields, including actor, kind, Provider and Workspace, are refused.

## Durable configuration

Private `config/state.json` uses configVersion 1, a 0700 subdirectory and owned 0600 regular files. Descriptor-relative no-follow opens, file fsync, atomic rename and directory fsync protect writes. Invalid, future or unsafe committed files remain unchanged and fail closed. Before a serialized write, an abandoned owned 0600 regular `state.tmp` is discarded; its contents are never promoted. Opening or reading alone never reclaims it. Recovery relies on one live Backend generation per supplied Data Directory and the policy transaction retaining exclusive write ownership after observer cancellation. Unsafe temporary paths fail closed rather than being replaced. Limits: 1 MiB, 256 selected contracts, 1,024 selected tools, 32 tokens, 96 UTF-8 label bytes. No plaintext credential is persisted. `eg1.<id>.<secret>` uses 16 random ID bytes and 32 random secret bytes, base64url without padding. Verifiers hash `external-gateway.token.v1\0`, ID bytes, generation as big-endian u64, and secret bytes, with SHA-256 and constant-time comparison.

A failed persistence operation never reports success. An uncertain operation rereads storage and keeps access closed. Pending denials are retained in memory until a successful durable user action reconciles them. Removal retains data. Disable and revoke all before removal if retained access is unwanted. Native code running as the same OS user is outside this protection boundary.

## Transport budgets

Timeout and interval values are seconds; byte values are bytes. The schema and Rust constants must match this table. These limits apply independently of Host route, Provider and byte budgets.

| Constant | Value |
|---|---:|
| `MAX_CONNECTIONS` | 32 |
| `MAX_HEADER_BYTES` | 16384 |
| `MAX_HEADERS` | 64 |
| `MAX_TARGET_BYTES` | 4096 |
| `MAX_HOST_CURSOR_BYTES` | 2048 |
| `MAX_HTTP_CURSOR_CHARS` | 2731 |
| `REQUEST_READ_TIMEOUT` | 5 |
| `RESPONSE_WRITE_TIMEOUT` | 5 |
| `REQUESTS_PER_SECOND` | 8 |
| `REQUEST_BURST` | 16 |
| `MAX_HOST_CALLS` | 12 |
| `MAX_STREAMS` | 8 |
| `STREAM_HEARTBEAT_SECONDS` | 15 |
| `STREAM_WRITE_SECONDS` | 5 |
| `STREAM_FRAME_ALLOWANCE_BYTES` | 4096 |
| `TOOL_WAIT_TIMEOUT` | 25 |
| `PORT_VALUE_BYTES` | 262144 |
| `ENVELOPE_ALLOWANCE_BYTES` | 4096 |
| `METADATA_BYTES` | 1048576 |
| `METADATA_CHUNK_BYTES` | 65536 |
| `SUMMARY_ROWS` | 32 |
| `SUMMARY_BYTES` | 32768 |
| `METADATA_CACHE_BYTES` | 16777216 |
| `MAX_CATALOG_RESTARTS` | 1 |
| `CATALOG_POLL_INTERVAL` | 5 |
| `CATALOG_READ_TIMEOUT` | 5 |
| `LISTENER_DRAIN_TIMEOUT` | 5 |

## Response contracts

`contracts/http-v1.schema.json` defines closed request and response envelopes. `contracts/http-v1.examples.json` contains validated examples. Successful responses use `application/json`:

| Route | Response definition |
|---|---|
| `/v1` | `manifest`: Workspace, version, readOnly:false, supportedShapes, discovery/subscription links and value/call/stream limits |
| `/v1/onboarding` | `onboarding`: generic Markdown with a `<BEARER>` placeholder |
| `/v1/contracts?cursor=...` | `contracts`: at most 32 currently usable selected contracts and nullable `nextCursor` |
| `/v1/contracts/{contract}/{major}` | `schema`: complete filtered document, original metadata digest and actual projection digest |
| `/v1/contracts/{contract}/{major}/guide` | `guide`: unedited live Markdown, original-byte SHA-256, contract major/minor and exposed-operation preface |
| `/v1/call` | `result`: the original JSON value, including arrays, scalars, null and envelope-like objects |

POST accepts exactly `application/json`, without a content encoding. The serialized original `request` and result each have a 256 KiB budget. The surrounding call envelope has an additional 4 KiB allowance. Schema responses retain complete reachable definitions and never truncate a successful document. A guide whose complete preface and original Markdown exceed the value budget plus allowance is refused with `too_large`, never truncated. Metadata cache capacity is 16 MiB of serialized documents, using least-recently-used eviction. A hit still requires current offered metadata/revision and current exact policy. Cached compilation has bounded nonwaiting admission; cancelled observers do not release its blocking-work permit early.

A catalog page can be empty while `nextCursor` is non-null. Pass that exact opaque cursor as `?cursor=<nextCursor>` without decoding it. HTTP wraps the unchanged opaque Host cursor in canonical base64url without padding: at most 2,731 ASCII characters encoding at most 2,048 UTF-8 bytes. The UI/Host cursor is not changed, and the request target remains limited to 4,096 bytes. Malformed, empty, noncanonical or oversized envelopes are refused before catalog dispatch; a valid envelope containing a stale Host cursor preserves `port_catalog_changed`. Internal contract lookup restarts a stale catalog at most once. No invocation is replayed after a stale revision. Disabled or stale-schema contracts expose neither schemas nor summaries.

## Refusals and transport policy

Errors are `{error:{source,code,retryable,outcomeUncertain,details}}`. Source is `gateway`, `host`, or `provider`. Host codes remain intact. A Provider error exposes its stable code and flags, with complete Provider-owned details up to 4,096 compact JSON bytes and depth 8. Invalid or over-budget details become `{}` as a whole; fields are not filtered or truncated. Uncertainty forbids a retryable verdict. Unrelated top-level Host diagnostics are omitted. Provider details are untrusted domain data, like successful results; the gateway cannot scrub arbitrary Provider JSON for secrets. It never injects bearer values, headers, private paths or internal diagnostics.

Local pre-dispatch refusals are certain. Explicit Host/Provider certainty remains authoritative. Invocation-local possible command dispatch is marked just before the first SDK poll under policy ownership, not at an enqueue receipt. Gateway observation timeout, lost completion or result serialization/size failure after that mark reports `outcomeUncertain:true` and `retryable:false`. A missing command response can follow a committed write. No command is replayed automatically.

| Gateway code suffix | HTTP status |
|---|---:|
| `unauthorized` | 401 |
| `bad_request` | 400 |
| `forbidden` | 403 |
| `not_found` | 404 |
| `method_not_allowed` | 405 |
| `unsupported_media_type` | 415 |
| `too_large` | 413 |
| `rate_limited` | 429 |
| `busy` | 503 |
| `timeout` | 504 |
| `catalog_changed` | 409 |
| `unavailable` | 503 |
| `storage_failed` | 503 |

Host `deadline_exceeded` uses 504; `port_catalog_changed` uses 409. Other Host/Provider errors use 503 alongside their machine-readable code. The gateway never retries work automatically.

Every application response has `Cache-Control: no-store`, `X-Content-Type-Options: nosniff` and a closed connection. Parser, incomplete-header, ambiguous-framing and connection-capacity refusals may close without an application response. Hyper owns HTTP framing. A bounded raw-header preflight refuses duplicate lengths and simultaneous Content-Length/Transfer-Encoding before Hyper can normalize them. Header bytes include the request line and terminating CRLFs. Exactly one numeric configured Host and one Authorization are required. Origin (including `null`), absolute-form targets, cookies, token query parameters, upgrades, trailers, batching, unsupported encodings and unknown call fields are refused. No CORS, redirects or WebSocket surface exists.

## Confirmed NDJSON subscriptions

A successful `POST /v1/subscribe` uses HTTP/1.1 chunked `application/x-ndjson`, no-store and nosniff. Before establishment, refusals use ordinary JSON/status responses, never HTTP 200 followed by fabricated ready. G requires process 1.1. Confirmed SDK opening succeeds only after Provider registration and Host Consumer registration. A legacy Provider or unconfirmed upstream refuses with `capability_version_unsupported`; this is a protocol requirement, not a Package-name exclusion. The Host has one 20-second setup budget; the SDK fallback is 21 seconds and the gateway observer ceiling remains 25 seconds.

Each compact frame ends in one newline. Closed schemas and independent examples cover every frame:

```json
{"type":"ready","contract":"dev.example.sample","major":1,"port":"sample.events","kind":"event","delivery":"stream"}
{"type":"value","value":null}
{"type":"heartbeat"}
{"type":"end","reason":"completed"}
{"type":"end","reason":"error","error":{"source":"host","code":"provider_exited","retryable":false,"outcomeUncertain":false,"details":{}}}
```

Stream terminal errors preserve the received stable code. Namespaced Provider codes (for example, `proof.stream_failed`) use source `provider`; bare Host codes and the reserved `package.*` SDK and `external_gateway.*` gateway codes keep their source. A stream-close carries no Provider details or recovery flags, so Provider terminals have empty details, `retryable:false` and `outcomeUncertain:false`.

Ready precedes all values. Values preserve arbitrary JSON roots and envelope-like objects. A terminal is best effort and emitted at most once. EOF without end means interruption, not successful completion. Reconnect explicitly, wait for ready, then read/reconcile according to the live Provider guide. There are no replay IDs, cursors, detached handles, delivery acknowledgements, automatic reconnects, automatic call replay or model-wakeup guarantees. A restart loses subscriptions, not saved selections.

Eight stream slots count pending and active subscriptions separately from twelve calls. An abandoned opening retains capacity until the actual SDK terminal; late success closes without exposure. The policy registry stores token ID/generation and exact contract/major/Provider/fingerprint, never plaintext bearer. Every value admission checks current policy under its mutation owner. Revoke/regenerate, exact-tool or Package disable and listener/lifecycle/membership stop signal affected idle streams. Failed persistence closes all streams. Unrelated tokens/tools stay live; already-admitted bounded bytes may drain. Cleanup holds no policy owner while waiting on SDK close, credit or socket output.

`recv_with_credit` and a capacity-one handoff permit at most one waiting frame and one HTTP-writer-owned frame beyond unchanged SDK buffers. Credit is returned after bounded acceptance, not external receipt. Original values retain the 256 KiB budget plus 4 KiB frame allowance. Control frames share bounded output. Ordinary responses retain one five-second whole-response clock. Streams instead have five seconds for pending output write/flush; quiet waits do not start that clock. A heartbeat follows 15 seconds without outgoing frames. Slow/disconnected peers release ownership without blocking other streams.

The convenience guide preserves Provider Markdown byte-for-byte and adds a shape-aware shared-scope preface. Structured guide fields remain available through an ordinary `runtime.guide` call. Native Epic and Collection setup remain prerequisites; Documents create/revise are separate commands with opaque Version Tokens, and Exhibit Draft updates use expectedRevision and complete file lists. Provider guides own those semantics; no generic transaction or version field is invented.

## Ownership and lifecycle

`MAX_HOST_CALLS` means 12 outstanding gateway SDK call operations, including guide reads and their metadata checks. The 13th is refused without a waiting queue. It does not mean 12 simultaneous Provider executions: unchanged Host route, Provider and byte budgets can refuse earlier.

The 25-second HTTP budget is an upper observation ceiling. Earlier Host, SDK and Provider terminals return immediately. Every SDK terminal releases the local slot, including timeout, unavailability and overload. A disconnected or timed-out HTTP observer does not release a still-pending local operation. Suspend closes accepts and drains connection observers for at most five seconds. Pending SDK operations remain counted across recovery; disposal/shutdown retires local tasks. A completed SDK timeout does not quarantine capacity or require restart. None of these terminals proves that arbitrary Provider-native work stopped.

First enable binds an IPv4 ephemeral port, durably saves it and desired enabled state, then publishes running status. Status `running` plus saved `port` identifies `http://127.0.0.1:<port>/v1`; no URL is published before persistence. Later activation/recovery binds only the saved port. A conflict leaves desired setup intact with `listener_bind_failed` and no running endpoint. An explicit port change closes the old endpoint and publishes the new one only after bind and durable commit; failure leaves stopped/error state, not an uncommitted URL. UI hiding does not change listener lifetime.

Authorization is checked again after metadata and slot acquisition at final gateway local admission. Successful disable/revoke/regenerate prevents later affected admission under the old policy. A call already admitted may still be sent to the Host and finish afterward, including a call waiting inside the SDK before transport enqueue. The first SDK poll is not an enqueue receipt. Revocation does not cancel admitted calls or promise immediate cessation of Provider work. Use the external agent's own stop control to stop its work. Tokens and desired setup survive restart/reinstall unless explicitly disabled/revoked.

Membership is an additional listener gate, independent of lifecycle-active and desired-enabled. The activation-owned `teamarness.membership` 1.0 watcher binds the local Installation and Workspace, reads the typed current roster, subscribes with `afterEpoch`, then revalidates. Durable catch-up covers the read/subscribe gap. Every notification invalidates admission before a sequential fresh roster lookup; event digests never identify the removed member. One credit-controlled subscription and one snapshot path are separate from the 12 ordinary SDK-operation slots. Snapshots retain ownership through the SDK's existing terminal/deadline; there is no polling or automatic reconnect loop.

Confirmed local absence reports `membership_removed`, stops accepts and retires all connections through the same five-second drain, including static manifest/onboarding and primed schema caches. Desired setup, port, tokens and selections remain intact. A causally removed identity cannot resume through recovery, restart, reload or reinstall. Invalid/wrong-Workspace/failed snapshots, malformed events and stream termination instead report `membership_unavailable` and close access. Valid later notifications can recover a still-live stream after fresh observation; a terminated stream requires explicit lifecycle recovery/reload. Neither notification nor late write/snapshot completion can undo manual suspension or desired-disabled state. Disposal cancels and joins the watcher. Already-authorized SDK work retains existing accounting, but retired HTTP connections need not receive its result; no call is replayed. Bytes already delivered cannot be retracted. Membership-only observation does not promise idle shutdown for arbitrary unsignaled Person presentation/storage errors; Host dynamic authority checks remain unchanged.

Served guidance treats Provider text as untrusted data. An external client may make at most one delayed retry of a retryable ordinary read only if the current Provider guide permits. Persistent failure, authentication/policy/schema refusal, and `port_catalog_changed` stop the procedure; no rapid retry loop is recommended.

## Settings and onboarding

The main view identifies its supplied Workspace and shows desired versus running state, saved port and safe errors. Only running committed state publishes a URL. The standard settings view controls listener/port, grouped Provider contracts, exact tools labeled Read, Write, Streaming query or Event and token create/regenerate/revoke/revoke-all. Package enable applies only to explicitly observed displayed contracts, never auto-selecting their tools. Package disable removes all saved choices for that Provider. Host-only unavailable selections support exact removal: `external-gateway.package.set` accepts `provider:null` only for disabling a nonempty, unique list of currently unavailable contract/major pairs. Null-provider enable, wrong majors, live Providers and ambiguous requests are refused. Local denial commits before Host cleanup; cleanup failure remains visible. The persisted version remains 1.

Settings consumes at most one 32-row summary page and one 32-tool page at a time. Open, Refresh and paging fetch through the UI bridge; polling runs only while settings is open, every five seconds after the previous refresh ends. Each refresh has a five-second observation deadline. Unmount/navigation aborts pending work and clears timers and one-time secret DOM. A mutation invalidates pending catalog replies; old scope or lost token replies cannot repopulate a later view. Status subscriptions contain no secrets. No browser network, storage, clipboard capability or Shell-specific controls are used.

`docs/onboarding.md` is the single secret-free Markdown bundled in the main view and served inside the onboarding JSON response. It describes manifest → paged contracts → complete schema/live guide → shape-appropriate call or subscription, separate guide/projection digests, safe local secret handling, shared scope and bounded retry policy. The automated generic HTTP client does not establish Don's actual external-agent observation (M5).
