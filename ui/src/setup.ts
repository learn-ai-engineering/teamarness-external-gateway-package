import { describePackageError, isAbortError, type PackageSubscription, type TypedPackageClient } from "@teamarness/package-ui-sdk";
import type { PackageContract, SetupStatus } from "./contract.generated";
import { action, input, node } from "./dom";
type Preview = PackageContract["requests"]["external-gateway.setup.preview"];
export type SetupIntent = Preview["request"]["intent"];
/** Observation belongs to this view; accepted apply belongs to the Backend, not this mount. */
export function setupView(root: HTMLElement, client: TypedPackageClient<PackageContract>, refreshPolicy: () => void = () => {}) {
  const section = node("section"); section.setAttribute("aria-label", "Managed setup");
  const error = node("p"); error.setAttribute("role", "alert");
  const progress = node("section", "Loading setup status…"); progress.setAttribute("aria-live", "polite");
  const destination = node("section"); const previewArea = node("section"); const controls = node("fieldset");
  let alive = true; let busy = false; let picking = false; let pickerRevision = 0; let statusRevision = 0; let previewRevision = 0;
  let selection: { selectionId: string; path: string } | undefined;
  let plan: Preview["response"] | undefined;
  let picker: PackageSubscription | undefined; let watch: PackageSubscription | undefined;
  const pending = new AbortController();
  const fail = (cause: unknown) => { if (alive && !isAbortError(cause)) error.textContent = `${describePackageError(cause)}. Get a new preview after resolving the problem. For modified or unrelated outputs, reselect a folder or resolve the collision outside setup; files are never blindly overwritten.`; };
  const discardPreview = () => { previewRevision++; plan = undefined; previewArea.replaceChildren(); };
  const closePicker = () => { pickerRevision++; picking = false; selection = undefined; const stream = picker; picker = undefined; void stream?.close().catch(() => undefined); };
  const showDestination = () => {
    destination.replaceChildren(node("p", picking ? "Choosing folder… No browsing time limit." : selection ? `Selected folder: ${selection.path}` : "Optional skill: choose a folder supported by your external agent. No previous folders are remembered."), action("Choose skill folder", () => { void choose(); }, busy || picking));
    if (picking || selection) destination.append(action("Cancel folder selection", () => { closePicker(); discardPreview(); showDestination(); }, busy));
  };
  const choose = async () => {
    if (!alive || busy || picking) return;
    closePicker(); discardPreview(); error.textContent = ""; picking = true; showDestination();
    const revision = pickerRevision;
    try {
      const stream = await client.subscribe("external-gateway.setup.choose-skill-folder", {}, value => {
        if (!alive || revision !== pickerRevision) return;
        picking = false;
        if (value.outcome === "selected") selection = { selectionId: value.selectionId, path: value.path };
        else { closePicker(); error.textContent = "Folder selection cancelled."; }
        showDestination();
      }, { onTerminated: cause => {
        if (!alive || revision !== pickerRevision) return;
        closePicker(); discardPreview(); showDestination(); if (cause) fail(cause); else error.textContent = "Folder selection ended. Choose again.";
      } });
      // SDK establishment is prompt, but cancel/unmount can still win its response race.
      if (!alive || revision !== pickerRevision) void stream.close().catch(() => undefined);
      else picker = stream;
    } catch (cause) { if (alive && revision === pickerRevision) { closePicker(); showDestination(); fail(cause); } }
  };
  const showStatus = (value: SetupStatus) => {
    progress.replaceChildren(node("h3", `Setup: ${value.phase}`), node("p", `Operation: ${value.operationId ?? "none"}`), node("p", `Command: ${value.command}`), node("p", `Skill: ${value.skill}`), node("p", `Access: ${value.access}`));
    if (value.commandPath) progress.append(node("p", `Command path: ${value.commandPath}`));
    if (value.skillPath) progress.append(node("p", `Skill path: ${value.skillPath}`));
    if (value.error) progress.append(node("p", `${value.error}. Required output failure does not roll back completed outputs. Reselect a safe folder or resolve the collision outside setup, then preview repair.`));
    refreshPolicy();
  };
  const previewIntent = async (intent: SetupIntent) => {
    if (!alive || busy || picking) return;
    busy = true; controls.disabled = true; error.textContent = ""; discardPreview(); showDestination();
    const revision = previewRevision;
    try {
      const selected = intent !== "disconnect" && selection ? { selectionId: selection.selectionId } : {};
      const value = await client.request("external-gateway.setup.preview", { intent, ...selected }, { signal: pending.signal });
      if (!alive || revision !== previewRevision) return;
      plan = value;
      previewArea.replaceChildren(node("h3", `Preview ${value.intent}`), node("p", `Command path: ${value.commandPath}`), node("p", `Skill path: ${value.skillPath ?? "not requested"}`), node("p", `Skill change: ${value.skillChange}`));
      if (value.commandChange) {
        const change = value.commandChange;
        previewArea.append(node("p", `Command change: ${change.disposition}`));
        for (const [label, version] of [["Installed", change.installed], ["Proposed", change.proposed]] as const) {
          if (version) previewArea.append(node("p", `${label}: ${version.installation} · ${version.version} · ${version.sha256}`));
        }
      }
      previewArea.append(node("p", value.credentialChange ? "This changes managed credentials. Rotation denies the old generation; access can be interrupted until publication is repaired. No tool permissions or listener enablement are added." : value.intent === "disconnect" ? "Disconnect revokes access before removing its matching local record; shared command and skill files remain." : "No managed credential change is proposed. Only recognized outputs can be published; existing access is retained."));
      const consent = input("Approve setup changes", "checkbox");
      const apply = action("Apply setup", () => { if (consent.checked) void applyPlan(); }, true);
      consent.addEventListener("change", () => { apply.disabled = !consent.checked; });
      previewArea.append(node("label", "I approve these exact output and access changes"), consent, apply, action("Cancel setup preview", () => { discardPreview(); }));
    } catch (cause) { fail(cause); }
    finally { busy = false; if (alive) { controls.disabled = false; showDestination(); } }
  };
  const applyPlan = async () => {
    if (!alive || busy || !plan) return;
    busy = true; controls.disabled = true; previewArea.replaceChildren(node("p", "Submitting setup… Accepted work continues if this view closes.")); showDestination();
    try {
      const accepted = await client.request("external-gateway.setup.apply", { planId: plan.planId }, { signal: pending.signal });
      if (!alive) return;
      discardPreview(); closePicker(); showDestination();
      previewArea.append(node("p", `Accepted operation: ${accepted.operationId}. Follow the separate command, skill and access results below.`));
      refreshPolicy();
    } catch (cause) { if (alive) { discardPreview(); fail(cause); } }
    finally { busy = false; if (alive) { controls.disabled = false; showDestination(); } }
  };
  controls.append(...(["connect", "repair", "rotate", "disconnect", "install-skill"] as const).map(intent => action(`Preview ${intent}`, () => { void previewIntent(intent); })));
  section.append(node("h2", "Managed command and access setup"), node("p", "Preview first, then explicitly approve. Enable the listener in Settings before connecting; select exact tools there separately. Setup never enables the listener or selects tools. Accepted setup continues independently of this view. Manual tokens remain available in Settings."), destination, controls, previewArea, progress, error); root.append(section); showDestination();
  const revision = statusRevision;
  void client.request("external-gateway.setup.status", {}, { signal: pending.signal }).then(value => { if (alive && revision === statusRevision) showStatus(value); }).catch(fail);
  void client.subscribe("external-gateway.setup.watch", {}, value => { if (alive) { statusRevision++; showStatus(value); } }, { onTerminated: cause => { if (alive) { if (cause) fail(cause); else error.textContent = "Setup updates ended. Reopen the view to observe the current operation."; } } }).then(stream => { if (alive) watch = stream; else void stream.close().catch(() => undefined); }).catch(fail);
  return { previewIntent, stop: () => { if (!alive) return; alive = false; pending.abort(); closePicker(); discardPreview(); void watch?.close().catch(() => undefined); } };
}
