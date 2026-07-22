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
    { href: "/", label: "서재" },
    { href: "/import", label: "가져오기" },
    { href: "/settings", label: "설정" },
  ] as const;

  const pathname = $derived(page.url.pathname);
  const isDetailScreen = $derived(
    pathname.startsWith("/chat") || pathname.startsWith("/character"),
  );

  function isActive(href: string): boolean {
    return href === "/" ? pathname === "/" : pathname.startsWith(href);
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
  <nav class="appnav" aria-label="주요 메뉴">
    {#each NAV_ITEMS as item (item.href)}
      <a
        href={item.href}
        class="navitem"
        aria-current={isActive(item.href) ? "page" : undefined}
      >
        {#if item.href === "/"}
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
            <path d="M4 19.5A2.5 2.5 0 0 1 6.5 17H20" />
            <path
              d="M6.5 2H20v20H6.5A2.5 2.5 0 0 1 4 19.5v-15A2.5 2.5 0 0 1 6.5 2Z"
            />
          </svg>
        {:else if item.href === "/import"}
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
            <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
            <path d="M7 10l5 5 5-5" />
            <path d="M12 15V3" />
          </svg>
        {:else}
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
            <circle cx="12" cy="12" r="3" />
            <path
              d="M19.4 15a1.7 1.7 0 0 0 .34 1.87l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.7 1.7 0 0 0-1.87-.34 1.7 1.7 0 0 0-1.03 1.56V21a2 2 0 1 1-4 0v-.09a1.7 1.7 0 0 0-1.11-1.56 1.7 1.7 0 0 0-1.87.34l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.7 1.7 0 0 0 .34-1.87 1.7 1.7 0 0 0-1.56-1.03H3a2 2 0 1 1 0-4h.09A1.7 1.7 0 0 0 4.65 8.9a1.7 1.7 0 0 0-.34-1.87l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.7 1.7 0 0 0 1.87.34h.09A1.7 1.7 0 0 0 10.13 3V3a2 2 0 1 1 4 0v.09a1.7 1.7 0 0 0 1.03 1.56 1.7 1.7 0 0 0 1.87-.34l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.7 1.7 0 0 0-.34 1.87v.09a1.7 1.7 0 0 0 1.56 1.03H21a2 2 0 1 1 0 4h-.09a1.7 1.7 0 0 0-1.51 1.03Z"
            />
          </svg>
        {/if}
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
    width: fit-content;
    bottom: calc(env(safe-area-inset-bottom, 0px) + var(--sp-3));
    display: flex;
    align-items: center;
    gap: var(--sp-1);
    height: var(--size-tabbar);
    padding: 0 var(--sp-2);
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
      env(safe-area-inset-bottom, 0px) + var(--size-tabbar) + var(--sp-3) +
        var(--sp-2)
    );
  }

  /* Android's committed wrapper already pads the WebView, so only the dock
     height may contribute here (mirrors the tokens.css inset-owner rule). */
  :global([data-native-inset-owner="android-view-padding"])
    .shell:not(.detail)
    .content {
    --safe-bottom: calc(var(--size-tabbar) + var(--sp-3) + var(--sp-2));
  }

  .shell.detail .appnav {
    display: none;
  }

  .navitem {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 0;
    width: 64px;
    height: 48px;
    padding: 0;
    border-radius: var(--r-pill);
    color: var(--text-faint);
    text-decoration: none;
    transition:
      color var(--dur-base) var(--ease-out),
      background var(--dur-slow) var(--ease-spring),
      transform var(--dur-base) var(--ease-spring);
  }

  .navitem:active {
    transform: scale(0.88);
  }

  .navitem span {
    max-height: 0;
    overflow: hidden;
    opacity: 0;
    white-space: nowrap;
    font-size: 10px;
    font-weight: 600;
    line-height: 1;
    letter-spacing: -0.01em;
    transition:
      max-height var(--dur-slow) var(--ease-spring),
      opacity var(--dur-base) var(--ease-out),
      margin var(--dur-slow) var(--ease-spring);
  }

  .navitem[aria-current="page"] {
    color: var(--tint);
    background: var(--tint-soft);
  }

  .navitem[aria-current="page"] span {
    max-height: 12px;
    margin-top: 3px;
    opacity: 1;
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

    .navitem {
      flex-direction: column;
      gap: 3px;
      align-self: stretch;
      height: 56px;
      margin: 0 var(--sp-2);
      padding: 0;
      border-radius: var(--r-block);
    }

    .navitem span,
    .navitem[aria-current="page"] span {
      max-height: none;
      margin-top: 0;
      opacity: 1;
      font-size: 10px;
    }

    .navitem:active {
      transform: scale(0.95);
    }
  }
</style>
