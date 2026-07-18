import { readdirSync, readFileSync, statSync } from "node:fs";

import { describe, expect, it } from "vitest";

const sourceRoot = new URL("../", import.meta.url);

function productionSources(directory: URL): URL[] {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const child = new URL(entry.isDirectory() ? `${entry.name}/` : entry.name, directory);
    if (entry.isDirectory()) return productionSources(child);
    if (!statSync(child).isFile()) return [];
    if (entry.name.endsWith(".test.ts") || entry.name.endsWith(".test.js")) return [];
    return /\.(?:ts|js|svelte)$/.test(entry.name) ? [child] : [];
  });
}

describe("frontend native command surface", () => {
  it("contains one literal invoke and only the M-1 lifecycle probe", () => {
    const invocations = productionSources(sourceRoot).flatMap((sourcePath) => {
      const source = readFileSync(sourcePath, "utf8");
      return [...source.matchAll(/\binvoke(?:<[^>]+>)?\(\s*["']([^"']+)["']/g)].map(
        (match) => match[1],
      );
    });

    expect(invocations).toEqual(["run_keychain_m1_probe"]);
  });

  it("imports the Tauri invoke API in only the bounded protocol module", () => {
    const importers = productionSources(sourceRoot)
      .filter((sourcePath) =>
        readFileSync(sourcePath, "utf8").includes("@tauri-apps/api/core"),
      )
      .map((sourcePath) => sourcePath.pathname.split("/").at(-1));

    expect(importers).toEqual(["keychain-probe.ts"]);
  });

  it("never serializes or reflects raw caught failures", () => {
    const production = productionSources(sourceRoot)
      .map((sourcePath) => readFileSync(sourcePath, "utf8"))
      .join("\n");

    expect(production).not.toContain("JSON.stringify");
    expect(production).not.toMatch(/String\(\s*(?:error|rawFailure)\s*\)/);
    expect(production).not.toMatch(/(?:error|rawFailure)\.message/);
  });
});
