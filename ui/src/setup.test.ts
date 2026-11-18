import { createFakePackageClient, createPackageRequestError } from "@teamarness/package-ui-sdk/testing";
import { typedPackageClient } from "@teamarness/package-ui-sdk";
import { describe, expect, it } from "vitest";
import type { PackageContract, SetupStatus } from "./contract.generated";
import { setupView } from "./setup";
import { Element, installDom, button, field, flush, deferred } from "./test-dom";
installDom();
const idle: SetupStatus = { operationId: null, intent: null, phase: "idle", command: "unchanged", skill: "not-requested", access: "unchanged", error: null, commandPath: null, skillPath: null };
const plan = { planId: "plan-one", intent: "connect", commandPath: "/private/fixture/.local/bin/tmx", skillPath: "/private/fixture/chosen/teamarness", credentialChange: true, commandChange: { disposition: "explicit-repair", installed: { installation: "old-owner", version: "1.0.0", sha256: "a".repeat(64) }, proposed: { installation: "new-owner", version: "1.1.0", sha256: "b".repeat(64) } }, skillChange: "publish" };
function fake(requests = {}) { return createFakePackageClient({ requests: { "external-gateway.setup.status": () => idle, "external-gateway.setup.preview": () => plan, "external-gateway.setup.apply": () => ({ operationId: "operation-one" }), ...requests }, subscriptions: { "external-gateway.setup.watch": () => undefined, "external-gateway.setup.choose-skill-folder": () => undefined } }); }
function start(client: ReturnType<typeof fake>, managed: boolean | "unknown" = false) {
  const root = new Element("section"); const view = setupView(root as unknown as HTMLElement, typedPackageClient<PackageContract>(client.client));
  if (managed !== "unknown") view.update({ desiredEnabled: false, tokens: managed ? [{ id: "managed", label: "Managed CLI", generation: 1, managed: true }] : [], running: false, port: null, storageFailed: false, warning: null, selectedContracts: 0, selectedTools: 0 });
  return { root, view };
}
describe("Managed setup visible interaction", () => {
  it("selects/cancels a folder, shows exact preview/change consent, and applies only its opaque plan", async () => {
    const client = fake(); const { root, view } = start(client); await flush();
    button(root, "Choose skill folder").click(); await flush(); expect(root.textContent).toContain("Choosing folder");
    button(root, "Cancel folder selection").click(); await client.emit("external-gateway.setup.choose-skill-folder", { outcome: "selected", selectionId: "stale", path: "/stale" }); expect(root.textContent).not.toContain("/stale");
    button(root, "Choose skill folder").click(); await flush();
    await client.emit("external-gateway.setup.choose-skill-folder", { outcome: "selected", selectionId: "selection-one", path: "/private/fixture/chosen" });
    expect(root.textContent).toContain("/private/fixture/chosen"); button(root, "Preview connect").click(); await flush();
    expect(client.calls).toContainEqual({ operation: "external-gateway.setup.preview", payload: { intent: "connect", selectionId: "selection-one" } });
    expect(root.textContent).toContain(plan.commandPath); expect(root.textContent).toContain(plan.skillPath); expect(root.textContent).toContain("old-owner"); expect(root.textContent).toContain("1.1.0"); expect(root.textContent).toContain("b".repeat(64));
    expect(button(root, "Apply setup").disabled).toBe(true); field(root, "Approve setup changes").click(); button(root, "Apply setup").click(); await flush();
    expect(client.calls).toContainEqual({ operation: "external-gateway.setup.apply", payload: { planId: "plan-one" } }); expect(root.textContent).toContain("operation-one"); view.stop();
  });
  it("shows partial outcomes, recovers on remount, and offers repair/rotate/disconnect without blind overwrite", async () => {
    const client = fake(); let { root, view } = start(client); await flush();
    const partial: SetupStatus = { ...idle, operationId: "operation-one", intent: "connect", phase: "repair-required", command: "installed", skill: "pending", access: "repair-required", error: "external_gateway.output_collision", commandPath: plan.commandPath, skillPath: plan.skillPath };
    await client.emit("external-gateway.setup.watch", partial);
    expect(root.textContent).toContain("Command: installed"); expect(root.textContent).toContain("Skill: pending"); expect(root.textContent).toContain("Access: repair-required"); expect(root.textContent).toContain("resolve the collision outside setup");
    for (const intent of ["repair", "rotate", "disconnect"]) { button(root, `Preview ${intent}`).click(); await flush(); expect(client.calls).toContainEqual({ operation: "external-gateway.setup.preview", payload: { intent } }); }
    button(root, "Choose skill folder").click(); await flush();
    await client.emit("external-gateway.setup.choose-skill-folder", { outcome: "selected", selectionId: "repair-folder", path: "/private/fixture/repair" });
    button(root, "Preview skill install").click(); await flush();
    expect(client.calls).toContainEqual({ operation: "external-gateway.setup.preview", payload: { intent: "install-skill", selectionId: "repair-folder" } });
    expect(root.textContent).not.toContain("Overwrite"); view.stop();
    const recovered = fake({ "external-gateway.setup.status": () => ({ ...partial, phase: "complete", skill: "installed", access: "configured", error: null }) }); ({ root, view } = start(recovered)); await flush();
    expect(root.textContent).toContain("Access: configured"); expect(root.textContent).toContain("operation-one"); expect(recovered.calls.filter(c => c.operation === "external-gateway.setup.apply")).toHaveLength(0); view.stop();
  });
  it("stale apply requires new preview, observes progress, and clears requests/subscriptions on stop", async () => {
    const client = fake({ "external-gateway.setup.apply": () => { throw createPackageRequestError({ packageError: { code: "external_gateway.setup_preview_stale" } }); } }); const { root, view } = start(client); await flush();
    button(root, "Preview connect").click(); await flush(); field(root, "Approve setup changes").click(); button(root, "Apply setup").click(); await flush(); expect(root.textContent).toContain("setup_preview_stale"); expect(root.textContent).toContain("new preview"); expect(root.all().some(e => e.textContent === "Apply setup")).toBe(false);
    await client.emit("external-gateway.setup.watch", { ...idle, operationId: "live", phase: "outputs", command: "installed", skill: "pending", access: "pending" }); expect(root.textContent).toContain("outputs");
    view.stop(); view.stop(); const previous = root.textContent; await client.emit("external-gateway.setup.watch", idle); expect(root.textContent).toBe(previous);
  });
  it("ignores late picker establishment and status/request replies after cancel or unmount", async () => {
    const late = deferred<SetupStatus>(); let signal: AbortSignal | undefined;
    const client = fake({ "external-gateway.setup.status": (_p: unknown, s?: AbortSignal) => { signal = s; return late.promise; } }); const { root, view } = start(client);
    button(root, "Choose skill folder").click(); view.stop(); late.resolve(idle); await flush(); expect(signal?.aborted).toBe(true);
    const previous = root.textContent; await client.emit("external-gateway.setup.choose-skill-folder", { outcome: "selected", selectionId: "late", path: "/late" }); expect(root.textContent).toBe(previous);
  });
});

