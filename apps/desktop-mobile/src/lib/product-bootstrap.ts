import { invoke } from "@tauri-apps/api/core";

import { IMPORTED_EXECUTABLE_POLICY } from "./imported-executable-quarantine";

export const PRODUCT_BOOTSTRAP_COMMAND = "get_product_bootstrap" as const;
export const PRODUCT_BOOTSTRAP_ERROR_MESSAGE =
  "제품 코어 상태를 불러오지 못했습니다.";

const DATA_POLICY = "DEVICE_LOCAL_EXCEPT_USER_SELECTED_LLM_REQUESTS" as const;
const IMPORTED_EXECUTABLE_CONTENT = IMPORTED_EXECUTABLE_POLICY;
const CARGO_SEMVER =
  /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/;
const CONTRACT_KEYS = [
  "contractVersion",
  "coreVersion",
  "dataPolicy",
  "importedExecutableContent",
  "productName",
] as const;

export interface ProductBootstrap {
  contractVersion: 2;
  productName: "LorePia";
  coreVersion: string;
  dataPolicy: typeof DATA_POLICY;
  importedExecutableContent: typeof IMPORTED_EXECUTABLE_CONTENT;
}

type InvokeCommand = (command: string) => Promise<unknown>;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function parseProductBootstrap(value: unknown): ProductBootstrap {
  if (!isRecord(value)) {
    throw new Error("invalid product bootstrap envelope");
  }

  const keys = Object.keys(value).sort();
  if (
    keys.length !== CONTRACT_KEYS.length ||
    !CONTRACT_KEYS.every((key, index) => keys[index] === key)
  ) {
    throw new Error("invalid product bootstrap keys");
  }

  if (
    value.contractVersion !== 2 ||
    value.productName !== "LorePia" ||
    typeof value.coreVersion !== "string" ||
    !CARGO_SEMVER.test(value.coreVersion) ||
    value.dataPolicy !== DATA_POLICY ||
    value.importedExecutableContent !== IMPORTED_EXECUTABLE_CONTENT
  ) {
    throw new Error("invalid product bootstrap values");
  }

  return {
    contractVersion: value.contractVersion,
    productName: value.productName,
    coreVersion: value.coreVersion,
    dataPolicy: value.dataPolicy,
    importedExecutableContent: value.importedExecutableContent,
  };
}

export async function requestProductBootstrap(
  invokeCommand: InvokeCommand = invoke,
): Promise<ProductBootstrap> {
  const response = await invokeCommand(PRODUCT_BOOTSTRAP_COMMAND);
  return parseProductBootstrap(response);
}

export function publicBootstrapError(): string {
  return PRODUCT_BOOTSTRAP_ERROR_MESSAGE;
}
