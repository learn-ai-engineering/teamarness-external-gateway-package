#!/usr/bin/env node
// Applies Package Admission's rules to the assembled Package Directory, and smoke-launches it.
//
// This is the same verdict the Package Host reaches when it admits the Package, so a build that
// passes here is one Teamarness will accept. It validates against this machine's native target,
// or against `--target <triple>` when that is the native one. The validator's own default target
// is never relied on. After the validator passes, the native Backend's picker and CLI entry
// modes are smoke-tested.

import { smokePicker } from "./picker-smoke.mjs";
import { smokeCli } from "./cli-smoke.mjs";
import { spawnSync } from "node:child_process";
import { readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

import { exitCodeFor, nativeTarget, parseTargetArgs, selectTarget } from "./targets.mjs";

const projectRoot = dirname(dirname(fileURLToPath(import.meta.url)));

try {
  const sourceManifest = JSON.parse(await readFile(join(projectRoot, "teamarness.package.json"), "utf8"));
  const target = selectTarget({
    requested: parseTargetArgs(process.argv.slice(2)),
    native: nativeTarget({ cwd: projectRoot }),
  });
  const directory = join("dist", sourceManifest.identity);
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
      directory,
      "--target",
      target,
      "--smoke",
    ],
    { cwd: projectRoot, stdio: "inherit" },
  );
  if (result.error) throw result.error;
  // A validator killed by a signal has no status; that is a failure, not a target refusal.
  process.exitCode = result.status ?? 1;
  if (result.status === 0) {
    // The validator has admitted the assembled manifest, which declares only the native Backend.
    const manifest = JSON.parse(await readFile(join(projectRoot, directory, "teamarness.package.json"), "utf8"));
    const backends = manifest.backends.filter((artifact) => artifact.target === target);
    if (backends.length !== 1) {
      throw new Error(`the assembled manifest declares ${backends.length} Backends for ${target}; expected exactly one`);
    }
    const executable = join(projectRoot, directory, backends[0].path);
    smokePicker(executable);
    await smokeCli(executable, manifest.version);
  }
} catch (error) {
  process.stderr.write(`${error.message}\n`);
  process.exitCode = exitCodeFor(error);
}
