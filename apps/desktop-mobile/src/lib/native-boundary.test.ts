import { readFileSync, readdirSync } from "node:fs";
import { runInNewContext } from "node:vm";
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
const appTemplate = readFileSync(new URL("../app.html", import.meta.url), "utf8");
const platformInit = readFileSync(
  new URL("../../static/platform-init.js", import.meta.url),
  "utf8",
);
const designTokens = readFileSync(
  new URL("./design/tokens.css", import.meta.url),
  "utf8",
);
const appleProject = readFileSync(
  new URL("../../src-tauri/gen/apple/project.yml", import.meta.url),
  "utf8",
);
const nativeBackSwift = readFileSync(
  new URL(
    "../../../../crates/tauri-plugin-native-back/ios/Sources/NativeBackPlugin.swift",
    import.meta.url,
  ),
  "utf8",
);
const capabilityFiles = readdirSync(
  new URL("../../src-tauri/capabilities", import.meta.url),
).sort();

function runPlatformInit({
  userAgent,
  platform,
  maxTouchPoints,
}: {
  userAgent: string;
  platform: string;
  maxTouchPoints: number;
}) {
  const dataset: Record<string, string> = {};
  runInNewContext(platformInit, {
    navigator: { userAgent, platform, maxTouchPoints },
    document: { documentElement: { dataset } },
  });
  return dataset;
}

