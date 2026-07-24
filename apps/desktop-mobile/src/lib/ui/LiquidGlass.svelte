<script lang="ts">
  import { onMount, type Snippet } from "svelte";

  import {
    approachValue,
    clampGlassPoint,
    liquidGlowIntensity,
    liquidRippleFrame,
    type LiquidGlassPoint,
    type LiquidGlassShape,
    type LiquidGlassVariant,
  } from "./liquid-glass";

  let {
    children,
    variant = "regular",
    shape = "rounded",
    interactive = false,
    materialize = true,
  }: {
    children?: Snippet;
    variant?: LiquidGlassVariant;
    shape?: LiquidGlassShape;
    interactive?: boolean;
    materialize?: boolean;
  } = $props();

  let host = $state<HTMLElement | null>(null);
  let glowCanvas = $state<HTMLCanvasElement | null>(null);
  let pressed = $state(false);
  let hovered = $state(false);
  let focused = $state(false);

  let context: CanvasRenderingContext2D | null = null;
  let animationFrame = 0;
  let cssWidth = 1;
  let cssHeight = 1;
  let pixelRatio = 1;
  let reduceMotion = false;
  let hasPointerPosition = false;

  let targetPoint: LiquidGlassPoint = { x: 0.5, y: 0.25 };
  let currentPoint: LiquidGlassPoint = { x: 0.5, y: 0.25 };
  let currentIntensity = 0;
  let rippleOrigin: LiquidGlassPoint = { x: 0.5, y: 0.5 };
  let rippleStartedAt: number | null = null;

  function scheduleFrame(): void {
    if (animationFrame !== 0) return;
    animationFrame = requestAnimationFrame(drawFrame);
  }

  function resizeCanvas(): void {
    if (host === null || glowCanvas === null) return;

    const bounds = host.getBoundingClientRect();
    cssWidth = Math.max(1, bounds.width);
    cssHeight = Math.max(1, bounds.height);
    pixelRatio = Math.min(Math.max(window.devicePixelRatio || 1, 1), 2);

    const width = Math.max(1, Math.round(cssWidth * pixelRatio));
    const height = Math.max(1, Math.round(cssHeight * pixelRatio));
    if (glowCanvas.width !== width) glowCanvas.width = width;
    if (glowCanvas.height !== height) glowCanvas.height = height;

    context = glowCanvas.getContext("2d", { alpha: true });
    if (!hasPointerPosition) {
      const restingPoint = { x: cssWidth * 0.5, y: cssHeight * 0.24 };
      targetPoint = restingPoint;
      currentPoint = restingPoint;
    } else {
      targetPoint = clampGlassPoint(targetPoint, {
        width: cssWidth,
        height: cssHeight,
      });
      currentPoint = clampGlassPoint(currentPoint, {
        width: cssWidth,
        height: cssHeight,
      });
    }
    scheduleFrame();
  }

  function localPointer(event: PointerEvent): LiquidGlassPoint {
    if (host === null) return targetPoint;
    const bounds = host.getBoundingClientRect();
    return clampGlassPoint(
      {
        x: event.clientX - bounds.left,
        y: event.clientY - bounds.top,
      },
      { width: bounds.width, height: bounds.height },
    );
  }

  function updatePointer(event: PointerEvent): void {
    if (!interactive) return;
    hasPointerPosition = true;
    targetPoint = localPointer(event);
    scheduleFrame();
  }

  function handlePointerEnter(event: PointerEvent): void {
    if (!interactive) return;
    if (event.pointerType !== "touch") hovered = true;
    updatePointer(event);
  }

  function handlePointerMove(event: PointerEvent): void {
    updatePointer(event);
  }

  function handlePointerLeave(): void {
    hovered = false;
    scheduleFrame();
  }

  function handlePointerDown(event: PointerEvent): void {
    if (!interactive || (event.pointerType === "mouse" && event.button !== 0)) {
      return;
    }

    pressed = true;
    updatePointer(event);
    rippleOrigin = targetPoint;
    rippleStartedAt = reduceMotion ? null : performance.now();
    scheduleFrame();
  }

  function releasePointer(): void {
    if (!pressed) return;
    pressed = false;
    scheduleFrame();
  }

  function handleFocusIn(): void {
    if (!interactive) return;
    focused = true;
    if (!hasPointerPosition) {
      targetPoint = { x: cssWidth * 0.5, y: cssHeight * 0.28 };
    }
    scheduleFrame();
  }

  function handleFocusOut(event: FocusEvent): void {
    const next = event.relatedTarget;
    if (next instanceof Node && host?.contains(next)) return;
    focused = false;
    scheduleFrame();
  }

  function drawGlow(
    drawingContext: CanvasRenderingContext2D,
    intensity: number,
  ): void {
    if (intensity <= 0.002) return;

    const radius = Math.max(72, Math.max(cssWidth, cssHeight) * 0.72);
    const gradient = drawingContext.createRadialGradient(
      currentPoint.x,
      currentPoint.y,
      0,
      currentPoint.x,
      currentPoint.y,
      radius,
    );
    gradient.addColorStop(0, `rgba(255, 255, 255, ${0.24 * intensity})`);
    gradient.addColorStop(0.24, `rgba(255, 255, 255, ${0.105 * intensity})`);
    gradient.addColorStop(0.62, `rgba(255, 255, 255, ${0.028 * intensity})`);
    gradient.addColorStop(1, "rgba(255, 255, 255, 0)");

    drawingContext.fillStyle = gradient;
    drawingContext.fillRect(0, 0, cssWidth, cssHeight);
  }

  function drawRipple(
    drawingContext: CanvasRenderingContext2D,
    now: number,
  ): boolean {
    if (rippleStartedAt === null) return false;

    const frame = liquidRippleFrame(now - rippleStartedAt);
    const radius = 8 + frame.eased * Math.max(cssWidth, cssHeight) * 0.82;

    drawingContext.beginPath();
    drawingContext.arc(rippleOrigin.x, rippleOrigin.y, radius, 0, Math.PI * 2);
    drawingContext.strokeStyle = `rgba(255, 255, 255, ${frame.opacity})`;
    drawingContext.lineWidth = 1.25;
    drawingContext.stroke();

    if (!frame.active) rippleStartedAt = null;
    return frame.active;
  }

  function drawFrame(now: number): void {
    animationFrame = 0;
    const drawingContext = context;
    if (drawingContext === null) return;

    const targetIntensity = liquidGlowIntensity({
      pressed,
      hovered,
      focused,
    });
    const response = reduceMotion ? 1 : pressed ? 0.34 : 0.17;

    currentPoint = {
      x: approachValue(currentPoint.x, targetPoint.x, response),
      y: approachValue(currentPoint.y, targetPoint.y, response),
    };
    currentIntensity = approachValue(currentIntensity, targetIntensity, response);

    drawingContext.setTransform(pixelRatio, 0, 0, pixelRatio, 0, 0);
    drawingContext.clearRect(0, 0, cssWidth, cssHeight);
    drawGlow(drawingContext, currentIntensity);
    const rippleActive = drawRipple(drawingContext, now);

    const unsettled =
      Math.abs(currentPoint.x - targetPoint.x) > 0.2 ||
      Math.abs(currentPoint.y - targetPoint.y) > 0.2 ||
      Math.abs(currentIntensity - targetIntensity) > 0.004;
    if (unsettled || rippleActive) scheduleFrame();
  }

  onMount(() => {
    if (host === null || glowCanvas === null) return;

    const motionQuery = window.matchMedia("(prefers-reduced-motion: reduce)");
    const handleMotionChange = (): void => {
      reduceMotion = motionQuery.matches;
      if (reduceMotion) rippleStartedAt = null;
      scheduleFrame();
    };
    handleMotionChange();

    const resizeObserver = new ResizeObserver(resizeCanvas);
    resizeObserver.observe(host);
    window.addEventListener("resize", resizeCanvas, { passive: true });
    window.addEventListener("pointerup", releasePointer, { passive: true });
    window.addEventListener("pointercancel", releasePointer, { passive: true });
    motionQuery.addEventListener("change", handleMotionChange);
    resizeCanvas();

    return () => {
      resizeObserver.disconnect();
      window.removeEventListener("resize", resizeCanvas);
      window.removeEventListener("pointerup", releasePointer);
      window.removeEventListener("pointercancel", releasePointer);
      motionQuery.removeEventListener("change", handleMotionChange);
      if (animationFrame !== 0) cancelAnimationFrame(animationFrame);
    };
  });
