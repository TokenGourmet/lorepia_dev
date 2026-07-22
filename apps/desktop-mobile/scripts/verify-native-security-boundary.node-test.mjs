import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  verifyNativeSecurityBoundary,
  verifySecurityContracts,
} from "./verify-native-security-boundary.mjs";

const scriptDirectory = resolve(fileURLToPath(new URL(".", import.meta.url)));
const tauriRoot = resolve(scriptDirectory, "../src-tauri");

function repositoryInputs() {
  const commandSource = readFileSync(resolve(tauriRoot, "src/app_commands.rs"), "utf8");
  const capability = JSON.parse(readFileSync(resolve(tauriRoot, "capabilities/default.json"), "utf8"));
  const tauriConfig = JSON.parse(readFileSync(resolve(tauriRoot, "tauri.conf.json"), "utf8"));
  const permissionFiles = Object.fromEntries(
    capability.permissions
      .filter((permission) => permission.startsWith("allow-"))
      .map((permission) => {
        const command = permission.slice("allow-".length).replaceAll("-", "_");
        return [
          `${command}.toml`,
          `commands.allow = ["${command}"]\ncommands.deny = ["${command}"]\n`,
        ];
      }),
  );
  return { commandSource, capability, tauriConfig, permissionFiles };
}

test("the repository keeps an exact native command, capability, and CSP boundary", () => {
  const result = verifyNativeSecurityBoundary(tauriRoot);
  assert.equal(result.commands, 20);
  assert.equal(result.permissions, 20);
});

test("rejects an added native command even if permissions were not regenerated", () => {
  const inputs = repositoryInputs();
  inputs.commandSource = inputs.commandSource.replace(
    "update_app_preferences,",
    "update_app_preferences,\n            open_shell,",
  );
  assert.throws(() => verifySecurityContracts(inputs), /NATIVE_COMMAND_SURFACE_DRIFT/u);
});

test("rejects wildcard WebView capability and a relaxed release CSP", () => {
  const wildcard = repositoryInputs();
  wildcard.capability.webviews = ["*"];
  assert.throws(() => verifySecurityContracts(wildcard), /EXACT_MAIN_WEBVIEW/u);

  const relaxed = repositoryInputs();
  relaxed.tauriConfig.app.security.csp["connect-src"] = "*";
  assert.throws(() => verifySecurityContracts(relaxed), /RELEASE_CSP_VALUE_DRIFT/u);
});

test("rejects a missing or mismatched generated permission", () => {
  const missing = repositoryInputs();
  delete missing.permissionFiles["create_chat.toml"];
  assert.throws(() => verifySecurityContracts(missing), /GENERATED_PERMISSION_DRIFT/u);

  const mismatched = repositoryInputs();
  mismatched.permissionFiles["create_chat.toml"] =
    'commands.allow = ["create_chat"]\ncommands.deny = ["delete_chat"]\n';
  assert.throws(() => verifySecurityContracts(mismatched), /INVALID_GENERATED_PERMISSION/u);
});
