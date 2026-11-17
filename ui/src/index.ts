import { adoptPackageStyles, describePackageError, isAbortError, typedPackageClient, type PackageMountContext, type PackageSubscription, type PackageUnmount } from "@teamarness/package-ui-sdk";
import type { PackageContract, Status } from "./contract.generated";
import { PACKAGE_STYLES } from "./styles";
import { action, node } from "./dom";
import { setupView } from "./setup";
import { settings } from "./settings";
import onboarding from "../../docs/onboarding.md?raw";

/** Mount lifetime is independent of Backend listener lifetime. */
export function mount(context: PackageMountContext): PackageUnmount {
  const client = typedPackageClient<PackageContract>(context.client);
  const cleanupStyles = adoptPackageStyles(context.root, PACKAGE_STYLES);
  const article = node("article");
  const nav = node("nav"); nav.setAttribute("aria-label", "Gateway views");
  const statusArea = node("section", "Loading…"); statusArea.setAttribute("aria-live", "polite");
  const errorArea = node("p"); errorArea.setAttribute("role", "alert");
  const content = node("section");
  article.append(node("h1", context.package.displayName), node("p", `Workspace: ${context.workspace.name} (${context.workspace.id})`), node("p", "Person-selected · Person-authorized access"), nav, statusArea, errorArea, content);
  context.root.append(article);
  let alive = true;
  let view = context.view;
  let revision = 0;
  let status: Status | undefined;
  let subscription: PackageSubscription | undefined;
  const pending = new AbortController();
  let stopView = () => {};
  let updateView: (value: Status) => void = () => {};
  const fail = (error: unknown) => {
    if (alive && !isAbortError(error)) errorArea.textContent = describePackageError(error);
  };
  const showStatus = (value: Status) => {
    status = value;
    statusArea.replaceChildren(
      node("p", `Desired access: ${value.desiredEnabled ? "enabled" : "disabled"}. ${value.running ? "Listener running" : "Listener stopped"}.`),
      node("p", `Saved port: ${value.port ?? "not allocated"}. ${value.tokens.length} tokens; ${value.selectedContracts} contracts; ${value.selectedTools} tools selected.`),
    );
    if (value.running && value.port) statusArea.append(node("code", `http://127.0.0.1:${value.port}/v1`));
    if (value.storageFailed) statusArea.append(node("p", "Storage failed. Access is closed until a durable action reconciles state."));
    if (value.warning) {
      const warning = value.warning === "membership_removed"
        ? "This Installation is no longer a Workspace member. External access stays stopped; saved setup is retained."
        : value.warning === "membership_unavailable"
          ? "Membership monitoring is unavailable. External access is closed. A live valid notification may recover it; after stream termination, explicitly recover or reload the Package."
          : value.warning;
      statusArea.append(node("p", warning));
    }
    updateView(value);
  };
  const renderView = () => {
    stopView(); updateView = () => {}; content.replaceChildren(); nav.replaceChildren();
    for (const name of ["main", "settings"] as const) {
      const button = action(name === "main" ? "Overview" : "Settings", () => { if (view !== name) { view = name; renderView(); } });
      if (view === name) button.setAttribute("aria-current", "page");
      nav.append(button);
    }
    const setup = setupView(content, client, refreshStatus);
    if (view === "main") {
      content.append(node("h2", "External-agent onboarding"), node("p", "Give the running URL to your outside client. Supply a token separately through its local secret mechanism. Save these instructions without credentials."), node("pre", onboarding));
      stopView = setup.stop;
    } else {
      const mounted = settings(content, client, status, refreshStatus, intent => { void setup.previewIntent(intent); });
      stopView = () => { setup.stop(); mounted.stop(); };
      updateView = mounted.update;
    }
  };
  function refreshStatus() {
    const requestedRevision = ++revision;
    void client.request("external-gateway.status.get", {}, { signal: pending.signal }).then(value => {
      if (alive && revision === requestedRevision) showStatus(value);
    }).catch(fail);
  }
  renderView();
  refreshStatus();
  void client.subscribe("external-gateway.status.watch", {}, value => {
    if (alive) { revision++; showStatus(value); }
  }, { onTerminated: error => { if (alive) { if (error) fail(error); else errorArea.textContent = "Status updates ended. Reopen the view to reconnect."; } } }).then(stream => {
    if (alive) subscription = stream;
    else void stream.close().catch(() => undefined);
  }).catch(fail);
  return () => {
    if (!alive) return;
    alive = false; pending.abort(); stopView(); status = undefined;
    void subscription?.close().catch(() => undefined);
    article.remove(); cleanupStyles();
  };
}
