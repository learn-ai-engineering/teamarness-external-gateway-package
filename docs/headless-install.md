# Install External Gateway on a headless Linux Host

Use this guide to build External Gateway, install it on a headless Teamarness
Installation, and set it up in each Workspace. When you finish, external agents
on the same machine can call Workspace tools through `~/.local/bin/tmx`.

The primary path is one setup command with an answers file. The manual steps
after it do the same thing one request at a time, and serve as the fallback and
reference. [Headless onboarding](headless-onboarding.md) explains each setup
request, including verify, rotate, disconnect, repair, and the error codes.

> [!NOTE]
> This sequence was verified on Ubuntu 26.04 x86_64 on 2026-11-21, with
> External Gateway 1.1.0 in two Workspaces. Other x86_64 Linux distributions
> with systemd and glibc are expected to work but are not verified.

## Prerequisite: a running headless Host

You need a running headless Teamarness Host on the target machine that has
joined at least one Workspace. Complete the Host guide, "Install headless
Teamarness on Linux" (`docs/headless/install-linux.md` in the Teamarness
repository), through "Run setup", with a Workspace Invitation, first. Host setup
prints the Workspace name and ID on its `workspace:` line.

The setup command below needs no helpers. The manual steps use the Host's `tm`
and `tm_req` helpers. Load them in each shell first:

```sh
source ~/.config/teamarness-headless/helpers.sh
```

Run the Host, the Gateway, `tmx`, and every external agent that uses `tmx`
under the same unprivileged OS account. The Gateway resolves that account's
home directory with `getent passwd "$(id -u)"` and never reads `HOME`.

Each command below runs on the target machine, unless the step says that it
runs on the build machine.

## Set up with an answers file

`external-gateway-setup` installs the Package, installs `tmx`, starts the
listener, connects the Workspace, and selects tools from one answers file. It
reads the current state first and changes only what differs, so you can run it
again at any time. It needs `python3` 3.11 or later and no secret.

1. Build the Package (step 1 below). On the target, create the Packages
   directory. No parent directory may be group- or other-writable (see step 2
   below):

   ```sh
   umask 077
   mkdir -p ~/.local/lib/teamarness-packages
   chmod go-w ~ ~/.local ~/.local/lib
   ```

2. From the build machine, copy the Package Directory and the setup command
   beside it. Copy each one by name. Do not copy all of `dist/` into
   `teamarness-packages`, because `--delete` there would remove other Packages:

   ```sh
   rsync -a --delete ~/src/external-gateway-package/dist/dev.teamarness.external-gateway/ target:.local/lib/teamarness-packages/dev.teamarness.external-gateway/
   rsync -a ~/src/external-gateway-package/dist/external-gateway-setup target:.local/lib/teamarness-packages/external-gateway-setup
   ```

3. Write the answers template, then fill it in. Set `workspace` to the
   Workspace name as shown in the desktop app, or its ID. Both are on the
   `workspace:` line of Host setup. To list every Workspace on the Host, source
   the helpers (`source ~/.config/teamarness-headless/helpers.sh`) and run
   `tm workspace list`. Setup looks the value up in that list. It stops with
   exit 2 when no Workspace matches, or when a name matches more than one, and
   lists the candidate IDs; set `workspace` to one of them. An older
   answers file that sets `workspace_id` (an ID only) still works; set one of
   the two keys, not both. Set
   `accept_native_code_risks = true` to accept the native-code risks that the
   template describes. Choose `tools`:

   ```sh
   ~/.local/lib/teamarness-packages/external-gateway-setup --print-template ~/gateway-answers.toml
   ```

4. Apply it:

   ```sh
   ~/.local/lib/teamarness-packages/external-gateway-setup --answers ~/gateway-answers.toml
   ```

   Setup first prints `workspace: <name> <id>`. Each step then prints one line,
   `unchanged:`, `changed:` or `stopped:`, followed by a summary. A successful first run ends with `check tmx reaches
   http://127.0.0.1:<port>`. A second run with the same file reports only
   `unchanged:` lines.

