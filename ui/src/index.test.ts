// Verifies: TEAM.SYS.PKG-R19. Visible UI behavior through the mount-scoped SDK client.
import { createFakePackageClient, createPackageRequestError, createTestMountContext } from "@teamarness/package-ui-sdk/testing";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { PackageUnmount } from "@teamarness/package-ui-sdk";
import type { Status, Contract, Tool } from "./contract.generated";
import { mount } from "./index";
import { Element, installDom, button, field, flush, deferred } from "./test-dom";
installDom();
const status: Status = { desiredEnabled: false, tokens: [], running: false, port: null, storageFailed: false, warning: null, selectedContracts: 0, selectedTools: 0 };
const emptyPage = { contracts: [], tools: [], nextCursor: null, nextOffset: null };
let unmount: PackageUnmount | undefined;
afterEach(async () => { await unmount?.(); unmount = undefined; vi.useRealTimers(); });
function setup(client: ReturnType<typeof createFakePackageClient>, view: "main" | "settings" = "main") {
  const root = new Element("shadow-root");
  unmount = mount(createTestMountContext({ root, client: client.client, view, displayName: "External Gateway" }));
  return root;
}
function fake(requests = {}) {
  return createFakePackageClient({ requests: { "external-gateway.status.get": () => status, "external-gateway.catalog.page": () => emptyPage, ...requests }, subscriptions: { "external-gateway.status.watch": () => undefined } });
}
describe("External Gateway main/settings mount", () => {
  it("shows Workspace, read-only status and secret-free onboarding; navigates settings without Shell controls", async () => {
    const client = fake(); const root = setup(client);
    expect(root.textContent).toContain("Loading");
    await flush();
    expect(root.textContent).toContain("Test Workspace");
    expect(root.textContent).toContain("workspace1");
    expect(root.textContent).toContain("Read-only");
    expect(root.textContent).toContain("Listener stopped");
    expect(root.textContent).toContain("<BEARER>");
    expect(client.calls.filter(c => c.operation === "external-gateway.catalog.page")).toHaveLength(0);
    button(root, "Settings").click(); await flush();
    expect(root.textContent).toContain("All tokens share");
    expect(client.calls.filter(c => c.operation === "external-gateway.catalog.page")).toHaveLength(1);
    button(root, "Overview").click(); await flush();
    expect(root.textContent).toContain("External-agent onboarding");
  });
  it("shows loading/error and subscription state; older status reply cannot replace a newer event", async () => {
    const pending = deferred<Status>();
    const client = fake({ "external-gateway.status.get": () => pending.promise }); const root = setup(client);
    await flush();
    await client.emit("external-gateway.status.watch", { ...status, desiredEnabled: true, running: true, port: 32123 });
    pending.resolve(status); await flush();
    expect(root.textContent).toContain("http://127.0.0.1:32123/v1");
    await client.emit("external-gateway.status.watch", { ...status, desiredEnabled: true, port: 32123, warning: "listener_bind_failed" });
    expect(root.textContent).toContain("listener_bind_failed");
    expect(root.textContent).not.toContain("http://127.0.0.1:32123/v1");
    client.terminate("external-gateway.status.watch", createPackageRequestError({ packageError: { code: "external_gateway.unavailable" } }));
    expect(root.textContent).toContain("external_gateway.unavailable");
  });
  it("distinguishes membership removal from monitoring failure without publishing a stopped URL", async () => {
    const client = fake(); const root = setup(client); await flush();
    await client.emit("external-gateway.status.watch", { ...status, desiredEnabled: true, port: 32123, warning: "membership_removed" });
    expect(root.textContent).toContain("This Installation is no longer a Workspace member");
    expect(root.textContent).not.toContain("http://127.0.0.1:32123/v1");
    await client.emit("external-gateway.status.watch", { ...status, desiredEnabled: true, port: 32123, warning: "membership_unavailable" });
    expect(root.textContent).toContain("Membership monitoring is unavailable");
    expect(root.textContent).not.toContain("This Installation is no longer a Workspace member");
  });
  it("reports initial failure and aborts work, closes late subscriptions and unmounts idempotently", async () => {
    const client = fake({ "external-gateway.status.get": () => { throw createPackageRequestError({ packageError: { code: "external_gateway.storage_failed" } }); } });
    const root = setup(client); await flush(); expect(root.textContent).toContain("external_gateway.storage_failed");
    await unmount?.(); await unmount?.(); expect(root.children).toHaveLength(0); expect(root.adoptedStyleSheets).toHaveLength(0);
    const pending = deferred<Status>(); let signal: AbortSignal | undefined;
    const delayed = fake({ "external-gateway.status.get": (_p: unknown, s?: AbortSignal) => { signal = s; return pending.promise; } });
    const other = setup(delayed); await unmount?.(); pending.resolve(status); await flush();
    await delayed.emit("external-gateway.status.watch", status);
    expect(signal?.aborted).toBe(true); expect(other.children).toHaveLength(0);
  });
});

