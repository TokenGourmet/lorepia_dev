import { spawnSync } from "node:child_process";
import {
  existsSync,
  lstatSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { basename, extname, join, relative, resolve } from "node:path";
import { inflateRawSync } from "node:zlib";
import { fileURLToPath, pathToFileURL } from "node:url";

const MAX_ARTIFACT_BYTES = 1024 * 1024 * 1024;
const MAX_ARCHIVE_ENTRIES = 200_000;
const MAX_ARCHIVE_ENTRY_BYTES = 512 * 1024 * 1024;
const MAX_ARCHIVE_EXPANDED_BYTES = 2 * 1024 * 1024 * 1024;
const SEARCH_CHUNK_BYTES = 1024 * 1024;
const SEARCH_OVERLAP_BYTES = 512;

// A runnable WebAssembly v1 module begins with both the four-byte magic and the
// four-byte version. Searching for the magic alone produces false positives in
// ordinary native machine code and string/data tables (observed in ELF and
// Android .so artifacts). Paths ending in .wasm are rejected separately, while
// this complete header still catches a module hidden in another resource or
// embedded in a native binary.
const WASM_V1_HEADER = Buffer.from([
  0x00, 0x61, 0x73, 0x6d,
  0x01, 0x00, 0x00, 0x00,
]);
const FORBIDDEN_PATH_PATTERNS = Object.freeze([
  ["QuickJS artifact", /(?:^|[\/_-])(?:lib)?quickjs(?:[-_.\/]|$)|quickjs-wasm/iu],
  ["Lua artifact", /(?:^|[\/_-])(?:lib)?lua(?:jit|5[1-4]|[-_.\/]|$)|\.lua(?:c)?$/iu],
  ["script-runner artifact", /(?:^|[\/_-])script[-_ ]runner(?:[-_.\/]|$)/iu],
  ["WebAssembly artifact", /\.wasm$/iu],
]);
const FORBIDDEN_CONTENT_PATTERNS = Object.freeze([
  ["QuickJS runtime", /\bquickjs(?:-ng|-wasm)?\b|\blibquickjs\b|\bqjs[_-]|\bJS_(?:NewRuntime|NewContext|Eval|ExecutePendingJob|SetInterruptHandler)\b/iu],
  ["Lua runtime", /\bliblua(?:jit|5[1-4])?\b|\bluajit\b|\bmlua\b|\blua[-_ ]limits\b|\bLua 5\.[1-4]\b|\bluaL?_(?:newstate|openlibs|loadbufferx?|loadfilex?|pcallk?|callk?|resume|sethook)\b/iu],
  ["script-runner runtime", /\blorepia[-_ ]script[-_ ]runner\b|\bscript[-_ ]runner[-_ ]spike\b/iu],
  ["Web Worker executor", /\b(?:new\s+)?(?:Shared)?Worker\s*\(/u],
  ["WebAssembly executor", /\bWebAssembly\s*\.(?:compile|instantiate|Module)\b/u],
]);

const THIN_MACHO_MAGICS = new Set([
  "cefaedfe",
  "cffaedfe",
  "feedface",
  "feedfacf",
]);
const FAT_MACHO_MAGICS = new Map([
  ["cafebabe", { entryBytes: 20, littleEndian: false }],
  ["cafebabf", { entryBytes: 32, littleEndian: false }],
  ["bebafeca", { entryBytes: 20, littleEndian: true }],
  ["bfbafeca", { entryBytes: 32, littleEndian: true }],
]);

export class NotRunError extends Error {
  constructor(message) {
    super(message);
    this.name = "NotRunError";
    this.code = "GO_011_NOT_RUN";
  }
}

function defaultCommandRunner(command, args) {
  return spawnSync(command, args, {
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
    windowsHide: true,
  });
}

function commandName(command) {
  return basename(command).toLowerCase().replace(/\.exe$/u, "");
}

function inspectorCandidates(format, environment) {
  if (format === "Mach-O") {
    return [environment.LOREPIA_MACHO_INSPECTOR ?? "otool"];
  }
  if (format === "ELF") {
    if (environment.LOREPIA_ELF_INSPECTOR) return [environment.LOREPIA_ELF_INSPECTOR];
    return ["readelf", "llvm-readobj", "objdump"];
  }
  if (format === "PE") {
    if (environment.LOREPIA_PE_INSPECTOR) return [environment.LOREPIA_PE_INSPECTOR];
    return ["llvm-readobj", "dumpbin", "objdump"];
  }
  throw new Error(`UNSUPPORTED_NATIVE_FORMAT:${format}`);
}

function inspectorArguments(command, format, artifactPath) {
  const name = commandName(command);
  if (format === "Mach-O" && name === "otool") return ["-hvL", artifactPath];
  if (format === "ELF" && name === "readelf") {
    return ["--file-header", "--dynamic", "--symbols", "--wide", artifactPath];
  }
  if (format === "ELF" && name === "llvm-readobj") {
    return ["--file-headers", "--dynamic-table", "--symbols", artifactPath];
  }
  if (format === "ELF" && name === "objdump") return ["-x", "-t", artifactPath];
  if (format === "PE" && name === "llvm-readobj") {
    return ["--file-headers", "--coff-imports", "--symbols", artifactPath];
  }
  if (format === "PE" && name === "dumpbin") {
    return ["/nologo", "/headers", "/imports", "/symbols", artifactPath];
  }
  if (format === "PE" && name === "objdump") return ["-x", "-t", artifactPath];
  throw new NotRunError(
    `NATIVE_INSPECTOR_OVERRIDE_UNRECOGNIZED:${format}:${command}`,
  );
}

function isMissingCommand(result) {
  return result?.error?.code === "ENOENT" || result?.error?.code === "UNKNOWN";
}

function runNativeInspector(
  format,
  artifactPath,
  { commandRunner = defaultCommandRunner, environment = process.env } = {},
) {
  const attempted = [];
  for (const command of inspectorCandidates(format, environment)) {
    attempted.push(command);
    let args;
    try {
      args = inspectorArguments(command, format, artifactPath);
    } catch (error) {
      if (error instanceof NotRunError) throw error;
      throw error;
    }
    const result = commandRunner(command, args);
    if (isMissingCommand(result)) continue;
    if (result?.error) {
      throw new Error(
        `NATIVE_INSPECTOR_EXECUTION_ERROR:${format}:${command}:${result.error.message}`,
      );
    }
    if (result?.status !== 0) {
      const diagnostic = `${result?.stderr ?? ""}`.trim().slice(0, 1024);
      throw new Error(
        `NATIVE_INSPECTION_FAILED:${format}:${command}:exit=${result?.status}:${diagnostic}`,
      );
    }
    const output = `${result?.stdout ?? ""}\n${result?.stderr ?? ""}`;
    if (output.trim().length === 0) {
      throw new Error(`NATIVE_INSPECTION_EMPTY:${format}:${command}`);
    }
    return { command, output };
  }
  throw new NotRunError(
    `NATIVE_INSPECTOR_UNAVAILABLE:${format}:tried=${attempted.join(",")}`,
  );
}

function detectPe(buffer) {
  if (buffer.length < 0x44 || buffer[0] !== 0x4d || buffer[1] !== 0x5a) return false;
  const peOffset = buffer.readUInt32LE(0x3c);
  return (
    peOffset <= buffer.length - 4 &&
    buffer[peOffset] === 0x50 &&
    buffer[peOffset + 1] === 0x45 &&
    buffer[peOffset + 2] === 0 &&
    buffer[peOffset + 3] === 0
  );
}

export function detectNativeFormat(buffer) {
  if (!Buffer.isBuffer(buffer) || buffer.length < 4) return null;
  if (buffer.subarray(0, 4).equals(Buffer.from([0x7f, 0x45, 0x4c, 0x46]))) {
    return "ELF";
  }
  const magic = buffer.subarray(0, 4).toString("hex");
  if (THIN_MACHO_MAGICS.has(magic)) return "Mach-O";
  const fat = FAT_MACHO_MAGICS.get(magic);
  if (fat && buffer.length >= 8) {
    const read32 = fat.littleEndian
      ? (offset) => buffer.readUInt32LE(offset)
      : (offset) => buffer.readUInt32BE(offset);
    const read64 = fat.littleEndian
      ? (offset) => buffer.readBigUInt64LE(offset)
      : (offset) => buffer.readBigUInt64BE(offset);
    const architectureCount = read32(4);
    const tableEnd = 8 + architectureCount * fat.entryBytes;
    if (architectureCount > 0 && architectureCount <= 64 && tableEnd <= buffer.length) {
      let valid = true;
      for (let index = 0; index < architectureCount; index += 1) {
        const offset = 8 + index * fat.entryBytes;
        const sliceOffsetValue = fat.entryBytes === 20
          ? BigInt(read32(offset + 8))
          : read64(offset + 8);
        const sliceBytesValue = fat.entryBytes === 20
          ? BigInt(read32(offset + 12))
          : read64(offset + 16);
        const alignment = read32(offset + (fat.entryBytes === 20 ? 16 : 24));
        if (
          sliceOffsetValue > BigInt(Number.MAX_SAFE_INTEGER) ||
          sliceBytesValue > BigInt(Number.MAX_SAFE_INTEGER)
        ) {
          valid = false;
          break;
        }
        const sliceOffset = Number(sliceOffsetValue);
        const sliceBytes = Number(sliceBytesValue);
        if (
          sliceBytes === 0 ||
          sliceOffset < tableEnd ||
          sliceOffset > buffer.length - sliceBytes ||
          alignment > 31
        ) {
          valid = false;
          break;
        }
      }
      if (valid) return "Mach-O";
    }
  }
  if (detectPe(buffer)) return "PE";
  return null;
}

function assertSafePath(path, label = path) {
  const normalized = path.replaceAll("\\", "/");
  const parts = normalized.split("/");
  if (
    normalized.length === 0 ||
    normalized.startsWith("/") ||
    path.includes("\\") ||
    /^[a-z]:/iu.test(normalized) ||
    normalized.includes("\0") ||
    parts.some((part) => part.length === 0 || part === "." || part === "..")
  ) {
    throw new Error(`UNSAFE_ARTIFACT_PATH:${label}`);
  }
  for (const [description, pattern] of FORBIDDEN_PATH_PATTERNS) {
    if (pattern.test(normalized)) {
      throw new Error(`FORBIDDEN_ARTIFACT_PATH:${description}:${label}`);
    }
  }
}

function findForbiddenText(text) {
  for (const [description, pattern] of FORBIDDEN_CONTENT_PATTERNS) {
    if (pattern.test(text)) return description;
  }
  return null;
}

function asciiSearchText(buffer) {
  return buffer.toString("latin1").replace(/[^\x20-\x7e\n\r\t]/gu, " ");
}

function utf16SearchText(buffer, start) {
  const characters = [];
  for (let index = start; index + 1 < buffer.length; index += 2) {
    const character = buffer[index];
    const high = buffer[index + 1];
    characters.push(high === 0 && character >= 0x20 && character <= 0x7e ? String.fromCharCode(character) : " ");
  }
  return characters.join("");
}

export function assertNoForbiddenRuntimeBytes(buffer, label) {
  if (buffer.indexOf(WASM_V1_HEADER) !== -1) {
    throw new Error(`FORBIDDEN_WASM_MAGIC:${label}`);
  }
  const step = SEARCH_CHUNK_BYTES - SEARCH_OVERLAP_BYTES;
  for (let offset = 0; offset < buffer.length; offset += step) {
    const chunk = buffer.subarray(offset, Math.min(buffer.length, offset + SEARCH_CHUNK_BYTES));
    const candidates = [
      asciiSearchText(chunk),
      utf16SearchText(chunk, 0),
      utf16SearchText(chunk, 1),
    ];
    for (const text of candidates) {
      const description = findForbiddenText(text);
      if (description) {
        throw new Error(`FORBIDDEN_RUNTIME_SURFACE:${description}:${label}`);
      }
    }
  }
}

function inspectNative(
  artifactPath,
  buffer,
  label,
  options,
) {
  const format = detectNativeFormat(buffer);
  if (!format) throw new Error(`NATIVE_FORMAT_NOT_RECOGNIZED:${label}`);
  const inspection = runNativeInspector(format, artifactPath, options);
  const forbiddenInspectionText = findForbiddenText(inspection.output);
  if (forbiddenInspectionText) {
    throw new Error(
      `FORBIDDEN_NATIVE_DEPENDENCY_OR_SYMBOL:${forbiddenInspectionText}:${label}`,
    );
  }
  assertNoForbiddenRuntimeBytes(buffer, label);
  return { format, inspector: commandName(inspection.command) };
}

function crc32(buffer) {
  let crc = 0xffffffff;
  for (const byte of buffer) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit += 1) {
      crc = (crc >>> 1) ^ (crc & 1 ? 0xedb88320 : 0);
    }
  }
  return (crc ^ 0xffffffff) >>> 0;
}