For another Workspace, copy the answers file, change `workspace`, and apply
it. The Package and `tmx` are already installed, so only that Workspace's
listener, connection and tools change.

Exit status 0 means done, 1 means stopped, and 2 means the answers file names no single Workspace, or the file or the
invocation is invalid. Setup checks the file before it sends any request. It
finds an unknown contract or port only when it reads the catalog, which can be
after it has enabled that contract; the output lists every change it made. When a line says `outcome unknown`, run
the inspect command that it prints, then run setup again. Setup never retries a
change.

Setup only adds: it never disconnects, rotates, repairs, removes tools,
installs a skill, or reloads the Package. Use the requests in
[headless onboarding](headless-onboarding.md) for those, and give the skill to
agents as described in [Hand over to external agents](#hand-over-to-external-agents).

The rest of this guide is the manual path.

## Machine tier

Do these steps once per machine.

### 1. Get the Package

Release archives do not exist yet. Until they do, build the Package on a
native Linux x86_64 build machine. The build machine can be the target
machine. It needs rustup, Node.js and npm, git, a C toolchain, and binutils
(`strip`, which a test needs).

On the build machine, from a clean checkout of this repository:

```sh
npm ci
npm run build
```

`npm run build` builds only for the native target, `x86_64-unknown-linux-gnu`,
validates the result, and runs a smoke launch of the Backend. The output is the
complete Package Directory `dist/dev.teamarness.external-gateway/`. If several
builds share the build machine, run each under one lock, for example
`flock ~/src/.heavy-build.lock npm run build`.

> [!NOTE]
> Update this step when release archives exist. The step then becomes
> "download and unpack the archive".

### 2. Copy the Package Directory

The Gateway checks every directory above its own executable. Each directory
from `/` down to the Package Directory must be owned by you or by `root`, must
not be group- or other-writable, and must not be a symbolic link. Ubuntu's
default `0002` umask makes `mkdir -p` create group-writable directories. The
Host setup already removes group and other write access from the directories
you own above its install location, including `~/.local/lib`. Create the
Package directory under `umask 077`, and run the check again:

```sh
umask 077
mkdir -p ~/.local/lib/teamarness-packages
chmod go-w ~ ~/.local ~/.local/lib
```

Copy the complete directory from the build machine. Do not use a symbolic
link, and do not edit any file after the copy. The Host checks the declared
sizes, digests, and modes.

```sh
rsync -a --delete ~/src/external-gateway-package/dist/dev.teamarness.external-gateway/ target:.local/lib/teamarness-packages/dev.teamarness.external-gateway/
```

If the build machine is the target, use `cp -a` into
`~/.local/lib/teamarness-packages/` instead.

### 3. Inspect and install the Package

Inspection does not run the Backend:

```sh
GW_DIR="$HOME/.local/lib/teamarness-packages/dev.teamarness.external-gateway"
tm packages inspect --directory "$GW_DIR"
```

The Gateway needs all three native-code consents. It writes `~/.local/bin/tmx`
and its private client store under your home directory (filesystem). It listens
on a `127.0.0.1` port (network). It runs `getent` (process). Install it with
that consent:

```sh
printf '%s\n' '{"acceptedRisks":["filesystem","network","process"]}' |
  tm packages install --directory "$GW_DIR" --package dev.teamarness.external-gateway --input -
```

The install returns `"kind":"personal_installed"`. One installation serves
every Workspace that the Installation joins. Install one External Gateway per
Installation.

## Workspace tier

Repeat these steps for each Workspace, after Host setup has joined it. Set `W` to the Workspace ID from `tm workspace list` (with the helpers sourced, as in the prerequisite), for example
`W=aad43be122caa818738f763c373ba820`. Always select Workspaces by ID.

`tmx` is installed once for your OS user. The listener and its port, the tool
selection, and the connection and its credential belong to each Workspace.
None of them carries over from another Workspace.

### 4. Define the Gateway helpers

Run `source ~/.config/teamarness-headless/helpers.sh` first, then paste these
helpers into the same shell.
`gw` sends one Gateway request with its JSON body on stdin. `gw_plan` previews
and applies one setup intent, then waits until the operation finishes:

```sh
GW=dev.teamarness.external-gateway
gw() { tm_req "$W" "$GW" "external-gateway.$1"; }
gw_plan() {
  local plan phase
  plan=$(printf '{"intent":"%s"}\n' "$1" | gw setup.preview |
    python3 -I -c 'import json,sys; print(json.load(sys.stdin)["planId"])') || return
  printf '{"planId":"%s"}\n' "$plan" | gw setup.apply || return
  for _ in $(seq 60); do
    phase=$(echo '{}' | gw setup.status |
      python3 -I -c 'import json,sys; d=json.load(sys.stdin); print(d["phase"]); print(json.dumps(d), file=sys.stderr)') || return
    case "$phase" in accepted|prepared|outputs|committed) sleep 2 ;; *) echo "phase: $phase"; return ;; esac
  done
  echo "setup still running; check setup.status before you do anything else" >&2
  return 1
}
```

Each setup apply is a mutation. If a helper fails partway, run
`echo '{}' | gw setup.inspect` and decide from its output. Do not run the
helper again automatically.

### 5. Inspect the setup

```sh
echo '{}' | gw setup.inspect
```

`cli.issue` must be `null`. If it is `external_gateway.output_unsafe`, fix the
directory modes as described in step 2 and inspect again. In the first
Workspace, `cli.state` is `missing`. In later Workspaces, it is `installed`.

### 6. Install `tmx` (first Workspace only)

Skip this step when `cli.state` is `installed`:

```sh
gw_plan install-cli
```

Expect `phase: complete`, with `command` `installed`. This step installs
`~/.local/bin/tmx`. It creates no credential and does not start the listener.

### 7. Start the listener

```sh
echo '{"enabled":true}' | gw listener.set
echo '{}' | gw status.get
```

`status.get` reports `running` `true` and this Workspace's port. The listener
binds only to `127.0.0.1`. The operating system assigns the port the first
time the listener starts, and the Gateway reuses that port afterwards.

### 8. Connect this Workspace

```sh
gw_plan connect
```

Expect `phase: complete`, with `access` `configured`. Connect creates the
Workspace's managed credential and stores it in the private client store under
`~/.local/share/teamarness/external-gateway/`. No response contains the
credential, and you supply no secret. Do not copy the credential from the
store.

### 9. Select Providers and tools

Nothing is selected by default, and selection never carries over from another
Workspace. Until you select a tool, every `tmx call` fails with
`external_gateway.forbidden`.

List the contracts first:

```sh
echo '{}' | gw catalog.page
```

Follow every `nextCursor` with `{"cursor":"<nextCursor>"}` until it is `null`.
`gw_select` enables one contract and selects each of its eligible tools that
is not already selected. It reads the Provider from the first contracts page
and follows `nextOffset` tool pages. Check `status.get` afterwards, because a
failed `tool.set` stops that contract's selection:

```sh
gw_select() {
  local contract=$1 provider offset=null body page
  provider=$(echo '{}' | gw catalog.page |
    python3 -I -c 'import json,sys
for r in json.load(sys.stdin)["contracts"]:
    if r["contract"]==sys.argv[1]: print(r["provider"])' "$contract") || return
  [ -n "$provider" ] || { echo "contract not listed: $contract" >&2; return 1; }
  printf '{"provider":"%s","enabled":true,"contracts":[{"contract":"%s","major":1}]}\n' "$provider" "$contract" |
    gw package.set || return
  while :; do
    if [ "$offset" = null ]; then body=$(printf '{"contract":"%s"}' "$contract")
    else body=$(printf '{"contract":"%s","offset":%s}' "$contract" "$offset"); fi
    page=$(printf '%s\n' "$body" | gw catalog.page) || return
    printf '%s' "$page" | python3 -I -c 'import json,sys
c,p=sys.argv[1],sys.argv[2]
for t in json.load(sys.stdin)["tools"]:
    if t["eligible"] and not t["selected"]:
        print(json.dumps({"contract":c,"major":1,"provider":p,"port":t["port"],"fingerprint":t["fingerprint"],"enabled":True}))' "$contract" "$provider" |
      while read -r tool; do printf '%s\n' "$tool" | gw tool.set || break; done
    offset=$(printf '%s' "$page" | python3 -I -c 'import json,sys; o=json.load(sys.stdin)["nextOffset"]; print("null" if o is None else o)')
    [ "$offset" = null ] && break
  done
}
for c in dev.teamarness.conversations dev.teamarness.documents dev.teamarness.backlog; do gw_select "$c"; done
echo '{}' | gw status.get
```

Select only the contracts that the external agent needs. `runtime.guide` tools
are not eligible, and the helper skips them. To select single tools instead,
send `package.set` once, then one `tool.set` per tool, as shown in
[headless onboarding](headless-onboarding.md#4-select-the-tools-to-expose).
`status.get` reports `selectedContracts` and `selectedTools` for this
Workspace.

### 10. Test with `tmx`

Run these as the same OS user:

```sh
~/.local/bin/tmx workspaces
~/.local/bin/tmx --workspace "$W" status
~/.local/bin/tmx --workspace "$W" call dev.teamarness.conversations channel.list --major 1 --json '{}'
```

`workspaces` lists each connected Workspace with its own port. `status`
reports `reachable` `true` for this Workspace. The call returns the Workspace's
visible Channels.

### Hand over to external agents

Give each external agent the exact command `~/.local/bin/tmx`, the Workspace
ID, and the skill from `~/.local/bin/tmx --skill`. To install the skill into an
agent's skill folder, see
[Install the skill manually](headless-onboarding.md#install-the-skill-manually).
There is no folder chooser on Linux.

Tell agents to pass the Workspace ID, not the name, to `--workspace`. Record
IDs can repeat across Workspaces. For example, `#general` has the same ID in
every Workspace, so a call with the wrong `--workspace` succeeds in the wrong
Workspace without an error. Calls through the Gateway act as this
Installation's Workspace Person.

## Update the Package

Build the new version, copy it over the same directory, and reload it:

```sh
rsync -a --delete ~/src/external-gateway-package/dist/dev.teamarness.external-gateway/ target:.local/lib/teamarness-packages/dev.teamarness.external-gateway/
rsync -a ~/src/external-gateway-package/dist/external-gateway-setup target:.local/lib/teamarness-packages/external-gateway-setup
tm packages reload --package dev.teamarness.external-gateway
```

Reload applies to every Workspace. Each Workspace keeps its listener port,
tool selection, and connection. `tm_req` looks up the new contract digest for
you. After a reload, run `external-gateway-setup --answers` with each
Workspace's answers file, which updates `tmx` when needed. By hand, inspect
each Workspace with `echo '{}' | gw setup.inspect`, and if `cli.change` is
`update`, run `gw_plan install-cli` once.

## Check the Gateway

| Check | Command |
|---|---|
| Setup state | `echo '{}' \| gw setup.inspect` |
| Listener and selection | `echo '{}' \| gw status.get` |
| Last setup operation | `echo '{}' \| gw setup.status` |
| Client view | `~/.local/bin/tmx --workspace "$W" status` |

For each error code and its fix, see
[Common errors](headless-onboarding.md#common-errors). If a request reports
`outcomeUncertain: true`, inspect the state before you do anything else. Do not
repeat the request automatically.
