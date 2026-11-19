import { createFakePackageClient, createPackageRequestError } from "@teamarness/package-ui-sdk/testing";
import { typedPackageClient } from "@teamarness/package-ui-sdk";
import { describe, expect, it } from "vitest";
import type { PackageContract, SetupInspection, SetupStatus, Status } from "./contract.generated";
import { setupView } from "./setup";
import { Element, installDom, button, field, flush, deferred } from "./test-dom";
installDom();
const idle: SetupStatus = { operationId: null, intent: null, phase: "idle", command: "unchanged", skill: "not-requested", access: "unchanged", error: null, commandPath: null, skillPath: null };
const policy: Status = { desiredEnabled: false, tokens: [], running: false, port: null, storageFailed: false, warning: null, selectedContracts: 0, selectedTools: 0 };
const facts: SetupInspection = { cli: { state: "installed", path: "/private/fixture/.local/bin/tmx", version: "1.1.0", change: "none", issue: null }, connection: { state: "not-connected", authority: "none", repair: "none", issue: null }, recovery: { state: "none", intent: null } };
const connected: SetupInspection = { ...facts, connection: { state: "connected", authority: "current", repair: "none", issue: null } };
const version = { installation: "opaque-owner", version: "1.1.0", sha256: "b".repeat(64) };
const plan = { planId: "opaque-plan", intent: "connect", commandPath: facts.cli.path, skillPath: null, credentialChange: true, commandChange: { disposition: "unchanged", installed: version, proposed: version }, skillChange: "not-requested" };
function fake(requests = {}) { return createFakePackageClient({ requests: { "external-gateway.setup.inspect": () => facts, "external-gateway.setup.status": () => idle, "external-gateway.setup.preview": p => { const { intent } = p as { intent: string }; return { ...plan, intent, credentialChange: ["connect", "rotate"].includes(intent), ...(intent === "install-skill" ? { skillChange: "publish", skillPath: "/chosen/teamarness" } : {}) }; }, "external-gateway.setup.apply": () => ({ operationId: "opaque-operation" }), ...requests }, subscriptions: { "external-gateway.setup.watch": () => undefined, "external-gateway.setup.choose-skill-folder": () => undefined } }); }
function start(client: ReturnType<typeof fake>) {
  const root = new Element("section"); const view = setupView(root as unknown as HTMLElement, typedPackageClient<PackageContract>(client.client)); view.update(policy);
  return { root, view };
}
const calls = (client: ReturnType<typeof fake>, operation: string) => client.calls.filter(c => c.operation === `external-gateway.setup.${operation}`);
const complete: SetupStatus = { ...idle, phase: "complete", intent: "connect", operationId: "opaque-operation", command: "installed", access: "configured" };
describe("Direct managed setup", () => {
  it("connects directly with no selection, visible protocol ceremony or opaque metadata", async () => {
    const client = fake(); const { root, view } = start(client); await flush();
    expect(root.textContent).toContain("CLI: Installed"); expect(root.textContent).toContain("Workspace connection: Not connected");
    button(root, "Connect").click(); await flush();
    expect(calls(client, "preview")).toEqual([{ operation: "external-gateway.setup.preview", payload: { intent: "connect" } }]);
    expect(calls(client, "apply")).toEqual([{ operation: "external-gateway.setup.apply", payload: { planId: "opaque-plan" } }]);
    for (const text of ["Preview", "Apply", "Approve", "opaque-plan", "opaque-operation", "opaque-owner", "b".repeat(64)]) expect(root.textContent).not.toContain(text);
    expect(root.all().some(e => e.type === "checkbox")).toBe(false); expect(root.textContent).toContain("Accepted"); expect(root.textContent).not.toContain("Connected"); view.stop();
  });
  it("installs CLI independently and updates recognized builds with one click", async () => {
    const client = fake({ "external-gateway.setup.inspect": () => ({ ...facts, cli: { ...facts.cli, state: "missing", version: null, change: "install" } }) }); const first = start(client); await flush(); button(first.root, "Install CLI").click(); await flush();
    expect(calls(client, "preview")[0].payload).toEqual({ intent: "install-cli" }); expect(calls(client, "apply")).toHaveLength(1); first.view.stop();
    const repair = fake({ "external-gateway.setup.inspect": () => ({ ...connected, cli: { ...facts.cli, change: "explicit-repair" } }), "external-gateway.setup.preview": () => ({ ...plan, intent: "install-cli", credentialChange: false, commandChange: { disposition: "explicit-repair", installed: { ...version, version: "1.2.0" }, proposed: version } }) }); const { root, view } = start(repair); await flush();
    button(root, "Update CLI").click(); await flush();
    expect(calls(repair, "apply")).toEqual([{ operation: "external-gateway.setup.apply", payload: { planId: "opaque-plan" } }]);
    expect(root.textContent).not.toContain("Confirm CLI update"); expect(root.textContent).not.toContain("Cancel CLI update"); expect(root.textContent).not.toContain("opaque-owner"); view.stop();
  });
  it.each(["connect", "rotate", "install-skill"] as const)("refuses hidden command publication for %s", async intent => {
    const client = fake({ "external-gateway.setup.inspect": () => intent === "rotate" ? connected : facts, "external-gateway.setup.preview": () => ({ ...plan, intent, credentialChange: intent !== "install-skill", commandChange: { ...plan.commandChange, disposition: "refresh" } }) }); const { root, view } = start(client); await flush();
    if (intent === "install-skill") { button(root, "Install skill…").click(); await flush(); await client.emit("external-gateway.setup.choose-skill-folder", { outcome: "selected", selectionId: "selection", path: "/chosen" }); }
    else button(root, intent === "connect" ? "Connect" : "Rotate credentials").click(); await flush();
    expect(calls(client, "apply")).toHaveLength(0); expect(root.textContent).toContain("Install or update CLI first"); view.stop();
  });
  it("requires a current issue-free CLI before any chooser or credential action", async () => {
    const client = fake({ "external-gateway.setup.inspect": () => ({ ...facts, cli: { ...facts.cli, issue: "external_gateway.reload_required" } }) }); const { root, view } = start(client); await flush();
    button(root, "Connect").click(); button(root, "Install skill…").click(); await flush(); expect(calls(client, "preview")).toHaveLength(0); expect(calls(client, "choose-skill-folder")).toHaveLength(0); expect(root.textContent).toContain("Reload"); view.stop();
  });
  it("keeps persistent current connection facts independent of listener and historical CLI/skill results", async () => {
    const client = fake({ "external-gateway.setup.inspect": () => connected }); const { root, view } = start(client); await flush();
    expect(root.textContent).toContain("Workspace connection: Connected"); expect(root.textContent).toContain("Listener is stopped");
    await client.emit("external-gateway.setup.watch", { ...complete, intent: "install-skill", skill: "installed", access: "unchanged" }); await flush();
    expect(root.textContent).toContain("Workspace connection: Connected"); expect(root.all().some(e => e.tagName === "button" && e.textContent === "Connect")).toBe(false);
    expect(button(root, "Rotate credentials").disabled).toBe(false); expect(button(root, "Disconnect").disabled).toBe(false); view.stop();
  });
  it.each([
    [{ state: "needs-repair", authority: "current", repair: "replace-credentials", issue: "external_gateway.credential_consent_required" }, "Credentials are missing", "Rotate credentials"],
    [{ state: "needs-repair", authority: "current", repair: "publication", issue: "external_gateway.publication_required" }, "Needs repair", "Repair"],
    [{ state: "not-connected", authority: "revoked", repair: "none", issue: null }, "Revoked", "Connect"],
    [{ state: "not-connected", authority: "disconnected", repair: "none", issue: null }, "Disconnected", "Connect"],
    [{ state: "conflict", authority: "unavailable", repair: "none", issue: "external_gateway.connection_origin_conflict" }, "another app profile", null],
  ] as const)("offers contextual current actions: %s", async (connection, text, expected) => {
    const client = fake({ "external-gateway.setup.inspect": () => ({ ...facts, connection }) }); const { root, view } = start(client); await flush(); expect(root.textContent).toContain(text); if (expected) expect(button(root, expected).disabled).toBe(false); else expect(root.textContent).not.toContain("Rotate credentials"); view.stop();
  });
  it("unknown, failed, invalid and pending inspections never assert connected or offer blind connect", async () => {
    const waiting = deferred<SetupInspection>(); const client = fake({ "external-gateway.setup.inspect": () => waiting.promise }); const { root, view } = start(client); await flush();
    expect(root.textContent).toContain("Checking"); expect(button(root, "Connect").disabled).toBe(true);
    waiting.resolve({ ...facts, connection: { state: "unavailable", authority: "unavailable", repair: "none", issue: "external_gateway.client_store_invalid" }, recovery: { state: "invalid", intent: null } }); await flush(); expect(root.textContent).toContain("unavailable"); expect(button(root, "Connect").disabled).toBe(true); view.stop();
    const failed = fake({ "external-gateway.setup.inspect": () => { throw createPackageRequestError({ packageError: { code: "external_gateway.unavailable" } }); } }); const second = start(failed); await flush(); expect(second.root.textContent).toContain("unavailable"); expect(button(second.root, "Connect").disabled).toBe(true); second.view.stop();
    const pending = fake({ "external-gateway.setup.inspect": () => ({ ...facts, connection: { ...facts.connection, state: "needs-repair", repair: "resume" }, recovery: { state: "pending", intent: "connect" } }) }); const third = start(pending); await flush(); expect(third.root.textContent).toContain("unfinished"); expect(button(third.root, "Repair").disabled).toBe(false); expect(third.root.all().some(e => e.tagName === "button" && e.textContent === "Connect")).toBe(false); third.view.stop();
  });
});

