import { readFile, stat, readdir } from "node:fs/promises";
import { resolve, relative, sep } from "node:path";

const buildRoot = resolve("build");
const rawErrorCanary = "LOREPIA_RAW_SECRET_MUST_NOT_CROSS_BOUNDARY";

async function walk(directory) {
  const files = [];
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const path = resolve(directory, entry.name);
    if (entry.isDirectory()) files.push(...(await walk(path)));
    else if (entry.isFile()) files.push(path);
  }
  return files;
}

function invariant(condition, message) {
  if (!condition) throw new Error(message);
}

const files = await walk(buildRoot);
const relativeFiles = files.map((path) =>
  relative(buildRoot, path).split(sep).join("/"),
);
const wasmFiles = relativeFiles.filter((path) => path.endsWith(".wasm"));
const workerEntries = relativeFiles.filter((path) =>
  /(^|\/)workers\/script-runner\.worker-[^/]+\.js$/.test(path),
);

invariant(wasmFiles.length === 1, `expected one WASM file, got ${wasmFiles.length}`);
invariant(
  workerEntries.length === 1,
  `expected one script runner Worker entry, got ${workerEntries.length}`,
);
invariant(
  !relativeFiles.some((path) => path.includes("/fixtures/")),
  "raw fixture files must not be copied as standalone assets",
);
invariant(
  !relativeFiles.some((path) => path.endsWith(".map")),
  "source maps must not ship in the spike bundle",
);

const wasmBytes = (await stat(resolve(buildRoot, wasmFiles[0]))).size;
const workerBytes = (await stat(resolve(buildRoot, workerEntries[0]))).size;
invariant(wasmBytes > 0 && wasmBytes <= 2 * 1024 * 1024, "unexpected WASM size");
invariant(
  workerBytes > 0 && workerBytes <= 128 * 1024,
  "unexpected Worker entry size",
);

const textFiles = relativeFiles.filter((path) =>
  /\.(?:css|html|js|json)$/.test(path),
);
for (const path of textFiles) {
  const content = await readFile(resolve(buildRoot, path), "utf8");
  invariant(
    !content.includes("@tauri-apps/api"),
    `Tauri JavaScript API leaked into ${path}`,
  );
  if (!path.includes("/workers/")) {
    invariant(
      !content.includes(rawErrorCanary),
      `host bundle contains the Worker-only raw-error canary: ${path}`,
    );
    invariant(
      !content.includes("__TAURI_INTERNALS__"),
      `host bundle contains the Worker-only forbidden-global probe: ${path}`,
    );
  }
}

const workerSource = await readFile(
  resolve(buildRoot, workerEntries[0]),
  "utf8",
);
invariant(
  workerSource.includes(rawErrorCanary),
  "Worker entry is missing the pinned raw-error fixture",
);
invariant(
  workerSource.includes("HOST_TERMINATED"),
  "Worker entry is missing the stable termination receipt contract",
);
invariant(
  workerSource.includes("WEDGE_STARTED"),
  "Worker entry is missing the invocation-bound wedge acknowledgement",
);

console.log(
  JSON.stringify({
    status: "PASS",
    workerEntry: workerEntries[0],
    workerBytes,
    wasm: wasmFiles[0],
    wasmBytes,
  }),
);
