import type { ChatMessage } from "$lib/chat/types";

/**
 * The native message page is canonical and ordered by ordinal. Because the
 * requested cursor is the first currently visible stored message, every
 * returned item belongs before the current transcript. Keep that ordering
 * while dropping replayed identities at the page boundary.
 */
export function prependOlderMessages(
  current: readonly ChatMessage[],
  older: readonly ChatMessage[],
): ChatMessage[] {
  const seen = new Set(current.map((message) => message.id));
  const uniqueOlder: ChatMessage[] = [];

  for (const message of older) {
    if (seen.has(message.id)) continue;
    seen.add(message.id);
    uniqueOlder.push(message);
  }

  return [...uniqueOlder, ...current];
}

/**
 * Keep the previously visible transcript anchored after older DOM rows are
 * inserted above it. Clamp because a small status-row height change can make
 * the total delta negative at the very top.
 */
export function preservedPrependScrollTop(
  previousTop: number,
  previousHeight: number,
  nextHeight: number,
): number {
  return Math.max(0, previousTop + nextHeight - previousHeight);
}
