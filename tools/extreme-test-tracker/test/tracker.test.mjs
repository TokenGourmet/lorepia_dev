import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

import {
  expandExtremeChecks,
  parseExtremeDocument,
  parseLaunchDocument,
  summarize,
  validateExtremeDocument,
  validateLaunchDocument,
  validateManifest,
} from "../tracker.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const MANIFEST_PATH = resolve(HERE, "../manifest.json");
const EXECUTION_POLICY_PATH = resolve(
  HERE,
  "../../../docs/m1/extreme-and-soak-execution-policy.md",
);

function manifest() {
  return JSON.parse(readFileSync(MANIFEST_PATH, "utf8"));
}

function clone(value) {
  return structuredClone(value);
}

function syntheticExtreme(current) {
  return [
    "# synthetic extreme plan",
    "### CHAT-001~040 declaration is not a checklist item",
    ...expandExtremeChecks(current).map(({ id }) => `- [ ] ${id} generated title`),
    "",
  ].join("\n");
}

function syntheticLaunch(current) {
  return [
    "# synthetic launch audit",
    ...current.launchAudit.findings.map(({ id, title }) => `### ${id}. ${title}`),
    "## 10. 공개 론칭 GO 조건",
    ...current.launchAudit.goConditions.map(({ title }) => `- [ ] ${title}`),
    "## 11. next section",
    "- [ ] this line is not a GO condition",
    "",
  ].join("\n");
}

test("manifest expands the complete 365-ID extreme catalog", () => {
  const current = manifest();
  assert.deepEqual(validateManifest(current), {
    extremeChecks: 365,
    findings: 18,
    goConditions: 17,
  });
  const checks = expandExtremeChecks(current);
  assert.equal(checks[0].id, "CHAT-001");
  assert.equal(checks.at(-1).id, "REL-020");
  assert.equal(new Set(checks.map(({ id }) => id)).size, 365);
  assert.ok(checks.every(({ status }) => status === "not_run"));
});

test("execution policy does not define a second extreme checklist", () => {
  const source = readFileSync(EXECUTION_POLICY_PATH, "utf8");
  assert.deepEqual(parseExtremeDocument(source), []);
});

test("range declarations are ignored and checklist IDs are parsed", () => {
  const parsed = parseExtremeDocument(
    "### CHAT-001~020\n- [ ] CHAT-001 first\n- [x] CHAT-002 second\n",
  );
  assert.deepEqual(parsed, [
    { id: "CHAT-001", title: "first", line: 2 },
    { id: "CHAT-002", title: "second", line: 3 },
  ]);
});

test("extreme source validation accepts the complete generated catalog", () => {
  const current = manifest();
  assert.equal(
    validateExtremeDocument(syntheticExtreme(current), current, { checkHash: false }),
    365,
  );
});

test("extreme source validation reports a missing ID", () => {
  const current = manifest();
  const source = syntheticExtreme(current).replace("- [ ] DB-017 generated title\n", "");
  assert.throws(
    () => validateExtremeDocument(source, current, { checkHash: false }),
    /missing=\[DB-017\]/,
  );
});

test("extreme source validation reports a duplicate ID", () => {
  const current = manifest();
  const source = `${syntheticExtreme(current)}- [ ] STREAM-004 duplicate\n`;
  assert.throws(
    () => validateExtremeDocument(source, current, { checkHash: false }),
    /duplicates=\[STREAM-004\]/,
  );
});

test("manifest validation rejects overlapping ranges and unknown overrides", () => {
  const overlap = clone(manifest());
  overlap.extremeChecks.ranges.push({ prefix: "CHAT", start: 40, end: 41 });
  assert.throws(() => validateManifest(overlap), /duplicate ID CHAT-040/);

  const unknown = clone(manifest());
  unknown.extremeChecks.overrides["CHAT-999"] = clone(unknown.extremeChecks.defaultTracking);
  assert.throws(() => validateManifest(unknown), /unknown ID CHAT-999/);
});

test("positive statuses require reproducible evidence", () => {
  const current = clone(manifest());
  current.extremeChecks.overrides["CHAT-001"] = {
    status: "pass",
    evidence: [],
    commit: null,
    commands: [],
  };
  assert.throws(() => validateManifest(current), /pass requires evidence, commands, and a full commit/);
});

test("launch parser and validator retain P0, P1, and ordered GO conditions", () => {
  const current = manifest();
  const source = syntheticLaunch(current);
  const parsed = parseLaunchDocument(source);
  assert.equal(parsed.findings.length, 18);
  assert.equal(parsed.goConditions.length, 17);
  assert.equal(parsed.goConditions[0].id, "GO-001");
  assert.equal(parsed.goConditions.at(-1).id, "GO-017");
  assert.deepEqual(
    validateLaunchDocument(source, current, { checkHash: false }),
    { findings: 18, goConditions: 17 },
  );
});

test("launch validation detects duplicate findings and GO reordering", () => {
  const current = manifest();
  const duplicateFinding = syntheticLaunch(current).replace(
    "## 10. 공개 론칭 GO 조건",
    `### P0-1. ${current.launchAudit.findings[0].title}\n## 10. 공개 론칭 GO 조건`,
  );
  assert.throws(
    () => validateLaunchDocument(duplicateFinding, current, { checkHash: false }),
    /duplicates=\[P0-1\]/,
  );

  const reordered = clone(current);
  [reordered.launchAudit.goConditions[0], reordered.launchAudit.goConditions[1]] = [
    reordered.launchAudit.goConditions[1],
    reordered.launchAudit.goConditions[0],
  ];
  assert.throws(
    () => validateLaunchDocument(syntheticLaunch(reordered), current, { checkHash: false }),
    /title\/order mismatch/,
  );
});

test("source hashes are fail-closed unless explicitly skipped", () => {
  const current = manifest();
  assert.throws(
    () => validateExtremeDocument(syntheticExtreme(current), current),
    /SHA-256 differs/,
  );
  assert.throws(
    () => validateLaunchDocument(syntheticLaunch(current), current),
    /SHA-256 differs/,
  );
});

test("summary reports conservative current status", () => {
  const result = summarize(manifest());
  assert.deepEqual(result.extreme.byStatus, { not_run: 365 });
  assert.equal(result.findings.total, 18);
  assert.equal(result.goConditions.total, 17);
  assert.equal(result.goConditions.byStatus.fail, 3);
});
