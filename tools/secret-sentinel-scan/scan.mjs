import {
  closeSync,
  createReadStream,
  existsSync,
  fsyncSync,
  lstatSync,
  mkdirSync,
  openSync,
  opendirSync,
  readFileSync,
  realpathSync,
  renameSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { createHash } from "node:crypto";
import { basename, dirname, join, relative, resolve, sep } from "node:path";
import { pathToFileURL } from "node:url";

const SCANNER_VERSION = 1;
const CHUNK_BYTES = 64 * 1024;
const MAX_SENTINEL_BYTES = 512;
const MIN_SENTINEL_BYTES = 8;
const MAX_FILES = 1_000_000;
const MAX_MATCHES_PER_FILE_VARIANT = 1_000;

function fail(code) {
  throw new Error(code);
}

function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}

function assertPrintableAscii(bytes) {
  if (
    bytes.length < MIN_SENTINEL_BYTES ||
    bytes.length > MAX_SENTINEL_BYTES ||
    !bytes.every((byte) => byte >= 0x21 && byte <= 0x7e)
  ) {
    fail("INVALID_SENTINEL");
  }
}

export function buildSentinelVariants(sentinelBytes) {
  const bytes = Buffer.from(sentinelBytes);
  assertPrintableAscii(bytes);
  const text = bytes.toString("utf8");
  const utf16le = Buffer.from(text, "utf16le");
  const utf16be = Buffer.allocUnsafe(utf16le.length);
  for (let index = 0; index < utf16le.length; index += 2) {
    utf16be[index] = utf16le[index + 1];
    utf16be[index + 1] = utf16le[index];
  }
  const candidates = [
    ["RAW_ASCII", bytes],
    ["HEX_LOWER", Buffer.from(bytes.toString("hex"), "ascii")],
    ["HEX_UPPER", Buffer.from(bytes.toString("hex").toUpperCase(), "ascii")],
    ["BASE64", Buffer.from(bytes.toString("base64"), "ascii")],
    [
      "BASE64URL_UNPADDED",
      Buffer.from(bytes.toString("base64url"), "ascii"),
    ],
    [
      "PERCENT_ENCODED_UPPER",
      Buffer.from(
        [...bytes].map((byte) => `%${byte.toString(16).padStart(2, "0").toUpperCase()}`).join(""),
        "ascii",
      ),
    ],
    [
      "PERCENT_ENCODED_LOWER",
      Buffer.from(
        [...bytes].map((byte) => `%${byte.toString(16).padStart(2, "0")}`).join(""),
        "ascii",
      ),
    ],
    ["JSON_ESCAPED", Buffer.from(JSON.stringify(text).slice(1, -1), "utf8")],
    ["UTF16_LE", utf16le],
    ["UTF16_BE", utf16be],
  ];
  const unique = new Map();
  for (const [name, pattern] of candidates) {
    const key = pattern.toString("hex");
    if (!unique.has(key)) unique.set(key, Object.freeze({ name, pattern }));
  }
  return Object.freeze([...unique.values()]);
}

function safeRelative(root, path) {
  const value = relative(root, path);
  if (
    value === "" ||
    value === ".." ||
    value.startsWith(`..${sep}`) ||
    value.startsWith(sep)
  ) {
    fail("SCAN_PATH_ESCAPE");
  }
  return value.split(sep).join("/");
}

