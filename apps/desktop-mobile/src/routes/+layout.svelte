<script lang="ts">
  import {
    afterNavigate,
    beforeNavigate,
    goto,
  } from "$app/navigation";
  import { onMount, type Component } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { page } from "$app/state";

  import "$lib/design/tokens.css";

  import { activeProviderProfile } from "$lib/providers/active-profile.svelte";
  import { getLlmProvider } from "$lib/providers/catalog";
  import { requestCredentialStatus } from "$lib/providers/credentials";
  import { appPreferences } from "$lib/storage/app-preferences.svelte";
  import { createPreferenceCloseHandler } from "$lib/storage/preference-close-guard";
  import { librarySearch } from "$lib/characters/library-search.svelte";
  import { dockChrome } from "$lib/ui/dock-chrome.svelte";
  import { PERSON_ICON_PATHS } from "$lib/ui/icons";
  import {
    backSwipeSurfaceHost,
    captureBackSwipeSurface,
    clearBackSwipeSurface,
    completeBackSwipeSurface,
  } from "$lib/ui/back-swipe-surface";
  import {
    completeNativeBack,
    connectNativeBackCommit,
    setNativeBackEnabled,
  } from "$lib/ui/native-back";
  import {
    isAndroidNativeBackRoute,
    planAndroidNativeBack,
  } from "$lib/ui/native-back-routing";

  let { children } = $props();
  let DevSizeTool = $state<Component | null>(null);
  let shellElement = $state<HTMLElement | null>(null);

  // Tabs are for everyday destinations, the iOS way: import and the site
  // preview are reached from Home, and 계정 carries the account card plus
  // settings — the "my" tab pattern.
  const NAV_ITEMS = [
    { href: "/home", label: "홈" },
    { href: "/", label: "서재" },
    { href: "/create", label: "생성" },
    { href: "/account", label: "계정" },
  ] as const;

  // One outline glyph per tab. Selection is not read from the glyph — the
  // sliding capsule and tint carry it — so no filled variants exist.
  const TAB_ICONS: Record<string, readonly string[]> = {
    "/home": [
      "M3 10.5 12 3l9 7.5",
      "M5.5 9.3V20a1 1 0 0 0 1 1H10v-6h4v6h3.5a1 1 0 0 0 1-1V9.3",
    ],
    "/": [
      "M4 19.5A2.5 2.5 0 0 1 6.5 17H20",
      "M6.5 2H20v20H6.5A2.5 2.5 0 0 1 4 19.5v-15A2.5 2.5 0 0 1 6.5 2Z",
    ],
    "/create": [
      "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18",
      "M12 8.2v7.6M8.2 12h7.6",
    ],
    "/account": [...PERSON_ICON_PATHS],
  };

  const pathname = $derived(page.url.pathname);
  function isDetailPath(path: string): boolean {
    return (
      path.startsWith("/chat") ||
      path.startsWith("/character") ||
      path.startsWith("/import") ||
      path.startsWith("/community")
    );
  }

  const isDetailScreen = $derived(isDetailPath(pathname));
  function isActive(href: string): boolean {
    return href === "/" ? pathname === "/" : pathname.startsWith(href);
  }

  function suppressNativeAppLinkPreview(event: Event): void {
    const platform = document.documentElement.dataset.nativePlatform;
    if (platform === "ios" || platform === "android") {
      event.preventDefault();
    }
  }

  const activeIndex = $derived(
    NAV_ITEMS.findIndex((item) => isActive(item.href)),
  );

  function localHref(url: URL): string {
    return `${url.pathname}${url.search}${url.hash}`;
  }

  function detailStackDepth(path: string): number {
    if (path.startsWith("/chat/report")) return 4;
    if (path.startsWith("/chat/info")) return 3;
    if (path === "/chat") return 2;
    if (
      path.startsWith("/character/") ||
      path === "/import" ||
      path === "/community"
    ) {
      return 1;
    }
    return 0;
  }

  function nativePlatform(): string | undefined {
    return document.documentElement.dataset.nativePlatform;
  }

  function handleAndroidNativeBack(): void {
    const openDialog =
      document.querySelector<HTMLDialogElement>("dialog[open]");
    if (openDialog !== null) {
      const cancel = new Event("cancel", { cancelable: true });
      if (openDialog.dispatchEvent(cancel) && openDialog.open) {
        openDialog.close("cancel");
      }
      return;
    }

    const plan = planAndroidNativeBack(
      page.url,
      page.state,
      window.history.length,
    );
    if (plan === null) {
      void setNativeBackEnabled(false);
      return;
    }
    if (plan.kind === "history") {
      window.history.back();
      return;
    }
    void goto(plan.href, { replaceState: true });
  }

  /* Capture a non-raster DOM visual surface just before SvelteKit unmounts the
     route. The clone is inert and does not pretend the old route is still
     running; browser history remains the authority for actual navigation. */
  beforeNavigate(({ from, to }) => {
    if (
      shellElement !== null &&
      from !== null &&
      to !== null &&
      isDetailPath(to.url.pathname) &&
      detailStackDepth(to.url.pathname) >
        detailStackDepth(from.url.pathname)
    ) {
      captureBackSwipeSurface(shellElement, localHref(from.url));
    }
  });

  afterNavigate(({ to }) => {
    if (to === null) return;

    completeBackSwipeSurface(localHref(to.url));
    if (!isDetailPath(to.url.pathname)) {
      clearBackSwipeSurface();
    }

    const platform = nativePlatform();
    if (platform === "android") {
      void setNativeBackEnabled(
        isAndroidNativeBackRoute(to.url.pathname),
      );
    } else if (platform === "ios") {
      if (!to.url.pathname.startsWith("/chat")) {
        void completeNativeBack();
      } else if (to.url.pathname !== "/chat") {
        void setNativeBackEnabled(false);
      }
    }
  });

  // Leaving a screen dismisses its transient chrome, as navigation does on
  // iOS: the library search closes and the dock restores.
  $effect(() => {
    void pathname;
    librarySearch.close();
    dockChrome.restore();
  });

  async function hydrateProductSession(): Promise<void> {
    await appPreferences.hydrate();
    const providerId = activeProviderProfile.selectedProviderId;
    if (getLlmProvider(providerId).authKind !== "api-key") {
      activeProviderProfile.setCredentialConfigured(providerId, false);
      return;
    }
    const epoch = activeProviderProfile.beginCredentialOperation(providerId);
    try {
      const status = await requestCredentialStatus(providerId);
      if (activeProviderProfile.isCredentialOperationCurrent(providerId, epoch)) {
        activeProviderProfile.setCredentialConfigured(
          providerId,
          status.configured,
        );
      }
    } catch {
      if (activeProviderProfile.isCredentialOperationCurrent(providerId, epoch)) {
        activeProviderProfile.setCredentialConfigured(providerId, "error");
      }
    }
  }

  onMount(() => {
    void hydrateProductSession();
    let mounted = true;
    let removeCloseGuard: (() => void) | null = null;
    let disconnectAndroidNativeBack = (): void => undefined;

    if (nativePlatform() === "android") {
      disconnectAndroidNativeBack =
        connectNativeBackCommit(handleAndroidNativeBack);
      void setNativeBackEnabled(
        isAndroidNativeBackRoute(page.url.pathname),
      );
    }

    if (
      import.meta.env.DEV &&
      import.meta.env.MODE === "size-tool"
    ) {
      void import("$lib/dev-size-tool/DevSizeTool.svelte").then(
        ({ default: component }) => {
          if (mounted) DevSizeTool = component;
        },
      );
    }

    /* Outside Tauri (browser dev preview) the window bridge is absent and
       getCurrentWindow throws; a crash here poisons the layout's effect
       tree and breaks every later page unmount, so the guard is set up
       only when the bridge exists. */
    try {
      const appWindow = getCurrentWindow();
      const closeWithFlushedPreferences = createPreferenceCloseHandler(
        () => appPreferences.flush(),
        () => appWindow.destroy(),
      );
      void appWindow
        .onCloseRequested(closeWithFlushedPreferences)
        .then((unlisten) => {
          if (mounted) {
            removeCloseGuard = unlisten;
          } else {
            unlisten();
          }
        })
        .catch(() => undefined);
    } catch {
      // No native window to guard; preference flushing below still runs.
    }

    const flushPreferences = (): void => {
      void appPreferences.flush();
    };
    const flushPreferencesWhenHidden = (): void => {
      if (document.visibilityState === "hidden") flushPreferences();
    };
    window.addEventListener("pagehide", flushPreferences);
    document.addEventListener(
      "visibilitychange",
      flushPreferencesWhenHidden,
    );

    return () => {
      mounted = false;
      DevSizeTool = null;
      disconnectAndroidNativeBack();
      if (nativePlatform() === "android") {
        void setNativeBackEnabled(false);
      }
      removeCloseGuard?.();
      window.removeEventListener("pagehide", flushPreferences);
      document.removeEventListener(
        "visibilitychange",
        flushPreferencesWhenHidden,
      );
      flushPreferences();
    };
  });
