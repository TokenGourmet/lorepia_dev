<script lang="ts">
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { onMount, tick } from "svelte";

  import "$lib/design/tokens.css";

  import {
    characterChatTitle,
    findSampleCharacter,
  } from "$lib/characters/sample";
  import { chatRoomPrefs } from "$lib/chat/room-prefs.svelte";
  import type { ThreadMode } from "$lib/chat/types";
  import {
    FIRST_CHAT_CHARACTER_ID,
    loadOrCreateCharacterChat,
  } from "$lib/storage/chat-history";
  import { storageClient } from "$lib/storage/client";
  import Avatar from "$lib/ui/Avatar.svelte";
  import { activateBackSwipeSurface } from "$lib/ui/back-swipe-surface";
  import LargeTitleHeader from "$lib/ui/LargeTitleHeader.svelte";
  import { edgeSwipeBack } from "$lib/ui/edge-back";

  const fallbackCharacter = findSampleCharacter(FIRST_CHAT_CHARACTER_ID)!;
  const character = $derived(
    findSampleCharacter(
      page.url.searchParams.get("character") ?? FIRST_CHAT_CHARACTER_ID,
    ) ?? fallbackCharacter,
  );
  const chatHref = $derived(
    `/chat?character=${encodeURIComponent(character.id)}`,
  );
  const characterHref = $derived(
    `/character/${encodeURIComponent(character.id)}`,
  );

  const modeOptions: { value: ThreadMode; label: string }[] = [
    { value: "chat", label: "채팅" },
    { value: "story", label: "스토리" },
  ];

  let chatId = $state<string | null>(null);
  let loadingChat = $state(true);
  let chatError = $state<string | null>(null);
  let deleting = $state(false);
  let deleteError = $state<string | null>(null);
  let deleteDialog = $state<HTMLDialogElement | null>(null);
  let cancelDeleteButton = $state<HTMLButtonElement | null>(null);
  const reportHref = $derived(
    chatId === null
      ? null
      : `/chat/report?character=${encodeURIComponent(character.id)}&chatId=${encodeURIComponent(chatId)}`,
  );

  async function loadChat(): Promise<void> {
    loadingChat = true;
    chatError = null;
    try {
      const loaded = await loadOrCreateCharacterChat(
        character.id,
        characterChatTitle(character.name),
      );
      chatId = loaded.chat.id;
    } catch {
      chatId = null;
      chatError = "대화 정보를 불러오지 못했습니다.";
    } finally {
      loadingChat = false;
    }
  }

  async function deleteChat(): Promise<void> {
    const targetChatId = chatId;
    if (targetChatId === null || deleting) return;
    deleting = true;
    deleteError = null;
    try {
      const receipt = await storageClient.deleteChat(targetChatId);
      if (receipt.chatId !== targetChatId) {
        throw new Error("DELETE_CHAT_RECEIPT_MISMATCH");
      }
      await goto("/", { replaceState: true });
    } catch {
      deleteError = "대화를 삭제하지 못했습니다. 잠시 후 다시 시도해 주세요.";
    } finally {
      deleting = false;
    }
  }

  async function openDeleteConfirmation(): Promise<void> {
    deleteError = null;
    deleteDialog?.showModal();
    await tick();
    cancelDeleteButton?.focus();
  }

  function closeDeleteConfirmation(): void {
    if (deleting) return;
    deleteError = null;
    deleteDialog?.close("cancel");
  }

  function handleDialogCancel(event: Event): void {
    if (deleting) {
      event.preventDefault();
      return;
    }
    deleteError = null;
  }

  function navigateBack(): void {
    const candidate = (page.state as { backHref?: unknown }).backHref;
    if (candidate === chatHref && window.history.length > 1) {
      window.history.back();
      return;
    }
    void goto(chatHref, { replaceState: true });
  }

  function handleBackClick(event: MouseEvent): void {
    event.preventDefault();
    navigateBack();
  }

  async function openReport(event: MouseEvent): Promise<void> {
    const target = reportHref;
    if (target === null) return;
    event.preventDefault();
    await goto(target, {
      state: {
        backHref: `${page.url.pathname}${page.url.search}${page.url.hash}`,
      },
    });
  }

  onMount(() => {
    void loadChat();
  });
</script>

<svelte:head>
  <title>LorePia — {character.name} 대화 설정</title>
</svelte:head>

<!-- The room detail Messages-style: pushed from the chat header's avatar,
     never a side drawer. -->
