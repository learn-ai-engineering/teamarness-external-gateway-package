# Native CLI (interaction major 1)

Each native Backend artifact (Apple Silicon macOS, or x86_64 Linux with glibc) also provides native `tmx` mode.
No Node, npm, source checkout or interpreter is needed at runtime. It provides ordinary operations, live subscriptions and recoverable managed
command/skill setup with native folder selection and owning-Installation refresh.
No installed-access qualification is claimed.

From an assembled Package Directory, use its absolute Backend executable path
with `--tmx-cli --help`, `--tmx-cli --version` or `--tmx-cli --skill`. A copy named `tmx` selects CLI
mode even without arguments. The admitted executable with no arguments remains
the Backend. Invalid arguments never enter Host stdio. `--tmx-picker` is a private native-helper entry, not a CLI command. There is no public endpoint or credential-path option.

To install and connect `tmx` on a node without the desktop Configuration page,
follow [headless onboarding](headless-onboarding.md).

```text
tmx workspaces
tmx --workspace <id-or-exact-unique-name> status
tmx --workspace <id-or-exact-unique-name> contracts [--cursor <opaque-cursor>]
tmx --workspace <id-or-exact-unique-name> schema <contract> --major <number>
tmx --workspace <id-or-exact-unique-name> guide <contract> --major <number>
tmx --workspace <id-or-exact-unique-name> call <contract> <port> --major <number> --json '{}'
tmx --workspace <id-or-exact-unique-name> call <contract> <port> --major <number> --input <file|->
tmx --workspace <id-or-exact-unique-name> subscribe <contract> <port> --major <number> --json null --until-cancelled
tmx --workspace <id-or-exact-unique-name> subscribe <contract> <port> --major <number> --input <file|-> --for 2m [--max-events 10]
```

Help, version, licenses and skill output are offline. `workspaces` lists only locally configured connections,
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

Exactly one JSON source is required for `call` and `subscribe`. Stdin is read only for explicit
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

## Live observations

`subscribe` supports event and streaming-query Ports, with the same exact
Workspace, contract, major, Port and explicit JSON input as calls. Use `null`
for events or the streaming query's request according to its live schema.
Exactly one of `--for <duration>` or `--until-cancelled` is required. There is no
implicit duration. Durations are positive integers with `ms`, `s`, `m` or `h`;
arithmetic exceeding a u64 nanosecond duration or the monotonic clock is refused.
Optional `--max-events` is a positive integer through 4,294,967,295, alongside
either lifetime mode. Duplicate, conflicting and overflowing flags refuse before
connecting. Only `value` frames count, never heartbeats or other control frames.

The 35-second opening budget includes validated ready. Ready must appear once,
match the selected contract/major/Port and declare query/event plus stream.
The duration starts when that ready is accepted, not on process launch. Original
bounded gateway NDJSON lines (`ready`, `value`, `heartbeat`, `end`) pass through
unchanged, including primitive values, large integers, decimal spelling and
whitespace. There is one owned observation, no reconnect or stream collection.

| Cause | Terminal | Exit |
| --- | --- | --- |
| Gateway completed/error | Original `end` | 0/1 |
| Requested duration | `{"type":"client_end","reason":"duration_limit"}` | 0 |
| Requested value count | `{"type":"client_end","reason":"event_limit"}` | 0 |
| SIGINT after ready | Best-effort `client_end`, reason `cancelled` | 130 |
| EOF, invalid frames/order, stalled transport after ready | `client_error` with `cli.stream_interrupted` | 1 |

Before ready, refusals/cancellation/transport errors are ordinary JSON, never a
fabricated ready or client stream terminal. The `client_error` contains the
ordinary error fields (`source:cli`, `retryable:false`, `outcomeUncertain:false`,
empty `details`). That flag is about command outcome, **not complete event
delivery**: an interruption signals a gap requiring Provider-guided reconciliation.

An already accepted gateway end wins. Otherwise cancellation precedes expired
duration, and both precede accepting another frame. The count-reaching value is
drained before its event-limit terminal; no next frame is accepted. A partially
written line is drained within its remaining output budget before any local
terminal, never spliced into another JSON line. A failed terminal write is exit
1, not successful observation. No two terminals are emitted.

