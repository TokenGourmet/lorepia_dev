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

  it("migrates and persists a configuration-only provider selection", async () => {
    const vertexPreferences: VersionedAppPreferences = {
      ...initial,
      value: {
        ...initial.value,
        selectedProviderId: "google-vertex-ai",
      },
    };
    const applied: AppPreferences[] = [];
    const updateAppPreferences = vi.fn(
      async (_revision: number, value: AppPreferences) => ({
        revision: 4,
        value,
      }),
    );
    const controller = createAppPreferencesController(
      {
        getAppPreferences: vi.fn(async () => vertexPreferences),
        updateAppPreferences,
      },
      (value) => applied.push(value),
    );

    await controller.hydrate();
    await controller.flush();

    expect(controller.current.selectedProviderId).toBe("openai");
    expect(applied.at(0)?.selectedProviderId).toBe("openai");
    expect(updateAppPreferences).toHaveBeenCalledWith(
      3,
      expect.objectContaining({ selectedProviderId: "openai" }),
    );
    expect(controller.saving).toBe(false);
  });

  it("does not allow a configuration-only provider to be selected again", async () => {
    const updateAppPreferences = vi.fn();
    const controller = createAppPreferencesController(
      {
        getAppPreferences: vi.fn(async () => initial),
        updateAppPreferences,
      },
      () => undefined,
    );
    await controller.hydrate();

    controller.setProvider("google-vertex-ai");
    await controller.flush();

    expect(controller.current.selectedProviderId).toBe("anthropic");
    expect(updateAppPreferences).not.toHaveBeenCalled();
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

  it("retries a failed hydration without discarding the local edit", async () => {
    const getAppPreferences = vi
      .fn<() => Promise<VersionedAppPreferences>>()
      .mockRejectedValueOnce(new Error("storage offline"))
      .mockResolvedValueOnce(initial);
    const updateAppPreferences = vi.fn(
      async (_revision: number, value: AppPreferences) => ({
        revision: 4,
        value,
      }),
    );
    const controller = createAppPreferencesController(
      { getAppPreferences, updateAppPreferences },
      () => undefined,
    );

    await controller.hydrate();
    expect(controller.unavailable).toBe(true);

    controller.setTheme("light");
    expect(controller.saving).toBe(true);
    await controller.retry();

    expect(getAppPreferences).toHaveBeenCalledTimes(2);
    expect(updateAppPreferences).toHaveBeenCalledWith(
      3,
      expect.objectContaining({ theme: "light" }),
    );
    expect(controller.unavailable).toBe(false);
    expect(controller.saving).toBe(false);
    expect(controller.current.theme).toBe("light");
  });

  it("refreshes the revision before retrying a failed write", async () => {
    const getAppPreferences = vi
      .fn<() => Promise<VersionedAppPreferences>>()
      .mockResolvedValueOnce(initial)
      .mockResolvedValueOnce({ ...initial, revision: 8 });
    const updateAppPreferences = vi
      .fn<
        (
          revision: number,
          value: AppPreferences,
        ) => Promise<VersionedAppPreferences>
      >()
      .mockRejectedValueOnce(new Error("write failed"))
      .mockImplementationOnce(async (_revision, value) => ({
        revision: 9,
        value,
      }));
    const controller = createAppPreferencesController(
      { getAppPreferences, updateAppPreferences },
      () => undefined,
    );
    await controller.hydrate();

    controller.setDefaultMode("chat");
    expect(controller.saving).toBe(true);
    await controller.flush();
    expect(controller.unavailable).toBe(true);
    expect(controller.saving).toBe(false);

    await controller.retry();

    expect(getAppPreferences).toHaveBeenCalledTimes(2);
    expect(updateAppPreferences).toHaveBeenLastCalledWith(
      8,
      expect.objectContaining({ defaultMode: "chat" }),
    );
    expect(controller.unavailable).toBe(false);
    expect(controller.saving).toBe(false);
  });
});
