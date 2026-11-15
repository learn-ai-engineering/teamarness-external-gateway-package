import { describePackageError, isAbortError, type TypedPackageClient } from "@teamarness/package-ui-sdk";
import type { PackageContract, Status } from "./contract.generated";
import { catalogView } from "./catalog";
import { action, input, node } from "./dom";

type Requests = PackageContract["requests"];
export type Client = TypedPackageClient<PackageContract>;

export function settings(root: HTMLElement, client: Client, initial: Status | undefined, refreshStatus: () => void) {
  const pending = new AbortController();
  let alive = true;
  let busy = false;
  let status = initial;
  const error = node("p"); error.setAttribute("role", "alert");
  const controls = node("fieldset");
  const listener = node("section");
  const port = input("Saved port", "number"); port.min = "1"; port.max = "65535";
  port.value = initial?.port?.toString() ?? "";
  const label = input("Token label"); label.maxLength = 96;
  const tokens = node("section");
  const secret = node("section"); secret.setAttribute("aria-label", "One-time token");
  const catalog = node("fieldset");
  const clearSecret = () => { secret.replaceChildren(); };
  const showSecret = (bearer: string) => {
    secret.replaceChildren(node("h3", "Save this token now"), node("p", "Shown only once. Use your client's local secret mechanism, not saved guidance. If this response is lost, regenerate the token."), node("code", bearer), action("Dismiss token", clearSecret));
  };
  const mutate = async <K extends keyof Requests>(operation: K, payload: Requests[K]["request"]) => {
    if (!alive || busy) return;
    busy = true; controls.disabled = true; catalog.disabled = true; catalogMount.pause(); error.textContent = ""; clearSecret();
    try {
      const response = await client.request(operation, payload, { signal: pending.signal });
      if (!alive) return;
      if ("bearer" in response) showSecret(response.bearer);
      if ("warning" in response && response.warning) error.textContent = response.warning;
      refreshStatus();
    } catch (cause) {
      if (alive && !isAbortError(cause)) error.textContent = describePackageError(cause);
    } finally { busy = false; controls.disabled = false; catalog.disabled = false; if (alive) catalogMount.resume(); }
  };
  const update = (value: Status) => {
    status = value;
    listener.replaceChildren(action(value.desiredEnabled ? "Disable listener" : "Enable listener", () => { void mutate("external-gateway.listener.set", { enabled: !status?.desiredEnabled }); }));
    tokens.replaceChildren(node("h3", "Tokens"));
    for (const token of value.tokens) {
      const row = node("div");
      row.append(node("p", `${token.label} · generation ${token.generation}`), action(`Regenerate ${token.label}`, () => { void mutate("external-gateway.token.regenerate", { id: token.id }); }), action(`Revoke ${token.label}`, () => { void mutate("external-gateway.token.revoke", { id: token.id }); }));
      tokens.append(row);
    }
  };
  controls.append(listener, node("label", "Saved port (1–65535)"), port, action("Change port", () => {
    const value = Number(port.value);
    if (!Number.isInteger(value) || value < 1 || value > 65535) { error.textContent = "Port must be an integer from 1–65535."; return; }
    void mutate("external-gateway.port.set", { port: value });
  }), node("label", "Token label"), label, action("Create token", () => {
    if (!label.value.trim() || new TextEncoder().encode(label.value).length > 96) { error.textContent = "Token label must be 1–96 UTF-8 bytes."; return; }
    void mutate("external-gateway.token.create", { label: label.value });
  }), tokens, action("Disable and revoke all tokens", () => { void mutate("external-gateway.disable-and-revoke-all", {}); }));
  root.append(node("h2", "Settings"), node("p", "All tokens share the same Package and exact tool permissions. Labels are not identities or separate permissions."), node("p", "Reads that passed local admission may still be sent to the Host and finish after disable, revoke or regeneration succeeds, including reads waiting inside the SDK. Later admissions use the new policy. To stop the external agent's work, use the external agent's own stop control; revocation is not cancellation."), node("p", "Enabled setup, saved port and tokens survive restart and reinstall. Hiding this UI or switching Workspaces does not stop the listener. Use Disable and revoke all tokens before removal if retained access is unwanted."), controls, secret, error, catalog);
  if (initial) update(initial);
  const catalogMount = catalogView(catalog, client, mutate);
  return { update, stop: () => { alive = false; pending.abort(); catalogMount.stop(); clearSecret(); status = undefined; } };
}
