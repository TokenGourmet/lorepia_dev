import { AUDIO_FIXTURE } from "./audio-contract";
import { AudioM1Error } from "./audio-error";

const MAX_FIXTURE_BYTES = 1_310_720;
const WAV_HEADER_BYTES = 44;

export type FixtureVerification = {
  bytes: Uint8Array;
  bytesAndSha256Matched: true;
  wavHeaderMatched: true;
};

export type DigestSha256 = (bytes: Uint8Array) => Promise<string>;
export type FixedFixtureFetch = (
  input: typeof AUDIO_FIXTURE.publicPath,
  init: Readonly<{
    cache: "no-store";
    credentials: "same-origin";
    redirect: "error";
    signal?: AbortSignal;
  }>,
) => Promise<Response>;

function ascii(bytes: Uint8Array, offset: number, length: number): string {
  return String.fromCharCode(...bytes.slice(offset, offset + length));
}

export function verifyWavHeader(bytes: Uint8Array): void {
  if (bytes.byteLength !== AUDIO_FIXTURE.bytes || bytes.byteLength < WAV_HEADER_BYTES) {
    throw new AudioM1Error("FIXTURE_MISMATCH");
  }
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const bytesPerSample = AUDIO_FIXTURE.bitsPerSample / 8;
  const blockAlign = AUDIO_FIXTURE.channels * bytesPerSample;
  const dataBytes = AUDIO_FIXTURE.frameCount * blockAlign;
  const byteRate = AUDIO_FIXTURE.sampleRateHz * blockAlign;
  if (
    ascii(bytes, 0, 4) !== "RIFF" ||
    view.getUint32(4, true) !== bytes.byteLength - 8 ||
    ascii(bytes, 8, 4) !== "WAVE" ||
    ascii(bytes, 12, 4) !== "fmt " ||
    view.getUint32(16, true) !== 16 ||
    view.getUint16(20, true) !== 1 ||
    view.getUint16(22, true) !== AUDIO_FIXTURE.channels ||
    view.getUint32(24, true) !== AUDIO_FIXTURE.sampleRateHz ||
    view.getUint32(28, true) !== byteRate ||
    view.getUint16(32, true) !== blockAlign ||
    view.getUint16(34, true) !== AUDIO_FIXTURE.bitsPerSample ||
    ascii(bytes, 36, 4) !== "data" ||
    view.getUint32(40, true) !== dataBytes
  ) {
    throw new AudioM1Error("FIXTURE_UNSUPPORTED");
  }
}

export async function browserSha256(bytes: Uint8Array): Promise<string> {
  const subtle = globalThis.crypto?.subtle;
  if (subtle === undefined) throw new AudioM1Error("FIXTURE_UNSUPPORTED");
  const copy = Uint8Array.from(bytes);
  const digest = await subtle.digest("SHA-256", copy.buffer);
  return [...new Uint8Array(digest)]
    .map((value) => value.toString(16).padStart(2, "0"))
    .join("");
}

export async function verifyFixtureBytes(
  bytes: Uint8Array,
  digestSha256: DigestSha256 = browserSha256,
): Promise<FixtureVerification> {
  if (bytes.byteLength > MAX_FIXTURE_BYTES) {
    throw new AudioM1Error("FIXTURE_TOO_LARGE");
  }
  if (bytes.byteLength !== AUDIO_FIXTURE.bytes) {
    throw new AudioM1Error("FIXTURE_MISMATCH");
  }
  verifyWavHeader(bytes);
  let digest: string;
  try {
    digest = await digestSha256(bytes);
  } catch (error) {
    if (error instanceof AudioM1Error) throw error;
    throw new AudioM1Error("FIXTURE_UNSUPPORTED");
  }
  if (digest !== AUDIO_FIXTURE.sha256) {
    throw new AudioM1Error("FIXTURE_MISMATCH");
  }
  return {
    bytes,
    bytesAndSha256Matched: true,
    wavHeaderMatched: true,
  };
}

async function readBoundedBody(response: Response): Promise<Uint8Array> {
  if (response.body === null) {
    throw new AudioM1Error("FIXTURE_LOAD_FAILED");
  }
  const reader = response.body.getReader();
  const chunks: Uint8Array[] = [];
  let totalBytes = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      totalBytes += value.byteLength;
      if (totalBytes > MAX_FIXTURE_BYTES) {
        await reader.cancel();
        throw new AudioM1Error("FIXTURE_TOO_LARGE");
      }
      chunks.push(value);
    }
  } catch (error) {
    if (error instanceof AudioM1Error) throw error;
    throw new AudioM1Error("FIXTURE_LOAD_FAILED");
  } finally {
    reader.releaseLock();
  }
  const bytes = new Uint8Array(totalBytes);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return bytes;
}

export async function loadApprovedFixture(
  fixedFetch: FixedFixtureFetch = globalThis.fetch.bind(globalThis),
  digestSha256: DigestSha256 = browserSha256,
  signal?: AbortSignal,
): Promise<FixtureVerification> {
  let response: Response;
  try {
    response = await fixedFetch(AUDIO_FIXTURE.publicPath, {
      cache: "no-store",
      credentials: "same-origin",
      redirect: "error",
      ...(signal === undefined ? {} : { signal }),
    });
  } catch {
    throw new AudioM1Error("FIXTURE_LOAD_FAILED");
  }
  if (!response.ok || response.redirected) {
    throw new AudioM1Error("FIXTURE_LOAD_FAILED");
  }
  const contentLength = response.headers.get("content-length");
  if (contentLength !== null) {
    const parsedLength = Number(contentLength);
    if (!Number.isSafeInteger(parsedLength) || parsedLength > MAX_FIXTURE_BYTES) {
      throw new AudioM1Error("FIXTURE_TOO_LARGE");
    }
    if (parsedLength !== AUDIO_FIXTURE.bytes) {
      throw new AudioM1Error("FIXTURE_MISMATCH");
    }
  }
  return verifyFixtureBytes(await readBoundedBody(response), digestSha256);
}