function findEndOfCentralDirectory(archive) {
  const minimum = Math.max(0, archive.length - 65_557);
  for (let offset = archive.length - 22; offset >= minimum; offset -= 1) {
    if (archive.readUInt32LE(offset) === 0x06054b50) return offset;
  }
  throw new Error("ZIP_END_OF_CENTRAL_DIRECTORY_MISSING");
}

function decodeEntryName(bytes, utf8) {
  if (!utf8) return bytes.toString("latin1");
  try {
    return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch {
    throw new Error("ZIP_ENTRY_NAME_INVALID_UTF8");
  }
}

function archiveEntries(archive) {
  if (archive.length < 22) throw new Error("ZIP_TOO_SMALL");
  const eocd = findEndOfCentralDirectory(archive);
  const disk = archive.readUInt16LE(eocd + 4);
  const centralDisk = archive.readUInt16LE(eocd + 6);
  const entriesOnDisk = archive.readUInt16LE(eocd + 8);
  const entryCount = archive.readUInt16LE(eocd + 10);
  const centralBytes = archive.readUInt32LE(eocd + 12);
  const centralOffset = archive.readUInt32LE(eocd + 16);
  const commentBytes = archive.readUInt16LE(eocd + 20);
  if (eocd + 22 + commentBytes !== archive.length) throw new Error("ZIP_TRAILING_DATA_OR_BAD_COMMENT");
  if (disk !== 0 || centralDisk !== 0 || entriesOnDisk !== entryCount) {
    throw new Error("ZIP_MULTIDISK_NOT_SUPPORTED");
  }
  if (entryCount === 0xffff || centralBytes === 0xffffffff || centralOffset === 0xffffffff) {
    throw new Error("ZIP64_NOT_SUPPORTED");
  }
  if (entryCount === 0 || entryCount > MAX_ARCHIVE_ENTRIES) throw new Error("ZIP_ENTRY_COUNT_INVALID");
  if (centralOffset + centralBytes !== eocd) throw new Error("ZIP_CENTRAL_DIRECTORY_BOUNDS_INVALID");

  const entries = [];
  const names = new Set();
  let expandedBytes = 0;
  let cursor = centralOffset;
  for (let index = 0; index < entryCount; index += 1) {
    if (cursor + 46 > eocd || archive.readUInt32LE(cursor) !== 0x02014b50) {
      throw new Error("ZIP_CENTRAL_ENTRY_INVALID");
    }
    const flags = archive.readUInt16LE(cursor + 8);
    const method = archive.readUInt16LE(cursor + 10);
    const expectedCrc = archive.readUInt32LE(cursor + 16);
    const compressedBytes = archive.readUInt32LE(cursor + 20);
    const uncompressedBytes = archive.readUInt32LE(cursor + 24);
    const nameBytes = archive.readUInt16LE(cursor + 28);
    const extraBytes = archive.readUInt16LE(cursor + 30);
    const entryCommentBytes = archive.readUInt16LE(cursor + 32);
    const externalAttributes = archive.readUInt32LE(cursor + 38);
    const localOffset = archive.readUInt32LE(cursor + 42);
    const end = cursor + 46 + nameBytes + extraBytes + entryCommentBytes;
    if (end > eocd) throw new Error("ZIP_CENTRAL_ENTRY_TRUNCATED");
    const name = decodeEntryName(archive.subarray(cursor + 46, cursor + 46 + nameBytes), Boolean(flags & 0x800));
    assertSafePath(name.replace(/\/$/u, ""), name);
    if (names.has(name)) throw new Error(`ZIP_DUPLICATE_ENTRY:${name}`);
    names.add(name);
    if (flags & 0x1) throw new Error(`ZIP_ENCRYPTED_ENTRY:${name}`);
    if (method !== 0 && method !== 8) throw new Error(`ZIP_COMPRESSION_METHOD_UNSUPPORTED:${name}:${method}`);
    if (compressedBytes === 0xffffffff || uncompressedBytes === 0xffffffff || localOffset === 0xffffffff) {
      throw new Error(`ZIP64_ENTRY_NOT_SUPPORTED:${name}`);
    }
    if (uncompressedBytes > MAX_ARCHIVE_ENTRY_BYTES) throw new Error(`ZIP_ENTRY_TOO_LARGE:${name}`);
    expandedBytes += uncompressedBytes;
    if (expandedBytes > MAX_ARCHIVE_EXPANDED_BYTES) throw new Error("ZIP_EXPANDED_SIZE_LIMIT");
    const unixMode = externalAttributes >>> 16;
    if ((unixMode & 0xf000) === 0xa000) throw new Error(`ZIP_SYMLINK_NOT_ALLOWED:${name}`);
    entries.push({ compressedBytes, expectedCrc, flags, localOffset, method, name, uncompressedBytes });
    cursor = end;
  }
  if (cursor !== eocd) throw new Error("ZIP_CENTRAL_DIRECTORY_SIZE_MISMATCH");
  return entries;
}

function extractArchiveEntry(archive, entry) {
  const offset = entry.localOffset;
  if (offset + 30 > archive.length || archive.readUInt32LE(offset) !== 0x04034b50) {
    throw new Error(`ZIP_LOCAL_HEADER_INVALID:${entry.name}`);
  }
  const localFlags = archive.readUInt16LE(offset + 6);
  const localMethod = archive.readUInt16LE(offset + 8);
  const nameBytes = archive.readUInt16LE(offset + 26);
  const extraBytes = archive.readUInt16LE(offset + 28);
  if (localFlags !== entry.flags || localMethod !== entry.method) {
    throw new Error(`ZIP_LOCAL_HEADER_MISMATCH:${entry.name}`);
  }
  const localName = decodeEntryName(archive.subarray(offset + 30, offset + 30 + nameBytes), Boolean(localFlags & 0x800));
  if (localName !== entry.name) throw new Error(`ZIP_LOCAL_NAME_MISMATCH:${entry.name}`);
  const dataOffset = offset + 30 + nameBytes + extraBytes;
  const dataEnd = dataOffset + entry.compressedBytes;
  if (dataEnd > archive.length) throw new Error(`ZIP_ENTRY_DATA_TRUNCATED:${entry.name}`);
  const compressed = archive.subarray(dataOffset, dataEnd);
  let content;
  try {
    content = entry.method === 0
      ? Buffer.from(compressed)
      : inflateRawSync(compressed, { maxOutputLength: entry.uncompressedBytes + 1 });
  } catch (error) {
    throw new Error(`ZIP_ENTRY_DECOMPRESSION_FAILED:${entry.name}:${error.message}`);
  }
  if (content.length !== entry.uncompressedBytes) throw new Error(`ZIP_ENTRY_SIZE_MISMATCH:${entry.name}`);
  if (crc32(content) !== entry.expectedCrc) throw new Error(`ZIP_ENTRY_CRC_MISMATCH:${entry.name}`);
  return content;
}

function readBoundedFile(path) {
  const metadata = statSync(path);
  if (!metadata.isFile()) throw new Error(`ARTIFACT_NOT_REGULAR_FILE:${path}`);
  if (metadata.size === 0 || metadata.size > MAX_ARTIFACT_BYTES) {
    throw new Error(`ARTIFACT_SIZE_INVALID:${path}:${metadata.size}`);
  }
  return readFileSync(path);
}

function verifyArchive(path, options) {
  const extension = extname(path).toLowerCase();
  if (extension !== ".apk" && extension !== ".aab") {
    throw new Error(`ANDROID_ARCHIVE_EXTENSION_REQUIRED:${path}`);
  }
  const archive = readBoundedFile(path);
  const entries = archiveEntries(archive);
  const temporary = mkdtempSync(join(tmpdir(), "lorepia-go011-native-"));
  let nativeFiles = 0;
  let resources = 0;
  const formats = new Set();
  const inspectors = new Set();
  try {
    for (const [index, entry] of entries.entries()) {
      if (entry.name.endsWith("/")) continue;
      assertSafePath(entry.name);
      const content = extractArchiveEntry(archive, entry);
      const format = detectNativeFormat(content);
      const nativeExtension = /\.(?:so|dll|dylib)$/iu.test(entry.name);
      if (nativeExtension && !format) throw new Error(`NATIVE_FORMAT_NOT_RECOGNIZED:${entry.name}`);
      if (format) {
        const extracted = join(temporary, `native-${index}${extname(entry.name) || ".bin"}`);
        writeFileSync(extracted, content, { mode: 0o600 });
        const result = inspectNative(extracted, content, entry.name, options);
        formats.add(result.format);
        inspectors.add(result.inspector);
        nativeFiles += 1;
      } else {
        assertNoForbiddenRuntimeBytes(content, entry.name);
        resources += 1;
      }
    }
  } finally {
    rmSync(temporary, { force: true, recursive: true });
  }
  if (nativeFiles === 0) throw new Error(`NO_NATIVE_BINARY_FOUND:${path}`);
  return { artifacts: 1, formats: [...formats].sort(), inspectors: [...inspectors].sort(), nativeFiles, resources };
}

function listDirectoryFiles(root) {
  const files = [];
  const pending = [root];
  while (pending.length > 0) {
    const directory = pending.pop();
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const path = join(directory, entry.name);
      if (entry.isSymbolicLink()) throw new Error(`ARTIFACT_SYMLINK_NOT_ALLOWED:${relative(root, path)}`);
      if (entry.isDirectory()) pending.push(path);
      else if (entry.isFile()) files.push(path);
      else throw new Error(`ARTIFACT_SPECIAL_FILE_NOT_ALLOWED:${relative(root, path)}`);
    }
  }
  return files.sort((left, right) => left.localeCompare(right));
}

