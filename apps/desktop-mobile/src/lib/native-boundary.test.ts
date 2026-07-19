import { readFileSync, readdirSync } from "node:fs";
import { describe, expect, it } from "vitest";

import capability from "../../src-tauri/capabilities/default.json";
import tauriConfig from "../../src-tauri/tauri.conf.json";

const androidGradle = readFileSync(
  new URL(
    "../../src-tauri/gen/android/app/build.gradle.kts",
    import.meta.url,
  ),
  "utf8",
);
const androidActivity = readFileSync(
  new URL(
    "../../src-tauri/gen/android/app/src/main/java/dev/lorepia/client/MainActivity.kt",
    import.meta.url,
  ),
  "utf8",
);
const appleProject = readFileSync(
  new URL("../../src-tauri/gen/apple/project.yml", import.meta.url),
  "utf8",
);
const capabilityFiles = readdirSync(
  new URL("../../src-tauri/capabilities", import.meta.url),
).sort();

describe("native product boundary", () => {
  it("grants only the product commands to the trusted main WebView", () => {
    expect(tauriConfig.app.security.capabilities).toEqual(["default"]);
    expect(capabilityFiles).toEqual(["default.json"]);
    expect(capability.webviews).toEqual(["main"]);
    expect(capability.permissions).toEqual([
      "allow-get-product-bootstrap",
      "allow-get-provider-credential-status",
      "allow-save-provider-api-key",
      "allow-delete-provider-credential",
      "allow-start-provider-stream",
      "allow-ack-provider-stream",
      "allow-cancel-provider-stream",
      "allow-get-provider-stream-snapshot",
    ]);
  });

  it("starts with network and executable WebView surfaces closed", () => {
    for (const csp of [
      tauriConfig.app.security.csp,
      tauriConfig.app.security.devCsp,
    ]) {
      expect(csp["frame-src"]).toBe("'none'");
      expect(csp["media-src"]).toBe("'none'");
      expect(csp["object-src"]).toBe("'none'");
      expect(csp["worker-src"]).toBe("'none'");
    }
    expect(tauriConfig.app.security.csp["connect-src"]).toBe("'self'");
    expect(tauriConfig.app.security.devCsp["connect-src"]).toBe(
      "'self' http://localhost:1423 ws://localhost:1424",
    );
    expect(tauriConfig.app.security.csp["script-src"]).toBe("'self'");
    expect(tauriConfig.app.security.devCsp["script-src"]).toBe(
      "'self' http://localhost:1423",
    );
  });

  it("keeps desktop and committed mobile wrapper identifiers aligned", () => {
    expect(tauriConfig.identifier).toBe("dev.lorepia.client");
    expect(androidGradle).toContain('namespace = "dev.lorepia.client"');
    expect(androidGradle).toContain('applicationId = "dev.lorepia.client"');
    expect(androidActivity).toContain("package dev.lorepia.client");
    expect(appleProject).toContain(
      "PRODUCT_BUNDLE_IDENTIFIER: dev.lorepia.client",
    );
  });
});