describe("Skill chooser and owned action lifetime", () => {
  it("native selection immediately installs only skill and retains its lease through acceptance", async () => {
    const acceptance = deferred<{ operationId: string }>(); const client = fake({ "external-gateway.setup.apply": () => acceptance.promise }); const closed: string[] = [];
    const wrapped: ReturnType<typeof fake> = { ...client, client: { request: client.client.request, subscribe: async (operation, payload, listener, options) => { const stream = await client.client.subscribe(operation, payload, listener, options); return { close: async () => { closed.push(operation); await stream.close(); } }; } } };
    const { root, view } = start(wrapped); await flush(); expect(root.textContent).toContain("helps your agent"); button(root, "Install skill…").click(); await flush();
    await client.emit("external-gateway.setup.choose-skill-folder", { outcome: "selected", selectionId: "selected-folder", path: "/chosen" }); await flush();
    expect(calls(client, "preview")).toEqual([{ operation: "external-gateway.setup.preview", payload: { intent: "install-skill", selectionId: "selected-folder" } }]); expect(calls(client, "apply")).toHaveLength(1); expect(closed).not.toContain("external-gateway.setup.choose-skill-folder");
    acceptance.resolve({ operationId: "accepted" }); await flush(); expect(closed).toContain("external-gateway.setup.choose-skill-folder"); expect(client.calls.some(c => c.operation.includes("token."))).toBe(false); view.stop();
  });
  it("cancel, terminal failure, stale selection and unmount never submit a skill", async () => {
    const client = fake(); const { root, view } = start(client); await flush(); button(root, "Install skill…").click(); await flush(); button(root, "Cancel folder selection").click();
    await client.emit("external-gateway.setup.choose-skill-folder", { outcome: "selected", selectionId: "late", path: "/late" }); expect(calls(client, "preview")).toHaveLength(0);
    button(root, "Install skill…").click(); await flush(); await client.emit("external-gateway.setup.choose-skill-folder", { outcome: "cancelled" }); expect(calls(client, "apply")).toHaveLength(0);
    button(root, "Install skill…").click(); await flush(); client.terminate("external-gateway.setup.choose-skill-folder", null); expect(root.textContent).toContain("Folder selection ended");
    button(root, "Install skill…").click(); await flush(); view.stop(); view.stop(); const text = root.textContent; await client.emit("external-gateway.setup.choose-skill-folder", { outcome: "selected", selectionId: "late", path: "/late" }); expect(root.textContent).toBe(text); expect(calls(client, "apply")).toHaveLength(0);
  });
  it("closes late chooser establishment and invalidates a preview if its lease ends", async () => {
    const opening = deferred<void>(); const preview = deferred<typeof plan>(); const closed: string[] = []; const client = fake({ "external-gateway.setup.preview": () => preview.promise });
    const wrapped: ReturnType<typeof fake> = { ...client, client: { request: client.client.request, subscribe: async (operation, payload, listener, options) => { const stream = await client.client.subscribe(operation, payload, listener, options); if (operation === "external-gateway.setup.choose-skill-folder") await opening.promise; return { close: async () => { closed.push(operation); await stream.close(); } }; } } };
    const first = start(wrapped); await flush(); button(first.root, "Install skill…").click(); await flush(); button(first.root, "Cancel folder selection").click(); opening.resolve(); await flush(); expect(closed).toContain("external-gateway.setup.choose-skill-folder"); first.view.stop();
    const second = start(client); await flush(); button(second.root, "Install skill…").click(); await flush(); await client.emit("external-gateway.setup.choose-skill-folder", { outcome: "selected", selectionId: "lease", path: "/chosen" }); await flush(); client.terminate("external-gateway.setup.choose-skill-folder", null); preview.resolve({ ...plan, intent: "install-skill" }); await flush(); expect(calls(client, "apply")).toHaveLength(0); second.view.stop();
  });
  it("ignores late acceptance after completed result; remount observes without replay", async () => {
    const accepted = deferred<{ operationId: string }>(); const client = fake({ "external-gateway.setup.apply": () => accepted.promise }); const { root, view } = start(client); await flush(); button(root, "Connect").click(); await flush();
    await client.emit("external-gateway.setup.watch", complete); accepted.resolve({ operationId: "opaque-operation" }); await flush(); expect(root.textContent).not.toContain("Accepted"); expect(root.textContent).toContain("Connection completed");
    const dismiss = button(root, "Dismiss connection notification"); await client.emit("external-gateway.setup.watch", complete); expect(button(root, "Dismiss connection notification")).toBe(dismiss); dismiss.click(); expect(field(root, "Connection notification").hidden).toBe(true);
    await client.emit("external-gateway.setup.watch", { ...complete, phase: "repair-required", error: "external_gateway.output_collision" }); expect(field(root, "Connection notification").hidden).toBe(false); view.stop();
    const remount = fake({ "external-gateway.setup.status": () => complete, "external-gateway.setup.inspect": () => connected }); const second = start(remount); await flush(); expect(calls(remount, "apply")).toHaveLength(0); expect(second.root.textContent).toContain("Workspace connection: Connected"); second.view.stop();
  });
  it("stale plan and collision failures are actionable and target-specific, not overwrite or folder advice for CLI", async () => {
    const client = fake({ "external-gateway.setup.apply": () => { throw createPackageRequestError({ packageError: { code: "external_gateway.setup_preview_stale" } }); } }); const { root, view } = start(client); await flush(); button(root, "Connect").click(); await flush(); expect(root.textContent).toContain("changed"); expect(root.textContent).not.toContain("setup_preview_stale"); expect(calls(client, "apply")).toHaveLength(1); view.stop();
    for (const output of ["command", "skill"]) { const broken = fake({ "external-gateway.setup.preview": () => { throw createPackageRequestError({ packageError: { code: "external_gateway.output_collision", details: { output } } }); } }); const next = start(broken); await flush(); button(next.root, "Connect").click(); await flush(); expect(next.root.textContent).toContain(output === "command" ? "CLI target" : "selected skill"); expect(next.root.textContent).not.toContain("output_collision"); expect(calls(broken, "apply")).toHaveLength(0); next.view.stop(); }
  });
});

