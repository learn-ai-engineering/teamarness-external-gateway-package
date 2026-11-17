import { describePackageError, isAbortError, packageErrorCode } from "@teamarness/package-ui-sdk";
import type { Contract, PackageContract, Tool } from "./contract.generated";
import type { Client } from "./settings";
import { action, icon, node, toggle } from "./dom";

type Requests = PackageContract["requests"];
type Mutation = <K extends "external-gateway.package.set" | "external-gateway.tool.set">(operation: K, payload: Requests[K]["request"]) => Promise<void>;
const POLL_MS = 5000;
const READ_MS = 5000;

/** Retains only the displayed summary page and one tool page, never an all-Provider catalog. */
export function catalogView(root: HTMLElement, client: Client, mutate: Mutation) {
  let alive = true;
  let generation = 0;
  let active: AbortController | undefined;
  let queued = false;
  let paused = false;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let deadline: ReturnType<typeof setTimeout> | undefined;
  let cursor: string | undefined;
  let nextCursor: string | null = null;
  let contracts: readonly Contract[] = [];
  let selected: string | undefined;
  let offset = 0;
  let nextOffset: number | null = null;
  let tools: readonly Tool[] = [];
  const error = node("p"); error.setAttribute("role", "alert");
  const message = node("p", "Loading Packages…"); message.setAttribute("aria-live", "polite");
  const rows = node("section");
  const pages = node("nav"); pages.setAttribute("aria-label", "Catalog pages");
  const heading = node("div"); heading.className = "section-title";
  const actions = node("div"); actions.className = "actions";
  const info = node("section"); info.hidden = true;
  info.append(node("p", "All tokens share the same Package and exact tool permissions. Package enable selects no tools; added tools stay off. Writes act as the local Workspace Person. Disable Package removes every saved tool choice, including other pages. This page refreshes every five seconds while open."));
  actions.append(icon("About shared access", "ⓘ", () => { info.hidden = !info.hidden; }), icon("Refresh available Packages", "↻", () => { void refresh(); }));
  heading.append(node("small", "Shared by every connection"), actions);
  const confirmation = node("dialog"); confirmation.setAttribute("aria-label", "Confirm Package disable");
  const closeConfirmation = () => { confirmation.close(); confirmation.replaceChildren(); };
  const confirmDisable = (provider: string | null, list: readonly Contract[], title: string) => {
    confirmation.replaceChildren(node("h3", `Disable ${title}?`), node("p", "This removes every saved tool permission for this Package, including other pages. Re-enabling does not restore them."), action("Cancel disable", closeConfirmation), action("Disable Package", () => { closeConfirmation(); void mutate("external-gateway.package.set", { provider, enabled: false, contracts: choices(list) }); }));
    confirmation.showModal();
  };
  root.append(heading, info, message, error, rows, pages, confirmation);
  const eligible = (c: Contract) => c.available && c.provider !== null && c.guideEligibility === "eligible";
  const choices = (list: readonly Contract[]) => list.map(c => ({ contract: c.contract, major: c.major }));
  function render() {
    rows.replaceChildren(); pages.replaceChildren();
    const groups = new Map<string, Contract[]>();
    for (const contract of contracts) {
      const key = contract.provider ?? contract.contract;
      groups.set(key, [...(groups.get(key) ?? []), contract]);
    }
    for (const group of groups.values()) {
      const first = group[0];
      const title = first.displayName ?? first.provider ?? "Unavailable Provider";
      const section = node("section"); section.className = "package";
      const heading = node("div"); heading.className = "package-row";
      heading.append(node("h3", title));
      const allowed = group.filter(eligible);
      const enabled = group.some(c => c.selected);
      heading.append(toggle(`Enable ${title} access`, enabled, () => {
        if (enabled) confirmDisable(first.provider, group, title);
        else if (first.provider) void mutate("external-gateway.package.set", { provider: first.provider, enabled: true, contracts: choices(allowed) });
      }, !enabled && allowed.length === 0));
      section.append(heading);
      for (const contract of group) {
        const row = node("section");
        row.append(node("h4", `${contract.contract} · major ${contract.major}`), node("p", contract.selected ? "Package selected" : "Package off"));
        if (!contract.available) row.append(node("p", "Provider unavailable. Retained choices can be removed."));
        else if (contract.guideEligibility !== "eligible") row.append(node("p", "Missing runtime guide: this contract cannot be enabled."));
        else {
          if (contract.selected && !contract.hostEnabled) row.append(node("p", "Host selection missing. Review and explicitly re-enable; it is never repaired automatically."));
          if (enabled && !contract.selected) row.append(action(`Enable contract · ${contract.contract}`, () => { void mutate("external-gateway.package.set", { provider: contract.provider, enabled: true, contracts: choices([contract]) }); }));
          const expand = action(`Tools · ${contract.contract}`, () => { selected = selected === contract.contract ? undefined : contract.contract; offset = 0; tools = []; navigate(); }, !contract.selected);
          expand.setAttribute("aria-expanded", String(selected === contract.contract && contract.selected)); row.append(expand);
          if (contract.selected && !contract.hostEnabled) row.append(action(`Review access · ${contract.contract}`, () => { void mutate("external-gateway.package.set", { provider: contract.provider, enabled: true, contracts: choices([contract]) }); }));
        }
        if (!contract.available && contract.provider === null && (contract.selected || contract.hostEnabled)) row.append(action(`Remove unavailable · ${contract.contract}`, () => { confirmDisable(contract.provider, [contract], contract.contract); }));
        if (contract.contract === selected && eligible(contract) && contract.selected && contract.hostEnabled) {
          row.append(node("p", "Guide included automatically when any operation is selected. All tokens share every selection; writes act as the local Workspace Person and can have lasting effects. Streams remain live until closed or denied."));
          for (const tool of tools) {
            const line = node("div");
            const shape = tool.kind === "command" ? "Write" : tool.kind === "event" ? "Event" : tool.delivery === "stream" ? "Streaming query" : "Read";
            line.append(node("strong", shape), node("code", tool.port), node("p", tool.reviewRequired ? "Review required: schema, Provider or major changed. Re-enable explicitly." : tool.selected ? "Selected" : "Off"));
            if (tool.eligible) {
              line.append(toggle(`${tool.reviewRequired ? "Review and enable" : tool.selected ? "Disable" : "Enable"} ${shape.toLowerCase()} ${tool.port}`, tool.selected && !tool.reviewRequired, () => { void mutate("external-gateway.tool.set", { contract: contract.contract, major: contract.major, provider: contract.provider!, port: tool.port, fingerprint: tool.fingerprint, enabled: tool.reviewRequired || !tool.selected }); }));
              if (tool.reviewRequired) line.append(action(`Disable ${tool.port}`, () => { void mutate("external-gateway.tool.set", { contract: contract.contract, major: contract.major, provider: contract.provider!, port: tool.port, fingerprint: tool.fingerprint, enabled: false }); }));
            } else line.append(node("p", tool.port === "runtime.guide" ? "Required guide" : "Unsupported shape"));
            row.append(line);
          }
          if (offset) row.append(action("First tools", () => { offset = 0; navigate(); }));
          if (nextOffset !== null) row.append(action("Next tools", () => { offset = nextOffset!; navigate(); }));
        }
        section.append(row);
      }
      rows.append(section);
    }
    if (!contracts.length) rows.append(node("p", "No contracts on this page."));
    if (cursor) pages.append(action("First contracts", () => { cursor = undefined; selected = undefined; tools = []; navigate(); }));
    if (nextCursor) pages.append(action("Next contracts", () => { cursor = nextCursor!; selected = undefined; tools = []; navigate(); }));
  }
  function navigate() {
    generation++;
    clearTimeout(timer);
    clearTimeout(deadline);
    if (active) { queued = true; active.abort(); }
    else void refresh();
  }
  async function refresh() {
    if (!alive || paused) return;
    if (active) return;
    clearTimeout(timer);
    const controller = new AbortController(); active = controller;
    const version = generation;
    const timeout = setTimeout(() => {
      controller.abort();
      if (alive && version === generation) {
        error.textContent = "Catalog read timed out. Refresh to try again.";
        message.textContent = "Catalog unavailable. Displayed choices may be stale.";
      }
    }, READ_MS);
    deadline = timeout;
    message.textContent = "Refreshing Packages…";
    try {
      let page;
      try { page = await client.request("external-gateway.catalog.page", cursor ? { cursor } : {}, { signal: controller.signal }); }
      catch (cause) {
        if (controller.signal.aborted || packageErrorCode(cause) !== "port_catalog_changed") throw cause;
        cursor = undefined; selected = undefined; offset = 0;
        page = await client.request("external-gateway.catalog.page", {}, { signal: controller.signal });
      }
      if (!alive || version !== generation || controller.signal.aborted) return;
      let detail: { tools: readonly Tool[]; nextOffset: number | null } = { tools: [], nextOffset: null };
      if (selected && page.contracts.some(c => c.contract === selected && eligible(c) && c.selected && c.hostEnabled)) {
        detail = await client.request("external-gateway.catalog.page", { contract: selected, offset }, { signal: controller.signal });
      }
      if (!alive || version !== generation || controller.signal.aborted) return;
      contracts = page.contracts; nextCursor = page.nextCursor; tools = detail.tools; nextOffset = detail.nextOffset;
      error.textContent = ""; message.textContent = "Catalog refreshed. New tools remain off."; render();
    } catch (cause) {
      if (alive && version === generation) {
        error.textContent = controller.signal.aborted || isAbortError(cause) ? "Catalog read timed out. Refresh to try again." : describePackageError(cause);
        message.textContent = "Catalog unavailable. Displayed choices may be stale.";
      }
    } finally {
      clearTimeout(timeout); active = undefined;
      if (alive && !paused) {
        if (queued) { queued = false; void refresh(); }
        else timer = setTimeout(() => { void refresh(); }, POLL_MS);
      }
    }
  }
  void refresh();
  return {
    pause: () => { paused = true; generation++; clearTimeout(timer); clearTimeout(deadline); active?.abort(); },
    resume: () => { paused = false; if (active) queued = true; else void refresh(); },
    stop: () => { alive = false; generation++; clearTimeout(timer); clearTimeout(deadline); active?.abort(); contracts = []; tools = []; closeConfirmation(); },
  };
}
