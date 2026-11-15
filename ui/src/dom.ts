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
