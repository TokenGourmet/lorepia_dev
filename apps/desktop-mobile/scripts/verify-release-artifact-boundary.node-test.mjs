import assert from "node:assert/strict";
import {
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import test, { afterEach } from "node:test";
import { fileURLToPath } from "node:url";
import { deflateRawSync } from "node:zlib";

import {
  NotRunError,
  detectNativeFormat,
  verifyReleaseArtifactBoundary,
} from "./verify-release-artifact-boundary.mjs";

const scriptDirectory = resolve(fileURLToPath(new URL(".", import.meta.url)));
const fixtures = JSON.parse(
  readFileSync(
    resolve(scriptDirectory, "fixtures/release-artifact-boundary/native-fixtures.json"),
    "utf8",
  ),
);
const temporaryDirectories = [];

function fixture(name) {
  return Buffer.from(fixtures[name], "hex");
}

function makeTemporaryDirectory() {
  const directory = mkdtempSync(join(tmpdir(), "lorepia-go011-test-"));
  temporaryDirectories.push(directory);
  return directory;
}

function writeFixture(root, path, content) {
  const target = join(root, path);
  mkdirSync(dirname(target), { recursive: true });
  writeFileSync(target, content);
  return target;
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

function storedZip(entries) {
  const locals = [];
  const central = [];
  let offset = 0;
  for (const [name, value] of Object.entries(entries)) {
    const nameBuffer = Buffer.from(name, "utf8");
    const content = Buffer.isBuffer(value) ? value : Buffer.from(value);
    const method = /\.(?:html|js)$/u.test(name) ? 8 : 0;
    const compressed = method === 8 ? deflateRawSync(content) : content;
    const checksum = crc32(content);
    const local = Buffer.alloc(30 + nameBuffer.length + compressed.length);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(0x800, 6);
    local.writeUInt16LE(method, 8);
    local.writeUInt32LE(checksum, 14);
    local.writeUInt32LE(compressed.length, 18);
    local.writeUInt32LE(content.length, 22);
    local.writeUInt16LE(nameBuffer.length, 26);
    nameBuffer.copy(local, 30);
    compressed.copy(local, 30 + nameBuffer.length);

    const directory = Buffer.alloc(46 + nameBuffer.length);
    directory.writeUInt32LE(0x02014b50, 0);
    directory.writeUInt16LE(0x031e, 4);
    directory.writeUInt16LE(0x800, 8);
    directory.writeUInt16LE(method, 10);
    directory.writeUInt32LE(checksum, 16);
    directory.writeUInt32LE(compressed.length, 20);
    directory.writeUInt32LE(content.length, 24);
    directory.writeUInt16LE(nameBuffer.length, 28);
    directory.writeUInt32LE((0o100644 << 16) >>> 0, 38);
    directory.writeUInt32LE(offset, 42);
    nameBuffer.copy(directory, 46);

    locals.push(local);
    central.push(directory);
    offset += local.length;
  }
  const centralBytes = central.reduce((total, entry) => total + entry.length, 0);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(central.length, 8);
  end.writeUInt16LE(central.length, 10);
  end.writeUInt32LE(centralBytes, 12);
  end.writeUInt32LE(offset, 16);
  return Buffer.concat([...locals, ...central, end]);
}

function successfulInspector(command, args) {
  return {
    status: 0,
    stdout: `validated by ${command} ${args.at(-1)}; no forbidden dependency`,
    stderr: "",
  };
}

afterEach(() => {
  for (const directory of temporaryDirectories.splice(0)) {
    rmSync(directory, { force: true, recursive: true });
  }
});

test("fixture headers distinguish actual ELF, Mach-O, and PE inputs", () => {
  assert.equal(detectNativeFormat(fixture("elfClean")), "ELF");
  assert.equal(detectNativeFormat(fixture("machoClean")), "Mach-O");
  assert.equal(detectNativeFormat(fixture("peClean")), "PE");
  assert.equal(detectNativeFormat(Buffer.from("not a binary")), null);

  const javaClass = Buffer.alloc(2_048);
  javaClass.writeUInt32BE(0xcafebabe, 0);
  javaClass.writeUInt16BE(0, 4);
  javaClass.writeUInt16BE(52, 6);
  assert.equal(detectNativeFormat(javaClass), null);
});

test("accepts clean native fixtures only after their platform inspector succeeds", () => {
  const root = makeTemporaryDirectory();
  const cases = [
    ["lorepia-app", "elfClean", "ELF", "readelf"],
    ["LorePia", "machoClean", "Mach-O", "otool"],
    ["lorepia-app.exe", "peClean", "PE", "llvm-readobj"],
  ];
  for (const [name, fixtureName, format, inspector] of cases) {
    const path = writeFixture(root, name, fixture(fixtureName));
    assert.deepEqual(
      verifyReleaseArtifactBoundary(path, { commandRunner: successfulInspector }),
      { artifacts: 1, formats: [format], inspectors: [inspector], nativeFiles: 1, resources: 0 },
    );
  }
});

test("rejects forbidden QuickJS and Lua bytes even when symbols are stripped from inspector output", () => {
  const root = makeTemporaryDirectory();
  const quickjs = writeFixture(root, "quick-host", fixture("elfQuickjs"));
  const lua = writeFixture(root, "host.exe", fixture("peLua"));
  assert.throws(
    () => verifyReleaseArtifactBoundary(quickjs, { commandRunner: successfulInspector }),
    /FORBIDDEN_RUNTIME_SURFACE:QuickJS runtime/u,
  );
  assert.throws(
    () => verifyReleaseArtifactBoundary(lua, { commandRunner: successfulInspector }),
    /FORBIDDEN_RUNTIME_SURFACE:Lua runtime/u,
  );
});

test("distinguishes an incidental wasm magic substring from a complete embedded module", () => {
  const root = makeTemporaryDirectory();
  const incidental = writeFixture(
    root,
    "incidental-native",
    Buffer.concat([
      fixture("elfClean"),
      Buffer.from([0x00, 0x61, 0x73, 0x6d, 0x7f, 0x45, 0x4c, 0x46]),
    ]),
  );
  assert.deepEqual(
    verifyReleaseArtifactBoundary(incidental, { commandRunner: successfulInspector }),
    { artifacts: 1, formats: ["ELF"], inspectors: ["readelf"], nativeFiles: 1, resources: 0 },
  );

  const embeddedModule = writeFixture(
    root,
    "embedded-module-native",
    Buffer.concat([
      fixture("elfClean"),
      Buffer.from([0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00]),
    ]),
  );
  assert.throws(
    () => verifyReleaseArtifactBoundary(embeddedModule, { commandRunner: successfulInspector }),
    /FORBIDDEN_WASM_MAGIC:embedded-module-native/u,
  );
});

test("rejects a forbidden linked dependency reported only by the native inspector", () => {
  const root = makeTemporaryDirectory();
  const path = writeFixture(root, "LorePia", fixture("machoClean"));
  assert.throws(
    () => verifyReleaseArtifactBoundary(path, {
      commandRunner: () => ({ status: 0, stdout: "/Frameworks/libquickjs.dylib", stderr: "" }),
    }),
    /FORBIDDEN_NATIVE_DEPENDENCY_OR_SYMBOL:QuickJS runtime/u,
  );
});

test("reports NOT RUN instead of passing when the required native inspector is unavailable", () => {
  const root = makeTemporaryDirectory();
  const path = writeFixture(root, "lorepia-app", fixture("elfClean"));
  assert.throws(
    () => verifyReleaseArtifactBoundary(path, {
      commandRunner: () => ({ error: Object.assign(new Error("missing"), { code: "ENOENT" }) }),
    }),
    (error) => error instanceof NotRunError && /NATIVE_INSPECTOR_UNAVAILABLE:ELF/u.test(error.message),
  );
});

test("scans a macOS app native executable and every packaged resource", () => {
  const root = makeTemporaryDirectory();
  const app = join(root, "LorePia.app");
  writeFixture(app, "Contents/MacOS/LorePia", fixture("machoClean"));
  writeFixture(app, "Contents/Resources/index.html", "<main>LorePia</main>");
  assert.deepEqual(
    verifyReleaseArtifactBoundary(app, { commandRunner: successfulInspector }),
    { artifacts: 1, formats: ["Mach-O"], inspectors: ["otool"], nativeFiles: 1, resources: 1 },
  );

  writeFixture(app, "Contents/Resources/runtime.wasm", Buffer.from([0, 0x61, 0x73, 0x6d]));
  assert.throws(
    () => verifyReleaseArtifactBoundary(app, { commandRunner: successfulInspector }),
    /FORBIDDEN_ARTIFACT_PATH:WebAssembly artifact/u,
  );
});

test("opens APK native libraries and resources instead of trusting the archive filename", () => {
  const root = makeTemporaryDirectory();
  const apk = writeFixture(
    root,
    "lorepia.apk",
    storedZip({
      "lib/arm64-v8a/liblorepia_app.so": fixture("elfClean"),
      "assets/index.html": "<main>LorePia</main>",
      "classes.dex": "dex fixture without an executor",
    }),
  );
  assert.deepEqual(
    verifyReleaseArtifactBoundary(apk, { commandRunner: successfulInspector }),
    { artifacts: 1, formats: ["ELF"], inspectors: ["readelf"], nativeFiles: 1, resources: 2 },
  );
});

test("rejects QuickJS in an APK native library and Lua or script-runner AAB resources", () => {
  const root = makeTemporaryDirectory();
  const apk = writeFixture(
    root,
    "bad.apk",
    storedZip({ "lib/arm64-v8a/liblorepia_app.so": fixture("elfQuickjs") }),
  );
  assert.throws(
    () => verifyReleaseArtifactBoundary(apk, { commandRunner: successfulInspector }),
    /FORBIDDEN_RUNTIME_SURFACE:QuickJS runtime/u,
  );

  const aab = writeFixture(
    root,
    "bad.aab",
    storedZip({
      "base/lib/arm64-v8a/liblorepia_app.so": fixture("elfClean"),
      "base/assets/plugin.lua": "return 1",
    }),
  );
  assert.throws(
    () => verifyReleaseArtifactBoundary(aab, { commandRunner: successfulInspector }),
    /FORBIDDEN_ARTIFACT_PATH:Lua artifact/u,
  );

  const runner = writeFixture(
    root,
    "runner.aab",
    storedZip({
      "base/lib/arm64-v8a/liblorepia_app.so": fixture("elfClean"),
      "base/assets/app.js": "const packageName = 'lorepia-script-runner-spike';",
    }),
  );
  assert.throws(
    () => verifyReleaseArtifactBoundary(runner, { commandRunner: successfulInspector }),
    /FORBIDDEN_RUNTIME_SURFACE:script-runner runtime/u,
  );
});

test("fails closed for native-looking archive entries that are not real native formats", () => {
  const root = makeTemporaryDirectory();
  const apk = writeFixture(
    root,
    "opaque.apk",
    storedZip({ "lib/arm64-v8a/liblorepia_app.so": "opaque bytes" }),
  );
  assert.throws(
    () => verifyReleaseArtifactBoundary(apk, { commandRunner: successfulInspector }),
    /NATIVE_FORMAT_NOT_RECOGNIZED/u,
  );
});