it.each([
  { ...version, sha256: "a".repeat(64) },
  { ...version, installation: "another-owner" },
])("applies a recognized same-version CLI update directly: %s", async installed => {
  const client = fake({ "external-gateway.setup.inspect": () => ({ ...connected, cli: { ...facts.cli, change: "explicit-repair" } }), "external-gateway.setup.preview": () => ({ ...plan, intent: "install-cli", credentialChange: false, commandChange: { disposition: "explicit-repair", installed, proposed: version } }) });
  const { root, view } = start(client); await flush(); button(root, "Update CLI").click(); await flush();
  expect(calls(client, "apply")).toEqual([{ operation: "external-gateway.setup.apply", payload: { planId: "opaque-plan" } }]);
  expect(root.textContent).not.toContain("Confirm CLI update"); expect(root.textContent).not.toContain("Cancel CLI update"); expect(client.calls.some(c => c.operation.includes("token."))).toBe(false); view.stop();
});

describe("Section-local setup feedback", () => {
  it.each(["install-cli", "connect", "rotate", "disconnect", "install-skill"] as const)("keeps %s progress and dismissible result above its own controls", async intent => {
    const current = intent === "install-cli" ? { ...facts, cli: { ...facts.cli, change: "explicit-repair" as const } } : intent === "rotate" || intent === "disconnect" ? connected : facts;
    const client = fake({ "external-gateway.setup.inspect": () => current }); const { root, view } = start(client); await flush();
    if (intent === "install-skill") { button(root, "Install skill…").click(); await flush(); await client.emit("external-gateway.setup.choose-skill-folder", { outcome: "selected", selectionId: "chosen", path: "/chosen" }); }
    else button(root, intent === "install-cli" ? "Update CLI" : intent === "rotate" ? "Rotate credentials" : intent === "disconnect" ? "Disconnect" : "Connect").click();
    await flush();
    const partName = intent === "install-cli" ? "CLI setup" : intent === "install-skill" ? "Agent skill setup" : "Workspace connection setup";
    const noticeName = intent === "install-cli" ? "CLI notification" : intent === "install-skill" ? "Skill notification" : "Connection notification";
    const part = () => field(root, partName);
    expect(field(root, noticeName).parent).toBe(part()); expect(field(root, noticeName).textContent).toContain("Accepted");
    const result: SetupStatus = { ...complete, intent, skill: intent === "install-skill" ? "installed" : "not-requested", access: intent === "disconnect" ? "revoked" : intent === "install-cli" || intent === "install-skill" ? "unchanged" : "configured" };
    await client.emit("external-gateway.setup.watch", result); await flush();
    const notice = field(root, noticeName); expect(notice.parent).toBe(part()); expect(notice.textContent).toContain("completed");
    const actionButton = part().all().find(e => e.tagName === "button" && !e.attributes.get("aria-label")?.startsWith("Dismiss"))!;
    expect(part().all().indexOf(notice)).toBeLessThan(part().all().indexOf(actionButton));
    button(root, `Dismiss ${noticeName.toLowerCase()}`).click(); await client.emit("external-gateway.setup.watch", result); await flush(); expect(field(root, noticeName).hidden).toBe(true); view.stop();
  });
  it("keeps CLI success in its section after the update button disappears and another action completes", async () => {
    let current: SetupInspection = { ...connected, cli: { ...facts.cli, change: "explicit-repair" } };
    const client = fake({ "external-gateway.setup.inspect": () => current }); const { root, view } = start(client); await flush(); button(root, "Update CLI").click(); await flush(); current = connected;
    await client.emit("external-gateway.setup.watch", { ...complete, intent: "install-cli", access: "unchanged" }); await flush();
    expect(field(root, "CLI setup").textContent).toContain("CLI installation completed"); expect(root.all().some(e => e.textContent === "Update CLI")).toBe(false);
    button(root, "Rotate credentials").click(); await flush(); await client.emit("external-gateway.setup.watch", { ...complete, intent: "rotate" }); await flush();
    expect(field(root, "CLI setup").textContent).toContain("CLI installation completed"); expect(field(root, "Workspace connection setup").textContent).toContain("Credential rotation completed"); view.stop();
  });
  it("keeps action errors in their originating section", async () => {
    const client = fake({ "external-gateway.setup.inspect": () => ({ ...facts, cli: { ...facts.cli, change: "explicit-repair" } }), "external-gateway.setup.preview": () => { throw createPackageRequestError({ packageError: { code: "external_gateway.output_collision", details: { output: "command" } } }); } });
    const { root, view } = start(client); await flush(); button(root, "Update CLI").click(); await flush(); expect(field(root, "CLI setup").textContent).toContain("CLI target"); expect(field(root, "Agent skill setup").textContent).not.toContain("CLI target"); view.stop();
  });
});

