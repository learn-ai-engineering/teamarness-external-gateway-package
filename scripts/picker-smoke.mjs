// Final artifact helper entry refuses private protocol faults before any AppKit interaction.
// This is not native focus/selection or installed-environment qualification.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
export function smokePicker(executable) {
  for (const input of ["", "{}\n", '{"version":2,"operation":"choose-skill-folder"}\n']) {
    const out = spawnSync(executable, ["--tmx-picker"], {
      input, encoding: "utf8", timeout: 5000, maxBuffer: 8192,
      env: { PATH: "/usr/bin:/bin" }, shell: false,
    });
    assert.ifError(out.error);
    assert.equal(out.status, 2);
    assert.equal(out.stderr, "");
    assert.equal(out.stdout, '{"code":"picker_request_invalid","outcome":"error","version":1}\n');
  }
  console.log("Final native picker mode: private protocol refusal without AppKit, Host or home lookup passed; actual native selection remains manual.");
}
