// SPDX-License-Identifier: Apache-2.0
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { join } from "node:path";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { fileURLToPath } from "node:url";
import test from "node:test";

const require = createRequire(import.meta.url);
const turboCli = require.resolve("turbo/bin/turbo");
const repositoryRoot = fileURLToPath(new URL("../", import.meta.url));
const anydashDist = join(repositoryRoot, "apps", "anydash", "dist");
const config = JSON.parse(
  readFileSync(new URL("../turbo.json", import.meta.url), "utf8"),
);
const installerInputs = [
  "../../packages/anydash/install/install.sh",
  "../../packages/anydash/install/install.ps1",
];
const lynxUiInputs = [
  "../../packages/lynx-ui/package.json",
  "../../packages/lynx-ui/src/components/button.tsx",
  "../../packages/lynx-ui/src/styles.css",
];
const lynxUiInputGlobs = [
  "$TURBO_ROOT$/packages/lynx-ui/package.json",
  "$TURBO_ROOT$/packages/lynx-ui/src/**",
];

function runDryPlan(taskName) {
  const run = spawnSync(
    process.execPath,
    [
      turboCli,
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
  return JSON.parse(run.stdout).tasks;
}

function runCachedBuild(taskName, cacheDirectory) {
  const run = spawnSync(
    process.execPath,
    [
      turboCli,
      "run",
      taskName,
      "--filter=@apps/anydash",
      "--cache=local:rw",
      `--cache-dir=${cacheDirectory}`,
    ],
    {
      cwd: repositoryRoot,
      encoding: "utf8",
      maxBuffer: 10 * 1024 * 1024,
      env: {
        ...process.env,
        TURBO_CACHE: "local:rw",
        TURBO_TEAM: "",
        TURBO_TOKEN: "",
      },
    },
  );
  assert.equal(
    run.status,
    0,
    `Turbo ${taskName} failed:\n${run.stdout}\n${run.stderr}`,
  );
  return run.stdout;
}

function outputContains(directory, marker) {
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isDirectory() && outputContains(path, marker)) return true;
    if (entry.isFile() && readFileSync(path).includes(Buffer.from(marker))) {
      return true;
    }
  }
  return false;
}

function assertSourceClean(repositoryPath) {
  const status = spawnSync(
    "git",
    ["status", "--porcelain", "--", repositoryPath],
    { cwd: repositoryRoot, encoding: "utf8" },
  );
  assert.equal(status.status, 0, status.stderr);
  assert.equal(
    status.stdout.trim(),
    "",
    `Refusing to modify a dirty source file: ${repositoryPath}`,
  );
}

function findTask(tasks, taskId) {
  const task = tasks.find((entry) => entry.taskId === taskId);
  assert.ok(task, `Expected Turbo dry plan to include ${taskId}`);
  return task;
}

const bundleCases = [
  {
    label: "shared TSX",
    sourcePath: "packages/lynx-ui/src/components/button.tsx",
    update(source, marker) {
      const originalClass = '"lui-button",';
      assert.ok(source.includes(originalClass));
      return source.replace(originalClass, `"lui-button-${marker}",`);
    },
  },
  {
    label: "shared CSS",
    sourcePath: "packages/lynx-ui/src/styles.css",
    update(source, marker) {
      return `${source}\n.${marker} { --${marker}: "included"; }\n`;
    },
  },
];

