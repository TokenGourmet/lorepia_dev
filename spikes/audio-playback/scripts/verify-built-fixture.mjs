import { readFileSync, readdirSync } from "node:fs";
import { relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

import {
  EXPECTED_SHA256,
  FIXTURE_SPEC,
  STATIC_FIXTURE_PATH,
  sha256,
} from "./generate-audio-fixture.mjs";

const EXPECTED_BUILT_PATH = `fixtures/${FIXTURE_SPEC.file}`;
const AUDIO_FILE_EXTENSION =
  /\.(?:aac|aif|aiff|caf|flac|m4a|mp3|mp4|oga|ogg|opus|wav|wave|weba|webm)$/i;

function listFiles(root, directory = root) {
  let entries;
  try {
    entries = readdirSync(directory, { withFileTypes: true });
  } catch (error) {
    throw new Error(`Unable to read built output directory: ${directory}`, {
      cause: error,
    });
  }
  return entries.flatMap((entry) => {
    const absolute = resolve(directory, entry.name);
    if (entry.isDirectory()) return listFiles(root, absolute);
    return [relative(root, absolute).split(sep).join("/")];
  });
}

export function verifyBuiltFixture(outputDirectory) {
  const root = resolve(outputDirectory);
  const files = listFiles(root);
  const audioFiles = files.filter((file) => AUDIO_FILE_EXTENSION.test(file));
  const unexpectedAudioFiles = audioFiles.filter(
    (file) => file !== EXPECTED_BUILT_PATH,
  );

  if (unexpectedAudioFiles.length > 0) {
    throw new Error(
      `Built output contains unexpected audio files: ${unexpectedAudioFiles.join(", ")}`,
    );
  }
  if (!audioFiles.includes(EXPECTED_BUILT_PATH)) {
    throw new Error(`Built output is missing ${EXPECTED_BUILT_PATH}`);
  }

  const builtFixture = readFileSync(resolve(root, EXPECTED_BUILT_PATH));
  if (builtFixture.length !== FIXTURE_SPEC.bytes) {
    throw new Error(
      `Built fixture has ${builtFixture.length} bytes; expected ${FIXTURE_SPEC.bytes}`,
    );
  }
  const builtSha256 = sha256(builtFixture);
  if (builtSha256 !== EXPECTED_SHA256) {
    throw new Error(
      `Built fixture SHA-256 ${builtSha256} does not match ${EXPECTED_SHA256}`,
    );
  }

  const sourceFixture = readFileSync(STATIC_FIXTURE_PATH);
  if (!builtFixture.equals(sourceFixture)) {
    throw new Error("Built fixture differs byte-for-byte from the source fixture");
  }

  return Object.freeze({
    audioFiles: Object.freeze([...audioFiles]),
    bytes: builtFixture.length,
    sha256: builtSha256,
  });
}

function runCli() {
  const args = process.argv.slice(2);
  const normalizedArgs = args[0] === "--check" ? args.slice(1) : args;
  if (normalizedArgs.length > 1) {
    throw new Error(
      "Usage: node scripts/verify-built-fixture.mjs [--check] [output-directory]",
    );
  }
  const outputDirectory =
    normalizedArgs[0] ?? fileURLToPath(new URL("../build/", import.meta.url));
  const result = verifyBuiltFixture(outputDirectory);
  process.stdout.write(
    `verified ${EXPECTED_BUILT_PATH}: ${result.bytes} bytes sha256 ${result.sha256}; ${result.audioFiles.length} audio file\n`,
  );
}

const invokedPath = process.argv[1] ? resolve(process.argv[1]) : "";
if (invokedPath === fileURLToPath(import.meta.url)) {
  try {
    runCli();
  } catch (error) {
    process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`);
    process.exitCode = 1;
  }
}
