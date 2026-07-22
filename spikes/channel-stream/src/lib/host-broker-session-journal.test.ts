import { describe, expect, it, vi } from "vitest";

import {
  HOST_BROKER_ROTATION_JOURNAL_KEY,
  HOST_BROKER_TOKEN_STORAGE_KEY,
  beginHostBrokerRotation,
  finalizeHostBrokerRotationAfterAudit,
  markHostBrokerRotationPendingAudit,
  readHostBrokerRotationJournal,
  recoverHostBrokerRotation,
} from "./host-broker-session-journal";

const currentToken = "a".repeat(64);
const nextToken = "b".repeat(64);

function storage(): Storage {
  const values = new Map<string, string>();
  return {
    get length() {
      return values.size;
    },
    clear() {
      values.clear();
    },
    getItem(key) {
      return values.get(key) ?? null;
    },
    key(index) {
      return [...values.keys()][index] ?? null;
    },
    removeItem(key) {
      values.delete(key);
    },
    setItem(key, value) {
      values.set(key, value);
    },
  };
}

function begin(target: Storage): void {
  target.setItem(HOST_BROKER_TOKEN_STORAGE_KEY, currentToken);
  beginHostBrokerRotation(target, {
    currentToken,
    nextToken,
    expectedGeneration: 3,
  });
}

