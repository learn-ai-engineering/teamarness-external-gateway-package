/** Revised A's flat About-style groups, using only Shell-supplied semantic tokens. */
export const PACKAGE_STYLES = `
  :host { color: var(--teamarness-color-text); background: var(--teamarness-color-background); font-family: var(--teamarness-font-body); font-size: 14px; }
  * { box-sizing: border-box; }
  [hidden] { display: none !important; }
  article { width: 100%; max-width: 56rem; margin: 0 auto; padding: 2rem clamp(1.25rem, 4vw, 3rem); }
  h1, h2, h3, h4, p { margin: 0; }
  h1 { font-size: 2rem; font-weight: 650; letter-spacing: -.04rem; }
  h2, h3 { font-size: 1rem; font-weight: 600; }
  h4 { font-size: .85rem; font-weight: 500; }
  p { line-height: 1.55; }
  small, header > p { color: var(--teamarness-color-text-muted); font-size: .85rem; }
  header { margin-bottom: 2rem; }
  header > p { margin-top: .6rem; }
  .heading, .actions, .listener-line, .package-row, .section-title { display: flex; align-items: center; justify-content: space-between; gap: .75rem; }
  .actions { justify-content: flex-end; gap: .3rem; }
  .group { display: grid; grid-template-columns: minmax(7.5rem, 1fr) minmax(0, 2.2fr); gap: 1.25rem 2.6rem; padding: 1.8rem 0; border-top: 1px solid var(--teamarness-color-border); }
  .group:first-child { border-top: 0; padding-top: .3rem; }
  .group-body { min-width: 0; }
  .group-body > p, .group-body > section > p { margin-bottom: .7rem; }
  fieldset { border: 0; margin: 0; padding: 0; min-width: 0; }
  fieldset:disabled { opacity: .65; }
  button, input { font: inherit; color: inherit; background: transparent; }
  button { border: 0; padding: .4rem; font-size: .85rem; font-weight: 500; cursor: pointer; width: fit-content; }
  button:hover { color: var(--teamarness-color-accent); }
  button:disabled { cursor: default; opacity: .45; }
  button:focus-visible, input:focus-visible, summary:focus-visible { outline: 2px solid var(--teamarness-color-focus); outline-offset: 3px; }
  .icon { display: inline-flex; align-items: center; justify-content: center; width: 2rem; height: 2rem; font-size: 1.3rem; padding: .3rem; }
  .status-toggle { display: inline-flex; align-items: center; gap: .6rem; }
  .status-toggle small { min-width: 3rem; text-align: right; font-size: .78rem; }
  .switch { position: relative; padding: 0; min-width: 32px; width: 32px; height: 18px; border: 1px solid var(--teamarness-color-text-muted); border-radius: 20px; }
  .switch::after { content: ""; position: absolute; top: 3px; left: 3px; width: 10px; height: 10px; border-radius: 50%; background: var(--teamarness-color-text-muted); }
  .switch[aria-checked=true] { background: var(--teamarness-color-accent); border-color: var(--teamarness-color-accent); }
  .switch[aria-checked=true]::after { left: 17px; background: var(--teamarness-color-background); }
  .listener-line { align-items: flex-start; }
  .listener-line code, .listener-line small { display: block; margin-top: .6rem; }
  .package { border-top: 1px solid var(--teamarness-color-border); padding: .6rem 0; }
  .package:first-child { border: 0; }
  .package-row { min-height: 2.5rem; }
  .package > section { margin-top: .7rem; }
  .package > section > p { color: var(--teamarness-color-text-muted); font-size: .8rem; margin: .4rem 0; }
  .package > section > div { display: flex; align-items: center; justify-content: space-between; flex-wrap: wrap; gap: .6rem; padding: .6rem 0; border-top: 1px solid var(--teamarness-color-border); }
  .package > section > div > p { font-size: .75rem; color: var(--teamarness-color-text-muted); }
  .package strong { font-size: .75rem; font-weight: 500; color: var(--teamarness-color-text-muted); }
  input { border: 1px solid var(--teamarness-color-border); padding: .6rem; max-width: 100%; }
  input[type=checkbox] { width: fit-content; }
  [aria-label="Managed setup"] > details > fieldset, .group-body > fieldset:not(:first-child) { display: grid; gap: .7rem; }
  [aria-label="Managed setup"] > details > section { margin: .8rem 0; }
  code, pre { font-family: var(--teamarness-font-mono); overflow-wrap: anywhere; white-space: pre-wrap; font-size: .8rem; }
  pre { line-height: 1.7; max-height: 60vh; overflow: auto; }
  .long-details { margin-top: 1rem; font-size: .85rem; color: var(--teamarness-color-text-muted); }
  .long-details > p { margin-top: .8rem; }
  summary { cursor: pointer; color: var(--teamarness-color-accent); padding: .5rem 0; }
  [role=alert], [role=alert] p { color: var(--teamarness-color-danger); }
  [role=alert]:empty { display: none; }
  [aria-label="One-time token"]:not(:empty) { border: 1px solid var(--teamarness-color-accent); padding: 1rem; margin: 1rem 0; }
  dialog { width: min(36rem, calc(100vw - 2rem)); max-height: 85vh; overflow: auto; color: var(--teamarness-color-text); background: var(--teamarness-color-background); border: 1px solid var(--teamarness-color-border); padding: 2rem; }
  dialog p { margin: 1rem 0; }
  dialog::backdrop { background: #0005; }
  @media (max-width: 650px) { .group { grid-template-columns: 1fr; gap: .8rem; } article { padding: 1.5rem 1.25rem; } }
`;
