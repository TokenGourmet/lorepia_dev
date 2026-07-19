import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

interface FixtureCatalog {
  protocolVersion: string;
  policyVersion: string;
  origin: string;
  license: string;
  fixtures: Array<{
    fixtureId: string;
    file: string;
    sourceBytes: number;
    sourceSha256: string;
  }>;
}

const catalogPath = new URL("../../fixtures/catalog.json", import.meta.url);
const catalog = JSON.parse(
  readFileSync(catalogPath, "utf8"),
) as FixtureCatalog;

describe("script runner fixture provenance", () => {
  it("pins the exact self-authored catalog contract", () => {
    expect(catalog.protocolVersion).toBe("m1-script-runner-fixtures-v1");
    expect(catalog.policyVersion).toBe("m1-script-runner-v1");
    expect(catalog.origin).toBe(
      "self-authored for LorePia M-1 security validation",
    );
    expect(catalog.license).toBe("Apache-2.0");
    expect(catalog.fixtures.map((fixture) => fixture.fixtureId)).toEqual([
      "allowed",
      "infinite-loop",
      "recursive-pressure",
      "allocator-pressure",
      "forbidden-globals",
      "oversized-output",
      "script-error",
    ]);
  });

  it("matches every source byte-for-byte", () => {
    for (const fixture of catalog.fixtures) {
      const bytes = readFileSync(
        new URL(`../../fixtures/${fixture.file}`, import.meta.url),
      );
      expect(bytes.byteLength, fixture.file).toBe(fixture.sourceBytes);
      expect(
        createHash("sha256").update(bytes).digest("hex"),
        fixture.file,
      ).toBe(fixture.sourceSha256);
    }
  });

  it("documents provenance, license, and the non-product scope", () => {
    const readme = readFileSync(
      new URL("../../fixtures/README.md", import.meta.url),
      "utf8",
    );
    expect(readme).toContain("self-authored");
    expect(readme).toContain("Apache-2.0");
    expect(readme).toContain("do not define the future public card API");
    expect(readme).toContain("Tauri IPC");
  });
});
