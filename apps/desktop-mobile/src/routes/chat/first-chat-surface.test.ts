import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const source = readFileSync(new URL("./+page.svelte", import.meta.url), "utf8");

describe("first chat surface", () => {
  it("binds the active profile to the native stream adapter", () => {
    expect(source).toContain(
      'from "$lib/providers/active-profile.svelte"',
    );
    expect(source).toContain('from "$lib/providers/stream"');
    expect(source).toContain(
      "startFirstChatStream(profile, targetChatId, text",
    );
    expect(source).toContain("streaming: true");
  });

  it("loads the canonical SQLite history and refreshes it after terminal", () => {
    expect(source).toContain('from "$lib/storage/chat-history"');
    expect(source).toContain("loadOrCreateFirstChat()");
    expect(source).toContain("storageClient.loadChatMessages(targetChatId)");
    expect(source).toContain("void reloadHistory(targetChatId)");
    expect(source).toContain("if (nativeTurnStarted)");
    expect(source).not.toContain('id: "m1"');
  });

  it("terminates stale native owner streams before reload reads history", () => {
    expect(source).toContain("resetProviderStreamOwner");
    expect(source).toMatch(
      /await resetProviderStreamOwner\(\);[\s\S]*await appPreferences\.hydrate\(\);[\s\S]*loadOrCreateFirstChat\(\)/,
    );
  });

  it("preserves the optimistic user message when native start is rejected", () => {
    expect(source).toContain("let nativeTurnStarted = false");
    expect(source).toContain("onStarted()");
    expect(source).toContain("nativeTurnStarted = true");
    expect(source).toMatch(
      /if \(nativeTurnStarted\) \{\s*void reloadHistory\(targetChatId\);/,
    );
  });

  it("exposes stop and clears streaming state for every terminal result", () => {
    expect(source).toContain("onCancel={handleCancel}");
    expect(source).toContain("handle.cancel()");
    expect(source).toContain("onTerminal(terminal)");
    expect(source).toContain("streaming: false");
  });

  it("keeps drafting available and reports exact readiness failures on send", () => {
    expect(source).toContain(
      "activeProviderProfile.sendBlockReason",
    );
    expect(source).toContain("blockedReason={sendBlockReason}");
    expect(source).toContain("validate={firstChatInputBlockReason}");
    expect(source).toContain(
      "로컬 저장소를 사용할 수 없어 메시지를 보낼 수 없습니다.",
    );
    expect(source).toContain(
      "대화를 준비하는 중이라 아직 메시지를 보낼 수 없습니다.",
    );
    expect(source).not.toContain(
      "disabled={activeProviderProfile.current === null || chatId === null}",
    );
  });

  it("starts the interactive fallback across the complete chat surface", () => {
    expect(source).toMatch(
      /<div\s+class="screen"\s+use:contentSwipeBack=/,
    );
    expect(source).toMatch(
      /<div\s+class="scroll"\s+bind:this=\{scrollRegion\}\s*>/,
    );
  });

  it("batches streaming deltas per frame and drains them before terminal", () => {
    expect(source).toContain('from "$lib/chat/frame-chunk-buffer"');
    expect(source).toContain("deltaBuffer.append(delta)");
    expect(source).toContain("const pendingText = deltaBuffer.close()");
    expect(source).toContain("activeDeltaBuffer?.flush()");
    expect(source).toContain("streamingChunks");
    expect(source).toContain("materializeStreamingText");
    expect(source).not.toContain("text: `${message.text}${delta}`");
    expect(source).not.toMatch(
      /onDelta\(delta\)\s*\{\s*replaceMessage\(assistantId/,
    );
  });

  it("keeps runtime layout changes compatible with the strict style CSP", () => {
    expect(source).not.toContain("style:");
    expect(source).not.toMatch(/\sstyle=/);
    expect(source).toContain("height={keyboardInset.value}");
    // The interactive back drag is the one runtime-styled surface; its CSSOM
    // writes live behind the content-back action, never in this markup.
    expect(source).toContain("use:contentSwipeBack");
    expect(source).toContain("enabled: !nativeBackActive");
  });
});
