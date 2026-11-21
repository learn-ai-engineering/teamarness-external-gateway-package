# Install External Gateway on a headless Linux Host

Use this guide to build External Gateway, install it on a headless Teamarness
Installation, and set it up in each Workspace. When you finish, external agents
on the same machine can call Workspace tools through `~/.local/bin/tmx`.

One setup command does the work, from an answers file.
[Headless onboarding](headless-onboarding.md) explains each request that setup
sends, and the requests setup never sends: verify, rotate, disconnect, repair,
and the error codes.

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

Run the Host, the Gateway, `tmx`, and every external agent that uses `tmx`
under the same unprivileged OS account. The Gateway resolves that account's
home directory with `getent passwd "$(id -u)"` and never reads `HOME`.

## Set up with an answers file

`external-gateway-setup` installs the Package, installs `tmx`, starts the
listener, connects the Workspace, and sets the tool selection, from one answers
file. It reads the current state first and changes only what differs, so you
can run it again at any time. It needs `python3` 3.11 or later and no secret.

1. Build the Package. Release archives do not exist yet, so build it on a
   native Linux x86_64 build machine, which can be the target machine. The build
   machine needs rustup, Node.js and npm, git, a C toolchain, and binutils
   (`strip`, which a test needs). From a clean checkout of this repository:

   ```sh
   npm ci
   npm run build
   ```

   The build validates the result and runs a smoke launch of the Backend. It
   writes the Package Directory `dist/dev.teamarness.external-gateway/` and the
   setup command `dist/external-gateway-setup`. If several builds share the
   build machine, run each under one lock, for example
   `flock ~/src/.heavy-build.lock npm run build`.

2. On the target, create the Packages directory. The Gateway checks every
   directory above its own executable: each one must be owned by you or by
   `root`, must not be group- or other-writable, and must not be a symbolic
   link. Ubuntu's default `0002` umask makes `mkdir -p` create group-writable
   directories, so use `umask 077`:

   ```sh
   umask 077
   mkdir -p ~/.local/lib/teamarness-packages
   chmod go-w ~ ~/.local ~/.local/lib
   ```

3. From the build machine, copy the Package Directory and the setup command
   beside it. Copy each one by name. Do not copy all of `dist/` into
   `teamarness-packages`, because `--delete` there would remove other Packages.
   Do not use a symbolic link, and do not edit any file after the copy; the Host
   checks the declared sizes, digests, and modes:

   ```sh
   rsync -a --delete ~/src/external-gateway-package/dist/dev.teamarness.external-gateway/ target:.local/lib/teamarness-packages/dev.teamarness.external-gateway/
   rsync -a ~/src/external-gateway-package/dist/external-gateway-setup target:.local/lib/teamarness-packages/external-gateway-setup
   ```

   If the build machine is the target, use `cp -a` instead.

4. Write the answers template, then fill it in:

   ```sh
   ~/.local/lib/teamarness-packages/external-gateway-setup --print-template ~/gateway-answers.toml
   ```

   - Set `workspace` to the Workspace name as shown in the desktop app, or its
     ID. Both are on the `workspace:` line of Host setup. Setup stops with exit
     2 when no Workspace matches, or when a name matches more than one, and
     lists the candidate IDs; set `workspace` to one of them. An older answers
     file that sets `workspace_id` (an ID only) still works; set one of the two
     keys, not both.
   - Set `accept_native_code_risks = true` to accept the native-code risks that
     the template describes: setup writes `~/.local/bin/tmx` and a private
     client store (filesystem), listens on a `127.0.0.1` port (network), and
     runs `getent` (process).
   - Choose `tools`. The answers file is the whole selection. `tools = "all"`
     exposes every tool of every available Package. To expose less, use the
     `[tools]` table that the template lists, commented out, with every Package
     and tool: uncomment the Packages you want and trim their lists. Each run
     turns off every tool and Package that the file does not name, including
     tools that someone selected in the Gateway settings since the last run.

