import { spawn } from "node:child_process";
import { readdir, rm } from "node:fs/promises";
import { dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const SCRIPT_DIR = dirname(fileURLToPath(import.meta.url));
const PROJECT_ROOT = resolve(SCRIPT_DIR, "..");
const GENERATED_APP_ROOT = resolve(
  PROJECT_ROOT,
  "src-tauri",
  "gen",
  "apple",
  "build",
);

function isDirectGeneratedApp(buildRoot, candidate, productName) {
  const expectedName = `${productName}.app`;
  const relativePath = relative(buildRoot, candidate);
  const segments = relativePath.split(sep);

  return (
    relativePath !== "" &&
    !relativePath.startsWith(`..${sep}`) &&
    segments.length === 2 &&
    segments[1] === expectedName
  );
}

export async function removeStaleIOSAppOutputs({
  buildRoot = GENERATED_APP_ROOT,
  productName = "LorePia",
} = {}) {
  let entries;
  try {
    entries = await readdir(buildRoot, { withFileTypes: true });
  } catch (error) {
    if (error?.code === "ENOENT") {
      return [];
    }
    throw error;
  }

  const removed = [];
  for (const entry of entries) {
    if (!entry.isDirectory() || entry.name.endsWith(".xcarchive")) {
      continue;
    }

    const candidate = join(buildRoot, entry.name, `${productName}.app`);
    if (!isDirectGeneratedApp(buildRoot, candidate, productName)) {
      throw new Error(`Refusing to remove an unsafe generated path: ${candidate}`);
    }

    await rm(candidate, { recursive: true, force: true });
    removed.push(candidate);
  }

  return removed;
}

export async function prepareTauriBuild({
  platform = process.env.TAURI_ENV_PLATFORM,
  buildRoot = GENERATED_APP_ROOT,
  productName = "LorePia",
} = {}) {
  if (platform !== "ios") {
    return [];
  }

  return removeStaleIOSAppOutputs({ buildRoot, productName });
}

function runFrontendBuild() {
  const npmCommand = process.platform === "win32" ? "npm.cmd" : "npm";
  const child = spawn(npmCommand, ["run", "build"], {
    cwd: PROJECT_ROOT,
    env: process.env,
    stdio: "inherit",
  });

  child.on("error", (error) => {
    console.error(error);
    process.exitCode = 1;
  });
  child.on("exit", (code, signal) => {
    if (signal) {
      process.kill(process.pid, signal);
      return;
    }
    process.exitCode = code ?? 1;
  });
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  await prepareTauriBuild();
  runFrontendBuild();
}
