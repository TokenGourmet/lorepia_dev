import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";

import { describe, expect, it } from "vitest";

const htmlPath = new URL("../../static/plugin-frame.html", import.meta.url);
const scriptPath = new URL("../../static/plugin-frame.js", import.meta.url);

describe("packaged plugin frame bootstrap", () => {
  it("keeps the audited inline fixture and CSP hash synchronized", () => {
    const html = readFileSync(htmlPath, "utf8");
    const source = readFileSync(scriptPath, "utf8");
    const start = html.indexOf("<script>") + "<script>".length;
    const end = html.indexOf("</script>", start);

    expect(start).toBeGreaterThan("<script>".length - 1);
    expect(end).toBeGreaterThan(start);
    expect(html.indexOf("</script>", end + 1)).toBe(-1);

    const inline = html.slice(start, end);
    expect(inline).toBe(source);
    expect(inline).not.toContain("</script");

    const digest = createHash("sha256").update(inline).digest("base64");
    expect(html).toContain(`script-src 'sha256-${digest}'`);
  });

  it("echoes one bounded host run ID in every exact test-result envelope", () => {
    const source = readFileSync(scriptPath, "utf8");

    expect(source).toContain(
      'const RUN_SUITE_KEYS = ["type", "sessionNonce", "runId"]',
    );
    expect(source).toContain("function postTestResult(runId, testId, passed, detail)");
    expect(source).toMatch(
      /type: "lorepia:plugin:test-result",\s+sessionNonce,\s+runId,\s+testId,/,
    );
    expect(source).toContain("async function runSuite(runId)");
    expect(source).toContain("void runSuite(value.runId)");
  });

  it("marks every direct native timeout inconclusive instead of passing", () => {
    const source = readFileSync(scriptPath, "utf8");
    const timeoutDetails = source.match(/INCONCLUSIVE:[^"`]+timed out/g) ?? [];

    expect(timeoutDetails).toHaveLength(4);
    expect(source).not.toMatch(/passed:\s*[^,\n]*timedOut/);
  });
});