5. Apply it:

   ```sh
   ~/.local/lib/teamarness-packages/external-gateway-setup --answers ~/gateway-answers.toml
   ```

   Setup first prints `workspace: <name> <id>`. Each step then prints one line,
   `unchanged:`, `changed:` or `stopped:`, followed by a summary. A successful
   first run ends with `check tmx reaches http://127.0.0.1:<port>`. A second run
   with the same file reports only `unchanged:` lines.

For another Workspace, copy the answers file, change `workspace`, and apply
it. The Package and `tmx` are already installed, so only that Workspace's
listener, connection and tools change.

Exit status 0 means done, 1 means stopped, and 2 means the answers file names
no single Workspace, an unavailable Package or an unknown tool, or the file or
the invocation is invalid. Setup checks all of this before its first change, so
exit 2 means nothing changed. When a line says `outcome unknown`, run the
inspect command that it prints, then run setup again. Setup never retries a
change.

Setup removes tools first and adds them after, so a stop part-way never exposes
more than the answers file allows. A Package that is unavailable when setup
runs, for example while its Provider restarts, keeps its saved selection, and
setup says so. Setup never disconnects, rotates, repairs, installs a skill, or
reloads the Package. Use the requests in
[headless onboarding](headless-onboarding.md) for those.

## Check the Gateway

These commands only read. They work in a fresh shell. Set `WS` to the Workspace
name as shown in the desktop app, or its ID:

```sh
source ~/.config/teamarness-headless/helpers.sh
WS="<Workspace name or ID>"
W=$(tm workspace list | python3 -I -c 'import json,sys; w=sys.argv[1]; m=[r["id"] for r in json.load(sys.stdin) if w in (r["id"], r["name"])]; print(m[0]) if len(m)==1 else sys.exit(f"{len(m)} Workspaces match {w!r}: {m}")' "$WS") && echo "W=$W"
GW=dev.teamarness.external-gateway
echo '{}' | tm_req "$W" "$GW" external-gateway.setup.inspect
echo '{}' | tm_req "$W" "$GW" external-gateway.status.get
echo '{}' | tm_req "$W" "$GW" external-gateway.setup.status
~/.local/bin/tmx workspaces
~/.local/bin/tmx --workspace "$W" status
~/.local/bin/tmx --workspace "$W" call dev.teamarness.conversations channel.list --major 1 --json '{}'
```

| Command | Expected result |
|---|---|
| `setup.inspect` | `cli.state` is `installed`; `connection.state` is `connected` and `authority` is `current` |
| `status.get` | `desiredEnabled` and `running` are `true`, with a `port`; `selectedTools` matches the answers file |
| `setup.status` | `phase` is `complete` |
| `tmx workspaces` | one line for each connected Workspace, each with its own port |
| `tmx status` | `reachable` is `true`, with the same Workspace ID |
| `channel.list` | the Workspace's visible Channels, including `#general` (needs `channel.list` selected) |

For each error code and its fix, see
[Common errors](headless-onboarding.md#common-errors). If a request reports
`outcomeUncertain: true`, inspect the state before you do anything else. Do not
repeat the request automatically.

## Hand over to external agents

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
source ~/.config/teamarness-headless/helpers.sh
tm packages reload --package dev.teamarness.external-gateway
```

Reload applies to every Workspace. Each Workspace keeps its listener port,
tool selection, and connection. Then run
`external-gateway-setup --answers` with each Workspace's answers file. It
updates `tmx` when needed.

## Manual setup (reference)

To send the requests one at a time instead of running setup, install the
Package with `tm packages inspect --directory <dir>` and
`tm packages install --directory <dir> --package dev.teamarness.external-gateway --input -`,
with `{"acceptedRisks":["filesystem","network","process"]}` on stdin. Then follow
[headless onboarding](headless-onboarding.md). It gives each Workspace request
with its exact JSON body; send each one with `tm_req`, as in
[Send requests through the Host](headless-onboarding.md#send-requests-through-the-host).
