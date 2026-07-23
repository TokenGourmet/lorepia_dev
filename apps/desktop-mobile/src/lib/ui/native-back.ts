import {
  invoke,
  isTauri,
} from "@tauri-apps/api/core";

const PLUGIN_NAME = "native-back";
const COMMAND_PREFIX = `plugin:${PLUGIN_NAME}|`;
const COMMIT_EVENT = "lorepia:native-back";

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

  const handleCommit = (): void => {
    onBack();
  };
  window.addEventListener(COMMIT_EVENT, handleCommit);

  try {
    const status = await setNativeBackEnabled(true);
    return {
      status,
      disconnect: () => {
        window.removeEventListener(COMMIT_EVENT, handleCommit);
        void setNativeBackEnabled(false);
      },
    };
  } catch {
    window.removeEventListener(COMMIT_EVENT, handleCommit);
    return {
      status: UNSUPPORTED_STATUS,
      disconnect: () => undefined,
    };
  }
}
