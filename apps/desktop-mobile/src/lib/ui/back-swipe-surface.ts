interface BackSwipeSnapshot {
  href: string;
  root: HTMLElement;
}

const REFERENCE_ATTRIBUTES = [
  "aria-activedescendant",
  "aria-controls",
  "aria-describedby",
  "aria-labelledby",
  "aria-owns",
  "for",
] as const;

let surfaceHost: HTMLElement | null = null;
let snapshot: BackSwipeSnapshot | null = null;

function copyRuntimeState(source: HTMLElement, clone: HTMLElement): void {
  const sourceElements = [
    source,
    ...source.querySelectorAll<HTMLElement>("*"),
  ];
  const cloneElements = [
    clone,
    ...clone.querySelectorAll<HTMLElement>("*"),
  ];

  for (
    let index = 0;
    index < Math.min(sourceElements.length, cloneElements.length);
    index += 1
  ) {
    const sourceElement = sourceElements[index];
    const cloneElement = cloneElements[index];
    cloneElement.scrollTop = sourceElement.scrollTop;
    cloneElement.scrollLeft = sourceElement.scrollLeft;

    if (
      sourceElement instanceof HTMLInputElement &&
      cloneElement instanceof HTMLInputElement
    ) {
      cloneElement.value = sourceElement.value;
      cloneElement.checked = sourceElement.checked;
    } else if (
      sourceElement instanceof HTMLTextAreaElement &&
      cloneElement instanceof HTMLTextAreaElement
    ) {
      cloneElement.value = sourceElement.value;
    } else if (
      sourceElement instanceof HTMLSelectElement &&
      cloneElement instanceof HTMLSelectElement
    ) {
      cloneElement.selectedIndex = sourceElement.selectedIndex;
    }
  }
}

function sanitizeSnapshot(root: HTMLElement): void {
  root.inert = true;
  root.setAttribute("aria-hidden", "true");
  root.setAttribute("data-back-swipe-snapshot", "");

  for (const element of [root, ...root.querySelectorAll<HTMLElement>("*")]) {
    element.removeAttribute("id");
    element.removeAttribute("aria-live");
    element.removeAttribute("autofocus");
    for (const attribute of REFERENCE_ATTRIBUTES) {
      element.removeAttribute(attribute);
    }
  }
}

export function captureBackSwipeSurface(
  source: HTMLElement,
  href: string,
): void {
  const clone = source.cloneNode(true) as HTMLElement;
  copyRuntimeState(source, clone);
  sanitizeSnapshot(clone);
  snapshot = { href, root: clone };
  surfaceHost?.replaceChildren();
  surfaceHost?.removeAttribute("data-ready");
}

export function activateBackSwipeSurface(
  expectedHref: string | null,
): HTMLElement | null {
  if (
    expectedHref === null ||
    surfaceHost === null ||
    snapshot === null ||
    snapshot.href !== expectedHref
  ) {
    return null;
  }

  if (surfaceHost.firstElementChild !== snapshot.root) {
    surfaceHost.replaceChildren(snapshot.root);
  }
  surfaceHost.setAttribute("data-ready", "true");
  return surfaceHost;
}

export function clearBackSwipeSurface(): void {
  snapshot = null;
  surfaceHost?.replaceChildren();
  surfaceHost?.removeAttribute("data-ready");
}

export function backSwipeSurfaceHost(
  node: HTMLElement,
): { destroy(): void } {
  surfaceHost = node;
  return {
    destroy(): void {
      if (surfaceHost === node) {
        surfaceHost = null;
      }
    },
  };
}
