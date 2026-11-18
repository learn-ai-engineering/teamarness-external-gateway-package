import { adoptPackageStyles, describePackageError, isAbortError, typedPackageClient, type PackageMountContext, type PackageSubscription, type PackageUnmount } from "@teamarness/package-ui-sdk";
import type { PackageContract, Status } from "./contract.generated";
import { PACKAGE_STYLES } from "./styles";
import { group, icon, node } from "./dom";
import { setupView } from "./setup";
import { settings } from "./settings";
import { controls } from "./controls";
import onboarding from "../../docs/onboarding.md?raw";

/** Mount lifetime is independent of Backend listener lifetime. */
export function mount(context: PackageMountContext): PackageUnmount {
  const client = typedPackageClient<PackageContract>(context.client);
  const cleanupStyles = adoptPackageStyles(context.root, PACKAGE_STYLES);
  const article = node("article");
  const errorArea = node("p"); errorArea.setAttribute("role", "alert");
  const content = node("section");
  if (context.view === "main") {
    const header = node("header"); const heading = node("div"); heading.className = "heading";
    heading.append(node("h1", context.package.displayName), icon("Open configuration", "⚙", () => { context.navigation?.openSettings(); }, !context.navigation));
    header.append(heading, node("p", `Workspace: ${context.workspace.name}`));
    if (!context.navigation) header.append(node("small", "Update Teamarness to open Configuration from this gear. Use the Shell's Package settings in the meantime."));
    article.append(header);
  }
  article.append(errorArea, content);
  context.root.append(article);
  let alive = true;
  let revision = 0;
  let status: Status | undefined;
  let subscription: PackageSubscription | undefined;
  const pending = new AbortController();
  let stopView = () => {};
  let updateView: (value: Status) => void = () => {};
  const fail = (error: unknown) => {
    if (alive && !isAbortError(error)) errorArea.textContent = describePackageError(error);
  };
  const showStatus = (value: Status) => { status = value; updateView(value); };
  const shared = controls(content, client, refreshStatus);
  const setup = setupView(group(content, "Agent setup"), client, refreshStatus);
  const mounted = context.view === "settings" ? settings(content, client, status, refreshStatus, intent => { void setup.runIntent(intent); }) : undefined;
  const guidance = node("details"); guidance.className = "long-details";
  guidance.append(node("summary", "External-agent onboarding"), node("p", "Person-selected · Person-authorized access. Supply a token separately through your client's local secret mechanism. Save these instructions without credentials."), node("pre", onboarding)); content.append(guidance);
  stopView = () => { shared.stop(); setup.stop(); mounted?.stop(); };
  updateView = value => { shared.update(value); setup.update(value); mounted?.update(value); };
  function refreshStatus() {
    const requestedRevision = ++revision;
    void client.request("external-gateway.status.get", {}, { signal: pending.signal }).then(value => {
      if (alive && revision === requestedRevision) showStatus(value);
    }).catch(fail);
  }
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
