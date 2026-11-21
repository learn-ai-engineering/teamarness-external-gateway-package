# Headless onboarding

This guide is for an operator who configures External Gateway on a Teamarness
node without the desktop Configuration page, for example a Linux server. On the
desktop, the Configuration page performs the same steps in the same order. On a
headless node, you send each Package request yourself.

The sequence below is tested by `backend/tests/headless_onboarding.rs`. That
test runs the real Backend in the SDK test harness with a private test home, a
scripted Provider and an installed fixture copy of `tmx`. It is not evidence of
a real Host. The real-Host run of this guide is pending.

## Prerequisites

- The Package `dev.teamarness.external-gateway` is registered and enabled in
  the Workspace.
- The Package Backend is running for that Workspace.
- You can send Package requests to it. Until the Host defines its operator
  command, see [Host command wrapper (planned, unverified)](#host-command-wrapper-planned-unverified).
- The Provider Package whose tools you want to expose is enabled in the same
  Workspace.
- The Backend's effective UID resolves to an absolute home directory through
  the OS account database within two seconds. On Linux the Backend runs
  `/usr/bin/getent passwd <uid>` with an empty environment and never reads
  `HOME`. If that lookup fails (no `getent`, no passwd entry, or a slow name
  service), the Backend does not activate; the activation error is
  `external_gateway.home_unavailable`. Check the lookup with
  `getent passwd "$(id -u)"` as the Backend's OS user.

Send each mutation once. Do not retry a mutation automatically. If the outcome
is unknown, inspect the current state before you decide what to do next.

## Order of operations

### 1. Inspect the current state

Send `external-gateway.setup.inspect` with this body:

```json
{}
```

On a new node, expect `cli.state` to be `missing`, `cli.change` to be `install`,
`connection.state` to be `not-connected` and `recovery.state` to be `none`.

### 2. Install the CLI

Send `external-gateway.setup.preview`:

```json
{"intent":"install-cli"}
```

The preview writes nothing. Read `commandPath` and `commandChange` in the
response. Then send `external-gateway.setup.apply` with the `planId` from the
preview:

```json
{"planId":"<planId from setup.preview>"}
```

A plan expires after ten minutes. Then send `external-gateway.setup.status`
with `{}` until `phase` is not `accepted`, `prepared`, `outputs` or `committed`.
Expect `phase` `complete`, `command` `installed` and `access` `unchanged`.

The command is installed at `~/.local/bin/tmx`. This step creates no
credential. It does not start the listener and does not select tools.

### 3. Start the listener

Setup never starts the listener for you. Send `external-gateway.listener.set`:

```json
{"enabled":true}
```

Then send `external-gateway.status.get` with `{}`. Expect `running` `true` and
a numeric `port`.

### 4. Select the tools to expose

Send `external-gateway.catalog.page` with `{}` to list contracts. Continue with
each `nextCursor` value as `{"cursor":"<nextCursor>"}` until `nextCursor` is
`null`.

Enable one Provider's contracts with `external-gateway.package.set`:

```json
{"provider":"dev.test.provider","enabled":true,"contracts":[{"contract":"dev.test.reads","major":1}]}
```

Enabling a package selects no tools. List its tools with
`external-gateway.catalog.page`:

```json
{"contract":"dev.test.reads"}
```

A page holds at most 32 tools. If `nextOffset` is not `null`, request the next
page with the same contract and that offset, and continue until `nextOffset` is
`null`. For example, if `nextOffset` is `32`:

```json
{"contract":"dev.test.reads","offset":32}
```

Use the `nextOffset` value from the previous page, not a calculated value.

Every tool has `selected` `false` at this point. Select each tool separately
with `external-gateway.tool.set`, using its `fingerprint` from the tool list:

```json
{"contract":"dev.test.reads","major":1,"provider":"dev.test.provider","port":"read.item","fingerprint":"<fingerprint>","enabled":true}
```

`dev.test.provider`, `dev.test.reads` and `read.item` are the names that the
test fixture uses. Use the `provider`, `contract`, `major` and `port` values
that `catalog.page` returns on your node. Select only the tools that the
external agent needs. Connect, rotate and repair never add a selection.

### 5. Connect this Workspace

Send `external-gateway.setup.preview`:

```json
{"intent":"connect"}
```

Send `external-gateway.setup.apply` with its `planId`, and poll
`external-gateway.setup.status` as in step 2. Expect `phase` `complete` and
`access` `configured`.

Connect creates one managed credential and stores it in the private client
store under `~/.local/share/teamarness/external-gateway/`. No response contains
the credential. Do not copy it from the store.

## Verify

Send `external-gateway.setup.inspect` with `{}`. Expect:

| Field | Value |
| --- | --- |
| `cli.state` | `installed` |
| `cli.change` | `none` |
| `cli.issue` | `null` |
| `connection.state` | `connected` |
| `connection.authority` | `current` |
| `connection.repair` | `none` |
| `recovery.state` | `none` |

Then run the CLI as the same OS user:

```sh
~/.local/bin/tmx workspaces
~/.local/bin/tmx --workspace <workspace-id> status
```

`workspaces` lists the Workspace ID, name and port. `status` prints JSON with
`reachable` `true`, the numeric `endpoint` and the Workspace identity, and exits
0. Then make one call to a selected tool:

```sh
~/.local/bin/tmx --workspace <workspace-id> call <contract> <port> --major <major> --json '{}'
```

A tool that you did not select fails with `external_gateway.forbidden` and exit
1. See [CLI usage and client custody](cli.md) for all commands.

## Rotate, disconnect and repair

To replace the credential, preview, apply and poll `rotate`:

```json
{"intent":"rotate"}
```

The old credential stops working immediately. `tmx` uses the new one with no
other change.

To remove access for this Workspace, preview, apply and poll `disconnect`:

```json
{"intent":"disconnect"}
```

Expect `access` `revoked`. `setup.inspect` then reports `connection.state`
`not-connected` and `connection.authority` `disconnected`. `tmx --workspace
<workspace-id> status` fails with `cli.workspace_not_configured` and exit 2.
Disconnect leaves `~/.local/bin/tmx` and any skill in place. To connect again,
use `connect`.

Use `repair` only when `setup.inspect` reports `connection.state`
`needs-repair`:

```json
{"intent":"repair"}
```

| `connection.repair` | Meaning | Action |
| --- | --- | --- |
| `publication` | The saved port or operation changed, and the credential is intact. | `repair` |
| `resume` | An accepted operation did not finish. | `repair` |
| `cleanup` | A disconnect revoked access but did not finish local cleanup. | `repair` |
| `replace-credentials` | The local credential is missing or does not match. | `rotate` |

Repair needs the listener to be running, except to finish a disconnect. Repair
never creates a new credential when the existing one is intact. If
`connection.authority` is `revoked`, a token was revoked manually; use
`connect`, not `repair`.

## Install the skill manually

On a headless node there is no folder chooser. Print the portable skill from
the installed command and write it to the external agent's skill folder:

```sh
(
  umask 077
  dir="$HOME/<agent skill folder>/teamarness"
  mkdir -p "$dir" &&
    ~/.local/bin/tmx --skill > "$dir/SKILL.md.tmp" &&
    mv "$dir/SKILL.md.tmp" "$dir/SKILL.md"
)
```

Replace `$HOME/<agent skill folder>`, inside the quotes, with the skill folder's path under your
home, or with its absolute path. Do not write `~` inside quotes: a quoted `~`
does not expand.

The subshell's `umask 077` makes new folders and the new file owner-only,
without changing your shell's umask. Existing folders keep their modes. Each
step runs only if the previous step succeeded, and the skill is written to
`SKILL.md.tmp` first. If `tmx --skill` fails, the command stops and leaves any
existing `SKILL.md` unchanged; delete `SKILL.md.tmp` and run the command again.
The skill contains no credential. Rerun the command after a CLI update. See
[Manual skill installation with `tmx --skill`](cli.md#manual-skill-installation-with-tmx---skill).

## Common errors

| Code | Cause | Action |
| --- | --- | --- |
| `external_gateway.listener_required` | `connect`, `rotate` or `repair` while the listener is stopped. | Send `listener.set` `{"enabled":true}`, then preview again. |
| `external_gateway.managed_setup_required` | `connect` while already connected, or `rotate` while not connected. | Inspect, then use `rotate` or `connect`. |
| `external_gateway.credential_consent_required` | `repair` has nothing it can repair without a new credential. | Use `connect` or `rotate`. |
| `external_gateway.not_found` | `disconnect` when this Workspace was never connected. | No action is needed. |
| `external_gateway.setup_preview_stale` | The plan expired, or the state changed after the preview. | Preview again, then apply the new plan. |
| `external_gateway.busy` | Other setup work is in progress. | Poll `setup.status` until `phase` is not `accepted`, `prepared`, `outputs` or `committed`, then try again. |
| `external_gateway.busy` | Four previews are open. Polling does not close them. | Apply one of them, which discards the other open previews, or wait for them to expire ten minutes after each was created. |
| `external_gateway.setup_repair_required` | An unfinished operation blocks new work. | Use `repair`. |
| `external_gateway.connection_origin_conflict` | Another Installation or profile owns this Workspace's local connection. | Disconnect from the original profile. See [CLI usage and client custody](cli.md#credential-custody). |
| `external_gateway.output_collision` | A file at the command path is not a recognized install. | Inspect and remove or move the file yourself, then preview again. |
| `external_gateway.output_consent_required` | The command needs an explicit update. | Preview and apply `install-cli`. |
| `external_gateway.output_unsafe` | A directory on the output path is unsafe: it is group- or world-writable, or it is owned by another user. The path runs through the home directory and its ancestors, `~/.local`, `~/.local/bin`, `~/.local/share`, `~/.local/share/teamarness` and `~/.local/share/teamarness/external-gateway`. Only the home directory's ancestors may be owned by `root`, and a `root`-owned ancestor with the sticky bit, such as `/tmp`, may be writable by others. The home directory and every directory under it that setup uses must be owned by you. A group-writable directory is common when it was created under Ubuntu's default `0002` umask. `setup.inspect` reports this code in `cli.issue`, with `cli.state` `blocked`. `setup.preview` fails with it for `install-cli`, and for `connect`, `rotate`, `repair` and `install-skill` when the directory became unsafe after the Backend activated. `disconnect` does not preview the command: it fails with `external_gateway.client_store_invalid` instead, or succeeds when only `~/.local/bin` is unsafe. If a directory on the setup state path was already unsafe when the Backend activated, credential intents fail with `external_gateway.setup_state_invalid` instead; see that row. | Fix the ownership and modes of each such directory. A directory under the home that another user owns must be returned to you, for example with `sudo chown <you> <dir>`. Run `chmod go-w <dir>` on the home directory, `~/.local`, `~/.local/bin` and `~/.local/share`. Run `chmod 700 <dir>` on `~/.local/share/teamarness`, `~/.local/share/teamarness/external-gateway` and, if they exist, its `managed` and `setup` folders, which must have mode exactly `0700`. Then reload the Package through the Host. The headless Host has not defined its reload command yet; see [Host command wrapper (planned, unverified)](#host-command-wrapper-planned-unverified). Then inspect and preview again. |
| `external_gateway.client_store_invalid` | The private client store under `~/.local/share/teamarness/external-gateway/` is unsafe or unreadable. For example, the home directory, `~/.local` or `~/.local/share` is not owned by you or is group- or world-writable, or `~/.local/share/teamarness` or `~/.local/share/teamarness/external-gateway` is not owned by you or does not have mode exactly `0700`. `setup.inspect` reports it as `connection.issue`. | Fix the ownership and modes as for `output_unsafe`. Then reload the Package through the Host. The headless Host has not defined its reload command yet; see [Host command wrapper (planned, unverified)](#host-command-wrapper-planned-unverified). Then inspect and preview again. |
| `external_gateway.setup_state_invalid` | When the Backend activated, it had no unfinished operation loaded and could not read or recover its setup state under `~/.local/share/teamarness/external-gateway/setup`. There are two causes. A directory on that path was unsafe: the home directory or one of its ancestors, `~/.local`, `~/.local/share`, `~/.local/share/teamarness`, `~/.local/share/teamarness/external-gateway` or `setup`, which must have mode exactly `0700`. Or a state file in `setup` is not valid. Until the Package is reloaded, every `setup.preview` except `install-cli` fails with this code. After you fix an unsafe directory, `setup.inspect` can report `recovery.state` `none`, but the refusal stays until the reload. When a state file is not valid, `setup.inspect` reports `recovery.state` `invalid`, and a reload fails the same way. | For an unsafe directory, fix the ownership and modes as for `output_unsafe`. Then reload the Package through the Host. The headless Host has not defined its reload command yet; see [Host command wrapper (planned, unverified)](#host-command-wrapper-planned-unverified). Then inspect and preview again. For a state file that is not valid, do not edit, move or delete it. Only `install-cli` remains available. Report the problem with the `setup.inspect` output. |
| `external_gateway.home_unavailable` | The Backend could not resolve its OS user's home. The Backend does not activate. | See [Prerequisites](#prerequisites). |
| `external_gateway.reload_required` | The Backend's admitted bytes changed. | Reload the Package through the Host, then preview again. The headless Host has not defined its reload command yet. |
| `external_gateway.forbidden` | `tmx call` to a tool that is not selected. | Select the tool with `tool.set`. |
| `external_gateway.unauthorized` | The credential was revoked or rotated elsewhere. | Inspect, then use `rotate` or `connect`. |
| `cli.workspace_not_configured` | No local connection exists for that Workspace. | Connect the Workspace. |
| `cli.home_unavailable` | `tmx` could not resolve the current OS user's home through the OS account database (`getent passwd <effective-uid>` on Linux). `tmx` never reads `HOME`. | Make the account resolvable, then run the command again. |
| `cli.workspace_required` | `tmx` was run without `--workspace`. | Add `--workspace <workspace-id>`. |
| `cli.command_outcome_unknown` | A call was sent, but no complete response arrived. | Do not retry. Check the result through the Provider. |

A setup failure that leaves `phase` `repair-required` in `setup.status` also
shows a code in `error`. Inspect, then follow the repair table.

## Host command wrapper (planned, unverified)

> **Planned, unverified.** The Teamarness Host has not released its headless
> operator command. The grammar below comes from the Host's plan and can
> change. Do not use it as a reference until the Host documents it.

The planned Host interface lists, describes and sends Package requests. It
requires an explicit Workspace and Package on every command:

```text
<host command> package operations --workspace <workspace-id> --package dev.teamarness.external-gateway
<host command> package schema --workspace <workspace-id> --package dev.teamarness.external-gateway --operation external-gateway.setup.preview
<host command> package request --workspace <workspace-id> --package dev.teamarness.external-gateway --operation external-gateway.setup.preview --contract-digest <digest> --input <file|->
```

Under that plan, each request body above goes in `--input`, as a private file or
on stdin with `-`. `--contract-digest` carries the admitted contract digest as
the Host reports it.
