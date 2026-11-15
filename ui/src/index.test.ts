import {
  createFakePackageClient,
  createPackageRequestError,
  createTestMountContext,
} from "@teamarness/package-ui-sdk/testing";
import { describe, expect, it } from "vitest";

import { mount } from "./index";

class FakeCSSStyleSheet {
  cssText = "";

  replaceSync(css: string): void {
    this.cssText = css;
  }
}

class FakeElement {
  readonly children: FakeElement[] = [];
  readonly attributes = new Map<string, string>();
  parent: FakeElement | undefined;
  className = "";
  textContent = "";
  adoptedStyleSheets: CSSStyleSheet[] = [];

  constructor(readonly tagName: string) {}

  append(...children: FakeElement[]): void {
    for (const child of children) {
      child.parent = this;
      this.children.push(child);
    }
  }

  remove(): void {
    if (!this.parent) return;
    const index = this.parent.children.indexOf(this);
    if (index >= 0) this.parent.children.splice(index, 1);
    this.parent = undefined;
  }

  setAttribute(name: string, value: string): void {
    this.attributes.set(name, value);
  }
}

class FakeDocument {
  createElement(tagName: string): FakeElement {
    return new FakeElement(tagName);
  }
}

globalThis.document = new FakeDocument() as unknown as Document;
globalThis.CSSStyleSheet = FakeCSSStyleSheet as unknown as typeof CSSStyleSheet;

function texts(root: FakeElement): string[] {
  return root.children[0]?.children.map(({ textContent }) => textContent) ?? [];
}

function context(root: FakeElement, client: ReturnType<typeof createFakePackageClient>) {
  return createTestMountContext({
    root,
    client: client.client,
    displayName: "External Gateway",
  });
}

describe("External Gateway status UI", () => {
  it("mounts synchronously and shows disabled status without starting a listener", async () => {
    const root = new FakeElement("shadow-root");
    const client = createFakePackageClient({
      requests: {
        "external-gateway.status.get": () => ({
          desiredEnabled: false, tokens: [], running: false, port: null, storageFailed: false, warning: null, selectedContracts: 0, selectedTools: 0,
        }),
      },
    });

    const unmount = mount(context(root, client));
    expect(texts(root)).toEqual(["External Gateway", "Loading…", ""]);
    expect(root.adoptedStyleSheets).toHaveLength(1);
    await Promise.resolve();
    await Promise.resolve();

    expect(client.calls).toEqual([{ operation: "external-gateway.status.get", payload: {} }]);
    expect(texts(root)).toEqual([
      "External Gateway",
      "Read-only access disabled",
      "0 tokens. HTTP listener stopped.",
    ]);

    unmount();
  });

  it("shows the Package error code when status fails", async () => {
    const root = new FakeElement("shadow-root");
    const client = createFakePackageClient({
      requests: {
        "external-gateway.status.get": () => {
          throw createPackageRequestError({
            packageError: { code: "external_gateway.storage_failed", retryable: true },
          });
        },
      },
    });

    const unmount = mount(context(root, client));
    await Promise.resolve();
    await Promise.resolve();

    expect(texts(root)).toContain("external_gateway.storage_failed");
    const alert = root.children[0]?.children.at(-1);
    expect(alert?.attributes.get("role")).toBe("alert");

    unmount();
  });

  it("aborts the pending status request and removes content on unmount", async () => {
    const root = new FakeElement("shadow-root");
    let observed: AbortSignal | undefined;
    const client = createFakePackageClient({
      requests: {
        "external-gateway.status.get": (_payload, signal) => {
          observed = signal;
          return new Promise<never>(() => undefined);
        },
      },
    });

    const unmount = mount(context(root, client));
    await Promise.resolve();
    unmount();

    expect(observed?.aborted).toBe(true);
    expect(root.children).toHaveLength(0);
    expect(root.adoptedStyleSheets).toHaveLength(0);
  });

  it("unmount is idempotent", () => {
    const root = new FakeElement("shadow-root");
    const client = createFakePackageClient({
      requests: { "external-gateway.status.get": () => ({ desiredEnabled: false, tokens: [], running: false, port: null, storageFailed: false, warning: null, selectedContracts: 0, selectedTools: 0 }) },
    });

    const unmount = mount(context(root, client));
    unmount();
    unmount();

    expect(root.children).toHaveLength(0);
  });
});
