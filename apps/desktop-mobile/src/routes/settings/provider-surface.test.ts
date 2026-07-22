import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const source = readFileSync(
  new URL("./+page.svelte", import.meta.url),
  "utf8",
);

describe("provider settings surface", () => {
  it("renders the product-owned provider catalog", () => {
    expect(source).toContain("LLM_PROVIDER_CATALOG");
    expect(source).toContain("LLM 제공자 선택");
    expect(source).toContain("selectedProvider.setupFields");
    expect(source).toContain('type="radio"');
    expect(source).not.toContain("aria-pressed");
  });

  // The pre-wiring "collect nothing" guard retired when this screen was wired
  // to the native vault through $lib/providers/credentials. The remaining
  // permanent contract: secrets flow only through that write-only module, are
  // masked at entry, and never touch web storage or direct transports.
  it("collects the key only through the write-only native vault path", () => {
    expect(source).toContain('from "$lib/providers/credentials"');
    expect(source).toMatch(/type="password"/);
    expect(source).toContain('autocomplete="off"');
    expect(source).not.toMatch(/localStorage|sessionStorage|document\.cookie/i);
    expect(source).not.toMatch(/\bfetch\s*\(|\binvoke\s*\(/i);
    expect(source).toContain("keyDraft = \"\"");
    expect(source).toMatch(/다시 읽어오는\s+경로 자체가 없습니다/);
  });

  it("activates a volatile non-secret provider and model profile", () => {
    expect(source).toContain(
      'from "$lib/providers/active-profile.svelte"',
    );
    expect(source).toContain('id="provider-model-id"');
    expect(source).toContain("activeProviderProfile.setModelId");
    expect(source).not.toMatch(/defaultModel|localStorage|sessionStorage/i);
  });

  it("persists only typed non-secret provider and display preferences", () => {
    expect(source).toContain(
      'from "$lib/storage/app-preferences.svelte"',
    );
    expect(source).toContain("appPreferences.setProvider");
    expect(source).toContain("appPreferences.setModelId");
    expect(source).toContain("appPreferences.setTheme");
    expect(source).toContain("appPreferences.setDefaultMode");
  });
});
