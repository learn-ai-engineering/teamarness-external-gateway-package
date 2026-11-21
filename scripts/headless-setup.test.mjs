import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, test } from "node:test";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const setup = join(here, "headless/external-gateway-setup");
const fakeHost = join(here, "headless/fake-host.py");
const W = "aad43be122caa818738f763c373ba820";

let roots = [];
afterEach(() => {
  for (const root of roots) rmSync(root, { recursive: true, force: true });
  roots = [];
});

function fixture(overrides = {}) {
  const root = mkdtempSync(join(tmpdir(), "gateway-setup-test-"));
  roots.push(root);
  const host = join(root, "host-app");
  writeFileSync(host, `#!/bin/sh\nexec python3 -I ${JSON.stringify(fakeHost)} "$@"\n`);
  chmodSync(host, 0o755);
  const tmx = join(root, "tmx");
  writeFileSync(tmx, `#!/bin/sh\necho '{"reachable":true,"endpoint":"http://127.0.0.1:40000","workspace":{"id":"${W}"}}'\n`);
  chmodSync(tmx, 0o755);
  const packageDir = join(root, "packages/dev.teamarness.external-gateway");
  mkdirSync(packageDir, { recursive: true });
  const state = {
    workspace: W,
    registered: null,
    tmx,
    log: [],
    faults: {},
    gateway: { cli: false, connection: "not-connected", listener: false },
    catalog: {
      "dev.teamarness.backlog": { provider: "dev.teamarness.backlog", enabled: false, selected_tools: [], ports: ["runtime.guide", "ticket.create", "ticket.get", "ticket.list"] },
      "dev.teamarness.conversations": { provider: "dev.teamarness.conversations", enabled: false, selected_tools: [], ports: ["channel.list", "message.post", "runtime.guide"] },
      "dev.teamarness.documents": { provider: "dev.teamarness.documents", enabled: false, selected_tools: [], ports: Array.from({ length: 40 }, (_, i) => `doc.port${i}`) },
    },
  };
  for (const [key, value] of Object.entries(overrides)) state[key] = typeof value === "object" && value && !Array.isArray(value) ? { ...state[key], ...value } : value;
  const statePath = join(root, "state.json");
  writeFileSync(statePath, JSON.stringify(state));
  return { root, host, packageDir, statePath };
}

function answers(f, extra = "") {
  const path = join(f.root, `answers-${Math.random().toString(16).slice(2)}.toml`);
  const base = {
    host_app: JSON.stringify(f.host),
    host_data_dir: JSON.stringify(join(f.root, "data")),
    package_dir: JSON.stringify(f.packageDir),
    accept_native_code_risks: "true",
    workspace: JSON.stringify(W),
    listener: "true",
    connect: "true",
    tools: '"all"',
  };
  const lines = [];
  for (const [k, v] of Object.entries(base)) {
    if (new RegExp(`^${k} *=`, "m").test(extra)) continue;
    if (k === "tools" && /^\[tools\]/m.test(extra)) continue;
    if (k === "workspace" && /^workspace_id *=/m.test(extra)) continue;
    lines.push(`${k} = ${v}`);
  }
  writeFileSync(path, `${lines.join("\n")}\n${extra}\n`);
  return path;
}

function run(f, args) {
  const result = spawnSync(setup, args, { encoding: "utf8", env: { ...process.env, FAKE_HOST_STATE: f.statePath } });
  return { ...result, state: JSON.parse(readFileSync(f.statePath, "utf8")) };
}

test("prints a template that parses with every key, and refuses to overwrite", () => {
  const f = fixture();
  const out = run(f, ["--print-template"]);
  assert.equal(out.status, 0);
  for (const key of ["host_app", "host_data_dir", "package_dir", "accept_native_code_risks", "workspace", "listener", "connect", "tools"]) {
    assert.match(out.stdout, new RegExp(`^${key} = `, "m"));
  }
  const path = join(f.root, "answers.toml");
  assert.equal(run(f, ["--print-template", path]).status, 0);
  const again = run(f, ["--print-template", path]);
  assert.equal(again.status, 2);
  assert.match(again.stderr, /refusing to overwrite/);
});

