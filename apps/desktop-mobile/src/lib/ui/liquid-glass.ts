export type LiquidGlassVariant = "regular" | "chrome" | "panel";
export type LiquidGlassShape = "rounded" | "pill" | "inherit";

export interface LiquidGlassPoint {
  x: number;
  y: number;
}

export interface LiquidGlassSize {
  width: number;
  height: number;
}

export interface LiquidGlassInteractionState {
  pressed: boolean;
  hovered: boolean;
  focused: boolean;
}

export interface LiquidGlassRippleFrame {
  progress: number;
  eased: number;
  opacity: number;
  active: boolean;
}

export function clampUnit(value: number): number {
  return Math.min(1, Math.max(0, value));
}

export function clampGlassPoint(
  point: LiquidGlassPoint,
  size: LiquidGlassSize,
): LiquidGlassPoint {
  return {
    x: Math.min(Math.max(point.x, 0), Math.max(size.width, 1)),
    y: Math.min(Math.max(point.y, 0), Math.max(size.height, 1)),
  };
}

export function liquidGlowIntensity({
  pressed,
  hovered,
  focused,
}: LiquidGlassInteractionState): number {
  if (pressed) return 1;
  if (hovered) return 0.56;
  if (focused) return 0.38;
  return 0;
}

export function approachValue(
  current: number,
  target: number,
  amount: number,
): number {
  return current + (target - current) * clampUnit(amount);
}

export function liquidRippleFrame(
  elapsedMs: number,
  durationMs = 460,
): LiquidGlassRippleFrame {
  const safeDuration = Math.max(1, durationMs);
  const progress = clampUnit(elapsedMs / safeDuration);
  const eased = 1 - Math.pow(1 - progress, 3);

  return {
    progress,
    eased,
    opacity: (1 - progress) * 0.22,
    active: progress < 1,
  };
}
