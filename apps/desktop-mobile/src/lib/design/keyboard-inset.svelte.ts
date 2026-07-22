export function computeKeyboardInset(
  windowInnerHeight: number,
  visualViewportHeight: number,
  visualViewportOffsetTop: number,
): number {
  const inset = Math.round(
    windowInnerHeight - visualViewportHeight - visualViewportOffsetTop,
  );
  return inset > 0 ? inset : 0;
}

let inset = $state(0);

export const keyboardInset = {
  get value(): number {
    return inset;
  },
  start(): (() => void) | undefined {
    if (typeof window === "undefined") {
      return undefined;
    }
    const viewport = window.visualViewport;
    if (!viewport) {
      return undefined;
    }

    const update = (): void => {
      inset = computeKeyboardInset(
        window.innerHeight,
        viewport.height,
        viewport.offsetTop,
      );
    };

    viewport.addEventListener("resize", update);
    viewport.addEventListener("scroll", update);
    update();

    return () => {
      viewport.removeEventListener("resize", update);
      viewport.removeEventListener("scroll", update);
      inset = 0;
    };
  },
};