for (const [name, extra, message] of [
  ["an unknown key", "colour = true", /unknown key/],
  ["an empty workspace", 'workspace = " "', /workspace must be a Workspace name or its 32-character ID/],
  ["a name in workspace_id", 'workspace_id = "Headless"', /workspace_id must be the 32-character/],
  ["both workspace and workspace_id", `workspace = "Headless"\nworkspace_id = "${W}"`, /set workspace or workspace_id, not both/],
  ["a wrong type", 'listener = "yes"', /listener has the wrong type/],
  ["connect without listener", "listener = false", /connect = true needs listener = true/],
  ["a bad tools value", 'tools = "some"', /tools must be/],
]) {
  test(`refuses ${name} with exit 2 and no Host call`, () => {
    const f = fixture();
    const out = run(f, ["--answers", answers(f, extra)]);
    assert.equal(out.status, 2, out.stdout + out.stderr);
    assert.match(out.stderr, message);
    assert.deepEqual(out.state.log, []);
  });
}

test("refuses a needed install without consent, with exit 2 and no change", () => {
  const f = fixture();
  const out = run(f, ["--answers", answers(f, "accept_native_code_risks = false")]);
  assert.equal(out.status, 2);
  assert.match(out.stderr, /accept_native_code_risks = true/);
  assert.deepEqual(out.state.log, []);
});

test("a fresh run sets everything up in order, and a rerun changes nothing", () => {
  const f = fixture({ gateway: { slow_polls: 1 } });
  const file = answers(f);
  const first = run(f, ["--answers", file]);
  assert.equal(first.status, 0, first.stdout + first.stderr);
  assert.match(first.stdout, /^changed: package installed/m);
  assert.match(first.stdout, /^changed: cli installed/m);
  assert.match(first.stdout, /^changed: listener on, port 40000/m);
  assert.match(first.stdout, /^changed: connection connected/m);
  assert.match(first.stdout, /^changed: tools dev\.teamarness\.backlog \+3 -0 \(3 selected\)$/m);
  assert.match(first.stdout, /^changed: tools dev\.teamarness\.documents \+40 -0 \(40 selected\)$/m);
  assert.match(first.stdout, /^unchanged: check tmx reaches http:\/\/127\.0\.0\.1:40000/m);
  assert.match(first.stdout, /^summary: 7 changed, 1 unchanged, 0 stopped$/m);
  const order = first.state.log.map((entry) => entry.split(" ")[0]);
  assert.deepEqual(order.slice(0, 5), ["packages.install", "setup.apply", "listener.set", "setup.apply", "package.set"]);
  assert.ok(!first.state.catalog["dev.teamarness.backlog"].selected_tools.includes("runtime.guide"));
  assert.doesNotMatch(first.stdout + first.stderr, /plan-secret/);

  const rerun = run(f, ["--answers", file]);
  assert.equal(rerun.status, 0, rerun.stdout + rerun.stderr);
  assert.deepEqual(rerun.state.log, first.state.log, "a rerun must not mutate");
  assert.doesNotMatch(rerun.stdout, /^changed:/m);
  assert.match(rerun.stdout, /^summary: 0 changed, 8 unchanged, 0 stopped$/m);
});

test("a per-contract tool list is the whole selection: unlisted tools and Packages are turned off", () => {
  const f = fixture();
  const pre = JSON.parse(readFileSync(f.statePath, "utf8"));
  pre.catalog["dev.teamarness.conversations"].enabled = true;
  pre.catalog["dev.teamarness.conversations"].selected_tools = ["channel.list"];
  pre.catalog["dev.teamarness.backlog"].enabled = true;
  pre.catalog["dev.teamarness.backlog"].selected_tools = ["ticket.create", "ticket.get"];
  writeFileSync(f.statePath, JSON.stringify(pre));
  const file = answers(f, '[tools]\n"dev.teamarness.backlog" = ["ticket.get", "ticket.list"]');
  const out = run(f, ["--answers", file]);
  assert.equal(out.status, 0, out.stdout + out.stderr);
  assert.deepEqual(out.state.catalog["dev.teamarness.backlog"].selected_tools, ["ticket.get", "ticket.list"]);
  assert.equal(out.state.catalog["dev.teamarness.conversations"].enabled, false);
  assert.deepEqual(out.state.catalog["dev.teamarness.conversations"].selected_tools, []);
  assert.deepEqual(out.state.catalog["dev.teamarness.documents"].selected_tools, []);
  assert.match(out.stdout, /^changed: tools dev\.teamarness\.conversations turned off \(-1\)$/m);
  assert.match(out.stdout, /^unchanged: tools dev\.teamarness\.documents not exposed$/m);
  assert.match(out.stdout, /^changed: tools dev\.teamarness\.backlog \+1 -1 \(2 selected\)$/m);
  const removals = out.state.log.findIndex((e) => e.startsWith("package.set"));
  const firstAdd = out.state.log.findIndex((e) => e.startsWith("tool.set") && e.includes('"enabled": true'));
  assert.ok(removals < firstAdd, "removals come before additions");
  const rerun = run(f, ["--answers", file]);
  assert.equal(rerun.status, 0, rerun.stdout + rerun.stderr);
  assert.deepEqual(rerun.state.log, out.state.log, "a rerun must not mutate");
});