function listFiles(root) {
  const rootStat = lstatSync(root);
  if (rootStat.isSymbolicLink()) fail("SYMLINK_SCAN_ROOT_REJECTED");
  if (rootStat.isFile()) {
    return [{ path: root, relativePath: basename(root) }];
  }
  if (!rootStat.isDirectory()) fail("SPECIAL_SCAN_ROOT_REJECTED");

  const files = [];
  const pending = [root];
  while (pending.length > 0) {
    const directory = pending.pop();
    const entries = [];
    const handle = opendirSync(directory);
    try {
      for (;;) {
        const entry = handle.readSync();
        if (entry === null) break;
        entries.push(entry.name);
      }
    } finally {
      handle.closeSync();
    }
    entries.sort((left, right) => left.localeCompare(right));
    for (let index = entries.length - 1; index >= 0; index -= 1) {
      const path = join(directory, entries[index]);
      const stat = lstatSync(path);
      if (stat.isSymbolicLink()) fail("SYMLINK_ENTRY_REJECTED");
      if (stat.isDirectory()) {
        pending.push(path);
      } else if (stat.isFile()) {
        files.push({ path, relativePath: safeRelative(root, path) });
        if (files.length > MAX_FILES) fail("SCAN_FILE_LIMIT_EXCEEDED");
      } else {
        fail("SPECIAL_FILE_REJECTED");
      }
    }
  }
  files.sort((left, right) => left.relativePath.localeCompare(right.relativePath));
  return files;
}

function scanBuffer(buffer, variants, minimumEnd = 0) {
  const findings = [];
  for (const { name, pattern } of variants) {
    let count = 0;
    let firstOffset = null;
    let saturated = false;
    let from = 0;
    for (;;) {
      const index = buffer.indexOf(pattern, from);
      if (index < 0) break;
      if (index + pattern.length > minimumEnd) {
        if (firstOffset === null) firstOffset = index;
        count += 1;
        if (count >= MAX_MATCHES_PER_FILE_VARIANT) {
          saturated = true;
          break;
        }
      }
      from = index + 1;
    }
    if (count > 0) findings.push({ variant: name, firstOffset, count, saturated });
  }
  return findings;
}

async function scanFile(path, variants) {
  const maximumPatternBytes = Math.max(...variants.map(({ pattern }) => pattern.length));
  let tail = Buffer.alloc(0);
  let consumed = 0;
  const aggregated = new Map();
  for await (const chunk of createReadStream(path, { highWaterMark: CHUNK_BYTES })) {
    const buffer = tail.length === 0 ? chunk : Buffer.concat([tail, chunk]);
    const baseOffset = consumed - tail.length;
    for (const finding of scanBuffer(buffer, variants, tail.length)) {
      const prior = aggregated.get(finding.variant);
      if (prior === undefined) {
        aggregated.set(finding.variant, {
          variant: finding.variant,
          firstOffset: baseOffset + finding.firstOffset,
          count: finding.count,
          saturated: finding.saturated,
        });
      } else if (!prior.saturated) {
        prior.count = Math.min(
          MAX_MATCHES_PER_FILE_VARIANT,
          prior.count + finding.count,
        );
        prior.saturated =
          finding.saturated || prior.count >= MAX_MATCHES_PER_FILE_VARIANT;
      }
    }
    consumed += chunk.length;
    const tailBytes = Math.min(maximumPatternBytes - 1, buffer.length);
    tail = Buffer.from(buffer.subarray(buffer.length - tailBytes));
  }
  return { bytes: consumed, findings: [...aggregated.values()] };
}

function parseArguments(args) {
  const roots = [];
  let sentinelFile = null;
  let receipt = null;
  for (let index = 0; index < args.length; index += 1) {
    const name = args[index];
    const value = args[index + 1];
    if (!["--root", "--sentinel-file", "--receipt"].includes(name) || value === undefined) {
      fail("INVALID_ARGUMENTS");
    }
    index += 1;
    if (name === "--root") roots.push(value);
    if (name === "--sentinel-file") sentinelFile = value;
    if (name === "--receipt") receipt = value;
  }
  if (roots.length === 0 || sentinelFile === null || receipt === null) {
    fail("MISSING_ARGUMENTS");
  }
  return { roots, sentinelFile, receipt };
}

