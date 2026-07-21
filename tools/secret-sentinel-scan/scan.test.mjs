import assert from "node:assert/strict";
import {
  mkdtempSync,
  mkdirSync,
  readFileSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { buildSentinelVariants, scanArtifactRoots } from "./scan.mjs";

const SENTINEL = "lorepia-test-key-1234567890";

function fixture() {
  const parent = mkdtempSync(join(tmpdir(), "lorepia-secret-scan-"));
  const root = join(parent, "artifacts");
  mkdirSync(root);
  const sentinelFile = join(parent, "sentinel");
  writeFileSync(sentinelFile, SENTINEL, { mode: 0o600 });
  return { parent, root, sentinelFile };
}

test("finds every unique encoding without putting the sentinel in the receipt", async () => {
  const { parent, root, sentinelFile } = fixture();
  const variants = buildSentinelVariants(Buffer.from(SENTINEL));
  writeFileSync(
    join(root, "encoded.bin"),
    Buffer.concat(variants.map(({ pattern }) => Buffer.concat([pattern, Buffer.from("\n")]))),
  );
  const receiptPath = join(parent, "receipt.json");
  const receipt = await scanArtifactRoots({ roots: [root], sentinelFile, receiptPath });
  const serialized = readFileSync(receiptPath, "utf8");

  assert.equal(receipt.passed, false);
  assert.equal(receipt.matches.length, variants.length);
  assert.equal(serialized.includes(SENTINEL), false);
  assert.equal(serialized.includes(Buffer.from(SENTINEL).toString("hex")), false);
  assert.match(serialized, /"pathSha256": "[a-f0-9]{64}"/u);
});

test("detects a raw sentinel split across the scanner chunk boundary", async () => {
  const { root, sentinelFile } = fixture();
  const prefix = Buffer.alloc(64 * 1024 - 3, 0x61);
  writeFileSync(join(root, "split.bin"), Buffer.concat([prefix, Buffer.from(SENTINEL)]));

  const receipt = await scanArtifactRoots({ roots: [root], sentinelFile });
  const raw = receipt.matches.find((match) => match.variant === "RAW_ASCII");
  assert.equal(receipt.passed, false);
  assert.equal(raw.firstOffset, prefix.length);
  assert.equal(raw.location, "CONTENT");
});

test("a clean tree passes and symlinks or an in-tree sentinel fail closed", async () => {
  const { root, sentinelFile } = fixture();
  writeFileSync(join(root, "clean.txt"), "no credentials here");
  const clean = await scanArtifactRoots({ roots: [root], sentinelFile });
  assert.equal(clean.passed, true);
  assert.equal(clean.matches.length, 0);

  const linked = join(root, "linked");
  symlinkSync(join(root, "clean.txt"), linked);
  await assert.rejects(
    scanArtifactRoots({ roots: [root], sentinelFile }),
    /SYMLINK_ENTRY_REJECTED/u,
  );

  const nestedSentinel = join(root, "sentinel");
  writeFileSync(nestedSentinel, SENTINEL);
  await assert.rejects(
    scanArtifactRoots({ roots: [root], sentinelFile: nestedSentinel }),
    /SENTINEL_FILE_INSIDE_SCAN_ROOT/u,
  );
});

test("sentinels are bounded printable ASCII values", () => {
  assert.throws(() => buildSentinelVariants(Buffer.from("short")), /INVALID_SENTINEL/u);
  assert.throws(
    () => buildSentinelVariants(Buffer.from("valid-prefix\nsecret")),
    /INVALID_SENTINEL/u,
  );
  assert.throws(
    () => buildSentinelVariants(Buffer.alloc(513, 0x41)),
    /INVALID_SENTINEL/u,
  );
});
