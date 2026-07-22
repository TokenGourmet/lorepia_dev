import { readFileSync } from "node:fs";

import { describe, expect, it } from "vitest";

const pagePath = new URL("../routes/+page.svelte", import.meta.url);

describe("stream finalization UI lifecycle", () => {
  it("turns every retained-run finalization failure into a retryable failed phase", () => {
    const source = readFileSync(pagePath, "utf8");

    expect(source).toMatch(
      /function failFinalization\([\s\S]*?run\.finalizationFailed = true;[\s\S]*?phase = "failed";/,
    );
    expect(source).toContain("if (!run.releaseConfirmed && !run.finalizationFailed)");
    expect(source).toContain("if (run.releaseConfirmed || run.finalizationFailed)");
    expect(source).not.toMatch(
      /finally \{\s*if \(activeRun === run\) \{\s*finalSnapshotPending = false;\s*activeRun = null;/,
    );
  });

  it("clears a successful run only after validating the exact release response", () => {
    const source = readFileSync(pagePath, "utf8");
    const releaseValidation = source.indexOf(
      "const releaseValidation = validateReleaseStreamResponse(",
    );
    const releaseAccepted = source.indexOf("if (!releaseValidation.accepted)");
    const releaseConfirmed = source.indexOf("run.releaseConfirmed = true;");

    expect(releaseValidation).toBeGreaterThan(-1);
    expect(releaseAccepted).toBeGreaterThan(releaseValidation);
    expect(releaseConfirmed).toBeGreaterThan(releaseAccepted);
  });

  it("records queued ACK failures and fails closed before terminal ACK or release", () => {
    const source = readFileSync(pagePath, "utf8");
    const awaitAckChain = source.indexOf("await run.ackChain;");
    const ackFailureGate = source.indexOf(
      "if (run.acknowledgementFailure !== null)",
      awaitAckChain,
    );
    const terminalAck = source.indexOf(
      "const terminalAck = await acknowledgeStream(",
      awaitAckChain,
    );
    const release = source.indexOf(
      "const rawRelease: unknown = await releaseStream(",
      awaitAckChain,
    );

    expect(source).toContain("run.acknowledgementFailure =");
    expect(ackFailureGate).toBeGreaterThan(awaitAckChain);
    expect(terminalAck).toBeGreaterThan(ackFailureGate);
    expect(release).toBeGreaterThan(terminalAck);
  });
});
