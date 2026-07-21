import assert from 'node:assert/strict';
import { mkdir, mkdtemp, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import test from 'node:test';

import { cargoMetadataToCycloneDx } from './cargo-metadata-to-cyclonedx.mjs';
import { buildHashManifest, decodeNulFileList } from './generate-hash-manifest.mjs';
import { verifyDependencyPolicy } from './verify-dependency-policy.mjs';

const COMMIT = '0123456789abcdef0123456789abcdef01234567';

test('converts Cargo metadata into a stable dependency graph', () => {
  const app = { id: 'app 0.1.0', name: 'app', version: '0.1.0', license: 'Apache-2.0' };
  const dep = { id: 'dep 1.2.3', name: 'dep', version: '1.2.3', license: null };
  const result = cargoMetadataToCycloneDx({
    packages: [dep, app],
    workspace_members: [app.id],
    resolve: {
      nodes: [
        { id: app.id, dependencies: [dep.id] },
        { id: dep.id, dependencies: [] },
      ],
    },
  });

  assert.equal(result.bomFormat, 'CycloneDX');
  assert.equal(result.components.length, 2);
  assert.equal(result.components[0].name, 'app');
  assert.deepEqual(result.dependencies[0].dependsOn, ['pkg:cargo/dep@1.2.3']);
});

test('rejects an unknown dependency from Cargo metadata', () => {
  assert.throws(
    () =>
      cargoMetadataToCycloneDx({
        packages: [{ id: 'app', name: 'app', version: '1' }],
        workspace_members: ['app'],
        resolve: { nodes: [{ id: 'app', dependencies: ['missing'] }] },
      }),
    /UNKNOWN_CARGO_DEPENDENCY/u,
  );
});

test('hash manifest is deterministic, recursive, and byte exact', async (context) => {
  const root = await mkdtemp(path.join(tmpdir(), 'lorepia-release-evidence-'));
  context.after(() => rm(root, { recursive: true, force: true }));
  await mkdir(path.join(root, 'nested'));
  await writeFile(path.join(root, 'z.txt'), 'z');
  await writeFile(path.join(root, 'nested', 'a.txt'), 'a');

  const first = await buildHashManifest({
    base: root,
    inputs: ['z.txt', 'nested'],
    label: 'test',
    commit: COMMIT,
  });
  const second = await buildHashManifest({
    base: root,
    inputs: ['z.txt', 'nested'],
    label: 'test',
    commit: COMMIT,
  });

  assert.deepEqual(first, second);
  assert.deepEqual(first.files.map((file) => file.path), ['nested/a.txt', 'z.txt']);
  assert.equal(first.files[0].size, 1);
  assert.match(first.files[0].sha256, /^[0-9a-f]{64}$/u);
});

test('hash manifest rejects symlinks and non-full commits', async (context) => {
  const root = await mkdtemp(path.join(tmpdir(), 'lorepia-release-evidence-'));
  context.after(() => rm(root, { recursive: true, force: true }));
  await writeFile(path.join(root, 'target'), 'safe');
  await symlink(path.join(root, 'target'), path.join(root, 'link'));

  await assert.rejects(
    buildHashManifest({ base: root, inputs: ['link'], label: 'test', commit: COMMIT }),
    /SYMLINK_NOT_ALLOWED/u,
  );
  await assert.rejects(
    buildHashManifest({ base: root, inputs: ['target'], label: 'test', commit: 'short' }),
    /FULL_COMMIT_REQUIRED/u,
  );
});

test('dependency policy rejects missing and unreviewed licenses', () => {
  const policy = {
    schemaVersion: 1,
    cargo: { allowedLicenseExpressions: ['MIT'] },
    npm: { allowedLicenseExpressions: ['Apache-2.0'] },
  };
  const valid = verifyDependencyPolicy({
    cargoMetadata: { packages: [{ name: 'safe', version: '1.0.0', license: 'MIT' }] },
    npmLock: {
      packages: {
        '': { name: 'root', version: '1.0.0' },
        'node_modules/safe': { name: 'safe', version: '1.0.0', license: 'Apache-2.0' },
      },
    },
    policy,
  });
  assert.deepEqual(valid, { cargoPackages: 1, npmPackages: 1 });

  assert.throws(
    () =>
      verifyDependencyPolicy({
        cargoMetadata: { packages: [{ name: 'new', version: '1.0.0', license: 'GPL-3.0' }] },
        npmLock: { packages: { '': {} } },
        policy,
      }),
    /DEPENDENCY_LICENSE_REVIEW_REQUIRED/u,
  );
  assert.throws(
    () =>
      verifyDependencyPolicy({
        cargoMetadata: { packages: [{ name: 'missing', version: '1.0.0', license: null }] },
        npmLock: { packages: { '': {} } },
        policy,
      }),
    /DEPENDENCY_LICENSE_REVIEW_REQUIRED/u,
  );
});

test('NUL file lists preserve spaces and reject empty entries', () => {
  assert.deepEqual(decodeNulFileList(Buffer.from('one\0path with spaces\0', 'utf8')), [
    'one',
    'path with spaces',
  ]);
  assert.throws(() => decodeNulFileList(Buffer.from('one\0\0two\0', 'utf8')), /MALFORMED/u);
  assert.throws(() => decodeNulFileList(Buffer.alloc(0)), /MALFORMED/u);
});