test("AnyDash cached build regression", async (t) => {
  for (const taskName of ["build", "build:frontend"]) {
    await t.test(
      `AnyDash ${taskName} inputs preserve installers and include Lynx UI source`,
      () => {
        const base = config.tasks[taskName];
        const anydash = config.tasks[`@apps/anydash#${taskName}`];
        assert.deepEqual(anydash.inputs, [
          ...base.inputs,
          "$TURBO_ROOT$/packages/anydash/install/**",
          ...lynxUiInputGlobs,
        ]);
        assert.deepEqual(anydash.outputs, base.outputs);

        const tasks = runDryPlan(taskName);
        const anydashTask = findTask(tasks, `@apps/anydash#${taskName}`);
        const controlTask = findTask(tasks, `@apps/delitools#${taskName}`);
        for (const path of installerInputs) {
          assert.ok(Object.hasOwn(anydashTask.inputs, path));
          assert.ok(!Object.hasOwn(controlTask.inputs, path));
        }
        for (const path of lynxUiInputs) {
          assert.ok(Object.hasOwn(anydashTask.inputs, path));
          assert.ok(!Object.hasOwn(controlTask.inputs, path));
        }
        assert.ok(Object.hasOwn(anydashTask.inputs, "src/App.tsx"));
        assert.deepEqual(anydashTask.outputs, ["dist/**"]);
        assert.deepEqual(controlTask.outputs, ["dist/**"]);
      },
    );

    await t.test(
      `AnyDash ${taskName} cache key includes Lynx UI TSX, CSS and ignored env files`,
      () => {
        const lynxUiSource = fileURLToPath(
          new URL("../packages/lynx-ui/src/", import.meta.url),
        );
        const anydashRoot = fileURLToPath(
          new URL("../apps/anydash/", import.meta.url),
        );
        const componentProbe = join(
          lynxUiSource,
          `turbo-cache-probe-${process.pid}.tsx`,
        );
        const styleProbe = join(
          lynxUiSource,
          `turbo-cache-probe-${process.pid}.css`,
        );
        const envProbe = join(
          anydashRoot,
          `.env.turbo-cache-probe-${process.pid}`,
        );

        try {
          writeFileSync(componentProbe, "export const probe = 'initial';\n");
          writeFileSync(styleProbe, ".probe { --probe: initial; }\n");
          writeFileSync(envProbe, "TURBO_CACHE_PROBE=initial\n");

          const initialTasks = runDryPlan(taskName);
          const initialAnyDash = findTask(
            initialTasks,
            `@apps/anydash#${taskName}`,
          );
          const initialDelitools = findTask(
            initialTasks,
            `@apps/delitools#${taskName}`,
          );
          const repeatedAnyDash = findTask(
            runDryPlan(taskName),
            `@apps/anydash#${taskName}`,
          );
          assert.equal(repeatedAnyDash.hash, initialAnyDash.hash);
          assert.ok(
            Object.hasOwn(
              initialAnyDash.inputs,
              `../../packages/lynx-ui/src/turbo-cache-probe-${process.pid}.tsx`,
            ),
          );
          assert.ok(
            Object.hasOwn(
              initialAnyDash.inputs,
              `../../packages/lynx-ui/src/turbo-cache-probe-${process.pid}.css`,
            ),
          );
          assert.ok(
            Object.hasOwn(
              initialAnyDash.inputs,
              `.env.turbo-cache-probe-${process.pid}`,
            ),
          );

          writeFileSync(componentProbe, "export const probe = 'changed';\n");
          const changedComponent = findTask(
            runDryPlan(taskName),
            `@apps/anydash#${taskName}`,
          );
          assert.notEqual(changedComponent.hash, initialAnyDash.hash);

          writeFileSync(styleProbe, ".probe { --probe: changed; }\n");
          const changedStyle = findTask(
            runDryPlan(taskName),
            `@apps/anydash#${taskName}`,
          );
          assert.notEqual(changedStyle.hash, changedComponent.hash);

          writeFileSync(envProbe, "TURBO_CACHE_PROBE=changed\n");
          const changedEnv = findTask(
            runDryPlan(taskName),
            `@apps/anydash#${taskName}`,
          );
          assert.notEqual(changedEnv.hash, changedStyle.hash);

          const unchangedControl = findTask(
            runDryPlan(taskName),
            `@apps/delitools#${taskName}`,
          );
          assert.equal(unchangedControl.hash, initialDelitools.hash);
        } finally {
          rmSync(componentProbe, { force: true });
          rmSync(styleProbe, { force: true });
          rmSync(envProbe, { force: true });
        }
      },
    );
  }

  for (const taskName of ["build", "build:frontend"]) {
    for (const { label, sourcePath, update } of bundleCases) {
      await t.test(
        `AnyDash ${taskName} rebuilds its bundle after a ${label} change`,
        () => {
          assertSourceClean(sourcePath);

          const sourceFile = join(repositoryRoot, sourcePath);
          const originalSource = readFileSync(sourceFile);
          const cacheDirectory = mkdtempSync(
            join(tmpdir(), `turbo-${taskName.replaceAll(":", "-")}-`),
          );
          const marker = `turbo-cache-${taskName.replaceAll(":", "-")}-${label.replaceAll(" ", "-")}-${process.pid}`;

          try {
            const baseline = runCachedBuild(taskName, cacheDirectory);
            assert.match(baseline, /cache miss, executing/);

            let repeated = runCachedBuild(taskName, cacheDirectory);
            if (!/cache hit, replaying logs/.test(repeated)) {
              repeated = runCachedBuild(taskName, cacheDirectory);
            }
            assert.match(repeated, /cache hit, replaying logs/);

            const originalText = originalSource.toString("utf8");
            const changedSource = update(originalText, marker);
            assert.notEqual(changedSource, originalText);
            writeFileSync(sourceFile, changedSource);

            const changed = runCachedBuild(taskName, cacheDirectory);
            assert.match(changed, /cache miss, executing/);
            assert.ok(
              outputContains(anydashDist, marker),
              `Expected the rebuilt AnyDash bundle to contain ${marker}`,
            );
          } finally {
            writeFileSync(sourceFile, originalSource);
            rmSync(anydashDist, { recursive: true, force: true });
            rmSync(cacheDirectory, { recursive: true, force: true });
          }
        },
      );
    }
  }
});