describe("Coalesced revision-guarded inspection", () => {
  it("coalesces Policy/event invalidations and never hashes on repeated progress", async () => {
    const initial = deferred<SetupInspection>(); let reads = 0; const client = fake({ "external-gateway.setup.inspect": () => ++reads === 1 ? initial.promise : connected }); const { root, view } = start(client); await flush();
    for (let n = 1; n < 10; n++) view.update({ ...policy, tokens: [{ id: "managed", label: "Managed", generation: n, managed: true }] });
    await client.emit("external-gateway.setup.watch", { ...idle, phase: "outputs", operationId: "running" }); for (let n = 0; n < 10; n++) await client.emit("external-gateway.setup.watch", { ...idle, phase: "outputs", operationId: "running" });
    expect(reads).toBe(1); initial.resolve(facts); await flush(); expect(reads).toBe(2); expect(root.textContent).toContain("Workspace connection: Connected");
    for (let n = 0; n < 10; n++) await client.emit("external-gateway.setup.watch", { ...idle, phase: "outputs", operationId: "running" }); expect(reads).toBe(2);
    await client.emit("external-gateway.setup.watch", complete); await flush(); expect(reads).toBe(3); view.stop();
  });
  it("aborts read on stop and ignores late initial status after a newer completion", async () => {
    const initial = deferred<SetupStatus>(); const inspect = deferred<SetupInspection>(); let signal: AbortSignal | undefined;
    const client = fake({ "external-gateway.setup.status": () => initial.promise, "external-gateway.setup.inspect": (_p: unknown, s?: AbortSignal) => { signal = s; return inspect.promise; } }); const { root, view } = start(client); await flush(); await client.emit("external-gateway.setup.watch", complete); initial.resolve(idle); await flush(); expect(root.textContent).toContain("Connection completed"); view.stop(); const text = root.textContent; inspect.resolve(facts); await flush(); expect(signal?.aborted).toBe(true); expect(root.textContent).toBe(text);
  });
});

