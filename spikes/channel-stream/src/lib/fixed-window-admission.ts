export type FixedWindowAdmissionOptions = Readonly<{
  maxAttempts: number;
  windowMs: number;
  clock?: () => number;
}>;

export type FixedWindowAdmission = Readonly<{
  consume: () => boolean;
}>;

export function createFixedWindowAdmission(
  options: FixedWindowAdmissionOptions,
): FixedWindowAdmission {
  const { maxAttempts, windowMs, clock = Date.now } = options;
  if (!Number.isSafeInteger(maxAttempts) || maxAttempts <= 0) {
    throw new RangeError("maxAttempts must be a positive safe integer");
  }
  if (!Number.isSafeInteger(windowMs) || windowMs <= 0) {
    throw new RangeError("windowMs must be a positive safe integer");
  }

  let windowStartedAt: number | null = null;
  let attemptCount = 0;

  return Object.freeze({
    consume() {
      let now: number;
      try {
        now = clock();
      } catch {
        return false;
      }
      if (!Number.isFinite(now)) return false;

      if (windowStartedAt === null || now >= windowStartedAt + windowMs) {
        windowStartedAt = now;
        attemptCount = 0;
      }
      // A regressing clock never opens a new window. It stays fail-closed
      // against the current window until monotonic time catches up.
      if (attemptCount >= maxAttempts) return false;
      attemptCount += 1;
      return true;
    },
  });
}
