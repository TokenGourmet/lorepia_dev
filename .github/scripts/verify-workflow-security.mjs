import { readFile, readdir } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const IMMUTABLE_ACTION = /^[^/@\s]+\/[^/@\s]+(?:\/[^@\s]+)?@([0-9a-f]{40})$/;
const IMMUTABLE_DOCKER_ACTION = /^docker:\/\/[^@\s]+@sha256:[0-9a-f]{64}$/i;
const USES_LINE = /^(\s*)(?:-\s+)?uses:\s*(?:"([^"]+)"|'([^']+)'|([^\s#]+))\s*(?:#.*)?$/;
const FLOW_USES = /^\s*-\s*\{[^}]*(?:["']uses["']|\buses)\s*:/u;
const PERSIST_CREDENTIALS_FALSE = /^\s*persist-credentials:\s*false\s*(?:#.*)?$/m;

export function validateWorkflowText(file, text) {
  const errors = [];
  const lines = text.split(/\r?\n/);
  let externalActionCount = 0;
  let checkoutCount = 0;

  for (let index = 0; index < lines.length; index += 1) {
    if (FLOW_USES.test(lines[index])) {
      errors.push(
        `${file}:${index + 1}: flow-style uses syntax is not accepted by the immutable-action verifier`,
      );
      continue;
    }
    const match = lines[index].match(USES_LINE);
    if (!match) {
      if (/^\s*(?:-\s+)?uses\s*:/u.test(lines[index])) {
        errors.push(
          `${file}:${index + 1}: uses value must be one closed scalar on the same line`,
        );
      }
      continue;
    }

    const [, indentation, doubleQuoted, singleQuoted, plain] = match;
    const action = doubleQuoted ?? singleQuoted ?? plain;
    if (/^[>|][+-]?$/u.test(action)) {
      errors.push(
        `${file}:${index + 1}: uses value must be one closed scalar on the same line`,
      );
      continue;
    }
    if (action.startsWith('./')) continue;

    if (action.startsWith('docker://')) {
      externalActionCount += 1;
      if (!IMMUTABLE_DOCKER_ACTION.test(action)) {
        errors.push(
          `${file}:${index + 1}: Docker action must use an immutable sha256 image digest: ${action}`,
        );
      }
      continue;
    }

    externalActionCount += 1;
    const immutable = action.match(IMMUTABLE_ACTION);
    if (!immutable) {
      errors.push(
        `${file}:${index + 1}: external action must use an immutable 40-character commit SHA: ${action}`,
      );
      continue;
    }

    if (!action.startsWith('actions/checkout@')) continue;
    checkoutCount += 1;

    const usesIndent = indentation.length;
    let end = index + 1;
    while (end < lines.length) {
      const candidate = lines[end];
      if (candidate.trim() === '') {
        end += 1;
        continue;
      }
      const candidateIndent = candidate.match(/^\s*/)[0].length;
      if (candidateIndent <= usesIndent && candidate.trimStart().startsWith('- ')) break;
      end += 1;
    }

    const stepBody = lines.slice(index + 1, end).join('\n');
    if (!PERSIST_CREDENTIALS_FALSE.test(stepBody)) {
      errors.push(
        `${file}:${index + 1}: actions/checkout must set persist-credentials: false`,
      );
    }
  }

  return { errors, externalActionCount, checkoutCount };
}

async function workflowFiles(directory) {
  const entries = await readdir(directory, { withFileTypes: true });
  return entries
    .filter((entry) => entry.isFile() && /\.ya?ml$/u.test(entry.name))
    .map((entry) => path.join(directory, entry.name))
    .sort();
}

export async function validateWorkflowDirectory(directory) {
  const files = await workflowFiles(directory);
  if (files.length === 0) {
    return {
      errors: [`${directory}: no workflow YAML files found`],
      files: 0,
      externalActionCount: 0,
      checkoutCount: 0,
    };
  }

  const aggregate = {
    errors: [],
    files: files.length,
    externalActionCount: 0,
    checkoutCount: 0,
  };
  for (const file of files) {
    const text = await readFile(file, 'utf8');
    const result = validateWorkflowText(file, text);
    aggregate.errors.push(...result.errors);
    aggregate.externalActionCount += result.externalActionCount;
    aggregate.checkoutCount += result.checkoutCount;
  }
  return aggregate;
}

async function main() {
  const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
  const defaultDirectory = path.resolve(scriptDirectory, '..', 'workflows');
  const target = process.argv[2]
    ? path.resolve(process.cwd(), process.argv[2])
    : defaultDirectory;
  const result = await validateWorkflowDirectory(target);

  if (result.errors.length > 0) {
    for (const error of result.errors) console.error(error);
    process.exitCode = 1;
    return;
  }

  console.log(
    `workflow security PASS: ${result.files} files, ${result.externalActionCount} immutable external actions, ${result.checkoutCount} non-persistent checkouts`,
  );
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await main();
}
