<script lang="ts">
  let {
    onSend,
    disabled = false,
    placeholder = "메시지 보내기",
  }: {
    onSend: (text: string) => void;
    disabled?: boolean;
    placeholder?: string;
  } = $props();

  let draft = $state("");

  const canSend = $derived(!disabled && draft.trim().length > 0);

  function submit(): void {
    const text = draft.trim();
    if (!text || disabled) {
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
</script>

<div class="composer">
  <textarea
    rows="1"
    {placeholder}
    enterkeyhint="send"
    bind:value={draft}
    onkeydown={handleKeydown}
    {disabled}
  ></textarea>
  <button
    type="button"
    class="send"
    onclick={submit}
    disabled={!canSend}
    aria-label="보내기"
  >
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
  </button>
</div>

<style>
  .composer {
    display: flex;
    align-items: flex-end;
    gap: var(--sp-2);
    padding: var(--sp-2) var(--sp-3)
      calc(var(--sp-3) + var(--safe-bottom));
    background: var(--surface-page);
    border-top: 0.5px solid var(--hairline);
  }

  textarea {
    flex: 1;
    min-height: var(--size-touch);
    max-height: calc(var(--fs-chat) * var(--lh-chat) * 5);
    box-sizing: border-box;
    resize: none;
    background: var(--surface-field);
    border: 0.5px solid var(--field-border);
    border-radius: calc(var(--size-touch) / 2);
    padding: 10px var(--sp-4);
    font-family: var(--font-ui);
    font-size: 16px;
    line-height: var(--lh-chat);
    color: var(--text-strong);
    outline: none;
    transition: border-color var(--dur-fast) var(--ease-out);
  }

  textarea::placeholder {
    color: var(--text-faint);
  }

  textarea:focus-visible {
    border-color: var(--text-mid);
  }

  .send {
    width: var(--size-touch);
    height: var(--size-touch);
    flex-shrink: 0;
    border: none;
    border-radius: var(--r-pill);
    background: var(--invert-surface);
    color: var(--invert-text);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    transition:
      opacity var(--dur-fast) var(--ease-out),
      transform var(--dur-fast) var(--ease-out);
  }

  .send:disabled {
    opacity: 0.35;
    cursor: default;
  }

  .send:not(:disabled):active {
    transform: scale(0.94);
  }
</style>
