<script lang="ts">
  import { onMount } from "svelte";
  import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";

  import {
    DEV_SIZE_LIMITS,
    DEV_SIZE_PRESETS,
    normalizeLogicalSize,
    type LogicalSizePreset,
  } from "./sizes";

  let open = $state(false);
  let applying = $state(false);
  let requestedWidth = $state(360);
  let requestedHeight = $state(780);
  let viewportWidth = $state(0);
  let viewportHeight = $state(0);
  let message = $state("");

  function syncViewportSize(): void {
    viewportWidth = window.innerWidth;
    viewportHeight = window.innerHeight;
  }

  onMount(() => {
    syncViewportSize();
    window.addEventListener("resize", syncViewportSize);

    return () => window.removeEventListener("resize", syncViewportSize);
  });

  function toggle(): void {
    open = !open;
    message = "";
  }

  function closeOnEscape(event: KeyboardEvent): void {
    if (event.key === "Escape" && open) {
      open = false;
    }
  }

  async function applySize(width: number, height: number): Promise<void> {
    const normalized = normalizeLogicalSize(width, height);
    requestedWidth = normalized.width;
    requestedHeight = normalized.height;
    applying = true;
    message = "";

    try {
      const appWindow = getCurrentWindow();
      await appWindow.setSize(
        new LogicalSize(normalized.width, normalized.height),
      );
      await appWindow.center();
      message = `${normalized.width} × ${normalized.height} 적용`;
    } catch {
      message = "개발용 Tauri 실행에서만 크기를 바꿀 수 있어요.";
    } finally {
      applying = false;
    }
  }

  function applyPreset(preset: LogicalSizePreset): void {
    void applySize(preset.width, preset.height);
  }
</script>

<svelte:window onkeydown={closeOnEscape} />

