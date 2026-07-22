import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";

import { describe, expect, it } from "vitest";

import { SQLITE_FIXTURE_SHA256, SQLITE_GOLDEN_RESULTS } from "./sqlite-probe";

type FixtureRecord = {
  id: number;
  title: string;
  rawText: string;
};

type FixtureQuery = {
  queryId: string;
  mode: "fts" | "like";
  term: string;
  expectedIds: number[];
};

type Fixture = {
  version: number;
  license: string;
  records: FixtureRecord[];
  queries: FixtureQuery[];
};

const fixtureUrl = new URL("../../fixtures/korean-fts-v1.json", import.meta.url);
const fixtureText = readFileSync(fixtureUrl, "utf8");
const fixture = JSON.parse(fixtureText) as Fixture;

function exactKeys(value: object, keys: string[]): void {
  expect(Object.keys(value).sort()).toEqual([...keys].sort());
}

describe("self-authored Korean FTS fixture", () => {
  it("has a fixed version, reusable license, and unique records", () => {
    exactKeys(fixture, ["version", "license", "records", "queries"]);
    expect(fixture.version).toBe(1);
    expect(fixture.license).toBe("CC0-1.0");
    expect(fixture.records).toHaveLength(5);
    expect(fixture.records.map((record) => record.id)).toEqual(
      fixture.records.map((record) => record.id).sort((left, right) => left - right),
    );
    expect(new Set(fixture.records.map((record) => record.id)).size).toBe(
      fixture.records.length,
    );
    for (const record of fixture.records) {
      exactKeys(record, ["id", "title", "rawText"]);
      expect(Number.isSafeInteger(record.id)).toBe(true);
      expect(record.id).toBeGreaterThan(0);
      expect(typeof record.title).toBe("string");
      expect(typeof record.rawText).toBe("string");
      expect(record.title.length).toBeGreaterThan(0);
      expect(record.rawText.length).toBeGreaterThan(0);
    }
  });

  it("covers trigram, 1-2 character fallback, escaped wildcards, and injection literals", () => {
    const ftsQueries = fixture.queries.filter((query) => query.mode === "fts");
    const shortQueries = fixture.queries.filter(
      (query) => query.mode === "like" && [...query.term].length <= 2,
    );
    expect(ftsQueries.length).toBeGreaterThanOrEqual(3);
    expect(ftsQueries.every((query) => [...query.term].length >= 3)).toBe(true);
    expect(shortQueries.length).toBeGreaterThanOrEqual(2);
    expect(
      fixture.queries.every(
        (query) =>
          query.mode === ([...query.term].length <= 2 ? "like" : "fts"),
      ),
    ).toBe(true);
    expect(fixture.queries).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          queryId: "q-like-escaped-wildcards",
          mode: "like",
          term: "%_",
          expectedIds: [5],
        }),
        expect.objectContaining({
          queryId: "q-fts-injection-literal",
          mode: "fts",
          term: "%' OR 1=1 --",
          expectedIds: [],
        }),
      ]),
    );
  });

  it("uses unique query IDs and sorted references to known records", () => {
    const recordIds = new Set(fixture.records.map((record) => record.id));
    expect(new Set(fixture.queries.map((query) => query.queryId)).size).toBe(
      fixture.queries.length,
    );
    for (const query of fixture.queries) {
      exactKeys(query, ["queryId", "mode", "term", "expectedIds"]);
      expect(query.queryId).toMatch(/^[a-z0-9][a-z0-9-]{0,63}$/);
      expect(["fts", "like"]).toContain(query.mode);
      expect(typeof query.term).toBe("string");
      expect([...query.term].length).toBeGreaterThan(0);
      expect([...query.term].length).toBeLessThanOrEqual(64);
      expect(query.expectedIds).toEqual(
        [...query.expectedIds].sort((left, right) => left - right),
      );
      expect(new Set(query.expectedIds).size).toBe(query.expectedIds.length);
      for (const id of query.expectedIds) {
        expect(Number.isSafeInteger(id)).toBe(true);
        expect(recordIds.has(id)).toBe(true);
      }
    }
  });

  it("pins the exact ordered golden receipt and fixture bytes", () => {
    expect(
      fixture.queries.map((query) => ({
        queryId: query.queryId,
        resultIds: query.expectedIds,
      })),
    ).toEqual(SQLITE_GOLDEN_RESULTS);
    expect(createHash("sha256").update(fixtureText).digest("hex")).toBe(
      SQLITE_FIXTURE_SHA256,
    );
  });

  it("has a per-file self-authored provenance record", () => {
    const provenance = readFileSync(
      new URL("../../fixtures/README.md", import.meta.url),
      "utf8",
    );
    expect(provenance).toContain("korean-fts-v1.json");
    expect(provenance).toMatch(/self-authored/i);
    expect(provenance).toContain("CC0-1.0");
    expect(provenance).toContain(SQLITE_FIXTURE_SHA256);
    expect(provenance).toContain("1698 bytes");
  });
});
