import { describe, expect, it, vi } from "vitest";

import {
  canRequestBrowserDownload,
  copySafetyArtifactJson,
  deliverSafetyArtifact,
  type ArtifactDeliveryPlatform,
} from "./artifact-delivery";
import { utf8ByteLength, type SafetyArtifact } from "./native-support";

const json = '{"contractVersion":1}\n';
const ARTIFACT: SafetyArtifact = {
  fileName: "lorepia-diagnostics.json",
  mediaType: "application/vnd.lorepia.diagnostics+json",
  byteLength: utf8ByteLength(json),
  json,
};

function platform(
  share: ArtifactDeliveryPlatform["share"],
): {
  value: ArtifactDeliveryPlatform;
  download: ReturnType<typeof vi.fn>;
  copy: ReturnType<typeof vi.fn>;
} {
  const download = vi.fn().mockReturnValue(true);
  const copy = vi.fn();
  return {
    value: { share, download, copy },
    download,
    copy,
  };
}

describe("safety artifact delivery", () => {
  it("refuses unconfirmable Android WebView anchor downloads", () => {
    expect(
      canRequestBrowserDownload(
        "Mozilla/5.0 (Linux; Android 16; sdk_gphone64_arm64) wv",
        false,
      ),
    ).toBe(false);
    expect(
      canRequestBrowserDownload(
        "Mozilla/5.0 (Linux; Android 16; sdk_gphone64_arm64)",
        true,
      ),
    ).toBe(false);
    expect(
      canRequestBrowserDownload(
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)",
        true,
      ),
    ).toBe(true);
  });

  it("uses file sharing without also starting a download", async () => {
    const share = vi.fn().mockResolvedValue(true);
    const target = platform(share);

    await expect(deliverSafetyArtifact(ARTIFACT, target.value)).resolves.toBe(
      "shared",
    );
    expect(share).toHaveBeenCalledWith(ARTIFACT);
    expect(target.download).not.toHaveBeenCalled();
  });

  it("falls back to a local file download when file sharing is unsupported", async () => {
    const share = vi.fn().mockResolvedValue(false);
    const target = platform(share);

    await expect(deliverSafetyArtifact(ARTIFACT, target.value)).resolves.toBe(
      "download-requested",
    );
    expect(target.download).toHaveBeenCalledWith(ARTIFACT);
  });

  it("does not report an unconfirmed download request as success", async () => {
    const target = platform(vi.fn().mockResolvedValue(false));
    target.download.mockReturnValue(false);

    await expect(
      deliverSafetyArtifact(ARTIFACT, target.value),
    ).rejects.toThrow("ARTIFACT_DOWNLOAD_UNAVAILABLE");
    expect(target.download).toHaveBeenCalledWith(ARTIFACT);
  });

  it("does not turn a cancelled share into an unexpected download", async () => {
    const cancellation = Object.assign(new Error("cancelled"), {
      name: "AbortError",
    });
    const share = vi.fn().mockRejectedValue(cancellation);
    const target = platform(share);

    await expect(
      deliverSafetyArtifact(ARTIFACT, target.value),
    ).rejects.toBe(cancellation);
    expect(target.download).not.toHaveBeenCalled();
  });

  it("copies only the already-validated JSON artifact", async () => {
    const target = platform(undefined);

    await copySafetyArtifactJson(ARTIFACT, target.value);
    expect(target.copy).toHaveBeenCalledWith(json);

    await expect(
      copySafetyArtifactJson(
        { ...ARTIFACT, byteLength: ARTIFACT.byteLength + 1 },
        target.value,
      ),
    ).rejects.toThrow("INVALID_ARTIFACT_DELIVERY_INPUT");
    expect(target.copy).toHaveBeenCalledTimes(1);
  });
});
