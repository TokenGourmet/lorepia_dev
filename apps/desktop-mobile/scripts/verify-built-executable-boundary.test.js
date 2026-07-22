import {
  mkdirSync,
  mkdtempSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";

import {
  REQUIRED_POLICY_MARKER,
  verifyBuiltExecutableBoundary,
} from "./verify-built-executable-boundary.mjs";

const temporaryDirectories = [];

function makeBuild(files) {
  const root = mkdtempSync(join(tmpdir(), "lorepia-built-boundary-"));
  temporaryDirectories.push(root);
  for (const [path, content] of Object.entries(files)) {
    const target = join(root, path);
    mkdirSync(dirname(target), { recursive: true });
    writeFileSync(target, content);
  }
  return root;
}

afterEach(() => {
  for (const directory of temporaryDirectories.splice(0)) {
    rmSync(directory, { force: true, recursive: true });
  }
});

describe("built executable-content boundary", () => {
  it("accepts the trusted host bundle and its normal Tauri invoke bridge", () => {
    const root = makeBuild({
      "index.html": '<script type="module" src="/assets/app.js"></script>',
      "assets/app.js": `window.__TAURI_INTERNALS__.invoke("get_product_bootstrap");const policy="${REQUIRED_POLICY_MARKER}";`,
    });

    expect(verifyBuiltExecutableBoundary(root)).toEqual({
      filesScanned: 2,
      policyMarker: REQUIRED_POLICY_MARKER,
    });
  });

  it.each([
    ["plugin-frame", "const asset = 'plugin-frame.html';"],
    ["plugin protocol", "postMessage('lorepia:plugin:ready');"],
    ["iframe", "<iframe sandbox></iframe>"],
    ["dynamic iframe", "document.createElement('iframe')"],
    ["srcdoc", "frame.srcdoc = markup;"],
    ["Function constructor", "new Function(source)"],
    ["Function constructor without new", "Function(source)"],
    ["eval", "eval(source)"],
    ["Worker", "new Worker('/plugin.js')"],
    ["SharedWorker", "SharedWorker('/plugin.js')"],
    ["WebAssembly", "WebAssembly.instantiate(bytes)"],
    ["stale policy", "DISABLED_PENDING_M1_EVIDENCE"],
    ["stale partial policy", "DISABLED_UNTIL_TERMINABLE_RUNTIME"],
  ])("rejects the %s marker", (_label, marker) => {
    const root = makeBuild({
      "assets/app.js": `${marker};const policy="${REQUIRED_POLICY_MARKER}";`,
    });

    expect(() => verifyBuiltExecutableBoundary(root)).toThrow();
  });

  it.each(["plugin.lua", "plugin.luac", "plugin.wasm"])(
    "rejects the %s executable artifact",
    (filename) => {
      const root = makeBuild({
        "assets/app.js": `const policy="${REQUIRED_POLICY_MARKER}";`,
        [`assets/${filename}`]: "fixture",
      });

      expect(() => verifyBuiltExecutableBoundary(root)).toThrow(
        "executable imported-content artifact",
      );
    },
  );

  it("scans CommonJS output instead of treating it as an opaque asset", () => {
    const root = makeBuild({
      "assets/app.js": `const policy="${REQUIRED_POLICY_MARKER}";`,
      "assets/plugin.cjs": "globalThis.eval(source);",
    });

    expect(() => verifyBuiltExecutableBoundary(root)).toThrow("eval identifier");
  });

  it("rejects unreviewed output types", () => {
    const root = makeBuild({
      "assets/app.js": `const policy="${REQUIRED_POLICY_MARKER}";`,
      "assets/plugin.bin": "opaque",
    });

    expect(() => verifyBuiltExecutableBoundary(root)).toThrow(
      "unreviewed frontend artifact type",
    );
  });

  it("rejects a build that does not carry the disabled policy contract", () => {
    const root = makeBuild({ "assets/app.js": "const trustedHost = true;" });

    expect(() => verifyBuiltExecutableBoundary(root)).toThrow(
      "required imported-content policy marker is absent",
    );
  });
});
