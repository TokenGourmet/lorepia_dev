import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";

import { describe, expect, it } from "vitest";

function productionSources(directory: URL): URL[] {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const child = new URL(entry.isDirectory() ? `${entry.name}/` : entry.name, directory);
    if (entry.isDirectory()) return productionSources(child);
    if (!statSync(child).isFile()) return [];
    if (entry.name.endsWith(".test.ts") || entry.name.endsWith(".test.js")) return [];
    return /\.(?:[cm]?[jt]sx?|svelte|html)$/.test(entry.name) ? [child] : [];
  });
}

const sourceRoot = new URL("../", import.meta.url);
const staticRoot = new URL("../../static/", import.meta.url);

function allProductionText(): string {
  const roots = [sourceRoot];
  if (existsSync(staticRoot)) roots.push(staticRoot);
  return roots
    .flatMap(productionSources)
    .map((path) => readFileSync(path, "utf8"))
    .join("\n");
}

describe("zero-IPC audio candidate surface", () => {
  it("contains no Tauri API import, invoke call, bridge global, or native command handler", () => {
    const production = allProductionText();
    const rust = readFileSync(
      new URL("../../src-tauri/src/lib.rs", import.meta.url),
      "utf8",
    );
    expect(production).not.toContain("@tauri-apps/api");
    expect(production).not.toMatch(/\binvoke\s*\(/);
    expect(production).not.toContain("__TAURI__");
    expect(production).not.toContain("__TAURI_INTERNALS__");
    expect(rust).toContain("const NATIVE_COMMANDS: &[&str] = &[];");
    expect(rust).not.toContain("invoke_handler");
    expect(rust).not.toContain("#[tauri::command]");
  });

  it("grants no capability permission to the trusted main WebView", () => {
    const capability = JSON.parse(
      readFileSync(
        new URL("../../src-tauri/capabilities/default.json", import.meta.url),
        "utf8",
      ),
    ) as Record<string, unknown>;
    expect(capability).toEqual({
      $schema: "../gen/schemas/desktop-schema.json",
      identifier: "default",
      description: "Static capability for the trusted Audio M-1 WebView",
      webviews: ["main"],
      permissions: [],
    });
  });

  it("has no audio backend dependency and no runtime Tauri JavaScript dependency", () => {
    const packageJson = JSON.parse(
      readFileSync(new URL("../../package.json", import.meta.url), "utf8"),
    ) as { dependencies?: Record<string, string>; devDependencies: Record<string, string> };
    const cargo = readFileSync(
      new URL("../../src-tauri/Cargo.toml", import.meta.url),
      "utf8",
    );
    expect(packageJson.dependencies ?? {}).toEqual({});
    expect(packageJson.devDependencies).not.toHaveProperty("@tauri-apps/api");
    expect(cargo).not.toMatch(/rodio|cpal|symphonia|avfoundation|media3/i);
    expect(cargo.match(/^tauri\s*=.*$/gm)).toHaveLength(1);
  });

  it("pins one same-origin fetch and creates playback only from verified bytes", () => {
    const fixtureSource = readFileSync(
      new URL("./audio-fixture.ts", import.meta.url),
      "utf8",
    );
    const controllerSource = readFileSync(
      new URL("./audio-controller.ts", import.meta.url),
      "utf8",
    );
    const contractSource = readFileSync(
      new URL("./audio-contract.ts", import.meta.url),
      "utf8",
    );
    expect(contractSource.match(/publicPath: "\/fixtures\/m1-audio-v1\.wav"/g)).toHaveLength(1);
    expect(fixtureSource.match(/fixedFetch\(AUDIO_FIXTURE\.publicPath/g)).toHaveLength(1);
    expect(fixtureSource).toContain('redirect: "error"');
    expect(fixtureSource).toContain('credentials: "same-origin"');
    expect(controllerSource.match(/createObjectUrl\(/g)).toHaveLength(1);
    expect(controllerSource).toContain('new Blob([byteCopy.buffer], { type: "audio/wav" })');
    expect(controllerSource).not.toMatch(/https?:\/\//);
  });

  it("keeps the diagnostic page plain and limited to eight functional buttons", () => {
    const page = readFileSync(
      new URL("../routes/+page.svelte", import.meta.url),
      "utf8",
    );
    expect(page.match(/<h1\b/g)).toHaveLength(1);
    expect(page.match(/<button\b/g)).toHaveLength(8);
    expect(page.match(/<pre\b/g)).toHaveLength(1);
    expect(page).not.toMatch(/<audio\b|<input\b|<style\b|\bclass=|\btransition:|\banimate:|\buse:/);
  });

  it("allows only local and blob media in the production CSP", () => {
    const config = JSON.parse(
      readFileSync(new URL("../../src-tauri/tauri.conf.json", import.meta.url), "utf8"),
    ) as {
      app: { security: { csp: Record<string, string>; devCsp: Record<string, string> } };
    };
    expect(config.app.security.csp["media-src"]).toBe(
      "'self' blob: asset: http://asset.localhost",
    );
    expect(config.app.security.csp["connect-src"]).toBe("'self'");
    expect(config.app.security.csp["frame-src"]).toBe("'none'");
  });
});