test("narrowing from all removes the tools no longer listed", () => {
  const f = fixture();
  const first = run(f, ["--answers", answers(f)]);
  assert.equal(first.status, 0, first.stdout + first.stderr);
  const out = run(f, ["--answers", answers(f, '[tools]\n"dev.teamarness.conversations" = ["channel.list"]')]);
  assert.equal(out.status, 0, out.stdout + out.stderr);
  assert.deepEqual(out.state.catalog["dev.teamarness.conversations"].selected_tools, ["channel.list"]);
  assert.equal(out.state.catalog["dev.teamarness.backlog"].enabled, false);
  assert.equal(out.state.catalog["dev.teamarness.documents"].enabled, false);
  assert.match(out.stdout, /^changed: tools dev\.teamarness\.documents turned off \(-40\)$/m);
  assert.match(out.stdout, /^changed: tools dev\.teamarness\.conversations \+0 -1 \(1 selected\)$/m);
});

test("an unavailable contract keeps its saved selection", () => {
  const f = fixture({ gateway: { cli: true, connection: "connected", listener: true } });
  const pre = JSON.parse(readFileSync(f.statePath, "utf8"));
  pre.registered = f.packageDir;
  Object.assign(pre.catalog["dev.teamarness.documents"], { available: false, enabled: true, selected_tools: ["doc.port1"] });
  writeFileSync(f.statePath, JSON.stringify(pre));
  const out = run(f, ["--answers", answers(f)]);
  assert.equal(out.status, 0, out.stdout + out.stderr);
  assert.deepEqual(out.state.catalog["dev.teamarness.documents"].selected_tools, ["doc.port1"]);
  assert.match(out.stdout, /^unchanged: tools dev\.teamarness\.documents unavailable; its saved selection is left as is$/m);
});

test("an uncertain connect stops at once with an inspect command, and makes no later change", () => {
  const f = fixture({ faults: { "setup.apply": "uncertain" }, gateway: { cli: true } });
  const out = run(f, ["--answers", answers(f)]);
  assert.equal(out.status, 1);
  assert.match(out.stdout, /^stopped: connection outcome unknown \(package_error: external_gateway\.storage_failed\) — inspect with: echo '\{\}' \| <host_app> --data-dir <host_data_dir> package request .*--operation external-gateway\.setup\.status .*, then rerun setup$/m);
  assert.equal(out.state.log.at(-1), "setup.apply");
  assert.ok(!out.state.log.includes("package.set"));
});

test("a failed read stops with exit 1 and no outcome-unknown wording", () => {
  const f = fixture({ faults: { "status.get": "fail" }, registered: "x" });
  const pre = JSON.parse(readFileSync(f.statePath, "utf8"));
  pre.registered = f.packageDir;
  pre.gateway.cli = true;
  writeFileSync(f.statePath, JSON.stringify(pre));
  const out = run(f, ["--answers", answers(f)]);
  assert.equal(out.status, 1);
  assert.match(out.stdout, /^stopped: listener external-gateway\.status\.get failed \(package_error: external_gateway\.busy\)$/m);
  assert.deepEqual(out.state.log, []);
});