it("immediately marks invalidated current facts as checking and rejects a preview overtaken by a Policy change", async () => {
  const held = deferred<typeof plan>(); let current = connected;
  const client = fake({ "external-gateway.setup.inspect": () => current, "external-gateway.setup.preview": () => held.promise }); const { root, view } = start(client); await flush();
  button(root, "Rotate credentials").click(); await flush(); expect(root.textContent).toContain("Rotating credentials…");
  current = { ...facts, connection: { ...facts.connection, authority: "revoked" } }; view.update({ ...policy, tokens: [] , port: 32123 });
  expect(root.textContent).not.toContain("Workspace connection: Connected"); expect(root.textContent).toContain("Checking");
  held.resolve({ ...plan, intent: "rotate" }); await flush(); expect(calls(client, "apply")).toHaveLength(0); expect(root.textContent).toContain("changed"); view.stop();
});

it.each(["connect", "rotate", "install-skill"] as const)("refuses all non-unchanged or missing command summaries for %s", async intent => {
  for (const commandChange of [null, ...["install", "explicit-repair"].map(disposition => ({ ...plan.commandChange, disposition }))]) {
    const client = fake({ "external-gateway.setup.inspect": () => intent === "rotate" ? connected : facts, "external-gateway.setup.preview": () => ({ ...plan, intent, credentialChange: intent !== "install-skill", commandChange }) }); const { root, view } = start(client); await flush();
    if (intent === "install-skill") { button(root, "Install skill…").click(); await flush(); await client.emit("external-gateway.setup.choose-skill-folder", { outcome: "selected", selectionId: "selection", path: "/chosen" }); }
    else button(root, intent === "connect" ? "Connect" : "Rotate credentials").click(); await flush(); expect(calls(client, "apply")).toHaveLength(0); expect(root.textContent).toContain("Install or update CLI first"); view.stop();
  }
});

it("rechecks non-null CLI issues before credential acceptance, without hiding the independent connection", async () => {
  let reads = 0; const client = fake({ "external-gateway.setup.inspect": () => ++reads === 1 ? connected : { ...connected, cli: { ...facts.cli, issue: "external_gateway.reload_required" } } }); const { root, view } = start(client); await flush(); button(root, "Rotate credentials").click(); await flush();
  expect(calls(client, "preview")).toHaveLength(0); expect(calls(client, "apply")).toHaveLength(0); expect(root.textContent).toContain("Reload"); expect(root.textContent).toContain("Workspace connection: Connected"); view.stop();
});

it("discards overtaken CLI preparation and surfaces stale direct apply without retry", async () => {
  const preview = deferred<typeof plan>();
  const client = fake({ "external-gateway.setup.inspect": () => ({ ...facts, cli: { ...facts.cli, change: "explicit-repair" } }), "external-gateway.setup.preview": () => preview.promise, "external-gateway.setup.apply": () => { throw createPackageRequestError({ packageError: { code: "external_gateway.setup_preview_stale" } }); } });
  const { root, view } = start(client); await flush(); button(root, "Update CLI").click(); await flush(); view.update({ ...policy, port: 32123 }); await flush();
  preview.resolve({ ...plan, intent: "install-cli", credentialChange: false, commandChange: { ...plan.commandChange, disposition: "explicit-repair" } }); await flush();
  expect(calls(client, "apply")).toHaveLength(0); expect(root.textContent).toContain("changed");
  button(root, "Update CLI").click(); await flush(); expect(calls(client, "apply")).toHaveLength(1); expect(root.textContent).toContain("changed"); expect(root.textContent).not.toContain("Confirm CLI update"); view.stop();
});

it("repairs cleanup without CLI publication and disconnects directly without a selected folder", async () => {
  const cleanup = fake({ "external-gateway.setup.inspect": () => ({ ...facts, connection: { ...facts.connection, authority: "disconnected", state: "needs-repair", repair: "cleanup" }, recovery: { state: "pending", intent: "disconnect" } }), "external-gateway.setup.preview": () => ({ ...plan, intent: "repair", commandChange: null, credentialChange: false }) }); const first = start(cleanup); await flush(); button(first.root, "Repair").click(); await flush(); expect(calls(cleanup, "apply")).toHaveLength(1); first.view.stop();
  const client = fake({ "external-gateway.setup.inspect": () => connected }); const { root, view } = start(client); await flush(); button(root, "Disconnect").click(); await flush(); expect(calls(client, "preview")[0].payload).toEqual({ intent: "disconnect" }); expect(calls(client, "apply")).toHaveLength(1); expect(root.textContent).toContain("shared CLI and skill files remain"); view.stop();
});

it("an unmounted action cannot apply a late preview or redraw a late acceptance", async () => {
  const preview = deferred<typeof plan>(); const client = fake({ "external-gateway.setup.preview": () => preview.promise }); const first = start(client); await flush(); button(first.root, "Connect").click(); await flush(); first.view.stop(); preview.resolve(plan); await flush(); expect(calls(client, "apply")).toHaveLength(0);
  const accepted = deferred<{ operationId: string }>(); const later = fake({ "external-gateway.setup.apply": () => accepted.promise }); const second = start(later); await flush(); button(second.root, "Connect").click(); await flush(); second.view.stop(); const text = second.root.textContent; accepted.resolve({ operationId: "late" }); await flush(); expect(second.root.textContent).toBe(text); expect(calls(later, "apply")).toHaveLength(1);
});