describe("Shared settings and one-time tokens", () => {
  it("changes master/port through policy and creates, regenerates, revokes tokens without scope editors", async () => {
    let current = status;
    const client = fake({
      "external-gateway.status.get": () => current,
      "external-gateway.listener.set": () => ({ ok: true, warning: null }),
      "external-gateway.port.set": () => ({ ok: true, warning: null }),
      "external-gateway.token.create": () => ({ id: "one", bearer: "test-only-created-secret" }),
      "external-gateway.token.regenerate": () => ({ id: "one", bearer: "test-only-regenerated-secret" }),
      "external-gateway.token.revoke": () => ({ ok: true, warning: "host_cleanup_failed" }),
      "external-gateway.disable-and-revoke-all": () => ({ ok: true, warning: null }),
    });
    const root = setup(client, "settings"); await flush();
    button(root, "Enable listener").click(); await flush();
    field(root, "Saved port").value = "32145"; button(root, "Change port").click(); await flush();
    expect(client.calls).toContainEqual({ operation: "external-gateway.port.set", payload: { port: 32145 } });
    field(root, "Token label").value = "coding client"; button(root, "Create token").click(); await flush();
    expect(root.textContent).toContain("test-only-created-secret");
    expect(root.textContent).toContain("Save this token now");
    button(root, "Dismiss token").click(); expect(root.textContent).not.toContain("test-only-created-secret");
    current = { ...status, tokens: [{ id: "one", label: "coding client", generation: 1 }] };
    await client.emit("external-gateway.status.watch", current);
    button(root, "Regenerate coding client").click(); await flush();
    expect(root.textContent).toContain("test-only-regenerated-secret");
    button(root, "Revoke coding client").click(); await flush();
    expect(root.textContent).not.toContain("test-only-regenerated-secret");
    expect(root.textContent).toContain("host_cleanup_failed");
    button(root, "Disable and revoke all tokens").click(); await flush();
    expect(client.calls).toContainEqual({ operation: "external-gateway.disable-and-revoke-all", payload: {} });
    expect(root.textContent).toContain("Already-authorized reads may finish");
    expect(root.textContent).toContain("restart and reinstall");
    expect(root.all().filter(e => e.tagName === "input").map(e => e.attributes.get("aria-label"))).toEqual(["Saved port", "Token label"]);
    expect(client.calls.filter(c => c.operation.includes("token.")).every(c => !JSON.stringify(c.payload).includes("secret"))).toBe(true);
  });
  it("validates port/UTF-8 labels and does not render lost secret replies after navigation or unmount", async () => {
    const secret = deferred<{ id: string; bearer: string }>(); let signal: AbortSignal | undefined;
    const client = fake({ "external-gateway.token.create": (_p: unknown, s?: AbortSignal) => { signal = s; return secret.promise; } });
    const root = setup(client, "settings"); await flush();
    field(root, "Saved port").value = "0"; button(root, "Change port").click(); await flush();
    expect(root.textContent).toContain("1–65535");
    field(root, "Token label").value = "🙂".repeat(25); button(root, "Create token").click(); await flush();
    expect(root.textContent).toContain("96 UTF-8 bytes");
    expect(client.calls.filter(c => c.operation === "external-gateway.token.create")).toHaveLength(0);
    field(root, "Token label").value = "client"; button(root, "Create token").click(); await flush();
    button(root, "Overview").click(); secret.resolve({ id: "lost", bearer: "test-only-lost-secret" }); await flush();
    expect(signal?.aborted).toBe(true); expect(root.textContent).not.toContain("test-only-lost-secret");
    button(root, "Settings").click(); await flush(); expect(root.textContent).not.toContain("test-only-lost-secret");
    await unmount?.(); expect(root.children).toHaveLength(0);
  });
});

