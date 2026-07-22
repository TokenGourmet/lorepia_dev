import { describe, expect, it, vi } from "vitest";

import type { AppPreferences, VersionedAppPreferences } from "./client";
import { createAppPreferencesController } from "./app-preferences.svelte";

const initial: VersionedAppPreferences = {
  revision: 3,
  value: {
    selectedProviderId: "anthropic",
    modelIds: { anthropic: "claude-old" },
    theme: "dark",
    defaultMode: "story",
  },
};

describe("app preferences controller", () => {
  it("hydrates the typed non-secret product settings", async () => {
    const applied: AppPreferences[] = [];
    const controller = createAppPreferencesController(
      {
        getAppPreferences: vi.fn(async () => initial),
        updateAppPreferences: vi.fn(),
      },
      (value) => applied.push(value),
    );

    await controller.hydrate();

    expect(controller.current).toEqual(initial.value);
    expect(applied).toEqual([initial.value]);
    expect(JSON.stringify(controller.current)).not.toMatch(
      /credential|api.?key|control.?token/i,
    );
  });

  it("preserves edits made while hydration is in flight", async () => {
    let resolveLoad: (value: VersionedAppPreferences) => void = () => undefined;
    const load = new Promise<VersionedAppPreferences>((resolve) => {
      resolveLoad = resolve;
    });
    const updateAppPreferences = vi.fn(
      async (_revision: number, value: AppPreferences) => ({
        revision: 4,
        value,
      }),
    );
    const controller = createAppPreferencesController(
      {
        getAppPreferences: () => load,
        updateAppPreferences,
      },
      () => undefined,
    );

    const hydrating = controller.hydrate();
    controller.setTheme("light");
    resolveLoad(initial);
    await hydrating;
    await controller.flush();

    expect(controller.current).toEqual({
      selectedProviderId: "anthropic",
      modelIds: { anthropic: "claude-old" },
      theme: "light",
      defaultMode: "story",
    });
    expect(updateAppPreferences).toHaveBeenCalledWith(
      3,
      {
        selectedProviderId: "anthropic",
        modelIds: { anthropic: "claude-old" },
        theme: "light",
        defaultMode: "story",
      },
    );
  });

  it("merges persisted fields when the first edit starts hydration", async () => {
    let resolveLoad: (value: VersionedAppPreferences) => void = () => undefined;
    const load = new Promise<VersionedAppPreferences>((resolve) => {
      resolveLoad = resolve;
    });
    const updateAppPreferences = vi.fn(
      async (_revision: number, value: AppPreferences) => ({
        revision: 4,
        value,
      }),
    );
    const controller = createAppPreferencesController(
      {
        getAppPreferences: () => load,
        updateAppPreferences,
      },
      () => undefined,
    );

    controller.setTheme("light");
    resolveLoad(initial);
    await controller.flush();

    expect(controller.current).toEqual({
      selectedProviderId: "anthropic",
      modelIds: { anthropic: "claude-old" },
      theme: "light",
      defaultMode: "story",
    });
    expect(updateAppPreferences).toHaveBeenCalledWith(3, controller.current);
  });

  it("overlays only the model id edited during hydration", async () => {
    let resolveLoad: (value: VersionedAppPreferences) => void = () => undefined;
    const load = new Promise<VersionedAppPreferences>((resolve) => {
      resolveLoad = resolve;
    });
    const updateAppPreferences = vi.fn(
      async (_revision: number, value: AppPreferences) => ({
        revision: 4,
        value,
      }),
    );
    const controller = createAppPreferencesController(
      {
        getAppPreferences: () => load,
        updateAppPreferences,
      },
      () => undefined,
    );

    const hydrating = controller.hydrate();
    controller.setModelId("openai", "gpt-new");
    resolveLoad({
      ...initial,
      value: {
        ...initial.value,
        modelIds: {
          anthropic: "claude-old",
          openai: "gpt-old",
          deepseek: "deepseek-old",
        },
      },
    });
    await hydrating;
    await controller.flush();

    expect(controller.current.modelIds).toEqual({
      anthropic: "claude-old",
      openai: "gpt-new",
      deepseek: "deepseek-old",
    });
  });

  it("flushes a debounced model edit without waiting for its timer", async () => {
    vi.useFakeTimers();
    try {
      const updateAppPreferences = vi.fn(
        async (_revision: number, value: AppPreferences) => ({
          revision: 4,
          value,
        }),
      );
      const controller = createAppPreferencesController(
        {
          getAppPreferences: vi.fn(async () => initial),
          updateAppPreferences,
        },
        () => undefined,
      );
      await controller.hydrate();

      controller.setModelId("anthropic", "claude-new");
      expect(updateAppPreferences).not.toHaveBeenCalled();
      await controller.flush();

      expect(updateAppPreferences).toHaveBeenCalledOnce();
      expect(controller.current.modelIds.anthropic).toBe("claude-new");
      expect(vi.getTimerCount()).toBe(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it("serializes writes with optimistic revisions", async () => {
    let revision = 0;
    const revisions: number[] = [];
    const controller = createAppPreferencesController(
      {
        getAppPreferences: vi.fn(async (): Promise<VersionedAppPreferences> => ({
          revision,
          value: {
            selectedProviderId: "openai",
            modelIds: {},
            theme: "system",
            defaultMode: "chat",
          },
        })),
        updateAppPreferences: vi.fn(async (expected, value) => {
          revisions.push(expected);
          revision += 1;
          return { revision, value };
        }),
      },
      () => undefined,
    );
    await controller.hydrate();

    controller.setProvider("deepseek");
    await controller.flush();
    controller.setDefaultMode("story");
    await controller.flush();

    expect(revisions).toEqual([0, 1]);
    expect(controller.current).toMatchObject({
      selectedProviderId: "deepseek",
      defaultMode: "story",
    });
  });
});
