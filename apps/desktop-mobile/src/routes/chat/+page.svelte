<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { goto } from "$app/navigation";

  import "$lib/design/tokens.css";

  import Composer from "$lib/chat/Composer.svelte";
  import {
    createFrameChunkBuffer,
    type FrameChunkBuffer,
  } from "$lib/chat/frame-chunk-buffer";
  import MessageThread from "$lib/chat/MessageThread.svelte";
  import type { ChatMessage, ThreadMode } from "$lib/chat/types";
  import { keyboardInset } from "$lib/design/keyboard-inset.svelte";
  import { activeProviderProfile } from "$lib/providers/active-profile.svelte";
  import {
    resetProviderStreamOwner,
    startFirstChatStream,
    type FirstChatStreamHandle,
  } from "$lib/providers/stream";
  import { FIRST_CHAT_MAX_INPUT_BYTES } from "$lib/providers/first-chat-request";
  import {
    loadOrCreateFirstChat,
    toChatMessage,
  } from "$lib/storage/chat-history";
  import { boundedTailById } from "$lib/storage/bounded-history-window";
  import { appPreferences } from "$lib/storage/app-preferences.svelte";
  import { storageClient } from "$lib/storage/client";
  import Avatar from "$lib/ui/Avatar.svelte";
  import { horizontalSwipe, type SwipeCommit } from "$lib/ui/horizontal-swipe";

  const characterName = "세라핀";
  const characterInitial = "세";
  const STREAM_TEXT_BLOCK_CHARACTERS = 8_192;

  let mode = $state<ThreadMode>("chat");
  let scrollRegion = $state<HTMLDivElement | null>(null);
  let panelElement = $state<HTMLElement | null>(null);

  let panelOpen = $state(false);
  let panelShift = $state(0);
  let backDrag = $state(0);
  let activeStream = $state<FirstChatStreamHandle | null>(null);
  let activeDeltaBuffer: FrameChunkBuffer | null = null;
  let chatId = $state<string | null>(null);
  let storageUnavailable = $state(false);
  let historyEpoch = 0;
  let disposed = false;

  let messages = $state<ChatMessage[]>([]);

  async function initializeHistory(): Promise<void> {
    const epoch = ++historyEpoch;
    try {
      // A hard WebView reload loses the old JS control token while the native
      // request may still be alive. Reset this injected window owner first,
      // wait for durable terminal state, then read canonical SQLite history.
      await resetProviderStreamOwner();
      await appPreferences.hydrate();
      const loaded = await loadOrCreateFirstChat();
      if (disposed || epoch !== historyEpoch) return;
      mode = appPreferences.current.defaultMode;
      chatId = loaded.chat.id;
      messages = [...boundedTailById(loaded.messages)];
      storageUnavailable = false;
    } catch {
      if (disposed || epoch !== historyEpoch) return;
      chatId = null;
      messages = [];
      storageUnavailable = true;
    }
  }

  async function reloadHistory(targetChatId: string): Promise<void> {
    cancelPendingDeltaRender();
    const epoch = ++historyEpoch;
    try {
      const loaded = await storageClient.loadChatMessages(targetChatId);
      if (disposed || epoch !== historyEpoch || chatId !== targetChatId) return;
      messages = [
        ...boundedTailById(loaded.items.map(toChatMessage)),
      ];
    } catch {
      if (!disposed && epoch === historyEpoch) {
        storageUnavailable = true;
      }
    }
  }

  function panelWidth(): number {
    return panelElement?.offsetWidth ?? 320;
  }

  function openPanel(): void {
    panelOpen = true;
    panelShift = panelWidth();
  }

  function closePanel(): void {
    panelOpen = false;
    panelShift = 0;
  }

  function clamp(value: number, min: number, max: number): number {
    return Math.min(Math.max(value, min), max);
  }

  function handleSwipeMove(dx: number): void {
    if (panelOpen) {
      panelShift = clamp(panelWidth() - dx, 0, panelWidth());
      return;
    }
    if (dx < 0) {
      backDrag = 0;
      panelShift = clamp(-dx, 0, panelWidth());
    } else {
      panelShift = 0;
      backDrag = dx;
    }
  }

  function handleSwipeEnd(commit: SwipeCommit): void {
    if (panelOpen) {
      if (commit === "right") {
        closePanel();
      } else {
        panelShift = panelWidth();
      }
      return;
    }
    if (panelShift > 0) {
      if (commit === "left") {
        openPanel();
      } else {
        panelShift = 0;
      }
      return;
    }
    if (backDrag > 0 && commit === "right") {
      void goto("/");
    }
    backDrag = 0;
  }

  function replaceMessage(
    id: string,
    update: (message: ChatMessage) => ChatMessage,
  ): void {
    if (disposed) return;
    messages = messages.map((message) =>
      message.id === id ? update(message) : message,
    );
  }

  function appendStreamingChunk(
    message: ChatMessage,
    chunk: string,
  ): ChatMessage {
    if (chunk.length === 0) return message;
    const chunks = message.streamingChunks ?? [];
    const tailIndex = chunks.length - 1;
    const tail = chunks[tailIndex];
    if (
      tail !== undefined &&
      tail.length + chunk.length <= STREAM_TEXT_BLOCK_CHARACTERS
    ) {
      chunks[tailIndex] = `${tail}${chunk}`;
    } else {
      chunks.push(chunk);
    }
    return {
      ...message,
      streamingChunks: chunks,
    };
  }

  function materializeStreamingText(
    message: ChatMessage,
    trailing = "",
  ): string {
    return [message.text, ...(message.streamingChunks ?? []), trailing].join("");
  }

  function cancelPendingDeltaRender(): void {
    const buffer = activeDeltaBuffer;
    if (buffer === null) return;
    buffer.cancel();
    activeDeltaBuffer = null;
  }

  function handleSend(text: string): void {
    const profile = activeProviderProfile.current;
    const targetChatId = chatId;
    if (activeStream !== null || profile === null || targetChatId === null) {
      return;
    }

    historyEpoch += 1;
    const nonce = `${Date.now()}-${crypto.randomUUID()}`;
    const assistantId = `assistant-${nonce}`;
    messages = [
      ...boundedTailById([
        ...messages,
        {
          id: `user-${nonce}`,
          role: "user" as const,
          text,
          sentAt: new Date(),
        },
        {
          id: assistantId,
          role: "character" as const,
          text: "",
          streamingChunks: [],
          sentAt: new Date(),
          streaming: true,
        },
      ]),
    ];

    let failureShown = false;
    let nativeTurnStarted = false;
    const deltaBuffer = createFrameChunkBuffer((delta) => {
      replaceMessage(assistantId, (message) =>
        appendStreamingChunk(message, delta),
      );
    });
    cancelPendingDeltaRender();
    activeDeltaBuffer = deltaBuffer;

    try {
      const handle = startFirstChatStream(profile, targetChatId, text, {
        onStarted() {
          nativeTurnStarted = true;
        },
        onDelta(delta) {
          deltaBuffer.append(delta);
        },
        onError(message) {
          failureShown = true;
          const pendingText = deltaBuffer.drain();
          replaceMessage(assistantId, (current) => {
            const hasVisibleText =
              current.text.length > 0 ||
              (current.streamingChunks?.length ?? 0) > 0 ||
              pendingText.length > 0;
            let next = appendStreamingChunk(current, pendingText);
            next = appendStreamingChunk(
              next,
              hasVisibleText ? `\n\n${message}` : message,
            );
            return next;
          });
        },
        onTerminal(terminal) {
          const pendingText = deltaBuffer.close();
          if (activeDeltaBuffer === deltaBuffer) {
            activeDeltaBuffer = null;
          }
          replaceMessage(assistantId, (message) => {
            const streamed = materializeStreamingText(message, pendingText);
            const text =
              streamed.length > 0
                ? streamed
                : terminal === "cancelled"
                  ? "응답을 중지했습니다."
                  : terminal === "completed"
                    ? "제공자가 빈 응답을 반환했습니다."
                    : failureShown
                      ? message.text
                      : "응답을 완료하지 못했습니다.";
            return {
              ...message,
              text,
              streamingChunks: undefined,
              streaming: false,
            };
          });
          // A rejected start command has no durable turn yet. Keep the
          // optimistic user bubble and error visible instead of replacing it
          // with an older DB snapshot and losing the submitted text.
          if (nativeTurnStarted) {
            void reloadHistory(targetChatId);
          }
        },
      });
      activeStream = handle;
      void handle.done.then(() => {
        if (activeStream === handle) {
          activeStream = null;
        }
      });
    } catch {
      const pendingText = deltaBuffer.close();
      if (activeDeltaBuffer === deltaBuffer) {
        activeDeltaBuffer = null;
      }
      replaceMessage(assistantId, (message) => {
        const streamed = materializeStreamingText(message, pendingText);
        return {
          ...message,
          text:
            streamed.length > 0
              ? `${streamed}\n\n메시지를 전송할 수 없습니다. 입력 내용을 확인해 주세요.`
              : "메시지를 전송할 수 없습니다. 입력 내용을 확인해 주세요.",
          streamingChunks: undefined,
          streaming: false,
        };
      });
    }
  }

  function handleCancel(): void {
    const handle = activeStream;
    if (handle !== null) {
      activeDeltaBuffer?.flush();
      void handle.cancel().catch(() => undefined);
    }
  }

  onMount(() => {
    const stopKeyboardInset = keyboardInset.start();
    void initializeHistory();
    return stopKeyboardInset;
  });
  onDestroy(() => {
    disposed = true;
    cancelPendingDeltaRender();
    const handle = activeStream;
    if (handle !== null) {
      void handle.cancel().catch(() => undefined);
    }
  });

  $effect(() => {
    void messages.length;
    void keyboardInset.value;
    const region = scrollRegion;
    if (region) {
      requestAnimationFrame(() => {
        region.scrollTop = region.scrollHeight;
      });
    }
  });
