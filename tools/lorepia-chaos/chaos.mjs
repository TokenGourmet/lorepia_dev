#!/usr/bin/env node

import { writeFile } from 'node:fs/promises';
import net from 'node:net';
import process from 'node:process';

export const PROFILE_NAMES = Object.freeze([
  'fragmented-sse',
  'malformed-json',
  'missing-content-type',
  'wrong-content-type',
  'oversized-frame',
  'header-bomb',
  'gzip',
  'deflate',
  'brotli',
  'zstd',
  'redirect-301',
  'redirect-302',
  'redirect-307',
  'redirect-308',
  'header-stall',
  'idle-stream',
  'connection-reset',
  'http-429',
  'http-500',
  'http-502',
  'http-503',
  'eof-no-terminal',
  'usage-only',
  'mixed-channels',
  'malicious-control',
]);

function seedNumber(seed) {
  const text = String(seed);
  let value = 0x811c9dc5;
  for (const character of text) {
    value ^= character.codePointAt(0);
    value = Math.imul(value, 0x01000193) >>> 0;
  }
  return value || 1;
}

function generator(seed) {
  let state = seedNumber(seed);
  return () => {
    state ^= state << 13;
    state ^= state >>> 17;
    state ^= state << 5;
    return (state >>> 0) / 0x1_0000_0000;
  };
}

export function buildPlan(seed, profile) {
  if (!PROFILE_NAMES.includes(profile)) {
    throw new Error(`unknown profile: ${profile}`);
  }
  const random = generator(seed);
  const fragmentBytes = Array.from({ length: 12 }, () => 1 + Math.floor(random() * 17));
  const fragmentDelayMs = Array.from({ length: 12 }, () => Math.floor(random() * 5));
  return Object.freeze({
    schemaVersion: 1,
    seedId: seedNumber(seed).toString(16).padStart(8, '0'),
    profile,
    fragmentBytes,
    fragmentDelayMs,
    declaredHostileBytes: profile === 'oversized-frame' ? 100 * 1024 * 1024 : 0,
    retryAfterSeconds: profile === 'http-429' ? 7 : null,
    recordsRawRequest: false,
    recordsHeaderValues: false,
    recordsBody: false,
  });
}

export function sanitizeRequest(bytes) {
  const headEnd = bytes.indexOf('\r\n\r\n');
  const headBytes = headEnd === -1 ? bytes : bytes.subarray(0, headEnd);
  const lines = headBytes.toString('latin1').split('\r\n');
  const [rawMethod = '', target = ''] = (lines.shift() ?? '').split(' ');
  const method = ['GET', 'POST', 'PUT', 'PATCH', 'DELETE', 'HEAD', 'OPTIONS'].includes(rawMethod)
    ? rawMethod
    : 'OTHER';
  const observedHeaderNames = lines
    .map((line) => {
      const index = line.indexOf(':');
      return index > 0 ? line.slice(0, index).trim().toLowerCase() : null;
    })
    .filter(Boolean);
  const safeHeaderAllowlist = new Set([
    'accept', 'accept-encoding', 'authorization', 'connection', 'content-length',
    'content-type', 'host', 'user-agent', 'x-api-key', 'x-goog-api-key',
  ]);
  const headerNames = observedHeaderNames
    .filter((name) => safeHeaderAllowlist.has(name))
    .sort();
  const otherHeaderCount = observedHeaderNames.length - headerNames.length;
  const contentLengthLine = lines.find((line) => /^content-length\s*:/i.test(line));
  const declaredBodyBytes = contentLengthLine
    ? Number.parseInt(contentLengthLine.slice(contentLengthLine.indexOf(':') + 1).trim(), 10) || 0
    : null;
  return Object.freeze({
    method,
    targetBytes: Buffer.byteLength(target),
    targetHasQuery: target.includes('?'),
    headerNames,
    otherHeaderCount,
    authorizationPresent: headerNames.includes('authorization') || headerNames.includes('x-api-key') || headerNames.includes('x-goog-api-key'),
    declaredBodyBytes,
    receivedBytes: bytes.length,
  });
}

function responseHead(status, headers = [], length = null) {
  const lines = [`HTTP/1.1 ${status}`, 'Connection: close', ...headers];
  if (length !== null) lines.push(`Content-Length: ${length}`);
  return `${lines.join('\r\n')}\r\n\r\n`;
}

function normalSseBody() {
  return Buffer.from([
    'event: response.output_text.delta',
    'data: {"type":"response.output_text.delta","delta":"hello"}',
    '',
    'event: response.completed',
    'data: {"type":"response.completed","response":{}}',
    '',
    '',
  ].join('\n'));
}