test("a tool that needs review stops without selecting tools in that contract", () => {
  const f = fixture({ gateway: { cli: true, connection: "connected", listener: true } });
  const pre = JSON.parse(readFileSync(f.statePath, "utf8"));
  pre.registered = f.packageDir;
  pre.catalog["dev.teamarness.backlog"].review = ["ticket.get"];
  writeFileSync(f.statePath, JSON.stringify(pre));
  const out = run(f, ["--answers", answers(f)]);
  assert.equal(out.status, 1);
  assert.match(out.stdout, /^stopped: tools dev\.teamarness\.backlog needs review for: ticket\.get/m);
  assert.ok(!out.state.log.some((entry) => entry.startsWith("tool.set") && entry.includes("backlog")));
});

test("a connection that needs repair stops without repairing", () => {
  const f = fixture({ gateway: { cli: true, connection: "needs-repair", repair: "publication", listener: true } });
  const pre = JSON.parse(readFileSync(f.statePath, "utf8"));
  pre.registered = f.packageDir;
  writeFileSync(f.statePath, JSON.stringify(pre));
  const out = run(f, ["--answers", answers(f)]);
  assert.equal(out.status, 1);
  assert.match(out.stdout, /^stopped: connection needs repair \(publication\)/m);
  assert.deepEqual(out.state.log, []);
});

test("a cli issue stops before installing tmx", () => {
  const f = fixture({ gateway: { cli_issue: "external_gateway.output_unsafe" } });
  const out = run(f, ["--answers", answers(f)]);
  assert.equal(out.status, 1);
  assert.match(out.stdout, /^stopped: cli external_gateway\.output_unsafe/m);
  assert.deepEqual(out.state.log, ["packages.install"]);
});

test("a registration from another directory stops without installing", () => {
  const f = fixture({ registered: "/elsewhere/dev.teamarness.external-gateway" });
  const out = run(f, ["--answers", answers(f)]);
  assert.equal(out.status, 1);
  assert.match(out.stdout, /^stopped: package registered from a different directory/m);
  assert.deepEqual(out.state.log, []);
});

test("an unknown contract or port is invalid input", () => {
  const f = fixture();
  const pre = JSON.parse(readFileSync(f.statePath, "utf8"));
  pre.registered = f.packageDir;
  pre.gateway = { cli: true, connection: "connected", listener: true };
  writeFileSync(f.statePath, JSON.stringify(pre));
  const missing = run(f, ["--answers", answers(f, '[tools]\n"dev.teamarness.nope" = "all"')]);
  assert.equal(missing.status, 2);
  assert.match(missing.stdout, /^stopped: tools contract dev\.teamarness\.nope is not available/m);
  const port = run(f, ["--answers", answers(f, '[tools]\n"dev.teamarness.backlog" = ["ticket.nope"]')]);
  assert.equal(port.status, 2);
  assert.match(port.stdout, /^stopped: tools dev\.teamarness\.backlog has no eligible port\(s\): ticket\.nope/m);
  assert.deepEqual(port.state.log, [], "invalid tools input changes nothing");
});

test("a Workspace name resolves to its ID through the Host's Workspace list", () => {
  const f = fixture({ workspaces: [{ id: "b".repeat(32), name: "Other" }, { id: W, name: "Headless" }] });
  const out = run(f, ["--answers", answers(f, 'workspace = "Headless"')]);
  assert.equal(out.status, 0, out.stdout + out.stderr);
  assert.match(out.stdout, new RegExp(`^workspace: Headless ${W}$`, "m"));
  assert.match(out.stdout, /^summary: 7 changed, 1 unchanged, 0 stopped$/m);
});

test("an ID is checked against the Host's Workspace list", () => {
  const f = fixture();
  const out = run(f, ["--answers", answers(f)]);
  assert.equal(out.status, 0, out.stdout + out.stderr);
  assert.match(out.stdout, new RegExp(`^workspace: Headless ${W}$`, "m"));
  const missing = run(f, ["--answers", answers(f, `workspace = "${"c".repeat(32)}"`)]);
  assert.equal(missing.status, 2);
  assert.match(missing.stdout, /^stopped: workspace 'c{32}' matches no Workspace on this Host; list them with: tm workspace list/m);
});