</script>

<div
  bind:this={host}
  class="liquid-glass"
  class:regular={variant === "regular"}
  class:chrome={variant === "chrome"}
  class:panel={variant === "panel"}
  class:shape-rounded={shape === "rounded"}
  class:shape-pill={shape === "pill"}
  class:shape-inherit={shape === "inherit"}
  class:interactive
  class:pressed
  class:hovered
  class:focused
  class:materialize
  data-liquid-glass={variant}
  onpointerenter={handlePointerEnter}
  onpointermove={handlePointerMove}
  onpointerleave={handlePointerLeave}
  onpointerdown={handlePointerDown}
  onpointerup={releasePointer}
  onpointercancel={releasePointer}
  onfocusin={handleFocusIn}
  onfocusout={handleFocusOut}
>
  <canvas bind:this={glowCanvas} class="glow" aria-hidden="true"></canvas>
  <div class="content">
    {#if children}
      {@render children()}
    {/if}
  </div>
</div>

<style>
  .liquid-glass {
    position: relative;
    display: block;
    min-width: 0;
    isolation: isolate;
    overflow: hidden;
    border: 1px solid var(--glass-border);
    background: var(--glass-fill);
    box-shadow:
      var(--glass-shadow),
      inset 0 1px 0 var(--glass-specular),
      inset 0 -1px 0 var(--glass-edge-shade);
    -webkit-backdrop-filter: blur(var(--glass-blur))
      saturate(var(--glass-saturation)) contrast(var(--glass-contrast));
    backdrop-filter: blur(var(--glass-blur)) saturate(var(--glass-saturation))
      contrast(var(--glass-contrast));
    transform: translate3d(0, 0, 0) scale(1);
    transform-origin: center;
    transition:
      transform 420ms var(--ease-liquid),
      border-color var(--dur-base) var(--ease-out),
      box-shadow 420ms var(--ease-liquid),
      background-color var(--dur-base) var(--ease-out);
  }

  .liquid-glass::before {
    content: "";
    position: absolute;
    inset: 0;
    z-index: 0;
    pointer-events: none;
    border-radius: inherit;
    background:
      linear-gradient(
        145deg,
        var(--glass-specular) 0%,
        transparent 28%,
        transparent 67%,
        var(--glass-edge-shade) 100%
      ),
      linear-gradient(
        90deg,
        transparent 0%,
        var(--glass-side-light) 48%,
        transparent 100%
      );
    opacity: 0.72;
  }

  .regular {
    background: var(--glass-fill);
  }

  .chrome {
    background: var(--glass-chrome-fill);
  }

  .panel {
    background: var(--glass-panel-fill);
    --glass-blur: 24px;
  }

  .shape-rounded {
    border-radius: var(--glass-radius);
  }

  .shape-pill {
    border-radius: var(--r-pill);
  }

  .shape-inherit {
    border-radius: inherit;
  }

  .interactive {
    will-change: transform;
  }

  .interactive.hovered:not(.pressed) {
    transform: translate3d(0, -0.5px, 0) scale(1.002);
  }

  .interactive.pressed {
    transform: translate3d(0, 1px, 0) scaleX(0.992) scaleY(0.982);
    box-shadow:
      var(--glass-shadow-pressed),
      inset 0 1px 0 var(--glass-specular),
      inset 0 -1px 0 var(--glass-edge-shade);
    transition-duration: 86ms;
  }

  .interactive.focused {
    border-color: var(--glass-focus-border);
  }

  .materialize {
    animation: liquid-glass-materialize 480ms var(--ease-liquid);
  }

  .glow {
    position: absolute;
    inset: 0;
    z-index: 1;
    display: block;
    width: 100%;
    height: 100%;
    pointer-events: none;
    mix-blend-mode: screen;
  }

  .content {
    position: relative;
    z-index: 2;
    min-width: 0;
    min-height: 0;
  }

  @keyframes liquid-glass-materialize {
    0% {
      opacity: 0;
      transform: translate3d(0, 5px, 0) scale(0.965);
      filter: saturate(0.82) contrast(0.96);
    }

    58% {
      opacity: 1;
      transform: translate3d(0, -1px, 0) scale(1.006);
      filter: saturate(1.06) contrast(1.02);
    }

    100% {
      opacity: 1;
      transform: translate3d(0, 0, 0) scale(1);
      filter: none;
    }
  }

  @supports not (
    (backdrop-filter: blur(1px)) or (-webkit-backdrop-filter: blur(1px))
  ) {
    .liquid-glass {
      background: var(--glass-fallback-fill);
    }
  }

  @media (prefers-contrast: more) {
    .liquid-glass {
      border-color: var(--text-mid);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .liquid-glass,
    .interactive.pressed,
    .interactive.hovered:not(.pressed) {
      animation: none;
      transition: none;
      transform: none;
    }
  }
</style>
