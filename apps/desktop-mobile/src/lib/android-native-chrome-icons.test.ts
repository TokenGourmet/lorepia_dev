import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { describe, expect, it } from "vitest";

const ANDROID_ROOT = resolve(
  import.meta.dirname,
  "../../../../crates/tauri-plugin-native-chrome/android/src/main",
);
const DRAWABLE_ROOT = resolve(ANDROID_ROOT, "res/drawable");
const KOTLIN_SOURCE = readFileSync(
  resolve(
    ANDROID_ROOT,
    "java/dev/lorepia/nativechrome/NativeChromePlugin.kt",
  ),
  "utf8",
);
const TAB_NAMES = [
  "home",
  "library",
  "create",
  "account",
] as const;

function readDrawable(name: string): string {
  return readFileSync(resolve(DRAWABLE_ROOT, name), "utf8");
}

describe("Android native chrome icon states", () => {
  it("keeps the bar geometry while using checked-state selectors", () => {
    expect(KOTLIN_SOURCE).toContain("itemIconSize = dp(22)");
    expect(KOTLIN_SOURCE).toContain("minimumHeight = dp(80)");
    expect(KOTLIN_SOURCE).toContain("setItemPaddingTop(dp(12))");
    expect(KOTLIN_SOURCE).toContain(
      "setItemPaddingBottom(dp(16))",
    );

    for (const tab of TAB_NAMES) {
      expect(KOTLIN_SOURCE).toContain(
        `R.drawable.ic_native_tab_${tab}_selector`,
      );
      const selector = readDrawable(
        `ic_native_tab_${tab}_selector.xml`,
      );
      expect(selector).toContain(
        'android:state_checked="true"',
      );
      expect(selector).toContain(
        `@drawable/ic_native_tab_${tab}_filled`,
      );
      expect(selector).toContain(
        `@drawable/ic_native_tab_${tab}`,
      );
    }
  });

  it("keeps outline and filled variants in one 22dp, 24-unit frame", () => {
    for (const tab of TAB_NAMES) {
      const outline = readDrawable(`ic_native_tab_${tab}.xml`);
      const filled = readDrawable(
        `ic_native_tab_${tab}_filled.xml`,
      );

      for (const drawable of [outline, filled]) {
        expect(drawable).toContain('android:width="22dp"');
        expect(drawable).toContain('android:height="22dp"');
        expect(drawable).toContain(
          'android:viewportWidth="24"',
        );
        expect(drawable).toContain(
          'android:viewportHeight="24"',
        );
      }
      expect(outline).toContain(
        'android:fillColor="@android:color/transparent"',
      );
      expect(filled).toContain(
        'android:fillColor="#FF000000"',
      );
    }
  });

  it("selects optimistically without letting stale route commits bounce the indicator", () => {
    expect(KOTLIN_SOURCE).toContain(
      "private var pendingTab: NativeChromeTab?",
    );
    expect(KOTLIN_SOURCE).toContain("pendingTab = tab");
    expect(KOTLIN_SOURCE).toContain(
      "val displayedTab = pendingTab ?: state.selectedTab",
    );
    expect(KOTLIN_SOURCE).toContain(
      "pendingSelectionGeneration == generation",
    );
    expect(KOTLIN_SOURCE).toContain(
      "// Material commits the checked item immediately.",
    );
  });
});
