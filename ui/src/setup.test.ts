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
function start(client: ReturnType<typeof fake>) { const root = new Element("section"); const view = setupView(root as unknown as HTMLElement, typedPackageClient<PackageContract>(client.client)); return { root, view }; }
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
    for (const intent of ["repair", "rotate", "disconnect", "install-skill"]) { button(root, `Preview ${intent}`).click(); await flush(); expect(client.calls).toContainEqual({ operation: "external-gateway.setup.preview", payload: { intent } }); }
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
