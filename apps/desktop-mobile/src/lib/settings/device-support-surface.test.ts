import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const settingsSource = readFileSync(
  new URL("./SettingsPane.svelte", import.meta.url),
  "utf8",
);
const supportSource = readFileSync(
  new URL("./DeviceSupportPane.svelte", import.meta.url),
  "utf8",
);

describe("device support settings surface", () => {
  it("mounts the native support pane in the visible data section", () => {
    expect(settingsSource).toContain(
      'import DeviceSupportPane from "./DeviceSupportPane.svelte"',
    );
    expect(settingsSource).toContain("<h2>데이터</h2>");
    expect(settingsSource).toContain("<DeviceSupportPane />");
  });

  it("connects every user-facing device status and diagnostic command", () => {
    expect(supportSource).toContain("storageClient.getStorageStatus()");
    expect(supportSource).toContain("requestAssetStoreStatus()");
    expect(supportSource).toContain("requestProductSafetyContract()");
    expect(supportSource).toContain("requestRedactedDiagnostics()");
    expect(supportSource).toContain("deliverSafetyArtifact");
    expect(supportSource).toContain("copySafetyArtifactJson");
  });

  it("keeps advanced JSON collapsed and labels the artifact as local", () => {
    expect(supportSource).toContain("<details>");
    expect(supportSource).toContain("생성된 JSON 검토");
    expect(supportSource).toContain("자동 전송이나 원격 신고는");
  });

  it("does not describe an unverified WebView download as completed", () => {
    expect(supportSource).toContain(
      "브라우저에 파일 저장을 요청했습니다.",
    );
    expect(supportSource).not.toContain("파일 저장을 완료했습니다.");
  });
});