it.each(["install-cli", "install-skill"] as const)("refuses credential changes in the independent %s action", async intent => {
  const client = fake({ "external-gateway.setup.inspect": () => intent === "install-cli" ? { ...facts, cli: { ...facts.cli, state: "missing", change: "install" } } : facts, "external-gateway.setup.preview": () => ({ ...plan, intent, credentialChange: true }) }); const { root, view } = start(client); await flush();
  if (intent === "install-skill") { button(root, "Install skill…").click(); await flush(); await client.emit("external-gateway.setup.choose-skill-folder", { outcome: "selected", selectionId: "selection", path: "/chosen" }); }
  else button(root, "Install CLI").click(); await flush(); expect(calls(client, "apply")).toHaveLength(0); expect(root.textContent).toContain("unexpectedly includes credentials"); view.stop();
});

it("installs CLI despite unavailable private credentials while unknown/collision state cannot authorize replacement", async () => {
  const client = fake({ "external-gateway.setup.inspect": () => ({ ...facts, cli: { ...facts.cli, state: "missing", change: "install" }, connection: { ...facts.connection, state: "unavailable", authority: "unavailable", issue: "external_gateway.client_store_invalid" }, recovery: { state: "invalid", intent: null } }) }); const first = start(client); await flush(); button(first.root, "Install CLI").click(); await flush(); expect(calls(client, "preview")[0].payload).toEqual({ intent: "install-cli" }); expect(calls(client, "apply")).toHaveLength(1); expect(first.root.textContent).not.toContain("Uninstall"); first.view.stop();
  const blocked = fake({ "external-gateway.setup.inspect": () => ({ ...facts, cli: { ...facts.cli, state: "blocked", issue: "external_gateway.output_collision" } }) }); const second = start(blocked); await flush(); button(second.root, "Update CLI").click(); expect(calls(blocked, "preview")).toHaveLength(0); expect(second.root.textContent).toContain("CLI target"); expect(second.root.textContent).not.toContain("Confirm CLI update"); second.view.stop();
});

it("closes late status subscription establishment and never replays a skill selected during an invalidated inspection", async () => {
  const opening = deferred<void>(); const inspection = deferred<SetupInspection>(); let reads = 0; const closed: string[] = [];
  const client = fake({ "external-gateway.setup.inspect": () => ++reads === 1 ? facts : inspection.promise });
  const wrapped: ReturnType<typeof fake> = { ...client, client: { request: client.client.request, subscribe: async (operation, payload, listener, options) => { const stream = await client.client.subscribe(operation, payload, listener, options); if (operation === "external-gateway.setup.watch") await opening.promise; return { close: async () => { closed.push(operation); await stream.close(); } }; } } };
  const { root, view } = start(wrapped); await flush(); button(root, "Install skill…").click(); await flush(); view.update({ ...policy, port: 32123 });
  await client.emit("external-gateway.setup.choose-skill-folder", { outcome: "selected", selectionId: "selected", path: "/chosen" }); await flush(); expect(calls(client, "preview")).toHaveLength(0);
  inspection.resolve({ ...facts, cli: { ...facts.cli, issue: "external_gateway.reload_required" } }); await flush(); expect(calls(client, "apply")).toHaveLength(0); expect(closed).toContain("external-gateway.setup.choose-skill-folder");
  view.stop(); opening.resolve(); await flush(); expect(closed).toContain("external-gateway.setup.watch");
});

it("does not let a late initial idle observation reopen actions after acceptance", async () => {
  const initial = deferred<SetupStatus>(); const client = fake({ "external-gateway.setup.status": () => initial.promise }); const { root, view } = start(client); await flush(); button(root, "Connect").click(); await flush();
  initial.resolve(idle); await flush(); expect(root.textContent).toContain("Accepted"); expect(button(root, "Connect").disabled).toBe(true); expect(calls(client, "apply")).toHaveLength(1); view.stop();
});

it("does not re-offer an action when Host acceptance is uncertain without a correlated terminal observation", async () => {
  const client = fake({ "external-gateway.setup.apply": () => { throw createPackageRequestError({ code: "deadline_exceeded" }); } }); const { root, view } = start(client); await flush(); button(root, "Connect").click(); await flush();
  expect(root.textContent).toContain("Acceptance could not be confirmed"); expect(button(root, "Connect").disabled).toBe(true); expect(calls(client, "apply")).toHaveLength(1);
  await client.emit("external-gateway.setup.watch", complete); await flush(); expect(button(root, "Connect").disabled).toBe(true);
  await client.emit("external-gateway.setup.watch", { ...complete, phase: "accepted" });
  await client.emit("external-gateway.setup.watch", complete); await flush(); expect(root.textContent).toContain("Connection completed"); view.stop();
});

