# External Gateway HTTP v1 reference

Phase 3 defines durable policy and contracts. This build starts no HTTP listener. Phase 4 implements and qualifies the transport below; these routes are not yet available.

## Planned authenticated routes

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

Timeout and interval values are seconds; byte values are bytes. The schema and Rust constants must match this table. Transport boundary tests arrive with phase 4; these numbers are not a passing transport claim.

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
| `MAX_HOST_CALLS` | 4 |
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
