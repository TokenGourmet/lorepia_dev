import type { ThreadMode } from "$lib/chat/types";
import { theme, type ThemePreference } from "$lib/design/theme.svelte";
import { activeProviderProfile } from "$lib/providers/active-profile.svelte";
import type {
  ApiKeyProviderId,
  LlmProviderId,
} from "$lib/providers/catalog";

import { storageClient, type AppPreferences } from "./client";

const MODEL_SAVE_DELAY_MS = 250;

type PreferencesClient = Pick<
  typeof storageClient,
  "getAppPreferences" | "updateAppPreferences"
>;

type ApplyPreferences = (value: AppPreferences) => void;

const DEFAULT_PREFERENCES: AppPreferences = Object.freeze({
  selectedProviderId: "openai",
  modelIds: Object.freeze({}),
  theme: "system",
  defaultMode: "chat",
});

function copyPreferences(value: AppPreferences): AppPreferences {
  return Object.freeze({
    selectedProviderId: value.selectedProviderId,
    modelIds: Object.freeze({ ...value.modelIds }),
    theme: value.theme,
    defaultMode: value.defaultMode,
  });
}

function applyToProduct(value: AppPreferences): void {
  theme.set(value.theme);
  activeProviderProfile.restoreNonSecretSettings(
    value.selectedProviderId,
    value.modelIds,
  );
}

export function createAppPreferencesController(
  client: PreferencesClient = storageClient,
  apply: ApplyPreferences = applyToProduct,
) {
  let current = $state<AppPreferences>(DEFAULT_PREFERENCES);
  let hydrated = $state(false);
  let unavailable = $state(false);
  let revision = 0;
  let generation = 0;
  let persistedGeneration = 0;
  let hydration: Promise<void> | null = null;
  let writeTail: Promise<void> = Promise.resolve();
  let timer: ReturnType<typeof setTimeout> | null = null;
  const touchedFields = new Set<
    "selectedProviderId" | "theme" | "defaultMode"
  >();
  const touchedModelIds = new Set<ApiKeyProviderId>();

  const setCurrent = (value: AppPreferences): void => {
    current = copyPreferences(value);
    apply(current);
  };

  const clearTimer = (): void => {
    if (timer !== null) {
      clearTimeout(timer);
      timer = null;
    }
  };

  const mergeHydratedPreferences = (
    loaded: AppPreferences,
  ): AppPreferences => {
    const modelIds: Partial<Record<ApiKeyProviderId, string>> = {
      ...loaded.modelIds,
    };
    for (const providerId of touchedModelIds) {
      const localModelId = current.modelIds[providerId];
      if (localModelId === undefined) {
        delete modelIds[providerId];
      } else {
        modelIds[providerId] = localModelId;
      }
    }

    return {
      selectedProviderId: touchedFields.has("selectedProviderId")
        ? current.selectedProviderId
        : loaded.selectedProviderId,
      modelIds,
      theme: touchedFields.has("theme") ? current.theme : loaded.theme,
      defaultMode: touchedFields.has("defaultMode")
        ? current.defaultMode
        : loaded.defaultMode,
    };
  };

  const hydrate = (): Promise<void> => {
    hydration ??= (async () => {
      try {
        const loaded = await client.getAppPreferences();
        revision = loaded.revision;
        hydrated = true;
        unavailable = false;
        setCurrent(mergeHydratedPreferences(loaded.value));
        if (generation === 0) {
          persistedGeneration = generation;
        } else {
          scheduleWrite(0);
        }
      } catch {
        hydrated = true;
        unavailable = true;
      }
    })();
    return hydration;
  };

  const queueWrite = (): Promise<void> => {
    clearTimer();
    writeTail = writeTail
      .catch(() => undefined)
      .then(async () => {
        if (!hydrated) {
          await hydrate();
        }
        if (unavailable || persistedGeneration >= generation) return;

        const writeGeneration = generation;
        const writeValue = copyPreferences(current);
        const saved = await client.updateAppPreferences(revision, writeValue);
        revision = saved.revision;
        persistedGeneration = writeGeneration;
        if (generation === writeGeneration) {
          setCurrent(saved.value);
        }
      })
      .catch(() => {
        unavailable = true;
      });
    return writeTail;
  };

  function scheduleWrite(delay: number): void {
    clearTimer();
    timer = setTimeout(() => {
      timer = null;
      void queueWrite();
    }, delay);
    const pending = timer as unknown as { unref?: () => void };
    pending.unref?.();
  }

  const update = (
    change: Partial<Pick<AppPreferences, "theme" | "defaultMode">> & {
      selectedProviderId?: LlmProviderId;
      modelIds?: Readonly<Partial<Record<ApiKeyProviderId, string>>>;
    },
    delay = 0,
    touchedModelId?: ApiKeyProviderId,
  ): void => {
    if (change.selectedProviderId !== undefined) {
      touchedFields.add("selectedProviderId");
    }
    if (change.theme !== undefined) touchedFields.add("theme");
    if (change.defaultMode !== undefined) touchedFields.add("defaultMode");
    if (touchedModelId !== undefined) touchedModelIds.add(touchedModelId);
    generation += 1;
    setCurrent({
      ...current,
      ...change,
      modelIds: change.modelIds ?? current.modelIds,
    });
    void hydrate();
    scheduleWrite(delay);
  };

  return {
    get current(): AppPreferences {
      return current;
    },
    get hydrated(): boolean {
      return hydrated;
    },
    get unavailable(): boolean {
      return unavailable;
    },
    hydrate,
    setProvider(providerId: LlmProviderId): void {
      update({ selectedProviderId: providerId });
    },
    setModelId(providerId: ApiKeyProviderId, modelId: string): void {
      update(
        { modelIds: { ...current.modelIds, [providerId]: modelId } },
        MODEL_SAVE_DELAY_MS,
        providerId,
      );
    },
    setTheme(value: ThemePreference): void {
      update({ theme: value });
    },
    setDefaultMode(value: ThreadMode): void {
      update({ defaultMode: value });
    },
    async flush(): Promise<void> {
      clearTimer();
      await queueWrite();
    },
  };
}

export const appPreferences = createAppPreferencesController();
