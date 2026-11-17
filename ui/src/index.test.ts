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
function setup(client: ReturnType<typeof createFakePackageClient>, view: "main" | "settings" = "main", navigation?: { openSettings(): void }) {
  const root = new Element("shadow-root");
  unmount = mount(createTestMountContext({ root, client: client.client, view, navigation, displayName: "External Gateway" }));
  return root;
}
function fake(requests = {}) {
  return createFakePackageClient({ requests: { "external-gateway.status.get": () => status, "external-gateway.catalog.page": () => emptyPage, ...requests }, subscriptions: { "external-gateway.status.watch": () => undefined } });
}
describe("External Gateway main/settings mount", () => {
  it("shows Workspace, shared status and secret-free onboarding with real settings mounted separately", async () => {
    const client = fake(); const root = setup(client); expect(root.textContent).toContain("Loading"); await flush();
    expect(root.textContent).toContain("Test Workspace"); expect(root.textContent).toContain("workspace1"); expect(root.textContent).toContain("Person-selected"); expect(root.textContent).toContain("Listener stopped"); expect(root.textContent).toContain("<BEARER>");
    expect(client.calls.filter(c => c.operation === "external-gateway.catalog.page")).toHaveLength(1);
    await unmount?.(); const config = setup(fake(), "settings"); await flush();
    expect(config.textContent).toContain("All tokens share"); expect(config.textContent).toContain("passed local admission may still be sent to the Host"); expect(config.textContent).toContain("external agent's own stop control");
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
  it("changes master through policy (without a port editor) and creates, regenerates, revokes tokens without scope editors", async () => {
    let current = status;
    const client = fake({
      "external-gateway.status.get": () => current,
      "external-gateway.listener.set": () => ({ ok: true, warning: null }),
      "external-gateway.token.create": () => ({ id: "one", bearer: "test-only-created-secret" }),
      "external-gateway.token.regenerate": () => ({ id: "one", bearer: "test-only-regenerated-secret" }),
      "external-gateway.token.revoke": () => ({ ok: true, warning: "host_cleanup_failed" }),
      "external-gateway.disable-and-revoke-all": () => ({ ok: true, warning: null }),
    });
    const root = setup(client, "settings"); await flush();
    field(root, "Enable gateway listener").click(); await flush();
    expect(client.calls).toContainEqual({ operation: "external-gateway.listener.set", payload: { enabled: true } });
    expect(client.calls.filter(c => c.operation === "external-gateway.port.set")).toHaveLength(0);
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
    expect(client.calls.filter(c => c.operation === "external-gateway.disable-and-revoke-all")).toHaveLength(0);
    button(root, "Disable and revoke").click(); await flush();
    expect(client.calls).toContainEqual({ operation: "external-gateway.disable-and-revoke-all", payload: {} });
    expect(root.textContent).toContain("may still be sent to the Host and finish after disable, revoke or regeneration succeeds");
    expect(root.textContent).toContain("Later admissions use the new policy");
    expect(root.textContent).toContain("restart and reinstall");
    expect(root.all().filter(e => e.tagName === "input").map(e => e.attributes.get("aria-label"))).toEqual(["Token label"]);
    expect(client.calls.filter(c => c.operation.includes("token.")).every(c => !JSON.stringify(c.payload).includes("secret"))).toBe(true);
  });
  it("validates UTF-8 labels and does not render lost secret replies after navigation or unmount", async () => {
    const secret = deferred<{ id: string; bearer: string }>(); let signal: AbortSignal | undefined;
    const client = fake({ "external-gateway.token.create": (_p: unknown, s?: AbortSignal) => { signal = s; return secret.promise; } });
    const root = setup(client, "settings"); await flush();
    field(root, "Token label").value = "🙂".repeat(25); button(root, "Create token").click(); await flush();
    expect(root.textContent).toContain("96 UTF-8 bytes");
    expect(client.calls.filter(c => c.operation === "external-gateway.token.create")).toHaveLength(0);
    field(root, "Token label").value = "client"; button(root, "Create token").click(); await flush();
    await unmount?.(); secret.resolve({ id: "lost", bearer: "test-only-lost-secret" }); await flush();
    expect(signal?.aborted).toBe(true); expect(root.textContent).not.toContain("test-only-lost-secret");
    const reopened = setup(fake(), "settings"); await flush(); expect(reopened.textContent).not.toContain("test-only-lost-secret");
    await unmount?.(); expect(root.children).toHaveLength(0);
  });
});

const contract: Contract = { contract: "dev.example.alpha", major: 1, provider: "dev.example.multi", displayName: "Multi", available: true, hostEnabled: true, selected: true, guideEligibility: "eligible" };
const tool: Tool = { port: "read.one", fingerprint: "a".repeat(64), eligible: true, selected: false, reviewRequired: false, kind: "query", delivery: "response" };
describe("Bounded catalog settings", () => {
  it("groups contracts, selects exact tools, keeps new tools off and explains missing/unavailable choices", async () => {
    let enabled = false;
    const rows = [contract, { ...contract, contract: "dev.example.beta" }, { ...contract, contract: "dev.example.missing", provider: "dev.example.guide-less", guideEligibility: "missing" }, { ...contract, contract: "dev.teamarness.conversations", provider: "dev.teamarness.conversations" }, { ...contract, contract: "dev.example.gone", provider: null, displayName: null, available: false, hostEnabled: true, guideEligibility: "unavailable" }];
    const client = fake({
      "external-gateway.catalog.page": (p: { contract?: string }) => p.contract ? { ...emptyPage, tools: [tool, { ...tool, port: "read.new" }, { ...tool, port: "read.changed", selected: true, reviewRequired: true }, { ...tool, port: "runtime.guide", eligible: false }, { ...tool, port: "write.one", kind: "command" }, { ...tool, port: "watch.one", delivery: "stream" }, { ...tool, port: "events.one", kind: "event", delivery: "stream" }] } : { ...emptyPage, contracts: rows.map(c => ({ ...c, selected: enabled && c.selected, hostEnabled: enabled && c.hostEnabled })) },
      "external-gateway.package.set": () => { enabled = true; return { ok: true, warning: null }; },
      "external-gateway.tool.set": () => ({ ok: true, warning: null }),
    });
    const root = setup(client, "settings"); await flush();
    expect(root.textContent).toContain("Missing runtime guide"); expect(button(root, "Tools · dev.teamarness.conversations").disabled).toBe(true);
    field(root, "Enable Multi access").click(); await flush();
    expect(client.calls).toContainEqual({ operation: "external-gateway.package.set", payload: { provider: "dev.example.multi", enabled: true, contracts: [{ contract: "dev.example.alpha", major: 1 }, { contract: "dev.example.beta", major: 1 }] } });
    button(root, "Tools · dev.example.alpha").click(); await flush();
    expect(button(root, "Enable read read.new").disabled).toBe(false);
    expect(root.textContent).toContain("Review required"); expect(root.textContent).toContain("Guide included automatically");
    expect(button(root, "Enable write write.one").disabled).toBe(false);
    expect(button(root, "Enable streaming query watch.one").disabled).toBe(false);
    expect(button(root, "Enable event events.one").disabled).toBe(false);
    expect(root.textContent).toContain("Write"); expect(root.textContent).toContain("Streaming query"); expect(root.textContent).toContain("Event");
    expect(root.textContent).toContain("writes act as the local Workspace Person");
    expect(client.calls.filter(c => c.operation === "external-gateway.tool.set")).toHaveLength(0);
    button(root, "Enable read read.one").click(); await flush();
    expect(client.calls).toContainEqual({ operation: "external-gateway.tool.set", payload: { contract: "dev.example.alpha", major: 1, provider: "dev.example.multi", port: "read.one", fingerprint: "a".repeat(64), enabled: true } });
    button(root, "Enable write write.one").click(); await flush();
    expect(client.calls).toContainEqual({ operation: "external-gateway.tool.set", payload: { contract: "dev.example.alpha", major: 1, provider: "dev.example.multi", port: "write.one", fingerprint: "a".repeat(64), enabled: true } });
    button(root, "Enable streaming query watch.one").click(); await flush();
    button(root, "Enable event events.one").click(); await flush();
    expect(client.calls.filter(c => c.operation === "external-gateway.tool.set")).toEqual(["read.one", "write.one", "watch.one", "events.one"].map(port => ({ operation: "external-gateway.tool.set", payload: { contract: "dev.example.alpha", major: 1, provider: "dev.example.multi", port, fingerprint: "a".repeat(64), enabled: true } })));
    button(root, "Remove unavailable · dev.example.gone").click(); button(root, "Disable Package").click(); await flush();
    expect(client.calls).toContainEqual({ operation: "external-gateway.package.set", payload: { provider: null, enabled: false, contracts: [{ contract: "dev.example.gone", major: 1 }] } });
  });
  it("paginates without accumulating catalog, polls the mounted body, and cancels/discards a stale refresh before mutation", async () => {
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
    field(root, "Refresh available Packages").click(); await vi.advanceTimersByTimeAsync(15000);
    expect(client.calls).toHaveLength(count);
    field(root, "Enable Multi access").click(); button(root, "Disable Package").click(); await flush();
    expect(observed?.aborted).toBe(true);
    hold = false; pending.resolve({ ...emptyPage, contracts: [{ ...contract, contract: "dev.example.stale" }] }); await flush();
    expect(root.textContent).not.toContain("dev.example.stale");
    await unmount?.(); const after = client.calls.length;
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
    const before = client.calls.length; field(root, "Refresh available Packages").click(); await flush();
    expect(client.calls.length - before).toBe(2); expect(root.textContent).toContain("port_catalog_changed");
    stale = false; field(root, "Refresh available Packages").click(); await flush();
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

describe("Managed token coordinator routing", () => {
  it("routes managed rotation through explicit preview/consent, retaining manual revoke and secret cleanup", async () => {
    const client = fake({
      "external-gateway.status.get": () => ({ ...status, tokens: [{ id: "managed", label: "Managed CLI", generation: 1, managed: true }] }),
      "external-gateway.setup.status": () => ({ operationId: null, intent: null, phase: "idle", command: "unchanged", skill: "not-requested", access: "unchanged", error: null, commandPath: null, skillPath: null }),
      "external-gateway.setup.preview": () => ({ planId: "rotate-plan", intent: "rotate", commandPath: "/private/fixture/.local/bin/tmx", skillPath: null, credentialChange: true, commandChange: null, skillChange: "not-requested" }),
      "external-gateway.token.revoke": () => ({ ok: true, warning: null }),
    });
    const root = setup(client, "settings"); await flush(); button(root, "Rotate Managed CLI").click(); await flush();
    expect(client.calls).toContainEqual({ operation: "external-gateway.setup.preview", payload: { intent: "rotate" } }); expect(client.calls.filter(c => c.operation === "external-gateway.token.regenerate")).toHaveLength(0);
    expect(button(root, "Apply setup").disabled).toBe(true); button(root, "Revoke Managed CLI").click(); await flush(); expect(client.calls).toContainEqual({ operation: "external-gateway.token.revoke", payload: { id: "managed" } });
    await unmount?.(); expect(root.children).toHaveLength(0);
  });
});


describe("Revised A mount-owned controls", () => {
  it.each(["main", "settings"] as const)("shares the flat control body in %s without imitation Shell controls or a port editor", async view => {
    const client = fake(); const root = setup(client, view); await flush();
    expect(root.all().filter(e => e.tagName === "h2").map(e => e.textContent)).toEqual(view === "main" ? ["Gateway", "Packages", "Agent setup"] : ["Gateway", "Packages", "Agent setup", "Tokens"]);
    expect(root.all().filter(e => e.tagName === "h1")).toHaveLength(view === "main" ? 1 : 0);
    expect(root.all().filter(e => e.tagName === "nav" && e.attributes.get("aria-label") === "Gateway views")).toHaveLength(0);
    expect(root.all().some(e => e.tagName === "input" && e.attributes.get("aria-label") === "Saved port")).toBe(false);
    expect(root.all().some(e => e.tagName === "button" && e.textContent === "Change port")).toBe(false);
    expect(client.calls.filter(c => c.operation === "external-gateway.catalog.page")).toHaveLength(1);
    expect(root.all().some(e => e.textContent === "Create token")).toBe(view === "settings");
  });
  it("uses only the offered no-argument own-settings intent; old hosts explain the disabled gear", async () => {
    const openSettings = vi.fn(); const client = fake(); let root = setup(client, "main", { openSettings }); await flush();
    field(root, "Open configuration").click(); expect(openSettings.mock.calls).toEqual([[]]);
    expect(client.calls.some(c => c.operation.includes("navigation"))).toBe(false);
    await unmount?.(); root = setup(fake()); await flush();
    expect(field(root, "Open configuration").disabled).toBe(true); expect(root.textContent).toContain("Update Teamarness");
  });
  it("dismisses bind presentation only, retries the saved port explicitly, and catalog refresh never changes a listener", async () => {
    let current = { ...status, desiredEnabled: true, port: 32123, warning: "listener_bind_failed" as string | null };
    let occupied = true;
    const client = fake({ "external-gateway.status.get": () => current, "external-gateway.listener.set": () => {
      if (occupied) throw createPackageRequestError({ packageError: { code: "external_gateway.unavailable", retryable: true } });
      current = { ...current, running: true, warning: null }; return { ok: true, warning: null };
    } });
    const root = setup(client); await flush(); expect(root.textContent).not.toContain("http://127.0.0.1:32123/v1");
    button(root, "Dismiss bind error").click(); expect(root.textContent).not.toContain("Free the saved port"); expect(root.textContent).toContain("Listener stopped");
    field(root, "Retry listener on port 32123").click(); await flush(); expect(root.textContent).toContain("Free the saved port");
    expect(client.calls.filter(c => c.operation === "external-gateway.listener.set")).toEqual([{ operation: "external-gateway.listener.set", payload: { enabled: true } }]);
    expect(root.textContent).toContain("external_gateway.unavailable"); expect(root.textContent).not.toContain("http://127.0.0.1:32123/v1");
    occupied = false; field(root, "Retry listener on port 32123").click(); await flush();
    expect(client.calls.filter(c => c.operation === "external-gateway.listener.set")).toEqual(Array.from({ length: 2 }, () => ({ operation: "external-gateway.listener.set", payload: { enabled: true } })));
    expect(root.textContent).toContain("http://127.0.0.1:32123/v1"); expect(root.all().some(e => e.attributes.get("aria-label") === "Retry listener on port 32123")).toBe(false);
    const before = client.calls.length; field(root, "Refresh available Packages").click(); await flush();
    expect(client.calls.slice(before)).toEqual([{ operation: "external-gateway.catalog.page", payload: {} }]);
  });
  it("refuses disabled expansion and confirms destructive Package removal with exact contracts", async () => {
    let enabled = false;
    const client = fake({ "external-gateway.catalog.page": () => ({ ...emptyPage, contracts: [{ ...contract, selected: enabled, hostEnabled: enabled }] }), "external-gateway.package.set": (p: { enabled: boolean }) => { enabled = p.enabled; return { ok: true, warning: null }; } });
    const root = setup(client); await flush(); expect(button(root, "Tools · dev.example.alpha").disabled).toBe(true);
    button(root, "Tools · dev.example.alpha").click(); expect(client.calls.filter(c => c.operation === "external-gateway.catalog.page")).toHaveLength(1);
    field(root, "Enable Multi access").click(); await flush();
    expect(client.calls.filter(c => c.operation === "external-gateway.package.set")).toEqual([{ operation: "external-gateway.package.set", payload: { provider: "dev.example.multi", enabled: true, contracts: [{ contract: "dev.example.alpha", major: 1 }] } }]);
    expect(button(root, "Tools · dev.example.alpha").disabled).toBe(false); field(root, "Enable Multi access").click();
    expect(root.textContent).toContain("Re-enabling does not restore"); expect(client.calls.filter(c => c.operation === "external-gateway.package.set")).toHaveLength(1);
    button(root, "Cancel disable").click(); field(root, "Enable Multi access").click(); button(root, "Disable Package").click(); await flush();
    expect(client.calls.filter(c => c.operation === "external-gateway.package.set").at(-1)).toEqual({ operation: "external-gateway.package.set", payload: { provider: "dev.example.multi", enabled: false, contracts: [{ contract: "dev.example.alpha", major: 1 }] } });
  });
  it("follows empty cursor pages without collecting prior pages and groups information beside refresh", async () => {
    const client = fake({ "external-gateway.catalog.page": (p: { cursor?: string }) => ({ ...emptyPage, contracts: p.cursor ? [contract] : [], nextCursor: p.cursor ? null : "empty-next" }) });
    const root = setup(client); await flush(); expect(root.textContent).toContain("No contracts on this page"); button(root, "Next contracts").click(); await flush();
    expect(client.calls.filter(c => c.operation === "external-gateway.catalog.page")).toEqual([{ operation: "external-gateway.catalog.page", payload: {} }, { operation: "external-gateway.catalog.page", payload: { cursor: "empty-next" } }]);
    const info = field(root, "About shared access"), refresh = field(root, "Refresh available Packages"); expect(info.parent).toBe(refresh.parent); expect(info.parent?.children).toEqual([info, refresh]); info.click(); expect(root.textContent).toContain("All tokens share");
  });
});


it("retains stale Host review, exact schema re-approval and mixed-Provider contract choices", async () => {
  const stale = { ...contract, hostEnabled: false };
  const off = { ...contract, contract: "dev.example.new", selected: false, hostEnabled: false };
  const changed = { ...tool, selected: true, reviewRequired: true };
  const client = fake({ "external-gateway.catalog.page": (p: { contract?: string }) => p.contract ? { ...emptyPage, tools: [changed] } : { ...emptyPage, contracts: [stale, off] }, "external-gateway.package.set": () => ({ ok: true, warning: null }), "external-gateway.tool.set": () => ({ ok: true, warning: null }) });
  const root = setup(client); await flush();
  expect(root.textContent).toContain("Host selection missing"); button(root, "Tools · dev.example.alpha").click(); await flush();
  expect(client.calls.filter(c => c.operation === "external-gateway.catalog.page" && (c.payload as { contract?: string }).contract)).toHaveLength(0);
  button(root, "Enable contract · dev.example.new").click(); await flush();
  button(root, "Review access · dev.example.alpha").click(); await flush();
  expect(client.calls.filter(c => c.operation === "external-gateway.package.set")).toEqual([
    { operation: "external-gateway.package.set", payload: { provider: "dev.example.multi", enabled: true, contracts: [{ contract: "dev.example.new", major: 1 }] } },
    { operation: "external-gateway.package.set", payload: { provider: "dev.example.multi", enabled: true, contracts: [{ contract: "dev.example.alpha", major: 1 }] } },
  ]);
  await unmount?.();
  const reviewed = fake({ "external-gateway.catalog.page": (p: { contract?: string }) => p.contract ? { ...emptyPage, tools: [changed] } : { ...emptyPage, contracts: [contract] }, "external-gateway.tool.set": () => ({ ok: true, warning: null }) });
  const next = setup(reviewed); await flush(); button(next, "Tools · dev.example.alpha").click(); await flush();
  expect(button(next, "Review and enable read read.one").attributes.get("aria-checked")).toBe("false"); button(next, "Review and enable read read.one").click(); await flush();
  expect(reviewed.calls.filter(c => c.operation === "external-gateway.tool.set")).toEqual([{ operation: "external-gateway.tool.set", payload: { contract: "dev.example.alpha", major: 1, provider: "dev.example.multi", port: "read.one", fingerprint: "a".repeat(64), enabled: true } }]);
});
it.each(["main", "settings"] as const)("keeps storage failures closed with cleanup in %s", async view => {
  vi.useFakeTimers(); const client = fake({ "external-gateway.catalog.page": () => ({ ...emptyPage, contracts: [contract] }), "external-gateway.status.get": () => ({ ...status, storageFailed: true }) });
  const root = setup(client, view); await flush(); expect(root.textContent).toContain("Storage failed. Access is closed");
  field(root, "Enable gateway listener").click(); field(root, "Enable Multi access").click();
  expect(client.calls.filter(c => c.operation.endsWith(".set"))).toHaveLength(0);
  await unmount?.(); expect(root.children).toHaveLength(0); expect(vi.getTimerCount()).toBe(0);
});

it("keeps explicit denial recovery available after a failed revoke with zero visible tokens", async () => {
  let current: Status = { ...status, tokens: [{ id: "one", label: "Manual", generation: 1 }] };
  const retry = deferred<{ ok: boolean; warning: null }>();
  const client = fake({
    "external-gateway.status.get": () => current,
    "external-gateway.token.revoke": async () => {
      current = { ...status, storageFailed: true };
      await client.emit("external-gateway.status.watch", current);
      throw createPackageRequestError({ packageError: { code: "external_gateway.storage_failed" } });
    },
    "external-gateway.disable-and-revoke-all": async () => {
      await retry.promise; current = status; return { ok: true, warning: null };
    },
  });
  const root = setup(client, "settings"); await flush();
  button(root, "Revoke Manual").click(); await flush();
  expect(root.textContent).toContain("Storage failed. Access is closed");
  expect(root.textContent).not.toContain("Revoke Manual");
  field(root, "Token label").value = "unsafe"; button(root, "Create token").click();
  field(root, "Enable gateway listener").click();
  button(root, "Disable and revoke all tokens").click();
  expect(client.calls.filter(c => c.operation === "external-gateway.disable-and-revoke-all")).toHaveLength(0);
  button(root, "Cancel revoke all").click();
  button(root, "Disable and revoke all tokens").click();
  const confirm = button(root, "Disable and revoke"); confirm.click(); confirm.click(); await flush();
  expect(client.calls.filter(c => c.operation === "external-gateway.disable-and-revoke-all")).toEqual([{ operation: "external-gateway.disable-and-revoke-all", payload: {} }]);
  expect(root.textContent).toContain("Storage failed. Access is closed");
  expect(client.calls.some(c => c.operation === "external-gateway.token.create" || c.operation.endsWith(".set"))).toBe(false);
  button(root, "Disable and revoke all tokens").click();
  retry.resolve({ ok: true, warning: null }); await flush();
  expect(root.textContent).not.toContain("Storage failed. Access is closed");
});

it("allows manual and managed revoke but not creation or rotation while storage is failed", async () => {
  const current: Status = { ...status, storageFailed: true, tokens: [{ id: "manual", label: "Manual", generation: 1 }, { id: "managed", label: "Managed", generation: 2, managed: true }] };
  const client = fake({ "external-gateway.status.get": () => current, "external-gateway.token.revoke": () => ({ ok: true, warning: null }) });
  const root = setup(client, "settings"); await flush();
  field(root, "Token label").value = "unsafe";
  for (const name of ["Create token", "Regenerate Manual", "Rotate Managed"]) button(root, name).click();
  button(root, "Revoke Manual").click(); await flush(); button(root, "Revoke Managed").click(); await flush();
  expect(client.calls.filter(c => c.operation.includes("token.") || c.operation === "external-gateway.setup.preview")).toEqual([
    { operation: "external-gateway.token.revoke", payload: { id: "manual" } },
    { operation: "external-gateway.token.revoke", payload: { id: "managed" } },
  ]);
});
