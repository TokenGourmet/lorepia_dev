import { execFileSync } from "node:child_process";
import { resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const PRODUCT_PACKAGE = "lorepia-app";
const PRODUCT_MANIFEST_SUFFIX = "/apps/desktop-mobile/src-tauri/Cargo.toml";
const FORBIDDEN_RUNTIME_NAMES = new Set([
  "boa_engine",
  "deno_core",
  "lorepia-lua-limits-spike",
  "lorepia-script-runner-spike",
  "luajit",
  "mlua",
  "rlua",
  "rquickjs",
  "rusty_v8",
  "quickjs-runtime",
  "quickjs-wasm-rs",
  "v8",
  "wasm3",
  "wasmer",
  "wasmi",
  "wasmtime",
]);
const FORBIDDEN_RUNTIME_NAME_PATTERN =
  /(?:^|[-_])(boa_engine|deno_core|luajit|mlua|rlua|rquickjs|rusty_v8|lua5[1-4]|v8|wasm3|wasmer|wasmi|wasmtime)(?:$|[-_])/i;

function hasForbiddenRuntimeName(name) {
  const normalizedName = name.toLowerCase();
  return (
    FORBIDDEN_RUNTIME_NAMES.has(name) ||
    FORBIDDEN_RUNTIME_NAME_PATTERN.test(name) ||
    normalizedName.includes("quickjs") ||
    /(?:^|[-_]|lib)lua(?:jit|5[1-4])?(?:$|[-_])/.test(normalizedName)
  );
}

function normalized(path) {
  return path.replaceAll("\\", "/");
}

function runtimeDependencyIds(node) {
  if (Array.isArray(node.deps)) {
    return node.deps
      .filter((dependency) =>
        dependency.dep_kinds?.some((kind) => kind.kind !== "dev"),
      )
      .map((dependency) => dependency.pkg);
  }
  return node.dependencies ?? [];
}

export function verifyNativeProductDependencyBoundary(metadata) {
  if (!metadata || !Array.isArray(metadata.packages) || !metadata.resolve?.nodes) {
    throw new Error("cargo metadata did not contain a resolved package graph");
  }
  const packages = new Map(metadata.packages.map((pkg) => [pkg.id, pkg]));
  const nodes = new Map(metadata.resolve.nodes.map((node) => [node.id, node]));
  const products = metadata.packages.filter(
    (pkg) =>
      pkg.name === PRODUCT_PACKAGE &&
      normalized(pkg.manifest_path).endsWith(PRODUCT_MANIFEST_SUFFIX),
  );
  if (products.length !== 1) {
    throw new Error(`expected exactly one ${PRODUCT_PACKAGE} product package`);
  }

  const pending = [products[0].id];
  const visited = new Set();
  while (pending.length > 0) {
    const id = pending.pop();
    if (visited.has(id)) continue;
    visited.add(id);
    const pkg = packages.get(id);
    const node = nodes.get(id);
    if (!pkg || !node) {
      throw new Error("cargo metadata graph referenced an unknown package");
    }
    const manifest = normalized(pkg.manifest_path);
    if (manifest.includes("/spikes/")) {
      throw new Error(`product dependency closure contains spike package: ${pkg.name}`);
    }
    if (
      hasForbiddenRuntimeName(pkg.name)
    ) {
      throw new Error(`product dependency closure contains executable runtime: ${pkg.name}`);
    }
    pending.push(...runtimeDependencyIds(node));
  }

  return { product: PRODUCT_PACKAGE, packagesChecked: visited.size };
}

function loadRepositoryMetadata() {
  const scriptDirectory = fileURLToPath(new URL(".", import.meta.url));
  const repository = resolve(scriptDirectory, "../../..");
  const output = execFileSync(
    "cargo",
    ["metadata", "--locked", "--format-version", "1"],
    { cwd: repository, encoding: "utf8", maxBuffer: 64 * 1024 * 1024 },
  );
  return JSON.parse(output);
}

const invokedPath = process.argv[1]
  ? pathToFileURL(resolve(process.argv[1])).href
  : "";
if (import.meta.url === invokedPath) {
  const result = verifyNativeProductDependencyBoundary(loadRepositoryMetadata());
  process.stdout.write(
    `verified native product dependency boundary: ${result.packagesChecked} packages, no spike or executable script runtime\n`,
  );
}
