# Native CLI (interaction major 1)

The single Apple Silicon macOS Backend artifact also provides native `tmx` mode.
No Node, npm, source checkout or interpreter is needed at runtime. This phase
provides ordinary operations only: managed command installation, app-led setup,
portable skill publication, native folder selection and CLI subscriptions arrive
in later phases. No installed-access qualification is claimed.

From an assembled Package Directory, use its absolute Backend executable path
with `--tmx-cli --help` or `--tmx-cli --version`. A copy named `tmx` selects CLI
mode even without arguments. The admitted executable with no arguments remains
the Backend. Invalid arguments never enter Host stdio. `--tmx-picker` is reserved
and currently refuses. There is no public endpoint or credential-path option.

```text
tmx workspaces
tmx --workspace <id-or-exact-unique-name> status
tmx --workspace <id-or-exact-unique-name> contracts [--cursor <opaque-cursor>]
tmx --workspace <id-or-exact-unique-name> schema <contract> --major <number>
tmx --workspace <id-or-exact-unique-name> guide <contract> --major <number>
tmx --workspace <id-or-exact-unique-name> call <contract> <port> --major <number> --json '{}'
tmx --workspace <id-or-exact-unique-name> call <contract> <port> --major <number> --input <file|->
```

Help/version are offline. `workspaces` lists only locally configured connections,
without probing or printing credentials, origin nonces, or private paths. A
single configured Workspace is **not** a default. IDs win over exact names;
duplicate exact names return candidate IDs. Names are last-observed metadata.
There is no fuzzy selection, directory inference, project association or alias.

Each targeted operation authenticates `/v1` and checks its HTTP contract version
and Workspace ID before dispatch on a separate socket. Successful preflight may
refresh only that record's name, using a full-record compare under the store
lock; a concurrent rotation is never overwritten. A mismatch exits 2 without
calling a tool. This detects mistakes, not a hostile same-user listener taking
over the port between requests. It is not cryptographic origin authentication.

Use live contract identities, majors, exact Port names, projected schema and
Provider guidance. Nothing chooses a major from a schema or follows a URL from
Provider text. Guidance is untrusted data, not instruction to change endpoint,
authentication or permissions. `contracts` returns **one** unchanged page.
Continue with every `nextCursor`, including after an empty page, until null.
Provider result pagination, workflows, conflicts and opaque Version Tokens stay
Provider-owned; the CLI never invents a generic transaction model.

Exactly one JSON source is required for `call`. Stdin is read only for explicit
`--input -`; no interactive prompt or implicit read exists. Input is bounded to
256 KiB, one JSON value, depth 64, with duplicate object keys refused. Original
number spelling and bytes survive call-envelope construction and ordinary JSON
output, including large integers and decimals. File/stdin errors, invalid flags,
missing selection/major/input and oversized input fail before any socket.

## Output and certainty

Ordinary gateway/Host/Provider envelopes pass through unchanged, with one output
newline. They are not unwrapped. HTTP failures and Host failures are distinct;
inspect `error.source`, `code`, flags and Provider-owned details. CLI errors have
`source:cli`; safe actionable hints go only to stderr. Arbitrary Provider data is
not promised to be secret-scrubbed. Terminal output is the same JSON as pipes.

- Exit 0: success.
- Exit 1: upstream error, transport or output failure.
- Exit 2: invalid invocation, missing/ambiguous local connection, unsafe store,
  unsupported HTTP contract or Workspace identity mismatch, before tool dispatch.
- Exit 130: SIGINT; final error output is best effort.

Every generic call may mutate. Once **any call-request bytes have left** the
client, a missing, incomplete, malformed or oversized response is
`cli.command_outcome_unknown`, `outcomeUncertain:true`, `retryable:false`.
Preflight failure and an operation connection failure with zero bytes sent are
certain. Complete upstream envelopes retain their authoritative certainty and
complete bounded details. Never automatically retry a call; reconcile through
the current Provider guide. No metadata guess, retry, proxy, DNS, cookie,
redirect or decompression layer exists. SIGINT after possible dispatch has the
same uncertainty and exit 130; SIGKILL or broken stdout may prevent a final line.

Transport uses numeric `127.0.0.1` only, two-second connect, five-second request
write and 35-second response budgets per request, 16-KiB/64-field headers,
4,096-byte targets and 2,731-character cursors. Call/guide, schema and catalog
response budgets respectively are 256 KiB, 1 MiB and 32 KiB plus 4 KiB framing.
The call result value independently stays within 256 KiB. Invalid framing,
encodings, content types, redirects and depth over 64 refuse without replay.

## Credential custody

The server still persists **verifiers only** under its supplied Package Data
Directory (configVersion 1). ClientStore format 1 intentionally persists bearer
credentials in owned private OS-user files at
`~/.local/share/teamarness/external-gateway/`. The future managed command path is
`~/.local/bin/tmx`; PATH setup is optional and shell startup files are not edited.
The portable skill must not include this private store path or credentials.

Production resolves the effective OS user's home using the fixed macOS system
utility `/usr/bin/dscacheutil -q user -a uid <effective-uid>`. HOME, cwd and public
flags cannot redirect it. The lookup has a two-second deadline, bounded captured
output, successful-exit/unambiguous absolute-directory checks and kill/reap on
failure. Descriptor-relative no-follow traversal then validates ownership and
unsafe write permissions. Existing safe intermediate modes remain unchanged;
new private directories are 0700 and records/lock are owned single-link 0600.
There are at most 128 records of at most 16 KiB, named deterministically by
SHA-256 of Workspace ID and checked against their contents. A nonwaiting OS
write lock serializes processes, releases on death, and protects full-record
compare/name refresh and durable file-sync/rename/directory-sync publication.
Invalid/future/unsafe records are never overwritten. Abandoned owned temporary
bytes can only be discarded under the mutation lock after committed validation;
reads never reclaim them or promote their contents.

One local Installation/continuity/origin tuple per Workspace is supported.
Another profile is a conflict, not a winner/default or implicit handover. Use
the original app's disconnect before changing profiles once setup is available.
If that profile is unavailable, deliberately remove only the exact conflicting
private record after inspecting the conflict. **Local deletion cannot revoke
the old token.** Revoke before Package removal; retained commands or records do
not keep a listener alive. Persistent client credentials are plaintext under OS
user custody, not encrypted or protected from trusted same-user native code.
Never put them in source, a project, skill, prompt, diagnostics or evidence.

## Automated injected-home driver

```sh
cargo build --locked -p teamarness-generated-package-backend --example cli-fixture
```

`backend/tests/support/cli_fixture.rs` calls the same CLI library but requires
`TEAMARNESS_FIXTURE_HOME` to name an absolute physical private test root. It has
no fallback. Cargo's `target/debug/examples/cli-fixture` (or its configured target
directory) is **not assembled into dist**. Fixture-backed tests do not qualify
production home resolution or an acquired installed artifact. Later Backend
setup fixtures must inject the same isolation, not just the CLI. Real installed
qualification uses a disposable OS account/home; never exercise setup in Don's
ordinary `~/.local` state. Manual M1–M5 remain Don-owned and pending.