Each line is at most 256 KiB plus 4 KiB including its newline; its value itself
is at most 256 KiB and whole-frame depth is at most 64. There is one pending line
and a bounded transport read buffer, not a producer queue. Descriptor-level
nonblocking, readiness-registered stdout has a five-second budget for each pending write. A stalled or
broken output closes the socket and exits nonzero, with **no further stdout
attempt**. SIGKILL or a broken pipe can prevent the final line entirely.

After ready, 45 seconds without a valid frame is a transport interruption, not
a total lifetime. Partial bytes do not reset it. Local output backpressure
suspends this liveness clock, but not a requested duration. Continuous mode has
no CLI lifetime cap. Gateway 15-second heartbeats remain in output and must not
be treated as domain events. An external harness may impose its own lifetime;
there is no daemon, model wakeup, durable replay or universal session cleanup.
Explicitly reconnect and reconcile with current Provider guidance after a gap.

Subscriptions require stdout to be a pollable pipe or terminal. Direct redirection
such as `> events.ndjson` is unsupported: regular-file writes cannot honor
O_NONBLOCK and the mandatory pending-write bound. This refuses **before network**
with exit 2, `cli.stream_stdout_unsupported` JSON and a hint on stderr; unsupported
stdout itself is never written. Ordinary command output is unchanged. To save a
stream, let an external writer own the file, for example in zsh/bash:

```sh
set -o pipefail
tmx --workspace Acme subscribe dev.example.provider changes --major 1 --json null --until-cancelled | tee events.ndjson
```

`pipefail` prevents a successful `tee` exit from hiding a failed CLI observation.
If the external writer stalls, the CLI's pipe backpressure still times out in
five seconds and closes its socket; it cannot guarantee the writer's disk I/O.

## Credential custody

The server still persists **verifiers only** under its supplied Package Data
Directory (configVersion 1, migrating to 2 only on managed authority commit). ClientStore format 1 intentionally persists bearer
credentials in owned private OS-user files at
`~/.local/share/teamarness/external-gateway/`. The fixed managed command path is
`~/.local/bin/tmx`; PATH setup is optional and shell startup files are not edited.
The portable skill must not include this private store path or credentials.

Production resolves the effective OS user's home from the OS account database,
never from `HOME`. On macOS it runs the fixed system utility
`/usr/bin/dscacheutil -q user -a uid <effective-uid>`. On Linux it runs
`/usr/bin/getent passwd <effective-uid>` with an empty environment and accepts
exactly one passwd line of exactly seven colon-separated fields whose UID field
equals the effective UID and whose home field is absolute and control-free.
HOME, cwd and public flags cannot redirect either lookup. Both have a two-second
deadline, bounded captured output, successful-exit/unambiguous absolute-directory
checks and kill/reap on failure; any failure is `cli.home_unavailable`, with no
fallback. Descriptor-relative no-follow traversal then validates ownership and
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
the original app's disconnect before changing profiles.
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
production home resolution or an acquired installed artifact. The Backend fixture described below injects the same isolation, not just the CLI. Real installed
qualification uses a disposable OS account/home; never exercise setup in Don's
ordinary `~/.local` state. Manual M1–M5 remain Don-owned and pending.

## Recognized command and selected skill publication

The Backend-owned `managed_install` / `skill_install` interfaces prepare read-only
previews and publish outputs without creating credentials, granting tools,
enabling a listener or changing Policy. Initial command installation requires
explicit consent. An automatic refresh is accepted only for an intact recognized
copy, same owning Installation, strictly newer Package version precedence, and
compatible CLI/store majors. Equal version/equal bytes is a no-op. Ownership
transfer, downgrade and same-version changed bytes require a new explicit preview.
Changed or unrelated executable bytes, missing/invalid/future receipts and unsafe
paths refuse even explicit repair. Do not delete a receipt to force replacement;
resolve a collision outside setup. Existing processes retain their executable
image; subsequent invocations use the new copy. The Package version is separate
from CLI interaction major 1 and client-store format 1.

