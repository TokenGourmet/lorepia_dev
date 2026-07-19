import { readFileSync } from "node:fs";
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

describe("native product boundary", () => {
  it("grants one bootstrap permission to the trusted main WebView", () => {
    expect(capability.webviews).toEqual(["main"]);
    expect(capability.permissions).toEqual(["allow-get-product-bootstrap"]);
  });

  it("starts with network and executable WebView surfaces closed", () => {
    const csp = tauriConfig.app.security.csp;
    expect(csp["connect-src"]).toBe("'self'");
    expect(csp["frame-src"]).toBe("'none'");
    expect(csp["media-src"]).toBe("'none'");
    expect(csp["object-src"]).toBe("'none'");
    expect(csp["worker-src"]).toBe("'none'");
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
