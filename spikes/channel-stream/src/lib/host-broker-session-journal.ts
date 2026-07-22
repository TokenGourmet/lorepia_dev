export const HOST_BROKER_TOKEN_STORAGE_KEY =
  "lorepia.m1.host-broker-token.v1";
export const HOST_BROKER_ROTATION_JOURNAL_KEY =
  "lorepia.m1.host-broker-rotation.v1";

export type RotatingHostBrokerJournal = Readonly<{
  phase: "rotating";
  currentToken: string;
  nextToken: string;
  expectedGeneration: number;
}>;

export type PendingAuditHostBrokerJournal = Readonly<{
  phase: "pending_audit";
  currentToken: string;
  nextToken: string;
  expectedGeneration: number;
  nextGeneration: number;
}>;

export type HostBrokerRotationJournal =
  | RotatingHostBrokerJournal
  | PendingAuditHostBrokerJournal;

export type RecoveredHostBrokerSession =
  | Readonly<{
      outcome: "recovered_next";
      token: string;
      generation: number;
    }>
  | Readonly<{
      outcome: "rolled_back";
      token: string;
      generation: number;
    }>;

export type HostBrokerTokenRegistration = (token: string) => Promise<number>;
export type HostBrokerStaleTokenAudit = (staleToken: string) => Promise<boolean>;

type SessionStorageBoundary = Pick<
  Storage,
  "getItem" | "setItem" | "removeItem"
>;

const TOKEN_PATTERN = /^[0-9a-f]{64}$/;

function isPlainRecord(value: unknown): value is Record<string, unknown> {
  return (
    typeof value === "object" &&
    value !== null &&
    !Array.isArray(value) &&
    Object.getPrototypeOf(value) === Object.prototype
  );
}

function isGeneration(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 1;
}

function requireToken(token: string, label: string): void {
  if (!TOKEN_PATTERN.test(token)) {
    throw new Error(`${label} must be a 256-bit lowercase hexadecimal token`);
  }
}

function requireGenerationTransition(
  expectedGeneration: number,
  nextGeneration: number,
): void {
  if (
    !isGeneration(expectedGeneration) ||
    !isGeneration(nextGeneration) ||
    nextGeneration !== expectedGeneration + 1
  ) {
    throw new Error("host broker rotation generation transition is invalid");
  }
}

function writeVerified(
  storage: SessionStorageBoundary,
  key: string,
  value: string,
): void {
  storage.setItem(key, value);
  if (storage.getItem(key) !== value) {
    throw new Error("host broker sessionStorage write verification failed");
  }
}

function removeVerified(storage: SessionStorageBoundary, key: string): void {
  storage.removeItem(key);
  if (storage.getItem(key) !== null) {
    throw new Error("host broker sessionStorage cleanup failed");
  }
}

function persistJournal(
  storage: SessionStorageBoundary,
  journal: HostBrokerRotationJournal,
): void {
  writeVerified(
    storage,
    HOST_BROKER_ROTATION_JOURNAL_KEY,
    JSON.stringify(journal),
  );
}

export function readHostBrokerToken(
  storage: SessionStorageBoundary,
): string | null {
  const token = storage.getItem(HOST_BROKER_TOKEN_STORAGE_KEY);
  if (token === null) return null;
  requireToken(token, "stored host broker token");
  return token;
}

export function persistHostBrokerToken(
  storage: SessionStorageBoundary,
  token: string,
): void {
  requireToken(token, "host broker token");
  writeVerified(storage, HOST_BROKER_TOKEN_STORAGE_KEY, token);
}

export function readHostBrokerRotationJournal(
  storage: SessionStorageBoundary,
): HostBrokerRotationJournal | null {
  const encoded = storage.getItem(HOST_BROKER_ROTATION_JOURNAL_KEY);
  if (encoded === null) return null;

  let value: unknown;
  try {
    value = JSON.parse(encoded);
  } catch {
    throw new Error("host broker rotation journal is malformed");
  }
  if (!isPlainRecord(value) || typeof value.phase !== "string") {
    throw new Error("host broker rotation journal has an invalid schema");
  }

  const expectedKeys =
    value.phase === "rotating"
      ? ["phase", "currentToken", "nextToken", "expectedGeneration"]
      : value.phase === "pending_audit"
        ? [
            "phase",
            "currentToken",
            "nextToken",
            "expectedGeneration",
            "nextGeneration",
          ]
        : null;
  if (
    expectedKeys === null ||
    Object.keys(value).length !== expectedKeys.length ||
    !expectedKeys.every((key) => Object.prototype.hasOwnProperty.call(value, key)) ||
    typeof value.currentToken !== "string" ||
    typeof value.nextToken !== "string" ||
    !isGeneration(value.expectedGeneration)
  ) {
    throw new Error("host broker rotation journal has an invalid schema");
  }

  requireToken(value.currentToken, "journal current token");
  requireToken(value.nextToken, "journal next token");
  if (value.currentToken === value.nextToken) {
    throw new Error("host broker rotation journal reuses the current token");
  }

  if (value.phase === "rotating") {
    return {
      phase: "rotating",
      currentToken: value.currentToken,
      nextToken: value.nextToken,
      expectedGeneration: value.expectedGeneration,
    };
  }

  if (!isGeneration(value.nextGeneration)) {
    throw new Error("host broker rotation journal has an invalid schema");
  }
  requireGenerationTransition(value.expectedGeneration, value.nextGeneration);
  return {
    phase: "pending_audit",
    currentToken: value.currentToken,
    nextToken: value.nextToken,
    expectedGeneration: value.expectedGeneration,
    nextGeneration: value.nextGeneration,
  };
}

