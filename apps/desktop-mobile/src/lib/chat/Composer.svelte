<script lang="ts">
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

<div class="composer">
  <button class="extra" type="button" disabled aria-label="첨부 (준비 중)">
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
      <path d="M12 5v14" />
      <path d="M5 12h14" />
    </svg>
  </button>
  <div class="field">
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
        <svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true">
          <rect x="7" y="7" width="10" height="10" rx="1.5" fill="currentColor" />
        </svg>
      {:else}
        <svg
          viewBox="0 0 24 24"
          width="18"
          height="18"
          fill="none"
          stroke="currentColor"
          stroke-width="2.4"
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
</div>

<style>
  .composer {
    display: flex;
    align-items: flex-end;
    gap: var(--sp-2);
    width: var(--dock-width);
    min-height: var(--size-tabbar);
    box-sizing: border-box;
    margin: var(--sp-2) auto 0;
    /* A 44px field plus this padding and the hairline resolves to the dock's
       52px resting height without shrinking the touch-sized input area. */
    padding: 3px;
    background: var(--surface-card);
    border: 0.5px solid var(--hairline);
    border-radius: 27px;
    box-shadow: var(--shadow-float);
    animation: lp-rise var(--dur-page) var(--ease-spring) backwards;
  }

  @media (min-width: 700px) {
    .composer {
      width: min(100% - var(--sp-3) * 2, 760px);
      margin-inline: auto;
      box-sizing: border-box;
    }
  }

  .extra {
    width: 34px;
    height: 34px;
    margin: 5px 0 5px 5px;
    flex-shrink: 0;
    border: none;
    border-radius: var(--r-pill);
    background: var(--surface-bubble);
    color: var(--text-mid);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    transition: transform var(--dur-base) var(--ease-spring);
  }

  .extra:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .field {
    flex: 1;
    position: relative;
    display: flex;
    min-width: 0;
  }

  textarea {
    flex: 1;
    min-height: var(--size-touch);
    max-height: calc(var(--fs-chat) * var(--lh-chat) * 5);
    box-sizing: border-box;
    resize: none;
    background: transparent;
    border: none;
    padding: 11px 46px 11px var(--sp-2);
    font-family: var(--font-ui);
    font-size: 16px;
    /* 22px type plus 11px vertical insets resolves to the shared 44px
       control height instead of making the 52px outer capsule grow. */
    line-height: 22px;
    color: var(--text-strong);
    outline: none;
  }

  textarea::placeholder {
    color: var(--text-faint);
  }

  .send {
    position: absolute;
    right: 5px;
    bottom: 5px;
    width: 34px;
    height: 34px;
    border: none;
    border-radius: var(--r-pill);
    background: var(--tint);
    color: #fff;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    transition:
      opacity var(--dur-fast) var(--ease-out),
      background var(--dur-base) var(--ease-out),
      color var(--dur-base) var(--ease-out),
      transform var(--dur-base) var(--ease-spring);
  }

  .send:disabled {
    opacity: 0;
    transform: scale(0.5);
    cursor: default;
    pointer-events: none;
  }

  .send:not(:disabled):active {
    transform: scale(0.88);
  }

  .send.stop {
    background: var(--tint-soft);
    color: var(--tint);
  }
</style>
