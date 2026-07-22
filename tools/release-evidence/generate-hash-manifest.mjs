import { createHash } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { lstat, readFile, readdir, realpath, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

async function hashFile(file) {
  const hash = createHash('sha256');
  for await (const chunk of createReadStream(file)) hash.update(chunk);
  return hash.digest('hex');
}

async function collect(base, target, output) {
  const info = await lstat(target);
  if (info.isSymbolicLink()) throw new Error(`SYMLINK_NOT_ALLOWED:${target}`);
  if (info.isDirectory()) {
    const entries = await readdir(target);
    entries.sort();
    for (const entry of entries) await collect(base, path.join(target, entry), output);
    return;
  }
  if (!info.isFile()) throw new Error(`UNSUPPORTED_FILE_TYPE:${target}`);
  const relative = path.relative(base, target).replaceAll(path.sep, '/');
  if (relative.startsWith('../') || path.isAbsolute(relative)) {
    throw new Error(`PATH_OUTSIDE_BASE:${target}`);
  }
  output.push({
    path: relative,
    size: info.size,
    sha256: await hashFile(target),
  });
}

export async function buildHashManifest({ base, inputs, label, commit }) {
  if (!Array.isArray(inputs) || inputs.length === 0) throw new Error('INPUTS_REQUIRED');
  if (!/^[0-9a-f]{40}$/u.test(commit)) throw new Error('FULL_COMMIT_REQUIRED');
  const canonicalBase = await realpath(base);
  const files = [];
  for (const input of inputs) {
    const resolved = path.resolve(canonicalBase, input);
    if (resolved !== canonicalBase && !resolved.startsWith(`${canonicalBase}${path.sep}`)) {
      throw new Error(`PATH_OUTSIDE_BASE:${input}`);
    }
    const inputInfo = await lstat(resolved);
    if (inputInfo.isSymbolicLink()) throw new Error(`SYMLINK_NOT_ALLOWED:${input}`);
    const canonical = await realpath(resolved);
    if (canonical !== canonicalBase && !canonical.startsWith(`${canonicalBase}${path.sep}`)) {
      throw new Error(`PATH_OUTSIDE_BASE:${input}`);
    }
    if (canonical !== resolved) throw new Error(`SYMLINK_NOT_ALLOWED:${input}`);
    await collect(canonicalBase, resolved, files);
  }
  files.sort((left, right) => left.path.localeCompare(right.path));
  const paths = new Set();
  for (const file of files) {
    if (paths.has(file.path)) throw new Error(`DUPLICATE_INPUT:${file.path}`);
    paths.add(file.path);
  }
  return { schemaVersion: 1, label, commit, files };
}

export function decodeNulFileList(bytes) {
  const text = Buffer.isBuffer(bytes) ? bytes.toString('utf8') : String(bytes);
  const entries = text.split('\0');
  if (entries.at(-1) === '') entries.pop();
  if (entries.length === 0 || entries.some((entry) => entry.length === 0)) {
    throw new Error('EMPTY_OR_MALFORMED_INPUT_LIST');
  }
  return entries;
}

function parseArguments(argv) {
  const options = { inputs: [] };
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === '--base') options.base = argv[++index];
    else if (argument === '--output') options.output = argv[++index];
    else if (argument === '--label') options.label = argv[++index];
    else if (argument === '--commit') options.commit = argv[++index];
    else if (argument === '--input-list0') options.inputList0 = argv[++index];
    else if (argument.startsWith('--')) throw new Error(`UNKNOWN_ARGUMENT:${argument}`);
    else options.inputs.push(argument);
  }
  if (!options.base || !options.output || !options.label || !options.commit) {
    throw new Error('BASE_OUTPUT_LABEL_COMMIT_REQUIRED');
  }
  return options;
}

async function main() {
  const options = parseArguments(process.argv.slice(2));
  if (options.inputList0) {
    options.inputs.push(...decodeNulFileList(await readFile(options.inputList0)));
  }
  const manifest = await buildHashManifest(options);
  await writeFile(options.output, `${JSON.stringify(manifest, null, 2)}\n`, {
    encoding: 'utf8',
    flag: 'wx',
  });
  process.stdout.write(`hashed ${manifest.files.length} files for ${manifest.label}\n`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await main();
}
