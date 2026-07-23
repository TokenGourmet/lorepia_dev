class DockChromeState {
  minimized = $state(false);

  restore(): void {
    this.minimized = false;
  }
}

export const dockChrome = new DockChromeState();

/* Attach to a screen's scroll container. Near the top the dock always
   restores; past that, direction decides, with a small dead zone so momentum
   jitter doesn't flicker the state. */
export function minimizeDockOnScroll(node: HTMLElement): { destroy(): void } {
  let lastTop = node.scrollTop;

  const onScroll = (): void => {
    const top = node.scrollTop;
    const delta = top - lastTop;
    lastTop = top;
    if (top <= 24) {
      dockChrome.minimized = false;
    } else if (delta > 4) {
      dockChrome.minimized = true;
    } else if (delta < -4) {
      dockChrome.minimized = false;
    }
  };

  node.addEventListener("scroll", onScroll, { passive: true });
  return {
    destroy(): void {
      node.removeEventListener("scroll", onScroll);
    },
  };
}
