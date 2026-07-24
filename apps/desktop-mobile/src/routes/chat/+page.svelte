<script lang="ts">
  import { onDestroy, onMount, tick, untrack } from "svelte";
  import { goto } from "$app/navigation";
  import { page } from "$app/state";

  import "$lib/design/tokens.css";

  import {
    characterChatTitle,
    findSampleCharacter,
  } from "$lib/characters/sample";
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
  import {
    FIRST_CHAT_MAX_INPUT_BYTES,
    firstChatInputBlockReason,
  } from "$lib/providers/first-chat-request";
  import {
    FIRST_CHAT_CHARACTER_ID,
    loadOrCreateCharacterChat,
    toChatMessage,
  } from "$lib/storage/chat-history";
  import { appPreferences } from "$lib/storage/app-preferences.svelte";
  import {
    MAX_MESSAGE_PAGE,
    storageClient,
    type MessageCursor,
  } from "$lib/storage/client";
  import Avatar from "$lib/ui/Avatar.svelte";
  import { activateBackSwipeSurface } from "$lib/ui/back-swipe-surface";
  import { chatRoomPrefs } from "$lib/chat/room-prefs.svelte";
  import { contentSwipeBack } from "$lib/ui/content-back";
  import {
    connectNativeBack,
    requestNativeBackPop,
    shouldOptimisticallyArmNativeBack,
    usesNativeBackChrome,
  } from "$lib/ui/native-back";
  import {
    prependOlderMessages,
    preservedPrependScrollTop,
  } from "./history-pagination";

  const fallbackCharacter = findSampleCharacter(FIRST_CHAT_CHARACTER_ID)!;
  const requestedCharacterId = $derived(
    page.url.searchParams.get("character") ?? FIRST_CHAT_CHARACTER_ID,
  );
  const character = $derived(
    findSampleCharacter(requestedCharacterId) ?? fallbackCharacter,
  );
  const characterName = $derived(character.name);
  const characterInitial = $derived(character.initial);
  const chatHref = $derived(
    `/chat?character=${encodeURIComponent(character.id)}`,
  );
  const STREAM_TEXT_BLOCK_CHARACTERS = 8_192;
  const NATIVE_ROOM_INFO_EVENT = "lorepia:native-room-info";

  let scrollRegion = $state<HTMLDivElement | null>(null);
  let activeStream = $state<FirstChatStreamHandle | null>(null);
  let activeDeltaBuffer: FrameChunkBuffer | null = null;
  let chatId = $state<string | null>(null);
  let storageUnavailable = $state(false);
  let historyHasMore = $state(false);
  let olderCursor = $state<MessageCursor | null>(null);
  let loadingOlder = $state(false);
  let olderLoadFailed = $state(false);
  let olderLoadAttempted = $state(false);
  let olderRequestId = 0;
  let suppressNextAutoScroll = false;
  let historyEpoch = 0;
  let initializedCharacterId: string | null = null;
  let disposed = false;
  let nativeBackActive = $state(false);
  const infoHref = $derived(
    `${chatHref.replace("/chat?", "/chat/info?")}${
      chatId === null ? "" : `&chatId=${encodeURIComponent(chatId)}`
    }`,
  );

  let messages = $state<ChatMessage[]>([]);
  const sendBlockReason = $derived(
    storageUnavailable
      ? "로컬 저장소를 사용할 수 없어 메시지를 보낼 수 없습니다."
      : chatId === null
        ? "대화를 준비하는 중이라 아직 메시지를 보낼 수 없습니다."
        : activeProviderProfile.sendBlockReason,
  );

  function backHref(): string | null {
    const candidate = (page.state as { backHref?: unknown }).backHref;
    return typeof candidate === "string" &&
      candidate.startsWith("/") &&
      !candidate.startsWith("//")
      ? candidate
      : null;
  }

  function navigateBack(): void {
    const fallback = backHref();
    if (fallback !== null && window.history.length > 1) {
      window.history.back();
      return;
    }
    void goto(fallback ?? "/");
  }

  async function handleBackClick(event: MouseEvent): Promise<void> {
    event.preventDefault();
    if (nativeBackActive) {
      const status = await requestNativeBackPop();
      if (status.active) return;
    }
    navigateBack();
  }

  async function openInfo(event?: MouseEvent): Promise<void> {
    event?.preventDefault();
    await goto(infoHref, {
      state: {
        backHref: chatHref,
      },
    });
  }

  async function initializeHistory(
    targetCharacterId = character.id,
    targetTitle = characterChatTitle(character.name),
  ): Promise<void> {
    const epoch = ++historyEpoch;
    try {
      // A hard WebView reload loses the old JS control token while the native
      // request may still be alive. Reset this injected window owner first,
      // wait for durable terminal state, then read canonical SQLite history.
      await resetProviderStreamOwner();
      await appPreferences.hydrate();
      const loaded = await loadOrCreateCharacterChat(
        targetCharacterId,
        targetTitle,
      );
      if (disposed || epoch !== historyEpoch) return;
      chatRoomPrefs.seedDefault(appPreferences.current.defaultMode);
      chatId = loaded.chat.id;
      messages = [...loaded.messages];
      historyHasMore = loaded.hasMore;
      olderCursor = loaded.olderCursor;
      loadingOlder = false;
      olderLoadFailed = false;
      olderLoadAttempted = false;
      storageUnavailable = false;
    } catch {
      if (disposed || epoch !== historyEpoch) return;
      chatId = null;
      messages = [];
      historyHasMore = false;
      olderCursor = null;
      loadingOlder = false;
      olderLoadFailed = false;
      olderLoadAttempted = false;
      olderRequestId += 1;
      storageUnavailable = true;
    }
  }

  async function reloadHistory(targetChatId: string): Promise<void> {
    cancelPendingDeltaRender();
    const epoch = ++historyEpoch;
    try {
      const loaded = await storageClient.loadChatMessages(targetChatId);
      if (disposed || epoch !== historyEpoch || chatId !== targetChatId) return;
      messages = loaded.items.map(toChatMessage);
      historyHasMore = loaded.hasMore;
      olderCursor = loaded.olderCursor;
      loadingOlder = false;
      olderLoadFailed = false;
      olderLoadAttempted = false;
      olderRequestId += 1;
      storageUnavailable = false;
    } catch {
      if (!disposed && epoch === historyEpoch) {
        storageUnavailable = true;
      }
    }
  }

  async function loadOlderHistory(): Promise<void> {
    const targetChatId = chatId;
    const requestedCursor = olderCursor;
    if (
      targetChatId === null ||
      requestedCursor === null ||
      !historyHasMore ||
      loadingOlder
    ) {
      return;
    }

    const requestId = ++olderRequestId;
    const region = scrollRegion;
    const previousHeight = region?.scrollHeight ?? 0;
    const previousTop = region?.scrollTop ?? 0;
    loadingOlder = true;
    olderLoadFailed = false;
    olderLoadAttempted = true;

    try {
      const loaded = await storageClient.loadChatMessages(
        targetChatId,
        MAX_MESSAGE_PAGE,
        requestedCursor,
      );
      if (
        disposed ||
        requestId !== olderRequestId ||
        chatId !== targetChatId ||
        olderCursor?.chatId !== requestedCursor.chatId ||
        olderCursor.ordinal !== requestedCursor.ordinal
      ) {
        return;
      }

      const merged = prependOlderMessages(
        messages,
        loaded.items.map(toChatMessage),
      );
      const inserted = merged.length > messages.length;
      historyHasMore = loaded.hasMore;
      olderCursor = loaded.olderCursor;
      olderLoadFailed = false;

      if (inserted) {
        suppressNextAutoScroll = true;
        messages = merged;
        await tick();
        if (scrollRegion === region && region !== null) {
          region.scrollTop = preservedPrependScrollTop(
            previousTop,
            previousHeight,
            region.scrollHeight,
          );
        }
      }
    } catch {
      if (
        !disposed &&
        requestId === olderRequestId &&
        chatId === targetChatId
      ) {
        olderLoadFailed = true;
      }
    } finally {
      if (
        !disposed &&
        requestId === olderRequestId &&
        chatId === targetChatId
      ) {
        loadingOlder = false;
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

  function handleSend(text: string): boolean {
    const profile = activeProviderProfile.current;
    const targetChatId = chatId;
    if (activeStream !== null || profile === null || targetChatId === null) {
      return false;
    }

    historyEpoch += 1;
    const nonce = `${Date.now()}-${crypto.randomUUID()}`;
    const assistantId = `assistant-${nonce}`;
    messages = [
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
    return true;
  }

  function handleCancel(): void {
    const handle = activeStream;
    if (handle !== null) {
      activeDeltaBuffer?.flush();
      void handle.cancel().catch(() => undefined);
    }
  }

  function activateCharacter(
    targetCharacterId: string,
    targetTitle: string,
  ): void {
    if (initializedCharacterId === targetCharacterId) return;
    initializedCharacterId = targetCharacterId;
    historyEpoch += 1;
    cancelPendingDeltaRender();
    const handle = activeStream;
    activeStream = null;
    if (handle !== null) {
      void handle.cancel().catch(() => undefined);
    }
    chatId = null;
    messages = [];
    historyHasMore = false;
    olderCursor = null;
    loadingOlder = false;
    olderLoadFailed = false;
    olderLoadAttempted = false;
    olderRequestId += 1;
    storageUnavailable = false;
    void initializeHistory(targetCharacterId, targetTitle);
  }

  $effect(() => {
    const targetCharacterId = character.id;
    const targetTitle = characterChatTitle(character.name);
    untrack(() => activateCharacter(targetCharacterId, targetTitle));
  });

  onMount(() => {
    const stopKeyboardInset = keyboardInset.start();
    let disconnectNativeBack = (): void => undefined;
    let nativeBackDisposed = false;
    const nativePlatform =
      document.documentElement.dataset.nativePlatform;
    nativeBackActive =
      shouldOptimisticallyArmNativeBack(nativePlatform);
    const openNativeRoomInfo = (): void => {
      if (!nativeBackDisposed) void openInfo();
    };
    window.addEventListener(NATIVE_ROOM_INFO_EVENT, openNativeRoomInfo);

    if (nativePlatform === "ios") {
      void connectNativeBack(() => {
        if (nativeBackDisposed) return;
        nativeBackActive = false;
        navigateBack();
      }).then((connection) => {
        if (nativeBackDisposed) {
          connection.disconnect();
          return;
        }
        disconnectNativeBack = connection.disconnect;
        nativeBackActive = usesNativeBackChrome(
          connection.status,
          nativePlatform,
        );
      });
    }

    return () => {
      nativeBackDisposed = true;
      window.removeEventListener(
        NATIVE_ROOM_INFO_EVENT,
        openNativeRoomInfo,
      );
      disconnectNativeBack();
      stopKeyboardInset?.();
    };
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
      if (suppressNextAutoScroll) {
        suppressNextAutoScroll = false;
        return;
      }
      requestAnimationFrame(() => {
        region.scrollTop = region.scrollHeight;
      });
    }
  });
</script>

<svelte:head>
  <title>LorePia — {characterName} 대화</title>
</svelte:head>

<div
  class="screen"
  use:contentSwipeBack={{
    onBack: navigateBack,
    getUnderlay: () => activateBackSwipeSurface(backHref()),
    enabled: !nativeBackActive,
  }}
>
  <header class="top">
    <a
      class="back"
      href="/"
      aria-label="이전 화면으로 돌아가기"
      aria-hidden={nativeBackActive}
      tabindex={nativeBackActive ? -1 : undefined}
      onclick={handleBackClick}
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
        <path d="m15 18-6-6 6-6" />
      </svg>
    </a>
    <!-- iOS Messages identity: avatar stacked over the name, centered; the
         chevron marks it as the door to the room detail. It is the single
         entry — Messages has no ⋯ in the chat bar. -->
    <a
      class="identity lp-state-layer"
      href={infoHref}
      aria-hidden={nativeBackActive}
      tabindex={nativeBackActive ? -1 : undefined}
      onclick={openInfo}
    >
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

  <div
    class="scroll"
    bind:this={scrollRegion}
  >
    {#if messages.length === 0}
      <div class="empty-thread">
        <Avatar initial={characterInitial} size={72} />
        <p class="empty-name">{characterName}</p>
        <p class="empty-tagline">{character.tagline}</p>
        {#if storageUnavailable}
          <div class="empty-hint error" role="alert">
            <span>로컬 저장소를 사용할 수 없어 대화를 불러오지 못했습니다</span>
            <button type="button" onclick={() => void initializeHistory()}>
              다시 시도
            </button>
          </div>
        {:else}
          <p class="empty-hint">첫 인사를 건네보세요</p>
        {/if}
      </div>
    {:else}
      {#if historyHasMore || olderLoadAttempted}
        <div class="history-pagination" aria-live="polite">
          {#if loadingOlder}
            <p role="status">이전 메시지를 불러오는 중…</p>
          {:else if olderLoadFailed}
            <p role="alert">이전 메시지를 불러오지 못했습니다.</p>
            <button
              type="button"
              aria-label="이전 메시지 다시 불러오기"
              onclick={() => void loadOlderHistory()}
            >
              다시 시도
            </button>
          {:else if historyHasMore}
            <button type="button" onclick={() => void loadOlderHistory()}>
              이전 메시지 불러오기
            </button>
          {:else}
            <p role="status">대화의 처음입니다</p>
          {/if}
        </div>
      {/if}
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
      blockedReason={sendBlockReason}
      validate={firstChatInputBlockReason}
      maxLength={FIRST_CHAT_MAX_INPUT_BYTES}
      placeholder="메시지 보내기"
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
    box-sizing: border-box;
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
      background var(--dur-fast) var(--ease-out);
  }

  .back:active {
    background: var(--surface-bubble);
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

  @media (max-width: 699px) {
    /* Apple iOS 26 top toolbar: a 44pt control centred in a 54pt row.
       Every top-level mobile header uses the same --size-navbar zone. */
    .top {
      box-sizing: content-box;
      height: var(--size-navbar);
      min-height: 0;
      padding: var(--safe-top) var(--sp-4) 0;
      background: transparent;
      -webkit-backdrop-filter: none;
      backdrop-filter: none;
    }

    .back {
      border: 0.5px solid var(--hairline);
      background: var(--bar-bg);
      -webkit-backdrop-filter: blur(20px) saturate(1.6);
      backdrop-filter: blur(20px) saturate(1.6);
      box-shadow: var(--shadow-float);
    }

    .identity {
      height: var(--size-navbar);
      min-height: 0;
      padding: 0;
    }
  }

  .scroll {
    flex: 1;
    overflow-y: auto;
    overscroll-behavior: none;
    scroll-behavior: smooth;
  }

  .history-pagination {
    min-height: var(--size-touch);
    padding: var(--sp-2) var(--sp-4) 0;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--sp-2);
    color: var(--text-faint);
    font-size: var(--fs-caption);
    text-align: center;
  }

  .history-pagination p {
    margin: 0;
  }

  .history-pagination button {
    min-height: var(--size-touch);
    padding: 0 var(--sp-4);
    border: 0;
    border-radius: var(--r-pill);
    background: transparent;
    color: var(--tint);
    font: inherit;
    font-weight: 600;
    cursor: pointer;
  }

  .history-pagination button:active {
    background: var(--tint-soft);
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

  .empty-hint.error {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--sp-2);
    border-radius: var(--r-card);
  }

  .empty-hint button {
    min-height: 32px;
    padding: 0 var(--sp-3);
    border: 0.5px solid var(--hairline);
    border-radius: var(--r-pill);
    background: var(--surface-card);
    color: var(--text-strong);
    font: inherit;
    cursor: pointer;
  }

  .composer-slot {
    flex-shrink: 0;
    box-sizing: border-box;
    padding-bottom: max(
      calc(env(safe-area-inset-bottom, 0px) + var(--sp-3)),
      var(--sp-4)
    );
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
