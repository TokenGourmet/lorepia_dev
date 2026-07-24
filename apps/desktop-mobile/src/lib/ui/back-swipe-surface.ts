interface CapturedBackSwipeSurface {
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
let capturedSurfaces: CapturedBackSwipeSurface[] = [];

function latestCapturedSurface(): CapturedBackSwipeSurface | null {
  return capturedSurfaces.at(-1) ?? null;
}

function capturedSurfaceForHref(
  href: string,
): CapturedBackSwipeSurface | null {
  for (let index = capturedSurfaces.length - 1; index >= 0; index -= 1) {
    const surface = capturedSurfaces[index];
    if (surface.href === href) return surface;
  }
  return null;
}

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

function makeCapturedSurfaceInert(root: HTMLElement): void {
  root.inert = true;
  root.setAttribute("aria-hidden", "true");
  root.setAttribute("data-back-swipe-captured-surface", "");

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
  makeCapturedSurfaceInert(clone);
  // This is a non-raster, inert DOM clone with captured form and scroll state.
  // It is a visual surface only, not the still-running previous route.
  capturedSurfaces = [
    ...capturedSurfaces.filter((surface) => surface.href !== href),
    { href, root: clone },
  ];
  surfaceHost?.replaceChildren();
  surfaceHost?.removeAttribute("data-ready");
}

export function activateBackSwipeSurface(
  expectedHref: string | null,
): HTMLElement | null {
  const capturedSurface =
    expectedHref === null
      ? null
      : capturedSurfaceForHref(expectedHref);
  if (
    expectedHref === null ||
    surfaceHost === null ||
    capturedSurface === null
  ) {
    return null;
  }

  if (surfaceHost.firstElementChild !== capturedSurface.root) {
    surfaceHost.replaceChildren(capturedSurface.root);
  }
  surfaceHost.setAttribute("data-ready", "true");
  return surfaceHost;
}

export function activateLatestBackSwipeSurface(): HTMLElement | null {
  const capturedSurface = latestCapturedSurface();
  if (surfaceHost === null || capturedSurface === null) return null;
  if (surfaceHost.firstElementChild !== capturedSurface.root) {
    surfaceHost.replaceChildren(capturedSurface.root);
  }
  surfaceHost.setAttribute("data-ready", "true");
  return surfaceHost;
}

export function completeBackSwipeSurface(href: string): void {
  const capturedSurface = latestCapturedSurface();
  if (capturedSurface?.href !== href) return;
  capturedSurfaces = capturedSurfaces.slice(0, -1);
  surfaceHost?.replaceChildren();
  surfaceHost?.removeAttribute("data-ready");
}

export function clearBackSwipeSurface(): void {
  capturedSurfaces = [];
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