describe("Setup subscription and acceptance lifetime", () => {
  it("closes picker/status subscriptions including late establishment and ignores stale initial status", async () => {
    const { typedPackageClient } = await import("@teamarness/package-ui-sdk");
    const late = deferred<SetupStatus>(); const opening = deferred<void>(); const closed: string[] = [];
    const fakeClient = fake({ "external-gateway.setup.status": () => late.promise });
    const client: import("@teamarness/package-ui-sdk").PackageClient = {
      request: fakeClient.client.request,
      subscribe: async (operation, payload, listener, options) => {
        const stream = await fakeClient.client.subscribe(operation, payload, listener, options);
        if (operation === "external-gateway.setup.choose-skill-folder") await opening.promise;
        return { close: async () => { closed.push(operation); await stream.close(); } };
      },
    };
    const root = new Element("section"); const view = setupView(root as unknown as HTMLElement, typedPackageClient<PackageContract>(client)); await flush();
    await fakeClient.emit("external-gateway.setup.watch", { ...idle, phase: "prepared", operationId: "current" }); late.resolve(idle); await flush(); expect(root.textContent).toContain("current");
    expect(button(root, "Choose skill folder").disabled).toBe(true);
    await fakeClient.emit("external-gateway.setup.watch", { ...idle, phase: "complete", operationId: "current" });
    button(root, "Choose skill folder").click(); await flush(); button(root, "Cancel folder selection").click(); opening.resolve(); await flush(); expect(closed).toEqual(["external-gateway.setup.choose-skill-folder"]);
    button(root, "Choose skill folder").click(); await flush(); view.stop(); await flush();
    expect(closed.sort()).toEqual(["external-gateway.setup.choose-skill-folder", "external-gateway.setup.choose-skill-folder", "external-gateway.setup.watch"].sort());
  });
  it("cancel/error outcomes keep no selection and an accepted apply remount never resubmits", async () => {
    const accepted = deferred<{ operationId: string }>(); const client = fake({ "external-gateway.setup.apply": () => accepted.promise }); const first = start(client); await flush();
    button(first.root, "Choose skill folder").click(); await flush(); await client.emit("external-gateway.setup.choose-skill-folder", { outcome: "cancelled" }); expect(first.root.textContent).toContain("selection cancelled");
    button(first.root, "Choose skill folder").click(); await flush(); client.terminate("external-gateway.setup.choose-skill-folder", createPackageRequestError({ packageError: { code: "external_gateway.picker_failed" } })); expect(first.root.textContent).toContain("picker_failed");
    button(first.root, "Preview connect").click(); await flush(); field(first.root, "Approve setup changes").click(); button(first.root, "Apply setup").click(); await flush(); first.view.stop();
    const remount = fake({ "external-gateway.setup.status": () => ({ ...idle, operationId: "accepted", phase: "accepted", access: "pending", command: "pending" }) }); const second = start(remount); accepted.resolve({ operationId: "accepted" }); await flush();
    expect(second.root.textContent).toContain("accepted"); expect(remount.calls.filter(c => c.operation === "external-gateway.setup.apply")).toHaveLength(0); expect(client.calls.filter(c => c.operation === "external-gateway.setup.apply")).toHaveLength(1); second.view.stop();
  });
});