function verifyDirectory(root, options) {
  const files = listDirectoryFiles(root);
  if (files.length === 0) throw new Error(`ARTIFACT_DIRECTORY_EMPTY:${root}`);
  let nativeFiles = 0;
  let resources = 0;
  const formats = new Set();
  const inspectors = new Set();
  for (const path of files) {
    const label = relative(root, path).replaceAll("\\", "/");
    assertSafePath(label);
    const content = readBoundedFile(path);
    const format = detectNativeFormat(content);
    const nativeExtension = /\.(?:so|dll|dylib|exe)$/iu.test(label);
    if (nativeExtension && !format) throw new Error(`NATIVE_FORMAT_NOT_RECOGNIZED:${label}`);
    if (format) {
      const result = inspectNative(path, content, label, options);
      formats.add(result.format);
      inspectors.add(result.inspector);
      nativeFiles += 1;
    } else {
      assertNoForbiddenRuntimeBytes(content, label);
      resources += 1;
    }
  }
  if (nativeFiles === 0) throw new Error(`NO_NATIVE_BINARY_FOUND:${root}`);
  return { artifacts: 1, formats: [...formats].sort(), inspectors: [...inspectors].sort(), nativeFiles, resources };
}

export function verifyReleaseArtifactBoundary(path, options = {}) {
  const artifactPath = resolve(path);
  if (!existsSync(artifactPath)) throw new Error(`ARTIFACT_NOT_FOUND:${artifactPath}`);
  const metadata = lstatSync(artifactPath);
  if (metadata.isSymbolicLink()) throw new Error(`ARTIFACT_SYMLINK_NOT_ALLOWED:${artifactPath}`);
  if (metadata.isDirectory()) return verifyDirectory(artifactPath, options);
  if (!metadata.isFile()) throw new Error(`ARTIFACT_NOT_REGULAR_FILE:${artifactPath}`);
  if ([".apk", ".aab"].includes(extname(artifactPath).toLowerCase())) {
    return verifyArchive(artifactPath, options);
  }
  assertSafePath(basename(artifactPath));
  const content = readBoundedFile(artifactPath);
  const result = inspectNative(artifactPath, content, basename(artifactPath), options);
  return { artifacts: 1, formats: [result.format], inspectors: [result.inspector], nativeFiles: 1, resources: 0 };
}

