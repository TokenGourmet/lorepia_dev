import {
  connectNativeBackProgress,
  deferNativeBackCommit,
  type NativeBackProgress,
} from "./native-back";

const AXIS_SLOP = 8;
const COMMIT_FRACTION = 0.35;
const FLICK_DISTANCE = 24;
const FLICK_VELOCITY = 0.5;
const VELOCITY_WINDOW_MS = 100;
const CANCEL_MS = 240;
const COMPLETE_MS = 220;
const WHEEL_RELEASE_MS = 90;
const RELEASE_EASE = "cubic-bezier(0.22, 1, 0.36, 1)";
const BACK_CORNER_RADIUS = 26;

const BLOCKING_SELECTOR = [
  "a",
  "button",
  "input",
  "textarea",
  "select",
  "option",
  "label",
  "[contenteditable]:not([contenteditable='false'])",
  "[role='button']",
  "[role='link']",
  "[role='slider']",
  "[role='tab']",
  "[data-back-swipe-block]",
].join(",");

export type ContentBackIntent = "pending" | "accept" | "reject";

export interface ContentBackSample {
  x: number;
  time: number;
}

export interface ContentBackVisualState {
  progress: number;
  foregroundXPercent: number;
  cornerRadius: number;
  underlayXPercent: number;
  underlayScale: number;
  shade: number;
}

export type ContentBackDirection = -1 | 1;

export interface ContentBackOptions {
  onBack: () => void;
  getUnderlay?: () => HTMLElement | null;
  enabled?: boolean;
  edgeWidth?: number;
}

interface StoredVisualStyles {
  element: HTMLElement;
  translate: string;
  transition: string;
  willChange: string;
  userSelect: string;
  state: string | null;
  progress: string;
  x: string;
  radius: string;
  underlayX: string;
  underlayScale: string;
  shade: string;
  shadowX: string;
  originX: string;
}

type GesturePhase =
  | "idle"
  | "possible"
  | "dragging"
  | "cancelling"
  | "committing";

export function classifyContentBackIntent(
  dx: number,
  dy: number,
): ContentBackIntent {
  const horizontal = Math.abs(dx);
  const vertical = Math.abs(dy);
  if (horizontal < AXIS_SLOP && vertical < AXIS_SLOP) {
    return "pending";
  }
  if (dx <= 0 || horizontal <= vertical) {
    return "reject";
  }
  return "accept";
}

export function shouldCommitContentBack(
  distance: number,
  width: number,
  velocity: number,
): boolean {
  if (width <= 0) return false;
  return (
    distance > width * COMMIT_FRACTION ||
    (distance > FLICK_DISTANCE && velocity > FLICK_VELOCITY)
  );
}

export function needsExplicitContentBackCapture(
  pointerType: string,
): boolean {
  // Touch and pen pointers already receive implicit capture. Android WebView
  // cancels the stream if setPointerCapture() is redundantly called mid-pan.
  return pointerType !== "touch" && pointerType !== "pen";
}

export function contentBackReleaseVelocity(
  samples: readonly ContentBackSample[],
  releaseTime: number,
): number {
  const recent = samples.filter(
    (sample) =>
      sample.time <= releaseTime &&
      releaseTime - sample.time <= VELOCITY_WINDOW_MS,
  );
  if (recent.length < 2) return 0;
  const first = recent[0];
  const last = recent[recent.length - 1];
  const elapsed = last.time - first.time;
  return elapsed > 0 ? (last.x - first.x) / elapsed : 0;
}

export function contentBackVisualState(
  rawProgress: number,
  direction: ContentBackDirection = 1,
): ContentBackVisualState {
  const progress = Math.min(Math.max(rawProgress, 0), 1);
  return {
    progress,
    foregroundXPercent: direction * progress * 100,
    cornerRadius:
      BACK_CORNER_RADIUS * Math.min(progress * 3, 1),
    underlayXPercent:
      progress === 1 ? 0 : direction * -7 * (1 - progress),
    underlayScale: 0.965 + progress * 0.035,
    shade: 0.14 * (1 - progress),
  };
}

export function contentBackWheelDelta(deltaX: number): number {
  // Trackpads report content-scroll direction. A two-finger motion to the
  // right (back) therefore arrives as a negative horizontal wheel delta.
  return -deltaX;
}

