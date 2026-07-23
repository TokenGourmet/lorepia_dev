<script lang="ts">
  import { onMount } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { page } from "$app/state";

  import "$lib/design/tokens.css";

  import { activeProviderProfile } from "$lib/providers/active-profile.svelte";
  import { getLlmProvider } from "$lib/providers/catalog";
  import { requestCredentialStatus } from "$lib/providers/credentials";
  import { appPreferences } from "$lib/storage/app-preferences.svelte";
  import { createPreferenceCloseHandler } from "$lib/storage/preference-close-guard";

  let { children } = $props();

  const NAV_ITEMS = [
    { href: "/home", label: "홈" },
    { href: "/", label: "서재" },
    { href: "/create", label: "생성" },
    { href: "/import", label: "가져오기" },
    { href: "/settings", label: "설정" },
  ] as const;

  // One outline glyph per tab. Selection is not read from the glyph — the
  // sliding capsule and tint carry it — so no filled variants exist.
  const TAB_ICONS: Record<string, string[]> = {
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
    "/import": [
      "M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4",
      "M7 10l5 5 5-5",
      "M12 15V3",
    ],
    "/settings": [
      "M12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6",
      "M19.4 15a1.7 1.7 0 0 0 .34 1.87l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.7 1.7 0 0 0-1.87-.34 1.7 1.7 0 0 0-1.03 1.56V21a2 2 0 1 1-4 0v-.09a1.7 1.7 0 0 0-1.11-1.56 1.7 1.7 0 0 0-1.87.34l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.7 1.7 0 0 0 .34-1.87 1.7 1.7 0 0 0-1.56-1.03H3a2 2 0 1 1 0-4h.09A1.7 1.7 0 0 0 4.65 8.9a1.7 1.7 0 0 0-.34-1.87l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.7 1.7 0 0 0 1.87.34h.09A1.7 1.7 0 0 0 10.13 3V3a2 2 0 1 1 4 0v.09a1.7 1.7 0 0 0 1.03 1.56 1.7 1.7 0 0 0 1.87-.34l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.7 1.7 0 0 0-.34 1.87v.09a1.7 1.7 0 0 0 1.56 1.03H21a2 2 0 1 1 0 4h-.09a1.7 1.7 0 0 0-1.51 1.03Z",
    ],
  };

  const pathname = $derived(page.url.pathname);
  const isDetailScreen = $derived(
    pathname.startsWith("/chat") || pathname.startsWith("/character"),
  );

  function isActive(href: string): boolean {
    return href === "/" ? pathname === "/" : pathname.startsWith(href);
  }

  const activeIndex = $derived(
    NAV_ITEMS.findIndex((item) => isActive(item.href)),
  );

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

<div class="shell" class:detail={isDetailScreen}>
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

  .appnav {
    position: absolute;
    z-index: 10;
    left: 0;
    right: 0;
    margin-inline: auto;
    width: var(--dock-width);
    box-sizing: border-box;
    /* Above a home indicator the dock rides the safe inset plus sp-3; with
       no inset it floors at sp-4 so the bottom gap matches the side margins. */
    bottom: max(
      calc(env(safe-area-inset-bottom, 0px) + var(--sp-3)),
      var(--sp-4)
    );
    display: flex;
    align-items: center;
    /* The 64x48 slot at the dock's current scale — except the height, which
       floors at --size-touch so the tap target survives the smallest screens
       (--size-tabbar carries the matching 55px floor). --slot-step is one
       slot plus one gap, the distance the indicator travels per tab. */
    --slot-w: calc(var(--dock-width) * 64 / var(--dock-span));
    --slot-h: max(
      var(--size-touch),
      calc(var(--dock-width) * 48 / var(--dock-span))
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

  .shell.detail .appnav {
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
  .appnav[data-active="4"] {
    --i: 4;
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
    transition: transform var(--dur-base) var(--ease-spring);
  }

  .navitem {
    /* Positioned so the slots paint above the sliding indicator. */
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 3px;
    /* The 64x48 unit the dock is built from, at whatever scale it ended up
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
      transform var(--dur-base) var(--ease-spring);
  }

  .navitem:active {
    transform: scale(0.88);
  }

  .navitem span {
    max-width: 100%;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    font-size: 10px;
    font-weight: 600;
    line-height: 1;
    letter-spacing: -0.01em;
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

    .appnav {
      position: static;
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

    .shell.detail .appnav {
      display: flex;
    }

    /* The slide math assumes the horizontal dock; the rail marks selection
       with its own filled row below. */
    .indicator {
      display: none;
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