function fixedResponse(profile, port) {
  switch (profile) {
    case 'malformed-json':
      return ['200 OK', ['Content-Type: text/event-stream'], Buffer.from('data: {bad-json}\n\n')];
    case 'missing-content-type':
      return ['200 OK', [], normalSseBody()];
    case 'wrong-content-type':
      return ['200 OK', ['Content-Type: application/json'], normalSseBody()];
    case 'header-bomb':
      return ['200 OK', ['Content-Type: text/event-stream', ...Array.from({ length: 80 }, (_, index) => `X-Fault-${index}: ${'x'.repeat(256)}`)], Buffer.alloc(0)];
    case 'gzip':
    case 'deflate':
      return ['200 OK', ['Content-Type: text/event-stream', `Content-Encoding: ${profile}`], Buffer.from('compressed-response-must-be-rejected-before-decoding')];
    case 'brotli':
      return ['200 OK', ['Content-Type: text/event-stream', 'Content-Encoding: br'], Buffer.from('compressed-response-must-be-rejected-before-decoding')];
    case 'zstd':
      return ['200 OK', ['Content-Type: text/event-stream', 'Content-Encoding: zstd'], Buffer.from('compressed-response-must-be-rejected-before-decoding')];
    case 'redirect-301':
    case 'redirect-302':
    case 'redirect-307':
    case 'redirect-308': {
      const code = profile.slice(-3);
      const reason = { 301: 'Moved Permanently', 302: 'Found', 307: 'Temporary Redirect', 308: 'Permanent Redirect' }[code];
      return [`${code} ${reason}`, [`Location: http://127.0.0.1:${port}/must-not-follow`], Buffer.alloc(0)];
    }
    case 'http-429':
      return ['429 Too Many Requests', ['Retry-After: 7'], Buffer.from('provider body is intentionally not recorded')];
    case 'http-500':
      return ['500 Internal Server Error', [], Buffer.alloc(0)];
    case 'http-502':
      return ['502 Bad Gateway', [], Buffer.alloc(0)];
    case 'http-503':
      return ['503 Service Unavailable', [], Buffer.alloc(0)];
    case 'eof-no-terminal':
      return ['200 OK', ['Content-Type: text/event-stream'], Buffer.from('data: {"type":"response.output_text.delta","delta":"partial"}\n\n')];
    case 'usage-only':
      return ['200 OK', ['Content-Type: text/event-stream'], Buffer.from('data: {"type":"response.completed","response":{"usage":{"input_tokens":9,"output_tokens":0,"total_tokens":9}}}\n\n')];
    case 'mixed-channels':
      return ['200 OK', ['Content-Type: text/event-stream'], Buffer.from([
        'data: {"type":"response.reasoning_summary_text.delta","delta":"think"}', '',
        'data: {"type":"response.output_text.delta","delta":"say"}', '',
        'data: {"type":"response.refusal.delta","delta":"no"}', '',
        'data: {"type":"response.completed","response":{}}', '', '',
      ].join('\n'))];
    case 'malicious-control':
      return ['200 OK', ['Content-Type: text/event-stream'], Buffer.from('data: {"type":"response.failed","response":{"error":{"message":"\\u0000\\u001b[31m\\nspoof"}}}\n\n')];
    default:
      return null;
  }
}

async function writeFragments(socket, bytes, plan) {
  let offset = 0;
  let index = 0;
  while (offset < bytes.length && !socket.destroyed) {
    const count = plan.fragmentBytes[index % plan.fragmentBytes.length];
    const fragment = bytes.subarray(offset, Math.min(offset + count, bytes.length));
    socket.write(fragment);
    offset += fragment.length;
    const delay = plan.fragmentDelayMs[index % plan.fragmentDelayMs.length];
    if (delay > 0) await new Promise((resolve) => setTimeout(resolve, delay));
    index += 1;
  }
  socket.end();
  return offset;
}

function parseArguments(argv) {
  const command = argv[0];
  const values = new Map();
  for (let index = 1; index < argv.length; index += 2) {
    const key = argv[index];
    const value = argv[index + 1];
    if (!key?.startsWith('--') || value === undefined) throw new Error(`invalid argument near ${key ?? '<end>'}`);
    values.set(key.slice(2), value);
  }
  return { command, values };
}

