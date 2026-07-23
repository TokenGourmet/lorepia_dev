import { describe, expect, it } from "vitest";

import { isChosungQuery, matchesQuery, toChosung } from "./search";

const SERAPHINE = {
  name: "세라핀",
  tagline: "달빛 서고의 사서",
  lastMessage: "짧은 우화집도 함께 챙겨드릴게요.",
};

describe("toChosung", () => {
  it("reduces composed syllables to their initial consonant", () => {
    expect(toChosung("세라핀")).toBe("ㅅㄹㅍ");
    expect(toChosung("윤슬")).toBe("ㅇㅅ");
    expect(toChosung("아델")).toBe("ㅇㄷ");
  });

  it("covers the first and last syllable of the Hangul block", () => {
    expect(toChosung("가")).toBe("ㄱ");
    expect(toChosung("힣")).toBe("ㅎ");
  });

  it("leaves spaces, latin, and punctuation in place", () => {
    expect(toChosung("달빛 서고")).toBe("ㄷㅂ ㅅㄱ");
    expect(toChosung("LorePia 서재")).toBe("LorePia ㅅㅈ");
  });
});

describe("isChosungQuery", () => {
  it("accepts bare initial consonants only", () => {
    expect(isChosungQuery("ㅅㄹㅍ")).toBe(true);
    expect(isChosungQuery("ㄲ")).toBe(true);
  });

  it("rejects empty, composed, vowel, and mixed input", () => {
    expect(isChosungQuery("")).toBe(false);
    expect(isChosungQuery("세라핀")).toBe(false);
    expect(isChosungQuery("ㅏ")).toBe(false);
    expect(isChosungQuery("ㅅㄹ핀")).toBe(false);
    expect(isChosungQuery("ㅅ ㄹ")).toBe(false);
  });

  it("rejects compound finals that never appear as an initial", () => {
    expect(isChosungQuery("ㄳ")).toBe(false);
    expect(isChosungQuery("ㄺ")).toBe(false);
  });
});

describe("matchesQuery", () => {
  it("matches everything on an empty or whitespace query", () => {
    expect(matchesQuery(SERAPHINE, "")).toBe(true);
    expect(matchesQuery(SERAPHINE, "   ")).toBe(true);
  });

  it("matches name, tagline, and last message on a text query", () => {
    expect(matchesQuery(SERAPHINE, "세라")).toBe(true);
    expect(matchesQuery(SERAPHINE, "달빛")).toBe(true);
    expect(matchesQuery(SERAPHINE, "우화집")).toBe(true);
    expect(matchesQuery(SERAPHINE, "등대")).toBe(false);
  });

  it("matches a partial initial-consonant run", () => {
    expect(matchesQuery(SERAPHINE, "ㅅㄹㅍ")).toBe(true);
    expect(matchesQuery(SERAPHINE, "ㅅㄹ")).toBe(true);
    expect(matchesQuery(SERAPHINE, "ㄷㅂ")).toBe(true);
    expect(matchesQuery(SERAPHINE, "ㅋㅇ")).toBe(false);
  });

  it("does not let an initial-consonant run span a word boundary", () => {
    expect(matchesQuery(SERAPHINE, "ㅍㄷ")).toBe(false);
  });

  it("keeps initial-consonant queries out of the message body", () => {
    expect(
      matchesQuery(
        { name: "카이", tagline: "별을 세는 등대지기", lastMessage: "짧은 우화집" },
        "ㅉㅇ",
      ),
    ).toBe(false);
  });

  it("folds case on latin queries", () => {
    const noah = {
      name: "노아",
      tagline: "비 오는 날의 라디오 DJ",
      lastMessage: "다음 곡은",
    };
    expect(matchesQuery(noah, "dj")).toBe(true);
    expect(matchesQuery(noah, "DJ")).toBe(true);
  });
});
