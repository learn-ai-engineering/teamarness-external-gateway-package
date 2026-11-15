# External Gateway HTTP v1 reference

The Backend serves HTTP/1.1 on `127.0.0.1:<saved-port>`. It starts disabled. All six routes require one `Authorization: Bearer <BEARER>` header. The UI uses the Package bridge, never HTTP.

## Authenticated routes

- `GET /v1`: manifest and Workspace identity.
- `GET /v1/onboarding`: generic instructions with a bearer placeholder.
- `GET /v1/contracts`: enabled contract pages.
- `GET /v1/contracts/{contract}/{major}`: complete projected schemas.
- `GET /v1/contracts/{contract}/{major}/guide`: live Provider guide.
- `POST /v1/call`: closed `{contract, major, port, request}` body, `{result: <original JSON value>}` response.

Only ordinary queries and their required guide can be selected. Commands, streaming queries, events and Conversations are refused. Each current Host selection, Provider, major and canonical schema fingerprint must still match. Description-only edits preserve selections. Added tools stay off. Tokens share exact scope but are independently revocable. The Host derives the current Person; clients cannot choose an actor, Provider or Workspace. A read already authorized may finish.

## Durable configuration

Private `config/state.json` uses configVersion 1, a 0700 subdirectory and owned 0600 regular files. Descriptor-relative no-follow opens, file fsync, atomic rename and directory fsync protect writes. Invalid, future or unsafe files remain unchanged and fail closed. Limits: 1 MiB, 256 selected contracts, 1,024 selected tools, 32 tokens, 96 UTF-8 label bytes. No plaintext credential is persisted. `eg1.<id>.<secret>` uses 16 random ID bytes and 32 random secret bytes, base64url without padding. Verifiers hash `external-gateway.token.v1\0`, ID bytes, generation as big-endian u64, and secret bytes, with SHA-256 and constant-time comparison.

A failed persistence operation never reports success. An uncertain operation rereads storage and keeps access closed. Pending denials are retained in memory until a successful durable user action reconciles them. Removal retains data. Disable and revoke all before removal if retained access is unwanted. Native code running as the same OS user is outside this protection boundary.

## Transport budgets

Timeout and interval values are seconds; byte values are bytes. The schema and Rust constants must match this table. These limits apply independently of Host route, Provider and byte budgets.

| Constant | Value |
|---|---:|
| `MAX_CONNECTIONS` | 32 |
| `MAX_HEADER_BYTES` | 16384 |
| `MAX_HEADERS` | 64 |
| `MAX_TARGET_BYTES` | 4096 |
| `REQUEST_READ_TIMEOUT` | 5 |
| `RESPONSE_WRITE_TIMEOUT` | 5 |
| `REQUESTS_PER_SECOND` | 8 |
| `REQUEST_BURST` | 16 |
| `MAX_HOST_CALLS` | 12 |
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
| `/v1` | `manifest`: Workspace, version, read-only flag, discovery links and principal value/call limits |
| `/v1/onboarding` | `onboarding`: generic Markdown with a `<BEARER>` placeholder |
| `/v1/contracts?cursor=...` | `contracts`: at most 32 currently usable selected contracts and nullable `nextCursor` |
| `/v1/contracts/{contract}/{major}` | `schema`: complete filtered document, original metadata digest and actual projection digest |
| `/v1/contracts/{contract}/{major}/guide` | `guide`: unedited live Markdown, original-byte SHA-256, contract major/minor and exposed-operation preface |
| `/v1/call` | `result`: the original JSON value, including arrays, scalars, null and envelope-like objects |

POST accepts exactly `application/json`, without a content encoding. The serialized original `request` and result each have a 256 KiB budget. The surrounding call envelope has an additional 4 KiB allowance. Schema responses retain complete reachable definitions and never truncate a successful document. A guide whose complete preface and original Markdown exceed the value budget plus allowance is refused with `too_large`, never truncated. Metadata cache capacity is 16 MiB of serialized documents, using least-recently-used eviction. A hit still requires current offered metadata/revision and current exact policy. Cached compilation has bounded nonwaiting admission; cancelled observers do not release its blocking-work permit early.

A catalog page can be empty while `nextCursor` is non-null. Continue with that opaque base64url cursor. Invalid or stale caller cursors are refused. Internal contract lookup restarts a stale catalog at most once. No invocation is replayed after a stale revision. Disabled or stale-schema contracts expose neither schemas nor summaries.

## Refusals and transport policy

