export type AndroidNativeBackPlan =
  | { kind: "history" }
  | { kind: "replace"; href: string };

function safeLocalHref(value: unknown): string | null {
  return typeof value === "string" &&
    value.startsWith("/") &&
    !value.startsWith("//")
    ? value
    : null;
}

function stateBackHref(state: unknown): string | null {
  if (typeof state !== "object" || state === null) return null;
  return safeLocalHref(
    (state as Record<string, unknown>).backHref,
  );
}

function sameCharacter(
  current: URL,
  candidate: URL,
): boolean {
  return (
    candidate.searchParams.get("character") ===
    current.searchParams.get("character")
  );
}

function matchingHistoryTarget(
  current: URL,
  candidateHref: string,
): boolean {
  const candidate = new URL(candidateHref, current.origin);
  if (current.pathname === "/chat/report") {
    return (
      candidate.pathname === "/chat/info" &&
      sameCharacter(current, candidate)
    );
  }
  if (current.pathname === "/chat/info") {
    return (
      candidate.pathname === "/chat" &&
      sameCharacter(current, candidate)
    );
  }
  return current.pathname === "/chat";
}

function chatQueryTarget(
  pathname: "/chat" | "/chat/info",
  current: URL,
  includeChatId: boolean,
): string {
  const query = new URLSearchParams();
  const character = current.searchParams.get("character");
  const chatId = current.searchParams.get("chatId");
  if (character !== null) query.set("character", character);
  if (includeChatId && chatId !== null) query.set("chatId", chatId);
  const search = query.toString();
  return `${pathname}${search === "" ? "" : `?${search}`}`;
}

export function isAndroidNativeBackRoute(
  pathname: string,
): boolean {
  return (
    pathname === "/chat" ||
    pathname === "/chat/info" ||
    pathname === "/chat/report" ||
    pathname.startsWith("/character/") ||
    pathname === "/import" ||
    pathname === "/community"
  );
}

export function planAndroidNativeBack(
  current: URL,
  state: unknown,
  historyLength: number,
): AndroidNativeBackPlan | null {
  if (!isAndroidNativeBackRoute(current.pathname)) return null;

  const backHref = stateBackHref(state);
  if (
    backHref !== null &&
    historyLength > 1 &&
    matchingHistoryTarget(current, backHref)
  ) {
    return { kind: "history" };
  }

  if (current.pathname === "/chat/report") {
    return {
      kind: "replace",
      href: chatQueryTarget("/chat/info", current, true),
    };
  }
  if (current.pathname === "/chat/info") {
    return {
      kind: "replace",
      href: chatQueryTarget("/chat", current, false),
    };
  }
  if (current.pathname === "/chat") {
    return {
      kind: "replace",
      href: backHref ?? "/",
    };
  }
  if (
    current.pathname === "/import" ||
    current.pathname === "/community"
  ) {
    return { kind: "replace", href: "/home" };
  }
  return { kind: "replace", href: "/" };
}
