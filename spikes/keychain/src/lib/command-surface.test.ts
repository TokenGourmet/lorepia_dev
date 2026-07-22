import { readdirSync, readFileSync, statSync } from "node:fs";

import { describe, expect, it } from "vitest";
import ts from "typescript";

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
  it("contains one direct literal invoke and only the M-1 lifecycle probe", () => {
    const coreImports: Array<{
      source: string;
      imported: string;
      local: string;
    }> = [];
    const unsafeInvokeBindingUses: Array<{ source: string; use: string }> = [];
    let coreModuleReferences = 0;
    const invocations = productionSources(sourceRoot).flatMap((sourcePath) => {
      const source = readFileSync(sourcePath, "utf8");
      const sourceName = sourcePath.pathname.split("/").at(-1) ?? sourcePath.pathname;
      coreModuleReferences += source.split("@tauri-apps/api/core").length - 1;
      const syntax = ts.createSourceFile(
        sourcePath.pathname,
        source,
        ts.ScriptTarget.Latest,
        true,
        sourcePath.pathname.endsWith(".svelte")
          ? ts.ScriptKind.TSX
          : ts.ScriptKind.TS,
      );
      const invokeBindings = new Set<string>();
      const calls: Array<{ callee: string; command: string | null; arguments: number }> = [];

      for (const statement of syntax.statements) {
        if (
          !ts.isImportDeclaration(statement) ||
          !ts.isStringLiteral(statement.moduleSpecifier) ||
          statement.moduleSpecifier.text !== "@tauri-apps/api/core"
        ) {
          continue;
        }
        const bindings = statement.importClause?.namedBindings;
        if (!bindings || !ts.isNamedImports(bindings)) {
          coreImports.push({ source: sourceName, imported: "<non-named>", local: "<non-named>" });
          continue;
        }
        for (const element of bindings.elements) {
          const imported = element.propertyName?.text ?? element.name.text;
          const local = element.name.text;
          coreImports.push({ source: sourceName, imported, local });
          if (imported === "invoke") invokeBindings.add(local);
        }
      }

      const visit = (node: ts.Node): void => {
        if (
          ts.isIdentifier(node) &&
          invokeBindings.has(node.text) &&
          !ts.isImportSpecifier(node.parent) &&
          !(ts.isCallExpression(node.parent) && node.parent.expression === node)
        ) {
          unsafeInvokeBindingUses.push({
            source: sourceName,
            use: node.parent.getText(syntax),
          });
        }
        if (ts.isCallExpression(node)) {
          const expression = node.expression;
          const isInvoke =
            (ts.isIdentifier(expression) &&
              (expression.text === "invoke" || invokeBindings.has(expression.text))) ||
            (ts.isPropertyAccessExpression(expression) && expression.name.text === "invoke") ||
            (ts.isElementAccessExpression(expression) &&
              expression.argumentExpression !== undefined &&
              ts.isStringLiteral(expression.argumentExpression) &&
              expression.argumentExpression.text === "invoke");
          if (isInvoke) {
            const first = node.arguments[0];
            calls.push({
              callee: expression.getText(syntax),
              command:
                first !== undefined && ts.isStringLiteral(first) ? first.text : null,
              arguments: node.arguments.length,
            });
          }
        }
        ts.forEachChild(node, visit);
      };
      visit(syntax);
      return calls;
    });

    expect(coreModuleReferences).toBe(1);
    expect(coreImports).toEqual([
      {
        source: "keychain-probe.ts",
        imported: "invoke",
        local: "invoke",
      },
    ]);
    expect(unsafeInvokeBindingUses).toEqual([]);
    expect(invocations).toEqual([
      {
        callee: "invoke",
        command: "run_keychain_m1_probe",
        arguments: 1,
      },
    ]);
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

  it("does not bypass the imported API through Tauri globals", () => {
    const production = productionSources(sourceRoot)
      .map((sourcePath) => readFileSync(sourcePath, "utf8"))
      .join("\n");

    expect(production).not.toContain("__TAURI__");
    expect(production).not.toContain("__TAURI_INTERNALS__");
  });
});

describe("native capability surface", () => {
  it("grants the exact probe command only to the main WebView", () => {
    const capability = JSON.parse(
      readFileSync(
        new URL("../../src-tauri/capabilities/default.json", import.meta.url),
        "utf8",
      ),
    ) as Record<string, unknown>;

    expect(capability).toEqual({
      $schema: "../gen/schemas/desktop-schema.json",
      identifier: "default",
      description: "Exact keychain probe capability for the trusted main WebView",
      webviews: ["main"],
      permissions: ["allow-run-keychain-m1-probe"],
    });
    expect(capability).not.toHaveProperty("windows");
    expect(capability).not.toHaveProperty("remote");
  });

  it("has one generated permission file for the exact command", () => {
    const permissionsDirectory = new URL(
      "../../src-tauri/permissions/autogenerated/",
      import.meta.url,
    );
    const files = readdirSync(permissionsDirectory).sort();
    expect(files).toEqual(["run_keychain_m1_probe.toml"]);

    const permission = readFileSync(new URL(files[0], permissionsDirectory), "utf8");
    expect(permission.match(/^identifier = /gm)).toHaveLength(2);
    expect(permission).toContain('identifier = "allow-run-keychain-m1-probe"');
    expect(permission).toContain('commands.allow = ["run_keychain_m1_probe"]');
    expect(permission).toContain('identifier = "deny-run-keychain-m1-probe"');
    expect(permission).toContain('commands.deny = ["run_keychain_m1_probe"]');
    expect(permission).not.toMatch(/commands\.(?:allow|deny) = \[[^\]]*,/);
  });

  it("enables only the named capability in Tauri config", () => {
    const config = JSON.parse(
      readFileSync(new URL("../../src-tauri/tauri.conf.json", import.meta.url), "utf8"),
    ) as {
      app?: { security?: { capabilities?: unknown }; windows?: Array<{ label?: string }> };
    };

    expect(config.app?.security?.capabilities).toEqual(["default"]);
    expect(config.app?.windows).toEqual([
      expect.objectContaining({ label: "main" }),
    ]);
  });
});