async function serve(values) {
  const seed = values.get('seed') ?? '1';
  const profile = values.get('profile') ?? 'fragmented-sse';
  const receiptPath = values.get('receipt');
  if (!receiptPath) throw new Error('--receipt is required');
  const requestedPort = Number.parseInt(values.get('port') ?? '0', 10);
  const lifetimeMs = Number.parseInt(values.get('lifetime-ms') ?? '1500', 10);
  if (!Number.isInteger(requestedPort) || requestedPort < 0 || requestedPort > 65535) throw new Error('invalid port');
  if (!Number.isInteger(lifetimeMs) || lifetimeMs < 100 || lifetimeMs > 600_000) throw new Error('invalid lifetime');
  const plan = buildPlan(seed, profile);
  const observations = [];
  const sockets = new Set();
  let responseBytesAttempted = 0;
  let finalized = false;
  const server = net.createServer((socket) => {
    sockets.add(socket);
    socket.once('close', () => sockets.delete(socket));
    const chunks = [];
    let handled = false;
    socket.on('data', async (chunk) => {
      if (handled) return;
      chunks.push(chunk);
      const received = Buffer.concat(chunks);
      if (!received.includes(Buffer.from('\r\n\r\n'))) return;
      handled = true;
      observations.push(sanitizeRequest(received));

      if (profile === 'connection-reset') {
        socket.destroy();
        return;
      }
      if (profile === 'header-stall') return;
      if (profile === 'idle-stream') {
        socket.write(responseHead('200 OK', ['Content-Type: text/event-stream']));
        return;
      }
      if (profile === 'oversized-frame') {
        socket.write(responseHead('200 OK', ['Content-Type: text/event-stream']));
        socket.write('data: ');
        const block = Buffer.alloc(4096, 0x78);
        for (let sent = 0; sent < plan.declaredHostileBytes && !socket.destroyed; sent += block.length) {
          responseBytesAttempted += block.length;
          if (!socket.write(block)) {
            await new Promise((resolve) => {
              const done = () => {
                socket.off('drain', done);
                socket.off('close', done);
                socket.off('error', done);
                resolve();
              };
              socket.once('drain', done);
              socket.once('close', done);
              socket.once('error', done);
            });
          }
        }
        socket.end();
        return;
      }
      if (profile === 'fragmented-sse') {
        const body = normalSseBody();
        const message = Buffer.concat([
          Buffer.from(responseHead('200 OK', ['Content-Type: text/event-stream'], body.length)),
          body,
        ]);
        responseBytesAttempted += await writeFragments(socket, message, plan);
        return;
      }
      const fixed = fixedResponse(profile, server.address().port);
      if (!fixed) throw new Error(`profile has no responder: ${profile}`);
      const [status, headers, body] = fixed;
      const message = Buffer.concat([Buffer.from(responseHead(status, headers, body.length)), body]);
      responseBytesAttempted += message.length;
      socket.end(message);
    });
    socket.on('error', () => {});
  });

  const finalize = async (exitCode = 0) => {
    if (finalized) return;
    finalized = true;
    for (const socket of sockets) socket.destroy();
    server.close();
    const receipt = {
      schemaVersion: 1,
      receiptId: `${seedNumber(seed).toString(16).padStart(8, '0')}-${profile}`,
      plan,
      observationCount: observations.length,
      observations,
      responseBytesAttempted,
      rawSecretOrBodyRecorded: false,
    };
    await writeFile(receiptPath, `${JSON.stringify(receipt, null, 2)}\n`, { mode: 0o600 });
    process.exitCode = exitCode;
  };

  server.listen(requestedPort, '127.0.0.1', () => {
    const address = server.address();
    process.stdout.write(`${JSON.stringify({ schemaVersion: 1, host: '127.0.0.1', port: address.port, profile, seedId: plan.seedId })}\n`);
  });
  const timer = setTimeout(() => void finalize(), lifetimeMs);
  timer.unref();
  process.once('SIGINT', () => void finalize(130));
  process.once('SIGTERM', () => void finalize(143));
  await new Promise((resolve) => server.once('close', resolve));
}

async function main() {
  const { command, values } = parseArguments(process.argv.slice(2));
  if (command === 'plan') {
    const seed = values.get('seed') ?? '1';
    const profile = values.get('profile') ?? 'fragmented-sse';
    process.stdout.write(`${JSON.stringify(buildPlan(seed, profile), null, 2)}\n`);
    return;
  }
  if (command === 'serve') {
    await serve(values);
    return;
  }
  throw new Error('usage: chaos.mjs plan|serve --seed <seed> --profile <profile> [--receipt <path>]');
}

if (process.argv[1] && new URL(import.meta.url).pathname === process.argv[1]) {
  main().catch((error) => {
    process.stderr.write(`lorepia-chaos: ${error.message}\n`);
    process.exitCode = 2;
  });
}
