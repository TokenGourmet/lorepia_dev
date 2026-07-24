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

export function shouldMeasureWebKeyboardInset(
  nativeInsetOwner: string | undefined,
): boolean {
  return nativeInsetOwner !== "android-view-padding";
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
    const nativeInsetOwner =
      typeof document === "undefined"
        ? undefined
        : document.documentElement.dataset.nativeInsetOwner;
    if (!shouldMeasureWebKeyboardInset(nativeInsetOwner)) {
      // The Android host applies system-bar and IME padding to the WebView.
      // A visualViewport spacer here would apply the same IME twice on
      // WebView versions that also report the reduced viewport.
      inset = 0;
      return () => {
        inset = 0;
      };
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
