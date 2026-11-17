export function node<K extends keyof HTMLElementTagNameMap>(tag: K, text = ""): HTMLElementTagNameMap[K] {
  const element = document.createElement(tag);
  element.textContent = text;
  return element;
}
export function action(text: string, run: () => void, disabled = false): HTMLButtonElement {
  const button = node("button", text);
  button.type = "button";
  button.disabled = disabled;
  button.addEventListener("click", run);
  return button;
}
export function input(name: string, type = "text"): HTMLInputElement {
  const field = node("input");
  field.type = type;
  field.setAttribute("aria-label", name);
  field.setAttribute("autocomplete", "off");
  return field;
}
export function group(root: HTMLElement, title: string): HTMLElement {
  const section = node("section"); section.className = "group";
  const intro = node("div"); intro.className = "group-intro"; intro.append(node("h2", title));
  const body = node("div"); body.className = "group-body";
  section.append(intro, body); root.append(section); return body;
}
export function toggle(name: string, enabled: boolean, run: () => void, disabled = false): HTMLElement {
  const line = node("span"); line.className = "status-toggle";
  const button = action("", run, disabled); button.className = "switch";
  button.setAttribute("role", "switch"); button.setAttribute("aria-label", name); button.setAttribute("aria-checked", String(enabled));
  line.append(node("small", enabled ? "Enabled" : "Disabled"), button); return line;
}
export function icon(name: string, symbol: string, run: () => void, disabled = false): HTMLButtonElement {
  const button = action("", run, disabled); button.className = "icon";
  button.setAttribute("aria-label", name); button.setAttribute("title", name);
  const decoration = node("span", symbol); decoration.setAttribute("aria-hidden", "true"); button.append(decoration); return button;
}