const contract: Contract = { contract: "dev.example.alpha", major: 1, provider: "dev.example.multi", displayName: "Multi", available: true, hostEnabled: false, selected: false, guideEligibility: "eligible", excluded: false };
const tool: Tool = { port: "read.one", fingerprint: "a".repeat(64), eligible: true, selected: false, reviewRequired: false };
describe("Bounded catalog settings", () => {
  it("groups contracts, selects exact tools, keeps new tools off and explains excluded/missing/unavailable choices", async () => {
    const rows = [contract, { ...contract, contract: "dev.example.beta" }, { ...contract, contract: "dev.example.missing", provider: "dev.example.guide-less", guideEligibility: "missing" }, { ...contract, contract: "dev.teamarness.conversations", provider: "dev.teamarness.conversations", excluded: true }, { ...contract, contract: "dev.example.gone", provider: null, displayName: null, available: false, hostEnabled: true, guideEligibility: "unavailable" }];
    const client = fake({
      "external-gateway.catalog.page": (p: { contract?: string }) => p.contract ? { ...emptyPage, tools: [tool, { ...tool, port: "read.new" }, { ...tool, port: "read.changed", selected: true, reviewRequired: true }, { ...tool, port: "runtime.guide", eligible: false }, { ...tool, port: "write.one", eligible: false }] } : { ...emptyPage, contracts: rows },
      "external-gateway.package.set": () => ({ ok: true, warning: null }),
      "external-gateway.tool.set": () => ({ ok: true, warning: null }),
    });
    const root = setup(client, "settings"); await flush();
    expect(root.textContent).toContain("Missing runtime guide"); expect(root.textContent).toContain("Conversations is excluded");
    button(root, "Enable displayed contracts · Multi").click(); await flush();
    expect(client.calls).toContainEqual({ operation: "external-gateway.package.set", payload: { provider: "dev.example.multi", enabled: true, contracts: [{ contract: "dev.example.alpha", major: 1 }, { contract: "dev.example.beta", major: 1 }] } });
    button(root, "Tools · dev.example.alpha").click(); await flush();
    expect(button(root, "Enable read.new").disabled).toBe(false);
    expect(root.textContent).toContain("Review required"); expect(root.textContent).toContain("Guide included automatically");
    expect(root.all().some(e => e.tagName === "button" && e.textContent === "Enable write.one")).toBe(false);
    button(root, "Enable read.one").click(); await flush();
    expect(client.calls).toContainEqual({ operation: "external-gateway.tool.set", payload: { contract: "dev.example.alpha", major: 1, provider: "dev.example.multi", port: "read.one", fingerprint: "a".repeat(64), enabled: true } });
    button(root, "Remove unavailable · dev.example.gone").click(); await flush();
    expect(client.calls).toContainEqual({ operation: "external-gateway.package.set", payload: { provider: null, enabled: false, contracts: [{ contract: "dev.example.gone", major: 1 }] } });
  });
  it("paginates without accumulating catalog, polls only settings, and cancels/discards a stale refresh before mutation", async () => {
    vi.useFakeTimers(); let hold = false; let observed: AbortSignal | undefined;
    const pending = deferred<Omit<typeof emptyPage, "contracts"> & { contracts: Contract[] }>();
    const client = fake({
      "external-gateway.catalog.page": (p: { cursor?: string }, signal?: AbortSignal) => {
        if (hold) { observed = signal; return pending.promise; }
        return { ...emptyPage, contracts: [p.cursor ? { ...contract, contract: "dev.example.next" } : contract], nextCursor: p.cursor ? null : "next" };
      },
      "external-gateway.package.set": () => ({ ok: true, warning: null }),
    });
    const root = setup(client, "settings"); await flush();
    button(root, "Next contracts").click(); await flush();
    expect(root.textContent).toContain("dev.example.next"); expect(root.textContent).not.toContain("dev.example.alpha");
    button(root, "First contracts").click(); await flush(); hold = true;
    await vi.advanceTimersByTimeAsync(5000); await flush();
    const count = client.calls.length;
    button(root, "Refresh").click(); await vi.advanceTimersByTimeAsync(15000);
    expect(client.calls).toHaveLength(count);
    button(root, "Disable Package · Multi").click(); await flush();
    expect(observed?.aborted).toBe(true);
    hold = false; pending.resolve({ ...emptyPage, contracts: [{ ...contract, contract: "dev.example.stale" }] }); await flush();
    expect(root.textContent).not.toContain("dev.example.stale");
    button(root, "Overview").click(); const after = client.calls.length;
    await vi.advanceTimersByTimeAsync(10000); expect(client.calls).toHaveLength(after);
    await unmount?.(); expect(vi.getTimerCount()).toBe(0);
  });
  it("reports catalog failures, retries stale cursor at most once, and cancels a pending tool-page on unmount", async () => {
    vi.useFakeTimers();
    const pending = deferred<typeof emptyPage>(); let signal: AbortSignal | undefined;
    let stale = false;
    const client = fake({ "external-gateway.catalog.page": (p: { contract?: string }, s?: AbortSignal) => {
      if (p.contract) { signal = s; return pending.promise; }
      if (stale) throw createPackageRequestError({ packageError: { code: "port_catalog_changed" } });
      return { ...emptyPage, contracts: [contract] };
    } });
    const root = setup(client, "settings"); await flush(); stale = true;
    const before = client.calls.length; button(root, "Refresh").click(); await flush();
    expect(client.calls.length - before).toBe(2); expect(root.textContent).toContain("port_catalog_changed");
    stale = false; button(root, "Refresh").click(); await flush();
    button(root, "Tools · dev.example.alpha").click(); await flush(); await unmount?.();
    expect(signal?.aborted).toBe(true); expect(vi.getTimerCount()).toBe(0); pending.resolve(emptyPage); await flush(); expect(root.children).toHaveLength(0);
  });
});