<div
  class="screen"
  use:edgeSwipeBack={{
    onBack: navigateBack,
    getUnderlay: () => activateBackSwipeSurface(chatHref),
  }}
>
  <LargeTitleHeader title="대화 설정">
    {#snippet leading()}
      <a
        class="back"
        href={chatHref}
        aria-label="대화로 돌아가기"
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
    {/snippet}
  </LargeTitleHeader>

  <section class="hero">
    <Avatar initial={character.initial} size={72} />
    <p class="hero-name">{character.name}</p>
    <p class="hero-tagline">{character.tagline}</p>
  </section>

  <section class="group" aria-label="표시">
    <div class="card">
      <div class="row">
        <span class="label">표시 모드</span>
        <div class="segment" role="group" aria-label="표시 모드 선택">
          {#each modeOptions as option (option.value)}
            <button
              type="button"
              class:active={chatRoomPrefs.mode === option.value}
              onclick={() => (chatRoomPrefs.mode = option.value)}
              >{option.label}</button
            >
          {/each}
        </div>
      </div>
    </div>
  </section>

  <section class="group" aria-label="바로가기">
    <div class="card">
      <a class="row link lp-state-layer" href={characterHref}>
        <span class="label">캐릭터 정보 보기</span>
        <svg
          class="chev"
          viewBox="0 0 24 24"
          width="14"
          height="14"
          fill="none"
          stroke="currentColor"
          stroke-width="2.2"
          stroke-linecap="round"
          stroke-linejoin="round"
          aria-hidden="true"
        >
          <path d="m9 18 6-6-6-6" />
        </svg>
      </a>
      <a class="row link lp-state-layer" href="/account">
        <span class="label">계정 및 설정</span>
        <svg
          class="chev"
          viewBox="0 0 24 24"
          width="14"
          height="14"
          fill="none"
          stroke="currentColor"
          stroke-width="2.2"
          stroke-linecap="round"
          stroke-linejoin="round"
          aria-hidden="true"
        >
          <path d="m9 18 6-6-6-6" />
        </svg>
      </a>
    </div>
  </section>

  <section class="group" aria-label="대화 관리">
    <p class="section-label">대화 관리</p>
    <div class="card">
      {#if loadingChat}
        <div class="row status" role="status">
          <span class="label">대화 정보를 불러오는 중입니다.</span>
        </div>
      {:else if chatError}
        <div class="row recovery" role="alert">
          <span class="label">{chatError}</span>
          <button type="button" onclick={loadChat}>다시 시도</button>
        </div>
      {:else if reportHref}
        <a
          class="row link lp-state-layer"
          href={reportHref}
          onclick={openReport}
        >
          <span class="label">AI 응답 신고 초안</span>
          <svg
            class="chev"
            viewBox="0 0 24 24"
            width="14"
            height="14"
            fill="none"
            stroke="currentColor"
            stroke-width="2.2"
            stroke-linecap="round"
            stroke-linejoin="round"
            aria-hidden="true"
          >
            <path d="m9 18 6-6-6-6" />
          </svg>
        </a>
        <button
          class="row danger"
          type="button"
          onclick={openDeleteConfirmation}
        >
          대화 삭제
        </button>
      {/if}
    </div>
  </section>

  <dialog
    bind:this={deleteDialog}
    class="dialog"
    role="alertdialog"
    aria-labelledby="delete-dialog-title"
    aria-describedby="delete-dialog-description"
    oncancel={handleDialogCancel}
  >
    <h2 id="delete-dialog-title">대화를 삭제할까요?</h2>
    <p id="delete-dialog-description">
      이 기기의 대화와 메시지가 삭제됩니다. 이 작업은 되돌릴 수 없습니다.
    </p>
    {#if deleteError}
      <p class="delete-error" role="alert">{deleteError}</p>
    {/if}
    <div class="delete-actions">
      <button
        bind:this={cancelDeleteButton}
        type="button"
        disabled={deleting}
        onclick={closeDeleteConfirmation}>취소</button
      >
      <button
        class="destructive"
        type="button"
        disabled={deleting}
        onclick={deleteChat}
        >{deleting ? "삭제하는 중…" : "삭제"}</button
      >
    </div>
  </dialog>
</div>

<style>
  .screen {
    height: 100%;
    overflow-y: auto;
    overscroll-behavior: none;
    display: flex;
    flex-direction: column;
    background: var(--surface-page);
    font-family: var(--font-ui);
    box-sizing: border-box;
    padding-bottom: calc(var(--sp-5) + var(--safe-bottom));
  }

  .back {
    width: var(--size-touch);
    height: var(--size-touch);
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--r-pill);
    color: var(--text-mid);
    transition:
      background var(--dur-fast) var(--ease-out),
      transform var(--dur-base) var(--ease-spring);
  }

  .back:active {
    background: var(--surface-bubble);
    transform: scale(0.9);
  }

  .hero {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--sp-1);
    padding: var(--sp-2) var(--sp-4) var(--sp-4);
    text-align: center;
    animation: lp-pop var(--dur-page) var(--ease-spring) backwards;
  }

  .hero-name {
    margin: var(--sp-2) 0 0;
    font-size: 18px;
    font-weight: 700;
    letter-spacing: -0.01em;
    color: var(--text-strong);
  }

  .hero-tagline {
    margin: 0;
    font-size: var(--fs-label);
    color: var(--text-mid);
  }

  .group {
    padding: 0 var(--sp-4);
    margin-top: var(--sp-3);
    animation: lp-rise var(--dur-page) var(--ease-out) backwards;
  }

  .group:nth-of-type(2) {
    animation-delay: 40ms;
  }

  .group:nth-of-type(3) {
    animation-delay: 90ms;
  }

  .group:nth-of-type(4) {
    animation-delay: 140ms;
  }

  .section-label {
    margin: 0 0 var(--sp-2) var(--sp-2);
    font-size: var(--fs-caption);
    color: var(--text-mid);
  }

  .card {
    padding: 0 var(--sp-4);
    background: var(--surface-card);
    border-radius: var(--r-card);
    box-shadow: var(--shadow-card);
  }

  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-3);
    min-height: 52px;
    padding: var(--sp-1) 0;
  }

  .row + .row {
    border-top: 0.5px solid var(--hairline);
  }

  .row.link {
    text-decoration: none;
  }

  button.row {
    width: 100%;
    border: 0;
    background: transparent;
    font-family: var(--font-ui);
    text-align: left;
    cursor: pointer;
  }

  .row.status {
    color: var(--text-mid);
  }

  .row.recovery button,
  .delete-actions button {
    min-height: var(--size-touch);
    padding: 0 var(--sp-3);
    border: 0.5px solid var(--hairline);
    border-radius: var(--r-pill);
    background: var(--surface-bubble);
    color: var(--text-strong);
    font-family: var(--font-ui);
    font-size: var(--fs-label);
    cursor: pointer;
  }

  .row.danger {
    justify-content: flex-start;
    color: #c62828;
    font-size: var(--fs-ui);
  }

  .dialog::backdrop {
    background: rgba(0, 0, 0, 0.28);
    -webkit-backdrop-filter: blur(3px);
    backdrop-filter: blur(3px);
  }

  .dialog {
    width: min(100%, 310px);
    box-sizing: border-box;
    margin: auto;
    padding: var(--sp-5);
    border: 0.5px solid var(--hairline);
    border-radius: 20px;
    background: var(--bar-bg);
    color: var(--text-strong);
    box-shadow: var(--shadow-float);
    text-align: center;
  }

  .dialog h2 {
    margin: 0;
    font-size: 18px;
    line-height: 1.3;
  }

  .dialog p {
    margin: var(--sp-2) 0 0;
    color: var(--text-mid);
    font-size: var(--fs-label);
    line-height: 1.5;
  }

  .dialog .delete-error {
    color: #c62828;
  }

  .delete-actions {
    display: flex;
    gap: var(--sp-2);
    margin-top: var(--sp-4);
  }

  .delete-actions button {
    flex: 1;
  }

  .delete-actions button.destructive {
    border-color: rgba(198, 40, 40, 0.25);
    background: rgba(198, 40, 40, 0.09);
    color: #c62828;
  }

  .delete-actions button:disabled {
    opacity: 0.45;
    cursor: default;
  }

  .label {
    font-size: var(--fs-ui);
    color: var(--text-strong);
  }

  .chev {
    flex-shrink: 0;
    color: var(--text-faint);
  }

  .segment {
    display: flex;
    background: var(--surface-bubble);
    border-radius: 10px;
    padding: 2px;
    gap: 2px;
  }

  .segment button {
    min-height: var(--size-touch);
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

  @media (min-width: 700px) {
    .hero,
    .group {
      width: min(100% - var(--sp-4) * 2, 680px);
      margin-inline: auto;
      box-sizing: border-box;
    }
  }
</style>
