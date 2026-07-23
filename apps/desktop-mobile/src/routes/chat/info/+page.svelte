<script lang="ts">
  import { goto } from "$app/navigation";

  import "$lib/design/tokens.css";

  import { chatRoomPrefs } from "$lib/chat/room-prefs.svelte";
  import type { ThreadMode } from "$lib/chat/types";
  import Avatar from "$lib/ui/Avatar.svelte";
  import LargeTitleHeader from "$lib/ui/LargeTitleHeader.svelte";
  import { edgeSwipeBack } from "$lib/ui/edge-back";

  const characterName = "세라핀";
  const characterInitial = "세";

  const modeOptions: { value: ThreadMode; label: string }[] = [
    { value: "chat", label: "채팅" },
    { value: "story", label: "스토리" },
  ];
</script>

<svelte:head>
  <title>LorePia — 대화 설정</title>
</svelte:head>

<!-- The room detail Messages-style: pushed from the chat header's avatar,
     never a side drawer. -->
<div class="screen" use:edgeSwipeBack={{ onBack: () => goto("/chat") }}>
  <LargeTitleHeader title="대화 설정">
    {#snippet leading()}
      <a class="back" href="/chat" aria-label="대화로 돌아가기">
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
    <Avatar initial={characterInitial} size={72} />
    <p class="hero-name">{characterName}</p>
    <p class="hero-tagline">달빛 서고의 사서</p>
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
      <a class="row link lp-state-layer" href="/character/seraphine">
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

  @media (min-width: 700px) {
    .hero,
    .group {
      width: min(100% - var(--sp-4) * 2, 680px);
      margin-inline: auto;
      box-sizing: border-box;
    }
  }
</style>