export function verifyReleaseArtifacts(paths, options = {}) {
  if (!Array.isArray(paths) || paths.length === 0) throw new Error("USAGE:verify-release-artifact-boundary <artifact> [...]");
  const totals = { artifacts: 0, formats: new Set(), inspectors: new Set(), nativeFiles: 0, resources: 0 };
  for (const path of paths) {
    const result = verifyReleaseArtifactBoundary(path, options);
    totals.artifacts += result.artifacts;
    totals.nativeFiles += result.nativeFiles;
    totals.resources += result.resources;
    for (const format of result.formats) totals.formats.add(format);
    for (const inspector of result.inspectors) totals.inspectors.add(inspector);
  }
  return {
    artifacts: totals.artifacts,
    formats: [...totals.formats].sort(),
    inspectors: [...totals.inspectors].sort(),
    nativeFiles: totals.nativeFiles,
    resources: totals.resources,
  };
}

const invokedPath = process.argv[1] ? pathToFileURL(resolve(process.argv[1])).href : "";
if (import.meta.url === invokedPath) {
  try {
    const result = verifyReleaseArtifacts(process.argv.slice(2));
    process.stdout.write(
      `GO_011_PASS artifacts=${result.artifacts} native=${result.nativeFiles} resources=${result.resources} formats=${result.formats.join(",")} inspectors=${result.inspectors.join(",")}\n`,
    );
  } catch (error) {
    const notRun = error instanceof NotRunError;
    process.stderr.write(`${notRun ? "GO_011_NOT_RUN" : "GO_011_FAIL"}:${error.message}\n`);
    process.exitCode = notRun ? 2 : 1;
  }
}