export function renderedContentBackDistance(
  currentLeft: number,
  originLeft: number,
  direction: ContentBackDirection,
  fallback: number,
): number {
  const rendered = (currentLeft - originLeft) * direction;
  return Number.isFinite(rendered)
    ? Math.max(rendered, 0)
    : fallback;
}

export function shouldApplyNativeBackProgress(
  phase: NativeBackProgress["phase"],
): boolean {
  return phase === "start" || phase === "progress";
}

function clampDistance(distance: number, width: number): number {
  return Math.min(Math.max(distance, 0), Math.max(width, 0));
}

function targetElement(target: EventTarget | null): Element | null {
  if (target instanceof Element) return target;
  if (target instanceof Node) return target.parentElement;
  return null;
}

function startsOnBlockedContent(
  target: EventTarget | null,
  gestureRegion: HTMLElement,
): boolean {
  const origin = targetElement(target);
  if (origin === null) return true;

  const explicitBlocker = origin.closest(BLOCKING_SELECTOR);
  if (
    explicitBlocker !== null &&
    gestureRegion.contains(explicitBlocker)
  ) {
    return true;
  }

  for (
    let current: Element | null = origin;
    current !== null && current !== gestureRegion;
    current = current.parentElement
  ) {
    const style = getComputedStyle(current);
    const horizontallyScrollable =
      (style.overflowX === "auto" || style.overflowX === "scroll") &&
      current.scrollWidth > current.clientWidth + 1;
    if (horizontallyScrollable) return true;
  }

  return false;
}

function storeVisualStyles(element: HTMLElement): StoredVisualStyles {
  return {
    element,
    translate: element.style.translate,
    transition: element.style.transition,
    willChange: element.style.willChange,
    userSelect: element.style.userSelect,
    state: element.getAttribute("data-back-transition-state"),
    progress: element.style.getPropertyValue("--back-transition-progress"),
    x: element.style.getPropertyValue("--back-transition-x"),
    radius: element.style.getPropertyValue("--back-transition-radius"),
    underlayX: element.style.getPropertyValue(
      "--back-transition-underlay-x",
    ),
    underlayScale: element.style.getPropertyValue(
      "--back-transition-underlay-scale",
    ),
    shade: element.style.getPropertyValue("--back-swipe-shade"),
    shadowX: element.style.getPropertyValue(
      "--back-transition-shadow-x",
    ),
    originX: element.style.getPropertyValue(
      "--back-transition-origin-x",
    ),
  };
}

function restoreProperty(
  element: HTMLElement,
  name: string,
  value: string,
): void {
  if (value === "") {
    element.style.removeProperty(name);
  } else {
    element.style.setProperty(name, value);
  }
}

function restoreVisualStyles(stored: StoredVisualStyles | null): void {
  if (stored === null) return;
  const { element } = stored;
  element.style.translate = stored.translate;
  element.style.transition = stored.transition;
  element.style.willChange = stored.willChange;
  element.style.userSelect = stored.userSelect;
  if (stored.state === null) {
    element.removeAttribute("data-back-transition-state");
  } else {
    element.setAttribute("data-back-transition-state", stored.state);
  }
  restoreProperty(
    element,
    "--back-transition-progress",
    stored.progress,
  );
  restoreProperty(element, "--back-transition-x", stored.x);
  restoreProperty(element, "--back-transition-radius", stored.radius);
  restoreProperty(
    element,
    "--back-transition-underlay-x",
    stored.underlayX,
  );
  restoreProperty(
    element,
    "--back-transition-underlay-scale",
    stored.underlayScale,
  );
  restoreProperty(element, "--back-swipe-shade", stored.shade);
  restoreProperty(
    element,
    "--back-transition-shadow-x",
    stored.shadowX,
  );
  restoreProperty(
    element,
    "--back-transition-origin-x",
    stored.originX,
  );
}

/**
 * iOS 26-style content backswipe for a vertically scrolling content region.
 * The gesture can begin anywhere noninteractive in the region; the complete
 * page follows the pointer and reveals a non-raster, inert DOM visual clone of
 * the prior route. Native Android progress and desktop pointer input both flow
 * through the same CSS-variable contract.
 */