</script>

<div class="stage">
  <div
    class="back-swipe-underlay"
    use:backSwipeSurfaceHost
    inert
    aria-hidden="true"
  ></div>
  <div
    class="shell"
    class:detail={isDetailScreen}
    bind:this={shellElement}
    data-back-swipe-foreground
  >
    <div class="dockrow" class:minimized={dockChrome.minimized}>
      <nav class="appnav" data-active={activeIndex} aria-label="주요 메뉴">
        {#if activeIndex >= 0}
          <span class="indicator" aria-hidden="true"></span>
        {/if}
        {#each NAV_ITEMS as item (item.href)}
          <a
            href={item.href}
            class="navitem"
            aria-current={isActive(item.href) ? "page" : undefined}
            oncontextmenu={suppressNativeAppLinkPreview}
            ondragstart={suppressNativeAppLinkPreview}
          >
            <svg
              viewBox="0 0 24 24"
              width="22"
              height="22"
              fill="none"
              stroke="currentColor"
              stroke-width="1.8"
              stroke-linecap="round"
              stroke-linejoin="round"
              aria-hidden="true"
            >
              {#each TAB_ICONS[item.href] as d (d)}
                <path {d} />
              {/each}
            </svg>
            <span>{item.label}</span>
          </a>
        {/each}
      </nav>
    </div>
    <main class="content">
      {@render children()}
    </main>
  </div>
</div>

{#if DevSizeTool}
  <DevSizeTool />
{/if}

<style>
  @property --back-swipe-shade {
    syntax: "<number>";
    inherits: true;
    initial-value: 0.12;
  }

  @property --back-transition-progress {
    syntax: "<number>";
    inherits: false;
    initial-value: 0;
  }

  @property --back-transition-x {
    syntax: "<length-percentage>";
    inherits: false;
    initial-value: 0%;
  }

  @property --back-transition-radius {
    syntax: "<length>";
    inherits: false;
    initial-value: 0px;
  }

  @property --back-transition-underlay-x {
    syntax: "<length-percentage>";
    inherits: false;
    initial-value: -7%;
  }

  @property --back-transition-underlay-scale {
    syntax: "<number>";
    inherits: false;
    initial-value: 0.965;
  }

  @property --back-transition-shadow-x {
    syntax: "<length>";
    inherits: false;
    initial-value: -18px;
  }

  @property --back-transition-origin-x {
    syntax: "<percentage>";
    inherits: false;
    initial-value: 0%;
  }

  .stage {
    position: relative;
    height: 100%;
    overflow: hidden;
    background: var(--surface-page);
  }

  .back-swipe-underlay {
    --back-swipe-shade: 0.12;
    position: absolute;
    z-index: 0;
    inset: 0;
    overflow: hidden;
    visibility: hidden;
    pointer-events: none;
    background: var(--surface-page);
    transform-origin: var(--back-transition-origin-x) center;
  }

  :global(.back-swipe-underlay[data-ready="true"]) {
    visibility: visible;
  }

  :global(.back-swipe-underlay[data-back-transition-state]) {
    translate: var(--back-transition-underlay-x) 0;
    scale: var(--back-transition-underlay-scale);
  }

  .back-swipe-underlay::after {
    content: "";
    position: absolute;
    inset: 0;
    z-index: 2;
    background: #000;
    opacity: var(--back-swipe-shade);
    pointer-events: none;
  }

  .back-swipe-underlay :global([data-back-swipe-captured-surface]),
  .back-swipe-underlay
    :global([data-back-swipe-captured-surface] *) {
    animation: none !important;
    transition: none !important;
    caret-color: transparent !important;
  }

  .shell {
    position: relative;
    z-index: 1;
    height: 100%;
    background: var(--surface-page);
  }

  :global(.shell[data-back-transition-state]) {
    translate: var(--back-transition-x) 0;
    border-radius: var(--back-transition-radius);
    overflow: clip;
    box-shadow:
      var(--back-transition-shadow-x) 0 46px
      rgb(0 0 0 / 0.2);
  }

  :global(.shell[data-back-transition-state="interactive"]),
  :global(
      .back-swipe-underlay[data-back-transition-state="interactive"]
    ) {
    transition: none !important;
  }

  .content {
    height: 100%;
    min-width: 0;
  }

  /* The bottom row holds only top-level navigation. It spans the screen so
     the dock stays centered; pointer events re-enable on the dock so the
     empty flanks don't swallow content taps. Above a home indicator it rides
     the safe inset plus sp-3, with a sp-4 floor when no inset is present. */
  .dockrow {
    position: absolute;
    z-index: 10;
    left: 0;
    right: 0;
    bottom: max(
      calc(env(safe-area-inset-bottom, 0px) + var(--sp-3)),
      var(--sp-4)
    );
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--sp-2);
    padding: 0 var(--sp-4);
    pointer-events: none;
  }

  .dockrow > :global(*) {
    pointer-events: auto;
  }

  .appnav {
    flex: none;
    width: var(--dock-width);
    box-sizing: border-box;
    display: flex;
    align-items: center;
    /* The 64x44 slot at the dock's current scale — except the height, which
       floors at --size-touch so the tap target survives the smallest screens
       (--size-tabbar carries the matching 52px floor). --slot-step is one
       slot plus one gap, the distance the indicator travels per tab. */
    --slot-w: calc(var(--dock-width) * 64 / var(--dock-span));
    --slot-h: max(
      var(--size-touch),
      calc(var(--dock-width) * 44 / var(--dock-span))
    );
    --slot-step: calc(var(--dock-width) * 68 / var(--dock-span));
    /* Gap and padding are part of the swept geometry, so they scale with it. */
    gap: calc(var(--dock-width) * 4 / var(--dock-span));
    height: var(--size-tabbar);
    padding: 0 calc(var(--dock-width) * 8 / var(--dock-span));
    border: 0.5px solid var(--hairline);
    border-radius: var(--r-pill);
    background: var(--bar-bg);
    -webkit-backdrop-filter: blur(24px) saturate(1.6);
    backdrop-filter: blur(24px) saturate(1.6);
    box-shadow: var(--shadow-float);
    font-family: var(--font-ui);
    transform-origin: center bottom;
    transition:
      transform var(--dur-base) var(--ease-out),
      height var(--dur-base) var(--ease-out);
  }

  :global(:root[data-native-platform="ios"]) .navitem,
  :global(:root[data-native-platform="android"]) .navitem {
    -webkit-user-select: none;
    user-select: none;
    -webkit-touch-callout: none;
  }

  /* iOS 26 minimize: while content scrolls down the dock compacts to an
     icon-only bar — the labels collapse and the bar's own height drops to
     fit just the glyphs (44 bar / 36 slot keeps the 4px inset, so the
     indicator stays concentric). Scrolling up restores it. */
  .dockrow.minimized .appnav {
    --slot-h: 36px;
    height: 44px;
    transform: translateY(4px);
  }

  .dockrow.minimized .navitem {
    gap: 0;
  }

  .dockrow.minimized .navitem span {
    height: 0;
    opacity: 0;
  }

  /* The floating dock overlays each screen's own scroll area, so widen the
     shared bottom inset that every screen already applies. */
  .shell:not(.detail) .content {
    --safe-bottom: calc(
      max(calc(env(safe-area-inset-bottom, 0px) + var(--sp-3)), var(--sp-4)) +
        var(--size-tabbar) + var(--sp-2)
    );
  }

  /* Android's committed wrapper already pads the WebView, so only the dock
     height may contribute here (mirrors the tokens.css inset-owner rule). */
  :global([data-native-inset-owner="android-view-padding"])
    .shell:not(.detail)
    .content {
    /* env() is 0 inside the padded WebView, so the dock sits at the sp-4
       floor here — mirror that, not the sp-3 inset ride. */
    --safe-bottom: calc(var(--size-tabbar) + var(--sp-4) + var(--sp-2));
  }

  .shell.detail .dockrow {
    display: none;
  }

  /* The selection reads iOS 26-style: a soft tint capsule glides to sit
     behind the active slot, and the glyph only changes color, never shape.
     Strict CSP forbids inline styles, so the slot index reaches CSS through
     data-active instead of a style directive — one rule per NAV_ITEMS slot. */
  .appnav[data-active="0"] {
    --i: 0;
  }
  .appnav[data-active="1"] {
    --i: 1;
  }
  .appnav[data-active="2"] {
    --i: 2;
  }
  .appnav[data-active="3"] {
    --i: 3;
  }

  .indicator {
    position: absolute;
    top: 50%;
    left: calc(var(--dock-width) * 8 / var(--dock-span));
    width: var(--slot-w);
    height: var(--slot-h);
    border-radius: var(--r-pill);
    background: var(--tint-soft);
    transform: translate(calc(var(--i) * var(--slot-step)), -50%);
    transition:
      transform var(--dur-base) var(--ease-spring),
      height var(--dur-base) var(--ease-out);
  }

  .navitem {
    /* Positioned so the slots paint above the sliding indicator. */
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 3px;
    /* The 64x44 unit the dock is built from, at whatever scale it ended up
       with. Fixed rather than flex, or the slot stops being the unit. */
    flex: none;
    min-width: 0;
    width: var(--slot-w);
    height: var(--slot-h);
    padding: 0;
    color: var(--text-faint);
    text-decoration: none;
    transition:
      color var(--dur-base) var(--ease-out),
      transform var(--dur-base) var(--ease-spring),
      height var(--dur-base) var(--ease-out),
      gap var(--dur-base) var(--ease-out);
  }

  .navitem:active {
    transform: scale(0.88);
  }

  .navitem span {
    max-width: 100%;
    /* Explicit height so the minimize collapse can animate it to zero. */
    height: 10px;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    font-size: 10px;
    font-weight: 600;
    line-height: 1;
    letter-spacing: -0.01em;
    transition:
      opacity var(--dur-base) var(--ease-out),
      height var(--dur-base) var(--ease-out);
  }

  /* The height floors at --size-touch, but the slots keep narrowing; below
     this the 4-character labels ellipsize into noise, so the icons carry the
     dock alone. */
  @media (max-width: 300px) {
    .navitem span {
      display: none;
    }
  }

  .navitem[aria-current="page"] {
    color: var(--tint);
  }

  @media (min-width: 700px) {
    .shell {
      display: grid;
      grid-template-columns: auto minmax(0, 1fr);
    }

    .shell:not(.detail) .content {
      --safe-bottom: env(safe-area-inset-bottom, 0px);
    }

    /* The rail takes the dock's place in the grid. */
    .dockrow,
    .shell.detail .dockrow {
      display: contents;
    }

    .appnav {
      position: static;
      display: flex;
      transform: none;
      flex-direction: column;
      justify-content: flex-start;
      gap: var(--sp-2);
      width: 84px;
      height: auto;
      border: 0;
      border-right: 0.5px solid var(--hairline);
      border-radius: 0;
      box-shadow: none;
      animation: none;
      padding: calc(var(--sp-4) + var(--safe-top)) 0 var(--sp-4);
    }

    /* The slide math assumes the horizontal dock; the rail marks selection
       with its own filled row below. */
    .indicator {
      display: none;
    }

    /* Minimize-on-scroll is phone dock behavior; the rail holds still. */
    .dockrow.minimized .appnav {
      height: auto;
      transform: none;
    }

    .dockrow.minimized .navitem {
      gap: 3px;
      height: 56px;
    }

    .dockrow.minimized .navitem span {
      height: 10px;
      opacity: 1;
    }

    .navitem {
      flex-direction: column;
      gap: 3px;
      /* The sidebar stacks vertically, so growing would stretch item heights
         down the rail instead of sharing width. */
      flex: none;
      align-self: stretch;
      /* The rail sets the width; drop the dock unit so stretch applies. */
      width: auto;
      height: 56px;
      margin: 0 var(--sp-2);
      padding: 0;
      border-radius: var(--r-block);
    }

    .navitem:active {
      transform: scale(0.95);
    }

    /* On the wide sidebar the selection reads as a filled row, the iPad
       idiom — the static counterpart of the dock's sliding capsule. */
    .navitem[aria-current="page"] {
      background: var(--tint-soft);
    }
  }
</style>
