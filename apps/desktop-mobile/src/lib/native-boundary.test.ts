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
const nativeChromeSwift = readFileSync(
  new URL(
    "../../../../crates/tauri-plugin-native-chrome/ios/Sources/NativeChromePlugin.swift",
    import.meta.url,
  ),
  "utf8",
);
const nativeChromeAndroid = readFileSync(
  new URL(
    "../../../../crates/tauri-plugin-native-chrome/android/src/main/java/dev/lorepia/nativechrome/NativeChromePlugin.kt",
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
      "native-chrome:default",
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
      "selectedTabNavigationController(",
    );
    expect(nativeBackSwift).toContain(
      "adoptSharedNavigationHost(",
    );
    expect(nativeBackSwift).toContain(
      "destinationController.hidesBottomBarWhenPushed = true",
    );
    expect(nativeBackSwift).toContain(
      ".setTabBarHidden(false, animated: false)",
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
    expect(nativeBackSwift).toContain(
      'target.accessibilityLabel = "대화 설정 열기"',
    );
    expect(nativeBackSwift).not.toContain(
      'target.accessibilityLabel = "세라핀"',
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
    expect(nativeBackSwift).toContain(
      '"dev.lorepia.nativeBack.prepareChromeUnderlay"',
    );
    expect(nativeBackSwift).toContain(
      "sourceController.view.insertSubview(snapshot, at: 0)",
    );
    expect(nativeBackSwift).toContain(
      "webview.takeSnapshot(with: configuration)",
    );
    expect(nativeBackSwift).toContain(
      "private var snapshotGeneration = 0",
    );
    expect(nativeBackSwift).toContain(
      "private var pendingSnapshotCompletions:",
    );
    expect(nativeBackSwift).toContain(
      "self.snapshotGeneration == generation",
    );
    expect(nativeBackSwift).toContain(
      "cancelSnapshotPreparation()",
    );
    expect(nativeBackSwift).toContain(
      "webview.drawHierarchy(",
    );
    expect(nativeBackSwift).toContain(
      "let snapshot = UIImageView(image: snapshotImage)",
    );
    expect(nativeBackSwift).toContain(
      '"dev.lorepia.nativeBack.clearChromeUnderlay"',
    );
    expect(nativeBackSwift).toContain(
      '"dev.lorepia.nativeBack.willAcquireWebView"',
    );
    expect(nativeBackSwift).toContain(
      '"dev.lorepia.nativeBack.didReleaseWebView"',
    );
    expect(nativeBackSwift).toMatch(
      /func navigationController\([\s\S]*?attachWebView\(to: sourceController, beneathSnapshot: true\)[\s\S]*?destinationController = nil\s*releaseWebViewLease\(\)/u,
    );
  });

  it("keeps four stable native iOS items without rebuilding tab controllers", () => {
    expect(nativeChromeSwift).toContain(
      "SystemTabDockView(interactive: true)",
    );
    expect(nativeChromeSwift).toContain(
      "private final class SystemTabDockView: UITabBar, UITabBarDelegate",
    );
    expect(nativeChromeSwift).toContain(
      "let dock = SystemTabDockView(interactive: false)",
    );
    expect(nativeChromeSwift).toContain(
      "private var pendingTab: NativeChromeTab?",
    );
    expect(nativeChromeSwift).toContain(
      "pendingTab = tab",
    );
    expect(nativeChromeSwift).toContain(
      "pendingGeneration == generation",
    );
    expect(nativeChromeSwift).toContain(
      "rootViewController?.view.bringSubviewToFront",
    );
    expect(nativeChromeSwift).not.toContain("UITabBarController");
    expect(nativeChromeSwift).not.toContain("setViewControllers(slots");
    expect(nativeChromeSwift).not.toContain("tabBarMinimizeBehavior");
    expect(nativeChromeSwift).not.toContain("moveSharedNavigationController");
    expect(nativeChromeSwift).not.toContain("UIGlassEffect");
    expect(nativeChromeSwift).not.toContain("UIGlassContainerEffect");
    expect(nativeChromeSwift).not.toContain(
      "private let indicator",
    );
    expect(nativeChromeSwift).not.toContain("selectionGlass");
    expect(nativeChromeSwift).not.toContain("selectionColor");
    expect(nativeChromeSwift).not.toContain("selectionIndicatorImage");
    expect(nativeChromeSwift).not.toContain("tabBar.tintColor");
    expect(nativeChromeSwift).not.toContain("tabBar.barTintColor");
    expect(nativeChromeSwift).not.toContain("tabBar.standardAppearance");
    expect(nativeChromeSwift).not.toContain("tabBar.scrollEdgeAppearance");
    expect(nativeChromeSwift).not.toContain("tabBar.backgroundImage");
    expect(nativeChromeSwift).toContain(
      'return "house"',
    );
    expect(nativeChromeSwift).toContain(
      'return "house.fill"',
    );
    expect(nativeChromeSwift).toContain(
      'return "books.vertical"',
    );
    expect(nativeChromeSwift).toContain(
      'return "books.vertical.fill"',
    );
    expect(nativeChromeSwift).toContain('return "plus.circle.fill"');
    expect(nativeChromeSwift).toContain('return "person.fill"');
    expect(nativeChromeSwift).toContain("pointSize: 22");
    expect(nativeChromeSwift).toContain(
      "weight: selected ? .semibold : .medium",
    );
    expect(nativeChromeSwift).toContain(
      "image: tab.iconImage(selected: false)",
    );
    expect(nativeChromeSwift).toContain(
      "selectedImage: tab.iconImage(selected: true)",
    );
    expect(nativeChromeSwift).toContain(
      "'lorepia:native-tab'",
    );
    expect(nativeChromeSwift).toContain(
      '"dev.lorepia.nativeBack.prepareChromeUnderlay"',
    );
    expect(nativeChromeSwift).toContain(
      "livePlacement?.dock.apply(",
    );
    expect(nativeChromeSwift).not.toMatch(
      /evaluateJavaScript\([^)]*(?:href|label|icon)/su,
    );
  });

  it("owns the same four Android tabs with one native Material view and no second runtime", () => {
    expect(nativeChromeAndroid).toContain(
      "class NativeChromePlugin(private val activity: Activity)",
    );
    expect(nativeChromeAndroid).toContain(
      "BottomNavigationView(activity)",
    );
    for (const contract of [
      '"home",\n    "홈"',
      '"library",\n    "서재"',
      '"create",\n    "생성"',
      '"account",\n    "계정"',
    ]) {
      expect(nativeChromeAndroid).toContain(contract);
    }
    expect(nativeChromeAndroid).toContain(
      "NavigationBarView.LABEL_VISIBILITY_LABELED",
    );
    expect(nativeChromeAndroid).toContain(
      "activity.addContentView(nativeDock, layoutParams)",
    );
    expect(nativeChromeAndroid).toContain(
      "setOnItemSelectedListener",
    );
    expect(nativeChromeAndroid).toContain(
      "Material commits the checked item immediately",
    );
    expect(nativeChromeAndroid).toContain(
      "val displayedTab = pendingTab ?: state.selectedTab",
    );
    expect(nativeChromeAndroid).toContain(
      "WindowInsetsCompat.Type.systemBars()",
    );
    expect(nativeChromeAndroid).toContain(
      "WindowInsetsCompat.Type.displayCutout()",
    );
    expect(nativeChromeAndroid).toContain(
      "WindowInsetsCompat.Type.ime()",
    );
    expect(nativeChromeAndroid).toContain(
      "'lorepia:native-tab'",
    );
    expect(nativeChromeAndroid).not.toContain("WebView(activity)");
    expect(nativeChromeAndroid).not.toContain("OnBackPressedCallback");
  });

  it("gives Android safe-area and IME ownership to one native WebView boundary", () => {
    expect(androidActivity).not.toContain(".style");
    expect(androidActivity).not.toContain("evaluateJavascript");
    expect(androidActivity).toContain("WindowInsetsCompat.Type.systemBars()");
    expect(androidActivity).toContain("WindowInsetsCompat.Type.displayCutout()");
    expect(androidActivity).toContain("WindowInsetsCompat.Type.ime()");
    expect(androidActivity).toContain(
      "imeVisible = insets.isVisible(WindowInsetsCompat.Type.ime())",
    );
    expect(androidActivity).toContain("layoutParams.setMargins(");
    expect(androidActivity).toContain(
      ".setInsets(WindowInsetsCompat.Type.ime(), Insets.NONE)",
    );
    expect(androidActivity).toContain(".setInsets(safeTypes, Insets.NONE)");
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

  it("keeps iOS app chrome out of text selection without disabling editable text", () => {
    const controlRule = designTokens.match(
      /:root\[data-native-platform="ios"\]\s+:where\(([^}]+)\)\s*\{([^}]+-webkit-touch-callout:\s*none;[^}]*)\}/su,
    );
    expect(controlRule).not.toBeNull();

    const selectors = controlRule?.[1] ?? "";
    const declarations = controlRule?.[2] ?? "";
    expect(selectors).toContain("a[href]");
    expect(selectors).toContain("button");
    expect(selectors).toContain('[role="button"]');
    expect(selectors).toContain('[role="tab"]');
    expect(selectors).toContain('[role="menuitem"]');
    expect(selectors).toContain(
      'label:has(> input:is([type="checkbox"], [type="radio"]))',
    );
    expect(selectors).not.toMatch(
      /(?:^|[\s,>+~])(?:textarea|\[contenteditable)/u,
    );
    expect(declarations).toMatch(/-webkit-user-select:\s*none;/u);
    expect(declarations).toMatch(/(?:^|\s)user-select:\s*none;/u);
    expect(declarations).toMatch(/-webkit-touch-callout:\s*none;/u);
    expect(designTokens).not.toMatch(
      /:root\[data-native-platform="ios"\][^{]*\b(?:input|textarea)\s*(?:,|\{)[^}]+user-select:\s*none;/su,
    );
  });
});
