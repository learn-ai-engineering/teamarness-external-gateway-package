// Offline final-artifact proof only. Never performs setup or resolves the user's client home.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { copyFile, chmod, mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

export async function smokeCli(executable, version) {
  const template = await readFile(new URL("../backend/resources/teamarness/SKILL.md", import.meta.url));
  assert.ok((await readFile(executable)).includes(template), "final native artifact embeds the complete portable skill");
  const encodedNotices = await readFile(new URL("../backend/resources/THIRD_PARTY_NOTICES.json", import.meta.url), "utf8");
  const notices = JSON.parse(encodedNotices);
  const originalRfd = await readFile(new URL("../backend/tests/fixtures/notices/rfd-0.16.1-LICENSE", import.meta.url));
  assert.ok(Buffer.from(notices).includes(originalRfd), "decoded notices retain independent original RFD bytes");
  assert.ok((await readFile(executable)).includes(Buffer.from(encodedNotices)), "final artifact embeds complete escaped third-party notices");
  const temporary = await mkdtemp(join(tmpdir(), "teamarness-cli-smoke-"));
  try {
    const copy = join(temporary, "tmx");
    await copyFile(executable, copy);
    await chmod(copy, 0o700);
    const invoke = (path, args, status = 0) => {
      const out = spawnSync(path, args, {
        cwd: temporary, env: { PATH: "/usr/bin:/bin" }, encoding: "utf8", timeout: 5000,
        maxBuffer: 1024 * 1024, shell: false,
      });
      assert.ifError(out.error);
      assert.equal(out.status, status, "offline entry status");
      assert.equal(out.stderr, "", "offline success diagnostics");
      return out.stdout;
    };
    for (const [path, prefix] of [[executable, ["--tmx-cli"]], [copy, []]]) {
      const help = invoke(path, prefix);
      assert.match(help, /tmx workspaces/);
      assert.match(help, /No automatic retries/);
      assert.match(help, /^\s+subscribe\s/m);
      assert.match(help, /--for <duration> \| --until-cancelled/);
      assert.match(help, /counts values only, not heartbeats/);
      assert.match(help, /No automatic retries or reconnects/);
      assert.equal(invoke(path, [...prefix, "--licenses"]), `${notices}\n`);
      assert.match(help, /--licenses/);
      assert.equal(invoke(path, [...prefix, "--version"]), `tmx ${version} (CLI 1)\n`);
      assert.equal(invoke(path, [...prefix, "--help"]), help);
    }
    console.log("Final embedded portable skill and native CLI modes: offline help/version/licenses, explicit and basename tmx passed.");
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
}
