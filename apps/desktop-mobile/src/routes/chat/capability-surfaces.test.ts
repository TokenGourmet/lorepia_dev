import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const librarySource = readFileSync(
  new URL("../+page.svelte", import.meta.url),
  "utf8",
);
const characterSource = readFileSync(
  new URL("../character/[id]/+page.svelte", import.meta.url),
  "utf8",
);
const infoSource = readFileSync(
  new URL("./info/+page.svelte", import.meta.url),
  "utf8",
);
const reportSource = readFileSync(
  new URL("./report/+page.svelte", import.meta.url),
  "utf8",
);

describe("character-specific chat capability surfaces", () => {
  it("routes every library and character-detail entry to its own chat", () => {
    expect(librarySource).toContain(
      "`/chat?character=${encodeURIComponent(characterId)}`",
    );
    expect(librarySource).toContain("openChat(event, character.id)");
    expect(characterSource).toContain(
      "`/chat?character=${encodeURIComponent(character.id)}`",
    );
  });

  it("uses durable message previews when available and keeps local fallbacks", () => {
    expect(librarySource).toContain("storageClient.listChats(100, before)");
    expect(librarySource).toContain(
      "storageClient.loadChatMessages(chat.id, 1)",
    );
    expect(librarySource).toContain(
      "if (message === undefined) return null;",
    );
    expect(librarySource).not.toContain("아직 대화가 없습니다.");
    expect(librarySource).toContain(
      "chatPreview: persistedPreviews[character.id] ?? null",
    );
    expect(librarySource).toContain("{#if character.chatPreview}");
    expect(librarySource).toContain(
      '<span class="preview introduction">{character.tagline}</span>',
    );
    expect(librarySource).toContain(
      "Library samples remain usable if local persistence is unavailable.",
    );
  });

  it("gives the library search controls a 44px hit box", () => {
    expect(librarySource).toMatch(
      /\.search\s*\{[\s\S]*?height:\s*var\(--size-touch\)/,
    );
    expect(librarySource).toMatch(
      /\.clear\s*\{[\s\S]*?width:\s*var\(--size-touch\);[\s\S]*?height:\s*var\(--size-touch\)/,
    );
  });

  it("connects deletion to an explicit destructive alert", () => {
    expect(infoSource).toContain("storageClient.deleteChat(targetChatId)");
    expect(infoSource).toContain('role="alertdialog"');
    expect(infoSource).toContain("<dialog");
    expect(infoSource).toContain("deleteDialog?.showModal()");
    expect(infoSource).toContain("이 작업은 되돌릴 수 없습니다.");
    expect(infoSource).toMatch(
      /\.delete-actions button\s*\{[\s\S]*?min-height:\s*var\(--size-touch\)/,
    );
  });

  it("pops nested chat screens without adding a back-navigation loop", () => {
    expect(infoSource).toContain("window.history.back()");
    expect(infoSource).toContain("void goto(chatHref, { replaceState: true })");
    expect(infoSource).toContain("onclick={handleBackClick}");
    expect(infoSource).toContain("onclick={openReport}");
    expect(reportSource).toContain("window.history.back()");
    expect(reportSource).toContain("void goto(infoHref, { replaceState: true })");
    expect(reportSource).toContain("isMatchingInfoHref(candidate)");
  });

  it("creates a local report draft from a selected durable assistant output", () => {
    expect(reportSource).toContain(
      'message.role === "assistant"',
    );
    expect(reportSource).toContain('page.url.searchParams.get("chatId")');
    expect(reportSource).toContain(
      "requested.characterId === character.id",
    );
    expect(reportSource).toContain("resolveReportChatId(loaded.chat.id)");
    expect(reportSource).toContain("selectedMessageId");
    expect(reportSource).toContain("requestAiOutputReport({");
    expect(reportSource).toContain("includeSelectedOutput");
    expect(reportSource).toContain("MAX_REPORT_EXCERPT_BYTES");
    expect(reportSource).toContain(
      "원격 제출이나 네트워크",
    );
    expect(reportSource).toContain(
      "어디에도 전송되지 않았습니다.",
    );
    expect(reportSource).toContain(
      "저장 기록만으로는 자동 판별할 수 없습니다.",
    );
    expect(reportSource).toMatch(
      /\.page-status button,[\s\S]*?min-height:\s*var\(--size-touch\)/,
    );
  });
});
