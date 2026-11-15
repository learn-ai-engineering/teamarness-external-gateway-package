import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
const read=(p)=>readFileSync(new URL(p,import.meta.url),"utf8");
const ui=JSON.parse(read("../ui-contract.json"));
assert.deepEqual(Object.keys(ui.requests).sort(),["status.get","catalog.page","listener.set","port.set","package.set","tool.set","token.create","token.regenerate","token.revoke","disable-and-revoke-all"].map(s=>`external-gateway.${s}`).sort());
assert.deepEqual(Object.keys(ui.subscriptions),["external-gateway.status.watch"]);
for(const request of [...Object.values(ui.requests),...Object.values(ui.subscriptions)])assert.equal(request.request.additionalProperties,false);
const manifest=JSON.parse(read("../teamarness.package.json"));
assert.deepEqual(manifest.capabilities,[{identity:"teamarness.workspace-context",major:1,minMinor:0,maxMinor:0},{identity:"teamarness.package-ports",major:3,minMinor:0,maxMinor:0}]);
assert.deepEqual(manifest.ui.views,["settings"]);
assert.equal(manifest.consumedPorts,undefined);assert.equal(manifest.providedPorts,undefined);assert.equal(manifest.activation,undefined);
const limits=JSON.parse(read("../contracts/http-v1.schema.json"))["x-limits"];
const rust=read("../backend/src/limits.rs");const docs=read("../docs/http-api.md");
for(const [name,value] of Object.entries(limits)){
 assert.ok(rust.includes(`pub const ${name}: usize = ${value};`));
 assert.ok(docs.includes(`| \`${name}\` | ${value} |`));
}
execFileSync("npm",["run","contract-types","--workspace","generated-package-ui","--","--check"],{stdio:"inherit"});
console.log("Closed UI contract, generated types, declared capabilities and planned HTTP budgets match.");
