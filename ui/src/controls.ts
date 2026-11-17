import { describePackageError, isAbortError } from "@teamarness/package-ui-sdk";
import type { PackageContract, Status } from "./contract.generated";
import type { Client } from "./settings";
import { catalogView } from "./catalog";
import { action, group, icon, node, toggle } from "./dom";

type Requests = PackageContract["requests"];
/** One shared control body for main and Shell Configuration. Policy alone owns authority. */
export function controls(root: HTMLElement, client: Client, refreshStatus: () => void) {
  let alive = true; let busy = false; let status: Status | undefined; let dismissedBind = false;
  const pending = new AbortController();
  const listener = group(root, "Gateway"); listener.setAttribute("aria-live", "polite"); listener.append(node("p", "Loading…"));
  const packages = group(root, "Packages"); const catalog = node("fieldset"); packages.append(catalog);
  const error = node("p"); error.setAttribute("role", "alert"); listener.append(error);
  const mutate = async <K extends "external-gateway.listener.set" | "external-gateway.package.set" | "external-gateway.tool.set">(operation: K, payload: Requests[K]["request"]) => {
    if (!alive || busy) return;
    busy = true; catalog.disabled = true; catalogMount.pause(); error.textContent = "";
    if (operation === "external-gateway.listener.set") dismissedBind = false;
    if (status) render(status);
    try {
      const response = await client.request(operation, payload, { signal: pending.signal });
      if (!alive) return;
      if (response.warning) error.textContent = response.warning;
      refreshStatus();
    } catch (cause) { if (alive && !isAbortError(cause)) error.textContent = describePackageError(cause); }
    finally { busy = false; if (alive) { catalog.disabled = !!status?.storageFailed; catalogMount.resume(); if (status) render(status); } }
  };
  const catalogMount = catalogView(catalog, client, mutate);
  function render(value: Status) {
    const line = node("div"); line.className = "listener-line";
    const details = node("div"); details.append(node("p", value.running ? "Listener running" : "Listener stopped"), node("small", `Saved port: ${value.port ?? "allocated when enabled"}. ${value.tokens.length} tokens; ${value.selectedContracts} contracts; ${value.selectedTools} tools selected.`));
    if (value.running && value.port) details.append(node("code", `http://127.0.0.1:${value.port}/v1`));
    const actions = node("div"); actions.className = "actions";
    if (!value.running && value.desiredEnabled && value.port && value.warning === "listener_bind_failed") actions.append(icon(`Retry listener on port ${value.port}`, "↻", () => { if (status && !status.running) void mutate("external-gateway.listener.set", { enabled: true }); }, busy || value.storageFailed));
    actions.append(toggle("Enable gateway listener", value.desiredEnabled, () => { if (status) void mutate("external-gateway.listener.set", { enabled: !status.desiredEnabled }); }, busy || value.storageFailed));
    line.append(details, actions); listener.replaceChildren(line);
    if (value.storageFailed) listener.append(node("p", "Storage failed. Access is closed until a durable action reconciles state. After restoring storage, use Configuration to revoke a token or Disable and revoke all tokens again."));
    if (value.warning === "listener_bind_failed") {
      if (!dismissedBind) {
        const warning = node("div"); warning.setAttribute("role", "alert"); warning.append(node("p", `listener_bind_failed: Port ${value.port} is unavailable. Free the saved port, then retry. The saved port will not change.`), action("Dismiss bind error", () => { dismissedBind = true; if (status) render(status); })); listener.append(warning);
      }
    } else if (value.warning) listener.append(node("p", value.warning === "membership_removed" ? "This Installation is no longer a Workspace member. External access stays stopped; saved setup is retained." : value.warning === "membership_unavailable" ? "Membership monitoring is unavailable. External access is closed. A live valid notification may recover it; after stream termination, explicitly recover or reload the Package." : value.warning));
    listener.append(error);
  }
  return {
    update: (value: Status) => { if (status?.warning !== value.warning || status?.port !== value.port || value.running) dismissedBind = false; status = value; catalog.disabled = busy || value.storageFailed; render(value); },
    stop: () => { alive = false; pending.abort(); catalogMount.stop(); status = undefined; },
  };
}
