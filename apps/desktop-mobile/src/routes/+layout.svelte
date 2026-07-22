<script lang="ts">
  import { onMount } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";

  import { activeProviderProfile } from "$lib/providers/active-profile.svelte";
  import { getLlmProvider } from "$lib/providers/catalog";
  import { requestCredentialStatus } from "$lib/providers/credentials";
  import { appPreferences } from "$lib/storage/app-preferences.svelte";
  import { createPreferenceCloseHandler } from "$lib/storage/preference-close-guard";

  let { children } = $props();

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

{@render children()}
