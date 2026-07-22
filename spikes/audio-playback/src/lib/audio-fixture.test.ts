import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";

import { describe, expect, it, vi } from "vitest";

import catalog from "../../fixtures/catalog.json";
import { AUDIO_FIXTURE } from "./audio-contract";
import { AudioM1Error } from "./audio-error";
import {
  loadApprovedFixture,
  verifyFixtureBytes,
  verifyWavHeader,
  type FixedFixtureFetch,
} from "./audio-fixture";

const sourceFixture = readFileSync(
  new URL("../../static/fixtures/m1-audio-v1.wav", import.meta.url),
);
const sourceBytes = new Uint8Array(
  sourceFixture.buffer,
  sourceFixture.byteOffset,
  sourceFixture.byteLength,
);
const nodeDigest = async (bytes: Uint8Array): Promise<string> =>
  createHash("sha256").update(bytes).digest("hex");

describe("fixed audio fixture", () => {
  it("matches the pinned catalog, hash, and WAV contract", async () => {
    expect(catalog).toMatchObject(AUDIO_FIXTURE);
    expect(catalog).toMatchObject({
      protocolVersion: "m1-audio-fixture-v1",
      sourcePath: "static/fixtures/m1-audio-v1.wav",
      origin: "self-authored",
      license: "CC0-1.0",
      dataBytes: 1_152_000,
      fadeMs: 20,
      peakAmplitude: 8_192,
    });
    expect(catalog.segments).toEqual([
      { startMs: 0, endMs: 3_000, frequencyHz: 440 },
      { startMs: 3_000, endMs: 6_000, frequencyHz: 660 },
      { startMs: 6_000, endMs: 9_000, frequencyHz: 880 },
      { startMs: 9_000, endMs: 12_000, frequencyHz: 1_100 },
    ]);
    expect(sourceFixture.byteLength).toBe(AUDIO_FIXTURE.bytes);
    expect(await nodeDigest(sourceBytes)).toBe(AUDIO_FIXTURE.sha256);
    expect(await verifyFixtureBytes(sourceBytes, nodeDigest)).toMatchObject({
      bytesAndSha256Matched: true,
      wavHeaderMatched: true,
    });
  });

  it("rejects wrong length, oversized bytes, header drift, and hash drift", async () => {
    await expect(
      verifyFixtureBytes(sourceBytes.slice(0, -1), nodeDigest),
    ).rejects.toMatchObject({ code: "FIXTURE_MISMATCH" });
    await expect(
      verifyFixtureBytes(new Uint8Array(1_310_721), nodeDigest),
    ).rejects.toMatchObject({ code: "FIXTURE_TOO_LARGE" });

    const wrongHeader = Uint8Array.from(sourceBytes);
    wrongHeader[0] = 0;
    expect(() => verifyWavHeader(wrongHeader)).toThrowError(
      expect.objectContaining({ code: "FIXTURE_UNSUPPORTED" }),
    );
    await expect(
      verifyFixtureBytes(sourceBytes, async () => "0".repeat(64)),
    ).rejects.toMatchObject({ code: "FIXTURE_MISMATCH" });
  });

  it("fetches only the literal same-origin fixture with bounded options", async () => {
    const fixedFetch = vi.fn(async () =>
      new Response(Uint8Array.from(sourceBytes), {
        status: 200,
        headers: {
          "content-length": String(AUDIO_FIXTURE.bytes),
          "content-type": "audio/wav",
        },
      }),
    ) as unknown as FixedFixtureFetch;

    const abortController = new AbortController();
    const verified = await loadApprovedFixture(
      fixedFetch,
      nodeDigest,
      abortController.signal,
    );
    expect(verified.bytes.byteLength).toBe(AUDIO_FIXTURE.bytes);
    expect(fixedFetch).toHaveBeenCalledExactlyOnceWith(AUDIO_FIXTURE.publicPath, {
      cache: "no-store",
      credentials: "same-origin",
      redirect: "error",
      signal: abortController.signal,
    });
  });

  it("rejects failed, oversized, and mismatched fetch responses", async () => {
    const notFound = vi.fn(async () => new Response(null, { status: 404 })) as unknown as FixedFixtureFetch;
    await expect(loadApprovedFixture(notFound, nodeDigest)).rejects.toMatchObject({
      code: "FIXTURE_LOAD_FAILED",
    });

    const tooLarge = vi.fn(async () =>
      new Response(null, { status: 200, headers: { "content-length": "1310721" } }),
    ) as unknown as FixedFixtureFetch;
    await expect(loadApprovedFixture(tooLarge, nodeDigest)).rejects.toMatchObject({
      code: "FIXTURE_TOO_LARGE",
    });

    const wrongLength = vi.fn(async () =>
      new Response(null, { status: 200, headers: { "content-length": "100" } }),
    ) as unknown as FixedFixtureFetch;
    await expect(loadApprovedFixture(wrongLength, nodeDigest)).rejects.toMatchObject({
      code: "FIXTURE_MISMATCH",
    });

    const undeclaredOversize = vi.fn(async () =>
      new Response(new Uint8Array(1_310_721), { status: 200 }),
    ) as unknown as FixedFixtureFetch;
    await expect(loadApprovedFixture(undeclaredOversize, nodeDigest)).rejects.toMatchObject({
      code: "FIXTURE_TOO_LARGE",
    });
  });

  it("never exposes a digest implementation failure", async () => {
    await expect(
      verifyFixtureBytes(sourceBytes, async () => {
        throw new Error("native digest path");
      }),
    ).rejects.toEqual(new AudioM1Error("FIXTURE_UNSUPPORTED"));
  });
});
