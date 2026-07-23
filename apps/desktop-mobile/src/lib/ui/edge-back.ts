/* iOS-style interactive pop: a drag starting at the screen's left edge pulls
   the screen with the finger; passing the commit threshold (or flicking)
   completes the back navigation, anything else settles the screen home.
   The strict-CSP build forbids markup styles, so every visual goes through
   runtime CSSOM writes, which the CSP allows. */

const EDGE_START = 24;
const AXIS_SLOP = 8;
const COMMIT_FRACTION = 0.35;
const FLICK_DISTANCE = 24;
const FLICK_VELOCITY = 0.5;
const SETTLE_MS = 220;

export interface EdgeBackOptions {
  onBack: () => void;
}

export function edgeSwipeBack(
  node: HTMLElement,
  options: EdgeBackOptions,
): { destroy(): void } {
  let pointerId: number | null = null;
  let startX = 0;
  let startY = 0;
  let lastX = 0;
  let lastTime = 0;
  let velocity = 0;
  let active = false;
  let settling = false;
  let settleTimer: ReturnType<typeof setTimeout> | null = null;

  /* Vertical panning stays native; horizontal motion reaches the handlers. */
  const previousTouchAction = node.style.touchAction;
  node.style.touchAction = "pan-y";

  function beginVisuals(): void {
    node.style.willChange = "transform";
    node.style.boxShadow = "-16px 0 40px rgba(0, 0, 0, 0.18)";
  }

  function clearVisuals(): void {
    node.style.transition = "";
    node.style.transform = "";
    node.style.willChange = "";
    node.style.boxShadow = "";
  }

  function handleDown(event: PointerEvent): void {
    if (settling) return;
    if (event.pointerType === "mouse" && event.button !== 0) return;
    if (event.clientX - node.getBoundingClientRect().left > EDGE_START) return;
    pointerId = event.pointerId;
    startX = lastX = event.clientX;
    startY = event.clientY;
    lastTime = event.timeStamp;
    velocity = 0;
    active = false;
  }

  function handleMove(event: PointerEvent): void {
    if (event.pointerId !== pointerId) return;
    const dx = event.clientX - startX;
    const dy = event.clientY - startY;
    if (!active) {
      if (Math.abs(dx) < AXIS_SLOP && Math.abs(dy) < AXIS_SLOP) return;
      if (Math.abs(dx) <= Math.abs(dy)) {
        pointerId = null;
        return;
      }
      active = true;
      try {
        node.setPointerCapture(event.pointerId);
      } catch {
        // Synthetic or already-released pointers can't be captured; the
        // drag still tracks through the listeners.
      }
      beginVisuals();
    }
    const elapsed = event.timeStamp - lastTime;
    if (elapsed > 0) {
      velocity = (event.clientX - lastX) / elapsed;
    }
    lastX = event.clientX;
    lastTime = event.timeStamp;
    node.style.transform = `translateX(${Math.max(dx, 0)}px)`;
  }

  function handleUp(event: PointerEvent): void {
    if (event.pointerId !== pointerId) return;
    pointerId = null;
    if (!active) return;
    active = false;
    const dx = event.clientX - startX;
    const commit =
      dx > node.clientWidth * COMMIT_FRACTION ||
      (dx > FLICK_DISTANCE && velocity > FLICK_VELOCITY);
    if (commit) {
      /* Navigation unmounts this screen anyway, so an exit animation would
         never be seen. The microtask hop moves the unmount out of this
         pointer event's dispatch. */
      clearVisuals();
      queueMicrotask(() => options.onBack());
      return;
    }
    /* Cancel springs the screen home. A timer instead of transitionend: the
       event is lost if anything unmounts mid-settle, and the pop must never
       wedge half-open. */
    settling = true;
    node.style.transition = `transform ${SETTLE_MS}ms ease-out`;
    requestAnimationFrame(() => {
      node.style.transform = "translateX(0px)";
    });
    settleTimer = setTimeout(() => {
      settleTimer = null;
      settling = false;
      clearVisuals();
    }, SETTLE_MS + 30);
  }

  node.addEventListener("pointerdown", handleDown);
  node.addEventListener("pointermove", handleMove);
  node.addEventListener("pointerup", handleUp);
  node.addEventListener("pointercancel", handleUp);

  return {
    destroy(): void {
      if (settleTimer !== null) {
        clearTimeout(settleTimer);
      }
      node.style.touchAction = previousTouchAction;
      node.removeEventListener("pointerdown", handleDown);
      node.removeEventListener("pointermove", handleMove);
      node.removeEventListener("pointerup", handleUp);
      node.removeEventListener("pointercancel", handleUp);
    },
  };
}
