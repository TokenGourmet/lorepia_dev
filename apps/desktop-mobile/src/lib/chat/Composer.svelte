<script lang="ts">
  import {
    resolveComposerNotice,
    resolveComposerSubmission,
  } from "./composer-submission";

  let {
    onSend,
    onCancel,
    busy = false,
    blockedReason = null,
    validate,
    maxLength,
    placeholder = "메시지 보내기",
  }: {
    onSend: (text: string) => boolean | void;
    onCancel?: () => void;
    busy?: boolean;
    blockedReason?: string | null;
    validate?: (text: string) => string | null;
    maxLength?: number;
    placeholder?: string;
  } = $props();

  let draft = $state("");
  let noticeRequested = $state(false);
  let inputNoticeReason = $state<string | null>(null);
  let noticeRevision = $state(0);

  const canSend = $derived(!busy && draft.trim().length > 0);
  const visibleNotice = $derived(
    resolveComposerNotice(
      noticeRequested,
      blockedReason,
      inputNoticeReason,
    ),
  );

  function submit(): void {
    const normalizedDraft = draft.trim();
    const inputBlockReason =
      normalizedDraft.length > 0 ? validate?.(normalizedDraft) : null;
    const submission = resolveComposerSubmission(
      draft,
      busy,
      blockedReason ?? inputBlockReason,
    );
    if (submission.kind === "ignore") {
      return;
    }
    if (submission.kind === "blocked") {
      noticeRequested = true;
      // Readiness can change after the click, so keep only input validation
      // locally and let the provider/storage reason stay reactive.
      inputNoticeReason = inputBlockReason ?? null;
      noticeRevision += 1;
      return;
    }

    const accepted = onSend(submission.text);
    if (accepted === false) {
      // Provider or storage readiness may change between render and click.
      // Keep the draft and let the newly reactive reason appear.
      noticeRequested = true;
      inputNoticeReason = null;
      noticeRevision += 1;
      return;
    }

    noticeRequested = false;
    inputNoticeReason = null;
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

  function handleInput(): void {
    noticeRequested = false;
    inputNoticeReason = null;
  }
</script>

<div class="composer-shell">
  {#if visibleNotice}
    {#key noticeRevision}
      <p class="send-notice" role="status" aria-live="polite">
        <svg
          viewBox="0 0 24 24"
          width="16"
          height="16"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
          aria-hidden="true"
        >
          <circle cx="12" cy="12" r="9" />
          <path d="M12 8v5" />
          <path d="M12 16.5h.01" />
        </svg>
        <span>{visibleNotice}</span>
      </p>
    {/key}
  {/if}

  <div class="composer-row">
    <div class="field">
      <!-- Add belongs to the composer capsule. Its visual is deliberately
           unframed, while the complete 44px leading slot remains a real
           button target for the future attachment surface. -->
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
      <textarea
        rows="1"
        {placeholder}
        enterkeyhint="send"
        maxlength={maxLength}
        bind:value={draft}
        oninput={handleInput}
        onkeydown={handleKeydown}
        disabled={busy}
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
</div>

<style>
  .composer-shell {
    position: relative;
    width: var(--dock-width);
    box-sizing: border-box;
    margin: var(--sp-2) auto 0;
    animation: lp-rise var(--dur-page) var(--ease-spring) backwards;
  }

  @media (min-width: 700px) {
    .composer-shell {
      width: min(100% - var(--sp-3) * 2, 760px);
      margin-inline: auto;
      box-sizing: border-box;
    }
  }

  .composer-row {
    display: flex;
    align-items: center;
    min-height: var(--size-tabbar);
  }

  .send-notice {
    position: absolute;
    z-index: 2;
    right: 0;
    bottom: calc(100% + var(--sp-1));
    left: 0;
    display: flex;
    align-items: flex-start;
    gap: var(--sp-2);
    box-sizing: border-box;
    margin: 0;
    padding: 10px 12px;
    border: 0.5px solid color-mix(in srgb, var(--danger) 28%, transparent);
    border-radius: 16px;
    background: color-mix(in srgb, var(--surface-card) 94%, var(--danger) 6%);
    box-shadow: var(--shadow-float);
    color: var(--danger);
    font-size: var(--fs-label);
    line-height: 1.35;
    animation: notice-in var(--dur-base) var(--ease-spring) both;
  }

  .send-notice svg {
    flex: 0 0 auto;
    margin-top: 1px;
  }

  .extra {
    position: absolute;
    z-index: 1;
    left: 0;
    bottom: 0;
    width: var(--size-touch);
    height: var(--size-touch);
    box-sizing: border-box;
    flex-shrink: 0;
    border: 0;
    border-radius: 0;
    background: transparent;
    color: var(--text-mid);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    padding: 0;
    transition:
      color var(--dur-fast) var(--ease-out),
      opacity var(--dur-fast) var(--ease-out);
  }

  .extra:disabled {
    opacity: 0.52;
    cursor: default;
  }

  .field {
    flex: 1;
    position: relative;
    display: flex;
    min-width: 0;
    min-height: var(--size-touch);
    border: 0.5px solid var(--hairline);
    border-radius: var(--r-pill);
    background: var(--bar-bg);
    -webkit-backdrop-filter: blur(20px) saturate(1.6);
    backdrop-filter: blur(20px) saturate(1.6);
    box-shadow: var(--shadow-float);
  }

  textarea {
    flex: 1;
    min-height: var(--size-touch);
    max-height: calc(var(--fs-chat) * var(--lh-chat) * 5);
    box-sizing: border-box;
    resize: none;
    background: transparent;
    border: none;
    padding:
      11px
      calc(var(--size-touch) + 2px)
      11px
      calc(var(--size-touch) + 2px);
    font-family: var(--font-ui);
    font-size: 16px;
    line-height: 22px;
    color: var(--text-strong);
    outline: none;
  }

  textarea::placeholder {
    color: var(--text-faint);
  }

  .send {
    position: absolute;
    z-index: 1;
    right: 0;
    bottom: 0;
    width: var(--size-touch);
    height: var(--size-touch);
    border: none;
    border-radius: var(--r-pill);
    background: transparent;
    color: #fff;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    transition:
      opacity var(--dur-fast) var(--ease-out),
      color var(--dur-base) var(--ease-out),
      transform var(--dur-base) var(--ease-spring);
  }

  .send::before {
    content: "";
    position: absolute;
    inset: 5px;
    border-radius: inherit;
    background: var(--tint);
    transition:
      background var(--dur-base) var(--ease-out),
      transform var(--dur-base) var(--ease-spring);
  }

  .send svg {
    position: relative;
    z-index: 1;
  }

  .send:disabled {
    opacity: 0;
    cursor: default;
    pointer-events: none;
  }

  .send:disabled::before {
    transform: scale(0.88);
  }

  .send:not(:disabled):active::before {
    transform: scale(0.88);
  }

  .send.stop::before {
    background: var(--tint-soft);
  }

  .send.stop {
    color: var(--tint);
  }

  @keyframes notice-in {
    from {
      opacity: 0;
      translate: 0 7px;
      scale: 0.98;
    }
    to {
      opacity: 1;
      translate: 0 0;
      scale: 1;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .composer-shell,
    .send-notice {
      animation-duration: 1ms;
    }
  }
</style>
