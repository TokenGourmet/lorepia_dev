#!/usr/bin/env node

import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const HERE = dirname(fileURLToPath(import.meta.url));
const DEFAULT_MANIFEST = resolve(HERE, "manifest.json");

const CHECK_STATUSES = new Set([
  "not_run",
  "in_progress",
  "pass",
  "fail",
  "blocked",
  "unsupported",
]);
const FINDING_STATUSES = new Set([
  "open",
  "in_progress",
  "fixed",
  "verified",
  "accepted",
]);
const EVIDENCE_KINDS = new Set([
  "commit",
  "path",
  "command",
  "artifact",
  "git",
  "manifest",
  "device",
  "measurement",
]);

function fail(message) {
  throw new Error(message);
}

function isObject(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function requireObject(value, path) {
  if (!isObject(value)) fail(`${path} must be an object`);
  return value;
}

function requireArray(value, path) {
  if (!Array.isArray(value)) fail(`${path} must be an array`);
  return value;
}

function requireNonEmptyString(value, path) {
  if (typeof value !== "string" || value.trim() === "") {
    fail(`${path} must be a non-empty string`);
  }
}

function validateCommit(value, path) {
  if (value !== null && !/^[0-9a-f]{40}$/.test(value)) {
    fail(`${path} must be null or a full lowercase Git commit`);
  }
}

function validateEvidence(value, path) {
  for (const [index, item] of requireArray(value, path).entries()) {
    const current = requireObject(item, `${path}[${index}]`);
    if (!EVIDENCE_KINDS.has(current.kind)) {
      fail(`${path}[${index}].kind is not supported: ${String(current.kind)}`);
    }
    requireNonEmptyString(current.value, `${path}[${index}].value`);
    requireNonEmptyString(current.note, `${path}[${index}].note`);
  }
}

function validateCommands(value, path) {
  for (const [index, command] of requireArray(value, path).entries()) {
    requireNonEmptyString(command, `${path}[${index}]`);
  }
}

function validateTracking(value, path, statuses = CHECK_STATUSES) {
  const tracking = requireObject(value, path);
  if (!statuses.has(tracking.status)) {
    fail(`${path}.status is not supported: ${String(tracking.status)}`);
  }
  validateEvidence(tracking.evidence, `${path}.evidence`);
  validateCommit(tracking.commit, `${path}.commit`);
  validateCommands(tracking.commands, `${path}.commands`);

  if (["pass", "fixed", "verified"].includes(tracking.status)) {
    if (tracking.evidence.length === 0 || tracking.commands.length === 0 || tracking.commit === null) {
      fail(`${path} status ${tracking.status} requires evidence, commands, and a full commit`);
    }
  }
  if (tracking.status === "in_progress" && (tracking.evidence.length === 0 || tracking.commit === null)) {
    fail(`${path} in_progress requires evidence and a full commit`);
  }
  if (tracking.status === "fail" && tracking.evidence.length === 0) {
    fail(`${path} fail requires evidence`);
  }
}

function paddedId(prefix, number) {
  return `${prefix}-${String(number).padStart(3, "0")}`;
}

export function expandExtremeChecks(manifest) {
  const extreme = requireObject(manifest.extremeChecks, "extremeChecks");
  const defaults = requireObject(extreme.defaultTracking, "extremeChecks.defaultTracking");
  const overrides = requireObject(extreme.overrides, "extremeChecks.overrides");
  const records = [];
  const seen = new Set();

  for (const [index, range] of requireArray(extreme.ranges, "extremeChecks.ranges").entries()) {
    requireObject(range, `extremeChecks.ranges[${index}]`);
    if (!/^[A-Z]+$/.test(range.prefix)) {
      fail(`extremeChecks.ranges[${index}].prefix must contain uppercase letters only`);
    }
    if (!Number.isInteger(range.start) || !Number.isInteger(range.end) || range.start < 1 || range.end > 999 || range.start > range.end) {
      fail(`extremeChecks.ranges[${index}] has an invalid inclusive range`);
    }
    for (let number = range.start; number <= range.end; number += 1) {
      const id = paddedId(range.prefix, number);
      if (seen.has(id)) fail(`extreme check range produces duplicate ID ${id}`);
      seen.add(id);
      records.push({ id, ...defaults, ...(overrides[id] ?? {}) });
    }
  }

  for (const id of Object.keys(overrides)) {
    if (!seen.has(id)) fail(`extremeChecks.overrides contains unknown ID ${id}`);
  }
  return records;
}

function expectedSequence(prefix, start, end, width = 0) {
  const result = [];
  for (let value = start; value <= end; value += 1) {
    result.push(width === 0 ? `${prefix}-${value}` : `${prefix}-${String(value).padStart(width, "0")}`);
  }
  return result;
}

function assertExactIds(actualIds, expectedIds, label) {
  const actual = new Set(actualIds);
  const expected = new Set(expectedIds);
  const duplicates = actualIds.filter((id, index) => actualIds.indexOf(id) !== index);
  const missing = expectedIds.filter((id) => !actual.has(id));
  const unknown = [...actual].filter((id) => !expected.has(id));
  if (duplicates.length || missing.length || unknown.length) {
    fail(
      `${label} ID mismatch` +
        `; duplicates=[${[...new Set(duplicates)].join(", ")}]` +
        `; missing=[${missing.join(", ")}]` +
        `; unknown=[${unknown.join(", ")}]`,
    );
  }
}

export function validateManifest(manifest) {
  requireObject(manifest, "manifest");
  if (manifest.schemaVersion !== 1) fail("schemaVersion must be 1");
  if (manifest.project !== "LorePia") fail("project must be LorePia");
  requireObject(manifest.asOf, "asOf");
  validateCommit(manifest.asOf.commit, "asOf.commit");
  requireNonEmptyString(manifest.asOf.date, "asOf.date");
  requireNonEmptyString(manifest.asOf.branch, "asOf.branch");

  const sources = requireObject(manifest.sources, "sources");
  for (const key of ["extremePlan", "launchAudit"]) {
    const source = requireObject(sources[key], `sources.${key}`);
    requireNonEmptyString(source.name, `sources.${key}.name`);
    if (!/^[0-9a-f]{64}$/.test(source.sha256)) {
      fail(`sources.${key}.sha256 must be a lowercase SHA-256 digest`);
    }
  }

  validateTracking(manifest.extremeChecks.defaultTracking, "extremeChecks.defaultTracking");
  for (const [id, tracking] of Object.entries(manifest.extremeChecks.overrides)) {
    if (!/^[A-Z]+-[0-9]{3}$/.test(id)) fail(`invalid extreme override ID ${id}`);
    validateTracking(tracking, `extremeChecks.overrides.${id}`);
  }
  const checks = expandExtremeChecks(manifest);
  if (checks.length !== 365) fail(`expected 365 extreme checks, got ${checks.length}`);
  for (const check of checks) validateTracking(check, `expandedExtremeChecks.${check.id}`);

  const audit = requireObject(manifest.launchAudit, "launchAudit");
  const findings = requireArray(audit.findings, "launchAudit.findings");
  const expectedFindings = [
    ...expectedSequence("P0", 1, 7),
    ...expectedSequence("P1", 1, 11),
  ];
  assertExactIds(findings.map((finding) => finding.id), expectedFindings, "launch finding");
  for (const [index, finding] of findings.entries()) {
    const path = `launchAudit.findings[${index}]`;
    requireObject(finding, path);
    if (!/^P[01]-[0-9]+$/.test(finding.id)) fail(`${path}.id is invalid`);
    if (finding.severity !== finding.id.slice(0, 2)) fail(`${path}.severity does not match its ID`);
    requireNonEmptyString(finding.title, `${path}.title`);
    validateTracking(finding, path, FINDING_STATUSES);
    requireNonEmptyString(finding.remaining, `${path}.remaining`);
  }

  const goConditions = requireArray(audit.goConditions, "launchAudit.goConditions");
  assertExactIds(
    goConditions.map((condition) => condition.id),
    expectedSequence("GO", 1, 17, 3),
    "launch GO condition",
  );
  for (const [index, condition] of goConditions.entries()) {
    const path = `launchAudit.goConditions[${index}]`;
    requireObject(condition, path);
    requireNonEmptyString(condition.title, `${path}.title`);
    validateTracking(condition, path);
  }
  return { extremeChecks: checks.length, findings: findings.length, goConditions: goConditions.length };
}

function normalizeTitle(value) {
  return value.trim().replace(/\s+/g, " ");
}

export function parseExtremeDocument(markdown) {
  const matches = [];
  for (const [zeroBased, line] of markdown.split(/\r?\n/).entries()) {
    const match = line.match(/^\s*-\s*\[[ xX]\]\s+([A-Z]+-[0-9]{3})\b\s*(.*)$/);
    if (match) matches.push({ id: match[1], title: normalizeTitle(match[2]), line: zeroBased + 1 });
  }
  return matches;
}

export function parseLaunchDocument(markdown) {
  const lines = markdown.split(/\r?\n/);
  const findings = [];
  const goConditions = [];
  let inGoSection = false;
  for (const [zeroBased, line] of lines.entries()) {
    const finding = line.match(/^###\s+(P[01]-[0-9]+)\.\s+(.+?)\s*$/);
    if (finding) {
      findings.push({ id: finding[1], title: normalizeTitle(finding[2]), line: zeroBased + 1 });
    }
    if (/^##\s+10\.\s+공개 론칭 GO 조건\s*$/.test(line)) {
      inGoSection = true;
      continue;
    }
    if (inGoSection && /^##\s+/.test(line)) {
      inGoSection = false;
    }
    if (inGoSection) {
      const condition = line.match(/^\s*-\s*\[[ xX]\]\s+(.+?)\s*$/);
      if (condition) {
        goConditions.push({
          id: paddedId("GO", goConditions.length + 1),
          title: normalizeTitle(condition[1]),
          line: zeroBased + 1,
        });
      }
    }
  }
  return { findings, goConditions };
}

function compareDocumentRecords(actual, expected, label) {
  assertExactIds(actual.map((item) => item.id), expected.map((item) => item.id), label);
  const expectedById = new Map(expected.map((item) => [item.id, item]));
  const mismatches = [];
  for (const item of actual) {
    const expectedItem = expectedById.get(item.id);
    if (expectedItem?.title && normalizeTitle(expectedItem.title) !== normalizeTitle(item.title)) {
      mismatches.push(`${item.id}@${item.line}`);
    }
  }
  if (mismatches.length) fail(`${label} title/order mismatch at ${mismatches.join(", ")}`);
}

function sha256(text) {
  return createHash("sha256").update(text).digest("hex");
}

export function validateExtremeDocument(markdown, manifest, { checkHash = true } = {}) {
  if (checkHash && sha256(markdown) !== manifest.sources.extremePlan.sha256) {
    fail("extreme plan SHA-256 differs from manifest; update the manifest intentionally before accepting a changed plan");
  }
  const actual = parseExtremeDocument(markdown);
  const expected = expandExtremeChecks(manifest).map(({ id }) => ({ id }));
  compareDocumentRecords(actual, expected, "extreme plan checklist");
  return actual.length;
}

export function validateLaunchDocument(markdown, manifest, { checkHash = true } = {}) {
  if (checkHash && sha256(markdown) !== manifest.sources.launchAudit.sha256) {
    fail("launch audit SHA-256 differs from manifest; update the manifest intentionally before accepting a changed audit");
  }
  const actual = parseLaunchDocument(markdown);
  compareDocumentRecords(actual.findings, manifest.launchAudit.findings, "launch finding");
  compareDocumentRecords(actual.goConditions, manifest.launchAudit.goConditions, "launch GO condition");
  return { findings: actual.findings.length, goConditions: actual.goConditions.length };
}

export function summarize(manifest) {
  const extreme = expandExtremeChecks(manifest);
  const countBy = (records) =>
    Object.fromEntries(
      [...new Set(records.map((record) => record.status))]
        .sort()
        .map((status) => [status, records.filter((record) => record.status === status).length]),
    );
  return {
    asOf: manifest.asOf,
    extreme: { total: extreme.length, byStatus: countBy(extreme) },
    findings: { total: manifest.launchAudit.findings.length, byStatus: countBy(manifest.launchAudit.findings) },
    goConditions: { total: manifest.launchAudit.goConditions.length, byStatus: countBy(manifest.launchAudit.goConditions) },
  };
}

function loadJson(path) {
  return JSON.parse(readFileSync(path, "utf8"));
}

function parseArgs(argv) {
  const [command = "validate", ...rest] = argv;
  const options = { command, manifest: DEFAULT_MANIFEST, checkHash: true, json: false };
  for (let index = 0; index < rest.length; index += 1) {
    const argument = rest[index];
    if (argument === "--manifest") options.manifest = resolve(rest[++index]);
    else if (argument === "--extreme-doc") options.extremeDoc = resolve(rest[++index]);
    else if (argument === "--launch-doc") options.launchDoc = resolve(rest[++index]);
    else if (argument === "--skip-source-hash") options.checkHash = false;
    else if (argument === "--json") options.json = true;
    else fail(`unknown argument: ${argument}`);
  }
  return options;
}

function printHumanSummary(summary) {
  console.log(`as-of: ${summary.asOf.branch}@${summary.asOf.commit}`);
  console.log(`extreme checks: ${summary.extreme.total} ${JSON.stringify(summary.extreme.byStatus)}`);
  console.log(`launch findings: ${summary.findings.total} ${JSON.stringify(summary.findings.byStatus)}`);
  console.log(`launch GO conditions: ${summary.goConditions.total} ${JSON.stringify(summary.goConditions.byStatus)}`);
}

function main(argv) {
  const options = parseArgs(argv);
  const manifest = loadJson(options.manifest);
  const counts = validateManifest(manifest);

  if (options.command === "validate") {
    if (options.extremeDoc) {
      counts.extremeDocumentChecks = validateExtremeDocument(
        readFileSync(options.extremeDoc, "utf8"),
        manifest,
        { checkHash: options.checkHash },
      );
    }
    if (options.launchDoc) {
      Object.assign(
        counts,
        validateLaunchDocument(readFileSync(options.launchDoc, "utf8"), manifest, {
          checkHash: options.checkHash,
        }),
      );
    }
    console.log(`PASS ${JSON.stringify(counts)}`);
    return;
  }
  if (options.command === "summary") {
    const result = summarize(manifest);
    if (options.json) console.log(JSON.stringify(result, null, 2));
    else printHumanSummary(result);
    return;
  }
  if (options.command === "expand") {
    const result = expandExtremeChecks(manifest);
    console.log(options.json ? JSON.stringify(result, null, 2) : result.map((item) => item.id).join("\n"));
    return;
  }
  fail(`unknown command: ${options.command}`);
}

const invokedDirectly = process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (invokedDirectly) {
  try {
    main(process.argv.slice(2));
  } catch (error) {
    console.error(`FAIL ${error instanceof Error ? error.message : String(error)}`);
    process.exitCode = 1;
  }
}
