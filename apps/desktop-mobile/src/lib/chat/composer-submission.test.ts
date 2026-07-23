import { describe, expect, it } from "vitest";

import {
  resolveComposerNotice,
  resolveComposerSubmission,
} from "./composer-submission";

describe("composer submission", () => {
  it("preserves a valid draft behind an exact send-time block reason", () => {
    expect(
      resolveComposerSubmission(
        "  안녕  ",
        false,
        "OpenAI API 키가 설정되지 않아 메시지를 보낼 수 없습니다.",
      ),
    ).toEqual({
      kind: "blocked",
      reason: "OpenAI API 키가 설정되지 않아 메시지를 보낼 수 없습니다.",
    });
  });

  it("sends normalized text only when the conversation is ready", () => {
    expect(resolveComposerSubmission("  안녕  ", false, null)).toEqual({
      kind: "send",
      text: "안녕",
    });
  });

  it("ignores empty drafts and submissions while streaming", () => {
    expect(resolveComposerSubmission("   ", false, null)).toEqual({
      kind: "ignore",
    });
    expect(resolveComposerSubmission("안녕", true, null)).toEqual({
      kind: "ignore",
    });
  });

  it("keeps provider readiness notices live after a blocked send", () => {
    const validationReason = "입력 형식이 올바르지 않아 보낼 수 없습니다.";

    expect(
      resolveComposerNotice(
        true,
        "프로바이더 자격 증명을 확인하는 중입니다.",
        validationReason,
      ),
    ).toBe("프로바이더 자격 증명을 확인하는 중입니다.");
    expect(
      resolveComposerNotice(
        true,
        "OpenAI API 키가 설정되지 않아 메시지를 보낼 수 없습니다.",
        validationReason,
      ),
    ).toBe("OpenAI API 키가 설정되지 않아 메시지를 보낼 수 없습니다.");
    expect(resolveComposerNotice(true, null, validationReason)).toBe(
      validationReason,
    );
    expect(resolveComposerNotice(false, null, validationReason)).toBeNull();
  });
});