Only complete bytes matching this running generation's admitted Host digest may
be copied or used for a helper. A source mismatch requires Package reload, not
trust in a manifest at that path. The installer uses a nonwaiting cross-process
OS lock, private exclusive copy, copied-byte rehash, executable mode 0755,
file sync, rename and directory sync. Its private bounded receipt records Package
identity/version/admitted digest, installed digest, owner and compatible majors.
A bounded secret-free old/new journal makes executable and receipt publication
recoverable rather than pretending they are one atomic transaction. Recovery
completes a matching already-published new output or reports explicit repair;
it never promotes temporary bytes or adopts arbitrary output. Failed sync can
leave effects visible: it is not a rollback guarantee.

A selected physical folder receives only `teamarness/SKILL.md` and its secret-free
receipt (owned 0600 files; new directory 0700). The portable resource is embedded
in the executable and substitutes one safely quoted absolute command path,
including spaces/apostrophes. It contains no credential, default Workspace,
private-store path or copied Provider contract. Preview pins the exact canonical
path and existing ancestor/destination identities; apply revalidates them.
Unrelated directories/files, edited skills, symlinks, hardlinks, writable
redirection and unsupported encodings refuse. New skill directories are staged
privately and published with no-replace rename. Existing recognized skills use
exact old/new digest recovery. Publication never runs a shell.

Selected destinations exist only in a live plan or an unfinished private journal.
There is no folder registry, scanning or automatic hunt for previous skills;
reselect a folder to update it. A crash before a new skill's journal becomes
durable may leave an empty random private staging directory in the selected
parent. A retry uses fresh exclusive entropy-named staging, never adopting,
scanning or reclaiming that unrecognized orphan. It may require deliberate
external cleanup after inspection; no false rollback/cleanup claim is made.
A command can be published while skill publication fails; no access was authorized.

The typed Backend coordinator supplies explicit consent and actual activation/
reload refresh through an injected-home seam. Both mount views use the native picker and coordinator. Tests never invoke setup against the ordinary OS home. Acquired
native output execution/update and real harness skill discovery remain M1/M3/M4.


## App-led managed setup protocol

`external-gateway.setup.preview` accepts `intent` (connect, repair, rotate,
disconnect, install-skill, install-cli) and optional opaque `selectionId`. It never accepts a
path, bearer, bytes or overwrite flag, and performs no writes. It returns the
exact command/optional skill destination, installed/proposed command owner,
version and digest, update disposition, skill write effect and whether credentials
change. Explicit repair never hides an ownership transfer, downgrade or same-
version byte change. Four
plans expire after ten minutes. `setup.apply` accepts only `planId`, rechecks
relevant Policy/listener state for credential plans, pinned path identities and
complete output digests,
then returns an operation ID. Repeated accepted apply discovers that operation;
`setup.status` and `setup.watch` report secret-free command/skill/access results.
Observer cancellation, remount and acceptance-response loss do not duplicate
accepted work. No setup intent enables a listener or expands tool selections.
Connect/rotate require a running committed listener. Install-skill does not
require credentials. The Package-owned picker supplies bounded mount-scoped selection leases.

One unfinished operation is owned per Instance. Its private format-1 client
journal is under the private client state's `setup` namespace, not server policy.
It contains pending credentials and must not be inspected in ordinary logs or
support evidence. Publication orders private journal, recognized required
outputs, Policy verifier/operation commit, then one atomic credential+endpoint
record. A bounded secret-free last receipt remains; completed receipts do not
retain selected folder paths. No prior-folder registry or scan exists.

A failure can leave a command/skill installed without access. A committed
verifier may briefly have no published credential. Recovery reconciles exact
operation/verifier and recognized output bytes, never arbitrary temp contents.
Never-committed proposals require explicit repair. A current committed operation
may finish publication; revoked, rotated, disconnected or disable-all-superseded
material cannot recreate authority. Uncertain Policy persistence remains closed.
A revoked managed token requires explicit connect consent, not silent repair.
Port-only repair preserves an intact bearer and generation. A missing credential
requires explicit rotation instead. A different local origin for the same
Workspace refuses; there is no automatic handover. If the old origin is
unavailable, deliberate deletion of its exact local record is an external Person
choice, not revocation of its server token.

