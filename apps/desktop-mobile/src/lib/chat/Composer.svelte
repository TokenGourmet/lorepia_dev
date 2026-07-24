<script lang="ts">
  import LiquidGlass from "$lib/ui/LiquidGlass.svelte";

  let {
    onSend,
    onCancel,
    busy = false,
    disabled = false,
    maxLength,
    placeholder = "메시지 보내기",
  }: {
    onSend: (text: string) => void;
    onCancel?: () => void;
    busy?: boolean;
    disabled?: boolean;
    maxLength?: number;
    placeholder?: string;
  } = $props();

  let draft = $state("");

  const canSend = $derived(!disabled && !busy && draft.trim().length > 0);

  function submit(): void {
    const text = draft.trim();
    if (!text || disabled || busy) {
      return;
    }
    onSend(text);
    draft = "";
  }

  function handleKeydown(event: KeyboardEvent): void {
    if (event.key === "Enter" && !event.shiftKey && !event.isComposing) {
      event.preventDefault();
      submit();
    }
  }

  function activate(): void {
    if (busy) {
      onCancel?.();
      return;
    }
    submit();
  }
</script>

<div class="composer-shell">
  <LiquidGlass variant="chrome" shape="pill" interactive={true}>
    <div class="composer">
      <textarea
        rows="1"
        {placeholder}
        enterkeyhint="send"
        maxlength={maxLength}
        bind:value={draft}
        onkeydown={handleKeydown}
        disabled={disabled || busy}
      ></textarea>
      <button
        type="button"
        class="send"
        class:stop={busy}
        onclick={activate}
        disabled={busy ? onCancel === undefined : !canSend}
        aria-label={busy ? "응답 중지" : "보내기"}
      >
        {#if busy}
          <svg viewBox="0 0 24 24" width="20" height="20" aria-hidden="true">
            <rect x="7" y="7" width="10" height="10" rx="1" fill="currentColor" />
          </svg>
        {:else}
          <svg
            viewBox="0 0 24 24"
            width="20"
            height="20"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
            aria-hidden="true"
          >
            <path d="M12 19V5" />
            <path d="m5 12 7-7 7 7" />
          </svg>
        {/if}
      </button>
    </div>
  </LiquidGlass>
</div>

<style>
  .composer-shell {
    padding: var(--sp-2) var(--sp-3)
      calc(var(--sp-3) + var(--safe-bottom));
    background: linear-gradient(to top, var(--surface-page), transparent);
  }

  .composer {
    display: flex;
    align-items: flex-end;
    gap: var(--sp-2);
    min-height: 52px;
    box-sizing: border-box;
    padding: 4px 5px 4px var(--sp-4);
  }

  textarea {
    flex: 1;
    min-width: 0;
    min-height: var(--size-touch);
    max-height: calc(var(--fs-chat) * var(--lh-chat) * 5);
    box-sizing: border-box;
    resize: none;
    overflow-y: auto;
    background: transparent;
    border: none;
    border-radius: 0;
    padding: 10px 0;
    font-family: var(--font-ui);
    font-size: 16px;
    line-height: var(--lh-chat);
    color: var(--text-strong);
    caret-color: var(--cursor-color);
    outline: none;
  }

  textarea::placeholder {
    color: var(--text-faint);
  }

  textarea:disabled {
    opacity: 0.72;
  }

  .send {
    width: var(--size-touch);
    height: var(--size-touch);
    flex-shrink: 0;
    border: 1px solid transparent;
    border-radius: var(--r-pill);
    background: var(--invert-surface);
    color: var(--invert-text);
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.12);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    transition:
      opacity var(--dur-fast) var(--ease-out),
      transform 360ms var(--ease-liquid),
      background-color var(--dur-fast) var(--ease-out),
      border-color var(--dur-fast) var(--ease-out),
      box-shadow 360ms var(--ease-liquid);
  }

  .send:disabled {
    opacity: 0.32;
    cursor: default;
    box-shadow: none;
  }

  .send:not(:disabled):active {
    transform: translateY(1px) scale(0.91);
    box-shadow: 0 1px 4px rgba(0, 0, 0, 0.1);
    transition-duration: 72ms;
  }

  .send.stop {
    background: var(--glass-control-fill);
    border-color: var(--glass-control-border);
    color: var(--text-strong);
    box-shadow:
      inset 0 1px 0 var(--glass-specular),
      0 4px 12px rgba(0, 0, 0, 0.08);
  }

  @media (prefers-reduced-motion: reduce) {
    .send {
      transition: none;
    }
  }
</style>