export function beginHostBrokerRotation(
  storage: SessionStorageBoundary,
  journal: Omit<RotatingHostBrokerJournal, "phase">,
): void {
  requireToken(journal.currentToken, "rotation current token");
  requireToken(journal.nextToken, "rotation next token");
  if (journal.currentToken === journal.nextToken) {
    throw new Error("rotation next token must differ from the current token");
  }
  if (!isGeneration(journal.expectedGeneration)) {
    throw new Error("rotation expected generation is invalid");
  }
  if (readHostBrokerRotationJournal(storage) !== null) {
    throw new Error("a host broker rotation journal is already pending");
  }
  persistJournal(storage, { phase: "rotating", ...journal });
}

export function markHostBrokerRotationPendingAudit(
  storage: SessionStorageBoundary,
  nextToken: string,
  nextGeneration: number,
): PendingAuditHostBrokerJournal {
  const journal = readHostBrokerRotationJournal(storage);
  if (journal === null || journal.nextToken !== nextToken) {
    throw new Error("host broker rotation journal does not match the native commit");
  }
  requireGenerationTransition(journal.expectedGeneration, nextGeneration);
  if (
    journal.phase === "pending_audit" &&
    journal.nextGeneration !== nextGeneration
  ) {
    throw new Error("host broker pending audit generation changed");
  }

  const pending: PendingAuditHostBrokerJournal = {
    phase: "pending_audit",
    currentToken: journal.currentToken,
    nextToken: journal.nextToken,
    expectedGeneration: journal.expectedGeneration,
    nextGeneration,
  };
  persistJournal(storage, pending);
  return pending;
}

function finalizeHostBrokerRotation(
  storage: SessionStorageBoundary,
  nextToken: string,
  nextGeneration: number,
): void {
  const journal = readHostBrokerRotationJournal(storage);
  if (
    journal?.phase !== "pending_audit" ||
    journal.nextToken !== nextToken ||
    journal.nextGeneration !== nextGeneration
  ) {
    throw new Error("host broker rotation is not ready to finalize");
  }
  persistHostBrokerToken(storage, nextToken);
  removeVerified(storage, HOST_BROKER_ROTATION_JOURNAL_KEY);
}

export async function finalizeHostBrokerRotationAfterAudit(
  storage: SessionStorageBoundary,
  nextToken: string,
  nextGeneration: number,
  auditStaleToken: HostBrokerStaleTokenAudit,
): Promise<void> {
  const pending = markHostBrokerRotationPendingAudit(
    storage,
    nextToken,
    nextGeneration,
  );
  if (!(await auditStaleToken(pending.currentToken))) {
    throw new Error("stale host broker token audit did not pass");
  }
  finalizeHostBrokerRotation(storage, nextToken, nextGeneration);
}

function rollbackHostBrokerRotation(
  storage: SessionStorageBoundary,
  currentToken: string,
): void {
  persistHostBrokerToken(storage, currentToken);
  removeVerified(storage, HOST_BROKER_ROTATION_JOURNAL_KEY);
}

export async function recoverHostBrokerRotation(
  storage: SessionStorageBoundary,
  registerToken: HostBrokerTokenRegistration,
  auditStaleToken: HostBrokerStaleTokenAudit,
): Promise<RecoveredHostBrokerSession | null> {
  const journal = readHostBrokerRotationJournal(storage);
  if (journal === null) return null;

  if (journal.phase === "pending_audit") {
    const generation = await registerToken(journal.nextToken);
    if (generation !== journal.nextGeneration) {
      throw new Error("pending host broker audit generation does not match native state");
    }
    await finalizeHostBrokerRotationAfterAudit(
      storage,
      journal.nextToken,
      generation,
      auditStaleToken,
    );
    return {
      outcome: "recovered_next",
      token: journal.nextToken,
      generation,
    };
  }

  let nextGeneration: number;
  try {
    nextGeneration = await registerToken(journal.nextToken);
  } catch (nextError) {
    try {
      const generation = await registerToken(journal.currentToken);
      if (generation !== journal.expectedGeneration) {
        throw new Error("rolled back host broker generation does not match native state");
      }
      rollbackHostBrokerRotation(storage, journal.currentToken);
      return {
        outcome: "rolled_back",
        token: journal.currentToken,
        generation,
      };
    } catch (currentError) {
      throw new AggregateError(
        [nextError, currentError],
        "host broker rotation could not be reconciled",
      );
    }
  }

  await finalizeHostBrokerRotationAfterAudit(
    storage,
    journal.nextToken,
    nextGeneration,
    auditStaleToken,
  );
  return {
    outcome: "recovered_next",
    token: journal.nextToken,
    generation: nextGeneration,
  };
}
