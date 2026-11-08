// SPDX-License-Identifier: Apache-2.0
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import test from "node:test";

const require = createRequire(import.meta.url);
const config = JSON.parse(
  readFileSync(new URL("../turbo.json", import.meta.url), "utf8"),
);
const installerInputs = [
  "../../packages/anydash/install/install.sh",
  "../../packages/anydash/install/install.ps1",
];

for (const taskName of ["build", "build:frontend"]) {
  test(`only AnyDash ${taskName} includes the copied CLI installers`, () => {
    const base = config.tasks[taskName];
    const anydash = config.tasks[`@apps/anydash#${taskName}`];
    assert.deepEqual(anydash.inputs, [
      ...base.inputs,
      "$TURBO_ROOT$/packages/anydash/install/**",
    ]);
    assert.deepEqual(anydash.outputs, base.outputs);

    const run = spawnSync(
      process.execPath,
      [
        require.resolve("turbo/bin/turbo"),
        "run",
        taskName,
        "--filter=@apps/anydash",
        "--filter=@apps/delitools",
        "--dry=json",
      ],
      {
        cwd: new URL("../", import.meta.url),
        encoding: "utf8",
        env: {
          ...process.env,
          TURBO_CACHE: "local:rw",
          TURBO_TEAM: "",
          TURBO_TOKEN: "",
        },
      },
    );
    assert.equal(run.status, 0, run.stderr);
    const tasks = JSON.parse(run.stdout).tasks;
    const anydashTask = tasks.find(
      (task) => task.taskId === `@apps/anydash#${taskName}`,
    );
    const controlTask = tasks.find(
      (task) => task.taskId === `@apps/delitools#${taskName}`,
    );
    assert.ok(anydashTask);
    assert.ok(controlTask);
    for (const path of installerInputs) {
      assert.ok(Object.hasOwn(anydashTask.inputs, path));
      assert.ok(!Object.hasOwn(controlTask.inputs, path));
    }
  });
}
