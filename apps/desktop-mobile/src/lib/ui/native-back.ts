import {
  invoke,
  isTauri,
} from "@tauri-apps/api/core";

const PLUGIN_NAME = "native-back";
const COMMAND_PREFIX = `plugin:${PLUGIN_NAME}|`;
const COMMIT_EVENT = "lorepia:native-back";
export const NATIVE_BACK_PROGRESS_EVENT =
  "lorepia:native-back-progress";

export type NativeBackProgressPhase =
  | "start"
  | "progress"
  | "cancel"
  | "commit";

export type NativeBackEdge = "left" | "right";

export interface NativeBackProgress {
  phase: NativeBackProgressPhase;
  progress: number;
  edge: NativeBackEdge;
}

export interface NativeBackStatus {
  supported: boolean;
  active: boolean;
  gestureEnabled: boolean;
}

export interface NativeBackConnection {
  status: NativeBackStatus;
  disconnect: () => void;
}

const UNSUPPORTED_STATUS: NativeBackStatus = Object.freeze({
  supported: false,
  active: false,
  gestureEnabled: false,
});

const NATIVE_BACK_PROGRESS_PHASES = new Set<NativeBackProgressPhase>([
  "start",
  "progress",
  "cancel",
  "commit",
]);
const NATIVE_BACK_EDGES = new Set<NativeBackEdge>([
  "left",
  "right",
]);
let pendingCommitBarrier: Promise<void> | null = null;

function statusValue(
  value: unknown,
  key: keyof NativeBackStatus,
): boolean {
  if (typeof value !== "object" || value === null) return false;
  return (value as Record<string, unknown>)[key] === true;
}

export function normalizeNativeBackStatus(
  value: unknown,
): NativeBackStatus {
  return {
    supported: statusValue(value, "supported"),
    active: statusValue(value, "active"),
    gestureEnabled: statusValue(value, "gestureEnabled"),
  };
}

export function normalizeNativeBackProgress(
  value: unknown,
): NativeBackProgress | null {
  if (typeof value !== "object" || value === null) return null;
  const record = value as Record<string, unknown>;
  if (
    typeof record.phase !== "string" ||
    !NATIVE_BACK_PROGRESS_PHASES.has(
      record.phase as NativeBackProgressPhase,
    ) ||
    typeof record.progress !== "number" ||
    !Number.isFinite(record.progress)
  ) {
    return null;
  }
  return {
    phase: record.phase as NativeBackProgressPhase,
    progress: Math.min(Math.max(record.progress, 0), 1),
    edge:
      typeof record.edge === "string" &&
      NATIVE_BACK_EDGES.has(record.edge as NativeBackEdge)
        ? (record.edge as NativeBackEdge)
        : "left",
  };
}

export function usesNativeBackChrome(
  status: NativeBackStatus,
  nativePlatform: string | undefined,
): boolean {
  // Only UIKit installs a native navigation/title overlay. Android publishes
  // OS-edge progress but keeps the Web chat header and content-wide fallback.
  return (
    nativePlatform === "ios" &&
    status.active &&
    status.gestureEnabled
  );
}

export function shouldOptimisticallyArmNativeBack(
  nativePlatform: string | undefined,
): boolean {
  // UIKit prepares its navigation host before the async status round-trip.
  // Arm only iOS during that gap; the resolved status immediately corrects
  // unsupported/error cases and Android must keep its web fallback throughout.
  return nativePlatform === "ios";
}

/**
 * Shared progress boundary for Android predictive back and any future native
 * host. Native code emits one CustomEvent with `{ phase, progress }`; the web
 * transition owns all visual mapping so desktop pointer fallback and native
 * progress cannot drift apart.
 */
export function connectNativeBackProgress(
  onProgress: (progress: NativeBackProgress) => void,
): () => void {
  if (typeof window.addEventListener !== "function") {
    return () => undefined;
  }
  const handleProgress = (event: Event): void => {
    // Android evaluateJavascript can deliver a cross-realm CustomEvent whose
    // prototype does not satisfy the WebView page's instanceof check.
    const progress = normalizeNativeBackProgress(
      (event as CustomEvent<unknown>).detail,
    );
    if (progress !== null) {
      if (progress.phase !== "commit") pendingCommitBarrier = null;
      onProgress(progress);
    }
  };
  window.addEventListener(NATIVE_BACK_PROGRESS_EVENT, handleProgress);
  return () => {
    window.removeEventListener(
      NATIVE_BACK_PROGRESS_EVENT,
      handleProgress,
    );
  };
}

export function deferNativeBackCommit(
  visualCompletion: Promise<void>,
): void {
  pendingCommitBarrier = Promise.race([
    visualCompletion.catch(() => undefined),
    new Promise<void>((resolve) => {
      window.setTimeout(resolve, 320);
    }),
  ]);
}

async function waitForNativeBackVisuals(): Promise<void> {
  const barrier = pendingCommitBarrier;
  pendingCommitBarrier = null;
  await barrier;
}

export function connectNativeBackCommit(
  onBack: () => void,
): () => void {
  if (typeof window.addEventListener !== "function") {
    return () => undefined;
  }
  const handleCommit = (): void => {
    void waitForNativeBackVisuals().then(onBack);
  };
  window.addEventListener(COMMIT_EVENT, handleCommit);
  return () => {
    window.removeEventListener(COMMIT_EVENT, handleCommit);
  };
}

async function callNativeBack(
  command: string,
  args?: Record<string, unknown>,
): Promise<NativeBackStatus> {
  if (!isTauri()) return UNSUPPORTED_STATUS;
  try {
    return normalizeNativeBackStatus(
      await invoke<unknown>(`${COMMAND_PREFIX}${command}`, args),
    );
  } catch {
    return UNSUPPORTED_STATUS;
  }
}

export function completeNativeBack(): Promise<NativeBackStatus> {
  return callNativeBack("complete");
}

export function prepareNativeBack(): Promise<NativeBackStatus> {
  return callNativeBack("prepare");
}

export function requestNativeBackPop(): Promise<NativeBackStatus> {
  return callNativeBack("pop");
}

export function setNativeBackEnabled(
  enabled: boolean,
): Promise<NativeBackStatus> {
  return callNativeBack("set_enabled", {
    payload: { enabled },
  });
}

export async function connectNativeBack(
  onBack: () => void,
): Promise<NativeBackConnection> {
  if (!isTauri()) {
    return {
      status: UNSUPPORTED_STATUS,
      disconnect: () => undefined,
    };
  }

  const disconnectCommit = connectNativeBackCommit(onBack);

  try {
    const status = await setNativeBackEnabled(true);
    return {
      status,
      disconnect: () => {
        disconnectCommit();
        void setNativeBackEnabled(false);
      },
    };
  } catch {
    disconnectCommit();
    return {
      status: UNSUPPORTED_STATUS,
      disconnect: () => undefined,
    };
  }
}
