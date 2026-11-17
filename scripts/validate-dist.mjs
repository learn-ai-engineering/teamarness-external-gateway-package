#!/usr/bin/env node
// Applies Package Admission's rules to the assembled Package Directory, and smoke-launches it.
//
// This is the same verdict the Package Host reaches when it admits the Package, so a build that
// passes here is one Teamarness will accept.

import { smokePicker } from "./picker-smoke.mjs";
import { smokeCli } from "./cli-smoke.mjs";
import { spawnSync } from "node:child_process";
import { readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const projectRoot = dirname(dirname(fileURLToPath(import.meta.url)));
const manifest = JSON.parse(await readFile(join(projectRoot, "teamarness.package.json"), "utf8"));

const result = spawnSync(
  "cargo",
  [
    "run",
    "--quiet",
    "--locked",
    "-p",
    "teamarness-package-contract",
    "--bin",
    "teamarness-package-validate",
    "--",
    "directory",
    join("dist", manifest.identity),
    "--smoke",
  ],
  { cwd: projectRoot, stdio: "inherit" },
);

if (result.error) {
  process.stderr.write(`${result.error.message}\n`);
  process.exitCode = 2;
} else {
  process.exitCode = result.status ?? 2;
}

if (result.status === 0 && !result.error) {
  smokePicker(join(projectRoot, "dist", manifest.identity, manifest.backends[0].path));
  await smokeCli(join(projectRoot, "dist", manifest.identity, manifest.backends[0].path), manifest.version);
}