describe("native product boundary", () => {
  it("grants only the product commands to the trusted main WebView", () => {
    expect(tauriConfig.app.security.capabilities).toEqual(["default"]);
    expect(capabilityFiles).toEqual(["default.json"]);
    expect(capability.webviews).toEqual(["main"]);
    expect(capability.permissions).toEqual([
      "core:window:allow-destroy",
      "native-back:default",
      "allow-get-product-bootstrap",
      "allow-get-provider-credential-status",
      "allow-save-provider-api-key",
      "allow-delete-provider-credential",
      "allow-start-provider-stream",
      "allow-ack-provider-stream",
      "allow-cancel-provider-stream",
      "allow-reset-provider-stream-owner",
      "allow-get-provider-stream-snapshot",
      "allow-get-storage-status",
      "allow-get-asset-store-status",
      "allow-create-chat",
      "allow-list-chats",
      "allow-load-chat-messages",
      "allow-delete-chat",
      "allow-get-app-preferences",
      "allow-update-app-preferences",
      "allow-get-product-safety-contract",
      "allow-prepare-ai-output-report",
      "allow-export-redacted-diagnostics",
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

  it("uses UIKit's iOS 26 navigation stack without taking over its recognizer", () => {
    expect(nativeBackSwift).toContain(
      "Plugin, UINavigationControllerDelegate",
    );
    expect(nativeBackSwift).toContain(
      "if #available(iOS 26.0, *)",
    );
    expect(nativeBackSwift).toContain(
      "interactiveContentPopGestureRecognizer",
    );
    expect(nativeBackSwift).toContain(
      "navigationController.setViewControllers",
    );
    expect(nativeBackSwift).toContain(
      "appearance.configureWithTransparentBackground()",
    );
    expect(nativeBackSwift).toContain(
      "navigationBar.tintColor = .clear",
    );
    expect(nativeBackSwift).toContain(
      "navigationBar.isUserInteractionEnabled = true",
    );
    expect(nativeBackSwift).toContain(
      "navigationBar.layer.mask = emptyChromeMask",
    );
    expect(nativeBackSwift).toContain(
      "UIBezierPath(rect: .zero).cgPath",
    );
    expect(nativeBackSwift).toContain(
      "sourceController.navigationItem.backBarButtonItem = backItem",
    );
    expect(nativeBackSwift).toContain(
      "backItem.hidesSharedBackground = true",
    );
    expect(nativeBackSwift).toContain(
      "insets.top = -navigationBarHeight",
    );
    expect(nativeBackSwift).toContain("makeRoomTitleHitTarget");
    expect(nativeBackSwift).toContain(
      '"window.dispatchEvent(new Event(\'lorepia:native-room-info\'))"',
    );
    expect(nativeBackSwift).not.toMatch(
      /interactiveContentPopGestureRecognizer\??\.(?:delegate|addTarget)/u,
    );
    expect(nativeBackSwift).toContain(
      "webview.scrollView.panGestureRecognizer.require(",
    );
    expect(nativeBackSwift).toContain(
      "toFail: contentPopGestureRecognizer",
    );
    expect(nativeBackSwift).toContain(
      "webview.allowsBackForwardNavigationGestures = false",
    );
    expect(nativeBackSwift).toContain(
      '"window.dispatchEvent(new Event(\'lorepia:native-back\'))"',
    );
  });

  it("gives Android safe-area ownership to native padding exactly once", () => {
    expect(androidActivity).not.toContain(".style");
    expect(androidActivity).not.toContain("evaluateJavascript");
    expect(androidActivity).toContain(
      "view.setPadding(0, bars.top, 0, bars.bottom)",
    );
    expect(androidActivity).toContain("ViewCompat.requestApplyInsets(webView)");
    expect(appTemplate).toContain(
      '<script src="%sveltekit.assets%/platform-init.js"></script>',
    );
    expect(appTemplate).not.toMatch(/<script(?![^>]+src=)[^>]*>/u);
    expect(platformInit).toContain('userAgent.includes("Android")');
    expect(platformInit).toContain(
      'document.documentElement.dataset.nativeInsetOwner = "android-view-padding"',
    );
    expect(designTokens).toContain(
      ':root[data-native-inset-owner="android-view-padding"]',
    );
    expect(designTokens).toMatch(
      /data-native-inset-owner="android-view-padding"[^}]+--safe-top:\s*0px;[^}]+--safe-bottom:\s*0px;/su,
    );
  });

  it("removes Safari's gray tap flash only inside the iOS app surface", () => {
    expect(
      runPlatformInit({
        userAgent:
          "Mozilla/5.0 (iPhone; CPU iPhone OS 26_0 like Mac OS X) AppleWebKit/605.1.15",
        platform: "iPhone",
        maxTouchPoints: 5,
      }),
    ).toEqual({ nativePlatform: "ios" });
    expect(
      runPlatformInit({
        userAgent:
          "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15) AppleWebKit/605.1.15",
        platform: "MacIntel",
        maxTouchPoints: 5,
      }),
    ).toEqual({ nativePlatform: "ios" });
    expect(
      runPlatformInit({
        userAgent:
          "Mozilla/5.0 (Linux; Android 16) AppleWebKit/537.36 Chrome/140.0 Mobile",
        platform: "Linux armv8l",
        maxTouchPoints: 5,
      }),
    ).toEqual({
      nativePlatform: "android",
      nativeInsetOwner: "android-view-padding",
    });
    expect(
      runPlatformInit({
        userAgent:
          "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 Chrome/140.0",
        platform: "MacIntel",
        maxTouchPoints: 0,
      }),
    ).toEqual({});

    expect(designTokens).toMatch(
      /:root\[data-native-platform="ios"\]\s*\{[^}]+-webkit-tap-highlight-color:\s*rgba\(0,\s*0,\s*0,\s*0\);/su,
    );
    expect(designTokens).toMatch(
      /:root\[data-native-platform="android"\][^{]+\{[^}]+-webkit-tap-highlight-color:\s*rgba\(0,\s*0,\s*0,\s*0\);/su,
    );
    expect(designTokens).toMatch(
      /\.lp-state-layer::after\s*\{[^}]+background:\s*var\(--text-strong\);[^}]+opacity:\s*0;/su,
    );
    expect(designTokens).toMatch(
      /:root\[data-native-platform="ios"\]\s+\.lp-state-layer:active::after\s*\{[^}]+opacity:\s*0\.06;/su,
    );
    expect(designTokens).toMatch(
      /:root\[data-native-platform="android"\]\s+\.lp-state-layer:active::after\s*\{[^}]+opacity:\s*0\.1;/su,
    );
    expect(designTokens).toMatch(
      /:root:not\(\[data-native-platform\]\)\s+\.lp-state-layer:hover::after\s*\{[^}]+opacity:\s*0\.05;/su,
    );
    expect(designTokens).not.toMatch(
      /html,\s*body\s*\{[^}]+-webkit-tap-highlight-color:/su,
    );
  });
});
