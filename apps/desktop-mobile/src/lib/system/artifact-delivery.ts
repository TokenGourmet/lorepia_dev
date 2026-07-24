import type { SafetyArtifact } from "./native-support";

export const ARTIFACT_DELIVERY_ERROR_MESSAGE =
  "이 환경에서 파일 전달을 확인하지 못했습니다. JSON 복사를 사용해 주세요.";
export const ARTIFACT_COPY_ERROR_MESSAGE = "JSON을 복사하지 못했습니다.";

const ALLOWED_MEDIA_TYPES = new Set([
  "application/vnd.lorepia.ai-output-report+json",
  "application/vnd.lorepia.diagnostics+json",
]);

export type ArtifactDeliveryMethod = "shared" | "download-requested";

export type ArtifactDeliveryPlatform = Readonly<{
  share?: (artifact: SafetyArtifact) => Promise<boolean>;
  download: (artifact: SafetyArtifact) => Promise<boolean> | boolean;
  copy: (text: string) => Promise<void>;
}>;

function validateArtifact(artifact: SafetyArtifact): void {
  if (
    !/^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/u.test(artifact.fileName) ||
    !ALLOWED_MEDIA_TYPES.has(artifact.mediaType) ||
    !Number.isSafeInteger(artifact.byteLength) ||
    artifact.byteLength < 1 ||
    new TextEncoder().encode(artifact.json).byteLength !== artifact.byteLength
  ) {
    throw new Error("INVALID_ARTIFACT_DELIVERY_INPUT");
  }
  try {
    JSON.parse(artifact.json);
  } catch {
    throw new Error("INVALID_ARTIFACT_DELIVERY_INPUT");
  }
}

export function canRequestBrowserDownload(
  userAgent: string,
  hasTauriInternals: boolean,
): boolean {
  return !(
    /\bAndroid\b/iu.test(userAgent) &&
    (/\bwv\b/iu.test(userAgent) || hasTauriInternals)
  );
}

function browserDownloadCanBeRequested(): boolean {
  return (
    typeof navigator !== "undefined" &&
    canRequestBrowserDownload(
      navigator.userAgent,
      typeof window !== "undefined" && "__TAURI_INTERNALS__" in window,
    )
  );
}

function createBrowserPlatform(): ArtifactDeliveryPlatform {
  return Object.freeze({
    async share(artifact: SafetyArtifact): Promise<boolean> {
      if (
        typeof navigator === "undefined" ||
        typeof navigator.share !== "function" ||
        typeof navigator.canShare !== "function" ||
        typeof File === "undefined"
      ) {
        return false;
      }
      const file = new File([artifact.json], artifact.fileName, {
        type: artifact.mediaType,
      });
      const data = { files: [file] };
      if (!navigator.canShare(data)) return false;
      await navigator.share(data);
      return true;
    },
    download(artifact: SafetyArtifact): boolean {
      if (
        !browserDownloadCanBeRequested() ||
        typeof document === "undefined" ||
        document.body === null ||
        typeof URL.createObjectURL !== "function"
      ) {
        return false;
      }
      const blob = new Blob([artifact.json], { type: artifact.mediaType });
      const url = URL.createObjectURL(blob);
      const anchor = document.createElement("a");
      if (!("download" in anchor)) {
        URL.revokeObjectURL(url);
        return false;
      }
      anchor.href = url;
      anchor.download = artifact.fileName;
      anchor.rel = "noopener";
      anchor.hidden = true;
      document.body.append(anchor);
      try {
        anchor.click();
      } finally {
        anchor.remove();
        setTimeout(() => URL.revokeObjectURL(url), 0);
      }
      return true;
    },
    async copy(text: string): Promise<void> {
      if (
        typeof navigator !== "undefined" &&
        navigator.clipboard &&
        typeof navigator.clipboard.writeText === "function"
      ) {
        await navigator.clipboard.writeText(text);
        return;
      }
      if (
        typeof document === "undefined" ||
        document.body === null ||
        typeof document.execCommand !== "function"
      ) {
        throw new Error("ARTIFACT_COPY_UNAVAILABLE");
      }
      const textarea = document.createElement("textarea");
      textarea.value = text;
      textarea.readOnly = true;
      textarea.setAttribute("aria-hidden", "true");
      textarea.style.position = "fixed";
      textarea.style.opacity = "0";
      textarea.style.pointerEvents = "none";
      document.body.append(textarea);
      textarea.select();
      try {
        if (!document.execCommand("copy")) {
          throw new Error("ARTIFACT_COPY_UNAVAILABLE");
        }
      } finally {
        textarea.remove();
      }
    },
  });
}

export async function deliverSafetyArtifact(
  artifact: SafetyArtifact,
  platform: ArtifactDeliveryPlatform = createBrowserPlatform(),
): Promise<ArtifactDeliveryMethod> {
  validateArtifact(artifact);
  if (platform.share && (await platform.share(artifact))) {
    return "shared";
  }
  if (!(await platform.download(artifact))) {
    throw new Error("ARTIFACT_DOWNLOAD_UNAVAILABLE");
  }
  return "download-requested";
}

export async function copySafetyArtifactJson(
  artifact: SafetyArtifact,
  platform: ArtifactDeliveryPlatform = createBrowserPlatform(),
): Promise<void> {
  validateArtifact(artifact);
  await platform.copy(artifact.json);
}

export function publicArtifactDeliveryError(): string {
  return ARTIFACT_DELIVERY_ERROR_MESSAGE;
}

export function publicArtifactCopyError(): string {
  return ARTIFACT_COPY_ERROR_MESSAGE;
}