describe("host broker rotation journal", () => {
  it("moves from rotating to pending audit without publishing the next token", () => {
    const target = storage();
    begin(target);

    expect(readHostBrokerRotationJournal(target)).toEqual({
      phase: "rotating",
      currentToken,
      nextToken,
      expectedGeneration: 3,
    });
    expect(markHostBrokerRotationPendingAudit(target, nextToken, 4)).toEqual({
      phase: "pending_audit",
      currentToken,
      nextToken,
      expectedGeneration: 3,
      nextGeneration: 4,
    });
    expect(target.getItem(HOST_BROKER_TOKEN_STORAGE_KEY)).toBe(currentToken);
    expect(target.getItem(HOST_BROKER_ROTATION_JOURNAL_KEY)).not.toBeNull();
  });

  it("publishes and removes the journal only after the stale audit passes", async () => {
    const target = storage();
    begin(target);
    const audit = vi.fn(async (token: string) => token === currentToken);

    await finalizeHostBrokerRotationAfterAudit(target, nextToken, 4, audit);

    expect(audit).toHaveBeenCalledWith(currentToken);
    expect(target.getItem(HOST_BROKER_TOKEN_STORAGE_KEY)).toBe(nextToken);
    expect(target.getItem(HOST_BROKER_ROTATION_JOURNAL_KEY)).toBeNull();
  });

  it.each([
    ["false", async () => false],
    ["throw", async () => Promise.reject(new Error("audit unavailable"))],
  ])("retains pending audit state when the audit returns %s", async (_label, audit) => {
    const target = storage();
    begin(target);

    await expect(
      finalizeHostBrokerRotationAfterAudit(target, nextToken, 4, audit),
    ).rejects.toThrow();
    expect(target.getItem(HOST_BROKER_TOKEN_STORAGE_KEY)).toBe(currentToken);
    expect(readHostBrokerRotationJournal(target)).toMatchObject({
      phase: "pending_audit",
      nextToken,
      nextGeneration: 4,
    });
  });

  it("fails closed on malformed, unknown-field, or token-reuse journals", () => {
    for (const encoded of [
      "not-json",
      JSON.stringify({
        phase: "rotating",
        currentToken,
        nextToken,
        expectedGeneration: 3,
        extra: true,
      }),
      JSON.stringify({
        phase: "rotating",
        currentToken,
        nextToken: currentToken,
        expectedGeneration: 3,
      }),
      JSON.stringify({
        phase: "unknown",
        currentToken,
        nextToken,
        expectedGeneration: 3,
      }),
    ]) {
      const target = storage();
      target.setItem(HOST_BROKER_ROTATION_JOURNAL_KEY, encoded);
      expect(() => readHostBrokerRotationJournal(target)).toThrow();
      expect(target.getItem(HOST_BROKER_ROTATION_JOURNAL_KEY)).toBe(encoded);
    }
  });

  it("recovers a native next-token commit but re-audits before publishing", async () => {
    const target = storage();
    begin(target);
    const register = vi.fn(async (token: string) => {
      if (token !== nextToken) throw new Error("wrong token");
      return 4;
    });
    const audit = vi.fn(async (token: string) => token === currentToken);

    await expect(
      recoverHostBrokerRotation(target, register, audit),
    ).resolves.toEqual({
      outcome: "recovered_next",
      token: nextToken,
      generation: 4,
    });
    expect(register).toHaveBeenCalledTimes(1);
    expect(audit).toHaveBeenCalledWith(currentToken);
    expect(target.getItem(HOST_BROKER_TOKEN_STORAGE_KEY)).toBe(nextToken);
    expect(target.getItem(HOST_BROKER_ROTATION_JOURNAL_KEY)).toBeNull();
  });

  it("keeps a recovered next token quarantined when its audit fails", async () => {
    const target = storage();
    begin(target);
    const register = vi.fn(async () => 4);
    const audit = vi.fn(async () => false);

    await expect(
      recoverHostBrokerRotation(target, register, audit),
    ).rejects.toThrow("audit did not pass");
    expect(register).toHaveBeenCalledTimes(1);
    expect(target.getItem(HOST_BROKER_TOKEN_STORAGE_KEY)).toBe(currentToken);
    expect(readHostBrokerRotationJournal(target)).toMatchObject({
      phase: "pending_audit",
      nextToken,
    });

    audit.mockResolvedValueOnce(true);
    await expect(
      recoverHostBrokerRotation(target, register, audit),
    ).resolves.toMatchObject({ outcome: "recovered_next" });
    expect(target.getItem(HOST_BROKER_TOKEN_STORAGE_KEY)).toBe(nextToken);
    expect(target.getItem(HOST_BROKER_ROTATION_JOURNAL_KEY)).toBeNull();
  });

  it("never falls back to the old token from pending-audit state", async () => {
    const target = storage();
    begin(target);
    markHostBrokerRotationPendingAudit(target, nextToken, 4);
    const register = vi.fn(async () => Promise.reject(new Error("unavailable")));
    const audit = vi.fn(async () => true);

    await expect(
      recoverHostBrokerRotation(target, register, audit),
    ).rejects.toThrow("unavailable");
    expect(register).toHaveBeenCalledTimes(1);
    expect(register).toHaveBeenCalledWith(nextToken);
    expect(audit).not.toHaveBeenCalled();
    expect(readHostBrokerRotationJournal(target)).toMatchObject({
      phase: "pending_audit",
    });
  });

  it("rolls back to the current token when native rotation did not commit", async () => {
    const target = storage();
    begin(target);
    const register = vi.fn(async (token: string) => {
      if (token === nextToken) throw new Error("next token is not current");
      if (token === currentToken) return 3;
      throw new Error("unexpected token");
    });
    const audit = vi.fn(async () => true);

    await expect(
      recoverHostBrokerRotation(target, register, audit),
    ).resolves.toEqual({
      outcome: "rolled_back",
      token: currentToken,
      generation: 3,
    });
    expect(register.mock.calls.map(([token]) => token)).toEqual([
      nextToken,
      currentToken,
    ]);
    expect(audit).not.toHaveBeenCalled();
    expect(target.getItem(HOST_BROKER_TOKEN_STORAGE_KEY)).toBe(currentToken);
    expect(target.getItem(HOST_BROKER_ROTATION_JOURNAL_KEY)).toBeNull();
  });

  it("retains the journal when neither token can reconcile native state", async () => {
    const target = storage();
    begin(target);
    const register = vi.fn(async () => Promise.reject(new Error("unavailable")));

    await expect(
      recoverHostBrokerRotation(target, register, async () => true),
    ).rejects.toThrow("could not be reconciled");
    expect(target.getItem(HOST_BROKER_ROTATION_JOURNAL_KEY)).not.toBeNull();
    expect(target.getItem(HOST_BROKER_TOKEN_STORAGE_KEY)).toBe(currentToken);
  });
});
