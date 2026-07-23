export interface LogicalSizePreset {
  id: string;
  label: string;
  hint: string;
  width: number;
  height: number;
}

export const DEV_SIZE_LIMITS = {
  minWidth: 280,
  maxWidth: 1600,
  minHeight: 500,
  maxHeight: 1200,
} as const;

export const DEV_SIZE_PRESETS: readonly LogicalSizePreset[] = [
  {
    id: "compact",
    label: "Compact",
    hint: "좁은 폭 점검",
    width: 320,
    height: 700,
  },
  {
    id: "s25",
    label: "S25 작업폭",
    hint: "현재 기준",
    width: 360,
    height: 780,
  },
  {
    id: "ios",
    label: "iOS 작업폭",
    hint: "일반 iPhone 폭",
    width: 390,
    height: 844,
  },
  {
    id: "large-phone",
    label: "Large phone",
    hint: "큰 모바일 폭",
    width: 430,
    height: 932,
  },
  {
    id: "desktop",
    label: "Desktop",
    hint: "데스크톱 분기",
    width: 1280,
    height: 800,
  },
] as const;

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.min(maximum, Math.max(minimum, value));
}

export function normalizeLogicalSize(
  width: number,
  height: number,
): { width: number; height: number } {
  const fallback = DEV_SIZE_PRESETS[1];
  const finiteWidth = Number.isFinite(width) ? width : fallback.width;
  const finiteHeight = Number.isFinite(height) ? height : fallback.height;

  return {
    width: clamp(
      Math.round(finiteWidth),
      DEV_SIZE_LIMITS.minWidth,
      DEV_SIZE_LIMITS.maxWidth,
    ),
    height: clamp(
      Math.round(finiteHeight),
      DEV_SIZE_LIMITS.minHeight,
      DEV_SIZE_LIMITS.maxHeight,
    ),
  };
}