export function contentSwipeBack(
  node: HTMLElement,
  initialOptions: ContentBackOptions,
): { update(options: ContentBackOptions): void; destroy(): void } {
  let options = initialOptions;
  let phase: GesturePhase = "idle";
  let pointerId: number | null = null;
  let touchId: number | null = null;
  let startX = 0;
  let startY = 0;
  let distance = 0;
  let dragOriginDistance = 0;
  let samples: ContentBackSample[] = [];
  let foreground: HTMLElement | null = null;
  let underlay: HTMLElement | null = null;
  let foregroundStyles: StoredVisualStyles | null = null;
  let underlayStyles: StoredVisualStyles | null = null;
  let animationFrame: number | null = null;
  let releaseTimer: ReturnType<typeof setTimeout> | null = null;
  let wheelReleaseTimer: ReturnType<typeof setTimeout> | null = null;
  let releaseResolve: (() => void) | null = null;
  let interruptedReleaseTarget: 0 | 1 | null = null;
  let wheelPendingDistance = 0;
  let touchUserSelect: string | null = null;
  let destroyed = false;
  let listenersAttached = false;
  let visualDirection: ContentBackDirection = 1;
  let foregroundOriginLeft = 0;

  const previousTouchAction = node.style.touchAction;

  const reducedMotion = window.matchMedia(
    "(prefers-reduced-motion: reduce)",
  );

  function visualWidth(): number {
    return Math.max(
      foreground?.clientWidth ?? 0,
      node.getBoundingClientRect().width,
      1,
    );
  }

  function appendSample(x: number, time: number): void {
    samples.push({ x, time });
    const cutoff = time - VELOCITY_WINDOW_MS;
    while (samples.length > 2 && samples[1].time < cutoff) {
      samples.shift();
    }
  }

  function sampleEvent(event: PointerEvent): void {
    const coalesced = event.getCoalescedEvents?.() ?? [];
    for (const sample of coalesced) {
      appendSample(sample.clientX, sample.timeStamp);
    }
    const last = coalesced[coalesced.length - 1];
    if (
      last === undefined ||
      last.clientX !== event.clientX ||
      last.timeStamp !== event.timeStamp
    ) {
      appendSample(event.clientX, event.timeStamp);
    }
  }

  function beginVisuals(): void {
    if (foreground !== null) {
      foreground.setAttribute(
        "data-back-transition-state",
        "interactive",
      );
      underlay?.setAttribute(
        "data-back-transition-state",
        "interactive",
      );
      return;
    }
    foreground =
      node.closest<HTMLElement>("[data-back-swipe-foreground]") ?? node;
    underlay = options.getUnderlay?.() ?? null;
    foregroundStyles = storeVisualStyles(foreground);
    foregroundOriginLeft = foreground.getBoundingClientRect().left;
    if (foreground === node && touchUserSelect !== null) {
      foregroundStyles.userSelect = touchUserSelect;
    }
    underlayStyles =
      underlay === null ? null : storeVisualStyles(underlay);

    foreground.style.willChange = "translate, border-radius";
    foreground.style.userSelect = "none";
    foreground.setAttribute(
      "data-back-transition-state",
      "interactive",
    );
    if (underlay !== null) {
      underlay.style.willChange = "translate, scale";
      underlay.setAttribute(
        "data-back-transition-state",
        "interactive",
      );
    }
    applyVisuals(distance);
  }

  function applyVisuals(nextDistance: number): void {
    if (foreground === null) return;
    const width = visualWidth();
    distance = clampDistance(nextDistance, width);
    const visual = contentBackVisualState(
      distance / width,
      visualDirection,
    );
    foreground.style.setProperty(
      "--back-transition-progress",
      `${visual.progress}`,
    );
    foreground.style.setProperty(
      "--back-transition-x",
      `${visual.foregroundXPercent}%`,
    );
    foreground.style.setProperty(
      "--back-transition-radius",
      `${visual.cornerRadius}px`,
    );
    foreground.style.setProperty(
      "--back-transition-shadow-x",
      `${visualDirection * -18}px`,
    );

    if (underlay !== null) {
      underlay.style.setProperty(
        "--back-transition-progress",
        `${visual.progress}`,
      );
      underlay.style.setProperty(
        "--back-transition-underlay-x",
        `${visual.underlayXPercent}%`,
      );
      underlay.style.setProperty(
        "--back-transition-underlay-scale",
        `${visual.underlayScale}`,
      );
      underlay.style.setProperty(
        "--back-swipe-shade",
        `${visual.shade}`,
      );
      underlay.style.setProperty(
        "--back-transition-origin-x",
        visualDirection === 1 ? "0%" : "100%",
      );
    }
  }

  function scheduleVisuals(nextDistance: number): void {
    distance = nextDistance;
    if (animationFrame !== null) return;
    animationFrame = requestAnimationFrame(() => {
      animationFrame = null;
      applyVisuals(distance);
    });
  }

  function flushVisuals(): void {
    if (animationFrame !== null) {
      cancelAnimationFrame(animationFrame);
      animationFrame = null;
    }
    applyVisuals(distance);
  }

  function clearTimer(): void {
    if (releaseTimer !== null) {
      clearTimeout(releaseTimer);
      releaseTimer = null;
    }
    releaseResolve?.();
    releaseResolve = null;
  }

  function clearWheelTimer(): void {
    if (wheelReleaseTimer !== null) {
      clearTimeout(wheelReleaseTimer);
      wheelReleaseTimer = null;
    }
  }

  function clearVisuals(): void {
    if (animationFrame !== null) {
      cancelAnimationFrame(animationFrame);
      animationFrame = null;
    }
    restoreVisualStyles(foregroundStyles);
    restoreVisualStyles(underlayStyles);
    foreground = null;
    underlay = null;
    foregroundStyles = null;
    underlayStyles = null;
    distance = 0;
    dragOriginDistance = 0;
    interruptedReleaseTarget = null;
    visualDirection = 1;
    foregroundOriginLeft = 0;
    restoreTouchSelection();
  }

  function suppressTouchSelection(): void {
    if (touchUserSelect !== null) return;
    touchUserSelect = node.style.userSelect;
    node.style.userSelect = "none";
  }

  function restoreTouchSelection(): void {
    if (touchUserSelect === null) return;
    node.style.userSelect = touchUserSelect;
    touchUserSelect = null;
  }

  function clearPointer(): void {
    pointerId = null;
    touchId = null;
    startX = 0;
    startY = 0;
    samples = [];
    restoreTouchSelection();
  }

  function animateRelease(
    target: 0 | 1,
    duration: number,
    onComplete: () => void,
  ): Promise<void> {
    if (foreground === null) return Promise.resolve();
    clearTimer();
    foreground.setAttribute("data-back-transition-state", "settling");
    underlay?.setAttribute("data-back-transition-state", "settling");
    foreground.style.transition =
      `translate ${duration}ms ${RELEASE_EASE}, ` +
      `border-radius ${duration}ms ${RELEASE_EASE}, ` +
      `box-shadow ${duration}ms ${RELEASE_EASE}`;
    if (underlay !== null) {
      underlay.style.transition =
        `translate ${duration}ms ${RELEASE_EASE}, ` +
        `scale ${duration}ms ${RELEASE_EASE}, ` +
        `--back-swipe-shade ${duration}ms ${RELEASE_EASE}`;
    }
    applyVisuals(target * visualWidth());

    return new Promise<void>((resolve) => {
      releaseResolve = resolve;
      releaseTimer = setTimeout(() => {
        releaseTimer = null;
        releaseResolve = null;
        if (!destroyed) onComplete();
        resolve();
      }, duration);
    });
  }

  function finishCancel(): void {
    if (phase !== "dragging" || foreground === null) {
      phase = "idle";
      clearPointer();
      return;
    }

    phase = "cancelling";
    const duration = reducedMotion.matches ? 0 : CANCEL_MS;
    clearPointer();
    void animateRelease(0, duration, () => {
      clearVisuals();
      phase = "idle";
    });
  }

  function finishCommit(): void {
    if (phase !== "dragging" || foreground === null) return;
    phase = "committing";
    const duration = reducedMotion.matches ? 0 : COMPLETE_MS;
    clearPointer();
    void animateRelease(1, duration, () => {
      options.onBack();
    });
  }

  function abandonPossibleGesture(): void {
    clearPointer();
    if (
      interruptedReleaseTarget !== null &&
      foreground !== null
    ) {
      phase = "dragging";
      if (interruptedReleaseTarget === 0) {
        finishCancel();
      } else {
        finishCommit();
      }
      interruptedReleaseTarget = null;
      return;
    }
    phase = "idle";
  }

  function interruptRelease(): void {
    if (phase !== "cancelling" && phase !== "committing") return;
    interruptedReleaseTarget = phase === "committing" ? 1 : 0;
    const visualDistance =
      foreground === null
        ? distance
        : renderedContentBackDistance(
            foreground.getBoundingClientRect().left,
            foregroundOriginLeft,
            visualDirection,
            distance,
          );
    clearTimer();
    if (foreground !== null) foreground.style.transition = "none";
    if (underlay !== null) underlay.style.transition = "none";
    applyVisuals(visualDistance);
    foreground?.setAttribute(
      "data-back-transition-state",
      "interactive",
    );
    underlay?.setAttribute(
      "data-back-transition-state",
      "interactive",
    );
    phase = "idle";
  }

  function handleDown(event: PointerEvent): void {
    if (
      (phase !== "idle" &&
        phase !== "cancelling" &&
        phase !== "committing") ||
      pointerId !== null
    ) {
      return;
    }
    if (options.enabled === false) return;
    if (!event.isPrimary) return;
    // Android WebView cancels a touch PointerEvent as soon as its transformed
    // ancestor begins following the pan. Its TouchEvent stream remains intact,
    // so touch input is handled by the dedicated path below.
    if (event.pointerType === "touch") return;
    if (event.pointerType === "mouse" && event.button !== 0) return;
    if (
      options.edgeWidth !== undefined &&
      event.clientX - node.getBoundingClientRect().left >
        options.edgeWidth
    ) {
      return;
    }
    if (startsOnBlockedContent(event.target, node)) return;
    interruptRelease();

    pointerId = event.pointerId;
    startX = event.clientX;
    startY = event.clientY;
    dragOriginDistance = distance;
    samples = [];
    appendSample(event.clientX, event.timeStamp);
    phase = "possible";
  }

  function handleMove(event: PointerEvent): void {
    if (event.pointerId !== pointerId) return;
    const dx = event.clientX - startX;
    const dy = event.clientY - startY;

    if (phase === "possible") {
      const intent = classifyContentBackIntent(dx, dy);
      if (intent === "pending") return;
      if (intent === "reject") {
        abandonPossibleGesture();
        return;
      }

      phase = "dragging";
      beginVisuals();
      if (needsExplicitContentBackCapture(event.pointerType)) {
        try {
          node.setPointerCapture(event.pointerId);
        } catch {
          // Synthetic pointers and an already released pointer can still be
          // followed while they remain over the content region.
        }
      }
    }

    if (phase !== "dragging") return;
    event.preventDefault();
    sampleEvent(event);
    scheduleVisuals(dragOriginDistance + dx);
  }

  function handleUp(event: PointerEvent): void {
    if (event.pointerId !== pointerId) return;
    if (phase === "possible") {
      abandonPossibleGesture();
      return;
    }
    if (phase !== "dragging") return;

    sampleEvent(event);
    distance =
      dragOriginDistance + event.clientX - startX;
    flushVisuals();
    const velocity = contentBackReleaseVelocity(samples, event.timeStamp);
    if (shouldCommitContentBack(distance, visualWidth(), velocity)) {
      finishCommit();
    } else {
      finishCancel();
    }
  }

  function handleCancel(event: PointerEvent): void {
    if (event.pointerId !== pointerId) return;
    if (phase === "dragging") {
      flushVisuals();
      finishCancel();
    } else {
      abandonPossibleGesture();
    }
  }

  function handleLostPointerCapture(event: PointerEvent): void {
    if (event.pointerId !== pointerId || phase !== "dragging") return;
    flushVisuals();
    finishCancel();
  }

  function findTouch(
    touches: TouchList,
    identifier: number,
  ): Touch | null {
    for (let index = 0; index < touches.length; index += 1) {
      const touch = touches.item(index);
      if (touch?.identifier === identifier) return touch;
    }
    return null;
  }

  function handleTouchStart(event: TouchEvent): void {
    if (touchId !== null) {
      if (event.touches.length !== 1) {
        if (phase === "dragging") {
          flushVisuals();
          finishCancel();
        } else {
          abandonPossibleGesture();
        }
      }
      return;
    }
    if (
      (phase !== "idle" &&
        phase !== "cancelling" &&
        phase !== "committing") ||
      pointerId !== null
    ) {
      return;
    }
    if (options.enabled === false || event.touches.length !== 1) return;
    if (startsOnBlockedContent(event.target, node)) return;

    const touch = event.touches.item(0);
    if (touch === null) return;
    if (
      options.edgeWidth !== undefined &&
      touch.clientX - node.getBoundingClientRect().left >
        options.edgeWidth
    ) {
      return;
    }
    interruptRelease();
    suppressTouchSelection();
    touchId = touch.identifier;
    startX = touch.clientX;
    startY = touch.clientY;
    dragOriginDistance = distance;
    samples = [];
    appendSample(touch.clientX, event.timeStamp);
    phase = "possible";
  }

  function handleTouchMove(event: TouchEvent): void {
    if (touchId === null) return;
    if (event.touches.length !== 1) {
      if (phase === "dragging") {
        flushVisuals();
        finishCancel();
      } else {
        abandonPossibleGesture();
      }
      return;
    }

    const touch = findTouch(event.touches, touchId);
    if (touch === null) return;
    const dx = touch.clientX - startX;
    const dy = touch.clientY - startY;

    if (phase === "possible") {
      const intent = classifyContentBackIntent(dx, dy);
      if (intent === "pending") return;
      if (intent === "reject") {
        abandonPossibleGesture();
        return;
      }
      phase = "dragging";
      beginVisuals();
    }

    if (phase !== "dragging") return;
    event.preventDefault();
    appendSample(touch.clientX, event.timeStamp);
    scheduleVisuals(dragOriginDistance + dx);
  }

  function handleTouchEnd(event: TouchEvent): void {
    if (touchId === null) return;
    const touch = findTouch(event.changedTouches, touchId);
    if (touch === null) return;
    if (event.touches.length > 0) {
      if (phase === "dragging") {
        flushVisuals();
        finishCancel();
      } else {
        abandonPossibleGesture();
      }
      return;
    }
    if (phase === "possible") {
      abandonPossibleGesture();
      return;
    }
    if (phase !== "dragging") return;

    appendSample(touch.clientX, event.timeStamp);
    distance = dragOriginDistance + touch.clientX - startX;
    flushVisuals();
    const velocity = contentBackReleaseVelocity(samples, event.timeStamp);
    if (shouldCommitContentBack(distance, visualWidth(), velocity)) {
      finishCommit();
    } else {
      finishCancel();
    }
  }

  function handleTouchCancel(event: TouchEvent): void {
    if (touchId === null) return;
    if (
      event.changedTouches.length > 0 &&
      findTouch(event.changedTouches, touchId) === null
    ) {
      return;
    }
    if (phase === "dragging") {
      flushVisuals();
      finishCancel();
    } else {
      abandonPossibleGesture();
    }
  }

  function finishWheelGesture(releaseTime: number): void {
    wheelReleaseTimer = null;
    wheelPendingDistance = 0;
    if (phase !== "dragging") return;
    flushVisuals();
    const velocity = contentBackReleaseVelocity(samples, releaseTime);
    if (shouldCommitContentBack(distance, visualWidth(), velocity)) {
      finishCommit();
    } else {
      finishCancel();
    }
  }

  function handleWheel(event: WheelEvent): void {
    if (options.enabled === false || event.deltaMode !== 0) {
      return;
    }
    const horizontal = Math.abs(event.deltaX);
    const vertical = Math.abs(event.deltaY);
    if (phase !== "dragging" && horizontal <= vertical) return;
    if (
      phase !== "dragging" &&
      startsOnBlockedContent(event.target, node)
    ) {
      return;
    }

    interruptRelease();
    if (phase !== "idle" && phase !== "dragging") return;

    const backDelta = contentBackWheelDelta(event.deltaX);
    if (phase === "idle") {
      if (backDelta <= 0) {
        wheelPendingDistance = 0;
        return;
      }
      event.preventDefault();
      wheelPendingDistance += backDelta;
      if (wheelPendingDistance <= AXIS_SLOP) {
        clearWheelTimer();
        wheelReleaseTimer = setTimeout(() => {
          wheelReleaseTimer = null;
          wheelPendingDistance = 0;
        }, WHEEL_RELEASE_MS);
        return;
      }
      clearWheelTimer();
      distance = 0;
      dragOriginDistance = 0;
      samples = [];
      beginVisuals();
      phase = "dragging";
      appendSample(0, event.timeStamp);
    }

    event.preventDefault();
    const nextDistance =
      distance +
      (wheelPendingDistance > 0
        ? wheelPendingDistance
        : backDelta);
    wheelPendingDistance = 0;
    scheduleVisuals(nextDistance);
    appendSample(
      clampDistance(nextDistance, visualWidth()),
      event.timeStamp,
    );
    clearWheelTimer();
    wheelReleaseTimer = setTimeout(
      () => finishWheelGesture(event.timeStamp),
      WHEEL_RELEASE_MS,
    );
  }

  function handleNativeProgress(
    nativeProgress: NativeBackProgress,
  ): void {
    if (destroyed) return;
    // Android back dismisses an open modal before navigating. Keep the
    // underlying route stationary while the root commit owner dispatches the
    // dialog's cancellable `cancel` event.
    if (document.querySelector("dialog[open]") !== null) return;

    if (
      phase === "cancelling" ||
      phase === "committing"
    ) {
      interruptRelease();
      interruptedReleaseTarget = null;
    }
    if (pointerId !== null || touchId !== null) {
      clearPointer();
    }
    visualDirection =
      nativeProgress.edge === "right" ? -1 : 1;
    beginVisuals();
    phase = "dragging";

    if (nativeProgress.phase === "cancel") {
      finishCancel();
      return;
    }
    if (nativeProgress.phase === "commit") {
      phase = "committing";
      clearPointer();
      const duration = reducedMotion.matches ? 0 : COMPLETE_MS;
      deferNativeBackCommit(
        animateRelease(1, duration, () => undefined),
      );
      return;
    }
    if (shouldApplyNativeBackProgress(nativeProgress.phase)) {
      applyVisuals(nativeProgress.progress * visualWidth());
    }
  }

  function attachFallbackListeners(): void {
    if (listenersAttached) return;
    listenersAttached = true;
    node.addEventListener("pointerdown", handleDown);
    node.addEventListener("pointermove", handleMove, { passive: false });
    node.addEventListener("pointerup", handleUp);
    node.addEventListener("pointercancel", handleCancel);
    node.addEventListener(
      "lostpointercapture",
      handleLostPointerCapture,
    );
    node.addEventListener("touchstart", handleTouchStart, {
      passive: true,
    });
    node.addEventListener("touchmove", handleTouchMove, {
      passive: false,
    });
    node.addEventListener("touchend", handleTouchEnd, {
      passive: true,
    });
    node.addEventListener("touchcancel", handleTouchCancel, {
      passive: true,
    });
    node.addEventListener("wheel", handleWheel, { passive: false });
  }

  function detachFallbackListeners(): void {
    if (!listenersAttached) return;
    listenersAttached = false;
    node.removeEventListener("pointerdown", handleDown);
    node.removeEventListener("pointermove", handleMove);
    node.removeEventListener("pointerup", handleUp);
    node.removeEventListener("pointercancel", handleCancel);
    node.removeEventListener(
      "lostpointercapture",
      handleLostPointerCapture,
    );
    node.removeEventListener("touchstart", handleTouchStart);
    node.removeEventListener("touchmove", handleTouchMove);
    node.removeEventListener("touchend", handleTouchEnd);
    node.removeEventListener("touchcancel", handleTouchCancel);
    node.removeEventListener("wheel", handleWheel);
  }

  function resetFallbackGesture(): void {
    clearTimer();
    clearWheelTimer();
    wheelPendingDistance = 0;
    if (pointerId !== null && node.hasPointerCapture(pointerId)) {
      try {
        node.releasePointerCapture(pointerId);
      } catch {
        // The browser can release capture first during a native handoff.
      }
    }
    clearPointer();
    clearVisuals();
    phase = "idle";
  }

  function syncFallbackState(): void {
    if (options.enabled === false) {
      detachFallbackListeners();
      resetFallbackGesture();
      node.style.touchAction = previousTouchAction;
      return;
    }

    node.style.touchAction = "pan-y";
    attachFallbackListeners();
  }

  const disconnectNativeProgress =
    connectNativeBackProgress(handleNativeProgress);
  syncFallbackState();

  return {
    update(nextOptions): void {
      options = nextOptions;
      syncFallbackState();
    },
    destroy(): void {
      destroyed = true;
      disconnectNativeProgress();
      detachFallbackListeners();
      resetFallbackGesture();
      node.style.touchAction = previousTouchAction;
    },
  };
}
