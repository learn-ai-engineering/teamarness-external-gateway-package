import { describePackageError, isAbortError, isPackageRequestError, packageErrorCode, type PackageSubscription, type TypedPackageClient } from "@teamarness/package-ui-sdk";
import type { PackageContract, SetupInspection, SetupStatus, Status } from "./contract.generated";
import { action, node, notification } from "./dom";
type Preview = PackageContract["requests"]["external-gateway.setup.preview"];
export type SetupIntent = Preview["request"]["intent"];

/** Observation belongs to this view; accepted apply belongs to the Backend, not this mount. */
export function setupView(root: HTMLElement, client: TypedPackageClient<PackageContract>, refreshPolicy: () => void = () => {}) {
  const section = node("section"); section.setAttribute("aria-label", "Managed setup");
  const choices = node("fieldset");
  const activity = node("p"); activity.setAttribute("aria-live", "polite");
  const confirmation = node("section"); const destination = node("section");
  const error = node("p"); error.setAttribute("role", "alert");
  const progress = notification("Setup notification");
  section.append(choices, destination, confirmation, activity, progress.element, error); root.append(section);
  const pending = new AbortController();
  let alive = true; let busy = false; let observing = false; let submitting = false;
  let submissionStatusRevision = 0;
  let facts: SetupInspection | undefined; let inspectionFailed = false; let policy: Status | undefined;
  let policyKey: string | undefined; let statusKey: string | undefined; let statusRevision = 0;
  let inspectRevision = 0; let inspectedRevision = -1; let inspection: Promise<void> | undefined;
  let actionRevision = 0; let pickerRevision = 0; let picking = false;
  let selection: { selectionId: string; path: string } | undefined;
  let picker: PackageSubscription | undefined; let watch: PackageSubscription | undefined;
  let confirmPlan: Preview["response"] | undefined;

  const issueText = (code: string | null, output: "command" | "skill" = "command") => {
    switch (code) {
      case "external_gateway.reload_required": return "Reload the Package before continuing; the available CLI source changed.";
      case "external_gateway.output_collision":
      case "external_gateway.output_unsafe":
      case "external_gateway.output_state_invalid": return output === "skill" ? "The selected skill output is unrecognized, modified or unsafe. Choose another folder or resolve that output outside setup; it cannot be replaced here." : "The CLI target is unrecognized, modified or unsafe. Inspect the command target under Command path and resolve the collision outside setup; it cannot be replaced here.";
      case "external_gateway.connection_origin_conflict": return "This Workspace has local credentials from another app profile. Disconnect using that profile first; there is no automatic transfer.";
      case "external_gateway.client_store_invalid":
      case "external_gateway.setup_state_invalid": return "Local connection information is unavailable or invalid. It has not been changed. Resolve the private client state before trying again.";
      case "external_gateway.credential_consent_required": return "Credentials are missing or no longer match. Use Rotate credentials to explicitly replace them.";
      case "external_gateway.publication_required": return "Repair the saved connection endpoint; intact credentials will be retained.";
      case "external_gateway.storage_failed": return "Storage failed. Access is closed. Restore storage, then use Configuration to revoke a token or Disable and revoke all tokens again.";
      case "external_gateway.output_storage_failed": return "The output could not be saved completely. Check storage and current installation status before trying again; partial files may remain.";
      case "external_gateway.setup_preview_stale": return "Setup changed before acceptance. Check the current status and choose the action again; nothing is retried automatically.";
      case "external_gateway.listener_required": return "Start the gateway listener on its saved port before connecting, rotating or repairing credentials.";
      case "external_gateway.output_consent_required": return "Install or update CLI first, then choose this action again.";
      case "external_gateway.output_repair_required": return "CLI publication is unfinished. Use Update CLI to repair recognized outputs.";
      case "external_gateway.setup_repair_required": return "There is unfinished setup work. Use Repair to continue it before starting another action.";
      case "external_gateway.installer_busy":
      case "external_gateway.setup_busy": return "Another setup action is in progress. Wait for it to finish, then check the current status.";
      case "external_gateway.picker_busy": return "A folder chooser is already open. Finish or cancel it before choosing again.";
      case "external_gateway.picker_failed": return "Folder selection failed. Choose the skill folder again.";
      case "external_gateway.managed_setup_required": return "Managed credentials changed. Check the current connection before choosing another action.";
      case "external_gateway.unavailable": return "Setup is unavailable. Check the Package and connection status, then try again.";
      default: return null;
    }
  };
  const fail = (cause: unknown) => {
    if (!alive || isAbortError(cause)) return;
    const output = isPackageRequestError(cause) && cause.packageError?.details?.output === "skill" ? "skill" : "command";
    error.textContent = issueText(describePackageError(cause), output) ?? "Setup could not finish. Check the Package and current setup status before choosing the action again.";
  };
  const cliReady = () => facts?.cli.state === "installed" && facts.cli.change === "none" && facts.cli.issue === null;
  const blocked = () => busy || observing || picking || !!confirmPlan;
  const usable = () => !!facts && facts.connection.state !== "conflict" && facts.connection.state !== "unavailable" && facts.recovery.state !== "invalid" && !policy?.storageFailed;
  const canRun = (intent: SetupIntent) => {
    if (!facts) return false;
    const { connection, recovery, cli } = facts;
    if (intent === "install-cli") return recovery.state !== "pending" && cli.state !== "blocked" && cli.state !== "unavailable" && (!cli.issue || cli.issue === "external_gateway.output_repair_required");
    if (!usable()) return false;
    if (intent === "repair") return recovery.state === "pending" || ["publication", "resume", "cleanup"].includes(connection.repair);
    if (recovery.state === "pending") return false;
    if (intent === "install-skill") return cliReady();
    if (intent === "connect") return connection.state === "not-connected" && connection.authority !== "current" && cliReady();
    if (intent === "rotate") return connection.authority === "current" && cliReady();
    return connection.authority === "current";
  };
  function render() {
    if (!alive) return;
    choices.replaceChildren(); destination.replaceChildren();
    const cli = node("section"); const connection = node("section"); const skill = node("section");
    cli.className = connection.className = skill.className = "setup-part";
    const cliLabel = !facts ? inspectionFailed ? "Status unavailable" : "Checking…" : facts.cli.state === "missing" ? "Not installed" : facts.cli.state === "installed" ? "Installed" : facts.cli.state === "blocked" ? "Needs attention" : "Status unavailable";
    cli.append(node("h3", `CLI: ${cliLabel}`));
    if (facts) {
      if (facts.cli.version) cli.append(node("small", `Version ${facts.cli.version}${facts.cli.change === "refresh" ? " · update available" : ""}`));
      const path = node("details"); path.append(node("summary", "Command path"), node("code", facts.cli.path)); cli.append(path);
      if (facts.cli.issue) cli.append(node("p", facts.recovery.state === "pending" && facts.cli.issue === "external_gateway.output_repair_required" ? "The required CLI output belongs to unfinished setup. Use Repair below to resume it." : issueText(facts.cli.issue) ?? "CLI status is unavailable. Check installation before continuing."));
    }
    const cliButton = action(facts?.cli.state === "missing" ? "Install CLI" : "Update CLI", () => { void runIntent("install-cli"); }, blocked() || !canRun("install-cli")); cliButton.className = "setup-button";
    if (!facts || facts.cli.change !== "none" || facts.cli.state !== "installed") cli.append(cliButton);
    let connectionLabel = inspectionFailed ? "Status unavailable" : "Checking…";
    if (facts) {
      const c = facts.connection;
      connectionLabel = c.state === "connected" ? "Connected" : c.state === "needs-repair" ? "Needs repair" : c.state === "conflict" ? "Profile conflict" : c.state === "unavailable" ? "Status unavailable" : c.authority === "revoked" ? "Revoked — not connected" : c.authority === "disconnected" ? "Disconnected" : "Not connected";
      connection.append(node("h3", `Workspace connection: ${connectionLabel}`));
      if (c.issue) connection.append(node("p", issueText(c.issue) ?? "Connection status is unavailable. Check setup before continuing."));
      if (policy && !policy.running) connection.append(node("p", c.state === "connected" ? "Listener is stopped. Saved credentials remain connected; start the listener to use them." : "Listener is stopped. Start it before connecting, rotating or repairing credentials; CLI and skill installation do not start it."));
      if (facts.recovery.state === "pending") connection.append(node("p", "There is unfinished setup work. Repair resumes the previously accepted changes, including any credential changes and selected skill output."));
      if (facts.recovery.state === "invalid") connection.append(node("p", "Saved setup recovery is unavailable or invalid. Existing data has not been reset."));
    } else connection.append(node("h3", `Workspace connection: ${connectionLabel}`));
    const c = facts?.connection;
    const add = (label: string, intent: SetupIntent) => { const button = action(label, () => { void runIntent(intent); }, blocked() || !canRun(intent)); button.className = "setup-button"; connection.append(button); };
    if (!facts || c?.state === "not-connected" && facts.recovery.state === "none" || c?.state === "unavailable" && c.authority !== "current") add("Connect", "connect");
    if (c?.authority === "current" && c.state !== "conflict" && facts?.recovery.state === "none") {
      connection.append(node("small", "Rotating credentials stops old access. Disconnect revokes access and removes this local connection; shared CLI and skill files remain."));
      add("Rotate credentials", "rotate"); add("Disconnect", "disconnect");
    }
    if (facts && (facts.recovery.state === "pending" || ["publication", "resume", "cleanup"].includes(facts.connection.repair))) add("Repair", "repair");
    skill.append(node("h3", "Agent skill (optional)"), node("p", "The skill helps your agent discover and use live Workspace tools through the CLI. Install it in a folder your agent reads; no credentials are included."));
    const install = action("Install skill…", () => { void choose(); }, blocked() || !canRun("install-skill")); install.className = "setup-button"; skill.append(install);
    if (facts && !cliReady()) skill.append(node("small", facts.recovery.state === "pending" ? "Finish unfinished setup with Repair before installing another skill." : "Install or update CLI first. No CLI or credential changes are included in skill installation."));
    choices.append(cli, connection, skill);
    if (picking || selection) {
      destination.append(node("p", selection ? `Installing skill in ${selection.path}…` : "Choose a folder in the native dialog. Cancelling installs nothing."));
      if (!submitting) destination.append(action("Cancel folder selection", () => { actionRevision++; closePicker(); busy = false; activity.textContent = ""; render(); }));
    }
    choices.disabled = busy || observing;
  }
  function closePicker() {
    pickerRevision++; picking = false; selection = undefined;
    const stream = picker; picker = undefined; void stream?.close().catch(() => undefined);
  }
  function clearConfirmation() { confirmPlan = undefined; confirmation.replaceChildren(); }
  function invalidate() {
    inspectRevision++;
    facts = undefined; inspectionFailed = false;
    clearConfirmation(); render();
    void inspect();
  }
  // One coalesced chain, never a filesystem poll or one hash per progress event.
  function inspect(): Promise<void> {
    if (inspection) return inspection;
    inspection = Promise.resolve().then(async () => {
      while (alive && inspectedRevision !== inspectRevision) {
        const revision = inspectRevision;
        try {
          const value = await client.request("external-gateway.setup.inspect", {}, { signal: pending.signal });
          if (!alive) return;
          if (revision !== inspectRevision) continue;
          facts = value; inspectionFailed = false;
        } catch (cause) {
          if (!alive) return;
          if (revision !== inspectRevision) continue;
          facts = undefined; inspectionFailed = true; fail(cause);
          error.textContent ||= "Setup status is unavailable. Reopen this view to check again.";
        }
        inspectedRevision = revision; render();
      }
    }).finally(() => {
      inspection = undefined;
      if (alive && inspectedRevision !== inspectRevision) void inspect();
    });
    return inspection;
  }
  async function inspectUntilCurrent() {
    do { await inspect(); } while (alive && inspectedRevision !== inspectRevision);
  }
  async function applyPlan(plan: Preview["response"], revision: number) {
    if (!alive || revision !== actionRevision) return;
    submitting = true; clearConfirmation(); render();
    const observed = statusRevision; submissionStatusRevision = observed;
    try {
      await client.request("external-gateway.setup.apply", { planId: plan.planId }, { signal: pending.signal });
      if (!alive || revision !== actionRevision) return;
      closePicker();
      if (observed === statusRevision) {
        statusRevision++; observing = true;
        progress.show("accepted", [node("p", "Accepted. Waiting for completion; closing this view does not cancel accepted work.")], false);
      }
      refreshPolicy(); invalidate();
    } catch (cause) {
      if (alive && revision === actionRevision) {
        closePicker();
        if (observed === statusRevision) {
          if (!packageErrorCode(cause) && !isAbortError(cause)) {
            statusRevision++; observing = true;
            progress.show("acceptance-unknown", [node("p", "Acceptance could not be confirmed. Accepted work may still continue. Wait for setup updates or reopen this view before starting another action.")], false);
          } else fail(cause);
        }
        invalidate();
      }
    }
    finally { if (alive && revision === actionRevision) { busy = false; submitting = false; render(); } }
  }
  async function runIntent(intent: SetupIntent, selectedRevision?: number) {
    if (!alive || busy || observing || confirmPlan || (picking && intent !== "install-skill")) return;
    if (!canRun(intent) && !(intent === "install-skill" && selection)) return;
    const selected = intent === "install-skill" ? selection : undefined;
    if (intent === "install-skill" && (!selected || selectedRevision !== pickerRevision)) return;
    busy = true; error.textContent = ""; const revision = ++actionRevision;
    activity.textContent = { connect: "Connecting…", rotate: "Rotating credentials…", disconnect: "Disconnecting…", repair: "Preparing connection repair…", "install-cli": "Preparing CLI installation…", "install-skill": "Preparing skill installation…" }[intent];
    render();
    try {
      invalidate(); await inspectUntilCurrent();
      if (!alive || revision !== actionRevision) return;
      if (!canRun(intent)) { error.textContent = "Setup status changed. Check the current facts before choosing an action again."; return; }
      const observed = inspectRevision;
      const value = await client.request("external-gateway.setup.preview", { intent, ...(selected ? { selectionId: selected.selectionId } : {}) }, { signal: pending.signal });
      if (!alive || revision !== actionRevision) return;
      if (observed !== inspectRevision) { error.textContent = "Setup changed before acceptance. Check current status and choose the action again."; return; }
      if (value.intent !== intent) { error.textContent = "Setup returned a different action. Nothing was submitted."; return; }
      if (["install-cli", "install-skill"].includes(intent) && value.credentialChange) { error.textContent = "This action unexpectedly includes credentials. Nothing was submitted."; return; }
      const pendingCommandRepair = intent === "repair" && facts?.recovery.state === "pending" && value.commandChange !== null && value.commandChange.disposition !== "unchanged";
      if (pendingCommandRepair && (facts?.cli.state === "blocked" || facts?.cli.state === "unavailable" || facts?.cli.issue && facts.cli.issue !== "external_gateway.output_repair_required")) {
        error.textContent = issueText(facts?.cli.issue ?? null) ?? "The required CLI output cannot be safely repaired here. Resolve its current status before continuing."; return;
      }
      if ((["connect", "rotate", "install-skill"].includes(intent) || intent === "repair" && value.commandChange !== null && !pendingCommandRepair) && (!cliReady() || value.commandChange?.disposition !== "unchanged")) {
        error.textContent = "Install or update CLI first, then choose this action again. No command or credential changes were submitted."; return;
      }
      if (intent !== "install-skill" && value.skillChange !== "not-requested") {
        // Resuming an explicitly chosen unfinished skill is allowed only by Repair.
        if (intent !== "repair") { error.textContent = "This action unexpectedly includes a skill. Nothing was submitted."; return; }
      }
      if (value.commandChange && (pendingCommandRepair || intent === "install-cli" && value.commandChange.disposition === "explicit-repair")) {
        confirmPlan = value;
        const { installed, proposed } = value.commandChange;
        const explanation = !installed ? `Publish the required CLI version ${proposed.version}.` : installed.installation !== proposed.installation ? `Transfer management of the recognized CLI from another app profile to this one, replacing version ${installed.version} with version ${proposed.version}.` : installed.version !== proposed.version ? `Replace the recognized CLI version ${installed.version} with version ${proposed.version}.` : installed.sha256 !== proposed.sha256 ? `Replace the recognized CLI with a different build of version ${proposed.version}.` : `Finish the recognized CLI publication for version ${proposed.version}.`;
        const consequence = pendingCommandRepair ? value.credentialChange ? "Resume the previously accepted credential changes and any selected outputs." : value.skillChange !== "not-requested" ? "Resume the previously accepted skill output; credentials are unchanged." : "Resume the previously accepted outputs; credentials are unchanged." : "Credentials and skills are unchanged.";
        confirmation.replaceChildren(node("h3", pendingCommandRepair ? "Resume unfinished setup?" : "Update recognized CLI?"), node("p", `${explanation} ${consequence}`), action(pendingCommandRepair ? "Cancel repair" : "Cancel CLI update", () => { actionRevision++; clearConfirmation(); render(); }), action(pendingCommandRepair ? "Confirm repair" : "Confirm CLI update", () => {
          if (!confirmPlan || busy || observing) return;
          const exact = confirmPlan; busy = true; void applyPlan(exact, revision);
        }));
        return;
      }
      await applyPlan(value, revision);
    } catch (cause) { if (alive && revision === actionRevision) fail(cause); }
    finally {
      if (alive && revision === actionRevision) {
        busy = false; activity.textContent = "";
        if (intent === "install-skill" && !submitting) closePicker();
        render();
      }
    }
  }
  async function choose() {
    if (!alive || blocked() || !canRun("install-skill")) return;
    closePicker(); error.textContent = ""; picking = true; render();
    const revision = pickerRevision;
    const installSelected = () => {
      if (!alive || revision !== pickerRevision || !picker || !selection) return;
      picking = false;
      if (busy || observing || confirmPlan) { closePicker(); error.textContent = "Another setup action started. Wait for it to finish, then choose the skill folder again."; render(); return; }
      void runIntent("install-skill", revision);
    };
    try {
      const stream = await client.subscribe("external-gateway.setup.choose-skill-folder", {}, value => {
        if (!alive || revision !== pickerRevision || selection) return;
        if (value.outcome === "selected") { selection = { selectionId: value.selectionId, path: value.path }; installSelected(); }
        else { closePicker(); render(); }
      }, { onTerminated: cause => {
        if (!alive || revision !== pickerRevision) return;
        actionRevision++; closePicker(); busy = false; activity.textContent = "";
        const wasSubmitting = submitting;
        const awaitingAcceptance = wasSubmitting && submissionStatusRevision === statusRevision;
        if (awaitingAcceptance) observing = true;
        submitting = false; render();
        if (awaitingAcceptance) error.textContent = "Folder selection ended while acceptance was pending. Completion is unknown; check setup updates or reopen this view before starting another action.";
        else if (!wasSubmitting) { if (cause) fail(cause); else error.textContent = "Folder selection ended. Choose again."; }
      } });
      if (!alive || revision !== pickerRevision) void stream.close().catch(() => undefined);
      else { picker = stream; installSelected(); }
    } catch (cause) { if (alive && revision === pickerRevision) { closePicker(); render(); fail(cause); } }
  }
  function showStatus(value: SetupStatus) {
    observing = ["accepted", "prepared", "outputs", "committed"].includes(value.phase);
    const key = JSON.stringify(value);
    if (value.phase !== "idle") {
      const text = observing ? "Setup is in progress. Accepted work continues if this view closes." : value.phase === "complete" ? value.intent === "install-cli" ? "CLI installation completed." : value.intent === "install-skill" ? "Skill installation completed." : value.intent === "disconnect" ? "Disconnection completed." : value.intent === "rotate" ? "Credential rotation completed." : value.intent === "repair" ? "Connection repair completed." : "Connection completed." : value.phase === "cleanup-pending" ? "Access was revoked; local cleanup still needs Repair." : "Setup did not finish. Check the current facts and repair options; completed outputs may remain.";
      progress.show(key, [node("p", text), ...(value.error ? [node("p", issueText(value.error, value.command === "installed" && value.skill === "pending" ? "skill" : "command") ?? "Setup needs attention. Check current status before trying again.")] : [])], value.phase === "complete" && !value.error);
    }
    const terminal = !observing && value.phase !== "idle";
    if (terminal && statusKey !== key) { invalidate(); refreshPolicy(); }
    statusKey = key; render();
  }
  render(); void inspect();
  const revision = statusRevision;
  void client.request("external-gateway.setup.status", {}, { signal: pending.signal }).then(value => { if (alive && revision === statusRevision) showStatus(value); }).catch(fail);
  void client.subscribe("external-gateway.setup.watch", {}, value => { if (alive) { statusRevision++; showStatus(value); } }, { onTerminated: cause => {
    if (!alive) return;
    if (cause) fail(cause); else error.textContent = "Setup updates ended. Reopen the view to check accepted work before trying again.";
  } }).then(stream => { if (alive) watch = stream; else void stream.close().catch(() => undefined); }).catch(fail);
  return {
    runIntent,
    update: (value: Status) => {
      if (!alive) return;
      policy = value;
      const key = JSON.stringify([value.port, value.storageFailed, value.tokens.map(t => [t.id, t.generation, !!t.managed])]);
      if (policyKey !== key) { policyKey = key; invalidate(); }
      render();
    },
    stop: () => { if (!alive) return; alive = false; actionRevision++; pending.abort(); closePicker(); clearConfirmation(); void watch?.close().catch(() => undefined); facts = undefined; policy = undefined; },
  };
}