<aside class="dev-size-tool" aria-label="개발용 창 크기 도구">
  {#if open}
    <section class="panel" id="dev-size-panel">
      <header>
        <div>
          <strong>Viewport lab</strong>
          <span>DEV ONLY</span>
        </div>
        <button class="close" type="button" aria-label="크기 도구 닫기" onclick={toggle}>
          <svg
            viewBox="0 0 24 24"
            width="17"
            height="17"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            aria-hidden="true"
          >
            <path d="M6 6l12 12M18 6 6 18" />
          </svg>
        </button>
      </header>

      <p class="current">
        현재 콘텐츠 <strong>{viewportWidth} × {viewportHeight}</strong>
      </p>

      <div class="presets" aria-label="창 크기 프리셋">
        {#each DEV_SIZE_PRESETS as preset (preset.id)}
          <button
            type="button"
            class:active={viewportWidth === preset.width && viewportHeight === preset.height}
            disabled={applying}
            onclick={() => applyPreset(preset)}
          >
            <span>{preset.label}</span>
            <strong>{preset.width} × {preset.height}</strong>
            <small>{preset.hint}</small>
          </button>
        {/each}
      </div>

      <div class="custom">
        <label>
          <span>너비</span>
          <input
            type="number"
            min={DEV_SIZE_LIMITS.minWidth}
            max={DEV_SIZE_LIMITS.maxWidth}
            bind:value={requestedWidth}
          />
        </label>
        <span class="times" aria-hidden="true">×</span>
        <label>
          <span>높이</span>
          <input
            type="number"
            min={DEV_SIZE_LIMITS.minHeight}
            max={DEV_SIZE_LIMITS.maxHeight}
            bind:value={requestedHeight}
          />
        </label>
        <button
          class="apply"
          type="button"
          disabled={applying}
          onclick={() => void applySize(requestedWidth, requestedHeight)}
        >
          {applying ? "변경 중" : "적용"}
        </button>
      </div>

      {#if message}
        <p class="message" role="status">{message}</p>
      {/if}
    </section>
  {:else}
    <button
      class="launcher"
      type="button"
      aria-label="개발용 창 크기 도구 열기"
      aria-expanded="false"
      aria-controls="dev-size-panel"
      onclick={toggle}
    >
      <svg
        viewBox="0 0 24 24"
        width="17"
        height="17"
        fill="none"
        stroke="currentColor"
        stroke-width="1.8"
        stroke-linecap="round"
        stroke-linejoin="round"
        aria-hidden="true"
      >
        <path d="M4 8V4h4M16 4h4v4M20 16v4h-4M8 20H4v-4" />
        <path d="M9 12h6M12 9v6" />
      </svg>
      <span>DEV</span>
    </button>
  {/if}
</aside>

<style>
  .dev-size-tool {
    position: fixed;
    z-index: 2147483000;
    top: max(calc(env(safe-area-inset-top, 0px) + 8px), 8px);
    right: 8px;
    font-family:
      ui-sans-serif, -apple-system, BlinkMacSystemFont, "SF Pro Text", sans-serif;
    color: #f8fafc;
  }

  button,
  input {
    font: inherit;
  }

  button {
    -webkit-tap-highlight-color: transparent;
  }

  .launcher {
    min-width: 54px;
    height: 36px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 5px;
    padding: 0 9px;
    border: 1px solid rgba(255, 255, 255, 0.2);
    border-radius: 10px;
    background: rgba(15, 23, 42, 0.88);
    color: #f8fafc;
    box-shadow: 0 8px 24px rgba(15, 23, 42, 0.22);
    -webkit-backdrop-filter: blur(16px);
    backdrop-filter: blur(16px);
    cursor: pointer;
  }

  .launcher span {
    font-size: 9px;
    font-weight: 800;
    letter-spacing: 0.08em;
  }

  .panel {
    width: min(286px, calc(100vw - 16px));
    box-sizing: border-box;
    padding: 12px;
    border: 1px solid rgba(255, 255, 255, 0.16);
    border-radius: 16px;
    background: rgba(15, 23, 42, 0.94);
    box-shadow: 0 18px 50px rgba(2, 6, 23, 0.34);
    -webkit-backdrop-filter: blur(24px) saturate(1.25);
    backdrop-filter: blur(24px) saturate(1.25);
  }

  header,
  header > div,
  .custom {
    display: flex;
    align-items: center;
  }

  header {
    justify-content: space-between;
    gap: 8px;
  }

  header > div {
    gap: 7px;
  }

  header strong {
    font-size: 14px;
    line-height: 1;
  }

  header span {
    padding: 3px 5px;
    border-radius: 5px;
    background: #f97316;
    color: #fff7ed;
    font-size: 8px;
    font-weight: 800;
    line-height: 1;
    letter-spacing: 0.08em;
  }

  .close {
    width: 32px;
    height: 32px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    border: 0;
    border-radius: 9px;
    background: rgba(255, 255, 255, 0.08);
    color: #cbd5e1;
    cursor: pointer;
  }

  .current {
    margin: 9px 0 10px;
    color: #94a3b8;
    font-size: 11px;
  }

  .current strong {
    color: #e2e8f0;
    font-variant-numeric: tabular-nums;
  }

  .presets {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 6px;
  }

  .presets button {
    min-width: 0;
    min-height: 64px;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    justify-content: center;
    padding: 8px 9px;
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: 10px;
    background: rgba(255, 255, 255, 0.055);
    color: #e2e8f0;
    text-align: left;
    cursor: pointer;
  }

  .presets button.active {
    border-color: rgba(56, 189, 248, 0.72);
    background: rgba(14, 165, 233, 0.16);
  }

  .presets button span {
    overflow: hidden;
    max-width: 100%;
    color: #cbd5e1;
    font-size: 10px;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .presets button strong {
    margin-top: 2px;
    font-size: 12px;
    font-variant-numeric: tabular-nums;
  }

  .presets button small {
    margin-top: 2px;
    color: #64748b;
    font-size: 9px;
  }

  .custom {
    gap: 5px;
    margin-top: 9px;
  }

  .custom label {
    flex: 1;
    min-width: 0;
  }

  .custom label span {
    display: block;
    margin: 0 0 3px 2px;
    color: #94a3b8;
    font-size: 9px;
  }

  .custom input {
    width: 100%;
    height: 34px;
    box-sizing: border-box;
    padding: 0 7px;
    border: 1px solid rgba(255, 255, 255, 0.14);
    border-radius: 8px;
    outline: none;
    background: rgba(2, 6, 23, 0.52);
    color: #f8fafc;
    font-size: 11px;
    font-variant-numeric: tabular-nums;
  }

  .custom input:focus {
    border-color: #38bdf8;
  }

  .times {
    align-self: flex-end;
    padding-bottom: 8px;
    color: #64748b;
    font-size: 11px;
  }

  .apply {
    align-self: flex-end;
    height: 34px;
    padding: 0 10px;
    border: 0;
    border-radius: 8px;
    background: #0ea5e9;
    color: #f0f9ff;
    font-size: 10px;
    font-weight: 700;
    cursor: pointer;
  }

  button:disabled {
    opacity: 0.48;
    cursor: wait;
  }

  .message {
    margin: 8px 1px 0;
    color: #bae6fd;
    font-size: 10px;
    line-height: 1.35;
  }

  @media (hover: hover) {
    .launcher:hover,
    .close:hover,
    .presets button:hover {
      border-color: rgba(125, 211, 252, 0.6);
    }
  }

  @media (prefers-reduced-motion: no-preference) {
    .launcher,
    .panel {
      animation: dev-tool-in 140ms ease-out backwards;
    }

    @keyframes dev-tool-in {
      from {
        opacity: 0;
        translate: 0 -4px;
      }
    }
  }
</style>
