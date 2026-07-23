<script lang="ts">
  import { onMount, tick } from "svelte";
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
  import { PERSON_ICON_PATHS, SEARCH_ICON_PATHS } from "$lib/ui/icons";

  let { children } = $props();

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
  const isDetailScreen = $derived(
    pathname.startsWith("/chat") ||
      pathname.startsWith("/character") ||
      pathname.startsWith("/import") ||
      pathname.startsWith("/community"),
  );
  const isLibraryScreen = $derived(pathname === "/");

  function isActive(href: string): boolean {
    return href === "/" ? pathname === "/" : pathname.startsWith(href);
  }

  const activeIndex = $derived(
    NAV_ITEMS.findIndex((item) => isActive(item.href)),
  );

  let bottomSearchField = $state<HTMLInputElement | null>(null);

  async function openBottomSearch(): Promise<void> {
    librarySearch.openSearch();
    await tick();
    bottomSearchField?.focus();
  }

  // Leaving a screen dismisses its transient chrome, as navigation does on
  // iOS: the bottom search collapses and the dock restores.
  $effect(() => {
    void pathname;
    librarySearch.close();
    dockChrome.restore();
  });

  function onWindowKeydown(event: KeyboardEvent): void {
    if (event.key === "Escape" && librarySearch.open) {
      librarySearch.close();
    }
  }

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
        activeProviderProfile.setCredentialConfigured(providerId, null);
      }
    }
  }

  onMount(() => {
    void hydrateProductSession();
    let mounted = true;
    let removeCloseGuard: (() => void) | null = null;

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

<svelte:window onkeydown={onWindowKeydown} />

<div class="shell" class:detail={isDetailScreen}>
  <div
    class="dockrow"
    class:searching={librarySearch.open}
    class:minimized={dockChrome.minimized}
    class:hasball={isLibraryScreen}
  >
    <nav class="appnav" data-active={activeIndex} aria-label="주요 메뉴">
      {#if activeIndex >= 0}
        <span class="indicator" aria-hidden="true"></span>
      {/if}
      {#each NAV_ITEMS as item (item.href)}
        <a
          href={item.href}
          class="navitem"
          aria-current={isActive(item.href) ? "page" : undefined}
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
    {#if isLibraryScreen && !librarySearch.open}
      <button
        class="searchball"
        type="button"
        aria-label="캐릭터 검색"
        onclick={openBottomSearch}
      >
        <svg
          viewBox="0 0 24 24"
          width="20"
          height="20"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          aria-hidden="true"
        >
          {#each SEARCH_ICON_PATHS as d (d)}
            <path {d} />
          {/each}
        </svg>
      </button>
    {/if}
    {#if librarySearch.open}
      <div class="bottomsearch">
        <svg
          viewBox="0 0 24 24"
          width="16"
          height="16"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          aria-hidden="true"
        >
          {#each SEARCH_ICON_PATHS as d (d)}
            <path {d} />
          {/each}
        </svg>
        <input
          type="search"
          bind:value={librarySearch.query}
          bind:this={bottomSearchField}
          placeholder="검색"
          aria-label="캐릭터 검색"
        />
      </div>
      <!-- The circle keeps the search button's exact seat, now wearing X:
           the dock morphs into the field in place, iOS 26 style. -->
      <button
        class="searchcancel"
        type="button"
        aria-label="검색 닫기"
        onclick={() => librarySearch.close()}
      >
        <svg
          viewBox="0 0 24 24"
          width="18"
          height="18"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          aria-hidden="true"
        >
          <path d="M6 6l12 12M18 6 6 18" />
        </svg>
      </button>
    {/if}
  </div>
  <main class="content">
    {@render children()}
  </main>
</div>

<style>
  .shell {
    position: relative;
    height: 100%;
    background: var(--surface-page);
  }

  .content {
    height: 100%;
    min-width: 0;
  }

  /* The bottom row holds the dock plus, on the library, the round search
     button beside it — the iOS 26 grouping. It spans the screen so the group
     centers as one unit; pointer events re-enable per child so the empty
     flanks don't swallow content taps. Above a home indicator it rides the
     safe inset plus sp-3; with no inset it floors at sp-4 to match the side
     margins. */
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

  /* With the search circle sharing the row, the swept span no longer fits
     narrow screens whole, so the dock's width budget gives up the circle
     and its gap; the slot geometry re-derives from the shrunken width. */
  .dockrow.hasball {
    --dock-width: min(
      calc(100vw - var(--sp-4) * 2 - var(--size-tabbar) - var(--sp-2)),
      calc(var(--dock-span) * 1px)
    );
  }

  /* While the bottom search is up it replaces the dock, as the iOS 26 tab
     bar morphs into a search field. */
  .dockrow.searching .appnav {
    display: none;
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

  /* iOS 26 minimize: while content scrolls down the dock compacts to an
     icon-only bar — the labels collapse and the bar's own height drops to
     fit just the glyphs (44 bar / 36 slot keeps the 4px inset, so the
     indicator stays concentric). Scrolling up restores it. The search
     pieces inherit the same compact size, so a search opened from the
     minimized dock keeps its footprint. */
  .dockrow.minimized .appnav {
    --slot-h: 36px;
    height: 44px;
    transform: translateY(4px);
  }

  .dockrow.minimized .searchball,
  .dockrow.minimized .searchcancel {
    width: 44px;
    height: 44px;
    translate: 0 4px;
  }

  .dockrow.minimized .bottomsearch {
    height: 44px;
    translate: 0 4px;
  }

  .dockrow.minimized .navitem {
    gap: 0;
  }

  .dockrow.minimized .navitem span {
    height: 0;
    opacity: 0;
  }

  /* The search circle and its X-wearing counterpart share one seat beside
     the dock, so open/close reads as the same control changing face. */
  .searchball,
  .searchcancel {
    flex: none;
    width: var(--size-tabbar);
    height: var(--size-tabbar);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    border: 0.5px solid var(--hairline);
    border-radius: var(--r-pill);
    background: var(--bar-bg);
    -webkit-backdrop-filter: blur(24px) saturate(1.6);
    backdrop-filter: blur(24px) saturate(1.6);
    box-shadow: var(--shadow-float);
    color: var(--text-mid);
    cursor: pointer;
    /* Offsets and presses live on the individual translate/scale properties,
       never the transform shorthand: entry keyframes animate `scale` while
       the minimized `translate` holds, so the swap cannot jump. */
    transition:
      translate var(--dur-base) var(--ease-out),
      scale var(--dur-base) var(--ease-spring),
      width var(--dur-base) var(--ease-out),
      height var(--dur-base) var(--ease-out);
    animation: lp-swap-in var(--dur-base) var(--ease-spring) backwards;
  }

  .searchball:active,
  .searchcancel:active {
    scale: 0.9;
  }

  /* The circle re-popping into its seat with a new face. */
  @keyframes lp-swap-in {
    from {
      opacity: 0;
      scale: 0.8;
    }
  }

  /* The field unfolds leftward out of the circle's side, the iOS 26
     circle-to-field morph. */
  @keyframes lp-field-in {
    from {
      opacity: 0;
      scale: 0.75 1;
    }
  }

  .bottomsearch {
    /* The dock's exact footprint, so the morph reads as the capsule
       changing role rather than a new bar appearing. */
    flex: none;
    width: var(--dock-width);
    box-sizing: border-box;
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    height: var(--size-tabbar);
    padding: 0 var(--sp-4);
    border: 0.5px solid var(--hairline);
    border-radius: var(--r-pill);
    background: var(--bar-bg);
    -webkit-backdrop-filter: blur(24px) saturate(1.6);
    backdrop-filter: blur(24px) saturate(1.6);
    box-shadow: var(--shadow-float);
    color: var(--text-faint);
    font-family: var(--font-ui);
    transform-origin: right center;
    transition:
      translate var(--dur-base) var(--ease-out),
      height var(--dur-base) var(--ease-out);
    animation: lp-field-in var(--dur-base) var(--ease-out) backwards;
  }

  .bottomsearch input {
    flex: 1;
    min-width: 0;
    border: 0;
    padding: 0;
    background: transparent;
    color: var(--text-strong);
    font-family: var(--font-ui);
    /* 16px keeps mobile WebViews from auto-zooming the focused field. */
    font-size: 16px;
    caret-color: var(--cursor-color);
  }

  .bottomsearch input:focus {
    outline: none;
  }

  .bottomsearch input::placeholder {
    color: var(--text-faint);
  }

  .bottomsearch input::-webkit-search-cancel-button {
    -webkit-appearance: none;
    appearance: none;
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

    /* The rail takes the dock's place in the grid; the bottom-search pieces
       are phone chrome and stay off. */
    .dockrow,
    .shell.detail .dockrow {
      display: contents;
    }

    .searchball,
    .bottomsearch,
    .searchcancel {
      display: none;
    }

    .appnav,
    .dockrow.searching .appnav {
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
