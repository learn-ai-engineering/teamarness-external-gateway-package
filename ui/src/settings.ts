import { describePackageError, isAbortError, type TypedPackageClient } from "@teamarness/package-ui-sdk";
import type { PackageContract, Status } from "./contract.generated";
import type { SetupIntent } from "./setup";
import { action, group, input, node } from "./dom";

type Requests = PackageContract["requests"];
export type Client = TypedPackageClient<PackageContract>;

export function settings(root: HTMLElement, client: Client, initial: Status | undefined, refreshStatus: () => void, runSetup: (intent: SetupIntent) => void) {
  const pending = new AbortController();
  let alive = true;
  let busy = false;
  let status = initial;
  const error = node("p"); error.setAttribute("role", "alert");
  const controls = node("fieldset");
  const label = input("Token label"); label.maxLength = 96;
  const tokens = node("section");
  const secret = node("section"); secret.setAttribute("aria-label", "One-time token");
  const confirmation = node("dialog"); confirmation.setAttribute("aria-label", "Confirm disable and revoke");
  const clearSecret = () => { secret.replaceChildren(); };
  const showSecret = (bearer: string) => {
    secret.replaceChildren(node("h3", "Save this token now"), node("p", "Shown only once. Use your client's local secret mechanism, not saved guidance. If this response is lost, regenerate the token."), node("code", bearer), action("Dismiss token", clearSecret));
  };
  const mutate = async <K extends keyof Requests>(operation: K, payload: Requests[K]["request"]) => {
    if (!alive || busy) return;
    busy = true; controls.disabled = true; error.textContent = ""; clearSecret();
    try {
      const response = await client.request(operation, payload, { signal: pending.signal });
      if (!alive) return;
      if ("bearer" in response) showSecret(response.bearer);
      if ("warning" in response && response.warning) error.textContent = response.warning;
      refreshStatus();
    } catch (cause) {
      if (alive && !isAbortError(cause)) error.textContent = describePackageError(cause);
    } finally { busy = false; controls.disabled = false; if (alive && status) update(status); }
  };
  const update = (value: Status) => {
    status = value;
    controls.disabled = busy;
    label.disabled = value.storageFailed;
    create.disabled = value.storageFailed;
    tokens.replaceChildren();
    for (const token of value.tokens) {
      const row = node("div"); row.className = "connection";
      const rotate = action(`${token.managed ? "Rotate" : "Regenerate"} ${token.label}`, () => { if (token.managed) { clearSecret(); runSetup("rotate"); } else void mutate("external-gateway.token.regenerate", { id: token.id }); });
      rotate.disabled = value.storageFailed;
      row.append(node("p", token.label), rotate, action(`Revoke ${token.label}`, () => { void mutate("external-gateway.token.revoke", { id: token.id }); }));
      tokens.append(row);
    }
  };
  const create = action("Create token", () => {
    if (!label.value.trim() || new TextEncoder().encode(label.value).length > 96) { error.textContent = "Token label must be 1–96 UTF-8 bytes."; return; }
    void mutate("external-gateway.token.create", { label: label.value });
  });
  controls.append(node("label", "Token label"), label, create, tokens, action("Disable and revoke all tokens", () => {
    clearSecret();
    confirmation.replaceChildren(node("h3", "Disable and revoke all tokens?"), node("p", "Stops listening and revokes every token, including managed access. Admitted work may finish. Installed command and skill files remain."), action("Cancel revoke all", () => { confirmation.close(); }), action("Disable and revoke", () => { confirmation.close(); void mutate("external-gateway.disable-and-revoke-all", {}); }));
    confirmation.showModal();
  }));
  const body = group(root, "Tokens");
  const details = node("details"); details.className = "long-details";
  details.append(node("summary", "Access and lifecycle details"), node("p", "All tokens share the same Package and exact tool permissions. Every active token gains each explicitly enabled operation. Writes act as the local Workspace Person and can have lasting effects; streams remain live until closed or denied. Labels are not identities or separate permissions."), node("p", "Calls that passed local admission may still be sent to the Host and finish after disable, revoke or regeneration succeeds, including commands waiting inside the SDK. Later admissions use the new policy. To stop the external agent's work, use the external agent's own stop control; revocation is not cancellation."), node("p", "Enabled setup, saved port and tokens survive restart and reinstall. Hiding this UI or switching Workspaces does not stop the listener. Use Disable and revoke all tokens before removal if retained access is unwanted."));
  body.append(node("small", "All tokens share the enabled tools."), controls, secret, error, details, confirmation);
  if (initial) update(initial);
  return { update, stop: () => { alive = false; pending.abort(); confirmation.close(); confirmation.replaceChildren(); clearSecret(); status = undefined; } };
}
