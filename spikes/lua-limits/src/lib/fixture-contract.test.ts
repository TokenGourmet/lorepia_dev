import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";

import { describe, expect, it } from "vitest";

import {
  LUA_FIXTURE_CATALOG_SHA256,
  LUA_POLICY_VERSION,
} from "./lua-probe";

type CatalogFixture = {
  fixtureId: string;
  file: string;
  sourceBytes: number;
  sourceSha256: string;
};

type FixtureCatalog = {
  protocolVersion: string;
  policyVersion: string;
  origin: string;
  license: string;
  fixtures: CatalogFixture[];
};

const expectedFixtures = [
  "allowed",
  "infinite-loop",
  "recursive-pressure",
  "allocator-pressure",
  "forbidden-globals",
  "bypass-surfaces",
] as const;
const catalogUrl = new URL("../../fixtures/catalog.json", import.meta.url);
const catalogBytes = readFileSync(catalogUrl);
const catalog = JSON.parse(catalogBytes.toString("utf8")) as FixtureCatalog;

function exactKeys(value: object, keys: string[]): void {
  expect(Object.keys(value).sort()).toEqual([...keys].sort());
}

describe("pinned self-authored Lua fixture catalog", () => {
  it("pins the exact catalog bytes", () => {
    expect(createHash("sha256").update(catalogBytes).digest("hex")).toBe(
      LUA_FIXTURE_CATALOG_SHA256,
    );
  });

  it("has the exact bounded root contract and provenance", () => {
    exactKeys(catalog, [
      "protocolVersion",
      "policyVersion",
      "origin",
      "license",
      "fixtures",
    ]);
    expect(catalog.protocolVersion).toBe("m1-lua-limits-fixtures-v1");
    expect(catalog.policyVersion).toBe(LUA_POLICY_VERSION);
    expect(catalog.origin).toBe("self-authored");
    expect(catalog.license).toBe("CC0-1.0");
  });

  it("pins every source byte, size, ID, and order", () => {
    expect(catalog.fixtures.map((fixture) => fixture.fixtureId)).toEqual(
      expectedFixtures,
    );
    expect(new Set(catalog.fixtures.map((fixture) => fixture.fixtureId)).size).toBe(
      expectedFixtures.length,
    );

    for (const fixture of catalog.fixtures) {
      exactKeys(fixture, ["fixtureId", "file", "sourceBytes", "sourceSha256"]);
      expect(fixture.fixtureId).toMatch(/^[a-z0-9][a-z0-9-]{0,63}$/);
      expect(fixture.file).toBe(`${fixture.fixtureId}.lua`);
      expect(fixture.file).not.toMatch(/[\\/]/);
      expect(fixture.sourceSha256).toMatch(/^[0-9a-f]{64}$/);
      expect(fixture.sourceBytes).toBeGreaterThan(0);
      expect(fixture.sourceBytes).toBeLessThanOrEqual(512);

      const source = readFileSync(new URL(`../../fixtures/${fixture.file}`, import.meta.url));
      expect(source.byteLength).toBe(fixture.sourceBytes);
      expect(createHash("sha256").update(source).digest("hex")).toBe(
        fixture.sourceSha256,
      );
    }
  });

  it("has a self-authored CC0 provenance record", () => {
    const provenance = readFileSync(
      new URL("../../fixtures/README.md", import.meta.url),
      "utf8",
    );
    expect(provenance).toMatch(/self-authored/i);
    expect(provenance).toContain("CC0-1.0");
    expect(provenance).toContain("catalog.json");
    expect(provenance).toContain("SHA-256");
  });
});
