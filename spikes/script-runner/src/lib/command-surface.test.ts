import { readFileSync, readdirSync } from "node:fs";
import { describe, expect, it } from "vitest";

import capability from "../../src-tauri/capabilities/default.json";
import tauriConfig from "../../src-tauri/tauri.conf.json";
import packageJson from "../../package.json";

const nativeLib = readFileSync(
  new URL("../../src-tauri/src/lib.rs", import.meta.url),
  "utf8",
);
const workerSource = readFileSync(
  new URL("./script-runner.worker.ts", import.meta.url),
  "utf8",
);
const controllerSource = readFileSync(
  new URL("./runner-controller.ts", import.meta.url),
  "utf8",
);
const capabilityFiles = readdirSync(
  new URL("../../src-tauri/capabilities", import.meta.url),
).sort();

describe("script runner command and WebView surface", () => {
  it("keeps the Tauri native command and permission surfaces empty", () => {
    expect(capabilityFiles).toEqual(["default.json"]);
    expect(capability.webviews).toEqual(["main"]);
    expect(capability.permissions).toEqual([]);
    expect(nativeLib).toContain("const NATIVE_COMMANDS: &[&str] = &[];");
    expect(nativeLib).not.toContain("generate_handler!");
    expect(nativeLib).not.toContain("invoke_handler");
  });

  it("allows only the static Worker and WASM compiler required by the spike", () => {
    expect(tauriConfig.app.security.csp["frame-src"]).toBe("'none'");
    expect(tauriConfig.app.security.csp["object-src"]).toBe("'none'");
    expect(tauriConfig.app.security.csp["worker-src"]).toBe("'self'");
    expect(tauriConfig.app.security.csp["script-src"]).toBe(
      "'self' 'wasm-unsafe-eval'",
    );
    expect(tauriConfig.app.security.csp["connect-src"]).toBe("'self'");
  });

  it("pins only the release QuickJS-WASM runtime packages", () => {
    expect(packageJson.dependencies).toEqual({
      "@jitl/quickjs-wasmfile-release-sync": "0.32.0",
      "quickjs-emscripten-core": "0.32.0",
    });
    expect(JSON.stringify(packageJson)).not.toContain("@tauri-apps/api");
  });

  it("never sends source, input JSON, engine limits, or a native command", () => {
    expect(workerSource).not.toContain("@tauri-apps/api");
    expect(controllerSource).not.toContain("@tauri-apps/api");
    expect(controllerSource).not.toMatch(/\binvoke\s*\(/);
    const requestBlock = controllerSource.match(
      /const request: WorkerRequest = \{[\s\S]*?\n\s*\};/,
    )?.[0];
    expect(requestBlock).toBeDefined();
    expect(requestBlock).not.toMatch(/source|input|limit|path/i);
  });
});
