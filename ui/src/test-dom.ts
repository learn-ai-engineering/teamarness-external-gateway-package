// Minimal browser boundary for client-seam tests; no Package behavior is implemented here.
export class Element {
  children: Element[] = [];
  attributes = new Map<string, string>();
  parent?: Element;
  className = "";
  value = "";
  type = "";
  disabled = false;
  checked = false;
  min = "";
  max = "";
  maxLength = 0;
  readOnly = false;
  adoptedStyleSheets: CSSStyleSheet[] = [];
  private text = "";
  private listeners = new Map<string, (() => void)[]>();
  constructor(readonly tagName: string) {}
  set textContent(value: string) { this.replaceChildren(); this.text = value; }
  get textContent(): string { return this.text + this.children.map(c => c.textContent).join(" "); }
  append(...children: Element[]): void {
    for (const child of children) { child.parent = this; this.children.push(child); }
  }
  replaceChildren(...children: Element[]): void {
    this.text = "";
    for (const child of this.children) child.parent = undefined;
    this.children = [];
    this.append(...children);
  }
  remove(): void {
    if (this.parent) this.parent.children = this.parent.children.filter(c => c !== this);
    this.parent = undefined;
  }
  setAttribute(name: string, value: string): void { this.attributes.set(name, value); }
  addEventListener(name: string, listener: () => void): void {
    this.listeners.set(name, [...(this.listeners.get(name) ?? []), listener]);
  }
  click(): void { if (!this.disabled) for (const listener of this.listeners.get("click") ?? []) listener(); }
  all(): Element[] { return [this, ...this.children.flatMap(c => c.all())]; }
}
class Sheet { replaceSync(): void { /* browser style boundary */ } }
export function installDom(): void {
  globalThis.document = { createElement: (tag: string) => new Element(tag) } as unknown as Document;
  globalThis.CSSStyleSheet = Sheet as unknown as typeof CSSStyleSheet;
}
export function button(root: Element, name: string): Element {
  const node = root.all().find(e => e.tagName === "button" && e.textContent === name);
  if (!node) throw new Error(`Missing button: ${name}`);
  return node;
}
export function field(root: Element, name: string): Element {
  const node = root.all().find(e => e.attributes.get("aria-label") === name);
  if (!node) throw new Error(`Missing field: ${name}`);
  return node;
}
export async function flush(): Promise<void> { for (let n = 0; n < 30; n++) await Promise.resolve(); }
export function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(r => { resolve = r; });
  return { promise, resolve };
}
