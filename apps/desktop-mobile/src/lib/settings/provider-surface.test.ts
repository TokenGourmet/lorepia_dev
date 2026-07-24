import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

// The settings surface lives in SettingsPane so every presentation — today
// the 계정 tab route — renders the same contract-bearing markup.
const source = readFileSync(
  new URL("./SettingsPane.svelte", import.meta.url),
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

  it("makes preference and credential failures recoverable in place", () => {
    expect(source).toContain("appPreferences.unavailable");
    expect(source).toContain("appPreferences.retry()");
    expect(source).toContain("retryCredentialStatus");
    expect(source).toContain("설정을 기기에 저장하지 못했습니다");
    expect(source).toContain("상태 확인 실패");
  });

  it("shows official setup guidance and labels unavailable providers honestly", () => {
    expect(source).toContain("selectedProvider.documentationUrl");
    expect(source).toContain("공식 문서 주소");
    expect(source).not.toContain('target="_blank"');
    expect(source).toContain(
      'provider.status === "configuration-only"',
    );
    expect(source).toContain("현재 버전에서는 연결할 수 없습니다");
    expect(source).not.toContain("연결 후 모델 목록에서 선택");
  });

  it("keeps compact settings controls at a 44px minimum hit height", () => {
    expect(source).toMatch(
      /\.segment button\s*\{[\s\S]*?min-height:\s*var\(--size-touch\)/,
    );
    expect(source).toMatch(
      /\.setting-input input\s*\{[\s\S]*?min-height:\s*var\(--size-touch\)/,
    );
    expect(source).toMatch(
      /\.remove\s*\{[\s\S]*?min-height:\s*var\(--size-touch\)/,
    );
  });
});
