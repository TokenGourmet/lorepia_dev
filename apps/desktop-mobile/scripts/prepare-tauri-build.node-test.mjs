import assert from "node:assert/strict";
import { mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import {
  prepareTauriBuild,
  removeStaleIOSAppOutputs,
} from "./prepare-tauri-build.mjs";

test("removes only direct generated iOS app outputs", async (t) => {
  const root = await mkdtemp(join(tmpdir(), "lorepia-tauri-build-"));
  t.after(() => rm(root, { recursive: true, force: true }));

  const staleArmApp = join(root, "arm64-sim", "LorePia.app");
  const staleIntelApp = join(root, "x86_64-sim", "LorePia.app");
  const otherApp = join(root, "arm64-sim", "Other.app");
  const archivedApp = join(
    root,
    "lorepia-app_iOS.xcarchive",
    "Products",
    "Applications",
    "LorePia.app",
  );

  await Promise.all([
    mkdir(staleArmApp, { recursive: true }),
    mkdir(staleIntelApp, { recursive: true }),
    mkdir(otherApp, { recursive: true }),
    mkdir(archivedApp, { recursive: true }),
  ]);
  await writeFile(join(staleArmApp, "sentinel"), "remove");
  await writeFile(join(staleIntelApp, "sentinel"), "remove");
  await writeFile(join(otherApp, "sentinel"), "keep");
  await writeFile(join(archivedApp, "sentinel"), "keep");

  const removed = await removeStaleIOSAppOutputs({ buildRoot: root });

  assert.deepEqual(removed.sort(), [staleArmApp, staleIntelApp].sort());
  await assert.rejects(readFile(join(staleArmApp, "sentinel")));
  await assert.rejects(readFile(join(staleIntelApp, "sentinel")));
  assert.equal(await readFile(join(otherApp, "sentinel"), "utf8"), "keep");
  assert.equal(await readFile(join(archivedApp, "sentinel"), "utf8"), "keep");
});

test("does nothing when the generated build directory does not exist", async () => {
  const missing = join(tmpdir(), `lorepia-missing-${process.pid}-${Date.now()}`);
  assert.deepEqual(
    await removeStaleIOSAppOutputs({ buildRoot: missing }),
    [],
  );
});

test("does not remove generated apps for non-iOS Tauri builds", async (t) => {
  const root = await mkdtemp(join(tmpdir(), "lorepia-tauri-android-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const app = join(root, "arm64", "LorePia.app");
  await mkdir(app, { recursive: true });
  await writeFile(join(app, "sentinel"), "keep");

  assert.deepEqual(
    await prepareTauriBuild({ platform: "android", buildRoot: root }),
    [],
  );
  assert.equal(await readFile(join(app, "sentinel"), "utf8"), "keep");
});