Managed authority uses one of the existing 32 token slots. Rotation replaces the
same slot with its next generation, closes old idle streams and never overlaps credentials.
Manual revoke is available; raw one-time regeneration of a managed ID returns
`external_gateway.managed_setup_required`. Disconnect durably revokes before
local removal. Uncertain revoke is not success; cleanup-pending retries remove
only the matching record and never mint credentials. Shared command/skills are
not deleted. Revoke before Package removal. Calls admitted earlier may finish.

Policy reads version 1, migrating durably to configVersion 2 only on managed
commit. Manual verifiers/generations, grants, desired state and port survive.
Managed markers/tombstones contain no bearer. Invalid/future state is untouched;
older Backends cannot read v2 and are not a rollback procedure. CLI/store format
1 and Package version are independent. The store mutation lock precedes bounded
Policy mutation; manual revocation never waits on local cleanup. No socket,
dialog or observation wait holds both owners. Lifecycle closes new setup
admission before stopping Policy and joining/checkpointing accepted disk work.

### Backend fixture for real-Host qualification

Build `cargo build --locked -p teamarness-generated-package-backend --example
backend-fixture` from the recorded clean revision. Its entry lives in
`backend/tests/support/backend_fixture.rs`, links the same router/Backend modules
and is never assembled into production `dist`. Backend mode requires
`TEAMARNESS_FIXTURE_ROOT` to name an existing canonical, owned, exact-0700 physical
directory before SDK stdio starts. At SDK activation it creates/validates an owned
0700 child using descriptor-relative no-follow operations under that pinned root.
The child name is lowercase hexadecimal SHA-256 of the canonical lowercase
Installation identity's UTF-8 bytes (the 64-hex identity string, not decoded key
bytes). All Workspaces of one Installation share this home; different Installations
have separate homes. Workspace ID and cwd never affect the child name. Unsafe or
replaced roots/children fail closed without repair or ordinary-home fallback.

The same fixture bytes, including a managed copy whose basename is `tmx`, use the
shared pre-runtime selector and production CLI library. For each CLI process set
`TEAMARNESS_FIXTURE_HOME` to the exact existing canonical owned-0700 child derived
above. CLI mode never rederives it or requires ROOT. Picker mode dispatches before
both environment checks, preserving the private request/lifeline protocol without
home/root effects. No fixture environment variable is read by production entry.

Provision ROOT **before** the Host process starts and HOME **before** a copied CLI
invocation. Backend mode accepts no arguments or only the actual fixed ordered
Host launch-token/protocol pair; arbitrary, duplicate and mixed modes refuse.
The SDK continues to own initialization and protocol validation unchanged.
Stage/hash/admit fixture bytes separately as a fixture Package Directory; Host
Initialization still supplies the actual admitted source digest. Never patch
production dist or substitute fixture hashes for the production artifact. These
are injected-home qualification results, not production OS-home/native installation evidence.
Installed checks require the disposable OS account/home and M1–M5 observations.

`tmx --licenses` prints embedded third-party license texts and their source
provenance offline, before home lookup, sockets or AppKit. Like help/version it
needs no Workspace and accepts no other options. The same native artifact retains
these notices after managed copy or Package Directory movement.

### Manual skill installation with `tmx --skill`

`tmx --skill` (or `<executable> --tmx-cli --skill`) prints the portable skill to
stdout, offline. It does no home lookup, client-store read, network or Host
work, and does not start AppKit. Like `--licenses`, it accepts no other options.
The output is the same rendering that selected-folder publication writes, byte
for byte, for the same command path. It contains no credential, default
Workspace, private-store path or copied Provider contract.

The skill names the running executable's own canonical absolute path as its one
shell-quoted command. Run it from the installed managed copy, so that the skill
names that copy:

```sh
(
  umask 077
  dir="$HOME/<agent skill folder>/teamarness"
  mkdir -p "$dir" &&
    ~/.local/bin/tmx --skill > "$dir/SKILL.md.tmp" &&
    mv "$dir/SKILL.md.tmp" "$dir/SKILL.md"
)
```

Replace `"$HOME/<agent skill folder>"` with the skill folder's path under your
home, or with its absolute path. A quoted `~` does not expand, so do not use it
inside the quotes.