test("a name that matches no Workspace, or more than one, stops with exit 2 before any change", () => {
  const f = fixture({ workspaces: [{ id: W, name: "Team" }, { id: "b".repeat(32), name: "Team" }, { id: "e".repeat(32), name: "Solo" }] });
  const none = run(f, ["--answers", answers(f, 'workspace = "Nope"')]);
  assert.equal(none.status, 2);
  assert.match(none.stdout, /^stopped: workspace 'Nope' matches no Workspace on this Host/m);
  assert.deepEqual(none.state.log, []);
  const two = run(f, ["--answers", answers(f, 'workspace = "Team"')]);
  assert.equal(two.status, 2);
  assert.match(two.stdout, new RegExp(`^stopped: workspace 'Team' matches more than one Workspace \\(${W}, ${"b".repeat(32)}\\); set workspace to one of these IDs$`, "m"));
  assert.deepEqual(two.state.log, []);
});

test("an older answers file with workspace_id still works", () => {
  const f = fixture();
  const out = run(f, ["--answers", answers(f, `workspace_id = "${W}"`)]);
  assert.equal(out.status, 0, out.stdout + out.stderr);
  assert.match(out.stdout, new RegExp(`^workspace: Headless ${W}$`, "m"));
});

function templateTools() {
  const f = fixture();
  const text = run(f, ["--print-template"]).stdout;
  const block = text.slice(text.indexOf("# [tools]\n"));
  const tools = {};
  for (const m of block.matchAll(/^# "([^"]+)" = (\[.*\])$/gm)) tools[m[1]] = JSON.parse(m[2]);
  return { text, tools };
}

test("the template's commented [tools] table uncomments into a valid answers file that selects every listed tool", () => {
  const { text, tools } = templateTools();
  assert.deepEqual(Object.keys(tools).sort(), ["dev.teamarness.backlog", "dev.teamarness.conversations", "dev.teamarness.documents", "dev.teamarness.exhibits"]);
  for (const ports of Object.values(tools)) {
    assert.ok(!ports.includes("runtime.guide"));
    assert.deepEqual(ports, [...ports].sort());
  }
  const catalog = {};
  for (const [c, ports] of Object.entries(tools)) catalog[c] = { provider: c, enabled: false, selected_tools: [], ports: ["runtime.guide", ...ports] };
  const f = fixture({ catalog });
  const uncommented = text.replace(/^tools = "all"\n/m, "").replace(/^# (\[tools\]|"dev\.)/gm, "$1");
  const path = join(f.root, "template-answers.toml");
  writeFileSync(path, uncommented.replace(/^(host_app|host_data_dir|package_dir|workspace|accept_native_code_risks) = .*$/gm, (line, k) => `${k} = ${{
    host_app: JSON.stringify(f.host), host_data_dir: JSON.stringify(join(f.root, "data")), package_dir: JSON.stringify(f.packageDir),
    workspace: JSON.stringify(W), accept_native_code_risks: "true" }[k]}`));
  const out = run(f, ["--answers", path]);
  assert.equal(out.status, 0, out.stdout + out.stderr);
  for (const [c, ports] of Object.entries(tools)) assert.deepEqual([...out.state.catalog[c].selected_tools].sort(), ports);
});

// The template lists the Provider tools by name. Set TEAMARNESS_CHECKOUT to a Teamarness checkout
// to compare the list with the Providers' ports.json, so a Provider change shows up as a failure.
const checkout = process.env.TEAMARNESS_CHECKOUT;
test("the template's tool list matches the Provider contracts", { skip: checkout ? false : "set TEAMARNESS_CHECKOUT to a Teamarness checkout to compare" }, () => {
  const expected = {};
  const packages = join(checkout, "packages");
  for (const name of readdirSync(packages)) {
    const file = join(packages, name, "ports.json");
    if (!existsSync(file)) continue;
    for (const contract of JSON.parse(readFileSync(file, "utf8")).contracts) {
      const ports = (Array.isArray(contract.ports) ? contract.ports.map((p) => p.name) : Object.keys(contract.ports)).filter((p) => p !== "runtime.guide");
      expected[contract.identity] = ports.sort();
    }
  }
  assert.deepEqual(templateTools().tools, expected);
});