describe("Catalog and secret lifetime boundaries", () => {
  it("keeps only a tool page, polls at five seconds and reports the bounded read deadline", async () => {
    vi.useFakeTimers(); let pendingRead = false;
    const pending = deferred<typeof emptyPage>();
    const client = fake({ "external-gateway.catalog.page": (p: { contract?: string; offset?: number }) => {
      if (pendingRead) return pending.promise;
      if (p.contract) return { ...emptyPage, tools: [{ ...tool, port: p.offset ? "read.last" : "read.first" }], nextOffset: p.offset ? null : 32 };
      return { ...emptyPage, contracts: [contract] };
    } });
    const root = setup(client, "settings"); await flush();
    button(root, "Tools · dev.example.alpha").click(); await flush();
    button(root, "Next tools").click(); await flush();
    expect(root.textContent).toContain("read.last"); expect(root.textContent).not.toContain("read.first");
    const count = client.calls.length; pendingRead = true;
    await vi.advanceTimersByTimeAsync(4999); expect(client.calls).toHaveLength(count);
    await vi.advanceTimersByTimeAsync(1); expect(client.calls).toHaveLength(count + 1);
    await vi.advanceTimersByTimeAsync(4999); expect(root.textContent).not.toContain("timed out");
    await vi.advanceTimersByTimeAsync(1); expect(root.textContent).toContain("timed out");
    await unmount?.(); expect(vi.getTimerCount()).toBe(0);
    pending.resolve(emptyPage); await flush(); expect(root.children).toHaveLength(0);
  });
  it("erases displayed secrets on unmount and ignores later subscription events", async () => {
    const client = fake({ "external-gateway.token.create": () => ({ id: "one", bearer: "test-only-displayed-secret" }) });
    const root = setup(client, "settings"); await flush();
    field(root, "Token label").value = "client"; button(root, "Create token").click(); await flush();
    const region = field(root, "One-time token"); expect(region.textContent).toContain("test-only-displayed-secret");
    await unmount?.(); expect(region.textContent).not.toContain("test-only-displayed-secret");
    await client.emit("external-gateway.status.watch", status); expect(root.children).toHaveLength(0);
  });
});

describe("Catalog navigation freshness", () => {
  it("never attaches a late tool page to a different contract and waits for the cancelled observation before refreshing", async () => {
    const pending = deferred<{ contracts: Contract[]; tools: Tool[]; nextCursor: null; nextOffset: null }>();
    let signal: AbortSignal | undefined;
    const client = fake({ "external-gateway.catalog.page": (p: { contract?: string }, s?: AbortSignal) => {
      if (p.contract === "dev.example.alpha") { signal = s; return pending.promise; }
      if (p.contract) return { ...emptyPage, tools: [{ ...tool, port: "read.beta" }] };
      return { ...emptyPage, contracts: [contract, { ...contract, contract: "dev.example.beta" }] };
    } });
    const root = setup(client, "settings"); await flush();
    button(root, "Tools · dev.example.alpha").click(); await flush();
    const before = client.calls.length; button(root, "Tools · dev.example.beta").click(); await flush();
    expect(signal?.aborted).toBe(true); expect(client.calls).toHaveLength(before);
    pending.resolve({ ...emptyPage, tools: [{ ...tool, port: "read.stale" }] }); await flush();
    expect(root.textContent).not.toContain("read.stale"); expect(root.textContent).toContain("read.beta");
  });
});
