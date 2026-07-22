export type SwipeCommit = "left" | "right" | null;

export interface HorizontalSwipeOptions {
  onMove: (dx: number) => void;
  onEnd: (commit: SwipeCommit, dx: number) => void;
}

const AXIS_SLOP = 12;
const AXIS_RATIO = 1.3;
const COMMIT_FRACTION = 0.28;
const FLICK_DISTANCE = 24;
const FLICK_VELOCITY = 0.5;

export function horizontalSwipe(
  node: HTMLElement,
  options: HorizontalSwipeOptions,
): { destroy(): void } {
  let pointerId: number | null = null;
  let startX = 0;
  let startY = 0;
  let lastX = 0;
  let lastTime = 0;
  let velocity = 0;
  let axis: "horizontal" | "vertical" | null = null;

  function handleDown(event: PointerEvent): void {
    if (event.pointerType === "mouse" && event.button !== 0) {
      return;
    }
    pointerId = event.pointerId;
    startX = lastX = event.clientX;
    startY = event.clientY;
    lastTime = event.timeStamp;
    velocity = 0;
    axis = null;
  }

  function handleMove(event: PointerEvent): void {
    if (event.pointerId !== pointerId) {
      return;
    }
    const dx = event.clientX - startX;
    const dy = event.clientY - startY;

    if (axis === null) {
      if (Math.abs(dx) < AXIS_SLOP && Math.abs(dy) < AXIS_SLOP) {
        return;
      }
      axis =
        Math.abs(dx) > Math.abs(dy) * AXIS_RATIO ? "horizontal" : "vertical";
      if (axis === "horizontal") {
        node.setPointerCapture(pointerId);
      }
    }

    if (axis !== "horizontal") {
      return;
    }

    const elapsed = event.timeStamp - lastTime;
    if (elapsed > 0) {
      velocity = (event.clientX - lastX) / elapsed;
    }
    lastX = event.clientX;
    lastTime = event.timeStamp;
    options.onMove(dx);
  }

  function handleUp(event: PointerEvent): void {
    if (event.pointerId !== pointerId) {
      return;
    }
    if (axis === "horizontal") {
      const dx = event.clientX - startX;
      const threshold = node.clientWidth * COMMIT_FRACTION;
      let commit: SwipeCommit = null;
      if (dx > threshold || (dx > FLICK_DISTANCE && velocity > FLICK_VELOCITY)) {
        commit = "right";
      } else if (
        dx < -threshold ||
        (dx < -FLICK_DISTANCE && velocity < -FLICK_VELOCITY)
      ) {
        commit = "left";
      }
      options.onEnd(commit, dx);
    }
    pointerId = null;
    axis = null;
  }

  node.addEventListener("pointerdown", handleDown);
  node.addEventListener("pointermove", handleMove);
  node.addEventListener("pointerup", handleUp);
  node.addEventListener("pointercancel", handleUp);

  return {
    destroy(): void {
      node.removeEventListener("pointerdown", handleDown);
      node.removeEventListener("pointermove", handleMove);
      node.removeEventListener("pointerup", handleUp);
      node.removeEventListener("pointercancel", handleUp);
    },
  };
}
