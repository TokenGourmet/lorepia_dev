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

// The temporary M0 no-styling guard retired on 2026-07-19 when the
// owner-authored design slice landed (tokens in src/lib/design, screens under
// src/routes). The executable-surface guards below remain permanent until a
// reviewed execution boundary replaces them.
describe("owner-authored product surface boundary", () => {
  it("styles only through the owner design tokens", () => {
    expect(pageSource).toContain('import "$lib/design/tokens.css"');
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
