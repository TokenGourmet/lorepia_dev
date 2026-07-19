import {
  existsSync,
  readFileSync,
  readdirSync,
  statSync,
} from "node:fs";
import { extname, relative, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

export const REQUIRED_POLICY_MARKER = "DISABLED_BY_SECURITY_POLICY";

const EXECUTABLE_EXTENSIONS = new Set([".lua", ".luac", ".wasm"]);
const TEXT_EXTENSIONS = new Set([
  ".cjs",
  ".css",
  ".html",
  ".js",
  ".json",
  ".map",
  ".mjs",
  ".svg",
  ".txt",
]);
const REVIEWED_BINARY_EXTENSIONS = new Set([
  ".gif",
  ".ico",
  ".icns",
  ".jpeg",
  ".jpg",
  ".otf",
  ".png",
  ".ttf",
  ".webp",
  ".woff",
  ".woff2",
]);
const FORBIDDEN_TEXT = [
  ["plugin frame asset", /plugin-frame/i],
  ["plugin message protocol", /lorepia:plugin:/i],
  ["iframe runtime", /\biframe\b/i],
  ["iframe srcdoc", /\bsrcdoc\b/i],
  ["dynamic Function constructor", /\b(?:new\s+)?Function\s*\(/],
  ["eval identifier", /\beval\b/],
  ["Web Worker runtime", /\b(?:Shared)?Worker\b/],
  ["WebAssembly runtime", /\bWebAssembly\b/],
  ["stale M0 executable-content policy", /DISABLED_PENDING_M1_EVIDENCE/],
  ["stale terminable-only policy", /DISABLED_UNTIL_TERMINABLE_RUNTIME/],
];

function listFiles(directory) {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = resolve(directory, entry.name);
    return entry.isDirectory() ? listFiles(path) : [path];
  });
}

export function verifyBuiltExecutableBoundary(buildDirectory) {
  const root = resolve(buildDirectory);
  if (!existsSync(root) || !statSync(root).isDirectory()) {
    throw new Error(`frontend build directory does not exist: ${root}`);
  }

  const files = listFiles(root);
  if (files.length === 0) {
    throw new Error(`frontend build directory is empty: ${root}`);
  }

  let policyMarkerFound = false;
  for (const file of files) {
    const relativePath = relative(root, file).replaceAll("\\", "/");
    const extension = extname(file).toLowerCase();

    if (EXECUTABLE_EXTENSIONS.has(extension)) {
      throw new Error(`executable imported-content artifact found: ${relativePath}`);
    }
    if (/plugin-frame/i.test(relativePath)) {
      throw new Error(`plugin frame artifact found: ${relativePath}`);
    }
    if (!TEXT_EXTENSIONS.has(extension)) {
      if (!REVIEWED_BINARY_EXTENSIONS.has(extension)) {
        throw new Error(`unreviewed frontend artifact type found: ${relativePath}`);
      }
      continue;
    }

    const content = readFileSync(file, "utf8");
    policyMarkerFound ||= content.includes(REQUIRED_POLICY_MARKER);

    for (const [label, pattern] of FORBIDDEN_TEXT) {
      if (pattern.test(content)) {
        throw new Error(`${label} found in ${relativePath}`);
      }
    }
  }

  if (!policyMarkerFound) {
    throw new Error(
      `required imported-content policy marker is absent: ${REQUIRED_POLICY_MARKER}`,
    );
  }

  return { filesScanned: files.length, policyMarker: REQUIRED_POLICY_MARKER };
}

const invokedPath = process.argv[1] ? pathToFileURL(resolve(process.argv[1])).href : "";
if (import.meta.url === invokedPath) {
  const scriptDirectory = resolve(fileURLToPath(new URL(".", import.meta.url)));
  const buildDirectory = resolve(scriptDirectory, "../build");
  const result = verifyBuiltExecutableBoundary(buildDirectory);
  process.stdout.write(
    `verified ${result.filesScanned} frontend build files; ${result.policyMarker}\n`,
  );
}