it.each(["connect", "install-skill"] as const)("resumes already accepted pending %s outputs without a separate CLI-install deadlock", async previousIntent => {
  for (const disposition of ["install", "refresh", "explicit-repair"]) {
    const client = fake({ "external-gateway.setup.inspect": () => ({ ...facts, cli: { ...facts.cli, state: "missing", version: null, change: disposition, issue: disposition === "explicit-repair" ? "external_gateway.output_repair_required" : null }, connection: { ...facts.connection, state: "needs-repair", repair: "resume" }, recovery: { state: "pending", intent: previousIntent } }), "external-gateway.setup.preview": () => ({ ...plan, intent: "repair", commandChange: { ...plan.commandChange, disposition, installed: null }, credentialChange: previousIntent === "connect", skillChange: previousIntent === "install-skill" ? "publish" : "not-requested" }) });
    const { root, view } = start(client); await flush(); expect(button(root, "Install CLI").disabled).toBe(true); button(root, "Repair").click(); await flush();
    expect(calls(client, "preview")[0].payload).toEqual({ intent: "repair" }); expect(calls(client, "apply")).toHaveLength(0); expect(root.textContent).toContain("previously accepted"); expect(root.textContent).toContain("1.1.0"); expect(root.textContent).not.toContain("opaque-owner");
    button(root, "Cancel repair").click(); expect(calls(client, "apply")).toHaveLength(0); button(root, "Repair").click(); await flush(); button(root, "Confirm repair").click(); await flush();
    expect(calls(client, "apply")).toEqual([{ operation: "external-gateway.setup.apply", payload: { planId: "opaque-plan" } }]); expect(calls(client, "preview").every(c => (c.payload as { intent: string }).intent === "repair")).toBe(true); view.stop();
  }
});

it("does not use pending repair consent to override an unrecognized CLI collision", async () => {
  const client = fake({ "external-gateway.setup.inspect": () => ({ ...facts, cli: { ...facts.cli, state: "blocked", issue: "external_gateway.output_collision" }, connection: { ...facts.connection, state: "needs-repair", repair: "resume" }, recovery: { state: "pending", intent: "connect" } }), "external-gateway.setup.preview": () => ({ ...plan, intent: "repair", commandChange: { ...plan.commandChange, disposition: "explicit-repair" } }) });
  const { root, view } = start(client); await flush(); button(root, "Repair").click(); await flush(); expect(calls(client, "apply")).toHaveLength(0); expect(root.textContent).not.toContain("Confirm repair"); expect(root.textContent).toContain("CLI target"); view.stop();
});

it("ignores a failed late skill preview after cancellation rather than replacing a newer observation", async () => {
  const release = deferred<void>(); const client = fake({ "external-gateway.setup.preview": async () => { await release.promise; throw createPackageRequestError({ packageError: { code: "external_gateway.output_collision", details: { output: "skill" } } }); } }); const { root, view } = start(client); await flush(); button(root, "Install skill…").click(); await flush(); await client.emit("external-gateway.setup.choose-skill-folder", { outcome: "selected", selectionId: "selected", path: "/chosen" }); await flush();
  button(root, "Cancel folder selection").click(); const text = root.textContent; release.resolve(); await flush(); expect(root.textContent).toBe(text); expect(calls(client, "apply")).toHaveLength(0); view.stop();
});

it("a lost skill lease after a terminal result cannot turn completed work back into uncertain acceptance", async () => {
  const acceptance = deferred<{ operationId: string }>(); const client = fake({ "external-gateway.setup.apply": () => acceptance.promise }); const { root, view } = start(client); await flush(); button(root, "Install skill…").click(); await flush(); await client.emit("external-gateway.setup.choose-skill-folder", { outcome: "selected", selectionId: "selected", path: "/chosen" }); await flush();
  await client.emit("external-gateway.setup.watch", { ...complete, intent: "install-skill", skill: "installed", access: "unchanged" }); await flush(); client.terminate("external-gateway.setup.choose-skill-folder", null); acceptance.resolve({ operationId: "opaque-operation" }); await flush();
  expect(root.textContent).toContain("Skill installation completed"); expect(root.textContent).not.toContain("Completion is unknown"); expect(button(root, "Install skill…").disabled).toBe(false); view.stop();
});

it("keeps attached setup controls and an open Command path through identical observations", async () => {
  const client = fake(); const { root, view } = start(client); await flush();
  const connect = button(root, "Connect"); const path = root.all().find(e => e.tagName === "details")!; path.open = true;
  view.update({ ...policy }); await flush();
  expect(button(root, "Connect")).toBe(connect); expect(root.all()).toContain(connect); expect(root.all()).toContain(path); expect(path.open).toBe(true);
  await client.emit("external-gateway.setup.watch", idle); await flush();
  expect(button(root, "Connect")).toBe(connect); expect(root.all()).toContain(path); expect(path.open).toBe(true);
  view.update({ ...policy, port: 32123 }); expect(button(root, "Connect").disabled).toBe(true); await flush(); expect(button(root, "Connect").disabled).toBe(false); view.stop();
});

it("does not assure access is closed when origin inspection fails while Policy remains live", async () => {
  const client = fake({ "external-gateway.setup.inspect": () => ({ ...connected, connection: { state: "unavailable", authority: "current", repair: "none", issue: "external_gateway.storage_failed" } }) });
  const { root, view } = start(client); view.update({ ...policy, running: true, desiredEnabled: true, storageFailed: false }); await flush();
  expect(root.textContent).toContain("Workspace connection: Status unavailable"); expect(root.textContent).not.toContain("Access is closed"); expect(root.textContent).toContain("Configuration");
  await client.emit("external-gateway.setup.watch", { ...complete, phase: "repair-required", error: "external_gateway.storage_failed" }); await flush(); expect(root.textContent).not.toContain("Access is closed"); view.stop();
});

