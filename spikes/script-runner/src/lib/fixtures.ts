import allowedSource from "../../fixtures/allowed.js?raw";
import allocatorPressureSource from "../../fixtures/allocator-pressure.js?raw";
import forbiddenGlobalsSource from "../../fixtures/forbidden-globals.js?raw";
import infiniteLoopSource from "../../fixtures/infinite-loop.js?raw";
import oversizedOutputSource from "../../fixtures/oversized-output.js?raw";
import recursivePressureSource from "../../fixtures/recursive-pressure.js?raw";
import scriptErrorSource from "../../fixtures/script-error.js?raw";

import type { FixtureId } from "./runner-contract";

export const FIXTURE_INPUT_JSON = JSON.stringify({
  text: " lorepia ",
  count: 1,
});

export const EXPECTED_ALLOWED_OUTPUT = JSON.stringify({
  text: "LOREPIA",
  count: 2,
});

export const EXPECTED_FORBIDDEN_GLOBALS_OUTPUT = JSON.stringify({
  tauri: true,
  tauriInternals: true,
  window: true,
  document: true,
  fetch: true,
  xhr: true,
  webSocket: true,
  worker: true,
  importScripts: true,
  process: true,
  require: true,
  deno: true,
});

const FIXTURE_SOURCES = {
  allowed: allowedSource,
  "infinite-loop": infiniteLoopSource,
  "recursive-pressure": recursivePressureSource,
  "allocator-pressure": allocatorPressureSource,
  "forbidden-globals": forbiddenGlobalsSource,
  "oversized-output": oversizedOutputSource,
  "script-error": scriptErrorSource,
} as const satisfies Record<Exclude<FixtureId, "host-watchdog">, string>;

export function fixtureSource(
  fixtureId: Exclude<FixtureId, "host-watchdog">,
): string {
  return FIXTURE_SOURCES[fixtureId];
}

export function allFixtureSources(): ReadonlyArray<readonly [string, string]> {
  return Object.entries(FIXTURE_SOURCES);
}