The subshell's `umask 077` keeps new folders and the new file owner-only without
changing the calling shell's umask; existing folders keep their modes. The
commands are chained with `&&`, and the skill is written to `SKILL.md.tmp`
before it replaces `SKILL.md`. If `tmx --skill` fails, the chain stops and any
existing `SKILL.md` is unchanged; delete `SKILL.md.tmp` and run it again.

If you run it from another copy, such as a Package Directory's Backend
executable, the skill names that copy instead. A manually written file has no
receipt, so the app's skill publication treats it as unrecognized and refuses
to replace it. Rerun the command after a CLI update, so the text matches the
new version, and after moving the executable. If the executable
path cannot be quoted safely (not absolute, not UTF-8, longer than 4096 bytes or
containing a control character), it refuses with `cli.skill_unavailable` and
exit 2.

### Independent internal CLI action and current facts

The typed coordinator also accepts `install-cli` through its existing exact
preview/apply protocol, with no folder selection. This is command publication
only, not Connect with a throwaway credential. It does not read or lock private
connection records, create/read an origin, issue a token, modify Policy, install
a skill, or write a credential setup completion receipt. Malformed/future or
other-profile private records do not block it and remain untouched. Valid
unfinished credential work and active owned work still block competing actions.
There is no CLI uninstall action. Command-only acceptance/progress is ephemeral;
after restart, the recognized installer receipt/journal supplies installed truth.
Recognized command recovery is independent of private credential observation;
failed CLI work never invokes credential recovery. If exact new command bytes
are already published but their receipt is still unfinished after storage failure,
restore storage and Reload the Package to reconcile them. Inspection remains
read-only and does not offer an unusable Update CLI action for that state.

The closed `external-gateway.setup.inspect` request returns three current facts:
`cli` (recognition/path/version and candidate change), `connection` (actual
Workspace authority and local publication), and `recovery` (pending or invalid
credential coordinator work, not a historical failure). Its issues are bounded
safe codes, never secrets/private credential paths/opaque identities. Output
errors from setup preview/rechecks include only `output: command` or
`output: skill` where the affected output is known, not arbitrary private paths.
Inspection is a read-only observation, not a filesystem watch: it creates no
directories, takes no mutation locks, performs no recovery or temporary cleanup,
and runs filesystem/hash checks on blocking work. A changed/unavailable Policy
revision makes the connection fact unavailable; it is not an atomic cross-store
snapshot and cannot authorize a later action without fresh admission checks.
An intact connection remains connected when the listener is stopped or after a
skill/CLI result. Missing bearer requires explicit replacement; endpoint-only
mismatch permits publication repair with intact credentials. Revoked/disconnected
tombstones never regain access through inspection. An unavailable inspection does
not prove that access stopped: existing Policy-authorized access may remain live.
Configuration retains explicit revoke and Disable-and-revoke controls.
Status/watch and all five legacy intents/disk forms remain unchanged. The new live CLI intent is explicitly
invalid in credential setup Journal/Completed files, so old readers never receive
a new durable CLI history form from this implementation.

Both Package views expose these seams as direct named actions: **Install CLI** /
**Update CLI**, **Connect**, **Rotate credentials**, **Disconnect**, **Repair**,
and optional **Install skill…**. Routine actions internally preview then apply
one exact plan. Clicking Install/Update CLI itself authorizes that recognized
output change, including a same-version development rebuild; there is no second
CLI confirmation. Unrecognized or modified outputs are refused. Progress, errors
and dismissible results stay next to the controls in their own setup section.
Connect, Rotate credentials and Install skill… refuse any CLI change or non-null
CLI issue; first use the independent CLI action. A valid already accepted pending
Repair is the narrow exception: it can resume required command publication with
one concise version/output confirmation, because the Backend reserves those
outputs and a separate CLI action cannot bypass that work. Unsafe/unrecognized
outputs are still refused. Skill chooser confirmation installs only the
skill, Cancel writes nothing, and credential actions never carry a folder
selection. The live chooser lease lasts through acceptance. Current connection
facts are separate from operation progress/results and listener running state;
closing a view never cancels an accepted operation. Idle or unrelated historical
updates cannot settle a locally submitted action. If acceptance is unknown and no
correlated operation update arrives, reopen the view to inspect current state;
setup never automatically retries the action. There is no CLI uninstall.