it("explains an unexpectedly ended chooser instead of silently keeping a pending selection", async () => {
  const client = fake(); const { root, view } = start(client); await flush();
  button(root, "Choose skill folder").click(); await flush(); client.terminate("external-gateway.setup.choose-skill-folder", null);
  expect(root.textContent).toContain("Folder selection ended. Choose again."); expect(root.textContent).not.toContain("Choosing folder"); view.stop();
});

it("routes an already connected Workspace to an additional folder without repeating connect or changing credentials", async () => {
  const connected: SetupStatus = { ...idle, operationId: "connected", intent: "connect", phase: "complete", command: "installed", access: "configured" };
  const client = fake({
    "external-gateway.setup.status": () => connected,
    "external-gateway.setup.preview": (p: { intent: string }) => {
      if (p.intent === "connect") throw createPackageRequestError({ packageError: { code: "external_gateway.managed_setup_required" } });
      return { ...plan, intent: "install-skill", credentialChange: false };
    },
  });
  const { root, view } = start(client); await flush();
  expect(root.all().some(e => e.tagName === "button" && e.textContent === "Preview connect" && !e.disabled)).toBe(false);
  expect(root.textContent).toContain("another folder");
  expect(button(root, "Preview skill install").disabled).toBe(true);
  button(root, "Choose skill folder").click(); await flush();
  await client.emit("external-gateway.setup.choose-skill-folder", { outcome: "selected", selectionId: "second-folder", path: "/private/fixture/second" });
  button(root, "Preview skill install").click(); await flush();
  expect(client.calls.filter(c => c.operation === "external-gateway.setup.preview")).toEqual([{ operation: "external-gateway.setup.preview", payload: { intent: "install-skill", selectionId: "second-folder" } }]);
  expect(root.textContent).toContain("No managed credential change");
  expect(button(root, "Apply setup").disabled).toBe(true);
  field(root, "Approve setup changes").click(); button(root, "Apply setup").click(); await flush();
  await client.emit("external-gateway.setup.watch", { ...connected, operationId: "skill-two", intent: "install-skill", skill: "installed", access: "unchanged" });
  expect(root.textContent).toContain("Command: installed"); expect(root.textContent).toContain("Skill: installed"); expect(root.textContent).toContain("Access: unchanged");
  expect(client.calls.some(c => c.operation.includes("token."))).toBe(false);
  expect(root.all().some(e => e.tagName === "button" && e.textContent === "Preview connect" && !e.disabled)).toBe(false);
  view.stop();
});

