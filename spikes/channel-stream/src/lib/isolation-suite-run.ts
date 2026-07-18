export const SUITE_RUN_ID_PATTERN = /^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$/;

export type SuiteResultAdmission = "accepted" | "complete" | "ignored";

export type IsolationSuiteRunGate = Readonly<{
  start: (runId: string) => void;
  accept: (runId: string, testId: string) => SuiteResultAdmission;
  isActive: (runId: string) => boolean;
  finish: (runId: string) => boolean;
  invalidate: () => void;
  readonly activeRunId: string | null;
  readonly auditStarted: boolean;
  readonly receivedCount: number;
}>;

export function isValidSuiteRunId(value: unknown): value is string {
  return typeof value === "string" && SUITE_RUN_ID_PATTERN.test(value);
}

export function createIsolationSuiteRunGate(
  expectedTestIds: readonly string[],
): IsolationSuiteRunGate {
  const expected = new Set(expectedTestIds);
  if (expected.size === 0 || expected.size !== expectedTestIds.length) {
    throw new Error("suite test IDs must be a non-empty unique list");
  }

  let activeRunId: string | null = null;
  let auditStarted = false;
  let received = new Set<string>();

  function clear(): void {
    activeRunId = null;
    auditStarted = false;
    received = new Set<string>();
  }

  return Object.freeze({
    get activeRunId() {
      return activeRunId;
    },
    get auditStarted() {
      return auditStarted;
    },
    get receivedCount() {
      return received.size;
    },

    start(runId) {
      if (!isValidSuiteRunId(runId)) {
        throw new Error("suite run ID is invalid");
      }
      if (activeRunId !== null) {
        throw new Error("an isolation suite run is already active");
      }
      activeRunId = runId;
      auditStarted = false;
      received = new Set<string>();
    },

    accept(runId, testId) {
      if (
        activeRunId !== runId ||
        auditStarted ||
        !expected.has(testId) ||
        received.has(testId)
      ) {
        return "ignored";
      }

      received.add(testId);
      if (received.size === expected.size) {
        // This transition is synchronous, so duplicate final messages cannot
        // schedule a second native side-effect audit.
        auditStarted = true;
        return "complete";
      }
      return "accepted";
    },

    isActive(runId) {
      return activeRunId === runId;
    },

    finish(runId) {
      if (activeRunId !== runId) return false;
      clear();
      return true;
    },

    invalidate() {
      clear();
    },
  });
}
