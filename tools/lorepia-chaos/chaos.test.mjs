import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { spawn } from 'node:child_process';
import test from 'node:test';

import { PROFILE_NAMES, buildPlan, sanitizeRequest } from './chaos.mjs';

test('all profiles produce deterministic bounded plans', () => {
  for (const profile of PROFILE_NAMES) {
    assert.deepEqual(buildPlan('stable-seed', profile), buildPlan('stable-seed', profile));
    assert.equal(buildPlan('stable-seed', profile).recordsRawRequest, false);
    assert.equal(buildPlan('stable-seed', profile).recordsHeaderValues, false);
    assert.equal(buildPlan('stable-seed', profile).recordsBody, false);
  }
  assert.notDeepEqual(
    buildPlan('seed-a', 'fragmented-sse').fragmentBytes,
    buildPlan('seed-b', 'fragmented-sse').fragmentBytes,
  );
});

test('request sanitizer records shape but not target, values, secret, or body', () => {
  const secret = 'CHAOS-SENTINEL-NEVER-RECORD';
  const raw = Buffer.from(`POST /v1/chat?token=${secret} HTTP/1.1\r\nAuthorization: Bearer ${secret}\r\nX-Name: value\r\nContent-Length: 6\r\n\r\nsecret`);
  const receipt = sanitizeRequest(raw);
  const encoded = JSON.stringify(receipt);
  assert.equal(receipt.authorizationPresent, true);
  assert.equal(receipt.targetHasQuery, true);
  assert.equal(receipt.declaredBodyBytes, 6);
  assert(!encoded.includes(secret));
  assert(!encoded.includes('/v1/chat'));
  assert(!encoded.includes('value'));
});

test('localhost server writes a secret-free receipt', async () => {
  const directory = await mkdtemp(path.join(os.tmpdir(), 'lorepia-chaos-'));
  const receiptPath = path.join(directory, 'receipt.json');
  const child = spawn(process.execPath, [
    path.join(import.meta.dirname, 'chaos.mjs'),
    'serve',
    '--seed', '42',
    '--profile', 'malformed-json',
    '--receipt', receiptPath,
    '--lifetime-ms', '300',
  ], { stdio: ['ignore', 'pipe', 'pipe'] });
  try {
    const startup = await new Promise((resolve, reject) => {
      let output = '';
      child.stdout.on('data', (chunk) => {
        output += chunk;
        const newline = output.indexOf('\n');
        if (newline !== -1) resolve(JSON.parse(output.slice(0, newline)));
      });
      child.once('error', reject);
    });
    const secret = 'CHAOS-SENTINEL-NEVER-RECORD';
    await new Promise((resolve, reject) => {
      const socket = net.connect(startup.port, '127.0.0.1', () => {
        socket.end(`POST /v1?api_key=${secret} HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer ${secret}\r\nContent-Length: 6\r\n\r\nsecret`);
      });
      socket.on('data', () => {});
      socket.once('close', resolve);
      socket.once('error', reject);
    });
    const exitCode = await new Promise((resolve) => child.once('exit', resolve));
    assert.equal(exitCode, 0);
    const rawReceipt = await readFile(receiptPath, 'utf8');
    const receipt = JSON.parse(rawReceipt);
    assert.equal(receipt.observationCount, 1);
    assert.equal(receipt.observations[0].authorizationPresent, true);
    assert.equal(receipt.rawSecretOrBodyRecorded, false);
    assert(!rawReceipt.includes(secret));
    assert(!rawReceipt.includes('/v1'));
  } finally {
    child.kill('SIGKILL');
    await rm(directory, { recursive: true, force: true });
  }
});
