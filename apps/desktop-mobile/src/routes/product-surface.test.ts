import { existsSync, readFileSync, readdirSync } from "node:fs";
import { extname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const pageSource = readFileSync(
  fileURLToPath(new URL("./+page.svelte", import.meta.url)),
  "utf8",
);
const productRoot = fileURLToPath(new URL("../..", import.meta.url));

function listRuntimeSources(directory: string): string[] {
  if (!existsSync(directory)) {
    return [];
  }

  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = resolve(directory, entry.name);
    if (entry.isDirectory()) {
      return listRuntimeSources(path);
    }
    if (/\.(?:test|spec)\.[^.]+$/.test(entry.name)) {
      return [];
    }
    return [".css", ".html", ".js", ".svelte", ".ts"].includes(
      extname(entry.name),
    )
      ? [path]
      : [];
  });
}

const runtimeSource = [resolve(productRoot, "src"), resolve(productRoot, "static")]
  .flatMap(listRuntimeSources)
  .map((path) => readFileSync(path, "utf8"))
  .join("\n");

// This is a temporary M0 ownership guard. Update it when the owner-authored
// visual design slice begins; it is not a permanent ban on product CSS.
describe("owner-authored product surface boundary", () => {
  it("contains no visual styling or animation", () => {
    expect(pageSource).not.toContain("<style");
    expect(pageSource).not.toMatch(/transition:|animate:|use:/);
  });

  it("does not enable unreviewed executable or media surfaces", () => {
    expect(pageSource).not.toMatch(/<iframe|<audio|{@html/i);
  });

  it("keeps executable imported-content surfaces out of all runtime sources", () => {
    expect(runtimeSource).not.toMatch(
      /\biframe\b|\bsrcdoc\b|{@html|\b(?:new\s+)?Function\s*\(|\beval\b|\b(?:Shared)?Worker\b|\bWebAssembly\b|plugin-frame|lorepia:plugin:/i,
    );
  });
});
