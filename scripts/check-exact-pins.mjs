import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
const expected = {
  "rfd": "=0.16.1",
  "objc2": "=0.6.5",
  "objc2-app-kit": "=0.3.2",
  "base64": "=0.23.1",
  "jsonschema": "=0.56.0",
  "mime": "=0.3.17",
  "semver": "=1.0.28",
  "serde": "=1.0.229",
  "serde_json": "=1.0.151",
  "sha2": "=0.11.0",
  "tempfile": "=3.27.0",
  "tokio": "=1.53.1",
  "getrandom": "=0.4.3",
  "subtle": "=2.6.1",
  "rustix": "=1.1.4",
  "hyper": "=1.11.1",
  "hyper-util": "=0.1.20",
  "http-body-util": "=0.1.5",
  "http": "=1.5.0",
  "bytes": "=1.12.1"
};
const cargo = readFileSync(new URL("../Cargo.toml", import.meta.url), "utf8");
const actual = Object.fromEntries([...cargo.matchAll(/^([\w-]+) = (?:\{ version = )?"(=[^"]+)"/gm)].map((m) => [m[1],m[2]]));
assert.deepEqual(actual, expected);
for (const path of ["../package.json", "../ui/package.json"]) {
 const manifest=JSON.parse(readFileSync(new URL(path,import.meta.url),"utf8"));
 for(const section of ["dependencies","devDependencies"])
  for(const pin of Object.values(manifest[section]??{})) assert.match(pin,/^(?:\d+\.\d+\.\d+(?:-[\w.-]+)?|file:.*)$/);
}
assert.match(cargo,/rust-version = "1\.99\.1"/);
assert.match(readFileSync(new URL("../rust-toolchain.toml",import.meta.url),"utf8"),/channel = "1\.99\.1"/);
console.log("Exact direct dependency and toolchain pins match.");