function writeReceipt(path, receipt) {
  const target = resolve(path);
  if (existsSync(target)) fail("RECEIPT_ALREADY_EXISTS");
  mkdirSync(dirname(target), { recursive: true });
  const temporary = join(
    dirname(target),
    `.${basename(target)}.${process.pid}.temporary`,
  );
  let descriptor;
  try {
    descriptor = openSync(temporary, "wx", 0o600);
    writeFileSync(descriptor, `${JSON.stringify(receipt, null, 2)}\n`, "utf8");
    fsyncSync(descriptor);
    closeSync(descriptor);
    descriptor = undefined;
    renameSync(temporary, target);
    // Windows does not permit opening a directory this way. The file itself
    // is still fsync'd above; POSIX hosts additionally persist the rename.
    if (process.platform !== "win32") {
      const directory = openSync(dirname(target), "r");
      try {
        fsyncSync(directory);
      } finally {
        closeSync(directory);
      }
    }
  } finally {
    if (descriptor !== undefined) closeSync(descriptor);
    rmSync(temporary, { force: true });
  }
}

export async function scanArtifactRoots({ roots, sentinelFile, receiptPath = null }) {
  const sentinelPath = realpathSync(resolve(sentinelFile));
  const sentinelStat = lstatSync(sentinelPath);
  if (!sentinelStat.isFile() || sentinelStat.isSymbolicLink()) fail("INVALID_SENTINEL_FILE");
  const variants = buildSentinelVariants(readFileSync(sentinelPath));
  const resolvedRoots = roots.map((root) => realpathSync(resolve(root)));
  for (const root of resolvedRoots) {
    if (root === sentinelPath || sentinelPath.startsWith(`${root}${sep}`)) {
      fail("SENTINEL_FILE_INSIDE_SCAN_ROOT");
    }
  }

  let fileCount = 0;
  let totalBytes = 0;
  const matches = [];
  for (let rootIndex = 0; rootIndex < resolvedRoots.length; rootIndex += 1) {
    const root = resolvedRoots[rootIndex];
    for (const file of listFiles(root)) {
      fileCount += 1;
      if (fileCount > MAX_FILES) fail("SCAN_FILE_LIMIT_EXCEEDED");
      const pathFindings = scanBuffer(Buffer.from(file.relativePath, "utf8"), variants);
      const content = await scanFile(file.path, variants);
      totalBytes += content.bytes;
      if (!Number.isSafeInteger(totalBytes)) fail("SCAN_BYTE_COUNT_OVERFLOW");
      const pathDigest = sha256(`${rootIndex}:${file.relativePath}`);
      for (const finding of pathFindings) {
        matches.push({
          rootIndex,
          pathSha256: pathDigest,
          location: "PATH",
          ...finding,
        });
      }
      for (const finding of content.findings) {
        matches.push({
          rootIndex,
          pathSha256: pathDigest,
          location: "CONTENT",
          ...finding,
        });
      }
    }
  }
  const receipt = {
    artifactKind: "LOREPIA_SECRET_SENTINEL_SCAN",
    scannerVersion: SCANNER_VERSION,
    passed: matches.length === 0,
    complete: true,
    rootCount: resolvedRoots.length,
    fileCount,
    totalBytes,
    sentinelBytes: variants[0].pattern.length,
    variantCount: variants.length,
    matches,
  };
  if (receiptPath !== null) writeReceipt(receiptPath, receipt);
  return receipt;
}

async function main() {
  const parsed = parseArguments(process.argv.slice(2));
  const receipt = await scanArtifactRoots({
    roots: parsed.roots,
    sentinelFile: parsed.sentinelFile,
    receiptPath: parsed.receipt,
  });
  process.stdout.write(
    `${receipt.passed ? "PASS" : "FAIL"}: scanned ${receipt.fileCount} files (${receipt.totalBytes} bytes), matches=${receipt.matches.length}\n`,
  );
  if (!receipt.passed) process.exitCode = 2;
}

const invoked = process.argv[1] ? pathToFileURL(resolve(process.argv[1])).href : "";
if (import.meta.url === invoked) {
  main().catch((error) => {
    process.stderr.write(`secret-sentinel-scan: ${error instanceof Error ? error.message : "UNKNOWN_ERROR"}\n`);
    process.exitCode = 1;
  });
}
