/** Package-owned styles; the mount removes this sheet on unmount. */
export const PACKAGE_STYLES = `
  :host { color: var(--teamarness-color-text); background: var(--teamarness-color-background); font-family: var(--teamarness-font-body); }
  * { box-sizing: border-box; }
  article { display: grid; gap: 1rem; padding: 1.25rem; max-width: 64rem; }
  h1, h2, h3, h4, p { margin: 0; }
  section, fieldset { display: grid; gap: .7rem; min-width: 0; }
  fieldset { border: 0; margin: 0; padding: 0; }
  fieldset:disabled { opacity: .65; }
  nav, .package > section > div { display: flex; flex-wrap: wrap; gap: .5rem; align-items: center; }
  .package { border: 1px solid var(--teamarness-color-border); border-radius: var(--teamarness-radius-panel); padding: 1rem; background: var(--teamarness-color-panel); }
  .package > section { border-top: 1px solid var(--teamarness-color-border); padding-top: .7rem; }
  button, input { font: inherit; color: inherit; background: var(--teamarness-color-background); border: 1px solid var(--teamarness-color-border); border-radius: var(--teamarness-radius-control); padding: .55rem .75rem; }
  button { cursor: pointer; width: fit-content; }
  button:disabled { cursor: default; opacity: .6; }
  button[aria-current] { border-color: var(--teamarness-color-accent); }
  button:focus-visible, input:focus-visible { outline: 3px solid var(--teamarness-color-focus); outline-offset: 2px; }
  input { max-width: 28rem; }
  code, pre { font-family: var(--teamarness-font-mono); overflow-wrap: anywhere; white-space: pre-wrap; }
  pre { margin: 0; line-height: 1.6; }
  [role=alert] { color: var(--teamarness-color-danger); }
  [aria-label="One-time token"]:not(:empty) { border: 1px solid var(--teamarness-color-accent); border-radius: var(--teamarness-radius-panel); padding: 1rem; }
`;
