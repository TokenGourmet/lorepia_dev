import assert from 'node:assert/strict';
import test from 'node:test';

import { validateWorkflowText } from './verify-workflow-security.mjs';

const SHA = '0123456789abcdef0123456789abcdef01234567';

test('accepts immutable actions and a non-persistent checkout', () => {
  const result = validateWorkflowText(
    'ok.yml',
    `jobs:
  verify:
    steps:
      - name: checkout
        uses: actions/checkout@${SHA} # v6
        with:
          persist-credentials: false
      - uses: ./local-action
      - uses: "owner/action/path@${SHA}"
      - uses: docker://registry.example/image@sha256:${'a'.repeat(64)}
`,
  );

  assert.deepEqual(result.errors, []);
  assert.equal(result.externalActionCount, 3);
  assert.equal(result.checkoutCount, 1);
});

test('rejects a moving action tag', () => {
  const result = validateWorkflowText(
    'tag.yml',
    `steps:
  - uses: actions/setup-node@v6
`,
  );

  assert.equal(result.errors.length, 1);
  assert.match(result.errors[0], /immutable 40-character commit SHA/u);
});

test('rejects checkout without an explicit credential policy', () => {
  const result = validateWorkflowText(
    'missing.yml',
    `steps:
  - uses: actions/checkout@${SHA}
  - run: true
`,
  );

  assert.equal(result.errors.length, 1);
  assert.match(result.errors[0], /persist-credentials: false/u);
});

test('rejects checkout with persistent credentials enabled', () => {
  const result = validateWorkflowText(
    'enabled.yml',
    `steps:
  - uses: actions/checkout@${SHA}
    with:
      persist-credentials: true
`,
  );

  assert.equal(result.errors.length, 1);
  assert.match(result.errors[0], /persist-credentials: false/u);
});

test('rejects moving Docker tags and accepts only image digests', () => {
  const result = validateWorkflowText(
    'docker.yml',
    `steps:
  - uses: docker://alpine:latest
  - uses: 'docker://registry.example/tool@sha256:${'b'.repeat(64)}'
`,
  );

  assert.equal(result.externalActionCount, 2);
  assert.equal(result.errors.length, 1);
  assert.match(result.errors[0], /immutable sha256 image digest/u);
});

test('rejects flow-style uses instead of silently skipping it', () => {
  const result = validateWorkflowText(
    'flow.yml',
    `steps:
  - { name: hidden, uses: actions/setup-node@v6 }
  - { name: quoted, "uses": "actions/checkout@${SHA}" }
`,
  );

  assert.equal(result.errors.length, 2);
  assert.ok(result.errors.every((error) => /flow-style uses syntax/u.test(error)));
});

test('rejects dynamic or multiline uses values instead of silently skipping them', () => {
  const result = validateWorkflowText(
    'dynamic.yml',
    `steps:
  - uses: \${{ matrix.action }}
  - uses: >-
      actions/setup-node@${SHA}
`,
  );

  assert.equal(result.errors.length, 2);
  assert.ok(result.errors.every((error) => /one closed scalar/u.test(error)));
});
