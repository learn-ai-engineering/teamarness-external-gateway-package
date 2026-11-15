import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
const read=(p)=>readFileSync(new URL(p,import.meta.url),"utf8");
const ui=JSON.parse(read("../ui-contract.json"));
assert.deepEqual(Object.keys(ui.requests).sort(),["status.get","catalog.page","listener.set","port.set","package.set","tool.set","token.create","token.regenerate","token.revoke","disable-and-revoke-all"].map(s=>`external-gateway.${s}`).sort());
assert.deepEqual(Object.keys(ui.subscriptions),["external-gateway.status.watch"]);
for(const request of [...Object.values(ui.requests),...Object.values(ui.subscriptions)])assert.equal(request.request.additionalProperties,false);
const manifest=JSON.parse(read("../teamarness.package.json"));
assert.deepEqual(manifest.capabilities,[{identity:"teamarness.workspace-context",major:1,minMinor:0,maxMinor:0},{identity:"teamarness.package-ports",major:3,minMinor:0,maxMinor:0},{identity:"teamarness.membership",major:1,minMinor:0,maxMinor:0}]);
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
const http=JSON.parse(read("../contracts/http-v1.schema.json"));
assert.equal(http["x-limits"].MAX_HOST_CALLS,12);
assert.equal(http.$defs.manifest.properties.limits.properties.outstandingSdkInvocations.const,12);
for(const name of ["call","result","error","manifest","contracts","schema","guide","onboarding"]){
 assert.equal(http.$defs[name].additionalProperties,false);
 assert.deepEqual([...http.$defs[name].required].sort(),Object.keys(http.$defs[name].properties).sort());
}
for(const [code,status] of Object.entries(http["x-gateway-status"]))assert.ok(docs.includes(`| \`${code}\` | ${status} |`));
for(const example of JSON.parse(read("../contracts/http-v1.examples.json")))assert.ok(http.$defs[example.definition]);
assert.ok(docs.includes("Every SDK terminal releases the local slot"));
assert.ok(docs.includes("25-second HTTP budget"));
console.log("HTTP envelope definitions, examples, documented status codes and invocation budgets match.");

const onboarding=read("../docs/onboarding.md");
assert.ok(onboarding.includes(`${limits.MAX_HOST_CALLS} outstanding gateway SDK operations`));
assert.ok(onboarding.includes(`${limits.TOOL_WAIT_TIMEOUT}-second upper HTTP observation budget`));
assert.ok(onboarding.includes("at most one delayed retry"));
assert.ok(onboarding.includes("Never retry an uncertain outcome"));
assert.ok(onboarding.includes("<BEARER>"));
const catalogUi=read("../ui/src/catalog.ts");
assert.ok(catalogUi.includes(`const POLL_MS = ${limits.CATALOG_POLL_INTERVAL*1000};`));
assert.ok(catalogUi.includes(`const READ_MS = ${limits.CATALOG_READ_TIMEOUT*1000};`));
console.log("Shared onboarding and settings deadlines match the accepted budgets.");

const cursor=http.$defs.contracts.properties.nextCursor.anyOf.find(s=>s.type==="string");
assert.equal(cursor.maxLength,limits.MAX_HTTP_CURSOR_CHARS);
assert.equal(cursor.minLength,1);
assert.equal(cursor.pattern,"^[A-Za-z0-9_-]+$");
assert.equal(limits.MAX_HTTP_CURSOR_CHARS,Math.ceil(limits.MAX_HOST_CURSOR_BYTES*4/3));
assert.equal(ui.requests["external-gateway.catalog.page"].request.properties.cursor.maxLength,limits.MAX_HOST_CURSOR_BYTES);
assert.equal(limits.MAX_TARGET_BYTES,4096);
console.log("HTTP cursor envelope bounds agree; Host/UI cursor and request-target budgets remain unchanged.");
