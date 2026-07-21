import {
  MAX_MESSAGE_PAGE,
  type MessageCursor,
  type MessagePage,
  type StoredMessage,
} from "./client";

export type BoundedHistoryWindow = Readonly<{
  items: readonly StoredMessage[];
  hasMore: boolean;
  olderCursor: MessageCursor | null;
  capacity: number;
}>;

function validateCapacity(capacity: number): void {
  if (
    !Number.isSafeInteger(capacity) ||
    capacity < 1 ||
    capacity > MAX_MESSAGE_PAGE
  ) {
    throw new Error("INVALID_HISTORY_WINDOW");
  }
}

function validateCanonicalItems(items: readonly StoredMessage[]): void {
  const ids = new Set<string>();
  const ordinals = new Set<number>();
  const chatId = items[0]?.chatId;
  let previousOrdinal = 0;
  for (const item of items) {
    if (
      item.chatId !== chatId ||
      ids.has(item.id) ||
      ordinals.has(item.ordinal) ||
      item.ordinal <= previousOrdinal
    ) {
      throw new Error("INVALID_HISTORY_WINDOW");
    }
    ids.add(item.id);
    ordinals.add(item.ordinal);
    previousOrdinal = item.ordinal;
  }
}

function sameStoredMessage(left: StoredMessage, right: StoredMessage): boolean {
  return (
    left.id === right.id &&
    left.chatId === right.chatId &&
    left.ordinal === right.ordinal &&
    left.role === right.role &&
    left.text === right.text &&
    left.state === right.state &&
    left.createdAtMs === right.createdAtMs &&
    left.updatedAtMs === right.updatedAtMs
  );
}

function freezeWindow(
  items: readonly StoredMessage[],
  hasMore: boolean,
  olderCursor: MessageCursor | null,
  capacity: number,
): BoundedHistoryWindow {
  validateCapacity(capacity);
  validateCanonicalItems(items);
  if (
    items.length > capacity ||
    hasMore !== (olderCursor !== null) ||
    (olderCursor !== null &&
      (olderCursor.chatId !== items[0]?.chatId ||
        olderCursor.ordinal !== items[0]?.ordinal))
  ) {
    throw new Error("INVALID_HISTORY_WINDOW");
  }
  return Object.freeze({
    items: Object.freeze([...items]),
    hasMore,
    olderCursor:
      olderCursor === null
        ? null
        : Object.freeze({
            chatId: olderCursor.chatId,
            ordinal: olderCursor.ordinal,
          }),
    capacity,
  });
}

export function createBoundedHistoryWindow(
  page: MessagePage,
  capacity = MAX_MESSAGE_PAGE,
): BoundedHistoryWindow {
  return freezeWindow(page.items, page.hasMore, page.olderCursor, capacity);
}

/**
 * Adds messages newer than the current tail and evicts the oldest retained
 * messages. Identity/ordinal conflicts fail closed instead of silently
 * corrupting the visible chronology.
 */
export function appendHistoryItems(
  window: BoundedHistoryWindow,
  incoming: readonly StoredMessage[],
): BoundedHistoryWindow {
  validateCanonicalItems(incoming);
  if (
    incoming.some(
      (item) =>
        window.items[0] !== undefined && item.chatId !== window.items[0].chatId,
    )
  ) {
    throw new Error("INVALID_HISTORY_WINDOW");
  }
  const byId = new Map(window.items.map((item) => [item.id, item]));
  const byOrdinal = new Map(window.items.map((item) => [item.ordinal, item]));
  const merged = [...window.items];
  for (const item of incoming) {
    const sameId = byId.get(item.id);
    const sameOrdinal = byOrdinal.get(item.ordinal);
    if (sameId !== undefined || sameOrdinal !== undefined) {
      if (
        sameId === undefined ||
        sameOrdinal === undefined ||
        sameId.id !== sameOrdinal.id ||
        !sameStoredMessage(sameId, item)
      ) {
        throw new Error("INVALID_HISTORY_WINDOW");
      }
      continue;
    }
    if (item.ordinal <= (merged.at(-1)?.ordinal ?? 0)) {
      throw new Error("INVALID_HISTORY_WINDOW");
    }
    merged.push(item);
    byId.set(item.id, item);
    byOrdinal.set(item.ordinal, item);
  }

  const trimmed = merged.length > window.capacity;
  const items = trimmed ? merged.slice(-window.capacity) : merged;
  const hasMore = window.hasMore || trimmed;
  const olderCursor = hasMore
    ? { chatId: items[0]!.chatId, ordinal: items[0]!.ordinal }
    : null;
  return freezeWindow(items, hasMore, olderCursor, window.capacity);
}

/**
 * Prepends an explicitly requested older page only while it fits in the
 * current bounded viewport. This helper intentionally has no newer cursor, so
 * it fails closed at capacity instead of discarding an unrecoverable newer
 * tail. A future scrolling UI must add a bidirectional viewport contract
 * before moving a full window backward.
 */
export function prependHistoryPage(
  window: BoundedHistoryWindow,
  requested: MessageCursor,
  page: MessagePage,
): BoundedHistoryWindow {
  if (
    window.olderCursor === null ||
    requested.chatId !== window.olderCursor.chatId ||
    requested.ordinal !== window.olderCursor.ordinal ||
    (page.olderCursor !== null &&
      (page.olderCursor.chatId !== requested.chatId ||
        page.olderCursor.ordinal >= requested.ordinal))
  ) {
    throw new Error("INVALID_HISTORY_CURSOR");
  }
  validateCanonicalItems(page.items);
  if (
    page.items.some(
      (item) =>
        item.chatId !== requested.chatId || item.ordinal >= requested.ordinal,
    )
  ) {
    throw new Error("INVALID_HISTORY_CURSOR");
  }

  const existingById = new Map(window.items.map((item) => [item.id, item]));
  const existingByOrdinal = new Map(
    window.items.map((item) => [item.ordinal, item]),
  );
  const older: StoredMessage[] = [];
  for (const item of page.items) {
    const sameId = existingById.get(item.id);
    const sameOrdinal = existingByOrdinal.get(item.ordinal);
    if (sameId !== undefined || sameOrdinal !== undefined) {
      if (
        sameId === undefined ||
        sameOrdinal === undefined ||
        sameId.id !== sameOrdinal.id ||
        !sameStoredMessage(sameId, item)
      ) {
        throw new Error("INVALID_HISTORY_WINDOW");
      }
      continue;
    }
    older.push(item);
  }
  const items = [...older, ...window.items];
  if (items.length > window.capacity) {
    throw new Error("HISTORY_WINDOW_CAP_REACHED");
  }
  return freezeWindow(items, page.hasMore, page.olderCursor, window.capacity);
}

/** Keeps transient UI messages bounded by identity without prescribing a UI. */
export function boundedTailById<T extends Readonly<{ id: string }>>(
  items: readonly T[],
  capacity = MAX_MESSAGE_PAGE,
): readonly T[] {
  validateCapacity(capacity);
  const seen = new Set<string>();
  const deduplicated: T[] = [];
  for (let index = items.length - 1; index >= 0; index -= 1) {
    const item = items[index]!;
    if (seen.has(item.id)) continue;
    seen.add(item.id);
    deduplicated.push(item);
    if (deduplicated.length === capacity) break;
  }
  deduplicated.reverse();
  return Object.freeze(deduplicated);
}