</script>

<svelte:head>
  <title>LorePia — 대화</title>
</svelte:head>

<div
  class="screen"
  use:horizontalSwipe={{ onMove: handleSwipeMove, onEnd: handleSwipeEnd }}
>
  <header class="top">
    <a class="back" href="/" aria-label="서재로 돌아가기">
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
        <path d="m15 18-6-6 6-6" />
      </svg>
    </a>
    <button class="identity" type="button" onclick={openPanel}>
      <Avatar initial={characterInitial} size={32} />
      <span class="titles">
        <span class="name">{characterName}</span>
        <span class="tagline">달빛 서고의 사서</span>
      </span>
    </button>
    <button
      class="more"
      type="button"
      onclick={openPanel}
      aria-label="대화 설정 열기"
    >
      <svg
        viewBox="0 0 24 24"
        width="20"
        height="20"
        fill="currentColor"
        aria-hidden="true"
      >
        <circle cx="5" cy="12" r="1.8" />
        <circle cx="12" cy="12" r="1.8" />
        <circle cx="19" cy="12" r="1.8" />
      </svg>
    </button>
  </header>

  <div class="scroll" bind:this={scrollRegion}>
    {#if messages.length === 0}
      <div class="empty-thread">
        <Avatar initial={characterInitial} size={72} />
        <p class="empty-name">{characterName}</p>
        <p class="empty-tagline">달빛 서고의 사서</p>
        <p class="empty-hint">
          {storageUnavailable
            ? "로컬 저장소를 사용할 수 없어 대화를 불러오지 못했습니다"
            : "첫 인사를 건네보세요"}
        </p>
      </div>
    {:else}
      <MessageThread {messages} {mode} {characterName} {characterInitial} />
    {/if}
  </div>

  <div class="composer-slot">
    <Composer
      onSend={handleSend}
      onCancel={handleCancel}
      busy={activeStream !== null}
      disabled={activeProviderProfile.current === null || chatId === null}
      maxLength={FIRST_CHAT_MAX_INPUT_BYTES}
      placeholder={storageUnavailable
        ? "로컬 저장소를 사용할 수 없습니다"
        : chatId === null
          ? "대화를 불러오는 중"
          : activeProviderProfile.current === null
        ? "설정에서 API 키와 모델을 준비하세요"
        : "메시지 보내기"}
    />
    <svg
      class="keyboard-spacer"
      width="1"
      height={keyboardInset.value}
      aria-hidden="true"
    ></svg>
  </div>

  <button
    class="scrim"
    class:open={panelOpen}
    type="button"
    aria-label="방 설정 닫기"
    aria-hidden={!panelOpen}
    tabindex={panelOpen ? 0 : -1}
    onclick={closePanel}
  ></button>

  <aside
    class="panel"
    class:open={panelOpen}
    bind:this={panelElement}
    aria-label="방 설정"
  >
    <div class="panel-hero">
      <Avatar initial={characterInitial} size={48} />
      <div>
        <p class="panel-name">{characterName}</p>
        <p class="panel-tagline">달빛 서고의 사서</p>
      </div>
    </div>

    <div class="panel-row">
      <span class="panel-label">표시 모드</span>
      <div class="segment" role="group" aria-label="표시 모드 선택">
        <button
          type="button"
          class:active={mode === "chat"}
          onclick={() => (mode = "chat")}>채팅</button
        >
        <button
          type="button"
          class:active={mode === "story"}
          onclick={() => (mode = "story")}>스토리</button
        >
      </div>
    </div>

    <a class="panel-link" href="/character/seraphine">캐릭터 정보 보기</a>
    <a class="panel-link" href="/settings">앱 설정</a>
  </aside>
</div>

<style>
  .screen {
    height: 100%;
    display: flex;
    flex-direction: column;
    background: var(--surface-page);
    font-family: var(--font-ui);
    position: relative;
    overflow: hidden;
    touch-action: pan-y;
  }

  .top {
    position: relative;
    z-index: 2;
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    min-height: calc(var(--size-touch) + var(--sp-2));
    padding: var(--sp-2) var(--sp-3);
    padding-top: calc(var(--sp-2) + var(--safe-top));
    background: var(--bar-bg);
    -webkit-backdrop-filter: blur(20px) saturate(1.6);
    backdrop-filter: blur(20px) saturate(1.6);
  }

  .back,
  .more {
    width: var(--size-touch);
    height: var(--size-touch);
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: none;
    padding: 0;
    background: transparent;
    border-radius: var(--r-pill);
    color: var(--text-mid);
    cursor: pointer;
    transition:
      background var(--dur-fast) var(--ease-out),
      transform var(--dur-base) var(--ease-spring);
  }

  .back:active,
  .more:active {
    background: var(--surface-bubble);
    transform: scale(0.9);
  }

  .identity {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--sp-3);
    min-width: 0;
    min-height: var(--size-touch);
    border: none;
    background: transparent;
    cursor: pointer;
    font-family: var(--font-ui);
    padding: 0;
  }

  .titles {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    min-width: 0;
  }

  .name {
    font-size: var(--fs-ui);
    font-weight: 500;
    color: var(--text-strong);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .tagline {
    font-size: var(--fs-caption);
    color: var(--text-mid);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .scroll {
    flex: 1;
    overflow-y: auto;
    overscroll-behavior: none;
    scroll-behavior: smooth;
  }

  .empty-thread {
    position: relative;
    height: 100%;
    box-sizing: border-box;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--sp-1);
    padding: var(--sp-6);
    text-align: center;
    animation: lp-pop var(--dur-page) var(--ease-spring) backwards;
  }

  .empty-thread::before {
    content: "";
    position: absolute;
    width: 260px;
    height: 260px;
    border-radius: var(--r-pill);
    background: radial-gradient(closest-side, var(--tint-soft), transparent);
    z-index: 0;
    pointer-events: none;
  }

  .empty-thread > :global(*) {
    position: relative;
    z-index: 1;
  }

  .empty-name {
    margin: var(--sp-2) 0 0;
    font-size: 18px;
    font-weight: 700;
    letter-spacing: -0.01em;
    color: var(--text-strong);
  }

  .empty-tagline {
    margin: 0;
    font-size: var(--fs-label);
    color: var(--text-mid);
  }

  .empty-hint {
    margin: var(--sp-4) 0 0;
    padding: var(--sp-2) var(--sp-4);
    background: var(--surface-bubble);
    border-radius: var(--r-pill);
    font-size: var(--fs-label);
    color: var(--text-mid);
  }

  .composer-slot {
    background: var(--surface-page);
  }

  .keyboard-spacer {
    display: block;
    width: 1px;
    max-width: 1px;
    flex: none;
  }

  .scrim {
    position: absolute;
    inset: 0;
    border: none;
    padding: 0;
    background: rgba(0, 0, 0, 0.35);
    -webkit-backdrop-filter: blur(6px);
    backdrop-filter: blur(6px);
    cursor: pointer;
    opacity: 0;
    visibility: hidden;
    transition:
      opacity var(--dur-base) var(--ease-out),
      visibility var(--dur-base) var(--ease-out);
  }

  .scrim.open {
    opacity: 1;
    visibility: visible;
  }

  .panel {
    position: absolute;
    top: 0;
    right: 0;
    bottom: 0;
    width: min(320px, 84vw);
    background: var(--surface-card);
    border-radius: var(--r-card) 0 0 var(--r-card);
    box-shadow: var(--shadow-float);
    padding: calc(var(--sp-4) + var(--safe-top)) var(--sp-4)
      calc(var(--sp-4) + var(--safe-bottom));
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
    box-sizing: border-box;
    transform: translateX(105%);
    transition: transform var(--dur-slow) var(--ease-out);
  }

  .panel.open {
    transform: translateX(0);
  }

  .panel-hero {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    padding-bottom: var(--sp-3);
    border-bottom: 0.5px solid var(--hairline);
  }

  .panel-name {
    margin: 0;
    font-size: var(--fs-chat);
    font-weight: 500;
    color: var(--text-strong);
  }

  .panel-tagline {
    margin: 0;
    font-size: var(--fs-label);
    color: var(--text-mid);
  }

  .panel-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-3);
    min-height: var(--size-touch);
  }

  .panel-label {
    font-size: var(--fs-ui);
    color: var(--text-strong);
  }

  .segment {
    display: flex;
    background: var(--surface-bubble);
    border-radius: 10px;
    padding: 2px;
    gap: 2px;
  }

  .segment button {
    min-height: 30px;
    padding: 0 var(--sp-3);
    border: none;
    border-radius: 8px;
    background: transparent;
    color: var(--text-mid);
    font-family: var(--font-ui);
    font-size: var(--fs-label);
    font-weight: 500;
    cursor: pointer;
    transition:
      background var(--dur-base) var(--ease-out),
      color var(--dur-base) var(--ease-out),
      box-shadow var(--dur-base) var(--ease-out),
      transform var(--dur-fast) var(--ease-spring);
  }

  .segment button:active {
    transform: scale(0.95);
  }

  .segment button.active {
    background: var(--segment-thumb);
    color: var(--text-strong);
    font-weight: 600;
    box-shadow: 0 1px 4px rgba(0, 0, 0, 0.12);
  }

  .panel-link {
    display: flex;
    align-items: center;
    min-height: var(--size-touch);
    font-size: var(--fs-ui);
    color: var(--text-strong);
    text-decoration: none;
    border-bottom: 0.5px solid var(--hairline);
  }

  @media (prefers-reduced-motion: reduce) {
    .scrim,
    .panel {
      transition: none;
    }

    .scroll {
      scroll-behavior: auto;
    }
  }
</style>
