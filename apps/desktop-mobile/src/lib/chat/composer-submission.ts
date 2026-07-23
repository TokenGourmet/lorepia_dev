export type ComposerSubmission =
  | Readonly<{ kind: "ignore" }>
  | Readonly<{ kind: "blocked"; reason: string }>
  | Readonly<{ kind: "send"; text: string }>;

export function resolveComposerNotice(
  requested: boolean,
  dynamicReason?: string | null,
  inputReason?: string | null,
): string | null {
  if (!requested) {
    return null;
  }

  const currentDynamicReason = dynamicReason?.trim() ?? "";
  if (currentDynamicReason.length > 0) {
    return currentDynamicReason;
  }

  const currentInputReason = inputReason?.trim() ?? "";
  return currentInputReason.length > 0 ? currentInputReason : null;
}

export function resolveComposerSubmission(
  draft: string,
  busy: boolean,
  blockedReason?: string | null,
): ComposerSubmission {
  const text = draft.trim();
  if (busy || text.length === 0) {
    return Object.freeze({ kind: "ignore" });
  }

  const reason = blockedReason?.trim() ?? "";
  if (reason.length > 0) {
    return Object.freeze({ kind: "blocked", reason });
  }

  return Object.freeze({ kind: "send", text });
}
