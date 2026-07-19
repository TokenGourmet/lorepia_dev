import { describe, expect, it, vi } from "vitest";

import {
  PRODUCT_BOOTSTRAP_COMMAND,
  PRODUCT_BOOTSTRAP_ERROR_MESSAGE,
  parseProductBootstrap,
  publicBootstrapError,
  requestProductBootstrap,
} from "./product-bootstrap";

const VALID_BOOTSTRAP = {
  contractVersion: 1,
  productName: "LorePia",
  coreVersion: "0.1.0",
  dataPolicy: "DEVICE_LOCAL_EXCEPT_USER_SELECTED_LLM_REQUESTS",
  importedExecutableContent: "DISABLED_PENDING_M1_EVIDENCE",
} as const;

describe("product bootstrap contract", () => {
  it("accepts the exact native response", () => {
    expect(parseProductBootstrap(VALID_BOOTSTRAP)).toEqual(VALID_BOOTSTRAP);
  });

  it("accepts Cargo prerelease and build metadata versions", () => {
    const prerelease = {
      ...VALID_BOOTSTRAP,
      coreVersion: "0.2.0-beta.1+mobile.7",
    };

    expect(parseProductBootstrap(prerelease)).toEqual(prerelease);
  });

  it.each([
    null,
    [],
    { ...VALID_BOOTSTRAP, contractVersion: 2 },
    { ...VALID_BOOTSTRAP, coreVersion: "development" },
    { ...VALID_BOOTSTRAP, extra: true },
    {
      ...VALID_BOOTSTRAP,
      importedExecutableContent: "ENABLED",
    },
  ])("rejects an invalid or widened native response", (response) => {
    expect(() => parseProductBootstrap(response)).toThrow();
  });

  it("invokes only the product bootstrap command", async () => {
    const invokeCommand = vi.fn().mockResolvedValue(VALID_BOOTSTRAP);

    await expect(requestProductBootstrap(invokeCommand)).resolves.toEqual(
      VALID_BOOTSTRAP,
    );
    expect(invokeCommand).toHaveBeenCalledOnce();
    expect(invokeCommand).toHaveBeenCalledWith(PRODUCT_BOOTSTRAP_COMMAND);
  });

  it("never exposes a native error to the product screen", () => {
    const nativeError = "secret/path/provider/key";
    expect(publicBootstrapError()).toBe(PRODUCT_BOOTSTRAP_ERROR_MESSAGE);
    expect(publicBootstrapError()).not.toContain(nativeError);
  });
});
