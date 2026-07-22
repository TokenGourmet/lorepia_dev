# Product security verifiers

## GO-011 final-artifact boundary

`verify-release-artifact-boundary.mjs` verifies the Store-Safe product artifact
itself. It complements, rather than replaces, the Cargo dependency-graph and
frontend-output verifiers.

The verifier accepts one or more of these inputs:

- a native ELF, Mach-O, or PE executable/library;
- a macOS/iOS `.app` directory; or
- an Android APK or AAB ZIP archive.

For every native binary it validates the file magic, requires a successful
format-aware platform inspection, and scans the resulting headers, imported
libraries, symbols, and embedded printable bytes for QuickJS, Lua, and the
disposable script-runner. Every app/package resource is also decompressed and
scanned. `.lua`, `.luac`, and `.wasm` resources, WASM magic, native-looking
files with an invalid binary format, unsafe archive paths, symlinks, encrypted
entries, and unsupported ZIP structures fail closed.

Run it after producing an artifact:

```sh
npm run verify:release-artifact-boundary -- ../../target/release/lorepia-app
npm run verify:release-artifact-boundary -- path/to/LorePia.app
npm run verify:release-artifact-boundary -- path/to/lorepia.apk
npm run verify:release-artifact-boundary -- path/to/lorepia.aab
```

Required inspectors are `otool` for Mach-O, one of `readelf`, `llvm-readobj`,
or `objdump` for ELF, and one of `llvm-readobj`, `dumpbin`, or `objdump` for
PE. An inspector can be pinned with
`LOREPIA_MACHO_INSPECTOR`, `LOREPIA_ELF_INSPECTOR`, or
`LOREPIA_PE_INSPECTOR`; an override must still name the matching supported
tool. If no required inspector exists, the command emits `GO_011_NOT_RUN` and
exits 2. It never turns a missing tool into PASS. A detected forbidden surface
or malformed/uninspectable artifact emits `GO_011_FAIL` and exits 1. Only a
complete inspection emits `GO_011_PASS` and exits 0.

The product workflow runs this verifier against each final debug compile
artifact on Linux, macOS, Windows, Android, and iOS; the Android job builds and
checks both APK and AAB forms. Those jobs are compile evidence only; they do
not turn an unsigned debug artifact into signed release or store evidence. The
same command must be rerun on each signed release candidate.

The archive reader deliberately rejects multidisk ZIP, ZIP64, encryption,
unknown compression methods, duplicate/unsafe names, entries over 512 MiB,
more than 200,000 entries, and more than 2 GiB expanded content. If a future
legitimate product artifact requires one of those structures, extend the
parser and fixtures before accepting it; do not bypass this gate.
