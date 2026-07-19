import { afterEach, describe, expect, it, vi } from "vitest";

import * as quarantineContract from "./imported-executable-quarantine";
import {
  IMPORTED_EXECUTABLE_DISPOSITION,
  IMPORTED_EXECUTABLE_METADATA_VERSION,
  IMPORTED_EXECUTABLE_POLICY,
  MAX_IMPORTED_EXECUTABLE_METADATA_BYTES,
  importedExecutablePolicy,
  parseQuarantinedJavaScriptMetadata,
} from "./imported-executable-quarantine";

const VALID_METADATA = {
  metadataVersion: IMPORTED_EXECUTABLE_METADATA_VERSION,
  language: "JAVASCRIPT",
  contentByteLength: 217,
  contentSha256:
    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
} as const;

const FIXED_POLICY = {
  disposition: IMPORTED_EXECUTABLE_DISPOSITION,
  executable: false,
  policy: IMPORTED_EXECUTABLE_POLICY,
} as const;

afterEach(() => {
  vi.restoreAllMocks();
});

describe("imported executable quarantine metadata", () => {
  it("keeps the runtime export surface metadata-only", () => {
    expect(Object.keys(quarantineContract).sort()).toEqual([
      "IMPORTED_EXECUTABLE_DISPOSITION",
      "IMPORTED_EXECUTABLE_METADATA_VERSION",
      "IMPORTED_EXECUTABLE_POLICY",
      "MAX_IMPORTED_EXECUTABLE_METADATA_BYTES",
      "importedExecutablePolicy",
      "parseQuarantinedJavaScriptMetadata",
    ]);
  });

  it("accepts metadata only and derives the fixed inert policy", () => {
    const record = parseQuarantinedJavaScriptMetadata(
      JSON.stringify(VALID_METADATA),
    );

    expect(record).toEqual({
      ...FIXED_POLICY,
      metadata: VALID_METADATA,
    });
    expect(Object.isFrozen(record)).toBe(true);
    expect(Object.isFrozen(record.metadata)).toBe(true);
    expect(Object.keys(record.metadata).sort()).toEqual([
      "contentByteLength",
      "contentSha256",
      "language",
      "metadataVersion",
    ]);
  });

  it.each([
    null,
    undefined,
    [],
    "metadata",
    {
      metadataVersion: VALID_METADATA.metadataVersion,
      language: VALID_METADATA.language,
      contentByteLength: VALID_METADATA.contentByteLength,
    },
    { ...VALID_METADATA, metadataVersion: 2 },
    { ...VALID_METADATA, language: "LUA" },
    { ...VALID_METADATA, contentByteLength: "217" },
    { ...VALID_METADATA, contentByteLength: -1 },
    { ...VALID_METADATA, contentByteLength: 1.5 },
    { ...VALID_METADATA, contentByteLength: Number.MAX_SAFE_INTEGER + 1 },
    { ...VALID_METADATA, contentSha256: "ABCDEF".repeat(10) + "ABCD" },
    { ...VALID_METADATA, contentSha256: "0".repeat(63) },
  ])("rejects invalid metadata values", (metadata) => {
    expect(() =>
      parseQuarantinedJavaScriptMetadata(JSON.stringify(metadata)),
    ).toThrow("invalid imported executable metadata");
  });

  it.each([
    ["source", "throw new Error('not inert')"],
    ["code", "while (true) {}"],
    ["script", "external.js"],
    ["url", "https://example.invalid/external.js"],
    ["enabled", true],
    ["policy", "ENABLED"],
    ["disposition", "RUNNABLE"],
    ["runtime", "quickjs"],
  ])("rejects the extra %s field instead of carrying it", (key, value) => {
    expect(() =>
      parseQuarantinedJavaScriptMetadata(
        JSON.stringify({ ...VALID_METADATA, [key]: value }),
      ),
    ).toThrow("invalid imported executable metadata");
  });

  it("accepts metadata at the UTF-8 byte limit", () => {
    const serialized = JSON.stringify(VALID_METADATA);
    const padding = " ".repeat(
      MAX_IMPORTED_EXECUTABLE_METADATA_BYTES -
        new TextEncoder().encode(serialized).byteLength,
    );

    expect(
      new TextEncoder().encode(`${padding}${serialized}`).byteLength,
    ).toBe(MAX_IMPORTED_EXECUTABLE_METADATA_BYTES);
    expect(
      parseQuarantinedJavaScriptMetadata(`${padding}${serialized}`),
    ).toEqual({
      ...FIXED_POLICY,
      metadata: VALID_METADATA,
    });
  });

  it("rejects oversized UTF-8 metadata before JSON parsing", () => {
    const serialized = JSON.stringify({
      ...VALID_METADATA,
      note: "한".repeat(1_350),
    });
    const parseSpy = vi.spyOn(JSON, "parse");

    expect(serialized.length).toBeLessThan(
      MAX_IMPORTED_EXECUTABLE_METADATA_BYTES,
    );
    expect(new TextEncoder().encode(serialized).byteLength).toBeGreaterThan(
      MAX_IMPORTED_EXECUTABLE_METADATA_BYTES,
    );
    expect(() => parseQuarantinedJavaScriptMetadata(serialized)).toThrow(
      "imported executable metadata exceeds byte limit",
    );
    expect(parseSpy).not.toHaveBeenCalled();
  });

  it("counts four-byte UTF-8 code points without decoding the envelope", () => {
    const serialized = `"${"😀".repeat(1_024)}"`;
    const parseSpy = vi.spyOn(JSON, "parse");

    expect(serialized.length).toBeLessThan(
      MAX_IMPORTED_EXECUTABLE_METADATA_BYTES,
    );
    expect(new TextEncoder().encode(serialized).byteLength).toBeGreaterThan(
      MAX_IMPORTED_EXECUTABLE_METADATA_BYTES,
    );
    expect(() => parseQuarantinedJavaScriptMetadata(serialized)).toThrow(
      "imported executable metadata exceeds byte limit",
    );
    expect(parseSpy).not.toHaveBeenCalled();
  });

  it("rejects malformed metadata without returning parser details", () => {
    expect(() => parseQuarantinedJavaScriptMetadata("{broken")).toThrow(
      "invalid imported executable metadata",
    );
  });
});

describe("imported executable policy influences", () => {
  it.each([
    {},
    { manifest: { executable: true, policy: "ENABLED" } },
    { importSettings: { runScripts: true } },
    { legacySettings: { javascriptEnabled: true } },
    {
      manifest: { executable: true },
      importSettings: { runScripts: true },
      legacySettings: { javascriptEnabled: true },
    },
  ])("cannot be enabled by untrusted imported state", (influences) => {
    expect(importedExecutablePolicy(influences)).toEqual(FIXED_POLICY);
  });

  it("does not inspect untrusted imported state", () => {
    const influences = Object.defineProperty({}, "manifest", {
      enumerable: true,
      get() {
        throw new Error("untrusted getter was read");
      },
    });

    expect(importedExecutablePolicy(influences)).toEqual(FIXED_POLICY);
  });

  it("returns one immutable fixed policy contract", () => {
    const policy = importedExecutablePolicy();

    expect(Object.isFrozen(policy)).toBe(true);
    expect(importedExecutablePolicy({ legacySettings: policy })).toBe(policy);
    expect(() =>
      Object.assign(policy, { executable: true, policy: "ENABLED" }),
    ).toThrow();
    expect(importedExecutablePolicy()).toEqual(FIXED_POLICY);
  });
});