Errors are `{error:{source,code,retryable,outcomeUncertain,details}}`. Source is `gateway`, `host`, or `provider`. Host codes remain intact. A Provider error exposes its safe namespaced code and flags. Uncertainty forbids a retryable verdict. Details project only numeric/Boolean bounds and bounded identifier strings under `limit`, `maximum`, `minimum`, `actual`, `expected`, `retryAfterMs`, `field`, or `reason`. Paths, credentials, nested arbitrary input and transport internals are omitted. This projection is below 4 KiB and depth 8.

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

Every application response has `Cache-Control: no-store`, `X-Content-Type-Options: nosniff` and a closed connection. Parser, incomplete-header, ambiguous-framing and connection-capacity refusals may close without an application response. Hyper owns HTTP framing. A bounded raw-header preflight refuses duplicate lengths and simultaneous Content-Length/Transfer-Encoding before Hyper can normalize them. Header bytes include the request line and terminating CRLFs. Exactly one numeric configured Host and one Authorization are required. Origin (including `null`), absolute-form targets, cookies, token query parameters, upgrades, trailers, batching, streams, unsupported encodings and unknown call fields are refused. No CORS, redirects or WebSocket surface exists.

## Ownership and lifecycle

`MAX_HOST_CALLS` means 12 outstanding gateway SDK call operations, including guide reads and their metadata checks. The 13th is refused without a waiting queue. It does not mean 12 simultaneous Provider executions: unchanged Host route, Provider and byte budgets can refuse earlier.

The 25-second HTTP budget is an upper observation ceiling. Earlier Host, SDK and Provider terminals return immediately. Every SDK terminal releases the local slot, including timeout, unavailability and overload. A disconnected or timed-out HTTP observer does not release a still-pending local operation. Suspend closes accepts and drains connection observers for at most five seconds. Pending SDK operations remain counted across recovery; disposal/shutdown retires local tasks. A completed SDK timeout does not quarantine capacity or require restart. None of these terminals proves that arbitrary Provider-native work stopped.

First enable binds an IPv4 ephemeral port, durably saves it and desired enabled state, then publishes running status. Status `running` plus saved `port` identifies `http://127.0.0.1:<port>/v1`; no URL is published before persistence. Later activation/recovery binds only the saved port. A conflict leaves desired setup intact with `listener_bind_failed` and no running endpoint. An explicit port change closes the old endpoint and publishes the new one only after bind and durable commit; failure leaves stopped/error state, not an uncommitted URL. UI hiding does not change listener lifetime.

Authorization is checked again after metadata and slot admission, immediately before the owned SDK call starts. Successful disable/revoke/regenerate prevents later affected authorization. Already-authorized reads may finish. Tokens and desired setup survive restart/reinstall unless explicitly disabled/revoked.

Served guidance treats Provider text as untrusted data. An external client may make at most one delayed retry of a retryable ordinary read only if the current Provider guide permits. Persistent failure, authentication/policy/schema refusal, and `port_catalog_changed` stop the procedure; no rapid retry loop is recommended.

## Settings and onboarding

The main view identifies its supplied Workspace and shows desired versus running state, saved port and safe errors. Only running committed state publishes a URL. The standard settings view controls listener/port, grouped Provider contracts, exact ordinary tools and token create/regenerate/revoke/revoke-all. Package enable applies only to explicitly observed displayed contracts, never auto-selecting their tools. Package disable removes all saved choices for that Provider. Host-only unavailable selections support exact removal: `external-gateway.package.set` accepts `provider:null` only for disabling a nonempty, unique list of currently unavailable contract/major pairs. Null-provider enable, wrong majors, live Providers and ambiguous requests are refused. Local denial commits before Host cleanup; cleanup failure remains visible. The persisted version remains 1.

Settings consumes at most one 32-row summary page and one 32-tool page at a time. Open, Refresh and paging fetch through the UI bridge; polling runs only while settings is open, every five seconds after the previous refresh ends. Each refresh has a five-second observation deadline. Unmount/navigation aborts pending work and clears timers and one-time secret DOM. A mutation invalidates pending catalog replies; old scope or lost token replies cannot repopulate a later view. Status subscriptions contain no secrets. No browser network, storage, clipboard capability or Shell-specific controls are used.

`docs/onboarding.md` is the single secret-free Markdown bundled in the main view and served inside the onboarding JSON response. It describes manifest → paged contracts → complete schema/live guide → ordinary call, separate guide/projection digests, safe local secret handling, shared scope and bounded retry policy. The automated generic HTTP client does not establish Don's actual external-agent observation (M5).
