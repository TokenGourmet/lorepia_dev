const AXIS_SLOP = 8;
const COMMIT_FRACTION = 0.35;
const FLICK_DISTANCE = 24;
const FLICK_VELOCITY = 0.5;
const VELOCITY_WINDOW_MS = 100;
const CANCEL_MS = 240;
const COMPLETE_MS = 220;
const RELEASE_EASE = "cubic-bezier(0.22, 1, 0.36, 1)";

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

export interface ContentBackOptions {
  onBack: () => void;
  getUnderlay?: () => HTMLElement | null;
  enabled?: boolean;
}

interface StoredVisualStyles {
  element: HTMLElement;
  translate: string;
  transition: string;
  willChange: string;
  boxShadow: string;
  userSelect: string;
  shade: string;
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
    boxShadow: element.style.boxShadow,
    userSelect: element.style.userSelect,
    shade: element.style.getPropertyValue("--back-swipe-shade"),
  };
}

function restoreVisualStyles(stored: StoredVisualStyles | null): void {
  if (stored === null) return;
  const { element } = stored;
  element.style.translate = stored.translate;
  element.style.transition = stored.transition;
  element.style.willChange = stored.willChange;
  element.style.boxShadow = stored.boxShadow;
  element.style.userSelect = stored.userSelect;
  if (stored.shade === "") {
    element.style.removeProperty("--back-swipe-shade");
  } else {
    element.style.setProperty("--back-swipe-shade", stored.shade);
  }
}

/**
 * iOS 26-style content backswipe for a vertically scrolling content region.
 * The gesture can begin anywhere noninteractive in the region; the complete
 * page follows the pointer and reveals an inert snapshot of the prior route.
 */
export function contentSwipeBack(
  node: HTMLElement,
  initialOptions: ContentBackOptions,
): { update(options: ContentBackOptions): void; destroy(): void } {
  let options = initialOptions;
  let phase: GesturePhase = "idle";
  let pointerId: number | null = null;
  let startX = 0;
  let startY = 0;
  let distance = 0;
  let samples: ContentBackSample[] = [];
  let foreground: HTMLElement | null = null;
  let underlay: HTMLElement | null = null;
  let foregroundStyles: StoredVisualStyles | null = null;
  let underlayStyles: StoredVisualStyles | null = null;
  let animationFrame: number | null = null;
  let releaseTimer: ReturnType<typeof setTimeout> | null = null;
  let destroyed = false;
  let listenersAttached = false;

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
    foreground =
      node.closest<HTMLElement>("[data-back-swipe-foreground]") ?? node;
    underlay = options.getUnderlay?.() ?? null;
    foregroundStyles = storeVisualStyles(foreground);
    underlayStyles =
      underlay === null ? null : storeVisualStyles(underlay);

    foreground.style.willChange = "translate";
    foreground.style.userSelect = "none";
    foreground.style.boxShadow = "-16px 0 40px rgba(0, 0, 0, 0.18)";
    if (underlay !== null) {
      underlay.style.willChange = "translate";
    }
    applyVisuals(0);
  }

  function applyVisuals(nextDistance: number): void {
    if (foreground === null) return;
    const width = visualWidth();
    distance = clampDistance(nextDistance, width);
    const progress = distance / width;
    foreground.style.translate = `${distance}px 0px`;

    if (underlay !== null) {
      const parallax = Math.min(width * 0.18, 64);
      underlay.style.translate = `${-parallax * (1 - progress)}px 0px`;
      underlay.style.setProperty(
        "--back-swipe-shade",
        `${0.12 * (1 - progress)}`,
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
  }

  function clearPointer(): void {
    pointerId = null;
    startX = 0;
    startY = 0;
    samples = [];
  }

  function finishCancel(): void {
    if (phase !== "dragging" || foreground === null) {
      phase = "idle";
      clearPointer();
      return;
    }

    phase = "cancelling";
    const duration = reducedMotion.matches ? 0 : CANCEL_MS;
    foreground.style.transition = `translate ${duration}ms ${RELEASE_EASE}`;
    if (underlay !== null) {
      underlay.style.transition =
        `translate ${duration}ms ${RELEASE_EASE}, ` +
        `--back-swipe-shade ${duration}ms ${RELEASE_EASE}`;
    }
    applyVisuals(0);
    clearPointer();
    clearTimer();
    releaseTimer = setTimeout(() => {
      releaseTimer = null;
      if (destroyed) return;
      clearVisuals();
      phase = "idle";
    }, duration);
  }

  function finishCommit(): void {
    if (phase !== "dragging" || foreground === null) return;
    phase = "committing";
    const duration = reducedMotion.matches ? 0 : COMPLETE_MS;
    foreground.style.transition = `translate ${duration}ms ${RELEASE_EASE}`;
    if (underlay !== null) {
      underlay.style.transition =
        `translate ${duration}ms ${RELEASE_EASE}, ` +
        `--back-swipe-shade ${duration}ms ${RELEASE_EASE}`;
    }
    applyVisuals(visualWidth());
    clearPointer();
    clearTimer();
    releaseTimer = setTimeout(() => {
      releaseTimer = null;
      if (destroyed) return;
      options.onBack();
    }, duration);
  }

  function abandonPossibleGesture(): void {
    phase = "idle";
    clearPointer();
  }

  function handleDown(event: PointerEvent): void {
    if (phase !== "idle" || pointerId !== null) return;
    if (options.enabled === false) return;
    if (!event.isPrimary) return;
    if (event.pointerType === "mouse" && event.button !== 0) return;
    if (startsOnBlockedContent(event.target, node)) return;

    pointerId = event.pointerId;
    startX = event.clientX;
    startY = event.clientY;
    distance = 0;
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
      try {
        node.setPointerCapture(event.pointerId);
      } catch {
        // Synthetic pointers and an already released pointer can still be
        // followed while they remain over the content region.
      }
    }

    if (phase !== "dragging") return;
    event.preventDefault();
    sampleEvent(event);
    scheduleVisuals(dx);
  }

  function handleUp(event: PointerEvent): void {
    if (event.pointerId !== pointerId) return;
    if (phase === "possible") {
      abandonPossibleGesture();
      return;
    }
    if (phase !== "dragging") return;

    sampleEvent(event);
    distance = event.clientX - startX;
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
  }

  function resetFallbackGesture(): void {
    clearTimer();
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

  syncFallbackState();

  return {
    update(nextOptions): void {
      options = nextOptions;
      syncFallbackState();
    },
    destroy(): void {
      destroyed = true;
      detachFallbackListeners();
      resetFallbackGesture();
      node.style.touchAction = previousTouchAction;
    },
  };
}
