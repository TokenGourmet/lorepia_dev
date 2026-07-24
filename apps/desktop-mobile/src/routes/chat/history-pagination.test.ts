import { describe, expect, it } from "vitest";

import type { ChatMessage } from "$lib/chat/types";
import {
  prependOlderMessages,
  preservedPrependScrollTop,
} from "./history-pagination";

function message(id: string, minute: number): ChatMessage {
  return {
    id,
    role: minute % 2 === 0 ? "character" : "user",
    text: id,
    sentAt: new Date(2026, 0, 1, 0, minute),
  };
}

describe("chat history pagination", () => {
  it("deduplicates a replayed boundary and keeps both pages chronological", () => {
    const current = [message("m3", 3), message("m4", 4)];
    const older = [
      message("m1", 1),
      message("m2", 2),
      message("m3", 3),
      message("m2", 2),
    ];

    expect(
      prependOlderMessages(current, older).map((item) => item.id),
    ).toEqual(["m1", "m2", "m3", "m4"]);
  });

  it("preserves the visible anchor by adding the inserted height", () => {
    expect(preservedPrependScrollTop(24, 800, 1_220)).toBe(444);
    expect(preservedPrependScrollTop(0, 100, 80)).toBe(0);
  });
});