it("gives reload guidance and no usable Update for recognized already-published pending receipt", async () => {
  const client = fake({ "external-gateway.setup.inspect": () => ({ ...facts, cli: { ...facts.cli, change: "explicit-repair", issue: "external_gateway.reload_required" } }) }); const { root, view } = start(client); await flush();
  expect(root.textContent).toContain("Reload"); expect(root.textContent).not.toContain("source changed"); button(root, "Update CLI").click(); await flush(); expect(calls(client, "preview")).toHaveLength(0); view.stop();
});

it.each(["accepted", "uncertain", "waiting"] as const)("idle and unrelated history cannot settle a local %s submission", async mode => {
  const accepted = deferred<{ operationId: string }>();
  const client = fake({ "external-gateway.setup.apply": () => mode === "waiting" ? accepted.promise : mode === "accepted" ? { operationId: "opaque-operation" } : Promise.reject(createPackageRequestError({ code: "deadline_exceeded" })) });
  const { root, view } = start(client); await flush();
  const old = { ...complete, operationId: "previous-operation" }; await client.emit("external-gateway.setup.watch", old); await flush();
  button(root, "Connect").click(); await flush();
  for (const value of [idle, old, { ...complete, operationId: "unconfirmed-history" }, { ...complete, intent: "install-cli", operationId: "unrelated" }]) { await client.emit("external-gateway.setup.watch", value); await flush(); }
  if (mode === "waiting") { accepted.resolve({ operationId: "opaque-operation" }); await flush(); }
  expect(button(root, "Connect").disabled).toBe(true); expect(root.textContent).toContain(mode === "uncertain" ? "Acceptance could not be confirmed" : "Accepted");
  button(root, "Connect").click(); await flush(); expect(calls(client, "apply")).toHaveLength(1);
  await client.emit("external-gateway.setup.watch", { ...complete, phase: "accepted" }); await client.emit("external-gateway.setup.watch", complete); await flush();
  expect(button(root, "Connect").disabled).toBe(false); expect(root.textContent).toContain("Connection completed"); view.stop();
});

it("idle during apply wait cannot suppress the eventual uncertain gate", async () => {
  const release = deferred<void>(); const client = fake({ "external-gateway.setup.apply": async () => { await release.promise; throw createPackageRequestError({ code: "deadline_exceeded" }); } });
  const { root, view } = start(client); await flush(); button(root, "Connect").click(); await flush(); await client.emit("external-gateway.setup.watch", idle); release.resolve(); await flush();
  expect(button(root, "Connect").disabled).toBe(true); expect(root.textContent).toContain("Acceptance could not be confirmed"); view.stop();
});

it("a lost skill lease and idle watch keep uncertain submission gated until exact late acceptance completes", async () => {
  const accepted = deferred<{ operationId: string }>(); const client = fake({ "external-gateway.setup.apply": () => accepted.promise }); const { root, view } = start(client); await flush();
  button(root, "Install skill…").click(); await flush(); await client.emit("external-gateway.setup.choose-skill-folder", { outcome: "selected", selectionId: "selected", path: "/chosen" }); await flush();
  client.terminate("external-gateway.setup.choose-skill-folder", null); await client.emit("external-gateway.setup.watch", idle); await flush();
  expect(button(root, "Connect").disabled).toBe(true); expect(root.textContent).toContain("Completion is unknown");
  await client.emit("external-gateway.setup.watch", { ...complete, intent: "install-skill", skill: "installed" }); accepted.resolve({ operationId: "opaque-operation" }); await flush();
  expect(root.textContent).toContain("Skill installation completed"); expect(root.textContent).not.toContain("Completion is unknown"); expect(button(root, "Install skill…").disabled).toBe(false); expect(calls(client, "apply")).toHaveLength(1); view.stop();
});

it.each(["connect", "repair"] as const)("correlates pending Repair completion in its original %s history without accepting stale terminals", async original => {
  const old = { ...complete, intent: original, phase: "repair-required", error: "external_gateway.output_storage_failed" };
  const client = fake({ "external-gateway.setup.status": () => old, "external-gateway.setup.inspect": () => ({ ...facts, connection: { ...facts.connection, state: "needs-repair", repair: "resume" }, recovery: { state: "pending", intent: original } }) });
  const { root, view } = start(client); await flush(); button(root, "Repair").click(); await flush();
  await client.emit("external-gateway.setup.watch", old); await flush(); expect(button(root, "Repair").disabled).toBe(true);
  await client.emit("external-gateway.setup.watch", { ...complete, intent: original }); await flush(); expect(button(root, "Repair").disabled).toBe(false); expect(root.textContent).toContain("completed"); expect(calls(client, "apply")).toHaveLength(1); view.stop();
});

it("settles uncertain resumed Repair from meaningful same-ID progress and original-intent completion", async () => {
  const old = { ...complete, phase: "repair-required", error: "external_gateway.output_storage_failed" };
  const client = fake({ "external-gateway.setup.status": () => old, "external-gateway.setup.inspect": () => ({ ...facts, connection: { ...facts.connection, state: "needs-repair", repair: "resume" }, recovery: { state: "pending", intent: "connect" } }), "external-gateway.setup.apply": () => Promise.reject(createPackageRequestError({ code: "deadline_exceeded" })) });
  const { root, view } = start(client); await flush(); button(root, "Repair").click(); await flush(); await client.emit("external-gateway.setup.watch", old); await client.emit("external-gateway.setup.watch", idle); await flush(); expect(button(root, "Repair").disabled).toBe(true);
  await client.emit("external-gateway.setup.watch", { ...old, phase: "outputs", error: null }); await client.emit("external-gateway.setup.watch", complete); await flush(); expect(button(root, "Repair").disabled).toBe(false); expect(root.textContent).toContain("completed"); view.stop();
});
