import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const source = readFileSync(new URL("./+page.svelte", import.meta.url), "utf8");

describe("first chat surface", () => {
  it("binds the active profile to the native stream adapter", () => {
    expect(source).toContain(
      'from "$lib/providers/active-profile.svelte"',
    );
    expect(source).toContain('from "$lib/providers/stream"');
    expect(source).toContain("startFirstChatStream(profile, text");
    expect(source).toContain("streaming: true");
  });

  it("exposes stop and clears streaming state for every terminal result", () => {
    expect(source).toContain("onCancel={handleCancel}");
    expect(source).toContain("handle.cancel()");
    expect(source).toContain("onTerminal(terminal)");
    expect(source).toContain("streaming: false");
  });
});