it("dismisses identical setup completion notifications without hiding pending work or later failures", async () => {
  const completed: SetupStatus = { ...idle, operationId: "done", intent: "connect", phase: "complete", command: "installed", skill: "not-requested", access: "configured" };
  const client = fake({ "external-gateway.setup.status": () => completed }); const { root, view } = start(client); await flush();
  button(root, "Dismiss setup notification").click();
  await client.emit("external-gateway.setup.watch", completed);
  const notice = field(root, "Setup notification"); expect(notice.hidden).toBe(true);
  await client.emit("external-gateway.setup.watch", { ...completed, phase: "outputs", skill: "pending" }); expect(notice.hidden).toBe(false); expect(notice.textContent).toContain("pending");
  await client.emit("external-gateway.setup.watch", { ...completed, phase: "repair-required", access: "repair-required", error: "external_gateway.output_collision" });
  expect(notice.hidden).toBe(false); expect(notice.textContent).toContain("output_collision"); view.stop();
});

it("does not let a late acceptance reply replace a completed dismissible outcome", async () => {
  const accepted = deferred<{ operationId: string }>();
  const client = fake({ "external-gateway.setup.apply": () => accepted.promise }); const { root, view } = start(client); await flush();
  button(root, "Preview connect").click(); await flush(); field(root, "Approve setup changes").click(); button(root, "Apply setup").click(); await flush();
  await client.emit("external-gateway.setup.watch", { ...idle, operationId: "fast", intent: "connect", phase: "complete", command: "installed", access: "configured" });
  accepted.resolve({ operationId: "fast" }); await flush();
  expect(root.textContent).not.toContain("Accepted operation:"); button(root, "Dismiss setup notification").click(); expect(field(root, "Setup notification").hidden).toBe(true); view.stop();
});

it("keeps explained maintenance secondary and requires fresh explicit consent for each action", async () => {
  const client = fake(); const { root, view } = start(client); await flush();
  const maintenance = root.all().find(e => e.tagName === "details" && e.children.some(c => c.tagName === "summary" && c.textContent === "Connection maintenance"))!;
  expect(maintenance.open).toBe(false); expect(maintenance.textContent).toContain("old generation stops working"); expect(maintenance.textContent).toContain("Shared command and skill files remain");
  for (const intent of ["repair", "rotate", "disconnect"]) {
    expect(button(root, `Preview ${intent}`).parent?.parent).toBe(maintenance);
    button(root, `Preview ${intent}`).click(); await flush(); expect(button(root, "Apply setup").disabled).toBe(true);
    field(root, "Approve setup changes").click(); button(root, "Cancel setup preview").click();
  }
  expect(client.calls.some(c => c.operation === "external-gateway.setup.apply")).toBe(false); view.stop();
});

it("waits for initial setup status before offering connect, including a slow already-current observation", async () => {
  const initial = deferred<SetupStatus>(); const client = fake({ "external-gateway.setup.status": () => initial.promise }); const { root, view } = start(client, "unknown");
  expect(button(root, "Preview connect").disabled).toBe(true); button(root, "Preview connect").click(); await flush();
  expect(client.calls.some(c => c.operation === "external-gateway.setup.preview")).toBe(false);
  initial.resolve({ ...idle, phase: "complete", access: "configured" }); await flush();
  expect(root.all().some(e => e.tagName === "button" && e.textContent === "Preview connect")).toBe(false); view.stop();
});
