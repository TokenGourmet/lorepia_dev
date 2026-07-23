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
  import type { ChatMessage } from "$lib/chat/types";
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
  import { chatRoomPrefs } from "$lib/chat/room-prefs.svelte";
  import { edgeSwipeBack } from "$lib/ui/edge-back";

  const characterName = "세라핀";
  const characterInitial = "세";
  const STREAM_TEXT_BLOCK_CHARACTERS = 8_192;

  let scrollRegion = $state<HTMLDivElement | null>(null);
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
      chatRoomPrefs.seedDefault(appPreferences.current.defaultMode);
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

<div class="screen" use:edgeSwipeBack={{ onBack: () => goto("/") }}>
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
    <!-- iOS Messages identity: avatar stacked over the name, centered; the
         chevron marks it as the door to the room detail. It is the single
         entry — Messages has no ⋯ in the chat bar. -->
    <a class="identity" href="/chat/info">
      <Avatar initial={characterInitial} size={36} />
      <span class="name">
        {characterName}
        <svg
          class="chev"
          viewBox="0 0 24 24"
          width="10"
          height="10"
          fill="none"
          stroke="currentColor"
          stroke-width="2.6"
          stroke-linecap="round"
          stroke-linejoin="round"
          aria-hidden="true"
        >
          <path d="m9 18 6-6-6-6" />
        </svg>
      </span>
    </a>
    <!-- Balances the back button so the identity stack stays centered. -->
    <span class="lead-balance" aria-hidden="true"></span>
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
      <MessageThread
        {messages}
        mode={chatRoomPrefs.mode}
        {characterName}
        {characterInitial}
      />
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

  .back {
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

  .back:active {
    background: var(--surface-bubble);
    transform: scale(0.9);
  }

  .lead-balance {
    width: var(--size-touch);
    flex-shrink: 0;
  }

  .identity {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 3px;
    min-width: 0;
    min-height: var(--size-touch);
    border: none;
    background: transparent;
    cursor: pointer;
    font-family: var(--font-ui);
    padding: var(--sp-1) 0;
    text-decoration: none;
  }

  .name {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    max-width: 100%;
    font-size: var(--fs-caption);
    font-weight: 600;
    letter-spacing: -0.01em;
    color: var(--text-strong);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .chev {
    flex-shrink: 0;
    color: var(--text-faint);
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

  @media (prefers-reduced-motion: reduce) {
    .scroll {
      scroll-behavior: auto;
    }
  }
</style>
