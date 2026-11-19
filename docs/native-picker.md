# Package-owned folder selection

The native folder chooser exists only on macOS (`aarch64-apple-darwin`). Its RFD,
objc2 and objc2-app-kit dependencies are macOS-only Cargo target dependencies, so
the Linux Backend compiles none of them. On Linux (and any other non-macOS target),
`external-gateway.setup.choose-skill-folder` refuses immediately with the
non-retryable `external_gateway.picker_unavailable`, before it takes picker
capacity, hashes the admitted executable or spawns a helper. Setup that installs
no skill, including the CLI and connection actions, is unaffected. On Linux the
private `--tmx-picker` entry still validates the exact request and lifeline as
below, then emits the closed `picker_failed` error terminal; the private helper
protocol and its error set are unchanged.

The single admitted native executable has three entries: no arguments starts the
Backend, basename `tmx` or `--tmx-cli` starts the offline-first CLI, and exactly
`--tmx-picker` enters the private helper **before any Tokio Backend runtime**.
Invalid arguments do not fall back to Host stdio. CLI help/version never opens
AppKit, resolves a client home or starts a listener.

`external-gateway.setup.choose-skill-folder` takes `{}` and establishes a typed
subscription promptly. One child may browse per Instance; another returns
`external_gateway.picker_busy`. There is no browsing lifetime cap or ordinary
request deadline extension. Complete admitted executable bytes are verified with
Phase-3 `Admitted::verify_source`, retained and identity-checked immediately before
spawn. A mismatch asks for reload, never trusts a filesystem manifest.

The fixed private stdin request is exactly one UTF-8 line:

```json
{"version":1,"operation":"choose-skill-folder"}
```

That pipe remains open as the parent lifeline. Extra input or EOF terminates the
helper, including parent-only SIGKILL. stdout is separate and admits exactly one
newline-terminated closed version-1 selected/cancelled/error object. A selected
path is absolute UTF-8, at most 4,096 bytes, control-free and canonicalized/reopened
in the parent; the entire frame is at most 8,192 bytes. Error codes are fixed safe
identifiers. Extra output, malformed/oversized/early EOF, or nonzero exit never
become selections. stderr is concurrently drained/discarded with an 8-KiB total
budget; its bytes never enter UI errors or logs. After a terminal line, exit and
EOF have a five-second budget. The parent closes the lifeline, kills and reaps on
cancel/failure; lifecycle stop cancels and joins every owned job. No credentials,
arbitrary scripts, helper paths or dialog configuration cross the private pipe.

On macOS the helper explicitly calls `NSApplication.sharedApplication(...).finishLaunching()`
on the OS main thread, then synchronous RFD `pick_folder`. Only the lifeline
reader is a thread; no Backend Tokio runtime or Host connection is constructed.
The modal native work ends with that process. This follows the accepted initialized
prototype, without copying its experimental directory/kind/timeout/env controls.

## Selection lease and consent

A selected event has exactly `outcome`, opaque `selectionId`, and inspectable
canonical `path`. A cancelled event has only `outcome`. Failures terminate with a
Package error. After the one selected result, the child is reaped and the busy
slot is released; the subscription stays open **only as a selection lease** and
emits no more events. At most four leases are retained. Close/reselect/unmount or
suspend/dispose invalidates the selection and any unaccepted preview using it.
Cancel/error retains neither child nor lease. Preview expiration remains ten
minutes. This is mount-scoped memory, not a destination registry.

The UI retains the lease through preview and apply acceptance. Apply rechecks
that the lease is still current at acceptance; thereafter its owned operation
retains the fixed pinned destination. Closing the view/lease cannot cancel or
roll back accepted setup. Status/watch and remount report separate command,
skill and access outcomes. **Install skill…** explains the optional skill and opens the chooser only with
a current recognized CLI. Native folder confirmation immediately runs an internal
exact skill-only preview/apply, retaining the lease through acceptance; Cancel
writes nothing. Routine actions do not display preview/checkbox/apply ceremony
or opaque provenance. A non-unchanged command change or CLI issue refuses skill
apply and asks for the separate Install/Update CLI action first. Credential
actions never carry a selected folder. Invalid or modified skill output offers
reselect/external collision resolution; a CLI collision instead identifies the
command target, never suggests that another skill folder would repair it.
Manual token revoke remains available; managed rotation goes through setup.

## Qualification boundary

`picker-fixture` is an unassembled fault/lifeline executable with private physical
roots, not a native dialog. Protocol/child/32-second health-and-real-call tests
qualify parent ownership and bounded pipes; they subscribe through the production
chooser path, so they run only on macOS. On other targets a refusal test proves
`picker_unavailable` precedes hashing and spawn, and the parent-only SIGKILL
lifeline test still runs; final artifact malformed-request
smoke qualifies dispatch without opening a panel. No automated test resolves or
installs into the ordinary home. Don still owns acquired-artifact/native focus,
Cancel/selection, installed setup and external-agent M1–M5. Prototype acceptance
is not installed production qualification and no AeroSpace cause is asserted.
