import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";

import { describe, expect, it } from "vitest";

import {
  IMPORT_FIXTURE_CATALOG_SHA256,
  IMPORT_GOLDEN_CASES,
  IMPORT_POLICY_VERSION,
  IMPORT_PROBE_ERROR_CODES,
} from "./import-probe";

type CatalogCase = {
  caseId: string;
  fixtureKind: "zip" | "png" | "raw";
  expectedOutcome: "ACCEPTED" | "REJECTED";
  expectedCode: string | null;
};

type FixtureCatalog = {
  version: number;
  policyVersion: string;
  license: string;
  provenance: string;
  cases: CatalogCase[];
};

const fixtureKinds = [
  "zip",
  "png",
  "raw",
  "raw",
  "zip",
  "zip",
  "zip",
  "zip",
  "zip",
  "zip",
  "zip",
  "zip",
  "zip",
  "zip",
  "zip",
  "zip",
  "zip",
  "zip",
  "zip",
  "zip",
  "zip",
  "zip",
  "zip",
  "zip",
  "zip",
  "zip",
] as const;

const catalogUrl = new URL("../../fixtures/import-cases-v1.json", import.meta.url);
const catalogBytes = readFileSync(catalogUrl);
const catalog = JSON.parse(catalogBytes.toString("utf8")) as FixtureCatalog;

function exactKeys(value: object, keys: string[]): void {
  expect(Object.keys(value).sort()).toEqual([...keys].sort());
}

describe("pinned self-authored import fixture catalog", () => {
  it("pins the exact catalog bytes", () => {
    expect(createHash("sha256").update(catalogBytes).digest("hex")).toBe(
      IMPORT_FIXTURE_CATALOG_SHA256,
    );
  });

  it("has the exact bounded root contract and provenance", () => {
    exactKeys(catalog, ["version", "policyVersion", "license", "provenance", "cases"]);
    expect(catalog.version).toBe(1);
    expect(catalog.policyVersion).toBe(IMPORT_POLICY_VERSION);
    expect(catalog.license).toBe("CC0-1.0");
    expect(catalog.provenance).toMatch(/^Self-authored /);
    expect(catalog.provenance.length).toBeLessThanOrEqual(256);
  });

  it("pins exact case order, kinds, outcomes, and codes", () => {
    expect(catalog.cases).toHaveLength(IMPORT_GOLDEN_CASES.length);
    expect(new Set(catalog.cases.map((entry) => entry.caseId)).size).toBe(
      catalog.cases.length,
    );
    catalog.cases.forEach((entry, index) => {
      exactKeys(entry, ["caseId", "fixtureKind", "expectedOutcome", "expectedCode"]);
      expect(entry.caseId).toMatch(/^[a-z0-9][a-z0-9-]{0,63}$/);
      expect(entry.fixtureKind).toBe(fixtureKinds[index]);
      if (entry.expectedCode !== null) {
        expect(IMPORT_PROBE_ERROR_CODES).toContain(entry.expectedCode);
      }
    });
    expect(
      catalog.cases.map((entry) => ({
        caseId: entry.caseId,
        outcome: entry.expectedOutcome,
        code: entry.expectedCode,
      })),
    ).toEqual(IMPORT_GOLDEN_CASES);
  });

  it("has a per-catalog self-authored and licensed provenance record", () => {
    const provenance = readFileSync(
      new URL("../../fixtures/README.md", import.meta.url),
      "utf8",
    );
    expect(provenance).toContain("import-cases-v1.json");
    expect(provenance).toMatch(/self-authored/i);
    expect(provenance).toContain("CC0-1.0");
    expect(provenance).toContain("SHA-256");
  });
});
